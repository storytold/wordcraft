//! The document's ribbon buttons (Developer › Ribbon Buttons): added, edited and removed as
//! undoable steps, saved as the package's ribbon part, and pressed to run their macro through a
//! macro add-in with the button as `control {id, tag}`. The add-in is this test binary started
//! again with `--serve-macros` (no libtest harness, so standard output is the protocol's alone).

use std::io::{BufRead, Write};

use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_engine::doc::{Document, StoryRef};

const PLUGIN: &str = "org.wordcraft.test-ribbon-macros";

fn main() {
    if std::env::args().any(|a| a == "--serve-macros") {
        serve();
        return;
    }
    let tests: [(&str, fn()); 2] = [
        ("ribbon_buttons_run_their_macro_and_are_saved", ribbon_buttons_run_their_macro_and_are_saved),
        ("a_richer_ribbon_from_a_file_runs_but_is_not_edited", a_richer_ribbon_from_a_file_runs_but_is_not_edited),
    ];
    for (name, t) in tests {
        t();
        println!("test {name} ... ok");
    }
}

/// The add-in: `Module1.Pressed` adds a paragraph saying which button called it (`id/tag`).
fn serve() {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut out = std::io::stdout();
    let mut next = 0u32;
    while let Some(Ok(line)) = lines.next() {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { break };
        let p = msg["params"].clone();
        let answer = match msg["method"].as_str() {
            Some("macros/list") => json!({"result": {"macros": [{"name": "Module1.Pressed", "module": "Module1", "procedure": "Pressed"}]}}),
            Some("macros/run") if p["name"] == "Module1.Pressed" => {
                let text = format!("{}/{}", p["control"]["id"].as_str().unwrap_or("none"), p["control"]["tag"].as_str().unwrap_or(""));
                next += 1;
                let hid = format!("h{next}");
                let action = json!({"op": "call", "path": [{"p": "document"}, {"p": "body"}], "method": "insertParagraph", "args": [text, "End"]});
                writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": hid, "method": "host/sync", "params": {"actions": [action]}})).unwrap();
                out.flush().unwrap();
                for line in lines.by_ref() {
                    let reply: Value = serde_json::from_str(&line.unwrap()).unwrap();
                    if reply["id"] == json!(hid) {
                        break;
                    }
                }
                json!({"result": {}})
            }
            Some("macros/run") => json!({"error": {"code": 1, "message": format!("Sub or Function not defined: {}", p["name"])}}),
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

fn load(s: &mut Session) {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-ribbon-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let manifest = json!({"id": PLUGIN, "name": "Test macros", "apps": ["wordcraft"], "macros": {"language": "TestBasic"}, "run": {"command": exe, "args": ["--serve-macros"]}});
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    s.run("plugins.load", &json!({"path": dir})).unwrap();
}

fn unload() {
    let mut s = Session::new(Document::new());
    assert!(s.run("plugins.unload", &json!({"id": PLUGIN})).is_ok());
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

fn ribbon_buttons_run_their_macro_and_are_saved() {
    let mut s = Session::new(Document::from_text("Dear reader"));
    load(&mut s);
    assert_eq!(s.run("developer.ribbon", &json!({})).unwrap(), json!({"tabs": [], "editable": true}));
    let id =
        s.run("developer.addRibbonButton", &json!({"macro": "Module1.Pressed", "label": "Go", "tab": "references", "tag": "north"})).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
    s.run("developer.addRibbonButton", &json!({"macro": "Module1.Fill", "large": false})).unwrap();
    let r = s.run("developer.ribbon", &json!({})).unwrap();
    assert_eq!((r["tabs"][0]["tab"].as_str(), r["tabs"][0]["builtin"].as_str()), (Some("References"), Some("TabReferences")), "{r}");
    assert_eq!(r["tabs"][0]["groups"][0]["buttons"][0]["label"], "Go");
    assert_eq!(r["tabs"][1]["tab"], "Macros", "a tab of its own by default");
    assert_eq!(r["tabs"][1]["groups"][0]["buttons"][0]["label"], "Fill", "labelled after the macro");
    assert_eq!(r["tabs"][1]["groups"][0]["buttons"][0]["large"], false);
    // A second button on the same tab and group joins them.
    s.run("developer.addRibbonButton", &json!({"macro": "Module1.Other", "tab": "References"})).unwrap();
    assert_eq!(s.doc.ribbon.as_ref().unwrap().tabs.len(), 2);

    // Pressing it runs the macro with the button, under Macro Security like any macro, as one step.
    assert!(s.run("developer.pressRibbonButton", &json!({"id": id})).unwrap_err().to_string().contains("disabled"));
    s.run("developer.enableMacros", &json!({})).unwrap();
    let depth = s.undo_depth();
    s.run("developer.pressRibbonButton", &json!({"id": id})).unwrap();
    assert_eq!(text(&s), format!("Dear reader\n{id}/north"));
    assert_eq!(s.undo_depth(), depth + 1);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s), "Dear reader");
    assert!(s.run("developer.pressRibbonButton", &json!({"id": "nope"})).is_err());

    // Saved with the document as the ribbon part, and read back.
    let bytes = wordcraft_docx::write_as(&s.doc, wordcraft_docx::Flavor::MacroDocument).unwrap();
    assert_eq!(wordcraft_docx::read(&bytes).unwrap().ribbon, s.doc.ribbon);

    // Edits and removals are undoable; the last button gone, the document has no ribbon.
    s.run("developer.editRibbonButton", &json!({"id": id, "label": "Run it", "image": "HappyFace", "tag": ""})).unwrap();
    let b = s.doc.ribbon.as_ref().unwrap().button(&id).unwrap().clone();
    assert_eq!((b.label.as_str(), b.image.as_deref(), b.tag.as_deref()), ("Run it", Some("HappyFace"), None));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.ribbon.as_ref().unwrap().button(&id).unwrap().label, "Go");
    assert!(s.run("developer.editRibbonButton", &json!({"id": id, "image": "<img>"})).is_err());
    assert!(s.run("developer.editRibbonButton", &json!({"id": "nope", "label": "x"})).is_err());
    assert!(s.run("developer.addRibbonButton", &json!({"macro": "x\"/><evil"})).is_err());
    assert!(s.run("developer.addRibbonButton", &json!({})).is_err());
    let all: Vec<String> = s.doc.ribbon.as_ref().unwrap().ids().map(str::to_string).filter(|i| i.starts_with("craftButton")).collect();
    assert_eq!(all.len(), 3);
    for b in &all {
        s.run("developer.removeRibbonButton", &json!({"id": b})).unwrap();
    }
    assert!(s.doc.ribbon.is_none());
    assert!(s.run("developer.removeRibbonButton", &json!({"id": all[0]})).is_err());
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.doc.ribbon.as_ref().is_some_and(|r| r.button(&all[2]).is_some()));
    unload();
}

