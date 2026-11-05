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
    let tests: [(&str, fn()); 9] = [
        ("plugin_commands_are_one_undo_step", plugin_commands_are_one_undo_step),
        ("a_failing_plugin_command_changes_nothing", a_failing_plugin_command_changes_nothing),
        ("plugin_commands_see_the_selection_and_paragraph", plugin_commands_see_the_selection_and_paragraph),
        ("plugins_turn_off_and_on_and_the_preferences_remember", plugins_turn_off_and_on_and_the_preferences_remember),
        ("confined_automation_cannot_load_plugins", confined_automation_cannot_load_plugins),
        ("commands_see_the_document_file_and_whether_it_is_saved", commands_see_the_document_file_and_whether_it_is_saved),
        ("commands_ask_to_open_files_which_are_checked", commands_ask_to_open_files_which_are_checked),
        ("commands_answer_with_dialogs_which_are_checked", commands_answer_with_dialogs_which_are_checked),
        (
            "document_events_reach_the_add_ins_that_declared_them_and_never_hold_up",
            document_events_reach_the_add_ins_that_declared_them_and_never_hold_up,
        ),
    ];
    for (name, t) in tests {
        t();
        println!("test {name} ... ok");
    }
}

/// The plugin side: `shout` replaces the selection with capitals and adds a paragraph, `broken`
/// does that and then runs a command that doesn't exist, `undo` tries to take history back, `echo`
/// returns what it was given. Protocol 1.2: `opener` asks to open `params.path`, `dialog` answers
/// with `params.dialog`; `documentSaved` tries `host/sync` (refused) and sends a message,
/// `documentOpened` takes a second and a half, `documentClosed` longer than its 5 s.
fn serve() {
    let mut out = std::io::stdout();
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(Ok(line)) = lines.next() {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { break };
        let p = &msg["params"];
        let mut ask = |method: &str, params: Value| -> Value {
            writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": "h1", "method": method, "params": params})).unwrap();
            out.flush().unwrap();
            for line in lines.by_ref() {
                let reply: Value = serde_json::from_str(&line.unwrap()).unwrap();
                if reply["id"] == "h1" {
                    return reply;
                }
            }
            Value::Null
        };
        let result = match (msg["method"].as_str(), p["command"].as_str()) {
            (Some("events/documentSaved"), _) => {
                let refused = ask("host/sync", json!({"actions": []}));
                ask("host/message", json!({"text": "hello from an event"}));
                let code = refused["error"]["data"]["code"].as_str().unwrap_or("allowed").to_string();
                json!({"message": format!("saved {} {} {code}", p["name"].as_str().unwrap_or(""), p["format"].as_str().unwrap_or(""))})
            }
            (Some("events/documentOpened"), _) => {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                json!({"message": format!("opened {}", p["name"].as_str().unwrap_or(""))})
            }
            (Some("events/documentClosed"), _) => {
                std::thread::sleep(std::time::Duration::from_secs(8));
                json!({})
            }
            (Some("commands/run"), Some("echo")) => json!({"result": p["context"], "message": "seen"}),
            (Some("commands/run"), Some("opener")) => {
                json!({"open": {"path": p["params"]["path"], "label": "Cloud · notes.docx"}, "message": "Opening notes.docx", "commands": [{"command": "text.insert", "params": {"text": "!", "raw": true}}]})
            }
            (Some("commands/run"), Some("dialog")) => {
                json!({"dialog": p["params"]["dialog"], "message": "pick one", "result": {"folder": "/Documents"}})
            }
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
    load_with(s, json!([]));
}

/// [`load`], the add-in hearing `events`.
fn load_with(s: &mut Session, events: Value) {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let manifest = json!({
        "id": PLUGIN, "name": "Engine test", "homepage": "https://example.org/docs", "apps": ["wordcraft"],
        "run": {"command": exe, "args": ["--serve-plugin"]},
        "commands": [{"id": "shout", "label": "Shout"}, {"id": "broken", "label": "Broken"}, {"id": "undo", "label": "Undo"}, {"id": "echo", "label": "Echo"},
                     {"id": "opener", "label": "Opener"}, {"id": "dialog", "label": "Dialog"}, {"id": "pick", "label": "Pick"}],
        "events": events,
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

fn run_with(s: &mut Session, command: &str, params: Value) -> Result<Value, wordcraft_engine::CmdError> {
    s.run("plugins.run", &json!({"plugin": PLUGIN, "command": command, "params": params}))
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-plugin-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn commands_see_the_document_file_and_whether_it_is_saved() {
    let mut s = session();
    load(&mut s);
    let doc = run_plugin(&mut s, "echo").unwrap()["result"]["document"].clone();
    assert_eq!((doc["path"].clone(), doc["name"].clone(), doc["dirty"].clone()), (Value::Null, Value::Null, json!(true)), "{doc}");
    let path = temp("context.docx");
    s.run("file.saveAs", &json!({"path": path})).unwrap();
    let doc = run_plugin(&mut s, "echo").unwrap()["result"]["document"].clone();
    assert_eq!(doc["path"], json!(path.display().to_string()));
    assert_eq!(doc["name"], "context.docx");
    assert_eq!(doc["dirty"], false);
    assert!(wordcraft_plugin_unload());
}

fn commands_ask_to_open_files_which_are_checked() {
    let mut s = session();
    load(&mut s);
    let file = temp("notes.docx");
    Session::new(Document::new()).run("file.saveAs", &json!({"path": file})).unwrap();
    // Run programmatically, the file isn't opened: the caller gets `open` to act on.
    let r = run_with(&mut s, "opener", json!({"path": file})).unwrap();
    assert_eq!(r["open"], json!({"path": file.display().to_string(), "label": "Cloud · notes.docx", "readOnly": false}));
    assert_eq!(text(&s), "quiet words here!", "the step ran; nothing else was opened");
    // Fields combine: the message comes with the file to open.
    assert_eq!((r["message"].clone(), r["ran"].clone()), (json!("Opening notes.docx"), json!(1)));
    // A relative path, a missing file, a folder or a kind of file WordCraft doesn't open fails the
    // command, and its step is taken back.
    let picture = temp("picture.png");
    std::fs::write(&picture, b"png").unwrap();
    for bad in [json!("notes.docx"), json!(temp("missing.docx")), json!(std::env::temp_dir()), json!(picture)] {
        let e = run_with(&mut s, "opener", json!({"path": bad})).unwrap_err().to_string();
        assert!(e.contains("asked to open"), "{e}");
        assert_eq!(text(&s), "quiet words here!");
    }
    // `file.open` with the label shows where the document came from.
    s.run("file.open", &json!({"path": file, "label": "Cloud · notes.docx"})).unwrap();
    assert_eq!(s.location_label.as_deref(), Some("Cloud · notes.docx"));
    assert_eq!(s.run("file.info", &json!({})).unwrap()["location"], "Cloud · notes.docx");
    s.run("file.saveAs", &json!({"path": temp("elsewhere.docx")})).unwrap();
    assert_eq!(s.location_label, None, "saved elsewhere, it isn't there any more");
    assert!(wordcraft_plugin_unload());
}

fn commands_answer_with_dialogs_which_are_checked() {
    let mut s = session();
    load(&mut s);
    let dialog = json!({"title": "Open from the cloud", "kind": "list", "prompt": "Documents",
        "items": [{"id": "/Documents/Reports", "label": "Reports", "detail": "Folder", "icon": "folder"}],
        "command": "pick", "params": {"account": "me"}, "okLabel": "Open",
        "buttons": [{"label": "Up", "command": "pick", "params": {"path": "/"}}]});
    s.dirty = false;
    let depth = s.undo_depth();
    let r = run_with(&mut s, "dialog", json!({"dialog": dialog})).unwrap();
    // A step that only answers changes nothing: no undo step, nothing unsaved.
    assert_eq!(s.undo_depth(), depth);
    assert!(!s.dirty);
    assert_eq!(r["dialog"]["title"], "Open from the cloud");
    assert_eq!(r["dialog"]["items"][0]["id"], "/Documents/Reports");
    assert_eq!(r["dialog"]["okLabel"], "Open");
    // Fields combine: the dialog comes with the message and the result.
    assert_eq!(r["message"], "pick one");
    assert_eq!(r["result"], json!({"folder": "/Documents"}));
    // A dialog that runs a command the add-in doesn't have, or a malformed one, fails the command.
    let mut stray = dialog.clone();
    stray["command"] = json!("nope");
    assert!(run_with(&mut s, "dialog", json!({"dialog": stray})).unwrap_err().to_string().contains("`nope`"));
    for bad in [
        json!({"kind": "web"}),
        json!({"kind": "list", "items": []}),
        json!({"kind": "form", "command": "pick", "fields": [{"id": "a", "kind": "slider"}]}),
        json!(42),
    ] {
        assert!(run_with(&mut s, "dialog", json!({"dialog": bad})).is_err(), "{bad}");
    }
    assert!(wordcraft_plugin_unload());
}

fn document_events_reach_the_add_ins_that_declared_them_and_never_hold_up() {
    use std::time::{Duration, Instant};
    let mut s = session();
    // Events are the process's: this save happens before the add-in listens.
    let other = temp("other.docx");
    Session::new(Document::new()).run("file.saveAs", &json!({"path": other})).unwrap();
    load_with(&mut s, json!(["documentSaved", "documentClosed"]));
    let _ = wordcraft_plugin::take_event_outcomes();
    // A save: the add-in hears of it after the fact; its `host/sync` is refused.
    let path = temp("events.docx");
    s.run("file.saveAs", &json!({"path": path})).unwrap();
    assert!(wordcraft_plugin::wait_for_events(Duration::from_secs(20)));
    let out: Vec<_> = wordcraft_plugin::take_event_outcomes().into_iter().filter_map(|o| o.message).collect();
    assert_eq!(out, ["hello from an event", "saved events.docx docx AccessDenied"]);
    // AutoSave's writes aren't saves the add-in hears of.
    s.run("text.insert", &json!({"text": "more ", "raw": true})).unwrap();
    s.run("file.save", &json!({"autosave": true})).unwrap();
    assert_eq!(wordcraft_plugin::events_in_flight(), 0);
    // Opening a document opens it beside this one: documentOpened (not declared) isn't sent.
    s.run("file.open", &json!({"path": other})).unwrap();
    assert_eq!(s.path.as_deref(), Some(other.as_path()));
    assert_eq!(wordcraft_plugin::events_in_flight(), 0);
    // Closing it: the add-in, slow to answer documentClosed, holds up nothing. Its timeout is a
    // failure.
    let t = Instant::now();
    s.run("file.closeDocument", &json!({"force": true})).unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "the close didn't wait: {:?}", t.elapsed());
    assert_eq!(s.path.as_deref(), Some(path.as_path()));
    assert!(wordcraft_plugin::wait_for_events(Duration::from_secs(20)));
    let out = wordcraft_plugin::take_event_outcomes();
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].event, wordcraft_plugin::DocumentEvent::Closed);
    assert!(out[0].error.as_deref().is_some_and(|e| e.contains("took longer than 5000 ms")), "{out:?}");
    // Declared, documentOpened is sent; the open still doesn't wait for it.
    load_with(&mut s, json!(["documentOpened"]));
    s.run("file.closeDocument", &json!({"force": true})).unwrap();
    let t = Instant::now();
    s.run("file.open", &json!({"path": path})).unwrap();
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert!(wordcraft_plugin::wait_for_events(Duration::from_secs(20)));
    let out: Vec<_> = wordcraft_plugin::take_event_outcomes().into_iter().filter_map(|o| o.message).collect();
    assert_eq!(out, ["opened events.docx"]);
    assert!(wordcraft_plugin_unload());
}
