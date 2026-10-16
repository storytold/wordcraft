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

#[test]
fn a_macro_add_in_brings_the_code_group_and_the_macros_bar() {
    let _g = serial();
    let m = wordcraft_plugin::Manifest::parse(
        r#"{"id": "t.ui-vba", "name": "VBA Test", "macros": {"language": "VBA"}, "run": {"command": "./not-started"}}"#,
    )
    .unwrap();
    wordcraft_plugin::add(m, std::env::temp_dir());
    struct Unload;
    impl Drop for Unload {
        fn drop(&mut self) {
            wordcraft_plugin::unload("t.ui-vba");
        }
    }
    let _unload = Unload;
    let mut doc = wordcraft_doc::Document::new();
    doc.passthrough.insert(wordcraft_docx::VBA_PROJECT_PART.to_string(), std::sync::Arc::new(b"project".to_vec()));
    let mut a = WordApp::new(Session::new(doc), Services::default());
    a.run("ui.tab", json!({"tab": "Developer"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    // A document with macros opens with them off, and the bar says so until Enable Content.
    assert!(h.query_by_label("Enable Content").is_some(), "the macros bar");
    h.get_by_label("Enable Content").click();
    h.run_steps(4);
    assert!(h.state().session.macros_enabled);
    assert!(h.query_by_label("Enable Content").is_none());
    // Developer › Code (its buttons open these dialogs: ribbon buttons are painted, not labelled).
    assert!(wordcraft_ui_egui::dialogs_developer::visible());
    h.state_mut().run("ui.dialog", json!({"name": "macroSecurity"})).unwrap();
    h.run_steps(4);
    h.get_by_label("Disable all macros").click();
    h.run_steps(4);
    assert_eq!(h.state().session.prefs.macro_security, "disable");
    // The Macros dialog says why the add-in can't start instead of hanging.
    h.state_mut().dialog = None;
    h.state_mut().run("ui.dialog", json!({"name": "vbaMacros"})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Macro name:").is_some());
    assert!(h.query_by_label("Add to Ribbon…").is_some(), "a macro goes on the ribbon from here");
    wordcraft_plugin::unload("t.ui-vba");
    assert!(!wordcraft_ui_egui::dialogs_developer::visible(), "no macro add-in, no Code group");
}

/// A web view host that records what it was asked and lets the test post a page's messages.
#[derive(Clone, Default)]
struct FakeWeb(std::sync::Arc<Mutex<Web>>);

#[derive(Default)]
struct Web {
    shown: Vec<(String, String)>,
    inbox: Vec<(String, String)>,
    replies: Vec<(String, String)>,
    hidden: Vec<String>,
    /// Pages loaded hidden: (key, url); calls into pages: (key, entry, json).
    hidden_pages: Vec<(String, String)>,
    calls: Vec<(String, String, String)>,
    closed: Vec<String>,
}

impl wordcraft_ui_egui::office_pane::OfficeHost for FakeWeb {
    fn show(&mut self, key: &str, url: &str, _rect: egui::Rect, _ppp: f32) {
        let mut w = self.0.lock().unwrap();
        if !w.shown.iter().any(|(k, _)| k == key) {
            w.shown.push((key.into(), url.into()));
        }
    }
    fn hide(&mut self, key: &str) {
        self.0.lock().unwrap().hidden.push(key.into());
    }
    fn close(&mut self, key: &str) {
        self.0.lock().unwrap().closed.push(key.into());
    }
    fn load_hidden(&mut self, key: &str, url: &str) {
        self.0.lock().unwrap().hidden_pages.push((key.into(), url.into()));
    }
    fn call(&mut self, key: &str, entry: &str, json: &str) {
        self.0.lock().unwrap().calls.push((key.into(), entry.into(), json.into()));
    }
    fn take_messages(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.0.lock().unwrap().inbox)
    }
    fn reply(&mut self, key: &str, json: &str) {
        self.0.lock().unwrap().replies.push((key.into(), json.into()));
    }
}

#[test]
fn an_office_add_ins_task_pane_reads_and_writes_the_document() {
    let _g = serial();
    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?>
<OfficeApp xmlns="http://schemas.microsoft.com/office/appforoffice/1.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="TaskPaneApp">
  <Id>0d5c7a1e-0000-4bcd-9eaf-0000000000bb</Id><Version>1.0.0.0</Version><ProviderName>Tests</ProviderName><DefaultLocale>en-US</DefaultLocale>
  <DisplayName DefaultValue="Pane Test"/><Description DefaultValue="Tests"/>
  <Hosts><Host Name="Document"/></Hosts>
  <DefaultSettings><SourceLocation DefaultValue="https://tests.example/pane.html"/></DefaultSettings>
  <Permissions>ReadWriteDocument</Permissions>
</OfficeApp>"#;
    let m = wordcraft_plugin::office::parse_manifest(manifest, wordcraft_plugin::HOST_ID).unwrap();
    let id = m.id.clone();
    wordcraft_plugin::add(m, std::env::temp_dir());
    let web = FakeWeb::default();
    let services = Services { office: Some(Box::new(web.clone())), ..Default::default() };
    let mut h = harness(WordApp::new(Session::new(wordcraft_doc::Document::from_text("Hello")), services));
    h.run_steps(2);
    h.state_mut().run("plugins.run", json!({"plugin": id, "command": "taskpane"})).unwrap();
    h.run_steps(3);
    let key = web.0.lock().unwrap().shown.first().cloned().map(|(k, url)| {
        assert_eq!(url, "https://tests.example/pane.html");
        k
    });
    let key = key.expect("the pane's page was shown");
    // One context.sync(): a paragraph at the end, then the body's text.
    let msg = json!({"id": 7, "actions": [
        {"op": "call", "path": [{"p": "document"}, {"p": "body"}], "method": "insertParagraph", "args": ["World", "End"]},
        {"op": "load", "path": [{"p": "document"}, {"p": "body"}], "props": ["text"]}
    ]});
    web.0.lock().unwrap().inbox.push((key.clone(), msg.to_string()));
    h.run_steps(2);
    let (to, reply) = web.0.lock().unwrap().replies.last().cloned().unwrap();
    assert_eq!(to, key);
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["results"][1]["text"], "Hello\nWorld", "{reply}");
    // A failing sync answers its error and changes nothing.
    let bad = json!({"id": 8, "actions": [{"op": "load", "path": [{"p": "nowhere"}], "props": ["text"]}]});
    web.0.lock().unwrap().inbox.push((key.clone(), bad.to_string()));
    h.run_steps(2);
    let (_, reply) = web.0.lock().unwrap().replies.last().cloned().unwrap();
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!((reply["id"].clone(), reply["error"]["code"].clone()), (json!(8), json!("InvalidArgument")), "{reply}");
    // The File tab hides the page (a native web view would cover it).
    h.state_mut().ui.backstage = true;
    h.run_steps(2);
    assert!(web.0.lock().unwrap().hidden.contains(&key));
    wordcraft_plugin::unload(&id);
}

#[test]
fn a_documents_ribbon_buttons_show_on_their_tabs_and_run_their_macros() {
    use wordcraft_doc::{Ribbon, RibbonButton, RibbonGroup, RibbonTab};
    // No macro add-in loaded meanwhile: the buttons show without one.
    let _g = serial();
    let button = |id: &str, label: &str| RibbonButton {
        id: id.into(),
        label: label.into(),
        on_action: format!("Module1.{label}"),
        large: true,
        ..Default::default()
    };
    let group = |b| vec![RibbonGroup { id: "g".into(), label: "Letters".into(), buttons: vec![b] }];
    let mut doc = wordcraft_doc::Document::new();
    doc.ribbon = Some(Ribbon {
        tabs: vec![
            RibbonTab { id: "tTools".into(), label: "Tools".into(), groups: group(button("b1", "Go")), ..Default::default() },
            RibbonTab { builtin: Some("TabReferences".into()), groups: group(button("b2", "Hello")), ..Default::default() },
        ],
    });
    let mut a = WordApp::new(Session::new(doc), Services::default());
    a.run("ui.tab", json!({"tab": "References"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    // On References, after its own groups.
    assert!(h.query_by_label("Hello").is_some(), "the document's button on References");
    assert!(h.query_by_label("Go").is_none());
    // Its own tab joins the tab row.
    h.state_mut().run("ui.tab", json!({"tab": "Tools"})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.tab, "Tools");
    h.get_by_label("Go").click();
    h.run_steps(4);
    // Macros start disabled: the click says so instead of running.
    let said = h.state().status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
    assert!(said.contains("disabled"), "{said}");
    // Developer › Ribbon Buttons (as the Macros window's Add to Ribbon opens it) adds one more,
    // saved in the document.
    h.state_mut().run("ui.dialog", json!({"name": "ribbonButtons", "macro": "Module1.Report"})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("ribbonButtons"));
    assert!(h.query_by_label("Tools › Letters › Go").is_some(), "the document's buttons are listed");
    h.get_by_label("Add").click();
    h.run_steps(4);
    let ribbon = h.state().session.doc.ribbon.clone().unwrap();
    let added = ribbon.tabs.iter().find(|t| t.label == "Macros").and_then(|t| t.groups.first()).and_then(|g| g.buttons.first()).cloned();
    assert_eq!(added.map(|b| (b.label, b.on_action, b.image)), Some(("Report".into(), "Module1.Report".into(), Some("MacroPlay".into()))));
    // Its tab joins the tab row: open it and the button is there; undo takes it away.
    h.state_mut().dialog = None;
    h.state_mut().run("ui.tab", json!({"tab": "Macros"})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Report").is_some());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Report").is_none());
}

/// Manifest 1.3: a blocked add-in's card says why and offers to install what it needs; its
/// ribbon group is absent until then; turning off an add-in another needs asks first.
#[test]
fn blocked_add_ins_say_why_and_offer_to_install_what_they_need() {
    let _g = serial();
    let m = wordcraft_plugin::Manifest::parse(
        r#"{"id": "t.ui-reports", "name": "Reports", "version": "1.4.0", "run": {"command": "./not-started"},
            "commands": [{"id": "go", "label": "Go"}],
            "ui": {"ribbon": [{"tab": "Review", "group": "R", "buttons": [{"command": "go", "label": "Reports Button"}]}]},
            "requires": {"addins": [
                {"id": "t.ui-vba", "version": "^0.1.4", "github": "sirmmo/craft-vba", "why": "Runs the report macros"},
                {"id": "t.ui-mqtt", "version": "*", "github": "sirmmo/craft-collab", "optional": true}]}}"#,
    )
    .unwrap();
    wordcraft_plugin::add(m, std::env::temp_dir());
    assert!(wordcraft_ui_egui::plugin_ui::groups("Review").is_empty(), "a blocked add-in's ribbon group is absent");
    let mut a = app();
    a.run("ui.backstage", json!({"value": true, "page": "options", "pane": "addins"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert!(h.query_by_label("Blocked · t.ui-reports").is_some());
    assert!(h.query_by_label("Needs craft-vba, which isn't installed").is_some());
    h.get_by_label("Install Missing").click();
    h.run_steps(3);
    let offer = h.state().plugin_install.offer.as_ref().expect("Install Missing offers the missing add-ins");
    assert_eq!(offer.plugin, "t.ui-reports");
    let ticked: Vec<(&str, bool)> = offer.items.iter().map(|(m, on)| (m.id.as_str(), *on)).collect();
    assert_eq!(ticked, [("t.ui-vba", true), ("t.ui-mqtt", false)], "optional add-ins are offered unticked");
    assert!(h.query_by_label("Reports needs craft-vba ^0.1.4 (Runs the report macros). Install it too?").is_some());
    assert!(h.query_by_label("Reports can use craft-collab *. Install it too?").is_some());
    h.get_by_label("Not Now").click();
    h.run_steps(3);
    assert!(h.state().plugin_install.offer.is_none());

    // Once the required add-in is there, the add-in unblocks; turning that one off asks first.
    let vba =
        wordcraft_plugin::Manifest::parse(r#"{"id": "t.ui-vba", "name": "craft-vba", "version": "0.1.5", "run": {"command": "./not-started"}}"#)
            .unwrap();
    wordcraft_plugin::add(vba, std::env::temp_dir());
    h.run_steps(3);
    assert!(h.query_by_label("Install Missing").is_none());
    assert_eq!(wordcraft_ui_egui::plugin_ui::groups("Review").len(), 1, "unblocked without a restart");
    let deactivate = h.query_all_by_label("Deactivate").last().expect("the second card's button");
    deactivate.click();
    h.run_steps(3);
    assert!(h.query_by_label("Reports needs craft-vba. Turn both off?").is_some());
    assert!(wordcraft_plugin::get("t.ui-vba").unwrap().enabled(), "nothing is turned off before the answer");
    h.get_by_label("Turn Both Off").click();
    h.run_steps(3);
    assert!(!wordcraft_plugin::get("t.ui-vba").unwrap().enabled());
    assert!(!wordcraft_plugin::get("t.ui-reports").unwrap().enabled());
    wordcraft_plugin::unload("t.ui-reports");
    wordcraft_plugin::unload("t.ui-vba");
}

#[test]
fn function_commands_run_in_their_page_and_are_given_up_on_if_they_never_complete() {
    let _g = serial();
    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?>
<OfficeApp xmlns="http://schemas.microsoft.com/office/appforoffice/1.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
  xmlns:bt="http://schemas.microsoft.com/office/officeappbasictypes/1.0" xsi:type="TaskPaneApp">
  <Id>0d5c7a1e-0000-4bcd-9eaf-0000000000cc</Id><Version>1.0.0.0</Version><ProviderName>Tests</ProviderName><DefaultLocale>en-US</DefaultLocale>
  <DisplayName DefaultValue="Function Test"/><Description DefaultValue="Tests"/>
  <Hosts><Host Name="Document"/></Hosts>
  <DefaultSettings><SourceLocation DefaultValue="https://tests.example/pane.html"/></DefaultSettings>
  <Permissions>ReadWriteDocument</Permissions>
  <VersionOverrides xmlns="http://schemas.microsoft.com/office/taskpaneappversionoverrides" xsi:type="VersionOverridesV1_0">
    <Hosts><Host xsi:type="Document"><DesktopFormFactor>
      <FunctionFile resid="Commands.Url"/>
      <ExtensionPoint xsi:type="PrimaryCommandSurface"><OfficeTab id="TabHome"><Group id="T.Group"><Label resid="L"/>
        <Control xsi:type="Button" id="T.Run"><Label resid="L"/><Action xsi:type="ExecuteFunction"><FunctionName>writeValue</FunctionName></Action></Control>
      </Group></OfficeTab></ExtensionPoint>
    </DesktopFormFactor></Host></Hosts>
    <Resources><bt:Urls><bt:Url id="Commands.Url" DefaultValue="https://tests.example/commands.html"/></bt:Urls>
      <bt:ShortStrings><bt:String id="L" DefaultValue="Run"/></bt:ShortStrings></Resources>
  </VersionOverrides>
</OfficeApp>"#;
    let m = wordcraft_plugin::office::parse_manifest(manifest, wordcraft_plugin::HOST_ID).unwrap();
    let id = m.id.clone();
    wordcraft_plugin::add(m, std::env::temp_dir());
    let web = FakeWeb::default();
    let services = Services { office: Some(Box::new(web.clone())), ..Default::default() };
    let mut h = harness(WordApp::new(Session::new(wordcraft_doc::Document::from_text("Hello")), services));
    h.run_steps(2);
    let run = |h: &mut Harness<'_, WordApp>| {
        h.state_mut().run("plugins.run", json!({"plugin": id, "command": "T.Run"})).unwrap();
        h.run_steps(2);
    };
    run(&mut h);
    let (key, url) = web.0.lock().unwrap().hidden_pages.last().cloned().expect("the function file was loaded");
    assert_eq!(url, "https://tests.example/commands.html");
    assert!(web.0.lock().unwrap().calls.is_empty(), "nothing is called before the page is ready");
    let sent = |w: &FakeWeb| {
        w.0.lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(_, e, _)| e == "__craftOfficeRunFunction")
            .map(|(k, _, j)| (k.clone(), j.clone()))
            .collect::<Vec<_>>()
    };
    // The page's library says it's ready: the call goes in, with the control's id.
    web.0.lock().unwrap().inbox.push((key.clone(), json!({"event": "ready"}).to_string()));
    h.run_steps(2);
    let calls = sent(&web);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, key);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&calls[0].1).unwrap(), json!({"call": 1, "name": "writeValue", "control": "T.Run"}));
    assert_eq!(h.state().office.calls.len(), 1, "waiting for event.completed()");
    // The page says the function completed: nothing waits any more.
    web.0.lock().unwrap().inbox.push((key.clone(), json!({"event": "functionDone", "call": 1}).to_string()));
    h.run_steps(2);
    assert!(h.state().office.calls.is_empty());
    assert!(web.0.lock().unwrap().replies.is_empty(), "a page's events aren't answered");
    // One that failed says why in the status bar; a ready page gets the call at once.
    run(&mut h);
    assert_eq!(sent(&web).len(), 2);
    web.0.lock().unwrap().inbox.push((key.clone(), json!({"event": "functionDone", "call": 2, "error": "no function `writeValue`"}).to_string()));
    h.run_steps(2);
    assert!(h.state().office.calls.is_empty());
    assert!(h.state().status_msg.as_ref().is_some_and(|(m, _)| m.contains("no function `writeValue`")), "{:?}", h.state().status_msg);
    // Ready pages hear of selection changes, once per change.
    let heard = |w: &FakeWeb| w.0.lock().unwrap().calls.iter().filter(|(k, e, _)| *k == key && e == "__craftOfficeEvent").count();
    h.state_mut().run("select.all", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(heard(&web), 1);
    h.run_steps(2);
    assert_eq!(heard(&web), 1, "once per change");
    // One that never completes is given up on, and its page closed.
    run(&mut h);
    assert_eq!(h.state().office.calls.len(), 1);
    wordcraft_ui_egui::office_pane::pump(h.state_mut(), 1.0e6);
    assert!(h.state().office.calls.is_empty());
    assert!(web.0.lock().unwrap().closed.contains(&key));
    assert!(!h.state().office.ready.contains(&key));
    assert!(h.state().status_msg.as_ref().is_some_and(|(m, _)| m.contains("Function Test")), "{:?}", h.state().status_msg);
    wordcraft_plugin::unload(&id);
}
