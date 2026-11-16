//! "Save changes?" as a sheet on the document's own window, the way Mac apps ask it, instead of
//! the built-in prompt: the hook for [`Services::save_prompt`](wordcraft_ui_egui::Services::save_prompt)
//! and the launcher that shows what it queues. A sheet holds only its window: the other windows
//! take input, and each can have a sheet of its own. The answer goes back through
//! `ui.saveChanges`, so quitting, closing and opening still wait on it.

use std::sync::mpsc::{Receiver, Sender, channel};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertThirdButtonReturn, NSApplication, NSModalResponse, NSWindow};
use objc2_foundation::NSString;
use wordcraft_ui_egui::i18n::t;

/// The document's window, the question, the detail and where the answer goes.
type Request = (Retained<NSWindow>, String, String, Sender<String>);

/// The hook for `Services::save_prompt`.
pub type Hook = Box<dyn Fn(String, String, String) -> Option<Receiver<String>>>;

/// Shows the prompts the app queued; lives in the eframe app.
pub struct Launcher {
    queue: Receiver<Request>,
    ctx: egui::Context,
}

pub fn hook(ctx: &egui::Context) -> (Hook, Launcher) {
    let (queue_tx, queue) = channel::<Request>();
    let hook: Hook = Box::new(move |title, question, detail| {
        // No window that is surely the document's: the app asks in its own prompt instead.
        let window = window_titled(MainThreadMarker::new()?, &title)?;
        let (answer, rx) = channel();
        // If the launcher is gone, the dropped answer reads as Cancel in the app.
        let _ = queue_tx.send((window, question, detail, answer));
        Some(rx)
    });
    (hook, Launcher { queue, ctx: ctx.clone() })
}

impl Launcher {
    /// Shows the queued prompts as sheets, each on its document's window. Call after every pass
    /// that may ask one.
    pub fn show(&self, mtm: MainThreadMarker) {
        while let Ok((parent, question, detail, answer)) = self.queue.try_recv() {
            // A sheet on a window in the Dock would ask out of sight.
            if parent.isMiniaturized() {
                parent.deminiaturize(None);
            }
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

/// The one window titled `title`, shown or in the Dock: the app gives each window its document's
/// name. Two documents with the same name are left to the built-in prompt, so Don't Save never
/// answers for the wrong one.
/// shortcut: matches by title; upgrade to the viewport's own window handle if egui exposes it.
fn window_titled(mtm: MainThreadMarker, title: &str) -> Option<Retained<NSWindow>> {
    let windows = NSApplication::sharedApplication(mtm).windows();
    let mut found = windows.iter().filter(|w| (w.isVisible() || w.isMiniaturized()) && w.title().to_string() == title);
    let window = found.next()?;
    found.next().is_none().then_some(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_the_documents_window_the_app_asks_itself() {
        let (hook, _launcher) = hook(&egui::Context::default());
        // Off the main thread, as here, or with no such window, no sheet can be placed.
        assert!(hook("Document1 - WordCraft".into(), "Do you want to save changes?".into(), String::new()).is_none());
    }
}
