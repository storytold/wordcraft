//! The menu bar a system file panel gets, as AppKit sees it, on the main thread (`harness = false`):
//! ⌘X, ⌘C, ⌘V and ⌘A reach the panel's text boxes only through it, it shows only while a panel is
//! the key window, and the usual menu bar returns when the panel closes.

#[cfg(target_os = "macos")]
#[path = "../src/panel_menu.rs"]
mod panel_menu;

/// ⌘X, ⌘C, ⌘V and ⌘A each have an item in the panel's menu bar that sends the matching edit action,
/// ⌘H hides the app, and there is no Quit item.
#[cfg(target_os = "macos")]
fn assert_panel_shortcuts(app: &objc2_app_kit::NSApplication, usual: &objc2_app_kit::NSMenu) {
    use objc2::sel;

    let bar = app.mainMenu().expect("the panel's menu bar");
    assert!(!std::ptr::eq(&*bar, usual), "the panel has its own menu bar");
    let mut items = Vec::new();
    for menu in (0..bar.numberOfItems()).filter_map(|i| bar.itemAtIndex(i)?.submenu()) {
        items.extend((0..menu.numberOfItems()).filter_map(|i| menu.itemAtIndex(i)));
    }
    for (key, action) in [("x", sel!(cut:)), ("c", sel!(copy:)), ("v", sel!(paste:)), ("a", sel!(selectAll:)), ("h", sel!(hide:))] {
        assert!(
            items.iter().any(|item| item.keyEquivalent().to_string() == key && item.action() == Some(action)),
            "no menu item sends {action:?} on ⌘{key}"
        );
    }
    assert!(!items.iter().any(|item| item.action() == Some(sel!(terminate:))), "no Quit while a panel is up");
}

/// Whether `menu` is the menu bar now.
#[cfg(target_os = "macos")]
fn is_menu_bar(app: &objc2_app_kit::NSApplication, menu: &objc2_app_kit::NSMenu) -> bool {
    app.mainMenu().is_some_and(|bar| std::ptr::eq(&*bar, menu))
}

/// Shows and hides the panel's menu bar against stand-ins for the usual one.
#[cfg(target_os = "macos")]
fn main() {
    // cargo-nextest asks a test binary for its tests (`--list`) before running them.
    if std::env::args().any(|arg| arg == "--list") {
        if !std::env::args().any(|arg| arg == "--ignored") {
            println!("panel_shortcuts: test");
        }
        return;
    }
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSMenu};

    let mtm = MainThreadMarker::new().expect("harness = false runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.finishLaunching();
    let usual = NSMenu::new(mtm);
    app.setMainMenu(Some(&usual));
    let mut menu = panel_menu::PanelMenu::default();

    // A dialog is open but no panel is the key window (another document's window is): no swap.
    menu.sync(true);
    assert!(is_menu_bar(&app, &usual), "no panel menu bar while no panel is key");
    assert!(!panel_menu::in_file_panel(None));
    assert!(panel_menu::in_file_panel(Some(objc2_app_kit::NSSavePanel::savePanel(mtm).into_super().into_super())), "a Save panel");
    let colours = objc2_app_kit::NSColorPanel::sharedColorPanel(mtm);
    assert!(!panel_menu::in_file_panel(Some(colours.into_super().into_super())), "another kind of panel");

    menu.set_shown(&app, true);
    assert_panel_shortcuts(&app, &usual);
    menu.set_shown(&app, false);
    assert!(is_menu_bar(&app, &usual), "the usual menu bar is back");

    // The app installs a new menu bar while the panel's is up: closing the panel leaves it there.
    menu.set_shown(&app, true);
    let rebuilt = NSMenu::new(mtm);
    app.setMainMenu(Some(&rebuilt));
    menu.set_shown(&app, false);
    assert!(is_menu_bar(&app, &rebuilt), "a menu bar installed meanwhile stays");

    // Replaced while the panel is still key: the panel's comes back, and the new one after it.
    menu.set_shown(&app, true);
    app.setMainMenu(Some(&usual));
    menu.set_shown(&app, true);
    assert_panel_shortcuts(&app, &usual);
    menu.set_shown(&app, false);
    assert!(is_menu_bar(&app, &usual), "the menu bar installed meanwhile is the one put back");
    println!("mac_panel_menu_appkit: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
