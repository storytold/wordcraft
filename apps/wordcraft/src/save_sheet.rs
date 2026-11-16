//! "Save changes?" as a sheet on the document's own window, the way Mac apps ask it, instead of
//! the built-in prompt: the hook for [`Services::save_prompt`](wordcraft_ui_egui::Services::save_prompt)
//! and the launcher that shows what it queues. A sheet holds only its window: the other windows
//! take input, and each can have a sheet of its own. The answer goes back through
//! `ui.saveChanges`, so quitting, closing and opening still wait on it.

use std::sync::mpsc::{Receiver, Sender, channel};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertThirdButtonReturn, NSApplication, NSModalResponse, NSWindow};
use objc2_foundation::NSString;
use wordcraft_ui_egui::i18n::t;

/// The window's title, the question, the detail and where the answer goes.
type Request = (String, String, String, Sender<String>);

/// The hook for `Services::save_prompt`.
pub type Hook = Box<dyn Fn(String, String, String) -> Receiver<String>>;

/// Shows the prompts the app queued; lives in the eframe app.
pub struct Launcher {
    queue: Receiver<Request>,
    ctx: egui::Context,
}

pub fn hook(ctx: &egui::Context) -> (Hook, Launcher) {
    let (queue_tx, queue) = channel::<Request>();
    let hook: Hook = Box::new(move |window, question, detail| {
        let (answer, rx) = channel();
        // If the launcher is gone, the dropped answer reads as Cancel in the app.
        let _ = queue_tx.send((window, question, detail, answer));
        rx
    });
    (hook, Launcher { queue, ctx: ctx.clone() })
}

impl Launcher {
    /// Shows the queued prompts as sheets, each on its document's window. Call after every pass
    /// that may ask one.
    pub fn show(&self, mtm: MainThreadMarker) {
        while let Ok((window, question, detail, answer)) = self.queue.try_recv() {
            let Some(parent) = window_titled(mtm, &window) else {
                // No window to hold it: answered as Cancel, so nothing is lost.
                log::warn!("save sheet: no window titled {window:?}");
                continue;
            };
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str(&question));
            alert.setInformativeText(&NSString::from_str(&detail));
            // Save is the default button, Cancel answers Escape and Don't Save sits apart, as in
            // every Mac app.
            alert.addButtonWithTitle(&NSString::from_str(t("Save")));
            alert.addButtonWithTitle(&NSString::from_str(t("Cancel"))).setKeyEquivalent(&NSString::from_str("\u{1b}"));
            alert.addButtonWithTitle(&NSString::from_str(t("Don't Save")));
            let ctx = self.ctx.clone();
            let keep = alert.clone();
            let done = RcBlock::new(move |response: NSModalResponse| {
                let _ = &keep; // the alert lives until it has answered
                let reply = match response {
                    r if r == NSAlertFirstButtonReturn => "save",
                    r if r == NSAlertThirdButtonReturn => "dontSave",
                    _ => "cancel",
                };
                let _ = answer.send(reply.into());
                ctx.request_repaint();
            });
            alert.beginSheetModalForWindow_completionHandler(&parent, Some(&done));
        }
    }
}

/// The document window titled `title`: the app gives each window its document's name.
/// shortcut: two windows on one document with the same title get the first; upgrade to the
/// viewport's own window handle if that shows up.
fn window_titled(mtm: MainThreadMarker, title: &str) -> Option<objc2::rc::Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    let found = windows.iter().find(|w| w.isVisible() && w.title().to_string() == title);
    found.or_else(|| app.mainWindow())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launcher_that_is_gone_reads_as_cancel() {
        let (hook, launcher) = hook(&egui::Context::default());
        let rx = hook("Document1 - WordCraft".into(), "Do you want to save changes?".into(), String::new());
        drop(launcher);
        assert!(rx.recv().is_err(), "the app answers a dropped sender with Cancel");
    }
}
