//! The plugin host against a real plugin process (`wordcraft-example-plugin`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::json;

/// The example plugin's id is fixed, so the tests take turns.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

const ID: &str = "org.wordcraft.example";

/// Writes the example plugin's manifest into a fresh folder and loads it.
fn load_example(tag: &str) -> (Arc<wordcraft_plugin::Plugin>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("wordcraft-plugin-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_wordcraft-example-plugin")).arg("--manifest").output().unwrap();
    std::fs::write(dir.join("plugin.json"), out.stdout).unwrap();
    (wordcraft_plugin::load(&dir).unwrap(), dir)
}

#[test]
fn commands_reply_with_engine_commands() {
    let _g = serial();
    let (p, dir) = load_example("commands");
    assert_eq!(p.status(), "ready");
    assert_eq!(p.manifest.ui().ribbon.len(), 2);
    let ctx = json!({"selection": {"text": "quiet words"}, "paragraph": {"text": "Some quiet words here."}});
    let r = wordcraft_plugin::run_command(ID, "upperCase", json!({}), ctx.clone()).unwrap();
    assert_eq!(r.commands, vec![("text.insert".to_string(), json!({"text": "QUIET WORDS", "raw": true}))]);
    assert_eq!(p.status(), "running");
    let r = wordcraft_plugin::run_command(ID, "countWords", json!({}), ctx).unwrap();
    assert_eq!(r.message.as_deref(), Some("2 words, 11 characters"));
    assert_eq!(r.result["words"], 2);
    // An error answer fails the command; the process keeps running.
    let e = wordcraft_plugin::run_command(ID, "upperCase", json!({}), json!({"selection": {"text": ""}})).unwrap_err();
    assert!(e.contains("Select some text"), "{e}");
    assert_eq!(p.status(), "running");
    assert!(wordcraft_plugin::unload(ID));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hangs_and_crashes_fail_the_command_and_restart_the_process() {
    let _g = serial();
    let (p, dir) = load_example("failures");
    // The process dies: the command fails, and the next one starts it again.
    assert!(wordcraft_plugin::run_command(ID, "crash", json!({}), json!({})).is_err());
    assert_eq!(p.status(), "ready");
    assert!(wordcraft_plugin::run_command(ID, "countWords", json!({}), json!({"paragraph": {"text": "a b"}})).is_ok());
    // Two more failures and it is stopped until reloaded.
    for _ in 0..2 {
        assert!(wordcraft_plugin::run_command(ID, "crash", json!({}), json!({})).is_err());
    }
    assert!(p.status().starts_with("failed"), "{}", p.status());
    assert!(wordcraft_plugin::run_command(ID, "countWords", json!({}), json!({})).is_err());
    // Turning it off and on again gives it a fresh start.
    wordcraft_plugin::set_enabled(ID, false).unwrap();
    wordcraft_plugin::set_enabled(ID, true).unwrap();
    assert!(wordcraft_plugin::run_command(ID, "countWords", json!({}), json!({})).is_ok());
    assert!(wordcraft_plugin::unload(ID));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_missing_program_is_an_error() {
    let _g = serial();
    let dir = std::env::temp_dir().join(format!("wordcraft-plugin-missing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = json!({"id": "t.missing", "name": "Missing", "run": {"command": "./no-such-program"}, "commands": [{"id": "go", "label": "Go"}]});
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    wordcraft_plugin::load(&dir).unwrap();
    let e = wordcraft_plugin::run_command("t.missing", "go", json!({}), json!({})).unwrap_err();
    assert!(e.contains("can't start"), "{e}");
    assert!(wordcraft_plugin::unload("t.missing"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn protocol_1_2_features_dialogs_and_events() {
    let _g = serial();
    wordcraft_plugin::set_host_language("it");
    let (_p, dir) = load_example("protocol-1-2");
    // `initialize` tells the plugin what the host does and its interface language.
    let info = wordcraft_plugin::run_command(ID, "hostInfo", json!({}), json!({})).unwrap().result;
    assert_eq!(info["features"], json!(["dialogs", "open", "events", "document"]));
    assert_eq!(info["language"], "it");
    assert_eq!(info["id"], "wordcraft");
    // A command answers with a form the app draws; its OK runs another command.
    let r = wordcraft_plugin::run_command(ID, "greeting", json!({}), json!({})).unwrap();
    let d = r.dialog.expect("a dialog");
    assert_eq!(d.kind, wordcraft_plugin::DialogKind::Form);
    assert_eq!((d.title.as_str(), d.command.as_deref(), d.ok_label.as_deref()), ("Greeting", Some("greet"), Some("Greet")));
    assert_eq!(d.fields[0].value, json!("World"));
    let params = wordcraft_plugin::dialog::with(&d.params, [("fields", json!({"name": "Ada"}))]);
    let r = wordcraft_plugin::run_command(ID, "greet", params, json!({})).unwrap();
    assert_eq!(r.commands[0].1["text"], "Hello, Ada!");
    // Events go to the plugins that declared them, on their own thread; the answer is queued.
    let _ = wordcraft_plugin::take_event_outcomes();
    let sent =
        wordcraft_plugin::send_event(wordcraft_plugin::DocumentEvent::Saved, json!({"path": "/tmp/a.docx", "name": "a.docx", "format": "docx"}));
    assert_eq!(sent, 1);
    assert_eq!(
        wordcraft_plugin::send_event(wordcraft_plugin::DocumentEvent::Opened, json!({"path": "/tmp/a.docx", "name": "a.docx"})),
        0,
        "not declared"
    );
    assert!(wordcraft_plugin::wait_for_events(std::time::Duration::from_secs(20)));
    let out = wordcraft_plugin::take_event_outcomes();
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].message.as_deref(), Some("Example saw a.docx saved (docx)"));
    assert!(wordcraft_plugin::unload(ID));
    let _ = std::fs::remove_dir_all(dir);
}
