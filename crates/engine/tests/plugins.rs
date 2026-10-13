//! Add-ins end to end: the engine loads a plugin and its commands run as one undoable step. The
//! plugin is this test binary started again with `--serve-plugin` (no libtest harness, so
//! standard output is the protocol's alone).

use std::io::{BufRead, Write};

use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_engine::doc::{Document, StoryRef};

const PLUGIN: &str = "org.wordcraft.test-engine";

fn main() {
    if std::env::args().any(|a| a == "--serve-plugin") {
        serve();
        return;
    }
    let tests: [(&str, fn()); 5] = [
        ("plugin_commands_are_one_undo_step", plugin_commands_are_one_undo_step),
        ("a_failing_plugin_command_changes_nothing", a_failing_plugin_command_changes_nothing),
        ("plugin_commands_see_the_selection_and_paragraph", plugin_commands_see_the_selection_and_paragraph),
        ("plugins_turn_off_and_on_and_the_preferences_remember", plugins_turn_off_and_on_and_the_preferences_remember),
        ("confined_automation_cannot_load_plugins", confined_automation_cannot_load_plugins),
    ];
    for (name, t) in tests {
        t();
        println!("test {name} ... ok");
    }
}

/// The plugin side: `shout` replaces the selection with capitals and adds a paragraph, `broken`
/// does that and then runs a command that doesn't exist, `undo` tries to take history back, `echo`
/// returns what it was given.
fn serve() {
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { break };
        let p = &msg["params"];
        let result = match (msg["method"].as_str(), p["command"].as_str()) {
            (Some("commands/run"), Some("echo")) => json!({"result": p["context"], "message": "seen"}),
            (Some("commands/run"), Some("undo")) => json!({"commands": [{"command": "edit.undo"}]}),
            (Some("commands/run"), Some(cmd)) => {
                let sel = p["context"]["selection"]["text"].as_str().unwrap_or("").to_uppercase();
                let mut cmds = vec![
                    json!({"command": "text.insert", "params": {"text": sel, "raw": true}}),
                    json!({"command": "caret.docEnd"}),
                    json!({"command": "text.newParagraph"}),
                    json!({"command": "text.insert", "params": {"text": "Signed", "raw": true}}),
                ];
                if cmd == "broken" {
                    cmds.push(json!({"command": "no.suchCommand"}));
                }
                json!({"commands": cmds, "message": "done"})
            }
            _ => json!({}),
        };
        if writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result})).is_err() {
            break;
        }
    }
}

fn session() -> Session {
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "quiet words here", "raw": true})).unwrap();
    s
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

fn load(s: &mut Session) {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let manifest = json!({
        "id": PLUGIN, "name": "Engine test", "homepage": "https://example.org/docs", "apps": ["wordcraft"],
        "run": {"command": exe, "args": ["--serve-plugin"]},
        "commands": [{"id": "shout", "label": "Shout"}, {"id": "broken", "label": "Broken"}, {"id": "undo", "label": "Undo"}, {"id": "echo", "label": "Echo"}],
    });
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    let info = s.run("plugins.load", &json!({"path": dir})).unwrap();
    assert_eq!(info["id"], PLUGIN);
}

fn run_plugin(s: &mut Session, command: &str) -> Result<Value, wordcraft_engine::CmdError> {
    s.run("plugins.run", &json!({"plugin": PLUGIN, "command": command}))
}

fn plugin_commands_are_one_undo_step() {
    let mut s = session();
    load(&mut s);
    s.run("select.text", &json!({"text": "words"})).unwrap();
    let depth = s.undo_depth();
    let r = run_plugin(&mut s, "shout").unwrap();
    assert_eq!(r["ran"], 4);
    assert_eq!(r["message"], "done");
    assert_eq!(text(&s), "quiet WORDS here\nSigned");
    assert_eq!(s.undo_depth(), depth + 1, "one undo step for the whole reply");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s), "quiet words here");
    // Repeat runs the plugin command again, not its last engine command.
    assert_eq!(s.last_command.as_ref().map(|(id, _)| id.as_str()), Some("plugins.run"));
    assert!(wordcraft_plugin_unload());
}

