//! Find reads a field's shown result, as Word does (#5552): citations, cross-references and
//! legacy form fields are found by the text they show. A match covers the whole field, and
//! Replace leaves such matches alone, since replacing one would delete its field.

use std::io::Write;

use serde_json::json;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{InlineObject, Pos, StoryRef};

use crate::Session;

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn field_count(s: &Session) -> usize {
    s.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).filter(|o| matches!(o, InlineObject::Field { .. })).count()
}

/// "See [1] and [1] again." with two IEEE citations of one source, inserted as WordCraft does.
fn cited() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "references.citationStyle", json!({"style": "IEEE"}));
    run(&mut s, "text.insert", json!({"text": "See "}));
    let src = json!({"source": {"author": "Rivera, Alex", "title": "Shared Spaces", "year": "2021"}});
    run(&mut s, "references.citation", src);
    run(&mut s, "text.insert", json!({"text": " and "}));
    let tag = s.doc.para_at(&Pos::body(0, 0)).and_then(|p| {
        p.objects.iter().find_map(|o| match o {
            InlineObject::Field { instr, .. } => instr.split_whitespace().nth(1).map(str::to_string),
            _ => None,
        })
    });
    run(&mut s, "references.citation", json!({"tag": tag.unwrap()}));
    run(&mut s, "text.insert", json!({"text": " again."}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "See [1] and [1] again.");
    assert_eq!(field_count(&s), 2);
    run(&mut s, "caret.docStart", json!({}));
    s
}

#[test]
fn find_matches_citation_results() {
    let mut s = cited();
    let r = run(&mut s, "edit.find", json!({"text": "[1]"}));
    assert_eq!(r["count"], 2, "{r}");
    // The first match is selected, and it is the whole first citation.
    let (a, b) = s.sel.ordered();
    let p = s.doc.para_at(&a).unwrap();
    assert_eq!(p.text.get(a.off..b.off), Some(OBJ.to_string().as_str()));
    assert!(matches!(p.object_at(a.off), Some(InlineObject::Field { result, .. }) if result == "[1]"));
    // Part of a result finds its field once, however often it matches there.
    assert_eq!(run(&mut s, "edit.find", json!({"text": "1"}))["count"], 2);
    // A match runs on from the text into the field and out of it again.
    assert_eq!(run(&mut s, "edit.find", json!({"text": "See [1] and"}))["count"], 1);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[1] again"}))["count"], 1);
    // Whole words see the result's characters, not an object character.
    assert_eq!(run(&mut s, "edit.find", json!({"text": "1", "wholeWord": true}))["count"], 2);
    // Find Next steps through both citations.
    run(&mut s, "edit.find", json!({"text": "[1]", "wholeWord": false}));
    let first = s.sel.ordered();
    run(&mut s, "edit.findNext", json!({}));
    assert_ne!(s.sel.ordered(), first);
    // Text that only looks like the result next to it isn't a match.
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[1]]"}))["count"], 0);
}

