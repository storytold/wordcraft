//! Plug-in runs against a real session, with modules assembled from WebAssembly text.

use serde_json::{Value, json};
use wordcraft_doc::Document;

use crate::Session;
use crate::plugin::craft_plugin::bundle::Bundle;

/// A module that calls `calls` (command, params JSON) in order. A refused or failed call fails the
/// run when `strict`; otherwise its message becomes the output and the run goes on. `tail` is WAT
/// run after the calls (a trap, an endless loop), before returning 0.
fn module(calls: &[(&str, String)], strict: bool, tail: &str) -> Vec<u8> {
    let mut data = String::new();
    let mut body = String::new();
    let mut at = 0usize;
    for (command, params) in calls {
        let (c, p) = (at, at + command.len());
        at = p + params.len();
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        data.push_str(&format!(r#"(data (i32.const {c}) "{}") (data (i32.const {p}) "{}")"#, esc(command), esc(params)));
        let on_error = if strict {
            "(return (i32.const 1))".to_string()
        } else {
            "(call $set_output (i32.const 8192) (call $reply_read (i32.const 8192) (call $reply_len)))".to_string()
        };
        body.push_str(&format!(
            "(if (i32.ne (call $call (i32.const {c}) (i32.const {}) (i32.const {p}) (i32.const {})) (i32.const 0)) (then {on_error}))\n",
            command.len(),
            params.len()
        ));
    }
    let wat = format!(
        r#"(module
          (import "craft" "call" (func $call (param i32 i32 i32 i32) (result i32)))
          (import "craft" "reply_len" (func $reply_len (result i32)))
          (import "craft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "craft" "set_output" (func $set_output (param i32 i32)))
          (memory (export "memory") 1)
          {data}
          (func (export "craft_run") (param $n i32) (result i32) {body} {tail} (i32.const 0)))"#
    );
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

/// The `.craftplugin` of plug-in `id` for WordCraft, asking for `access`.
fn bundle(id: &str, access: &str, wasm: Vec<u8>) -> Vec<u8> {
    let manifest = json!({
        "id": id, "name": "Test Plug-in", "version": "1.0.0", "main": "plugin.wasm",
        "apps": {"wordcraft": {"api": 6}},
        "permissions": {"access": access},
        "commands": [{"id": "go", "label": "Go"}]
    });
    Bundle { manifest: manifest.to_string(), wasm, assets: vec![] }.encode()
}

fn data(bytes: &[u8]) -> String {
    crate::cmd::insert::base64_encode(bytes)
}

fn session() -> Session {
    Session::new(Document::from_text("Hello world"))
}

/// A session with plug-in `id` installed for the session (no store, no network).
fn with_plugin(id: &str, access: &str, wasm: Vec<u8>) -> Session {
    let mut s = session();
    let r = s.run("plugin.install", &json!({"data": data(&bundle(id, access, wasm)), "name": "test.craftplugin"})).unwrap();
    assert_eq!(r["installed"], true);
    assert_eq!(r["stored"], false, "without a store, a plug-in is kept for the session");
    s
}

fn text(s: &mut Session) -> String {
    s.run("document.text", &json!({})).unwrap()["text"].as_str().unwrap().to_string()
}

fn call(command: &str, params: Value) -> (&str, String) {
    (command, params.to_string())
}

#[test]
fn every_allowed_command_exists_and_none_is_always_refused() {
    let reg = crate::cmd::registry();
    for id in super::ALLOWED {
        assert!(reg.get(id).is_some(), "{id} isn't a command");
        assert!(!super::craft_plugin::ALWAYS_DENIED.iter().any(|p| super::craft_plugin::permissions::pattern_matches(p, id)), "{id}");
        assert!(!id.starts_with("plugin.") && !id.starts_with("file.") && !id.starts_with("mailings.") && !id.starts_with("view."), "{id}");
    }
    for id in reg.all().iter().map(|c| c.id).filter(|id| !super::ALLOWED.contains(id)) {
        assert!(super::policy().refuses(id), "{id} is refused unless it is allowed");
    }
}

#[test]
fn a_write_plugin_runs_real_commands_as_one_undo_step_named_after_it() {
    let wasm = module(
        &[call("caret.docEnd", json!({})), call("text.insert", json!({"text": "!"})), call("select.all", json!({})), call("format.bold", json!({}))],
        true,
        "",
    );
    let mut s = with_plugin("io.example.shout", "write", wasm);
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Oh, "})).unwrap();
    let steps = s.undo_depth();

    let r = s.run("plugin.run", &json!({"id": "io.example.shout"})).unwrap();

    assert_eq!(r["calls"], 4, "{r}");
    assert_eq!(text(&mut s), "Oh, Hello world!");
    assert_eq!(s.run("format.state", &json!({})).unwrap()["bold"], true);
    assert_eq!(s.undo_depth(), steps + 1, "four commands, one undo step");
    assert_eq!(s.undo_label(), Some("Test Plug-in"));
    assert!(!s.typing_open(), "typing after the run is a step of its own");

    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&mut s), "Oh, Hello world", "one undo takes the whole run back");
    assert_eq!(s.undo_depth(), steps, "the document's own steps are still there");
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(text(&mut s), "Oh, Hello world!");
}

#[test]
fn a_run_that_changes_nothing_records_no_step_and_keeps_redo() {
    let mut s = with_plugin("io.example.reader", "write", module(&[call("document.text", json!({}))], true, ""));
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    let steps = s.undo_depth();

    s.run("plugin.run", &json!({"id": "io.example.reader"})).unwrap();

    assert_eq!(s.undo_depth(), steps);
    assert!(s.can_redo(), "redo survives a run that edited nothing");
}

#[test]
fn a_read_plugin_reads_and_its_edits_are_refused() {
    let wasm = module(&[call("document.text", json!({})), call("text.insert", json!({"text": "sneaky"}))], false, "");
    let mut s = with_plugin("io.example.peek", "read", wasm);
    let steps = s.undo_depth();

    let r = s.run("plugin.run", &json!({"id": "io.example.peek"})).unwrap();

    let message = r["output"].as_str().unwrap();
    assert!(message.contains("may only read"), "{message}");
    assert_eq!(text(&mut s), "Hello world", "nothing was typed");
    assert_eq!(s.undo_depth(), steps, "nothing to undo");
    assert!(!s.can_redo(), "and nothing to redo");
}

#[test]
fn a_read_plugins_call_that_changed_the_document_is_put_back() {
    // The registry's flag decides up front; whatever a call changed anyway is put back after it.
    let mut s = session();
    let mark = s.call_mark();
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    assert!(s.changed_since(&mark));
    s.put_back(mark);
    assert_eq!(text(&mut s), "Hello world");
    assert!(!s.can_undo() && !s.can_redo());
}

#[test]
fn refused_commands_and_file_paths_are_refused() {
    for (command, params, why) in [
        ("file.save", json!({"path": "/tmp/x.docx"}), "isn't available to plugins"),
        ("file.open", json!({"path": "/etc/hosts"}), "isn't available to plugins"),
        ("file.exportPdf", json!({}), "isn't available to plugins"),
        ("edit.undo", json!({}), "isn't available to plugins"),
        ("edit.paste", json!({}), "isn't available to plugins"),
        ("edit.copy", json!({}), "isn't available to plugins"),
        ("insert.picture", json!({"path": "/etc/hosts"}), "isn't available to plugins"),
        ("insert.textFromFile", json!({"path": "/etc/hosts"}), "isn't available to plugins"),
        ("insert.object", json!({}), "isn't available to plugins"),
        ("mailings.recipients", json!({"path": "/tmp/r.csv"}), "isn't available to plugins"),
        ("tools.attachTemplate", json!({"path": "/tmp/t.dotx"}), "isn't available to plugins"),
        ("tools.macros", json!({"run": "x"}), "isn't available to plugins"),
        ("tools.recordMacro", json!({}), "isn't available to plugins"),
        ("review.addToDictionary", json!({"word": "x"}), "isn't available to plugins"),
        ("review.restrict", json!({}), "isn't available to plugins"),
        ("view.zoom", json!({"value": 200}), "isn't available to plugins"),
        ("plugin.list", json!({}), "isn't available to plugins"),
        ("text.insert", json!({"text": "x", "nested": {"file": "/etc/hosts"}}), "can't name files"),
    ] {
        let wasm = module(&[call(command, params)], false, "");
        let mut s = with_plugin("io.example.sneaky", "write", wasm);
        let steps = s.undo_depth();
        let r = s.run("plugin.run", &json!({"id": "io.example.sneaky"})).unwrap();
        let message = r["output"].as_str().unwrap_or_default();
        assert!(message.contains(why), "{command}: {message}");
        assert_eq!((text(&mut s).as_str(), s.undo_depth()), ("Hello world", steps), "{command}");
    }
}

#[test]
fn a_call_that_opens_a_dialog_is_refused() {
    // Page Setup without a section asks the window for its dialog.
    let mut s = with_plugin("io.example.dialog", "write", module(&[call("layout.pageSetup", json!({}))], false, ""));
    let requests = s.ui_requests.clone();
    let r = s.run("plugin.run", &json!({"id": "io.example.dialog"})).unwrap();
    assert!(r["output"].as_str().unwrap_or_default().contains("opens a dialog"), "{r}");
    assert_eq!(s.ui_requests, requests, "no dialog is asked for");
}

#[test]
fn a_failing_plugin_leaves_the_document_and_history_unchanged() {
    for (name, tail) in [("trap", "(unreachable)"), ("fuel", "(loop $l (br $l))"), ("status", "(return (i32.const 3))")] {
        let wasm = module(&[call("text.insert", json!({"text": "half done"})), call("select.all", json!({}))], true, tail);
        let mut s = with_plugin("io.example.broken", "write", wasm);
        s.run("text.insert", &json!({"text": "Mine: "})).unwrap();
        let (steps, sel) = (s.undo_depth(), s.sel.clone());
        let mut params = json!({"id": "io.example.broken"});
        if name == "fuel" {
            params["fuel"] = json!(100_000);
        }
        let e = s.run("plugin.run", &params).unwrap_err().to_string();
        assert!(e.contains("Test Plug-in"), "{name}: {e}");
        assert_eq!(text(&mut s), "Mine: Hello world", "{name}");
        assert_eq!((s.undo_depth(), &s.sel), (steps, &sel), "{name}");
        assert_eq!(s.undo_label(), Some("Typing"), "{name}");
    }
}

#[test]
fn a_run_may_make_only_so_many_edits() {
    let calls: Vec<(&str, String)> =
        (0..=super::MAX_RUN_STEPS).map(|i| call(if i % 2 == 0 { "format.bold" } else { "format.italic" }, json!({}))).collect();
    let mut s = with_plugin("io.example.busy", "write", module(&calls, false, ""));
    s.run("select.all", &json!({})).unwrap();
    let r = s.run("plugin.run", &json!({"id": "io.example.busy"})).unwrap();
    assert!(r["output"].as_str().unwrap_or_default().contains("at most"), "{r}");
    assert_eq!(s.undo_label(), Some("Test Plug-in"), "what it did is one step");
}

#[test]
fn macros_and_repeat_keep_the_run_not_its_calls() {
    let mut s = with_plugin("io.example.typist", "write", module(&[call("text.insert", json!({"text": "x"}))], true, ""));
    s.run("tools.recordMacro", &json!({"name": "m"})).unwrap();
    s.run("plugin.run", &json!({"id": "io.example.typist"})).unwrap();
    let steps: Vec<String> = s.recording.as_ref().unwrap().1.iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(steps, ["plugin.run"]);
    assert_eq!(s.last_command.as_ref().map(|(id, _)| id.as_str()), Some("plugin.run"));
}

#[test]
fn settings_are_bounded() {
    let mut all = std::collections::BTreeMap::new();
    super::set_setting(&mut all, "p", "k", Some("[1,2]")).unwrap();
    assert_eq!(all["p"]["k"], json!([1, 2]));
    assert!(super::set_setting(&mut all, "p", &"k".repeat(200), Some("1")).is_err());
    assert!(super::set_setting(&mut all, "p", "big", Some(&format!("\"{}\"", "x".repeat(70_000)))).is_err());
    assert!(super::set_setting(&mut all, "p", "bad", Some("not json")).is_err());
    super::set_setting(&mut all, "p", "k", None).unwrap();
    assert!(all.is_empty());
}

#[test]
fn another_apps_plugin_is_refused() {
    let manifest = json!({
        "id": "io.example.other", "name": "Other", "version": "1.0.0", "main": "plugin.wasm",
        "apps": {"designcraft": {"api": 6}}, "permissions": {"access": "write"}, "commands": [{"id": "go", "label": "Go"}]
    });
    let b = Bundle { manifest: manifest.to_string(), wasm: module(&[], true, ""), assets: vec![] }.encode();
    let mut s = session();
    let e = s.run("plugin.install", &json!({"data": data(&b)})).unwrap_err().to_string();
    assert!(e.contains("can't be installed"), "{e}");
    assert!(s.plugins.list().is_empty());
}

#[test]
fn a_reviewed_bundle_installs_by_its_sha256_and_no_other() {
    let b = bundle("io.example.review", "write", module(&[], true, ""));
    let mut s = session();
    let request = json!({"data": data(&b), "name": "r.craftplugin"});
    let mut review = request.clone();
    review["review"] = json!(true);
    let info = s.run("plugin.install", &review).unwrap();
    assert_eq!((info["installed"].clone(), info["kind"].clone(), info["inCatalog"].clone()), (json!(false), json!("command"), Value::Null));
    assert!(s.plugins.list().is_empty(), "a review installs nothing");
    let mut wrong = request.clone();
    wrong["sha256"] = json!("0".repeat(64));
    assert!(s.run("plugin.install", &wrong).is_err());
    let mut right = request;
    right["sha256"] = info["sha256"].clone();
    assert_eq!(s.run("plugin.install", &right).unwrap()["installed"], true);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn install_list_and_remove_through_the_store() {
    let dir = std::env::temp_dir().join(format!("wc-plugins-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("test.craftplugin");
    std::fs::write(&file, bundle("io.example.stored", "write", module(&[call("text.insert", json!({"text": "!"}))], true, ""))).unwrap();

    let mut s = session();
    s.plugins.open_store(&dir);
    let r = s.run("plugin.install", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!((r["installed"].clone(), r["stored"].clone()), (json!(true), json!(true)));
    s.plugins.prefs.settings.insert("io.example.stored".into(), serde_json::Map::from_iter([("k".to_string(), json!(1))]));
    s.plugins.save_prefs();

    // Another session reads what is installed, and its settings.
    let mut t = session();
    t.plugins.open_store(&dir);
    let list = t.run("plugin.list", &json!({})).unwrap();
    assert_eq!(list["plugins"][0]["id"], "io.example.stored", "{list}");
    assert_eq!(list["plugins"][0]["source"]["type"], "file");
    assert_eq!(t.plugins.prefs.settings["io.example.stored"]["k"], 1);
    t.run("plugin.run", &json!({"id": "io.example.stored"})).unwrap();
    assert_eq!(text(&mut t), "!Hello world");

    assert_eq!(t.run("plugin.remove", &json!({"id": "io.example.stored"})).unwrap()["removed"], "io.example.stored");
    assert!(t.run("plugin.list", &json!({})).unwrap()["plugins"].as_array().unwrap().is_empty());
    assert!(!t.plugins.prefs.settings.contains_key("io.example.stored"), "its settings go with it");
    let mut u = session();
    u.plugins.open_store(&dir);
    assert!(u.plugins.list().is_empty(), "removed from the folder");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn catalogs_must_be_on_github_or_local() {
    let mut s = session();
    assert!(s.run("plugin.catalog.add", &json!({"url": "https://example.com/index.json"})).is_err());
    let ok = "https://raw.githubusercontent.com/example/catalog/main/index.json";
    assert_eq!(s.run("plugin.catalog.add", &json!({"url": ok})).unwrap()["catalogs"], json!([ok]));
    assert_eq!(s.run("plugin.catalog.add", &json!({"url": ok, "remove": true})).unwrap()["catalogs"], json!([]));
    // No network in a headless session: a repository can't be fetched, and says so.
    let e = s.run("plugin.install", &json!({"repo": "example/plugin"})).unwrap_err().to_string();
    assert!(e.contains("no network access"), "{e}");
}

#[test]
fn get_add_ins_lists_without_opening_anything() {
    let mut s = session();
    let r = s.run("addins.get", &json!({})).unwrap();
    assert_eq!(r["pending"], "plugins");
    assert!(s.ui_requests.is_empty());
    assert!(!s.can_undo());
}

#[test]
fn new_ui_requests_are_the_tail() {
    let (a, b, c) = (json!(1), json!(2), json!(3));
    let ab = [a.clone(), b.clone()];
    assert_eq!(super::new_ui_requests(&ab, &[a.clone(), b.clone(), c.clone()]), std::slice::from_ref(&c));
    assert_eq!(super::new_ui_requests(&ab, &[b, c.clone()]), std::slice::from_ref(&c), "the oldest was dropped");
    assert!(super::new_ui_requests(std::slice::from_ref(&a), std::slice::from_ref(&a)).is_empty());
}
