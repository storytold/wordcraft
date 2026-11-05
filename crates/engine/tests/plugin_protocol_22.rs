//! Plugin protocol 2.2 end to end, as it applies to a word processor: `host.protocol` "2.2" with
//! exactly the features WordCraft implements; the document `id` in `context.document` and in the
//! document events (the same through a save, new for a document that replaces another, never
//! reused); multi-line form fields; `text.insert {literal: true}`; and `host/functions/invalidate`
//! refused as unknown (WordCraft has no plugin functions). The plugin is this test binary started
//! again with `--serve-plugin` (no libtest harness, so standard output is the protocol's alone).

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_engine::doc::{Document, StoryRef};

const PLUGIN: &str = "org.wordcraft.test-protocol-22";

fn main() {
    if std::env::args().any(|a| a == "--serve-plugin") {
        serve();
        return;
    }
    load();
    let tests: [(&str, fn()); 5] = [
        ("initialize_announces_protocol_2_2_and_only_what_applies", initialize_announces_protocol_2_2_and_only_what_applies),
        ("documents_carry_an_id_in_context_and_events", documents_carry_an_id_in_context_and_events),
        ("literal_text_goes_in_as_given", literal_text_goes_in_as_given),
        ("multiline_fields_are_validated_and_returned", multiline_fields_are_validated_and_returned),
        ("invalidate_is_an_unknown_host_method", invalidate_is_an_unknown_host_method),
    ];
    for (name, t) in tests {
        t();
        println!("test {name} ... ok");
    }
    wordcraft_plugin::unload(PLUGIN);
    let _ = std::fs::remove_dir_all(dir());
}

// ---------------------------------------------------------------- the plugin

/// - `hello`: what `initialize` said, and `context.document`.
/// - events: answer `{"message": "<event> <name> <id>"}`, so the outcomes show what came.
/// - `literal`: types its `params.text` with `literal: true`.
/// - `form`: a form with a code field, a multi-line field, a number field asking for lines (ignored)
///   and a one-line field.
/// - `invalidate`: asks the host to invalidate its functions during the command, and returns the
///   answer.
fn serve() {
    let mut out = std::io::stdout();
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut init = Value::Null;
    while let Some(Ok(line)) = lines.next() {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { break };
        let p = msg["params"].clone();
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let result = match (method.as_str(), p["command"].as_str()) {
            ("initialize", _) => {
                init = p.clone();
                json!({"protocolVersion": 1})
            }
            (m, _) if m.starts_with("events/") => {
                let event = m.trim_start_matches("events/");
                json!({"message": format!("{event} {} {}", p["name"].as_str().unwrap_or("?"), p["id"].as_str().unwrap_or("none"))})
            }
            ("commands/run", Some("hello")) => json!({"result": {"init": init, "document": p["context"]["document"]}}),
            ("commands/run", Some("literal")) => {
                json!({"commands": [{"command": "text.insert", "params": {"text": p["params"]["text"], "literal": true}}]})
            }
            ("commands/run", Some("form")) => json!({"dialog": {
                "kind": "form", "title": "Run", "command": "hello",
                "fields": [
                    {"id": "code", "label": "Code", "value": "x <- 1\ny <- 2", "multiline": true, "code": true, "rows": 99},
                    {"id": "notes", "label": "Notes", "multiline": true},
                    {"id": "n", "label": "N", "kind": "number", "multiline": true},
                    {"id": "one", "label": "One"}
                ]
            }}),
            ("commands/run", Some("invalidate")) => {
                writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": "h1", "method": "host/functions/invalidate", "params": {}})).unwrap();
                out.flush().unwrap();
                let mut answer = Value::Null;
                for line in lines.by_ref() {
                    let reply: Value = serde_json::from_str(&line.unwrap()).unwrap();
                    if reply["id"] == "h1" {
                        answer = reply;
                        break;
                    }
                }
                json!({"result": answer})
            }
            _ => json!({}),
        };
        if writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result})).and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}

// ---------------------------------------------------------------- helpers

fn dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("wordcraft-engine-protocol-22-{}", std::process::id()))
}

fn load() {
    let pdir = dir().join(PLUGIN);
    std::fs::create_dir_all(&pdir).unwrap();
    let manifest = json!({
        "id": PLUGIN, "name": "Protocol 2.2", "apps": ["wordcraft"],
        "run": {"command": std::env::current_exe().unwrap(), "args": ["--serve-plugin"]},
        "commands": [{"id": "hello", "label": "Hello"}, {"id": "literal", "label": "Literal"},
                     {"id": "form", "label": "Form"}, {"id": "invalidate", "label": "Invalidate"}],
        "events": ["documentOpened", "documentSaved", "documentClosed"],
    });
    std::fs::write(pdir.join("plugin.json"), manifest.to_string()).unwrap();
    wordcraft_plugin::load(&pdir).unwrap();
}

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.run("plugins.run", &json!({"plugin": PLUGIN, "command": command, "params": params})).unwrap()
}

fn document(s: &mut Session) -> Value {
    run(s, "hello", json!({}))["result"]["document"].clone()
}

/// The messages of the events delivered since the last call.
fn events() -> Vec<String> {
    assert!(wordcraft_plugin::wait_for_events(Duration::from_secs(20)));
    wordcraft_plugin::take_event_outcomes().into_iter().filter_map(|o| o.message).collect()
}

