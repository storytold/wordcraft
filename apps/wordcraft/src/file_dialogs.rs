//! Native file dialogs (Open, Save As, Export, pictures) that don't freeze the window (#94).
//!
//! The blocking `rfd::FileDialog` ran on the UI thread: on Linux the xdg-desktop-portal call
//! blocked the event loop until the dialog closed, and with no parent window the dialog could open
//! behind WordCraft, which then sat there frozen. Now the app's request ([`Services::file_dialog`])
//! is queued; the frame (which has the window) builds an `rfd::AsyncFileDialog` parented to the
//! window, and a background thread waits for the answer, hands it to the app and wakes the UI.
//!
//! [`Services::file_dialog`]: wordcraft_ui_egui::Services::file_dialog

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{Receiver, Sender, channel};

use wordcraft_ui_egui::FileDialogRequest;

/// A dialog the app asked for, with the channel its answer goes back through.
type Request = (FileDialogRequest, Sender<Option<String>>);

/// The `Services::file_dialog` hook.
pub type Hook = Box<dyn Fn(FileDialogRequest) -> Receiver<Option<String>>>;

/// A picker's answer, waited for off the UI thread.
type Picking = Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>>;

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
                FileDialogRequest::Open { purpose } => Box::pin(open_filters(dialog, &purpose).pick_file()),
                FileDialogRequest::Save { name } => Box::pin(dialog.set_file_name(name).save_file()),
            };
            let ctx = self.ctx.clone();
            let waiting = std::thread::Builder::new().name("file dialog".into()).spawn(move || {
                let picked = pollster::block_on(picking).map(|f| f.path().to_string_lossy().to_string());
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

/// The file types Open offers: pictures for Insert/Change Picture, recipient lists for Select
/// Recipients › Use an Existing List…, documents otherwise.
fn open_filters(d: rfd::AsyncFileDialog, purpose: &str) -> rfd::AsyncFileDialog {
    if purpose == "picture" {
        d.add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
    } else if purpose == "recipients" {
        d.add_filter("Recipient lists", &["csv", "tsv", "txt", "xlsx", "xlsm", "ods"]).add_filter("All files", &["*"])
    } else {
        d.add_filter("Documents", &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "rtf", "txt", "md", "html", "htm", "tex", "json"])
            .add_filter("Word document", &["docx"])
            .add_filter("All files", &["*"])
    }
}
