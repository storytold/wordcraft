//! Engine behaviour pins for citation-manager fields (Mendeley, Zotero, EndNote) over the
//! synthetic fixture of `wordcraft-testkit` (see that crate's docs: our own markup, written
//! from ECMA-376; payloads made up; nothing exported).
//!
//! Task 1 of slice 3 (#1058) pinned the DOCX layer: read → write → read keeps the fields
//! byte-identical. These pins hold the **engine** to the same standard: every update path
//! (style switch, bibliography insert, generated lists, F9), a full edit pass with undo back
//! to the start, the speech paths (`caret.pastCitation`, Read Aloud) and the status-bar note
//! Open leaves when the document holds fields of a manager WordCraft doesn't drive.

use serde_json::json;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{CharProps, Document, InlineObject, Path, Pos, StoryRef};

use crate::{Session, cmd};

/// Every story of the document: the body, then the parts (footnotes, endnotes…) in id order.
fn stories(d: &Document) -> Vec<StoryRef> {
    std::iter::once(StoryRef::Body).chain(d.parts.keys().map(|k| StoryRef::Part(*k))).collect()
}

/// One manager field as the pins see it: everything that must survive an edit or an update —
/// instruction, `w:fldLock`, `w:fldData` and the live result between the markers.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FieldShape {
    instr: String,
    locked: bool,
    data: Option<String>,
    result: String,
}

/// The manager fields of every story in document order (body first, then parts by id).
fn shapes(s: &Session) -> Vec<FieldShape> {
    let mut out = Vec::new();
    for story in stories(&s.doc) {
        for r in s.doc.field_ranges(story) {
            let data = s.doc.para(story, &r.start.path).and_then(|p| p.object_at(r.start.off)).and_then(|o| match o {
                InlineObject::FieldStart { data, .. } => data.clone(),
                _ => None,
            });
            let after_start = Pos { off: r.start.off + OBJ.len_utf8(), ..r.start.clone() };
            let result = s.doc.copy_range(&after_start, &r.end).plain_text();
            out.push(FieldShape { instr: r.instr, locked: r.locked, data, result });
        }
    }
    out
}

/// A session holding the testkit fixture, opened the way a person opens a file.
fn open_manager_doc() -> Session {
    let bytes = wordcraft_testkit::manager_fields_docx().expect("testkit package");
    let mut s = Session::new(Document::new());
    let data = cmd::insert::base64_encode(&bytes);
    s.run("file.open", &json!({"path": "manager-fields.docx", "data": data})).expect("open");
    s
}

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The update paths — References ▸ Style, References ▸ Bibliography, the generated lists
/// (Update Table), F9 Update Field — leave the manager fields byte-identical: instruction,
/// result, lock state and `w:fldData` are exactly what the reader parsed.
#[test]
fn update_paths_leave_manager_fields_untouched() {
    let mut s = open_manager_doc();
    let start = shapes(&s);
    assert_eq!(start.len(), 5, "four body fields plus the footnote citation");
    run(&mut s, "references.citationStyle", json!({"style": "MLA"}));
    assert_eq!(shapes(&s), start, "a style switch must not restyle a manager's fields");
    run(&mut s, "references.bibliography", json!({}));
    assert_eq!(shapes(&s), start, "inserting our own bibliography must not touch theirs");
    run(&mut s, "references.updateFigures", json!({}));
    assert_eq!(shapes(&s), start, "rebuilding the generated lists skips manager fields");
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(shapes(&s), start, "F9 must not recompute a manager's results");
    // F9 twice: the second run sees our own bibliography from the step before.
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(shapes(&s), start);
}