/// While field codes show (View › Field Codes, Alt+F9), Find matches the code in braces the page
/// shows, as Word does, not the hidden result; Replace leaves those matches alone too.
#[test]
fn find_matches_field_codes_while_they_show() {
    let mut s = cited();
    run(&mut s, "view.fieldCodes", json!({"value": true}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[1]"}))["count"], 0);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "{ CITATION"}))["count"], 2);
    let r = run(&mut s, "edit.replaceAll", json!({"text": "CITATION", "with": "X"}));
    assert_eq!((r["replaced"].as_u64(), r["skippedInFields"].as_u64()), (Some(0), Some(2)), "{r}");
    assert_eq!(field_count(&s), 2);
    run(&mut s, "view.fieldCodes", json!({"value": false}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[1]"}))["count"], 2);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "CITATION"}))["count"], 0);
}

/// #5560 review: Alt+F9 changes what Find matches without changing the document, so Reading
/// Highlight and the Navigation pane's results are searched again rather than kept from before.
#[test]
fn showing_field_codes_searches_highlights_and_results_again() {
    let mut s = cited();
    run(&mut s, "edit.advancedFind", json!({"text": "[1]", "highlight": true}));
    assert_eq!((s.find_highlights().len(), s.find.results.len()), (2, 2));
    run(&mut s, "view.fieldCodes", json!({}));
    assert_eq!((s.find_highlights().len(), s.find.results.len()), (0, 0));
    run(&mut s, "view.fieldCodes", json!({}));
    assert_eq!((s.find_highlights().len(), s.find.results.len()), (2, 2));
}

fn simple(instr: &str, result: &str) -> InlineObject {
    InlineObject::Field { instr: instr.into(), result: result.into(), locked: false, dirty: false, code: false, form: String::new() }
}

/// #5560 review: Find reads each field as layout draws it, not as its saved result: ASK and SET
/// show nothing, TITLE, AUTHOR and FILENAME with no result show the document's, PAGEREF and
/// STYLEREF in the body show their result. Fields whose number depends on the page they land on
/// aren't matched at all, rather than on a number the page may not show; nor is an EQ overstrike.
/// Every text Find matches is exactly a text layout drew for the field.
#[test]
fn find_reads_each_field_as_layout_draws_it() {
    // (code, saved result, what Find matches)
    let kinds: &[(&str, &str, Option<&str>)] = &[
        ("CITATION Smi20 \\l 1033", "[7]", Some("[7]")),
        ("REF _Ref2 \\h", "Table 2", Some("Table 2")),
        ("SEQ Figure \\* ARABIC", "3", Some("3")),
        ("SET Code \"COD\"", "COD", None),
        ("ASK Name \"Who?\"", "Ada", None),
        ("TITLE", "", Some("Field Notes")),
        ("TITLE", "Saved title", Some("Saved title")),
        ("AUTHOR", "", Some("Rees Hart")),
        ("FILENAME", "", Some("FILENAME")),
        ("PAGE", "4", None),
        ("NUMPAGES", "9", None),
        ("SECTIONPAGES", "8", None),
        ("SECTION", "6", None),
        ("PAGEREF _Ref1 \\h", "12", Some("12")),
        ("STYLEREF \"Heading 1\"", "Methods", Some("Methods")),
        ("EQ \\o(a,b)", "", None),
    ];
    let mut d = wordcraft_doc::Document::from_text(&vec!["at "; kinds.len()].join("\n"));
    d.core.title = "Field Notes".into();
    d.core.creator = "Rees Hart".into();
    for (i, (code, result, _)) in kinds.iter().enumerate() {
        d.insert_object(&Pos::body(i, 3), simple(code, result), &Default::default()).unwrap();
    }
    let mut s = Session::new(d);
    let filename = s.resolved_filenames().0;
    let l = s.layout();
    let drawn: Vec<String> = l
        .pages
        .iter()
        .flat_map(|pg| pg.items.iter())
        .filter_map(
            |it| if let wordcraft_layout::Placed::Lines { para, .. } = it { Some(para.shown.iter().map(|(_, t, _)| t.clone())) } else { None },
        )
        .flatten()
        .collect();
    for (code, result, finds) in kinds {
        let finds = finds.map(|f| if f == "FILENAME" { filename.as_str() } else { f });
        match finds {
            Some(text) => {
                assert!(drawn.iter().any(|t| t == text), "{code}: layout draws {text:?} (drawn: {drawn:?})");
                let r = run(&mut s, "edit.find", json!({"text": text, "matchCase": true}));
                assert_eq!(r["count"], 1, "{code}: Find {text:?}: {r}");
            }
            None if !result.is_empty() => {
                assert_eq!(run(&mut s, "edit.find", json!({"text": result, "matchCase": true}))["count"], 0, "{code}: {result:?} isn't shown");
            }
            None => {}
        }
    }
}

/// Headers and footers work out PAGEREF and STYLEREF from the pages, so there Find leaves them out
/// as it does page numbers; the body and notes show (and match) their result.
#[test]
fn page_worked_fields_in_headers_are_not_matched() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("x"));
    let header = run(&mut s, "insert.header", json!({"text": "h"}))["story"].as_u64().unwrap() as u32;
    s.sel = crate::Selection::caret(Pos::body(0, 0));
    run(&mut s, "references.footnote", json!({"text": "n"}));
    let note = s.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(id, _)| *id).unwrap();
    for code in ["PAGEREF _Ref1 \\h", "STYLEREF \"Heading 1\""] {
        let o = simple(code, "12");
        let read = |story| crate::cmd::fields::FieldShow::of(&s, story).text(&o);
        assert_eq!(read(StoryRef::Part(header)), None, "{code}");
        assert_eq!(read(StoryRef::Body).as_deref(), Some("12"), "{code}");
        assert_eq!(read(StoryRef::Part(note)).as_deref(), Some("12"), "{code}");
    }
}

/// #5560 review: Find looks up a paragraph's fields in one walk of it, so a long paragraph of
/// objects costs about 8x the steps for 8x the objects, not 64x (each lookup by offset counted the
/// objects before it again, #3614).
#[test]
fn find_walks_a_paragraph_of_objects_once() {
    let make = |n: usize| {
        let mut d = wordcraft_doc::Document::from_text(&"word ".repeat(n));
        for i in (0..n).rev() {
            let o = if i % 2 == 0 { simple("CITATION Smi20", "[7]") } else { InlineObject::BookmarkStart { name: format!("b{i}") } };
            d.insert_object(&Pos::body(0, i * 5), o, &Default::default()).unwrap();
        }
        let mut s = Session::new(d);
        s.find.query = "[7]word".into();
        s
    };
    let (small, big) = (make(500), make(4000));
    let count = |s: &Session| {
        let mut n = 0;
        let steps = wordcraft_doc::work::count(|| n = crate::cmd::edit::search(s, StoryRef::Body).unwrap().len());
        (n, steps)
    };
    let ((ns, s1), (nb, s8)) = (count(&small), count(&big));
    assert_eq!((ns, nb), (250, 2000));
    let ratio = s8 as f64 / s1.max(1) as f64;
    assert!(s1 > 0 && ratio <= 12.0, "8x the objects took {ratio:.1}x the steps ({s1} to {s8})");
}

#[test]
fn replace_leaves_matches_in_fields() {
    let mut s = cited();
    let r = run(&mut s, "edit.replaceAll", json!({"text": "[1]", "with": "[2]"}));
    assert_eq!((r["replaced"].as_u64(), r["skippedInFields"].as_u64()), (Some(0), Some(2)), "{r}");
    // The message says why Find's two matches gave no replacements.
    assert_eq!(s.status, "All done. We made 0 replacements. 2 matches are in fields and were left as they are.");
    let r = run(&mut s, "edit.replaceAll", json!({"text": "again", "with": "twice"}));
    assert_eq!((r["replaced"].as_u64(), r["skippedInFields"].as_u64()), (Some(1), Some(0)), "{r}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "See [1] and [1] twice.");
    assert_eq!(field_count(&s), 2);
    // Replace on a selected match in a field moves on without deleting it.
    run(&mut s, "edit.find", json!({"text": "[1]"}));
    run(&mut s, "edit.replace", json!({"with": "[2]"}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "See [1] and [1] twice.");
    assert_eq!(field_count(&s), 2);
}

fn docx(body: &str) -> Session {
    let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
    let doc = format!(r#"<w:document {w}><w:body>{body}</w:body></w:document>"#);
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [("_rels/.rels", rels), ("word/document.xml", doc.as_str())] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    Session::new(crate::io::open_bytes("t.docx", &z.finish().unwrap().into_inner()).unwrap())
}

fn t(s: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#)
}

/// A complex field: begin, code, separate, result, end.
fn field(code: &str, ff_data: &str, result: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin">{ff_data}</w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> {code} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        t(result)
    )
}

/// #5552: a received document's IEEE citations, each a `CITATION` field showing "[7]", are all
/// found, in the body and in a table.
#[test]
fn find_matches_citations_in_an_opened_docx() {
    let cite = field("CITATION Smi20 \\l 1033", "", "[7]");
    let para = format!("<w:p>{}{cite}{}{cite}{}</w:p>", t("As shown in "), t(" and later "), t("."));
    let cell = format!("<w:tbl><w:tr><w:tc><w:p>{cite}</w:p></w:tc></w:tr></w:tbl>");
    let mut s = docx(&format!("{para}{cell}<w:p>{}{cite}</w:p>", t("Also ")));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[7]"}))["count"], 4);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[7]."}))["count"], 1);
}

