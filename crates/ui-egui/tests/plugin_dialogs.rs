//! Add-in dialogs and files (protocol 1.2) in the interface: a command run from the interface
//! shows the list its add-in answered with, a click and OK run the next command, whose answer is
//! the next dialog (dialogs chain), and the last one opens a file through File › Open. A form's
//! OK sends its fields; a failing step keeps the dialog open with the error. Run by a script, the
//! same command returns the dialog instead. The add-in is this test binary started again with
//! `--serve-plugin` (no libtest harness, so standard output is the protocol's alone).

use std::io::{BufRead, Write};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_ui_egui::dialogs::Dialog;
use wordcraft_ui_egui::{Services, WordApp};

const ID: &str = "org.wordcraft.ui-dialogs";

fn main() {
    if std::env::args().any(|a| a == "--serve-plugin") {
        serve();
        return;
    }
    let tests: [(&str, fn()); 2] = [
        ("dialogs_chain_and_end_by_opening_a_file", dialogs_chain_and_end_by_opening_a_file),
        ("forms_send_their_fields_and_failures_keep_the_dialog", forms_send_their_fields_and_failures_keep_the_dialog),
    ];
    for (name, t) in tests {
        t();
        println!("test {name} ... ok");
    }
    wordcraft_plugin::unload(ID);
}

fn doc_path() -> std::path::PathBuf {
    // The same in the add-in process (another process id).
    std::env::temp_dir().join("wordcraft-ui-dialogs-test").join("Budget.docx")
}

/// `browse {id?}`: the root lists a folder and a file; the folder lists the file. `pick {id}`
/// opens the file. `signIn` answers with a form; `hello {fields}` types its name, or fails
/// without one.
fn serve() {
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { break };
        let p = &msg["params"];
        let args = &p["params"];
        let file = json!({"id": doc_path(), "label": "Budget.docx", "detail": "12 KB", "icon": "workbook"});
        let answer = match (msg["method"].as_str(), p["command"].as_str()) {
            (Some("commands/run"), Some("browse")) if args["id"] == "/Reports" => {
                json!({"result": {"dialog": {"title": "Reports", "kind": "list", "items": [file], "command": "pick", "okLabel": "Open",
                    "buttons": [{"label": "Up", "command": "browse"}]}}})
            }
            (Some("commands/run"), Some("browse")) => {
                json!({"result": {"dialog": {"title": "Open from the cloud", "kind": "list", "prompt": "Documents",
                "items": [{"id": "/Reports", "label": "Reports", "detail": "Folder", "icon": "folder"}, file],
                "command": "browse", "okLabel": "Go", "buttons": [{"label": "Up", "command": "browse"}]}, "message": "Connected"}})
            }
            (Some("commands/run"), Some("pick")) => {
                json!({"result": {"open": {"path": args["id"], "label": "Cloud · Budget.docx"}, "message": "Opened from the cloud"}})
            }
            (Some("commands/run"), Some("signIn")) => json!({"result": {"dialog": {"title": "Sign in", "kind": "form", "command": "hello",
                "fields": [{"id": "name", "label": "Name"}, {"id": "pw", "label": "Password", "kind": "secret"}, {"id": "n", "label": "Copies", "kind": "number", "value": 2}]}}}),
            (Some("commands/run"), Some("hello")) => match args["fields"]["name"].as_str().filter(|n| !n.is_empty()) {
                Some(name) => {
                    json!({"result": {"commands": [{"command": "text.insert", "params": {"text": format!("Hi {name} ({})", args["fields"]["n"]), "raw": true}}]}})
                }
                None => json!({"error": {"code": 1, "message": "Type a name."}}),
            },
            _ => json!({"result": {}}),
        };
        let mut reply = json!({"jsonrpc": "2.0", "id": id});
        if let (Some(r), Value::Object(a)) = (reply.as_object_mut(), answer) {
            r.extend(a);
        }
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}

