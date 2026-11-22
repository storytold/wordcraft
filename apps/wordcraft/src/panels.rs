//! The Paragraph dialog (with its Tabs page) as a Mac utility panel (`Services::paragraph_window`): the app shows it
//! in a window of its own, and this ties that window to the document window it was opened from,
//! as a child window. It moves with that window, stays just above it, goes behind another
//! document's window with it, and hides while WordCraft is in the background. It is not listed
//! in the Window menu. Each document window may have its own panels open, but only the frontmost
//! document's show: the others are hidden until their document comes to the front.
//!
//! The one exception to the workspace's no-`unsafe` rule (AGENTS.md names this module): AppKit
//! marks `addChildWindow:ordered:` unsafe, and nothing else in these bindings ties two windows.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindow, NSWindowOrderingMode};
use wordcraft_ui_egui::i18n::t;

/// The translated titles of the windows the app shows as utility panels.
fn panel_titles() -> [&'static str; 1] {
    [t("Paragraph")]
}

/// Ties every new panel to the frontmost document window, the one it was opened from, and shows
/// only that document's panels. Call every frame.
pub fn tie(mtm: MainThreadMarker) {
    let titles = panel_titles();
    // The match is exact: a document window's title always ends in " - WordCraft"
    // (`windows::title`), so a document named "Paragraph" ("Paragraph - WordCraft") never matches
    // the panel's bare translated title.
    let is_panel = |w: &NSWindow| titles.iter().any(|title| w.title().to_string() == *title);
    let app = NSApplication::sharedApplication(mtm);
    // AppKit leaves panels out of `orderedWindows`, which lists the documents front to back: the
    // first visible one is the frontmost document. The panels are found among all the windows.
    let front = app.orderedWindows().iter().find(|w| w.isVisible() && !is_panel(w));
    for panel in app.windows().iter().filter(|w| is_panel(w) && w.isVisible()) {
        // A new panel was opened from the frontmost document.
        let parent = match (panel.parentWindow(), &front) {
            (Some(parent), _) => parent,
            (None, Some(front)) => {
                panel.setHidesOnDeactivate(true);
                panel.setExcludedFromWindowsMenu(true);
                add_child(front, &panel);
                front.clone()
            }
            (None, None) => continue,
        };
        // Only the frontmost document's panels show, and none while WordCraft is in the background:
        // `hidesOnDeactivate` alone does not keep a child panel hidden.
        let shown = app.isActive() && front.as_ref().is_some_and(|front| front.windowNumber() == parent.windowNumber());
        if (panel.alphaValue() > 0.0) != shown {
            panel.setAlphaValue(if shown { 1.0 } else { 0.0 });
            panel.setIgnoresMouseEvents(!shown);
        }
    }
}

#[allow(unsafe_code)]
fn add_child(parent: &NSWindow, child: &NSWindow) {
    // SAFETY: both are live windows of this app on the main thread (`MainThreadMarker`), and the
    // child is not already in a window hierarchy (`parentWindow` is nil): the documented
    // preconditions. AppKit keeps the child alive while it is attached.
    unsafe { parent.addChildWindow_ordered(child, NSWindowOrderingMode::Above) };
}
