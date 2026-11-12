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
//! On Linux and the BSDs, rfd reports a portal that failed to show its dialog as a cancel (#2999).
//! There a cancel quicker than anyone could dismiss a dialog is taken as that failure: the dialog
//! is asked for once more without a parent window, and if that also comes back at once the
//! built-in browser shows. Windows and macOS always show their dialog, so a quick cancel there is
//! the person's and is answered as one.
//!
//! [`Services::file_dialog`]: wordcraft_ui_egui::Services::file_dialog

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{Receiver, Sender, channel};
#[cfg(all(unix, not(target_os = "macos")))]
use std::sync::{Arc, Mutex};

use wordcraft_ui_egui::{FileDialogAnswer, FileDialogRequest};

/// A dialog the app asked for, with the channel its answer goes back through.
type Request = (FileDialogRequest, Sender<FileDialogAnswer>);

/// The `Services::file_dialog` hook.
pub type Hook = Box<dyn Fn(FileDialogRequest) -> Receiver<FileDialogAnswer>>;

/// Cancels the portal Save dialog the launcher is waiting on, if any.
///
/// The portal's dialog lives outside the app's window, so a portal that stops answering cannot be
/// dismissed from WordCraft. Giving the dialog up (File › Open › Browse in WordCraft…) cancels the
/// request: the waiting thread closes it and reports a cancellation, and the built-in browser
/// opens instead. A no-op where Save As doesn't ask the portal (macOS and Windows).
#[derive(Clone, Default)]
pub struct CancelHandle {
    #[cfg(all(unix, not(target_os = "macos")))]
    armed: Arc<Mutex<Option<Armed>>>,
    #[cfg(all(unix, not(target_os = "macos")))]
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

/// The token a request installed in [`CancelHandle`], with the identity that owns it.
#[cfg(all(unix, not(target_os = "macos")))]
struct Armed {
    id: u64,
    token: crate::portal_save::CancelToken,
}

impl CancelHandle {
    /// Cancels the pending portal request, if there is one.
    pub fn cancel(&self) {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let mut slot = self.armed.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(armed) = slot.take() {
                armed.token.cancel();
            }
        }
    }

    /// Arms the handle for the portal request about to wait, returning its token and identity.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn arm(&self) -> Armed {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed).wrapping_add(1);
        let token = crate::portal_save::CancelToken::new();
        *self.armed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Armed { id, token: token.clone() });
        Armed { id, token }
    }

    /// Disarms the handle once the request with `id` has answered, leaving a newer request's
    /// token alone: an abandoned request finishing late must not make the next one uncancellable.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn disarm(&self, id: u64) {
        let mut slot = self.armed.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|armed| armed.id == id) {
            *slot = None;
        }
    }
}

/// A picker's answer, waited for off the UI thread.
type Picking = Pin<Box<dyn Future<Output = Vec<rfd::FileHandle>> + Send>>;

/// Shows the dialogs the app queued; lives in the eframe app, which has the window.
pub struct Launcher {
    queue: Receiver<Request>,
    ctx: egui::Context,
    cancel: CancelHandle,
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
    (hook, Launcher { queue, ctx: ctx.clone(), cancel: CancelHandle::default() })
}