fn load() {
    let dir = std::env::temp_dir().join(format!("wordcraft-ui-dialogs-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = json!({
        "id": ID, "name": "Cloud", "apps": ["wordcraft"],
        "run": {"command": std::env::current_exe().unwrap(), "args": ["--serve-plugin"]},
        "commands": [{"id": "browse", "label": "Browse"}, {"id": "pick", "label": "Pick"}, {"id": "signIn", "label": "Sign In"}, {"id": "hello", "label": "Hello"}],
    });
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    wordcraft_plugin::load(&dir).unwrap();
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

fn shown(h: &Harness<'_, WordApp>) -> Option<String> {
    match &h.state().dialog {
        Some(Dialog::Plugin(d)) => Some(d.dialog.title.clone()),
        _ => None,
    }
}

fn dialogs_chain_and_end_by_opening_a_file() {
    load();
    std::fs::create_dir_all(doc_path().parent().unwrap()).unwrap();
    Session::new(wordcraft_doc::Document::new()).run("file.saveAs", &json!({"path": doc_path()})).unwrap();
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    // Nothing unsaved, so the open doesn't ask about it first.
    a.session.dirty = false;
    // Run by a script, the dialog comes back in the result and nothing shows.
    let r = a.execute("plugins.run", json!({"plugin": ID, "command": "browse"})).unwrap();
    assert_eq!(r["dialog"]["title"], "Open from the cloud");
    assert!(a.dialog.is_none());
    // Run from the interface (a ribbon button, a menu item), the app draws it.
    a.run("plugins.run", json!({"plugin": ID, "command": "browse"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert_eq!(shown(&h).as_deref(), Some("Open from the cloud"));
    assert!(h.query_by_label("Documents").is_some(), "the prompt");
    assert!(h.query_by_label("Up").is_some(), "the add-in's button");
    // A click picks the folder, OK runs `browse` with its id, and the answer is the next dialog.
    h.get_by_label("Reports, Folder").click();
    h.run_steps(3);
    h.get_by_label("Go").click();
    h.run_steps(4);
    assert_eq!(shown(&h).as_deref(), Some("Reports"), "dialogs chain");
    // Picking the file opens it as File › Open does, with where it came from.
    h.get_by_label("Budget.docx, 12 KB").click();
    h.run_steps(3);
    h.get_by_label("Open").click();
    h.run_steps(4);
    assert!(h.state().dialog.is_none(), "the last dialog closed: {:?} {:?}", h.state().dialog.as_ref().map(|d| d.name()), h.state().status_msg);
    assert_eq!(h.state().session.path.as_deref(), Some(doc_path().as_path()));
    assert_eq!(h.state().session.location_label.as_deref(), Some("Cloud · Budget.docx"));
    // Cancel just closes.
    h.state_mut().run("plugins.run", json!({"plugin": ID, "command": "browse"})).unwrap();
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
}

fn forms_send_their_fields_and_failures_keep_the_dialog() {
    load();
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.run("plugins.run", json!({"plugin": ID, "command": "signIn"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert_eq!(shown(&h).as_deref(), Some("Sign in"));
    assert!(h.query_by_label("Password").is_some());
    // Without a name the add-in refuses: the dialog stays, saying why.
    assert!(wordcraft_ui_egui::plugin_dialog::press_ok(h.state_mut()));
    h.run_steps(3);
    assert_eq!(shown(&h).as_deref(), Some("Sign in"));
    assert!(h.query_by_label("Type a name.").is_some(), "the add-in's error, as given");
    // A number that isn't one doesn't go to the add-in.
    if let Some(Dialog::Plugin(d)) = &mut h.state_mut().dialog {
        assert!(d.set_field("n", "many"));
    }
    assert!(!wordcraft_ui_egui::plugin_dialog::press_ok(h.state_mut()));
    h.run_steps(3);
    assert!(h.query_by_label("Copies must be a number.").is_some());
    if let Some(Dialog::Plugin(d)) = &mut h.state_mut().dialog {
        assert!(d.set_field("n", "3") && d.set_field("name", "Ada"));
    }
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body), "Hi Ada (3.0)");
}
