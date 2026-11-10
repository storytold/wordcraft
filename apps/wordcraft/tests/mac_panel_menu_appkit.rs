//! The menu bar a system file panel gets, as AppKit sees it, on the main thread (`harness = false`):
//! ⌘X, ⌘C, ⌘V and ⌘A reach the panel's text boxes only through it, and the usual menu bar returns
//! when the panel closes.

#[cfg(target_os = "macos")]
#[path = "../src/panel_menu.rs"]
mod panel_menu;

#[cfg(target_os = "macos")]
fn assert_panel_shortcuts(app: &objc2_app_kit::NSApplication, usual: &objc2_app_kit::NSMenu) {
    use objc2::sel;

    let bar = app.mainMenu().expect("the panel's menu bar");
    assert!(!std::ptr::eq(&*bar, usual), "the panel has its own menu bar");
    let mut items = Vec::new();
    for menu in (0..bar.numberOfItems()).filter_map(|i| bar.itemAtIndex(i)?.submenu()) {
        items.extend((0..menu.numberOfItems()).filter_map(|i| menu.itemAtIndex(i)));
    }
    for (key, action) in [("x", sel!(cut:)), ("c", sel!(copy:)), ("v", sel!(paste:)), ("a", sel!(selectAll:))] {
        assert!(
            items.iter().any(|item| item.keyEquivalent().to_string() == key && item.action() == Some(action)),
            "no menu item sends {action:?} on ⌘{key}"
        );
    }
}

#[cfg(target_os = "macos")]
fn main() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSMenu};

    let mtm = MainThreadMarker::new().expect("harness = false runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.finishLaunching();
    let usual = NSMenu::new(mtm);
    app.setMainMenu(Some(&usual));

    let mut menu = panel_menu::PanelMenu::default();
    menu.sync(true);
    assert_panel_shortcuts(&app, &usual);
    menu.sync(false);

    let back = app.mainMenu().expect("the usual menu bar");
    assert!(std::ptr::eq(&*back, &*usual), "the usual menu bar is back");
    println!("mac_panel_menu_appkit: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
