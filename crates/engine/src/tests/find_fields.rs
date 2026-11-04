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

#[test]
fn find_skips_results_while_field_codes_show() {
    let mut s = cited();
    s.view.field_codes = true;
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[1]"}))["count"], 0);
    s.view.field_codes = false;
    assert_eq!(run(&mut s, "edit.find", json!({"text": "[1]"}))["count"], 2);
}

#[test]
fn replace_leaves_matches_in_fields() {
    let mut s = cited();
    let r = run(&mut s, "edit.replaceAll", json!({"text": "[1]", "with": "[2]"}));
    assert_eq!((r["replaced"].as_u64(), r["skippedInFields"].as_u64()), (Some(0), Some(2)), "{r}");
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
