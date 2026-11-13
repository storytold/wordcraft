use super::{run, s};
use serde_json::json;
use wordcraft_doc::{Document, Pos};
const PHRASE: &str = "ខ្ញុំស្រឡាញ់ភាសាខ្មែរ";

#[test]
fn khmer_statistics_agree_for_document_selection_notes_and_edits() {
    let mut s = s();
    let n = if wordcraft_proof::khmer::data_available() { 4 } else { 1 };
    run(&mut s, "document.setText", json!({"text": format!("Hello {PHRASE}")}));
    assert_eq!(s.word_count(), n + 1);
    assert_eq!(run(&mut s, "review.wordCount", json!({}))["words"], n + 1);
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, "Hello ".len()), "focus": Pos::body(0, "Hello ".len() + PHRASE.len())}));
    let selected = run(&mut s, "review.wordCount", json!({}));
    assert_eq!(selected["words"], n);
    assert_eq!(selected["charactersWithSpaces"], PHRASE.chars().count());
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": format!(" {PHRASE}")}));
    assert_eq!(s.word_count(), n * 2 + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.word_count(), n + 1);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.word_count(), n * 2 + 1);
    // A referenced text box contributes its segmented words, not its object marker.
    run(&mut s, "insert.textBox", json!({"text": PHRASE}));
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(s.word_count(), n * 3 + 1);
    assert_eq!(run(&mut s, "review.wordCount", json!({"includeTextBoxes": false}))["words"], n * 2 + 1);
}

#[test]
fn khmer_word_counts_ignore_width_formatting_and_justification() {
    let doc = Document::from_text(PHRASE);
    let original = doc.plain_text(wordcraft_doc::StoryRef::Body);
    let n = if wordcraft_proof::khmer::data_available() { 4 } else { 1 };
    let mut s = crate::Session::new(doc);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "para.align", json!({"value": "justify"}));
    assert_eq!(s.word_count(), n);
    assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), original);
    s.view.web_width = 140.0;
    s.view.mode = wordcraft_layout::ViewMode::Web;
    let _ = s.layout();
    assert_eq!(s.word_count(), n);
    assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), original, "statistics and layout add no spaces");
}

#[test]
fn khmer_word_counts_reach_fields_and_saved_docx_metadata() {
    use std::{
        io::{Cursor, Read},
        sync::Arc,
    };
    use wordcraft_doc::{Block, CharProps, InlineObject, Paragraph, PartKind, StoryRef};
    let n = if wordcraft_proof::khmer::data_available() { 4 } else { 1 };
    let mut doc = Document::from_text(PHRASE);
    let mut header = Paragraph::new();
    header
        .insert_object(
            0,
            InlineObject::Field { instr: "NUMWORDS".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
    let id = doc.add_part(PartKind::Header, vec![Arc::new(Block::Para(header))]);
    let mut s = crate::Session::new(doc);
    let path = wordcraft_doc::Path::top(0);
    run(&mut s, "caret.set", json!({"pos": Pos::new(StoryRef::Part(id), path.clone(), 0)}));
    run(&mut s, "references.updateFields", json!({}));
    let obj = &s.doc.para(StoryRef::Part(id), &path).unwrap().objects[0];
    assert!(matches!(obj, InlineObject::Field { result, .. } if result == &n.to_string()));
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("docProps/app.xml").unwrap().read_to_string(&mut xml).unwrap();
    assert!(xml.contains(&format!("<Words>{n}</Words>")), "{xml}");
}

/// Formatting runs do not split words: bold that starts inside a word still counts it once.
#[test]
fn khmer_word_split_by_a_formatting_run_counts_as_one_word() {
    let n = if wordcraft_proof::khmer::data_available() { 4 } else { 1 };
    let split = "ខ្ញុំស្រឡាញ់ភា".len();
    assert!(PHRASE.is_char_boundary(split) && PHRASE[..split].ends_with('ា'));
    let mut s = crate::Session::new(Document::from_text(PHRASE));
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, split), "focus": Pos::body(0, PHRASE.len())}));
    run(&mut s, "format.bold", json!({}));
    assert!(s.doc.para_at(&Pos::body(0, 0)).unwrap().runs.len() > 1, "the word really is split across runs");
    assert_eq!(s.word_count(), n);
}
