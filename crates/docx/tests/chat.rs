//! The document chat in DOCX (craft-collab `docs/chat.md`): the part `/craft/chat.json`, related
//! from the package root, round trips modelled (not kept as an opaque part); an empty chat writes
//! no part; a malformed part is ignored and the document opens.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use wordcraft_doc::chat::{Author, CONTENT_TYPE, Chat, Message, PART_NAME, RELATIONSHIP};
use wordcraft_doc::{Document, Path, StoryRef};

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut parts = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        parts.insert(name, b);
    }
    parts
}

fn zip(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn text(parts: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(parts.get(name).cloned().unwrap_or_default()).unwrap()
}

fn chat() -> Chat {
    let ann = Author { id: "u-ann".into(), name: "Ann Lee".into(), color: "#3B7DD8".into() };
    let bo = Author { id: "u-bo".into(), name: "Bo".into(), color: "#D8513B".into() };
    Chat {
        cleared_at: Some(1_760_000_000_000),
        messages: vec![
            Message {
                id: "aaaa1111".into(),
                author: ann.clone(),
                at: 1_760_000_100_000,
                text: Some("Shall we start?\nSee https://example.org".into()),
                ..Message::default()
            },
            Message {
                id: "bbbb2222".into(),
                author: bo.clone(),
                at: 1_760_000_200_000,
                text: Some("Yes @Ann Lee".into()),
                reply_to: Some("aaaa1111".into()),
                edited: Some(1_760_000_250_000),
                deleted: false,
            },
            Message { id: "cccc3333".into(), author: bo, at: 1_760_000_300_000, text: None, deleted: true, ..Message::default() },
        ],
    }
}

fn doc_with_chat() -> Document {
    let mut d = Document::from_text("Minutes");
    d.chat = Arc::new(chat());
    d
}

#[test]
fn the_chat_part_round_trips() {
    let d = doc_with_chat();
    let bytes = wordcraft_docx::write(&d).unwrap();
    let parts = unzip(&bytes);
    let part = parts.get(PART_NAME).expect("the chat part is written");
    let json: serde_json::Value = serde_json::from_slice(part).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["messages"].as_array().unwrap().len(), 3);
    let root = text(&parts, "_rels/.rels");
    assert!(root.contains(RELATIONSHIP) && root.contains(PART_NAME), "{root}");
    let types = text(&parts, "[Content_Types].xml");
    assert!(types.contains(&format!("/{PART_NAME}")) && types.contains(CONTENT_TYPE), "{types}");

    let back = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(*back.chat, chat());
    assert!(!back.passthrough.keys().any(|k| k.contains("chat")), "modelled, not kept: {:?}", back.passthrough.keys());
    assert!(!back.passthrough.values().any(|v| String::from_utf8_lossy(v).contains("craft/chat")), "no kept relationship to it");
    // Saved again: one part, one relationship.
    let again = unzip(&wordcraft_docx::write(&back).unwrap());
    assert_eq!(text(&again, "_rels/.rels").matches(RELATIONSHIP).count(), 1);
    assert_eq!(again.get(PART_NAME), parts.get(PART_NAME));
    // The native JSON keeps it too.
    let native: Document = serde_json::from_slice(&serde_json::to_vec(&back).unwrap()).unwrap();
    assert_eq!(*native.chat, chat());
}

#[test]
fn an_empty_chat_writes_no_part() {
    let d = Document::from_text("Plain");
    let parts = unzip(&wordcraft_docx::write(&d).unwrap());
    assert!(!parts.contains_key(PART_NAME));
    assert!(!text(&parts, "_rels/.rels").contains(RELATIONSHIP));
    assert!(!serde_json::to_string(&d).unwrap().contains("\"chat\""), "the native JSON leaves an empty chat out");
}

#[test]
fn a_malformed_chat_part_is_ignored() {
    let good = unzip(&wordcraft_docx::write(&doc_with_chat()).unwrap());
    for bad in [&b"{not json"[..], br#"{"version": 9, "messages": []}"#, br#"[1, 2]"#, br#"{"version": 1}"#] {
        let mut parts = good.clone();
        parts.insert(PART_NAME.into(), bad.to_vec());
        let d = wordcraft_docx::read(&zip(&parts)).expect("the document still opens");
        assert!(d.chat.is_empty(), "{}", String::from_utf8_lossy(bad));
        assert_eq!(d.para(StoryRef::Body, &Path::top(0)).map(|p| p.text.clone()).as_deref(), Some("Minutes"));
        // Nor kept as an opaque part.
        let again = unzip(&wordcraft_docx::write(&d).unwrap());
        assert!(!again.contains_key(PART_NAME));
    }
    // Malformed messages are skipped, the rest kept.
    let mut parts = good.clone();
    parts.insert(
        PART_NAME.into(),
        br#"{"version": 1, "messages": [{"id": "BAD", "author": {"id": "a", "name": "A", "color": ""}, "at": 1, "text": "x"},
            {"id": "okok0000", "author": {"id": "a", "name": "A", "color": ""}, "at": 1, "text": "kept"}]}"#
            .to_vec(),
    );
    let d = wordcraft_docx::read(&zip(&parts)).unwrap();
    assert_eq!(d.chat.messages.len(), 1);
    assert_eq!(d.chat.messages[0].text(), "kept");
}
