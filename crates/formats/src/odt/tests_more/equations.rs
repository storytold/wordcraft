//! Equations written as ODF formula objects (ODF 1.3 §10.4.3 `draw:object`, Part 2 packages: a
//! formula sub-document holding MathML), read back by the reader that opens LibreOffice's.

use crate::odt::{export, import};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Document, Paragraph, para_block};

fn eq_para(before: &str, linear: &str, display: bool) -> std::sync::Arc<wordcraft_doc::Block> {
    let mut p = Paragraph::new();
    p.text = format!("{before}{OBJ}");
    p.objects.push(InlineObject::Equation { linear: linear.into(), display, math: Default::default() });
    p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: Default::default() }];
    para_block(p)
}

fn equations(d: &Document) -> Vec<(String, bool)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::Equation { linear, display, .. } = o { Some((linear.clone(), *display)) } else { None })
        .collect()
}

#[test]
fn equations_round_trip_as_formula_objects() {
    let mut d = Document::new();
    d.body = vec![eq_para("Inline ", "x^2+y^2", false), eq_para("", "a/b", true)];
    let bytes = export(&d).unwrap();
    // A formula sub-document per equation, declared in the manifest.
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let names: Vec<String> = z.file_names().map(String::from).collect();
    assert!(names.iter().any(|n| n == "Object 1/content.xml") && names.iter().any(|n| n == "Object 2/content.xml"), "{names:?}");
    let mut manifest = String::new();
    std::io::Read::read_to_string(&mut z.by_name("META-INF/manifest.xml").unwrap(), &mut manifest).unwrap();
    assert!(
        manifest
            .contains(r#"manifest:full-path="Object 1/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.formula""#),
        "{manifest}"
    );
    // A formula document's content.xml has a `math:math` root (ODF 1.3 Part 3 §2.2.9 B), not an
    // `office:document-content` (the schema's start pattern doesn't cover it; the spec does).
    let mut object = String::new();
    std::io::Read::read_to_string(&mut z.by_name("Object 1/content.xml").unwrap(), &mut object).unwrap();
    let root = object.trim_start_matches(|c| c != '>').get(1..).unwrap_or("").trim_start();
    assert!(root.starts_with("<math ") && root.contains(r#"xmlns="http://www.w3.org/1998/Math/MathML""#), "{object}");
    let back = import(&bytes).unwrap();
    assert_eq!(equations(&back), [("x^2+y^2".to_string(), false), ("a/b".to_string(), true)]);
    // The equation is no longer written as text beside itself.
    assert_eq!(back.body[0].as_para().unwrap().text.replace(OBJ, "#"), "Inline #");
}

#[test]
fn a_formula_wrapped_in_office_math_still_reads() {
    use crate::odt::tests::{NS_T, zip_of};
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:body><office:text><text:p><draw:frame text:anchor-type="as-char"><draw:object xlink:href="./Object 1"/></draw:frame></text:p></office:text></office:body></office:document-content>"#
    );
    let math = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:math><math xmlns="http://www.w3.org/1998/Math/MathML" display="block"><mi>x</mi></math></office:math></office:body></office:document-content>"#;
    let bytes =
        zip_of(&[("mimetype", b"application/vnd.oasis.opendocument.text"), ("content.xml", content.as_bytes()), ("Object 1/content.xml", math)]);
    assert_eq!(equations(&import(&bytes).unwrap()), [("x".to_string(), true)]);
}
