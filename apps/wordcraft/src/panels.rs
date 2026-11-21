//! The Paragraph and Tabs dialogs as Mac utility panels (`Services::paragraph_window`): the app shows each
//! in a window of its own, and this ties that window to the document window it was opened from,
//! as a child window. It moves with that window, stays just above it, goes behind another
//! document's window with it, and hides while WordCraft is in the background. It is not listed
//! in the Window menu.
//!
//! The one exception to the workspace's no-`unsafe` rule (AGENTS.md names this module): AppKit
//! marks `addChildWindow:ordered:` unsafe, and nothing else in these bindings ties two windows.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindow, NSWindowOrderingMode};
use wordcraft_ui_egui::i18n::t;

/// The translated titles of the windows the app shows as utility panels.
fn panel_titles() -> [&'static str; 2] {
    [t("Paragraph"), t("Tabs")]
}

/// Ties every panel that has no parent yet to the window just below it, the one it was opened
/// from. Call every frame; it does nothing once they are tied.
pub fn tie(mtm: MainThreadMarker) {
    let titles = panel_titles();
    // The match is exact: a document window's title always ends in " - WordCraft"
    // (`windows::title`), so a document named "Paragraph" ("Paragraph - WordCraft") never matches
    // the panel's bare translated title.
    let is_panel = |w: &NSWindow| titles.iter().any(|title| w.title().to_string() == *title);
    let ordered = NSApplication::sharedApplication(mtm).orderedWindows();
    let ordered: Vec<_> = ordered.iter().filter(|w| w.isVisible()).collect();
    for (i, panel) in ordered.iter().enumerate() {
        if !is_panel(panel) || panel.parentWindow().is_some() {
            continue;
        }
        let Some(parent) = ordered[i + 1..].iter().find(|w| !is_panel(w)) else {
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
