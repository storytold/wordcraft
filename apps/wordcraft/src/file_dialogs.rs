//! Native file dialogs (Open, Save As, Export, pictures) that don't freeze the window (#94).
//!
//! The blocking `rfd::FileDialog` ran on the UI thread: on Linux the xdg-desktop-portal call
//! blocked the event loop until the dialog closed, and with no parent window the dialog could open
//! behind WordCraft, which then sat there frozen. Now the app's request ([`Services::file_dialog`])
//! is queued; the frame (which has the window) builds an `rfd::AsyncFileDialog` parented to the
//! window, and a background thread waits for the answer, hands it to the app and wakes the UI.
//!
//! On Linux and the BSDs rfd asks the xdg-desktop-portal FileChooser and, failing that, runs
//! `zenity`; with neither (a minimal Wayland compositor such as labwc with only
//! xdg-desktop-portal-wlr, #576) it answers "cancelled" and nothing ever shows. So the waiting
//! thread first checks that one of them is there and otherwise answers with an error, and the app
//! shows its built-in file browser. `WORDCRAFT_FILE_DIALOG=builtin` always uses the built-in one.
//!
//! [`Services::file_dialog`]: wordcraft_ui_egui::Services::file_dialog

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{Receiver, Sender, channel};

use wordcraft_ui_egui::{FileDialogAnswer, FileDialogRequest};

/// A dialog the app asked for, with the channel its answer goes back through.
type Request = (FileDialogRequest, Sender<FileDialogAnswer>);

/// The `Services::file_dialog` hook.
pub type Hook = Box<dyn Fn(FileDialogRequest) -> Receiver<FileDialogAnswer>>;

/// A picker's answer, waited for off the UI thread.
type Picking = Pin<Box<dyn Future<Output = Vec<rfd::FileHandle>> + Send>>;

/// Shows the dialogs the app queued; lives in the eframe app, which has the window.
pub struct Launcher {
    queue: Receiver<Request>,
    ctx: egui::Context,
}

/// The hook for [`Services::file_dialog`](wordcraft_ui_egui::Services::file_dialog) and the
/// launcher that shows what it queues.
pub fn hook(ctx: &egui::Context) -> (Hook, Launcher) {
    let (queue_tx, queue) = channel::<Request>();
    let hook: Hook = Box::new(move |req| {
        let (answer, rx) = channel();
        // If the launcher is gone, the dropped answer reads as "cancelled" in the app.
        let _ = queue_tx.send((req, answer));
        rx
    });
    (hook, Launcher { queue, ctx: ctx.clone() })
}

impl Launcher {
    /// Show the queued dialogs with the window as their parent, so the portal (or the system)
    /// keeps them on top of it. Call after every pass that may ask for one.
    pub fn show(&self, parent: &eframe::Frame) {
        while let Ok((req, answer)) = self.queue.try_recv() {
            // Built here, on the UI thread: macOS shows the panel as a sheet on the window now.
            let dialog = rfd::AsyncFileDialog::new().set_parent(parent);
            let picking: Picking = match req {
                FileDialogRequest::Open { purpose } => {
                    Box::pin(async move { open_filters(dialog, &purpose).pick_file().await.into_iter().collect() })
                }
                FileDialogRequest::OpenMany { purpose } => {
                    Box::pin(async move { open_filters(dialog, &purpose).pick_files().await.unwrap_or_default() })
                }
                FileDialogRequest::Save { name } => {
                    let mut dialog = dialog.set_file_name(name.clone());
                    for (label, exts) in save_filters(&name) {
                        dialog = dialog.add_filter(label, &exts);
                    }
                    Box::pin(async move { dialog.save_file().await.into_iter().collect() })
                }
            };
            let ctx = self.ctx.clone();
            let waiting = std::thread::Builder::new().name("file dialog".into()).spawn(move || {
                // The picker only starts when first polled: without a system dialog it never does.
                let picked = match no_system_dialog() {
                    Some(reason) => Err(reason),
                    None => Ok(pollster::block_on(picking).iter().map(|f| f.path().to_string_lossy().to_string()).collect()),
                };
                let _ = answer.send(picked);
                ctx.request_repaint();
            });
            // The answer's sender went with the closure, so the app sees the dialog as cancelled.
            if let Err(e) = waiting {
                log::error!("couldn't wait for the file dialog: {e}");
            }
        }
    }
}

/// The file types Open offers (shared with the built-in browser).
fn open_filters(mut d: rfd::AsyncFileDialog, purpose: &str) -> rfd::AsyncFileDialog {
    for (name, exts) in wordcraft_ui_egui::open_filters(purpose) {
        d = d.add_filter(*name, exts);
    }
    d
}

/// Why no system file dialog can be shown, if it can't (#576); run off the UI thread.
fn no_system_dialog() -> Option<String> {
    if std::env::var_os("WORDCRAFT_FILE_DIALOG").is_some_and(|v| v == "builtin") {
        return Some("WORDCRAFT_FILE_DIALOG=builtin".into());
    }
    portal_or_zenity_missing()
}

