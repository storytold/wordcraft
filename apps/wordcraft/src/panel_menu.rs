//! macOS: Cut, Copy, Paste and Select All shortcuts in the system file panels.
//!
//! AppKit finds these shortcuts through the menu bar, and WordCraft's menu bar has no Edit menu, so
//! the text boxes of an Open or Save panel (Go to Folder, the file name) ignored ⌘V. While a panel
//! is showing, this small menu bar replaces the usual one. It goes away with the panel because
//! WordCraft's own text handling gets these keys from egui.

use muda::{Menu, PredefinedMenuItem, Submenu};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSMenu};

/// The panel's menu bar, and the one to put back. The `Menu` owns the AppKit items on show.
struct Shown {
    _menu: Menu,
    before: Option<Retained<NSMenu>>,
}

/// Swaps the menu bar while a file panel is showing.
#[derive(Default)]
pub struct PanelMenu(Option<Shown>);

impl PanelMenu {
    /// Show the panel's menu bar when `panel_open`, else put the usual one back.
    pub fn sync(&mut self, panel_open: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        match (panel_open, self.0.take()) {
            (true, None) => {
                if let Some(menu) = edit_menu() {
                    let before = app.mainMenu();
                    menu.init_for_nsapp();
                    self.0 = Some(Shown { _menu: menu, before });
                }
            }
            (false, Some(shown)) => app.setMainMenu(shown.before.as_deref()),
            (_, current) => self.0 = current,
        }
    }
}

/// AppKit treats a menu bar's first menu as the application menu, so Edit comes second.
fn edit_menu() -> Option<Menu> {
    let menu = Menu::new();
    let app = Submenu::new("WordCraft", true);
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
