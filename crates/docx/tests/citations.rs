//! Citation-manager documents (Zotero and the like): `ADDIN` fields keep their formatted,
//! multi-paragraph results, and custom document properties round-trip. All markup here is
//! synthetic, written from ECMA-376.

use std::io::Write;

use wordcraft_doc::para::OBJ;
use wordcraft_doc::{CharProps, Document, InlineObject, Path, StoryRef};
use wordcraft_docx::{read, write};

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties" Target="docProps/custom.xml"/></Relationships>"#;

const CITE_A: &str = r#" ADDIN ZOTERO_ITEM CSL_CITATION {"citationID":"k1","properties":{"formattedCitation":"(Doe, 2020)"},"#;
const CITE_B: &str = r#""citationItems":[{"id":7,"uris":["http://zotero.org/users/local/x/items/ABCD"]}]} "#;
const BIBL: &str = r#" ADDIN ZOTERO_BIBL {"uncited":[],"omitted":[],"custom":[]} CSL_BIBLIOGRAPHY "#;

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn fld(kind: &str) -> String {
    format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#)
}
fn instr(s: &str) -> String {
    format!(r#"<w:r><w:instrText xml:space="preserve">{}</w:instrText></w:r>"#, s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;"))
}
fn t(s: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#)
}
fn ti(s: &str) -> String {
    format!(r#"<w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">{s}</w:t></w:r>"#)
}

fn custom_xml() -> String {
    let p = |pid: u32, name: &str, v: &str| {
        format!(r#"<property fmtid="{{D5CDD505-2E9C-101B-9397-08002B2CF9AE}}" pid="{pid}" name="{name}">{v}</property>"#)
    };
    format!(
        r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">{}{}{}{}</Properties>"#,
        p(2, "ZOTERO_PREF_1", r#"<vt:lpwstr>&lt;data data-version="3"&gt;&lt;session id="abc"/&gt;</vt:lpwstr>"#),
        p(3, "ZOTERO_PREF_2", r#"<vt:lpwstr>&lt;style id="http://www.zotero.org/styles/apa"/&gt;&lt;/data&gt;</vt:lpwstr>"#),
        p(4, "Pages", "<vt:i4>12</vt:i4>"),
        p(5, "Tags", r#"<vt:vector size="2" baseType="lpwstr"><vt:lpwstr>a</vt:lpwstr><vt:lpwstr>b &amp; c</vt:lpwstr></vt:vector>"#),
    )
}

fn package(body: &str) -> Vec<u8> {
    let doc = format!(r#"<w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    zip(&[("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes()), ("docProps/custom.xml", custom_xml().as_bytes())])
}

/// A cited sentence, a PAGE field, and a bibliography spanning two paragraphs.
fn zotero_docx() -> Vec<u8> {
    let p0 = [
        t("See "),
        fld("begin"),
        instr(CITE_A),
        instr(CITE_B),
        fld("separate"),
        t("(Doe, "),
        ti("2020"),
        t(")"),
        fld("end"),
        t(". Page "),
        fld("begin"),
        instr("PAGE"),
        fld("separate"),
        t("1"),
        fld("end"),
    ];
    let p0 = format!("<w:p>{}</w:p>", p0.concat());
    let p1 = format!("<w:p>{}{}{}{}{}</w:p>", fld("begin"), instr(BIBL), fld("separate"), t("Doe, J. (2020). "), ti("A Title"));
    let p2 = format!("<w:p>{}{}</w:p>", t("Roe, R. (2019). Other."), fld("end"));
    let p3 = format!("<w:p>{}</w:p>", t("After"));
    package(&format!("{p0}{p1}{p2}{p3}"))
}

fn para_text(d: &Document, i: usize) -> String {
    d.para(StoryRef::Body, &Path::top(i)).map(|p| p.plain_text()).unwrap_or_default()
}

fn italic_at(d: &Document, i: usize, needle: &str) -> bool {
    let p = d.para(StoryRef::Body, &Path::top(i)).unwrap();
    let off = p.text.find(needle).unwrap();
    p.props_of_char(off).italic == Some(true)
}

fn check_zotero(d: &Document) {
    assert_eq!(para_text(d, 0), "See (Doe, 2020). Page 1");
    assert_eq!(para_text(d, 1), "Doe, J. (2020). A Title");
    assert_eq!(para_text(d, 2), "Roe, R. (2019). Other.");
    assert_eq!(para_text(d, 3), "After");
    // The result keeps its formatting.
    assert!(italic_at(d, 0, "2020"));
    assert!(!italic_at(d, 0, "(Doe"));
    assert!(italic_at(d, 1, "A Title"));

    let r = d.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 2, "{r:?}");
    assert_eq!(r[0].instr, format!("{CITE_A}{CITE_B}").trim());
    assert_eq!(r[0].start.path, Path::top(0));
    assert_eq!(r[0].end.path, Path::top(0));
    assert_eq!(r[1].instr, BIBL.trim());
    assert_eq!((r[1].start.path.clone(), r[1].start.off), (Path::top(1), 0));
    assert_eq!(r[1].end.path, Path::top(2));
    assert!(!d.has_unbalanced_field_ranges());

    // Engine fields stay atomic.
    let p0 = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert!(p0.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if instr == "PAGE")));

    let names: Vec<&str> = d.custom_props.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["ZOTERO_PREF_1", "ZOTERO_PREF_2", "Pages", "Tags"]);
    assert_eq!(d.custom_prop("ZOTERO_PREF_1"), Some(r#"<data data-version="3"><session id="abc"/>"#));
    assert_eq!(d.custom_prop("zotero_pref_2"), Some(r#"<style id="http://www.zotero.org/styles/apa"/></data>"#));
    assert_eq!((d.custom_props[2].kind.as_str(), d.custom_props[2].value.as_str()), ("i4", "12"));
    assert_eq!(d.custom_props[3].kind, "raw");
    assert!(d.custom_props[3].value.contains("b &amp; c"));
}

fn count(xml: &str, kind: &str) -> usize {
    xml.matches(&format!(r#"w:fldCharType="{kind}""#)).count()
}

fn document_xml(docx: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut s).unwrap();
    s
}

#[test]
fn zotero_fields_and_prefs_are_read() {
    check_zotero(&read(&zotero_docx()).unwrap());
}

#[test]
fn zotero_fields_and_prefs_round_trip() {
    let d = read(&zotero_docx()).unwrap();
    let out = write(&d).unwrap();
    let xml = document_xml(&out);
    assert_eq!(count(&xml, "begin"), 3);
    assert_eq!(count(&xml, "separate"), 3);
    assert_eq!(count(&xml, "end"), 3);
    let d2 = read(&out).unwrap();
    check_zotero(&d2);
    assert_eq!(d.custom_props, d2.custom_props);
    // And a second generation is stable.
    assert_eq!(read(&write(&d2).unwrap()).unwrap().custom_props, d.custom_props);
}

#[test]
fn unterminated_and_resultless_addin_fields() {
    // A field left open at the end of the document, and one with no result at all.
    let body = format!(
        "<w:p>{}{}{}{}</w:p><w:p>{}{}{}{}</w:p>",
        fld("begin"),
        instr(" ADDIN EMPTY "),
        fld("end"),
        t("x"),
        fld("begin"),
        instr(BIBL),
        fld("separate"),
        t("dangling"),
    );
    let d = read(&package(&body)).unwrap();
    let r = d.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].instr, "ADDIN EMPTY");
    assert_eq!(para_text(&d, 0), "x");
    assert!(d.has_unbalanced_field_ranges());
    let out = write(&d).unwrap();
    let xml = document_xml(&out);
    assert_eq!(count(&xml, "begin"), count(&xml, "end"));
    let d2 = read(&out).unwrap();
    assert!(!d2.has_unbalanced_field_ranges());
    assert_eq!(para_text(&d2, 1), "dangling");
}

#[test]
fn orphan_markers_from_editing_are_dropped_on_save() {
    let mut d = Document::from_text("one\ntwo");
    let c = CharProps::default();
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_object(0, InlineObject::FieldEnd, &c).unwrap();
    d.para_mut(StoryRef::Body, &Path::top(1))
        .unwrap()
        .insert_object(1, InlineObject::FieldStart { instr: "ADDIN X".into(), locked: true }, &c)
        .unwrap();
    let xml = document_xml(&write(&d).unwrap());
    assert_eq!(count(&xml, "begin"), 0);
    assert_eq!(count(&xml, "end"), 0);
    // The document itself is untouched.
    assert!(d.has_unbalanced_field_ranges());
}

#[test]
fn programmatic_range_field_writes_as_word_field() {
    let mut d = Document::from_text("ab");
    let c = CharProps::default();
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.insert_object(1, InlineObject::FieldStart { instr: "ADDIN ZOTERO_ITEM {}".into(), locked: false }, &c).unwrap();
    let end = 1 + OBJ.len_utf8() + 1;
    p.insert_object(end, InlineObject::FieldEnd, &c).unwrap();
    d.set_custom_prop("ZOTERO_PREF_1", "<data/>");
    let d2 = read(&write(&d).unwrap()).unwrap();
    let r = d2.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].instr, "ADDIN ZOTERO_ITEM {}");
    assert_eq!(para_text(&d2, 0), "ab");
    assert_eq!(d2.custom_prop("ZOTERO_PREF_1"), Some("<data/>"));
}