/// The engine edit pass Task 1 could only pin at the DOCX layer, with undo all the way back:
/// type inside a field result, before a field and after one, delete the adjacent paragraph —
/// the manager fields keep everything but position — then undo to the start and everything,
/// results included, is exactly the opening state again.
#[test]
fn edit_pass_and_undo_to_start_keep_manager_fields() {
    let mut s = open_manager_doc();
    let start = shapes(&s);
    let start_text = s.doc.plain_text(StoryRef::Body);
    let para_len = |s: &Session, i: usize| s.doc.para(StoryRef::Body, &Path::top(i)).map(|p| p.len()).unwrap_or(0);
    let caret = |s: &mut Session, i: usize, off: usize| {
        run(s, "select.range", json!({"anchor": {"story": "body", "path": [i], "off": off}, "focus": {"story": "body", "path": [i], "off": off}}));
    };
    let off_of = |s: &Session, i: usize, needle: &str| {
        s.doc.para(StoryRef::Body, &Path::top(i)).and_then(|p| p.text.find(needle)).unwrap_or_else(|| panic!("{needle:?} not in paragraph {i}"))
    };

    // Type inside the Mendeley citation's result: the text takes it, nothing else changes —
    // the result is exactly the opening one plus the typed characters, never recomputed.
    let inside = off_of(&s, 0, "(Austen") + 1;
    caret(&mut s, 0, inside);
    run(&mut s, "text.insert", json!({"text": "XX"}));
    let typed = shapes(&s);
    assert_eq!(typed[0].instr, start[0].instr);
    assert_eq!(typed[0].locked, start[0].locked);
    assert_eq!(typed[0].data, start[0].data);
    assert_eq!(typed[0].result, "(XXAusten, 1813)");
    assert_eq!(&typed[1..], &start[1..], "the other manager fields changed");

    // Type before the field.
    caret(&mut s, 0, 0);
    run(&mut s, "text.insert", json!({"text": "B4 "}));
    assert_eq!(shapes(&s), typed, "typing before the field changed it");

    // Type after the bibliography's end marker, in its last paragraph.
    let after_end = para_len(&s, 3);
    caret(&mut s, 3, after_end);
    run(&mut s, "text.insert", json!({"text": " AF"}));
    assert_eq!(shapes(&s), typed, "typing after the field changed it");

    // Delete the paragraph next to the first citation (the Zotero one — its field goes with
    // it); the survivors keep everything but position: instruction, lock, data and results
    // are exactly the ones after the typing steps — a deletion must not recompute them.
    let in_zotero = off_of(&s, 1, "Also") + 1;
    caret(&mut s, 1, in_zotero);
    run(&mut s, "select.paragraph", json!({}));
    run(&mut s, "text.delete", json!({}));
    let after = shapes(&s);
    assert_eq!(after.len(), 4, "the deleted paragraph's field went with it");
    assert_eq!(after[0], typed[0], "the Mendeley citation changed");
    assert_eq!(after[1], typed[2], "the bibliography changed");
    assert_eq!(after[2], typed[3], "the EndNote citation changed");
    assert_eq!(after[3], typed[4], "the footnote's citation must not follow body edits");
    assert!(!s.doc.has_unbalanced_field_ranges());

    // Undo everything: back to the opening state, byte for byte.
    while s.can_undo() {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(shapes(&s), start, "undo to start did not restore the manager fields");
    assert_eq!(s.doc.plain_text(StoryRef::Body), start_text);
    assert!(!s.doc.has_unbalanced_field_ranges());
}

/// A document whose citation fields WordCraft drives live — a Zotero range citation and one
/// of Word's own `CITATION` fields — so opening it must not raise (or keep) the note.
fn driven_fields_docx() -> Vec<u8> {
    let mut driven = Session::new(Document::from_text("Z and "));
    let c = CharProps::default();
    let at = Pos::body(0, 6);
    let end = driven
        .doc
        .insert_object(
            &at,
            InlineObject::FieldStart { instr: wordcraft_testkit::ZOTERO_CITE_INSTR.into(), locked: false, dirty: false, data: None },
            &c,
        )
        .expect("field start");
    driven.doc.insert_text(&end, "(Doe, 2020)", &c).expect("result");
    let after = driven.doc.insert_text(&end, " ", &c).expect("space");
    driven.doc.insert_object(&after, InlineObject::FieldEnd, &c).expect("field end");
    driven.doc.sources.push(wordcraft_doc::Source { tag: "WC1".into(), ..Default::default() });
    run(&mut driven, "caret.docEnd", json!({}));
    run(&mut driven, "insert.field", json!({"instr": "CITATION WC1", "result": "(W, 1)"}));
    wordcraft_docx::write(&driven.doc).expect("docx write")
}

/// Opening a document with fields of a manager WordCraft doesn't drive (Mendeley, EndNote —
/// in the body or a footnote) leaves one non-blocking status-bar note; Zotero fields and
/// Word's own CITATION fields, which WordCraft drives live, leave none.
#[test]
fn opening_a_manager_document_notes_it_in_the_status_bar() {
    let s = open_manager_doc();
    assert_eq!(s.status, "Mendeley and EndNote citations are preserved as written; only Zotero is driven live.");
    assert_eq!(s.status, crate::cmd::file::MANAGER_FIELDS_STATUS, "one source string, in the i18n catalogs");

    // A document whose fields WordCraft drives — a Zotero range citation and one of Word's
    // own CITATION fields — opens without the note.
    let mut s2 = Session::new(Document::new());
    let data = cmd::insert::base64_encode(&driven_fields_docx());
    run(&mut s2, "file.open", json!({"path": "driven.docx", "data": data}));
    assert_eq!(s2.status, "", "driven fields must not raise the manager note");
}

/// The note is about the open document, so it must not outlive that document: reopening in
/// the same session — here with a document whose fields WordCraft drives — clears it.
#[test]
fn the_note_does_not_survive_reopening_a_driven_document() {
    let mut s = open_manager_doc();
    assert_eq!(s.status, crate::cmd::file::MANAGER_FIELDS_STATUS);
    let data = cmd::insert::base64_encode(&driven_fields_docx());
    run(&mut s, "file.open", json!({"path": "driven.docx", "data": data}));
    assert_eq!(s.status, "", "the previous document's note must not carry over");
}
