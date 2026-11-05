//! Mendeley-style citation fields: every manager variant and hard shape survives
//! open → (edit) → save as a live field. The packages are built by `wordcraft-testkit`
//! from our own markup (ECMA-376 shapes, synthetic payloads — nothing exported).
//!
//! Assertions are model-level: read → write → read, then compare the first parse to the
//! second (instruction, result, `locked`, `w:fldData`, field boundaries, view state).

use wordcraft_doc::para::NoteKind;
use wordcraft_doc::{CharProps, Document, InlineObject, Path, Pos, StoryRef};
use wordcraft_docx::{read, write};
use wordcraft_testkit::{
    EN_CITE_INSTR, FLD_DATA, MENDELEY_BIBL_INSTR, MENDELEY_CITE, NOTE_CITE_INSTR, NOTE_FLD_DATA, ZOTERO_CITE_INSTR, hostile_fields_docx,
    manager_fields_docx,
};

/// The reader's instruction cap (`MAX_INSTR` in `read/story.rs`): oversized codes are
/// truncated, never fatal.
const INSTR_CAP: usize = 2 * 1024 * 1024;

fn pkg() -> Vec<u8> {
    manager_fields_docx().expect("testkit package")
}

fn para(d: &Document, i: usize) -> wordcraft_doc::Paragraph {
    d.para(StoryRef::Body, &Path::top(i)).expect("para").clone()
}

fn para_text(d: &Document, i: usize) -> String {
    d.para(StoryRef::Body, &Path::top(i)).map(|p| p.plain_text().to_string()).unwrap_or_default()
}

