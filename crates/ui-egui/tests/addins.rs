//! Add-ins in the interface: their ribbon tab and groups, the right-click menu items, File ›
//! Options › Add-ins (turning one off and on, installing from GitHub up to the confirmation) and
//! the Add-ins dialog. Its own test binary: add-ins are process-wide, and the other interface
//! tests shouldn't see this one.

use std::sync::{Mutex, MutexGuard};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

const ID: &str = "org.wordcraft.ui-test";

fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// An add-in whose program is never started here: a tab of its own, a group on Review and menu
/// items.
fn add_plugin() {
    let manifest = json!({
        "id": ID, "name": "UI Test Add-in", "version": "1.0", "author": "Tests", "homepage": "https://example.org/addin",
        "description": "Tidies the selection.",
        "run": {"command": "./not-started"},
        "commands": [{"id": "tidy", "label": "Tidy Up", "description": "Tidies the selection"}, {"id": "count", "label": "Count"}],
        "uiFor": {"wordcraft": {
            "ribbon": [
                {"tab": "Tidy", "group": "Cleaning", "buttons": [{"command": "tidy", "icon": "clearFormatting"}, {"command": "count", "size": "small"}]},
                {"tab": "Review", "group": "Tidy", "buttons": [{"command": "count", "label": "Count Here"}]}
            ],
            "menus": [{"command": "tidy"}, {"menu": "table", "command": "count", "label": "Count Cells"}]
        }},
        "ui": {"menus": [{"menu": "cell", "command": "tidy"}]}
    });
    let m = wordcraft_plugin::Manifest::parse(&manifest.to_string()).unwrap();
    wordcraft_plugin::add(m, std::env::temp_dir());
}

fn harness(app: WordApp) -> Harness<'static, WordApp> {
    Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

fn app() -> WordApp {
    WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
}

#[test]
fn add_ins_bring_ribbon_tabs_groups_and_menu_items() {
    let _g = serial();
    add_plugin();
    let mut a = app();
    a.run("ui.tab", json!({"tab": "Tidy"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert_eq!(h.state().ui.tab, "Tidy", "the add-in's own tab is a tab");
    assert!(h.query_by_label("Tidy Up").is_some(), "its large button");
    assert!(h.query_by_label("Count").is_some(), "its small button");
    h.state_mut().run("ui.tab", json!({"tab": "Review"})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Count Here").is_some(), "a group at the end of a built-in tab");
    // The right-click menu: text items always, table items in a table; the plain `ui` (another
    // app's cell menu) isn't used because `uiFor.wordcraft` is there.
    let labels = |t: bool, p: bool| wordcraft_ui_egui::plugin_ui::menu_items(t, p).into_iter().map(|i| i.label).collect::<Vec<_>>();
    assert_eq!(labels(false, false), ["Tidy Up"]);
    assert_eq!(labels(true, false), ["Tidy Up", "Count Cells"]);
    // Turned off, all of it goes.
    wordcraft_plugin::set_enabled(ID, false).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Count Here").is_none());
    assert!(labels(true, true).is_empty());
    assert!(wordcraft_ui_egui::plugin_ui::extra_tabs().is_empty());
    wordcraft_plugin::unload(ID);
}

#[test]
fn options_lists_add_ins_turns_them_off_and_on_and_asks_before_installing() {
    let _g = serial();
    add_plugin();
    let mut a = app();
    a.run("ui.backstage", json!({"value": true, "page": "options", "pane": "addins"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert!(h.query_by_label("UI Test Add-in 1.0").is_some(), "the add-in's card");
    assert!(h.query_by_label("Documentation").is_some(), "a link to its web page");
    h.get_by_label("Deactivate").click();
    h.run_steps(3);
    assert!(!wordcraft_plugin::get(ID).unwrap().enabled());
    assert_eq!(h.state().session.prefs.disabled_plugins, [ID.to_string()], "remembered for the next start");
    h.get_by_label("Activate").click();
    h.run_steps(3);
    assert!(wordcraft_plugin::get(ID).unwrap().enabled());
    assert!(h.state().session.prefs.disabled_plugins.is_empty());

    // Installing: a bad address is an error; a good one asks first and Cancel stops there.
    h.state_mut().plugin_install.url = "https://gitlab.com/acme/tools".into();
    h.run_steps(2);
    h.get_by_label("Install").click();
    h.run_steps(3);
    assert!(matches!(h.state().plugin_install.outcome, Some(Err(_))));
    h.state_mut().plugin_install.url = "acme/tools".into();
    h.run_steps(2);
    h.get_by_label("Install").click();
    h.run_steps(3);
    assert_eq!(h.state().plugin_install.confirming.as_deref(), Some("github.com/acme/tools"));
    assert!(h.query_by_label("Install and run github.com/acme/tools?").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().plugin_install.confirming.is_none());
    assert!(!h.state().plugin_install.busy(), "nothing was downloaded");
    wordcraft_plugin::unload(ID);
}

#[test]
fn the_add_ins_dialog_lists_commands_and_unloads() {
    let _g = serial();
    add_plugin();
    let mut a = app();
    // What Insert › Add-ins › Get Add-ins and Developer › Add-ins run.
    a.run("ui.dialog", json!({"name": "addIns"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("addIns"));
    assert!(h.query_by_label("UI Test Add-in 1.0").is_some());
    assert!(h.query_by_label("Tidy Up").is_some(), "a button per command");
    h.get_by_label("Unload").click();
    h.run_steps(4);
    assert!(wordcraft_plugin::get(ID).is_none());
    assert!(h.query_by_label("No add-ins are loaded.").is_some());
}
