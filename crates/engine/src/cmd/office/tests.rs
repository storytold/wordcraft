//! `office.sync`: what an Office add-in's task pane does to the document through the Word
//! JavaScript API's objects.

use serde_json::{Value, json};
use wordcraft_doc::{Document, StoryRef};

use crate::Session;

fn manifest(id: &str, permission: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<OfficeApp xmlns="http://schemas.microsoft.com/office/appforoffice/1.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="TaskPaneApp">
  <Id>{id}</Id><Version>1.0.0.0</Version><ProviderName>Tests</ProviderName><DefaultLocale>en-US</DefaultLocale>
  <DisplayName DefaultValue="Test pane"/><Description DefaultValue="Tests"/>
  <Hosts><Host Name="Document"/></Hosts>
  <DefaultSettings><SourceLocation DefaultValue="https://tests.example/pane.html"/></DefaultSettings>
  <Permissions>{permission}</Permissions>
</OfficeApp>"#
    )
}

/// One Office add-in loaded per test (ids differ: tests run in parallel), unloaded on drop.
struct Addin(String);

impl Addin {
    fn new(id: &str, permission: &str) -> Addin {
        let m = wordcraft_plugin::office::parse_manifest(&manifest(id, permission), wordcraft_plugin::HOST_ID).unwrap();
        let plugin = m.id.clone();
        wordcraft_plugin::add(m, std::env::temp_dir());
        Addin(plugin)
    }
}

impl Drop for Addin {
    fn drop(&mut self) {
        wordcraft_plugin::unload(&self.0);
    }
}

fn sync(s: &mut Session, a: &Addin, actions: Value) -> Result<Vec<Value>, Value> {
    match s.run("office.sync", &json!({"addin": a.0, "actions": actions})) {
        Ok(r) => Ok(r["results"].as_array().cloned().unwrap()),
        Err(e) => {
            let text = e.to_string();
            Err(serde_json::from_str(text.trim_start_matches(|c| c != '{')).unwrap_or(json!({"message": text})))
        }
    }
}

fn ok(s: &mut Session, a: &Addin, actions: Value) -> Vec<Value> {
    sync(s, a, actions).unwrap_or_else(|e| panic!("{e}"))
}

fn session(text: &str) -> Session {
    Session::new(Document::from_text(text))
}

fn body() -> Value {
    json!([{"p": "document"}, {"p": "body"}])
}

fn paragraphs() -> Value {
    json!([{"p": "document"}, {"p": "paragraphs"}])
}

fn by_id(id: &Value) -> Value {
    json!([{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [id]}])
}

fn then(base: &Value, extra: Value) -> Value {
    let mut p = base.as_array().cloned().unwrap();
    p.extend(extra.as_array().cloned().unwrap());
    Value::Array(p)
}

#[test]
fn paragraphs_are_added_formatted_read_and_deleted() {
    let a = Addin::new("0d5c7a1e-0000-4bcd-9eaf-000000000001", "ReadWriteDocument");
    let mut s = session("First\nSecond");
    let undo = s.undo_depth();
    let r = ok(
        &mut s,
        &a,
        json!([
            {"op": "call", "path": body(), "method": "insertParagraph", "args": ["Third", "End"]},
            {"op": "set", "path": [{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [{"result": 0}]}, {"p": "font"}], "prop": "bold", "value": true},
            {"op": "set", "path": [{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [{"result": 0}]}], "prop": "alignment", "value": "Centered"},
            {"op": "load", "path": paragraphs(), "props": ["items", "text", "alignment"]},
            {"op": "load", "path": then(&paragraphs(), json!([{"m": "getLast"}, {"p": "font"}])), "props": ["bold", "size"]},
            {"op": "load", "path": then(&body(), json!([{"p": "font"}])), "props": ["bold"]},
            {"op": "load", "path": body(), "props": ["text"]},
        ]),
    );
    let third = r[0]["id"].clone();
    let items = r[3]["items"].as_array().unwrap();
    assert_eq!(items.iter().map(|i| i["text"].as_str().unwrap()).collect::<Vec<_>>(), ["First", "Second", "Third"]);
    assert_eq!(items[2]["alignment"], "Centered");
    assert_eq!(items[2]["id"], third);
    assert_eq!(r[4]["bold"], true);
    assert_eq!(r[5]["bold"], Value::Null, "mixed reads are null, as in Word");
    assert_eq!(r[6]["text"], "First\nSecond\nThird");
    assert_eq!(s.undo_depth(), undo + 1, "one sync, one undo step");
    // Reads alone make no undo step.
    ok(&mut s, &a, json!([{"op": "load", "path": body(), "props": ["text"]}]));
    assert_eq!(s.undo_depth(), undo + 1);
    // Text before, a paragraph before, then the first's style; ids stay right within the sync.
    let first = ok(&mut s, &a, json!([{"op": "load", "path": then(&paragraphs(), json!([{"m": "getFirst"}])), "props": ["id"]}]))[0]["id"].clone();
    let r = ok(
        &mut s,
        &a,
        json!([
            {"op": "call", "path": by_id(&first), "method": "insertParagraph", "args": ["Title", "Before"]},
            {"op": "call", "path": by_id(&json!({"result": 0})), "method": "insertText", "args": ["My ", "Start"]},
            {"op": "set", "path": by_id(&json!({"result": 0})), "prop": "styleBuiltIn", "value": "Heading1"},
            {"op": "load", "path": paragraphs(), "props": ["items", "text", "styleBuiltIn"]},
        ]),
    );
    let items = r[3]["items"].as_array().unwrap();
    assert_eq!(items[0]["text"], "My Title");
    assert_eq!(items[0]["styleBuiltIn"], "Heading1");
    assert_eq!(items[1]["text"], "First");
    // Deleting a paragraph; undo puts it back.
    ok(&mut s, &a, json!([{"op": "call", "path": then(&paragraphs(), json!([{"m": "getLast"}])), "method": "delete"}]));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "My Title\nFirst\nSecond");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "My Title\nFirst\nSecond\nThird");
}

