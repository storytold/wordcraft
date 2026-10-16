//! `collab/snapshot {file: true, format}`: the file comes in the format the transport asks for
//! (craft-live's `save {format}`: the Nextcloud file's own) when WordCraft writes it, and as
//! `.docx` otherwise, named after the document.

use serde_json::{Value as Json, json};
use wordcraft_engine::Session;
use wordcraft_engine::collab::{IN_MEMORY, file_name_as, snapshot_format};
use wordcraft_engine::doc::Document;
use wordcraft_plugin::push::{Incoming, Pushed};

fn shared(name: &str) -> Session {
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "Live in Nextcloud"})).unwrap();
    s.file_name = Some(name.into());
    s.collab_join_in_memory(json!({"room": "nc:cloud:7", "mode": "share", "title": name})).unwrap();
    s.collab_take_outgoing();
    s
}

fn snapshot(s: &mut Session, params: Json) -> Json {
    let req = json!({"jsonrpc": "2.0", "id": "snap", "method": "collab/snapshot", "params": params});
    s.collab_poll_pushed(10, vec![Pushed { plugin: IN_MEMORY.into(), incoming: Incoming::Message(req) }]);
    let out = s.collab_take_outgoing();
    out.into_iter().find(|m| m["id"] == "snap").expect("an answer")
}

#[test]
fn the_file_comes_in_the_format_asked_for() {
    let mut s = shared("Notes.odt");
    let a = snapshot(&mut s, json!({"room": "nc:cloud:7", "file": true, "format": "odt"}));
    assert_eq!(a["result"]["name"], "Notes.odt");
    assert_eq!(a["result"]["format"], "odt");
    let bytes = wordcraft_collab::b64_decode(a["result"]["file"].as_str().unwrap()).unwrap();
    let doc = wordcraft_engine::io::open_bytes("Notes.odt", &bytes).unwrap();
    assert!(wordcraft_collab::canonical(&doc).to_string().contains("Live in Nextcloud"));

    // A macro-enabled package keeps its flavour; the extension is case-insensitive.
    let a = snapshot(&mut s, json!({"room": "nc:cloud:7", "file": true, "format": ".DOCM"}));
    assert_eq!(a["result"]["name"], "Notes.docm");
    assert_eq!(a["result"]["format"], "docm");
}

#[test]
fn without_a_format_or_with_one_wordcraft_cant_write_it_is_docx() {
    let mut s = shared("Plan.docx");
    let a = snapshot(&mut s, json!({"room": "nc:cloud:7", "file": true}));
    assert_eq!(a["result"]["name"], "Plan.docx");
    assert_eq!(a["result"]["format"], "docx");
    let a = snapshot(&mut s, json!({"room": "nc:cloud:7", "file": true, "format": "pages"}));
    assert_eq!(a["result"]["name"], "Plan.docx", "an unknown format: .docx, and craft-live saves a copy");
    let bytes = wordcraft_collab::b64_decode(a["result"]["file"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"PK"));
}

#[test]
fn names_follow_the_format() {
    assert_eq!(file_name_as("Report.odt", "docx"), "Report.docx");
    assert_eq!(file_name_as("Report.DOCX", "docx"), "Report.DOCX");
    assert_eq!(file_name_as("Report", "odt"), "Report.odt");
    assert_eq!(file_name_as("v1.2 notes", "docx"), "v1.2 notes.docx");
    assert_eq!(file_name_as(".docx", "docx"), ".docx.docx");
    assert_eq!(snapshot_format(Some(" RTF ")), "rtf");
    assert_eq!(snapshot_format(Some("doc")), "docx", "WordCraft doesn't write Word 97-2003 files");
    assert_eq!(snapshot_format(None), "docx");
}