fn a_failing_plugin_command_changes_nothing() {
    let mut s = session();
    load(&mut s);
    s.run("select.text", &json!({"text": "words"})).unwrap();
    let depth = s.undo_depth();
    let e = run_plugin(&mut s, "broken").unwrap_err().to_string();
    assert!(e.contains("nothing was changed") && e.contains("no.suchCommand"), "{e}");
    assert_eq!(text(&s), "quiet words here");
    assert_eq!(s.undo_depth(), depth);
    let e = run_plugin(&mut s, "undo").unwrap_err().to_string();
    assert!(e.contains("can't run `edit.undo`"), "{e}");
    assert!(run_plugin(&mut s, "nope").unwrap_err().to_string().contains("no command"));
    assert!(wordcraft_plugin_unload());
}

fn plugin_commands_see_the_selection_and_paragraph() {
    let mut s = session();
    load(&mut s);
    s.run("select.text", &json!({"text": "quiet"})).unwrap();
    let r = run_plugin(&mut s, "echo").unwrap();
    let ctx = &r["result"];
    assert_eq!(ctx["selection"]["text"], "quiet");
    assert_eq!(ctx["selection"]["empty"], false);
    assert_eq!(ctx["paragraph"]["text"], "quiet words here");
    assert_eq!(ctx["document"]["words"], 3);
    assert_eq!(r["ran"], 0);
    assert_eq!(s.status, "seen", "the plugin's message reaches the status bar");
    assert!(wordcraft_plugin_unload());
}

fn plugins_turn_off_and_on_and_the_preferences_remember() {
    let mut s = session();
    load(&mut s);
    let info = s.run("plugins.setEnabled", &json!({"id": PLUGIN, "enabled": false})).unwrap();
    assert_eq!(info["status"], "off");
    assert_eq!(s.prefs.disabled_plugins, vec![PLUGIN.to_string()]);
    assert!(run_plugin(&mut s, "shout").unwrap_err().to_string().contains("turned off"));
    // Loaded again (the next start), it stays off.
    load(&mut s);
    let listed = s.run("plugins.list", &json!({})).unwrap();
    assert_eq!(listed[0]["enabled"], false);
    s.run("plugins.setEnabled", &json!({"id": PLUGIN, "enabled": true})).unwrap();
    assert!(s.prefs.disabled_plugins.is_empty());
    assert!(run_plugin(&mut s, "echo").is_ok());
    // The preferences turn it off at startup.
    s.prefs.disabled_plugins.push(PLUGIN.into());
    wordcraft_engine::cmd::plugins::apply_prefs(&s.prefs);
    assert_eq!(s.run("addins.get", &json!({})).unwrap()[0]["status"], "off");
    let url = s.run("plugins.openHomepage", &json!({"id": PLUGIN})).unwrap();
    assert_eq!(url["url"], "https://example.org/docs");
    assert_eq!(s.ui_requests.last(), Some(&json!({"openUrl": "https://example.org/docs"})));
    assert!(s.run("plugins.unload", &json!({"id": PLUGIN})).is_ok());
    assert!(s.run("plugins.unload", &json!({"id": PLUGIN})).is_err());
}

fn confined_automation_cannot_load_plugins() {
    let mut s = session();
    let root = std::env::temp_dir();
    let roots = wordcraft_engine::file_access::AutomationRoots::new(Some(&root), Some(&root)).unwrap();
    wordcraft_engine::file_access::confine(roots.as_ref(), || {
        let e = s.run("plugins.load", &json!({"path": root})).unwrap_err().to_string();
        assert!(e.contains("confined"), "{e}");
        assert!(s.run("plugins.install", &json!({"url": "acme/tools"})).unwrap_err().to_string().contains("confined"));
    });
}

fn wordcraft_plugin_unload() -> bool {
    let mut s = Session::new(Document::new());
    s.run("plugins.unload", &json!({"id": PLUGIN})).is_ok()
}