#[test]
fn the_selection_ranges_and_search() {
    let a = Addin::new("0d5c7a1e-0000-4bcd-9eaf-000000000002", "ReadWriteDocument");
    let mut s = session("The quick fox\nA quick reply");
    s.run("select.text", &json!({"text": "quick"})).unwrap();
    let sel = json!([{"p": "document"}, {"m": "getSelection"}]);
    let r = ok(
        &mut s,
        &a,
        json!([
            {"op": "load", "path": sel, "props": ["text", "isEmpty"]},
            {"op": "call", "path": [{"p": "document"}], "method": "getSelectedData", "args": ["text"]},
            {"op": "call", "path": sel, "method": "insertText", "args": ["slow", "Replace"]},
            {"op": "call", "path": [{"p": "document"}, {"m": "getRangeById", "a": [{"result": 2}]}], "method": "select", "args": ["End"]},
            {"op": "call", "path": body(), "method": "search", "args": ["QUICK", {"matchCase": false}]},
        ]),
    );
    assert_eq!(r[0], json!({"text": "quick", "isEmpty": false}));
    assert_eq!(r[1], json!("quick"));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The slow fox\nA quick reply");
    assert_eq!(s.sel.focus.off, "The slow".len(), "select(End) is the selection afterwards: {r:?}");
    let found = r[4]["items"].as_array().unwrap();
    assert_eq!(found.len(), 1);
    // Typing a paragraph break is insertText("\n").
    let hit = found[0]["id"].clone();
    ok(
        &mut s,
        &a,
        json!([{"op": "set", "path": [{"p": "document"}, {"m": "getRangeById", "a": [hit]}, {"p": "font"}], "prop": "highlightColor", "value": "Yellow"}]),
    );
    ok(&mut s, &a, json!([{"op": "call", "path": [{"p": "document"}, {"m": "getSelection"}], "method": "insertText", "args": ["\n", "Replace"]}]));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The slow\n fox\nA quick reply");
    // The document's properties.
    ok(&mut s, &a, json!([{"op": "set", "path": [{"p": "document"}, {"p": "properties"}], "prop": "title", "value": "Foxes"}]));
    let r = ok(&mut s, &a, json!([{"op": "load", "path": [{"p": "document"}, {"p": "properties"}], "props": ["title", "author"]}]));
    assert_eq!(r[0]["title"], "Foxes");
}

#[test]
fn a_failing_action_changes_nothing_and_permissions_hold() {
    let a = Addin::new("0d5c7a1e-0000-4bcd-9eaf-000000000003", "ReadWriteDocument");
    let mut s = session("Only");
    let undo = s.undo_depth();
    let e = sync(
        &mut s,
        &a,
        json!([
            {"op": "call", "path": body(), "method": "insertParagraph", "args": ["More", "End"]},
            {"op": "load", "path": by_id(&json!("p:99")), "props": ["text"]},
        ]),
    )
    .unwrap_err();
    assert_eq!(e["code"], "ItemNotFound", "{e}");
    assert_eq!(e["action"], 1);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Only", "the inserted paragraph was taken back");
    assert_eq!(s.undo_depth(), undo);
    for bad in [
        json!({"op": "load", "path": [{"p": "nowhere"}], "props": ["text"]}),
        json!({"op": "load", "path": body(), "props": ["nope"]}),
        json!({"op": "set", "path": then(&body(), json!([{"p": "font"}])), "prop": "size", "value": -3}),
        json!({"op": "call", "path": body(), "method": "insertParagraph", "args": ["a\nb", "End"]}),
        json!({"op": "load", "path": [{"p": "document"}, {"m": "getRangeById", "a": ["r:0:99-0:100"]}], "props": ["text"]}),
    ] {
        assert!(sync(&mut s, &a, json!([bad])).is_err(), "{bad}");
    }
    let read_only = Addin::new("0d5c7a1e-0000-4bcd-9eaf-000000000004", "ReadDocument");
    assert!(sync(&mut s, &read_only, json!([{"op": "load", "path": body(), "props": ["text"]}])).is_ok());
    let e = sync(&mut s, &read_only, json!([{"op": "call", "path": body(), "method": "insertText", "args": ["x", "End"]}])).unwrap_err();
    assert_eq!(e["code"], "AccessDenied");
    // Settings need no permission and load whole.
    let none = Addin::new("0d5c7a1e-0000-4bcd-9eaf-000000000005", "Restricted");
    ok(&mut s, &none, json!([{"op": "call", "path": [{"p": "settings"}], "method": "save", "args": [{"k": 1}]}]));
    assert_eq!(ok(&mut s, &none, json!([{"op": "load", "path": [{"p": "settings"}], "props": []}]))[0], json!({"k": 1}));
}
