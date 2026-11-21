//! Find and Replace as a Mac utility panel (`Services::find_window`): the app shows it in a window
//! of its own, and this ties that window to the document window it was opened from, as a child
//! window. It moves with that window, stays just above it, goes behind another document's window
//! with it, and hides while WordCraft is in the background. It is not listed in the Window menu.
//!
//! The one exception to the workspace's no-`unsafe` rule (AGENTS.md): AppKit marks
//! `addChildWindow:ordered:` unsafe, and nothing else in these bindings ties two windows.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindow, NSWindowOrderingMode};
use wordcraft_ui_egui::i18n::t;

/// Ties every Find and Replace window that has no parent yet to the window just below it, the one
/// it was opened from. Call every frame; it does nothing once they are tied.
pub fn tie(mtm: MainThreadMarker) {
    let title = t("Find and Replace");
    let ordered = NSApplication::sharedApplication(mtm).orderedWindows();
    let ordered: Vec<_> = ordered.iter().filter(|w| w.isVisible()).collect();
    for (i, panel) in ordered.iter().enumerate() {
        if panel.title().to_string() != title || panel.parentWindow().is_some() {
            continue;
        }
        let Some(parent) = ordered[i + 1..].iter().find(|w| w.title().to_string() != title) else {
            continue;
        };
        panel.setHidesOnDeactivate(true);
        panel.setExcludedFromWindowsMenu(true);
        add_child(parent, panel);
    }
}

#[allow(unsafe_code)]
fn add_child(parent: &NSWindow, child: &NSWindow) {
    // SAFETY: both are live windows of this app on the main thread (`MainThreadMarker`), and the
    // child is not already in a window hierarchy (`parentWindow` is nil): the documented
    // preconditions. AppKit keeps the child alive while it is attached.
    unsafe { parent.addChildWindow_ordered(child, NSWindowOrderingMode::Above) };
}
