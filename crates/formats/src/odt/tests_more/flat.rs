//! Flat OpenDocument text (`.fodt`, ODF 1.3 Part 3 §2.1.2: the whole document in one
//! `office:document` element, pictures inline as `office:binary-data`, formulas as
//! `math:math` inside their object): opened and saved like a package.

use crate::odt::{export_flat, import_flat};
use wordcraft_doc::Document;
use wordcraft_doc::para::InlineObject;

/// A 1×1 PNG, base64.
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn flat() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:math="http://www.w3.org/1998/Math/MathML" office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:meta><dc:title>Flat title</dc:title></office:meta>
 <office:styles><style:style style:name="Quiet" style:family="paragraph"><style:text-properties fo:font-style="italic"/></style:style></office:styles>
 <office:automatic-styles>
  <style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
  <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="5in" fo:page-height="7in"/></style:page-layout>
 </office:automatic-styles>
 <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:footer><text:p>The footer</text:p></style:footer></style:master-page></office:master-styles>
 <office:body><office:text>
  <text:p text:style-name="Quiet">Some <text:span text:style-name="T1">bold</text:span> words.</text:p>
  <text:p><draw:frame text:anchor-type="as-char" svg:width="1in" svg:height="1in"><draw:image><office:binary-data>
{PNG_B64}
  </office:binary-data></draw:image></draw:frame></text:p>
  <text:p><draw:frame text:anchor-type="as-char" svg:width="1in" svg:height="0.5in"><draw:object><math:math><math:semantics><math:mi>x</math:mi><math:annotation encoding="StarMath 5.0">x</math:annotation></math:semantics></math:math></draw:object></draw:frame></text:p>
 </office:text></office:body>
</office:document>"#
    )
}

/// What a flat document holds: (its title, page width, body text, bold text, the paragraph
/// style, its footer's text, its pictures' sizes in bytes, its equations).
#[allow(clippy::type_complexity)]
fn summary(d: &Document) -> (String, f32, Vec<String>, Vec<String>, Option<String>, String, Vec<usize>, Vec<String>) {
    let paras: Vec<_> = d.body.iter().filter_map(|b| b.as_para()).collect();
    let text = paras.iter().map(|p| p.text.replace(wordcraft_doc::para::OBJ, "#")).collect();
    let bold = paras
        .iter()
        .flat_map(|p| p.run_ranges().filter(|(_, c)| c.bold == Some(true)).map(|(r, _)| p.text.get(r).unwrap_or("").to_string()).collect::<Vec<_>>())
        .collect();
    let style = paras.first().and_then(|p| p.props.style.clone());
    let footer = d
        .last_section
        .footers
        .default
        .and_then(|id| d.parts.get(&id))
        .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect())
        .unwrap_or_default();
    let mut pictures = Vec::new();
    let mut equations = Vec::new();
    for p in &paras {
        for o in &p.objects {
            match o {
                InlineObject::Image { media, .. } => pictures.push(d.media.get(media).map_or(0, |m| m.len())),
                InlineObject::Equation { linear, .. } => equations.push(linear.clone()),
                _ => {}
            }
        }
    }
    (d.core.title.clone(), d.last_section.page_w, text, bold, style, footer, pictures, equations)
}

#[test]
fn a_flat_document_opens() {
    let d = import_flat(flat().as_bytes()).unwrap();
    let s = summary(&d);
    assert_eq!(s.0, "Flat title");
    assert_eq!(s.1, 360.0);
    assert_eq!(s.2, ["Some bold words.", "#", "#"]);
    assert_eq!(s.3, ["bold"]);
    assert_eq!(s.4.as_deref(), Some("Quiet"));
    assert_eq!(s.5, "The footer");
    // The picture's own bytes, decoded.
    assert_eq!(s.6, [70]);
    assert_eq!(s.7, ["x"]);
}

#[test]
fn a_flat_document_round_trips() {
    let d = import_flat(flat().as_bytes()).unwrap();
    let bytes = export_flat(&d).unwrap();
    let xml = String::from_utf8(bytes.clone()).unwrap();
    assert!(xml.contains("<office:document ") && xml.contains("office:mimetype=\"application/vnd.oasis.opendocument.text\""), "{xml}");
    assert!(xml.contains("<office:binary-data>") && !xml.contains("Pictures/"), "pictures inline: {xml}");
    assert_eq!(xml.matches("<office:automatic-styles>").count(), 1, "one set of automatic styles");
    assert_eq!(summary(&import_flat(&bytes).unwrap()), summary(&d));
    // And by its extension.
    assert_eq!(summary(&crate::import("fodt", &bytes).unwrap().unwrap()), summary(&d));
}

