//! Hostile documents and calls stay cheap: work grows with the document, not with how often a
//! crafted file or call repeats something (#3692).

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wordcraft_doc::para::{NoteKind, OBJ};
use wordcraft_doc::{CharProps, Document, InlineObject, Paragraph, PartKind, Run, StoryRef, para_block};
use wordcraft_engine::Session;

use crate::wire::Call;
use crate::{Bridge, Headless, fields};

const CITE: &str = r#"ADDIN ZOTERO_ITEM CSL_CITATION {"citationID":"k1","citationItems":[{"id":7}]}"#;

/// A paragraph of `objects`, one marker each, in one plain run.
fn objects_para(objects: Vec<InlineObject>) -> Paragraph {
    let mut p = Paragraph::new();
    p.text = std::iter::repeat_n(OBJ, objects.len()).collect();
    p.runs = vec![Run { len: p.text.len(), props: CharProps::default() }];
    p.objects = objects;
    p
}

/// `n` empty Zotero citation fields in a row.
fn citations(n: usize) -> Vec<InlineObject> {
    (0..n).flat_map(|_| [InlineObject::FieldStart { instr: CITE.into(), locked: false, dirty: false }, InlineObject::FieldEnd]).collect()
}

// Finding 12: one endnote referenced many times is listed once, not once per reference.
#[test]
fn a_note_referenced_many_times_is_listed_once() {
    let mut doc = Document::from_text("");
    let note = doc.add_part(PartKind::Endnote, vec![para_block(objects_para(citations(200)))]);
    let refs = (0..5_000).map(|_| InlineObject::NoteRef { kind: NoteKind::Endnote, id: note, custom: String::new() }).collect();
    doc.body = vec![para_block(objects_para(refs))];
    // No clock: listing the note once per reference gave a million fields, which the count catches.
    let list = fields::list(&doc);
    assert_eq!(list.len(), 200, "each field once, at the note's first reference");
    assert!(list.iter().all(|f| f.start.story == StoryRef::Part(note) && f.note_index == 1));
}

#[test]
fn listing_stops_at_the_field_budget() {
    let mut doc = Document::from_text("");
    // Many short paragraphs: the per-paragraph object scan is another finding's (#3614).
    doc.body = (0..fields::MAX_FIELDS / 100 + 1).map(|_| para_block(objects_para(citations(100)))).collect();
    assert_eq!(fields::list(&doc).len(), fields::MAX_FIELDS);
    // Codes count too: a few huge codes fill the byte budget before the field budget.
    let big = format!("{CITE}{}", "x".repeat(fields::MAX_CODE_BYTES / 4 - 1000));
    let huge = (0..5).flat_map(|_| [InlineObject::FieldStart { instr: big.clone(), locked: false, dirty: false }, InlineObject::FieldEnd]).collect();
    doc.body = vec![para_block(objects_para(huge))];
    assert_eq!(fields::list(&doc).len(), 4);
}

// Finding 16: rich text with alternating formatting is built in one pass, not run by run with
// every earlier run merged again.
#[test]
fn rich_text_with_many_runs_is_linear() {
    let html: String = "<i>a</i>b".repeat(50_000);
    let t = Instant::now();
    let frag = crate::rich::fragment(&html, true, &CharProps::default());
    let elapsed = t.elapsed();
    let Some(wordcraft_doc::Block::Para(p)) = frag.blocks.first() else { panic!("no paragraph") };
    assert_eq!(p.text, "ab".repeat(50_000));
    assert_eq!(p.runs.len(), 100_000);
    assert_eq!(p.runs.iter().map(|r| r.len).sum::<usize>(), p.text.len());
    // Under half a second on a loaded machine; merging again after every run ran past 9 minutes.
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
    // Neighbours with the same formatting are one run.
    let frag = crate::rich::fragment("<i>a</i><i>b</i>c", true, &CharProps::default());
    let Some(wordcraft_doc::Block::Para(p)) = frag.blocks.first() else { panic!("no paragraph") };
    assert_eq!((p.text.as_str(), p.runs.len()), ("abc", 2));
    assert_eq!(p.props_of_char(1).italic, Some(true));
}

/// Call `method` on a bridge over `s`.
fn call(b: &mut Bridge, s: &mut Session, method: &str, args: Value) -> Result<Value, String> {
    let Value::Array(args) = args else { return Err("args must be an array".into()) };
    b.handle(s, &mut Headless::default(), &Call::new(method, args))
}

// Finding 32: Document_convert checks each distinct id once against one list of the fields.
#[test]
fn convert_with_one_id_repeated_is_one_check() {
    let mut doc = Document::from_text("");
    // Checking each id against a fresh list of the fields took 81 s with 300 of them; with 1,000
    // that is minutes, against milliseconds for one indexed list.
    doc.body = (0..1000).map(|_| para_block(objects_para(citations(1)))).collect();
    let mut s = Session::new(doc);
    let mut b = Bridge::new();
    call(&mut b, &mut s, "Application_getActiveDocument", json!([3])).unwrap();
    let got = call(&mut b, &mut s, "Document_getFields", json!([1, "Field"])).unwrap();
    let first = got[0][0].clone();
    let ids = vec![first.clone(); 100_000];
    let before = format!("{:?}", s.doc.body);
    let t = Instant::now();
    call(&mut b, &mut s, "Document_convert", json!([1, ids, 0, 0])).unwrap();
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    assert_eq!(format!("{:?}", s.doc.body), before, "already in the text: nothing to do");
    // An unknown id is refused before anything moves.
    let r = call(&mut b, &mut s, "Document_convert", json!([1, [first.clone(), 999_999_999u64], 0, [1, 1]]));
    assert!(r.is_err());
    assert!(s.doc.parts.is_empty(), "nothing converted");
}

#[test]
fn convert_with_a_repeated_id_takes_its_last_note_type() {
    let mut doc = Document::from_text("");
    doc.body = (0..3).map(|_| para_block(objects_para(citations(1)))).collect();
    let mut s = Session::new(doc);
    let mut b = Bridge::new();
    call(&mut b, &mut s, "Application_getActiveDocument", json!([3])).unwrap();
    let got = call(&mut b, &mut s, "Document_getFields", json!([1, "Field"])).unwrap();
    let (a, c) = (got[0][0].clone(), got[0][2].clone());
    call(&mut b, &mut s, "Document_convert", json!([1, [a.clone(), c, a], 0, [2, 1, 1]])).unwrap();
    let kinds: Vec<PartKind> = s.doc.parts.values().map(|p| p.kind).collect();
    assert_eq!(kinds, vec![PartKind::Footnote, PartKind::Footnote]);
    // Ids stay valid: the fields are where Zotero now expects them.
    let got = call(&mut b, &mut s, "Document_getFields", json!([1, "Field"])).unwrap();
    assert_eq!(got[2], json!([1, 0, 2]));
}