/// The `FieldStart` objects of a story, in order: (instr, locked, data).
fn field_starts(d: &Document, story: StoryRef) -> Vec<(String, bool, Option<String>)> {
    let mut out = Vec::new();
    let mut stack = vec![d.story(story).expect("story").clone()];
    while let Some(blocks) = stack.pop() {
        for b in blocks.iter() {
            match b.as_para() {
                Some(p) => {
                    for o in &p.objects {
                        if let InlineObject::FieldStart { instr, locked, data, .. } = o {
                            out.push((instr.clone(), *locked, data.clone()));
                        }
                    }
                }
                None => {
                    if let Some(t) = b.as_table() {
                        for row in &t.rows {
                            for c in &row.cells {
                                stack.push(c.blocks.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// The first `NoteRef` of a paragraph: the part id of the note it points at.
fn note_part(d: &Document, i: usize) -> u32 {
    let p = para(d, i);
    for o in &p.objects {
        if let InlineObject::NoteRef { kind: NoteKind::Footnote, id, .. } = o {
            return *id;
        }
    }
    panic!("no footnote reference in paragraph {i}");
}

/// Everything the first parse must show — and the second parse must show again.
fn check_manager_fields(d: &Document) {
    // Results, with the bibliography spanning paragraphs 2–3.
    let texts: Vec<String> = (0..8).map(|i| para_text(d, i)).collect();
    assert_eq!(
        texts,
        [
            "See (Austen, 1813).",
            "Also (Doe, 2020).",
            "Austen, J. (1813). Pride and Prejudice. London.",
            "Doe, J. (2020). A Study. Somewhere.",
            "Cf. note.",
            "Tracked (Roe, 2019).",
            "Page 1.",
            "After.",
        ]
    );
    // The result keeps its formatting.
    let p0 = para(d, 0);
    let at = p0.text.find("1813").expect("italic year");
    assert_eq!(p0.props_of_char(at).italic, Some(true));

    // Field boundaries: four live fields, the bibliography crossing a paragraph mark.
    let r = d.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 4, "{r:?}");
    assert_eq!((r[0].instr.as_str(), r[0].locked), (MENDELEY_CITE, true));
    assert_eq!((r[0].start.path.clone(), r[0].end.path.clone()), (Path::top(0), Path::top(0)));
    assert_eq!((r[1].instr.as_str(), r[1].locked), (ZOTERO_CITE_INSTR, false));
    assert_eq!((r[2].instr.as_str(), r[2].locked), (MENDELEY_BIBL_INSTR, false));
    assert_eq!((r[2].start.path.clone(), r[2].end.path.clone()), (Path::top(2), Path::top(3)));
    assert_eq!((r[3].instr.as_str(), r[3].locked), (EN_CITE_INSTR, false));
    assert!(!d.has_unbalanced_field_ranges());

    // The field code's own pieces: the split instruction joined, and the opaque `w:fldData`.
    let starts = field_starts(d, StoryRef::Body);
    assert_eq!(
        starts,
        [
            (MENDELEY_CITE.to_string(), true, Some(FLD_DATA.to_string())),
            (ZOTERO_CITE_INSTR.to_string(), false, None),
            (MENDELEY_BIBL_INSTR.to_string(), false, None),
            (EN_CITE_INSTR.to_string(), false, None),
        ]
    );

    // A simple field stays atomic, with its cached result and view state off.
    let p6 = para(d, 6);
    let page = p6.objects.iter().find(|o| matches!(o, InlineObject::Field { .. })).expect("PAGE field");
    assert_eq!(
        page,
        &InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, code: false, dirty: false, form: String::new() }
    );

    // The footnote's story holds its own citation, with its own data.
    let note = note_part(d, 4);
    let note_starts = field_starts(d, StoryRef::Part(note));
    assert_eq!(note_starts, [(NOTE_CITE_INSTR.to_string(), false, Some(NOTE_FLD_DATA.to_string()))]);
    assert_eq!(d.field_ranges(StoryRef::Part(note)).len(), 1);

    // The EndNote citation is a tracked insertion: its markers carry the revision.
    let p5 = para(d, 5);
    let fs_off = p5.text.find(wordcraft_doc::para::OBJ).expect("FieldStart marker");
    assert!(p5.props_of_char(fs_off).ins.is_some(), "tracked-insert citation lost its revision");

    // Custom document properties ride along untouched.
    let names: Vec<&str> = d.custom_props.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["WC_PREF_1", "Citations"]);
    assert_eq!(d.custom_prop("WC_PREF_1"), Some(r#"<settings synthetic="1"/>"#));
    assert_eq!(d.custom_prop("Citations"), Some("4"));
}

#[test]
fn manager_fields_are_read() {
    check_manager_fields(&read(&pkg()).expect("read"));
}

#[test]
fn manager_fields_round_trip_model_level() {
    let d1 = read(&pkg()).expect("read");
    let out = write(&d1).expect("write");
    let d2 = read(&out).expect("re-read");
    // The whole shape inventory again…
    check_manager_fields(&d2);
    // …and, model against model, the first parse equals the second.
    assert_eq!(d1.field_ranges(StoryRef::Body), d2.field_ranges(StoryRef::Body));
    assert_eq!(field_starts(&d1, StoryRef::Body), field_starts(&d2, StoryRef::Body));
    assert_eq!(field_starts(&d1, StoryRef::Part(note_part(&d1, 4))), field_starts(&d2, StoryRef::Part(note_part(&d2, 4))));
    assert_eq!(d1.custom_props, d2.custom_props);
    // A second generation is stable (no drift each save).
    let d3 = read(&write(&d2).expect("write")).expect("re-read");
    check_manager_fields(&d3);
    assert_eq!(d2.field_ranges(StoryRef::Body), d3.field_ranges(StoryRef::Body));
}

#[test]
fn edit_pass_keeps_manager_fields_live() {
    let d1 = read(&pkg()).expect("read");
    let fields_before = field_starts(&d1, StoryRef::Body);
    let mut d = d1.clone();

    let c = CharProps::default();
    // Type inside the Mendeley citation's result.
    let off = para(&d, 0).text.find("(Austen").expect("result") + 1;
    d.insert_text(&Pos::new(StoryRef::Body, Path::top(0), off), "XX", &c).expect("type inside");
    // Type before the field.
    d.insert_text(&Pos::new(StoryRef::Body, Path::top(0), 0), "B4 ", &c).expect("type before");
    // Type after the bibliography's end marker, in its last paragraph.
    let after_end = para(&d, 3).text.len();
    d.insert_text(&Pos::new(StoryRef::Body, Path::top(3), after_end), " AF", &c).expect("type after");
    // Delete the paragraph next to the first citation (the Zotero one — its field goes with it).
    d.remove_block(StoryRef::Body, &Path::top(1)).expect("delete adjacent paragraph");

    let d2 = read(&write(&d).expect("write")).expect("re-read");
    // The edits landed.
    assert!(para_text(&d2, 0).starts_with("B4 See ("), "{}", para_text(&d2, 0));
    assert!(para_text(&d2, 0).contains("(XXAusten"), "{}", para_text(&d2, 0));
    assert!(para_text(&d2, 2).ends_with("Somewhere. AF"), "{}", para_text(&d2, 2));
    // Three fields survive (the Zotero paragraph was deleted with its field), unchanged in
    // everything but position: instruction, lock state and data are exactly the starting ones.
    let after = field_starts(&d2, StoryRef::Body);
    assert_eq!(&after[..1], &fields_before[..1], "Mendeley citation changed by the edit pass");
    assert_eq!(&after[1..], &fields_before[2..], "bibliography and EndNote citation changed by the edit pass");
    let r = d2.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 3, "{r:?}");
    assert_eq!((r[0].start.path.clone(), r[0].end.path.clone()), (Path::top(0), Path::top(0)));
    assert_eq!((r[1].start.path.clone(), r[1].end.path.clone()), (Path::top(1), Path::top(2)), "bibliography no longer spans paragraphs");
    assert!(!d2.has_unbalanced_field_ranges());
    // The footnote's citation is untouched by body edits.
    assert_eq!(field_starts(&d2, StoryRef::Part(note_part(&d2, 3))), field_starts(&d1, StoryRef::Part(note_part(&d1, 4))));
}

#[test]
fn hostile_field_constructs_never_panic() {
    let bytes = hostile_fields_docx(1024).expect("testkit package");
    let d = read(&bytes).expect("read");
    let texts: Vec<String> = (0..6).map(|i| para_text(&d, i)).collect();
    assert_eq!(
        texts,
        [
            "",           // truncated field: marker only, no result
            "",           // instruction text outside any field: dropped
            "(Un, 1)",    // unterminated JSON payload: still a live field
            "(Weird, 2)", // non-base64 data and a post-separator fldData
            "(Big, 3)",   // oversized instruction
            "After.",
        ]
    );
    // The truncated field kept its code (spilled to an atomic field at the paragraph end).
    let p0 = para(&d, 0);
    assert!(p0.objects.iter().any(|o| matches!(
        o,
        InlineObject::Field { instr, result, .. } if instr == r#"ADDIN MENDELEY_Citation {"citationId":"trunc"# && result.is_empty()
    )));
    // Unterminated JSON is data like any other: round-trips verbatim.
    let starts = field_starts(&d, StoryRef::Body);
    assert_eq!(starts[0].0, r#"ADDIN MENDELEY_Citation {"citationId":"unterminated"#);
    // Not-base64 fldData is preserved as-is (opaque), the one after the separator ignored.
    assert_eq!(starts[1].2.as_deref(), Some("%%%not base64%%%"));
    // Re-saving repairs nothing (all fields here were closed or spilled atomically) but must
    // be balanced and panic-free. The spilled truncated field saves as a compact field and
    // re-reads as a live (empty) range field; its `w:fldData` was spilled away — known loss
    // on that degenerate shape. Everything else is byte-identical.
    let d2 = read(&write(&d).expect("write")).expect("re-read");
    assert!(!d2.has_unbalanced_field_ranges());
    let starts2 = field_starts(&d2, StoryRef::Body);
    assert_eq!(starts2[0], (r#"ADDIN MENDELEY_Citation {"citationId":"trunc"#.to_string(), true, None));
    assert_eq!(&starts2[1..], &starts[..]);
    assert_eq!(para_text(&d2, 0), "");
}

#[test]
fn oversized_instruction_and_fld_data_are_capped_not_fatal() {
    // A megabyte payload, under the cap: preserved exactly, byte for byte — code and data.
    let under = hostile_fields_docx(1024 * 1024).expect("testkit package");
    let d = read(&under).expect("read");
    let big = field_starts(&d, StoryRef::Body)[2].0.clone();
    // ` ADDIN MENDELEY_Citation {"pad":"` + payload + `"}`, leading space trimmed on read.
    assert_eq!(big.len(), 34 + 1024 * 1024, "a payload under the cap must be kept whole");
    assert_eq!(field_starts(&d, StoryRef::Body)[2].2.as_deref().map(str::len), Some(1024 * 1024));
    let d2 = read(&write(&d).expect("write")).expect("re-read");
    let starts2 = field_starts(&d2, StoryRef::Body);
    assert_eq!(starts2.last(), Some(&(big.clone(), false, Some("A".repeat(1024 * 1024)))));

    // Far over the cap: truncated to it, never fatal, and stable across generations.
    let over = hostile_fields_docx(3 * 1024 * 1024).expect("testkit package");
    let d = read(&over).expect("read");
    let starts = field_starts(&d, StoryRef::Body);
    assert!(starts[2].0.len() <= INSTR_CAP, "instruction not capped: {}", starts[2].0.len());
    assert!(starts[2].2.as_deref().is_some_and(|s| s.len() <= INSTR_CAP), "fldData not capped");
    let d2 = read(&write(&d).expect("write")).expect("re-read");
    let starts2 = field_starts(&d2, StoryRef::Body);
    assert_eq!(starts2.last().map(|(i, _, _)| i.clone()), starts.last().map(|(i, _, _)| i.clone()));
    assert_eq!(starts2.last().and_then(|(_, _, d)| d.clone()), starts.last().and_then(|(_, _, d)| d.clone()));
}