#[test]
fn hostile_flat_documents_do_not_panic() {
    let f = flat();
    for bad in [
        String::new(),
        "<office:document>".to_string(),
        f.replace("</office:body>", ""),
        f.replace(PNG_B64, "!!!not base64!!!"),
        f.replace("<office:binary-data>", "<office:binary-data><office:binary-data>"),
        f.replace("</math:math>", ""),
        format!("<office:document>{}</office:document>", "<office:body>".repeat(10_000)),
        "\u{feff}<?xml version=\"1.0\"?><office:document-content/>".to_string(),
    ] {
        if let Ok(d) = import_flat(bad.as_bytes()) {
            let _ = export_flat(&d).unwrap();
        }
    }
}

#[test]
fn an_inline_sub_document_does_not_end_the_body() {
    // A chart object inline holds an `office:document` with an `office:body` of its own: the
    // outer body goes on past it (the text after the chart stays), and deep nesting is bounded.
    let chart = r#"<draw:frame svg:width="2in" svg:height="1in"><draw:object><office:document office:mimetype="application/vnd.oasis.opendocument.chart"><office:body><office:chart/></office:body></office:document></draw:object></draw:frame>"#;
    let xml = format!(
        r#"<?xml version="1.0"?><office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"><office:body><office:text><text:p>Before{chart}</text:p><text:p>After the chart.</text:p></office:text></office:body></office:document>"#
    );
    let d = import_flat(xml.as_bytes()).unwrap();
    let text: Vec<String> = d.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect();
    assert_eq!(text, ["Before", "After the chart."]);
    let deep = format!("<office:document><office:body>{}", "<office:body>".repeat(5000));
    let _ = import_flat(deep.as_bytes());
}

#[test]
fn a_template_is_saved_and_opened_as_one() {
    // An OpenDocument text template (`.ott`): the package with the template's media type, in
    // `mimetype` and as the manifest's root entry (ODF 1.3 Part 2 §3.3).
    let d = crate::import("ott", &crate::odt::tests::package(&format!("<office:document-content {}><office:body><office:text><text:p>Template text.</text:p></office:text></office:body></office:document-content>", crate::odt::tests::NS_T), None))
        .unwrap()
        .unwrap();
    let bytes = crate::export("ott", &d).unwrap().unwrap();
    assert_eq!(crate::odt::tests::entry(&bytes, "mimetype"), crate::odt::TEMPLATE_MIME);
    assert!(
        crate::odt::tests::entry(&bytes, "META-INF/manifest.xml")
            .contains(r#"manifest:full-path="/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.text-template""#)
    );
    let back = crate::import("ott", &bytes).unwrap().unwrap();
    assert_eq!(back.body[0].as_para().map(|p| p.text.as_str()), Some("Template text."));
    // A document is still a document.
    assert_eq!(crate::odt::tests::entry(&crate::export("odt", &d).unwrap().unwrap(), "mimetype"), crate::odt::TEXT_MIME);
}

#[test]
fn a_template_of_styles_only_opens_empty_with_its_styles() {
    // LibreOffice's style templates hold `styles.xml` and no `content.xml`.
    use std::io::Write;
    let styles = format!(
        r#"<office:document-styles {}><office:styles><style:style style:name="Standard" style:family="paragraph"><style:text-properties style:font-name="DejaVu Serif"/></style:style></office:styles></office:document-styles>"#,
        crate::odt::tests::NS_T
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", crate::odt::TEMPLATE_MIME), ("styles.xml", styles.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    let d = crate::import("ott", &z.finish().unwrap().into_inner()).unwrap().unwrap();
    assert_eq!(d.body.len(), 1);
    assert_eq!(d.styles.default_chr.font.as_deref(), Some("DejaVu Serif"));
    // Without styles either, it isn't a document.
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file("mimetype", o).unwrap();
    z.write_all(crate::odt::TEXT_MIME.as_bytes()).unwrap();
    assert!(crate::import("odt", &z.finish().unwrap().into_inner()).unwrap().is_err());
}
