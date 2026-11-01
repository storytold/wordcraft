//! macOS open-documents and quit Apple events (#27).
//!
//! Finder double-clicks, Open With, drops on the Dock icon and `open -a WordCraft letter.docx` don't
//! pass paths on the command line: LaunchServices sends the running (or just-launched) app a
//! `kAEOpenDocuments` ('odoc') Apple event. winit 0.30 doesn't handle it and owns the
//! `NSApplicationDelegate`, so AppKit answered "WordCraft cannot open files in the Word Document
//! format". Handling it ourselves needs Objective-C class declarations, i.e. `unsafe`, which this
//! workspace forbids. The `fmv-macos-events` crate (also used by PdfCraft and PhotoCraft)
//! wraps exactly that, an `NSAppleEventManager` handler registered before Finder's launch event
//! that leaves winit's delegate alone, behind a safe main-thread API.

use fmv_macos_events::{Event, Inbox, Registration};
use wordcraft_ui_egui::WordApp;

/// Keeps the Apple-event handlers registered; hold it until the event loop returns.
pub struct AppleEvents {
    _registration: Registration,
    inbox: Inbox,
}

impl AppleEvents {
    /// Register the handlers. Call on the main thread before the event loop starts, so the event
    /// that launched the app (a Finder double-click) is caught too.
    pub fn install() -> Self {
        let (registration, inbox) = Registration::install();
        Self { _registration: registration, inbox }
    }

    /// The queue the app drains every frame ([`poll`]); events arriving later wake `ctx`.
    pub fn connect(&self, ctx: &egui::Context) -> Inbox {
        let ctx = ctx.clone();
        self.inbox.set_wake(move || ctx.request_repaint());
        self.inbox.clone()
    }
}

/// Open the documents and act on the quit requests that arrived since the last frame. A document
/// opens like File › Open: beside the open ones, in place of an untouched blank one, or, when it
/// is already open, by showing it. Quitting closes the window, which asks about every document
/// with unsaved changes.
pub fn poll(inbox: &Inbox, app: &mut WordApp, ctx: &egui::Context) {
    for e in inbox.drain() {
        match e {
            Event::Open(paths) => {
                for p in paths.iter().filter_map(|p| p.to_str()) {
                    match app.open_file(p, None) {
                        // Leave Backstage, as opening from the Open dialog does.
                        Ok(_) => app.ui.backstage = false,
                        Err(e) => {
                            app.report_open_failure(p, &e);
                            // Backstage hides the status bar, so leave it to show the error.
                            app.ui.backstage = false;
                        }
                    }
                }
            }
            // Quitting asks about every document with unsaved changes, each in its window; the
            // app closes once that's done (`quit_requested`).
            Event::Quit => {
                app.quit();
                ctx.request_repaint();
            }
        }
    }
}