impl Launcher {
    /// A handle that cancels the portal Save dialog the launcher is waiting on, if any.
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// Show the queued dialogs with the window as their parent, so the portal (or the system)
    /// keeps them on top of it. Call after every pass that may ask for one.
    pub fn show(&self, parent: &eframe::Frame) {
        while let Ok((req, answer)) = self.queue.try_recv() {
            // Built here, on the UI thread: macOS shows the panel as a sheet on the window now.
            let picking = picker(rfd::AsyncFileDialog::new().set_parent(parent), &req);
            #[cfg(all(unix, not(target_os = "macos")))]
            let unparented = picker(rfd::AsyncFileDialog::new(), &req);
            #[cfg(all(unix, not(target_os = "macos")))]
            let parent_id = {
                use eframe::wgpu::rwh::HasWindowHandle;
                crate::portal_save::parent_id(parent.window_handle().ok().map(|h| h.as_raw()))
            };
            // Armed before the thread starts, so giving the dialog up cannot race its start.
            #[cfg(all(unix, not(target_os = "macos")))]
            let portal_arm = matches!(&req, FileDialogRequest::Save { .. }).then(|| self.cancel.arm());
            #[cfg(all(unix, not(target_os = "macos")))]
            let portal_id = portal_arm.as_ref().map(|armed| armed.id);
            #[cfg(all(unix, not(target_os = "macos")))]
            let cancel_handle = self.cancel.clone();
            let ctx = self.ctx.clone();
            let waiting = std::thread::Builder::new().name("file dialog".into()).spawn(move || {
                // The pickers only start when first polled: without a system dialog they never do.
                let picked = match no_system_dialog() {
                    Some(reason) => Err(reason),
                    None => {
                        #[cfg(all(unix, not(target_os = "macos")))]
                        {
                            if let FileDialogRequest::Save { name, dir } = &req {
                                let cancel = portal_arm.as_ref().map(|armed| armed.token.clone());
                                let portal_req = crate::portal_save::SaveRequest {
                                    parent: parent_id,
                                    name: Some(name.clone()),
                                    suggested: Some(name.clone()),
                                    folder: dir.clone().map(std::path::PathBuf::from),
                                    filters: save_filters(name),
                                    current: Some(0),
                                    cancel,
                                    timeout: None,
                                };
                                let portal_res = zbus::blocking::Connection::session()
                                    .map_err(|e| e.to_string())
                                    .and_then(|conn| crate::portal_save::save_as(&conn, portal_req));
                                match portal_res {
                                    Ok(Some(path)) => Ok(vec![path.to_string_lossy().into_owned()]),
                                    Ok(None) => Ok(Vec::new()),
                                    Err(e) if crate::portal_save::is_cancelled_error(&e) => Ok(Vec::new()),
                                    Err(e) if crate::portal_save::is_ambiguous_filter_error(&e) => {
                                        log::warn!("portal save format ambiguous ({e}); cancelling save without explicit format choice");
                                        Ok(Vec::new())
                                    }
                                    Err(e) => {
                                        // Giving the dialog up cancels the request. A portal that then
                                        // fails with another error must not open the rfd dialog, which
                                        // nothing would be waiting for.
                                        if portal_arm.as_ref().is_some_and(|armed| armed.token.is_cancelled()) {
                                            log::info!("portal save dialog given up ({e})");
                                            Ok(Vec::new())
                                        } else {
                                            log::info!("portal save dialog unavailable ({e}), using rfd");
                                            let picked = pick(picking, unparented, TOO_QUICK);
                                            picked.map(|f| f.iter().map(|f| f.path().to_string_lossy().to_string()).collect())
                                        }
                                    }
                                }
                            } else {
                                // Only the portal folds a failure into "cancelled" (#2999).
                                let picked = pick(picking, unparented, TOO_QUICK);
                                picked.map(|f| f.iter().map(|f| f.path().to_string_lossy().to_string()).collect())
                            }
                        }
                        #[cfg(not(all(unix, not(target_os = "macos"))))]
                        {
                            let picked: Result<_, String> = Ok(pollster::block_on(picking));
                            picked.map(|f| f.iter().map(|f| f.path().to_string_lossy().to_string()).collect())
                        }
                    }
                };
                #[cfg(all(unix, not(target_os = "macos")))]
                if let Some(armed) = &portal_arm {
                    cancel_handle.disarm(armed.id);
                }
                let _ = answer.send(picked);
                ctx.request_repaint();
            });
            // The answer's sender went with the closure, so the app sees the dialog as cancelled.
            if let Err(e) = waiting {
                log::error!("couldn't wait for the file dialog: {e}");
                #[cfg(all(unix, not(target_os = "macos")))]
                if let Some(id) = portal_id {
                    self.cancel.disarm(id);
                }
            }
        }
    }
}

