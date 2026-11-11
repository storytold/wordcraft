//! macOS: Cut, Copy, Paste and Select All shortcuts in the system file panels.
//!
//! AppKit finds these shortcuts through the menu bar, and WordCraft's menu bar has no Edit menu, so
//! the text boxes of an Open or Save panel (Go to Folder, the file name) ignored ⌘V. While the key
//! window is a panel or a sheet on one (Go to Folder, New Folder), this small menu bar replaces the
//! usual one. It goes away when another document's window becomes key or the panel closes, because
//! WordCraft's own text handling gets these keys from egui. Its application menu has Hide, Hide
//! Others and Show All; Quit is left out until the panel closes.

use muda::{Menu, PredefinedMenuItem, Submenu};
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{ClassType, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSMenu, NSSavePanel, NSWindow};

/// The panel's menu bar as AppKit holds it, and the one to put back. The `Menu` owns its items.
struct Shown {
    _menu: Menu,
    ours: Option<Retained<NSMenu>>,
    before: Option<Retained<NSMenu>>,
}

/// Swaps the menu bar while a file panel is the key window.
#[derive(Default)]
pub struct PanelMenu(Option<Shown>);

impl PanelMenu {
    /// While `panel_open`, show the panel's menu bar when the key window is in an Open or Save
    /// panel; otherwise put the usual one back. Call again whenever either may have changed.
    pub fn sync(&mut self, panel_open: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        self.set_shown(&app, panel_open && in_file_panel(app.keyWindow()));
    }

    /// Show the panel's menu bar or put the usual one back. A menu bar someone else installed
    /// since ours stays: it becomes the one to put back, or is left alone.
    pub fn set_shown(&mut self, app: &NSApplication, panel: bool) {
        let current = app.mainMenu();
        let ours = |shown: &Shown| same(&current, &shown.ours);
        match (panel, self.0.take()) {
            (true, Some(shown)) if ours(&shown) => self.0 = Some(shown),
            (true, _) => {
                if let Some(menu) = edit_menu() {
                    menu.init_for_nsapp();
                    self.0 = Some(Shown { _menu: menu, ours: app.mainMenu(), before: current });
                }
            }
            (false, Some(shown)) if ours(&shown) => app.setMainMenu(shown.before.as_deref()),
            (false, _) => {}
        }
    }
}

/// Whether `window` is an Open or Save panel, or a window on one: Go to Folder and New Folder are
/// sheets on the panel, in a window of their own.
pub fn in_file_panel(mut window: Option<Retained<NSWindow>>) -> bool {
    // A panel, its sheet and the document window under it: a few steps at most.
    for _ in 0..8 {
        let Some(w) = window else { return false };
        if w.isKindOfClass(NSSavePanel::class()) {
            return true;
        }
        window = w.sheetParent().or_else(|| w.parentWindow());
    }
    false
}

/// Whether two menu bars are the same AppKit object.
fn same(a: &Option<Retained<NSMenu>>, b: &Option<Retained<NSMenu>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => std::ptr::eq(&**a, &**b),
        _ => false,
    }
}

/// AppKit treats a menu bar's first menu as the application menu, so Edit comes second.
fn edit_menu() -> Option<Menu> {
    let menu = Menu::new();
    let app = Submenu::new("WordCraft", true);
    app.append_items(&[&PredefinedMenuItem::hide(None), &PredefinedMenuItem::hide_others(None), &PredefinedMenuItem::show_all(None)]).ok()?;
    let edit = Submenu::new("Edit", true);
    edit.append_items(&[
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
    ])
    .ok()?;
    menu.append_items(&[&app, &edit]).ok()?;
    Some(menu)
}