/// #3605: legacy text and drop-down form fields are found by what they show; a check box shows no
/// text.
#[test]
fn find_matches_legacy_form_field_text() {
    let text = field("FORMTEXT", "<w:ffData><w:name w:val=\"Name\"/><w:textInput/></w:ffData>", "Ada Lovelace");
    let drop = field(
        "FORMDROPDOWN",
        r#"<w:ffData><w:name w:val="City"/><w:ddList><w:result w:val="1"/><w:listEntry w:val="Vienna"/><w:listEntry w:val="Graz"/></w:ddList></w:ffData>"#,
        "",
    );
    let check = field("FORMCHECKBOX", "<w:ffData><w:name w:val=\"Ok\"/><w:checkBox><w:default w:val=\"1\"/></w:checkBox></w:ffData>", "");
    let mut s = docx(&format!("<w:p>{}{text}{}{drop}{check}</w:p>", t("Name: "), t(" from ")));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "Lovelace"}))["count"], 1);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "from Graz"}))["count"], 1);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "Vienna"}))["count"], 0);
}

/// A Replace that replaces nothing (its selected match is in a field, or nothing it would replace
/// is selected) moves on to the next match like Find Next: the document has no unsaved changes
/// from it and Undo has no step for it (#5560 review).
#[test]
fn a_replace_that_skips_is_no_edit() {
    let mut s = cited();
    run(&mut s, "edit.find", json!({"text": "[1]"}));
    let first = s.sel.ordered();
    s.dirty = false;
    let undo = s.undo_depth();
    run(&mut s, "edit.replace", json!({"with": "[2]"}));
    assert_ne!(s.sel.ordered(), first, "Replace moved on to the next match");
    assert!(!s.dirty, "skipping a match in a field is no edit");
    assert_eq!(s.undo_depth(), undo);
    // The same for a selection that isn't a match at all.
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "edit.replace", json!({"text": "again", "with": "twice"}));
    assert!(!s.dirty);
    assert_eq!(s.undo_depth(), undo);
    // Replacing the selected plain-text match is an edit.
    run(&mut s, "edit.replace", json!({"with": "twice"}));
    assert!(s.dirty);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "See [1] and [1] twice.");
    assert_eq!(s.undo_depth(), undo + 1);
}