/// The picker for `req` on `dialog`.
fn picker(dialog: rfd::AsyncFileDialog, req: &FileDialogRequest) -> Picking {
    match req.clone() {
        FileDialogRequest::Open { purpose } => Box::pin(async move { open_filters(dialog, &purpose).pick_file().await.into_iter().collect() }),
        FileDialogRequest::OpenMany { purpose } => Box::pin(async move { open_filters(dialog, &purpose).pick_files().await.unwrap_or_default() }),
        FileDialogRequest::Save { name, dir } => {
            let mut dialog = dialog.set_file_name(name.clone());
            // Save as Word Template starts in the user templates folder (#2852).
            if let Some(dir) = dir.filter(|d| std::path::Path::new(d).is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            for (label, exts) in save_filters(&name) {
                dialog = dialog.add_filter(label, &exts);
            }
            Box::pin(async move { dialog.save_file().await.into_iter().collect() })
        }
    }
}

/// Quicker than anyone can see a dialog and dismiss it.
// ponytail: a timing heuristic, as rfd folds the portal's failure code into "cancelled". A portal
// failing slower than this still reads as a cancel; ask rfd for the response code if it matters.
#[cfg(all(unix, not(target_os = "macos")))]
const TOO_QUICK: std::time::Duration = std::time::Duration::from_millis(300);

/// Wait for `picking`. Nothing picked within `too_quick` is a portal that failed to show its
/// dialog (#2999), not a person cancelling: try `unparented`, as a portal may fail on the parent
/// window; if that too comes back at once, an error, so the app shows its built-in browser.
/// Linux and the BSDs only (built everywhere for its test).
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn pick<T>(
    picking: impl Future<Output = Vec<T>>,
    unparented: impl Future<Output = Vec<T>>,
    too_quick: std::time::Duration,
) -> Result<Vec<T>, String> {
    fn timed<T>(f: impl Future<Output = Vec<T>>, too_quick: std::time::Duration) -> (Vec<T>, bool) {
        let start = std::time::Instant::now();
        let picked = pollster::block_on(f);
        let quick = picked.is_empty() && start.elapsed() < too_quick;
        (picked, quick)
    }
    match timed(picking, too_quick) {
        (_, true) => log::warn!("the system file dialog closed at once; asking again without a parent window"),
        (picked, false) => return Ok(picked),
    }
    match timed(unparented, too_quick) {
        (_, true) => Err("the system file dialog closed at once".into()),
        (picked, false) => Ok(picked),
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
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase)
        // A .markdown file is under Markdown's one entry in the list.
        .map(|e| if e == "markdown" { "md".to_string() } else { e })
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
/// On Linux and the BSDs, [`crate::portal_save`] asks the portal directly over D-Bus and gives a
/// name the dialog left alone the chosen type's extension (#6180). When falling back to `rfd`, the
/// extension in the name wins, as `rfd` doesn't report which entry the user picked.
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
        "ott" => "OpenDocument Text Template".into(),
        "fodt" => "Flat OpenDocument Text".into(),
        "tex" => "LaTeX".into(),
        "png" => "PNG Image".into(),
        "json" => "JSON".into(),
        "xml" => "XML".into(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::{pick, save_exts, save_filters};
    use std::time::Duration;

    /// #2999: a dialog that "cancels" at once is a portal that failed: asked again without a
    /// parent, then the built-in browser. A pick, or a cancel after a while, is the answer.
    #[test]
    fn an_instant_cancel_retries_then_falls_back() {
        let quick = Duration::from_millis(200);
        let slow_cancel = || async {
            // The test polls this future by hand, with no async runtime, so a blocking sleep
            // models a slow portal response.
            std::thread::sleep(Duration::from_millis(250)); // NOSONAR
            Vec::<u8>::new()
        };
        assert_eq!(pick(async { vec![1u8] }, async { vec![2] }, quick), Ok(vec![1]));
        assert_eq!(pick(async { vec![] }, async { vec![2u8] }, quick), Ok(vec![2]), "asked again without a parent");
        assert!(pick(async { Vec::<u8>::new() }, async { vec![] }, quick).is_err(), "the built-in browser");
        assert_eq!(pick(slow_cancel(), async { vec![2u8] }, quick), Ok(vec![]), "a real cancel isn't asked again");
        assert_eq!(pick(async { vec![] }, slow_cancel(), quick), Ok(vec![]), "the second dialog cancelled for real");
    }

    /// Giving the portal Save dialog up cancels the request the launcher armed, and a request
    /// that has already answered is not affected by a later cancel.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn cancelling_a_pending_portal_request_marks_its_token() {
        let handle = super::CancelHandle::default();
        let armed = handle.arm();
        assert!(!armed.token.is_cancelled());
        handle.cancel();
        assert!(armed.token.is_cancelled());

        let answered = handle.arm();
        handle.disarm(answered.id);
        handle.cancel();
        assert!(!answered.token.is_cancelled(), "an answered request is no longer cancelled");
    }

    /// A request that finishes late must not disarm the token of a request started after it:
    /// otherwise the newer Save dialog would become uncancellable.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_late_answer_keeps_the_newer_requests_token() {
        let handle = super::CancelHandle::default();
        let abandoned = handle.arm();
        let newer = handle.arm();
        handle.disarm(abandoned.id);
        handle.cancel();
        assert!(newer.token.is_cancelled(), "the newer request is still armed");
        assert!(!abandoned.token.is_cancelled(), "only the armed request is cancelled");
    }

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
        assert_eq!(save_exts("notes.md")[0], "md");
        assert_eq!(save_exts("notes.markdown")[0], "md");
        // A suggested name always has an extension; anything unreadable falls back to Word's.
        assert_eq!(save_exts("no extension")[0], "docx");
        assert_eq!(save_exts("mystery.rtf2")[0], "docx");
        // Every savable format is still offered, once.
        let exts = save_exts("novel.docx");
        assert!(wordcraft_engine::io::SAVE_EXTS.iter().all(|e| exts.iter().any(|x| x == e)));
        assert_eq!(exts.len(), wordcraft_engine::io::SAVE_EXTS.len());
    }
}
