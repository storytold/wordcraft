//! #1671: a field nested in another field's code is an ordinary field in the editor: Update
//! Field keeps its cached result and its nesting, editing its code replaces the nesting, and
//! Check Compatibility says it isn't updated. Synthetic markup from ECMA-376 Part 1 §17.16.
#![cfg(not(target_arch = "wasm32"))]

use std::io::{Read, Write};

use serde_json::json;
use wordcraft_engine::Session;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

fn package(body: &str) -> Vec<u8> {
    let doc = format!(r#"<w:document {NS}><w:body>{body}</w:body></w:document>"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn t(s: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#)
}
fn fld(kind: &str) -> String {
    format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#)
}
fn instr(s: &str) -> String {
    format!(r#"<w:r><w:instrText xml:space="preserve">{s}</w:instrText></w:r>"#)
}

/// Bookmark `amount` = 0, then `{ REF { QUOTE "amount" } }` showing a stale "old", which Update
/// Field would change to "0" if it evaluated the flattened code `REF amount`.
fn session() -> Session {
    let quote = format!("{}{}{}{}{}", fld("begin"), instr(r#" QUOTE "amount" "#), fld("separate"), t("amount"), fld("end"));
    let body = format!(
        r#"<w:p><w:bookmarkStart w:id="0" w:name="amount"/>{}<w:bookmarkEnd w:id="0"/></w:p><w:p>{}{}{quote}{}{}{}</w:p>"#,
        t("0"),
        fld("begin"),
        instr(" REF "),
        fld("separate"),
        t("old"),
        fld("end"),
    );
    let doc = wordcraft_docx::read(&package(&body)).unwrap();
    Session::new(doc)
}

fn saved(s: &Session) -> String {
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml
}

fn begins(xml: &str) -> usize {
    xml.matches(r#"w:fldCharType="begin""#).count()
}

#[test]
fn update_field_keeps_a_nested_fields_result_and_nesting() {
    let mut s = session();
    s.run("references.updateFields", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), "0\nold");
    assert_eq!(begins(&saved(&s)), 2);
}

/// SEQ and STYLEREF are filled by their own pass, which must skip a field with nested code too.
#[test]
fn update_field_keeps_a_nested_seq_and_styleref_result() {
    let mut paras = String::new();
    for name in ["SEQ Figure ", "STYLEREF 1 "] {
        let quote = format!("{}{}{}{}{}", fld("begin"), instr(r#" QUOTE "x" "#), fld("separate"), t("x"), fld("end"));
        paras += &format!("<w:p>{}{}{quote}{}{}{}</w:p>", fld("begin"), instr(&format!(" {name}")), fld("separate"), t("old"), fld("end"),);
    }
    let doc = wordcraft_docx::read(&package(&paras)).unwrap();
    let mut s = Session::new(doc);
    let before = begins(&saved(&s));
    s.run("references.updateFields", &json!({})).unwrap();
    let text = s.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(!text.contains('1'), "{text}");
    assert_eq!(begins(&saved(&s)), before);
}

#[test]
fn a_nested_field_is_a_field_its_code_can_be_edited() {
    let mut s = session();
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("fields.next", &json!({})).unwrap();
    let sel = s.run("fields.selected", &json!({})).unwrap();
    assert_eq!(sel[0]["instr"], "REF amount", "{sel}");
    // A new code replaces the nested one.
    s.run("fields.setCode", &json!({"code": "QUOTE \"new\""})).unwrap();
    let xml = saved(&s);
    assert_eq!(begins(&xml), 1, "{xml}");
    assert!(xml.contains("QUOTE &quot;new&quot;"), "{xml}");
}

#[test]
fn check_compatibility_says_nested_fields_are_not_updated() {
    let mut s = session();
    let r = s.run("file.compatibility", &json!({})).unwrap();
    let issues = r["issues"].to_string();
    assert!(issues.contains("Fields nested in other fields show their last result and aren't updated."), "{issues}");
    assert!(!issues.contains("embedded objects"), "{issues}");
}

/// A package whose footer holds `{ IF { PAGE } = 1 "first" "later" }`.
fn footer_package() -> Vec<u8> {
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#;
    let page = format!("{}{}{}{}{}", fld("begin"), instr(" PAGE "), fld("separate"), t("1"), fld("end"));
    let footer = format!(
        r#"<w:ftr {NS}><w:p>{}{}{page}{}{}{}{}</w:p></w:ftr>"#,
        fld("begin"),
        instr(" IF "),
        instr(r#" = 1 "first" "later" "#),
        fld("separate"),
        t("first"),
        fld("end")
    );
    let doc = format!(
        r#"<w:document {NS} xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p>{}</w:p><w:sectPr><w:footerReference w:type="default" r:id="rIdF"/></w:sectPr></w:body></w:document>"#,
        t("Body")
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/footer1.xml", footer.as_bytes()),
    ] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

#[test]
fn check_compatibility_finds_nested_fields_in_footers() {
    let mut s = Session::new(wordcraft_docx::read(&footer_package()).unwrap());
    assert!(!s.doc.parts.is_empty(), "the footer was read");
    let issues = s.run("file.compatibility", &json!({})).unwrap()["issues"].to_string();
    assert!(issues.contains("Fields nested in other fields show their last result and aren't updated."), "{issues}");
}
