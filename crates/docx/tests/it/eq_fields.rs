//! `EQ` fields: Word draws the formula their code describes. Exam generators write these fields
//! with no stored result, so the reader lays the formula out from the code itself (see
//! [`wordcraft_doc::eq`]) and a save keeps the field as it was. All markup is synthetic, from
//! ECMA-376 Part 1 §17.16.5 and the field code's own syntax.

use std::io::Write;

use wordcraft_doc::InlineObject;
use wordcraft_doc::eq::Formula;
use wordcraft_doc::math::MNode;
use wordcraft_docx::{read, write};

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

fn document_xml(docx: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut s).unwrap();
    s
}

/// A complex field with `code` and no result, as the exams' generators write it.
fn field(code: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve">{code}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

/// The one field of a single-paragraph document: its instruction, result and formula.
fn read_field(body: String) -> (String, String, Option<std::sync::Arc<Formula>>) {
    let doc = read(&package(&format!("<w:p>{body}</w:p>"))).unwrap();
    let Some(p) = doc.body.first().and_then(|b| b.as_para()) else { panic!("one paragraph") };
    let Some(InlineObject::Field { instr, result, math, .. }) = p.objects.first() else { panic!("one field") };
    (instr.clone(), result.clone(), math.clone())
}

#[test]
fn an_eq_field_reads_its_formula_from_the_code() {
    // `\R(,1\f(1,2))`, how question 1 of the 11_題目 exam writes √(1½).
    let (instr, result, math) = read_field(field(" EQ \\R(,1\\f(1,2))"));
    assert_eq!(instr, "EQ \\R(,1\\f(1,2))");
    assert!(result.is_empty(), "these fields store no result: {result:?}");
    let Some(math) = math else { panic!("a formula") };
    let Some(MNode::Rad { deg_hide, e, .. }) = math.math.nodes.first() else { panic!("a radical: {:?}", math.math.nodes) };
    assert!(deg_hide);
    assert!(matches!(e.as_slice(), [MNode::Run(r0), MNode::Frac { .. }] if r0.text == "1"), "{e:?}");
    assert!(
        matches!(&e[1], MNode::Frac { num, den, .. } if matches!(num.as_slice(), [MNode::Run(n)] if n.text == "1") && matches!(den.as_slice(), [MNode::Run(d)] if d.text == "2"))
    );
}

#[test]
fn superscript_runs_in_the_code_are_raised_in_the_formula() {
    // `\R(,a` then a superscript `2` then `)`: √(a²).
    let body = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> EQ \R(,a</w:instrText></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:instrText xml:space="preserve">2</w:instrText></w:r><w:r><w:instrText xml:space="preserve">)</w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#;
    let (_, _, math) = read_field(body.into());
    let Some(math) = math else { panic!("a formula") };
    let Some(MNode::Rad { e, .. }) = math.math.nodes.first() else { panic!("a radical: {:?}", math.math.nodes) };
    assert!(matches!(e.as_slice(), [MNode::Script { sup, .. }] if matches!(sup.as_slice(), [MNode::Run(s)] if s.text == "2")), "{e:?}");
}

#[test]
fn an_eq_field_is_saved_as_the_sample_field_not_omml() {
    let doc = read(&package(&format!("<w:p>{}</w:p>", field(" EQ \\f(1,2)")))).unwrap();
    let xml = document_xml(&write(&doc).unwrap());
    assert!(xml.contains("EQ \\f(1,2)"), "{xml}");
    assert!(!xml.contains("<m:oMath"), "a formula read from a field stays a field: {xml}");
}

#[test]
fn a_superscript_run_in_the_code_is_written_back() {
    // Word draws the formula with the code's raised run; a save must keep that run, or Word
    // would show `a2` after the round trip.
    let body = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> EQ \R(,a</w:instrText></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:instrText xml:space="preserve">2</w:instrText></w:r><w:r><w:instrText xml:space="preserve">)</w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#;
    let doc = read(&package(&format!("<w:p>{body}</w:p>"))).unwrap();
    let bytes = write(&doc).unwrap();
    let xml = document_xml(&bytes);
    assert!(xml.contains(r#"<w:vertAlign w:val="superscript"/>"#), "the raised run is kept: {xml}");
    // Read back, the formula still has the raised 2 on the a.
    let back = read(&bytes).unwrap();
    let Some(p) = back.body.first().and_then(|b| b.as_para()) else { panic!("one paragraph") };
    let Some(InlineObject::Field { math: Some(math), .. }) = p.objects.first() else { panic!("a field with a formula") };
    let Some(MNode::Rad { e, .. }) = math.math.nodes.first() else { panic!("a radical: {:?}", math.math.nodes) };
    assert!(matches!(e.as_slice(), [MNode::Script { sup, .. }] if matches!(sup.as_slice(), [MNode::Run(s)] if s.text == "2")), "{e:?}");
}

#[test]
fn an_enclosure_is_not_read_as_a_formula() {
    // `eq \o\ac(○,字)` is drawn as a shape around the character, as before (#297).
    let (instr, result, math) = read_field(field(" eq \\o\\ac(○,字)"));
    assert_eq!(instr, "eq \\o\\ac(○,字)");
    assert_eq!(result, "字");
    assert!(math.is_none());
}

#[test]
fn an_overstruck_fraction_is_read_as_the_fraction() {
    // Some generators overstrike a space with a fraction (`\o\ac(　,\F(5,2))`), which Word draws
    // as the fraction — question 10 of the 11_題目 exam, option (A).
    let (_, _, math) = read_field(field(" EQ \\o\\ac(　,\\F(5,2))"));
    let Some(math) = math else { panic!("a formula") };
    assert!(matches!(math.math.nodes.first(), Some(MNode::Frac { .. })), "{:?}", math.math.nodes);
}

#[test]
fn an_eq_switch_we_do_not_read_keeps_the_stored_result() {
    // An array (`\a`) isn't laid out: the field draws its result (empty here), as before.
    let (_, _, math) = read_field(field(" EQ \\a(1,2)"));
    assert!(math.is_none());
}
