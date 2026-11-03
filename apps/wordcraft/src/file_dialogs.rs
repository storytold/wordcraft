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
            let picking = picker(rfd::AsyncFileDialog::new().set_parent(parent), &req);
            #[cfg(all(unix, not(target_os = "macos")))]
            let unparented = picker(rfd::AsyncFileDialog::new(), &req);
            let ctx = self.ctx.clone();
            let waiting = std::thread::Builder::new().name("file dialog".into()).spawn(move || {
                // The pickers only start when first polled: without a system dialog they never do.
                let picked = match no_system_dialog() {
                    Some(reason) => Err(reason),
                    None => {
                        // Only the portal folds a failure into "cancelled" (#2999).
                        #[cfg(all(unix, not(target_os = "macos")))]
                        let picked = pick(picking, unparented, TOO_QUICK);
                        #[cfg(not(all(unix, not(target_os = "macos"))))]
                        let picked: Result<_, String> = Ok(pollster::block_on(picking));
                        picked.map(|f| f.iter().map(|f| f.path().to_string_lossy().to_string()).collect())
                    }
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
            for (label, exts) in wordcraft_ui_egui::save_filters(&name) {
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

#[cfg(test)]
mod tests {
    use super::pick;
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
}