/// Linux and the BSDs: rfd needs a file chooser portal or zenity. When `dbus-send` can't ask
/// the session bus, the answer is unknown and rfd gets to try.
#[cfg(all(unix, not(target_os = "macos")))]
fn portal_or_zenity_missing() -> Option<String> {
    if on_path("zenity") {
        return None;
    }
    let portal = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply=literal",
            "--reply-timeout=10000",
            "--dest=org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.DBus.Properties.Get",
            "string:org.freedesktop.portal.FileChooser",
            "string:version",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match portal {
        Ok(status) if !status.success() => Some("no xdg-desktop-portal FileChooser on the session bus and no zenity".into()),
        Ok(_) => None,
        Err(e) => {
            log::debug!("couldn't ask D-Bus for the file chooser portal: {e}");
            None
        }
    }
}

/// Windows and macOS always have their system dialog.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn portal_or_zenity_missing() -> Option<String> {
    None
}

/// Whether an executable named `program` is in a `PATH` folder.
#[cfg(all(unix, not(target_os = "macos")))]
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The file types Save As offers: every format the engine saves, with the suggested name's own
/// format first. rfd makes a filter's first extension the dialog's default, which the Windows,
/// macOS and Linux panels append to a file name the user typed without one; with no filter at
/// all the name came back bare and the save failed, since the format is chosen by extension
/// (#689).
fn save_exts(name: &str) -> Vec<String> {
    let suggested = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| wordcraft_engine::io::SAVE_EXTS.contains(&e.as_str()))
        .unwrap_or_else(|| "docx".to_string());
    let mut exts: Vec<String> = Vec::with_capacity(wordcraft_engine::io::SAVE_EXTS.len());
    exts.push(suggested.clone());
    exts.extend(wordcraft_engine::io::SAVE_EXTS.iter().filter(|e| **e != suggested).map(|e| (*e).to_string()));
    exts
}

/// The "Save as type" list (#1399): one named entry per format, the suggested one first (see
/// [`save_exts`] for why the order matters).
///
/// rfd doesn't report which entry the user picked, so if they choose "PDF" but leave "name.docx"
/// in the name box, the extension in the name wins, as the format is chosen by extension.
fn save_filters(name: &str) -> Vec<(String, Vec<String>)> {
    save_exts(name).into_iter().map(|e| (format!("{} (*.{e})", format_name(&e)), vec![e])).collect()
}

/// A readable name for a save format's extension.
fn format_name(ext: &str) -> String {
    match ext {
        "docx" => "Word Document".into(),
        "docm" => "Word Macro-Enabled Document".into(),
        "dotx" => "Word Template".into(),
        "dotm" => "Word Macro-Enabled Template".into(),
        "pdf" => "PDF".into(),
        "txt" => "Plain Text".into(),
        "md" => "Markdown".into(),
        "html" => "Web Page".into(),
        "rtf" => "Rich Text Format".into(),
        "odt" => "OpenDocument Text".into(),
        "tex" => "LaTeX".into(),
        "png" => "PNG Image".into(),
        "json" => "JSON".into(),
        "xml" => "XML".into(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::{save_exts, save_filters};

    /// #1399: the Save dialog's type list has one named entry per format, the suggested one first,
    /// every savable extension exactly once.
    #[test]
    fn save_dialog_lists_one_named_type_per_format() {
        let filters = save_filters("report.odt");
        assert_eq!(filters.len(), wordcraft_engine::io::SAVE_EXTS.len());
        assert_eq!(filters[0].1, vec!["odt".to_string()]);
        assert!(filters[0].0.contains("OpenDocument"), "{}", filters[0].0);
        let all: Vec<&String> = filters.iter().flat_map(|(_, e)| e).collect();
        for e in wordcraft_engine::io::SAVE_EXTS {
            assert_eq!(all.iter().filter(|x| x.as_str() == *e).count(), 1, "{e}");
        }
        assert!(filters.iter().all(|(l, e)| !l.trim().is_empty() && e.len() == 1));
        let mut labels: Vec<&String> = filters.iter().map(|(l, _)| l).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), filters.len(), "labels are distinct");
        // Unknown names fall back to Word's format first.
        assert_eq!(save_filters("no extension")[0].1, vec!["docx".to_string()]);
    }

    /// #689: the Save dialog's default extension is the suggested name's own format, so a file
    /// name the user types without one is saved in the document's format instead of coming back
    /// bare and failing.
    #[test]
    fn save_dialog_defaults_to_the_suggested_format() {
        assert_eq!(save_exts("novel.docx")[0], "docx");
        assert_eq!(save_exts("report.odt")[0], "odt");
        assert_eq!(save_exts("annual report.pdf")[0], "pdf");
        // A suggested name always has an extension; anything unreadable falls back to Word's.
        assert_eq!(save_exts("no extension")[0], "docx");
        assert_eq!(save_exts("mystery.rtf2")[0], "docx");
        // Every savable format is still offered, once.
        let exts = save_exts("novel.docx");
        assert!(wordcraft_engine::io::SAVE_EXTS.iter().all(|e| exts.iter().any(|x| x == e)));
        assert_eq!(exts.len(), wordcraft_engine::io::SAVE_EXTS.len());
    }
}