fn file(name: &str) -> std::path::PathBuf {
    let d = dir().join("files");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

// ---------------------------------------------------------------- tests

fn initialize_announces_protocol_2_2_and_only_what_applies() {
    let mut s = Session::new(Document::new());
    let host = run(&mut s, "hello", json!({}))["result"]["init"]["host"].clone();
    assert_eq!(host["protocol"], "2.2");
    let features = host["features"].as_array().unwrap();
    for f in ["multiline", "literalValues"] {
        assert!(features.contains(&json!(f)), "{f} is announced: {features:?}");
    }
    // Formula functions are GridCraft's: their features aren't announced here.
    for f in ["functionDocument", "openedBeforeCalc", "emptyResult", "invalidate"] {
        assert!(!features.contains(&json!(f)), "{f} isn't announced: {features:?}");
    }
}

fn documents_carry_an_id_in_context_and_events() {
    let _ = events();
    let mut s = Session::new(Document::new());
    let doc = document(&mut s);
    let id = doc["id"].as_str().unwrap().to_string();
    assert_eq!(id, s.plugin_document_id());
    assert!(!id.is_empty() && id.chars().count() <= 64, "{id}");
    // Another window's document is another document.
    let mut other = Session::new(Document::new());
    assert_ne!(document(&mut other)["id"], json!(id));

    // Saved and edited, the document keeps its id; the save's event carries it.
    let path = file("Ids.docx");
    s.run("file.saveAs", &json!({"path": path})).unwrap();
    s.run("text.insert", &json!({"text": "edited", "raw": true})).unwrap();
    assert_eq!(document(&mut s)["id"], json!(id));
    assert_eq!(events(), [format!("documentSaved Ids.docx {id}")]);
    let again = file("Ids again.docx");
    s.run("file.saveAs", &json!({"path": again})).unwrap();
    assert_eq!(document(&mut s)["id"], json!(id), "Save As keeps the id");
    assert_eq!(events(), [format!("documentSaved Ids again.docx {id}")]);

    // Opening a file opens another document beside this one: the opened one has a new id, which
    // commands then see.
    s.run("file.open", &json!({"path": path})).unwrap();
    let opened = document(&mut s)["id"].as_str().unwrap().to_string();
    let opened_num = s.document_id();
    assert_ne!(opened, id);
    assert_eq!(events(), [format!("documentOpened Ids.docx {opened}")]);

    // A new document is another one too, and ids are never reused.
    s.run("file.new", &json!({})).unwrap();
    let fresh = document(&mut s)["id"].as_str().unwrap().to_string();
    assert!(fresh != opened && fresh != id);
    assert!(events().is_empty(), "a new document has no file to open");

    // Closing a document: its event has its own id, whichever document is active.
    s.run("file.closeDocument", &json!({"id": opened_num, "force": true})).unwrap();
    assert_eq!(document(&mut s)["id"], json!(fresh));
    assert_eq!(events(), [format!("documentClosed Ids.docx {opened}")]);
    s.run("file.closeDocument", &json!({"force": true})).unwrap();
    assert_eq!(events(), Vec::<String>::new(), "the new document had no file");
    s.run("file.closeDocument", &json!({"force": true})).unwrap();
    assert_eq!(events(), [format!("documentClosed Ids again.docx {id}")]);
}

fn literal_text_goes_in_as_given() {
    let mut s = Session::new(Document::new());
    // Typed, "1." and a space start a numbered list and a quote becomes a curly one.
    s.run("text.insert", &json!({"text": "1."})).unwrap();
    s.run("text.insert", &json!({"text": " "})).unwrap();
    s.run("text.insert", &json!({"text": "\""})).unwrap();
    let para = s.doc.para_at(&s.sel.focus).unwrap();
    assert!(para.props.numbering.is_some(), "typed: a list");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "\u{201C}");

    // Literal: exactly as given, from a plugin's reply and directly, one character at a time too.
    let mut s = Session::new(Document::new());
    run(&mut s, "literal", json!({"text": "1."}));
    run(&mut s, "literal", json!({"text": " "}));
    s.run("text.insert", &json!({"text": "\"", "literal": true})).unwrap();
    s.run("text.insert", &json!({"text": "teh -- x' ", "literal": true})).unwrap();
    let para = s.doc.para_at(&s.sel.focus).unwrap();
    assert!(para.props.numbering.is_none(), "literal: no list");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "1. \"teh -- x' ");

    // Paragraph breaks are text, as pasted: an empty list item between two stays.
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "*"})).unwrap();
    s.run("text.insert", &json!({"text": " "})).unwrap();
    assert!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.is_some());
    s.run("text.insert", &json!({"text": "a\r\rb", "literal": true})).unwrap();
    let listed = s.doc.body.iter().filter_map(|b| if let wordcraft_engine::doc::Block::Para(p) = b.as_ref() { Some(p) } else { None });
    let items: Vec<bool> = listed.map(|p| p.props.numbering.is_some()).collect();
    assert_eq!(items, [true, true, true], "the empty item didn't end the list");
}

fn multiline_fields_are_validated_and_returned() {
    let mut s = Session::new(Document::new());
    let r = run(&mut s, "form", json!({}));
    let f = &r["dialog"]["fields"];
    assert_eq!(f[0]["multiline"], true);
    assert_eq!(f[0]["code"], true);
    assert_eq!(f[0]["rows"], 30, "rows are bounded");
    assert_eq!(f[0]["value"], "x <- 1\ny <- 2");
    assert_eq!(f[1]["multiline"], true);
    assert!(f[1].get("code").is_none());
    assert_eq!(f[1]["rows"], 8, "the default height");
    assert!(f[2].get("multiline").is_none(), "only text fields are multi-line");
    assert!(f[3].get("multiline").is_none() && f[3].get("rows").is_none());
}

fn invalidate_is_an_unknown_host_method() {
    let mut s = Session::new(Document::new());
    let r = run(&mut s, "invalidate", json!({}));
    assert_eq!(r["result"]["error"]["code"], -32601, "{r}");
}