fn a_richer_ribbon_from_a_file_runs_but_is_not_edited() {
    let rich = r#"<customUI xmlns="http://schemas.microsoft.com/office/2009/07/customui" onLoad="Loaded"><ribbon><tabs><tab idMso="TabHome"><group id="g" label="Mine"><menu id="m" label="More"><button id="inMenu" label="Inside" tag="t1" onAction="Module1.Pressed"/></menu></group></tab></tabs></ribbon></customUI>"#;
    let doc_xml = r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#;
    let root = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2007/relationships/ui/extensibility" Target="customUI/customUI14.xml"/></Relationships>"#;
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in [("_rels/.rels", root), ("word/document.xml", doc_xml), ("customUI/customUI14.xml", rich)] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(body.as_bytes()).unwrap();
    }
    let bytes = zw.finish().unwrap().into_inner();
    let mut s = Session::new(wordcraft_docx::read(&bytes).unwrap());
    load(&mut s);
    let r = s.run("developer.ribbon", &json!({})).unwrap();
    assert_eq!(r["editable"], false, "{r}");
    assert_eq!(r["tabs"][0]["tab"], "Home");
    assert_eq!(r["tabs"][0]["groups"][0]["buttons"][0]["id"], "inMenu");
    let e = s.run("developer.addRibbonButton", &json!({"macro": "Module1.Pressed"})).unwrap_err().to_string();
    assert!(e.contains("doesn't change it"), "{e}");
    assert!(s.run("developer.removeRibbonButton", &json!({"id": "inMenu"})).is_err());
    s.run("developer.macroSecurity", &json!({"level": "enable"})).unwrap();
    s.run("developer.pressRibbonButton", &json!({"id": "inMenu"})).unwrap();
    assert_eq!(text(&s), "Body\ninMenu/t1");
    s.run("developer.macroSecurity", &json!({"level": "prompt"})).unwrap();
    unload();
}
