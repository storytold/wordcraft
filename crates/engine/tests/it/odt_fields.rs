//! Fields in an ODT file update in WordCraft as they do in Word: a reference to a bookmark shows
//! the bookmarked text (a bookmark over text, `text:bookmark-start`…`-end`, not a point). The
//! ODT is original and synthetic, shaped like LibreOffice's.

use std::io::Write;

use serde_json::json;
use wordcraft_engine::{Session, io};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#;

fn odt(body: &str) -> Vec<u8> {
    let content = format!(r#"<office:document-content {NS}><office:body><office:text>{body}</office:text></office:body></office:document-content>"#);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn text(s: &Session) -> Vec<String> {
    s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect()
}

/// The displayed results of the fields in the body.
fn results(s: &Session) -> Vec<String> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            wordcraft_doc::para::InlineObject::Field { result, .. } => Some(result.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_reference_to_a_bookmark_updates_to_its_text() {
    let doc = io::open_bytes(
        "refs.odt",
        &odt(r#"<text:h text:outline-level="1"><text:bookmark-start text:name="__RefHeading___Toc1"/>Planning ahead<text:bookmark-end text:name="__RefHeading___Toc1"/></text:h><text:p>See <text:bookmark-ref text:reference-format="text" text:ref-name="__RefHeading___Toc1">Planning</text:bookmark-ref>.</text:p>"#),
    )
    .unwrap();
    let mut s = Session::new(doc);
    assert_eq!(results(&s), ["Planning"]);
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    assert_eq!(results(&s), ["Planning ahead"], "{:?}", text(&s));
    // And after a save in ODT and opening again.
    let again = io::open_bytes("refs.odt", &io::save_bytes("refs.odt", &s.doc).unwrap()).unwrap();
    let mut s2 = Session::new(again);
    s2.run("references.updateFields", &json!({"all": true})).unwrap();
    assert_eq!(results(&s2), ["Planning ahead"]);
}

#[test]
fn captions_chapters_and_dates_update_as_words() {
    // Stale results, as a file can hold them: they are recomputed.
    let doc = io::open_bytes(
        "fields.odt",
        &odt(r#"<text:h text:outline-level="1">Methods</text:h>
        <text:p>Figure <text:sequence text:ref-name="refFigure0" text:name="Figure" text:formula="ooow:Figure+1" style:num-format="1">7</text:sequence>: a cat</text:p>
        <text:p>Figure <text:sequence text:ref-name="refFigure1" text:name="Figure" text:formula="ooow:Figure+1" style:num-format="1">9</text:sequence>: a dog</text:p>
        <text:p>See <text:sequence-ref text:reference-format="category-and-value" text:ref-name="refFigure1">Figure 9</text:sequence-ref> in <text:chapter text:display="name" text:outline-level="1">Old</text:chapter>.</text:p>"#),
    )
    .unwrap();
    let mut s = Session::new(doc);
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    assert_eq!(results(&s), ["1", "2", "Figure 2", "Methods"], "{:?}", text(&s));
}

#[test]
fn a_flat_document_opens_and_saves_by_its_extension() {
    let fodt = r#"<?xml version="1.0" encoding="UTF-8"?><office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.text"><office:body><office:text><text:p>Flat words.</text:p></office:text></office:body></office:document>"#;
    let doc = io::open_bytes("plain.fodt", fodt.as_bytes()).unwrap();
    let s = Session::new(doc);
    assert_eq!(text(&s), ["Flat words."]);
    let saved = io::save_bytes("plain.fodt", &s.doc).unwrap();
    assert!(saved.starts_with(b"<?xml"));
    assert_eq!(text(&Session::new(io::open_bytes("plain.fodt", &saved).unwrap())), ["Flat words."]);
}

#[test]
fn a_user_field_updates_from_its_property() {
    // An ODT user field opens as `DOCPROPERTY`: updating fields shows the property's value.
    let content = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:user-field-decls><text:user-field-decl text:name="Owner" office:value-type="string" office:string-value="Ada"/></text:user-field-decls><text:p>By <text:user-field-get text:name="Owner">stale</text:user-field-get>.</text:p></office:text></office:body></office:document-content>"#;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content)] {
        z.start_file(name, o).unwrap();
        std::io::Write::write_all(&mut z, data.as_bytes()).unwrap();
    }
    let mut s = Session::new(io::open_bytes("user.odt", &z.finish().unwrap().into_inner()).unwrap());
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    assert_eq!(results(&s), ["Ada"]);
}

#[test]
fn a_table_formula_updates_as_a_word_formula() {
    // LibreOffice's sum of the cells above, its value out of date: Update Field works it out.
    let cell = |v: &str| {
        format!(
            r#"<table:table-row><table:table-cell office:value-type="float" office:value="{v}"><text:p>{v}</text:p></table:table-cell></table:table-row>"#
        )
    };
    let body = format!(
        r#"<table:table xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0">{}{}<table:table-row><table:table-cell table:formula="ooow:sum &lt;A1:A2&gt;" office:value-type="float" office:value="99"><text:p>99</text:p></table:table-cell></table:table-row></table:table>"#,
        cell("1.5"),
        cell("3")
    );
    let mut s = Session::new(io::open_bytes("sum.odt", &odt(&body)).unwrap());
    let shown = |s: &Session| -> Vec<(String, String)> {
        let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        t.rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .flat_map(|c| c.blocks.iter().filter_map(|b| b.as_para()))
            .flat_map(|p| p.objects.iter())
            .filter_map(|o| match o {
                wordcraft_doc::para::InlineObject::Field { instr, result, .. } => Some((instr.clone(), result.clone())),
                _ => None,
            })
            .collect()
    };
    assert_eq!(shown(&s), [("=SUM(A1:A2)".to_string(), "99".to_string())]);
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    assert_eq!(shown(&s), [("=SUM(A1:A2)".to_string(), "4.50".to_string())]);
}

#[test]
fn odt_variables_update_as_set_and_ref() {
    // A variable set, displayed; set again, hidden; then shown: LibreOffice shows "Ada", nothing,
    // "Grace". Its results out of date here, Update Field works them out again.
    let body = r#"<text:variable-decls><text:variable-decl office:value-type="string" text:name="who"/></text:variable-decls><text:p><text:variable-set text:name="who" office:value-type="string" office:string-value="Ada">stale</text:variable-set>|<text:variable-set text:name="who" text:display="none" office:value-type="string" office:string-value="Grace"/>|<text:variable-get text:name="who">stale</text:variable-get></text:p>"#;
    let mut s = Session::new(io::open_bytes("vars.odt", &odt(body)).unwrap());
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    let r = results(&s);
    assert_eq!((r[1].as_str(), r[3].as_str()), ("Ada", "Grace"));
}
