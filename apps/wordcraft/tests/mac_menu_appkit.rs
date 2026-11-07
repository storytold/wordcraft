//! The macOS menu bar as AppKit sees it, on the main thread (`harness = false`): install it through
//! the real muda backend (`src/native_menu.rs`) and read NSApp back. The Window menu must be NSApp's
//! `windowsMenu`: that is where macOS adds Move & Resize, Fill and Center, the targets of the system
//! window tiling shortcuts (Fn+Ctrl+arrows, Fn+F), and lists the open windows.

#[cfg(target_os = "macos")]
#[path = "../src/native_menu.rs"]
#[allow(unused_imports, dead_code)]
mod native_menu;

#[cfg(target_os = "macos")]
fn main() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let mtm = MainThreadMarker::new().expect("harness = false runs on the main thread");
    let nsapp = NSApplication::sharedApplication(mtm);
    nsapp.finishLaunching();

    let ctx = egui::Context::default();
    let mut app =
        wordcraft_ui_egui::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), wordcraft_ui_egui::Services::default());
    let mut menu = None;
    native_menu::NativeMenu::frame(&mut menu, &mut app, &ctx);

    let bar = nsapp.mainMenu().expect("a main menu");
    let tops: Vec<String> = (0..bar.numberOfItems()).filter_map(|i| bar.itemAtIndex(i)?.submenu()).map(|m| m.title().to_string()).collect();
    assert!(tops.starts_with(&["WordCraft".into(), "File".into(), "Edit".into(), "View".into(), "Go".into(), "Insert".into()]));
    assert_eq!(nsapp.windowsMenu().map(|m| m.title().to_string()).as_deref(), Some("Window"), "AppKit knows the Window menu");

    // Read the actual AppKit key equivalents, not just our menu description.
    use objc2::rc::Retained;
    use objc2_app_kit::{NSEventModifierFlags, NSMenu, NSMenuItem};
    fn submenu(bar: &NSMenu, name: &str) -> Retained<NSMenu> {
        (0..bar.numberOfItems()).filter_map(|i| bar.itemAtIndex(i)?.submenu()).find(|m| m.title().to_string() == name).unwrap()
    }
    fn item(menu: &NSMenu, name: &str) -> (isize, Retained<NSMenuItem>) {
        (0..menu.numberOfItems()).filter_map(|i| Some((i, menu.itemAtIndex(i)?))).find(|(_, m)| m.title().to_string() == name).unwrap()
    }
    let application = submenu(&bar, "WordCraft");
    let edit = submenu(&bar, "Edit");
    let file = submenu(&bar, "File");
    let window = submenu(&bar, "Window");
    for (menu, title, key) in [
        (&application, "Settings…", ","),
        (&application, "Hide WordCraft", "h"),
        (&application, "Quit WordCraft", "q"),
        (&window, "Minimize", "m"),
        (&edit, "Undo", "z"),
        (&edit, "Cut", "x"),
        (&edit, "Copy", "c"),
        (&edit, "Paste", "v"),
        (&edit, "Select All", "a"),
    ] {
        let (_, entry) = item(menu, title);
        assert_eq!(entry.keyEquivalent().to_string(), key, "{title}");
        assert!(entry.keyEquivalentModifierMask().contains(NSEventModifierFlags::Command), "{title}");
    }
    let (_, redo) = item(&edit, "Redo");
    assert_eq!(redo.keyEquivalent().to_string().to_lowercase(), "z");
    assert!(redo.keyEquivalentModifierMask().contains(NSEventModifierFlags::Shift));
    for key in ["n", "o", "s", "w", "p"] {
        assert!((0..file.numberOfItems()).filter_map(|i| file.itemAtIndex(i)).any(|m| m.keyEquivalent().to_string() == key), "File shortcut {key}");
    }
    // Native clicks enter the event queue and take the same guarded quit path as the window.
    app.run("text.insert", serde_json::json!({"text":"Keep this writing"})).unwrap();
    let (quit, _) = item(&application, "Quit WordCraft");
    application.performActionForItemAtIndex(quit);
    native_menu::NativeMenu::frame(&mut menu, &mut app, &ctx);
    assert!(!app.quit_requested, "dirty documents must not quit immediately");
    assert!(app.modal_open(), "Quit shows Save Changes");
    assert!(!item(&application, "Settings…").1.isEnabled(), "Settings cannot replace the save prompt");
    assert!(item(&edit, "Copy").1.isEnabled(), "dialog text editing remains available");
    app.run("ui.saveChanges", serde_json::json!({"answer":"cancel"})).unwrap();
    native_menu::NativeMenu::frame(&mut menu, &mut app, &ctx);
    assert!(!app.quit_requested);
    assert!(app.session.dirty);
    assert!(item(&application, "Settings…").1.isEnabled());

    // Exercise the actual native Select All / Cut handlers with an egui text field focused.
    // The document behind it must survive unchanged.
    let original = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    let mut field = String::from("Search terms");
    app.canvas.focused = false;
    let field_id = egui::Id::unique("native-menu-test-field");
    ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.add(egui::TextEdit::singleline(&mut field).id(field_id)).request_focus();
    })
    .drop_without_applying_deltas();
    for title in ["Select All", "Cut", "Undo", "Redo"] {
        let (index, _) = item(&edit, title);
        edit.performActionForItemAtIndex(index);
        native_menu::NativeMenu::frame(&mut menu, &mut app, &ctx);
        let events = std::mem::take(&mut menu.as_mut().unwrap().input);
        assert!(!events.is_empty(), "{title} reaches the focused field");
        let mut raw = egui::RawInput::default();
        app.route_input(&ctx, &mut raw, events);
        ctx.run_ui(raw, |ui| {
            ui.add(egui::TextEdit::singleline(&mut field).id(field_id));
        })
        .drop_without_applying_deltas();
        if title == "Undo" {
            assert_eq!(field, "Search terms", "Undo targets the focused field");
        }
    }
    assert!(field.is_empty(), "Cut changes the focused field");
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), original);

    println!("mac_menu_appkit: menus, shortcuts, input routing, and guarded Quit passed");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
