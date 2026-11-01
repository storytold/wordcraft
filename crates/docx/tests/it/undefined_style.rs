//! A paragraph whose `w:pStyle` names a style `styles.xml` doesn't define is formatted as Normal,
//! as in Word, not with the document defaults alone (#5077). Pandoc writes footers as
//! `<w:pStyle w:val="Footer"/>` without defining Footer.
use std::io::Write;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{Block, Document, PartKind};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn package(normal_id: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R}"><w:body><w:p><w:pPr><w:pStyle w:val="Footer"/></w:pPr><w:r><w:t>body</w:t></w:r></w:p><w:p><w:r><w:t>normal</w:t></w:r></w:p><w:sectPr><w:footerReference w:type="default" r:id="rId2"/></w:sectPr></w:body></w:document>"#
    );
    let footer = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="{W}"><w:p><w:pPr><w:pStyle w:val="Footer"/><w:jc w:val="center"/></w:pPr><w:r><w:t>1</w:t></w:r></w:p></w:ftr>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="24"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="200"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="{normal_id}"><w:name w:val="Normal"/><w:pPr><w:spacing w:before="20" w:after="40" w:line="259" w:lineRule="auto"/></w:pPr><w:rPr><w:sz w:val="26"/></w:rPr></w:style></w:styles>"#
    );
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#;
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in [
        ("_rels/.rels", root),
        ("word/document.xml", doc.as_str()),
        ("word/_rels/document.xml.rels", rels),
        ("word/styles.xml", styles.as_str()),
        ("word/footer1.xml", footer.as_str()),
    ] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(body.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn footer_para(d: &Document) -> &wordcraft_doc::Paragraph {
    let part = d.parts.values().find(|p| p.kind == PartKind::Footer).expect("a footer");
    match part.blocks.first().map(|b| &**b) {
        Some(Block::Para(p)) => p,
        _ => panic!("the footer starts with a paragraph"),
    }
}

#[test]
fn a_paragraph_naming_an_undefined_style_is_formatted_as_normal() {
    // Also when the file's default paragraph style has another id (read as Normal).
    for normal_id in ["Normal", "a"] {
        let d = wordcraft_docx::read(&package(normal_id)).unwrap();
        let Some(Block::Para(body)) = d.body.first().map(|b| &**b) else { panic!("a body paragraph") };
        let Some(Block::Para(normal)) = d.body.get(1).map(|b| &**b) else { panic!("a second body paragraph") };
        let normal_rp = d.styles.resolve_para(&normal.props);
        assert_eq!((normal_rp.space_before, normal_rp.space_after), (1.0, 2.0), "{normal_id}");
        for p in [body, footer_para(&d)] {
            // The paragraph keeps the style it names, for the round trip.
            assert_eq!(p.props.style.as_deref(), Some("Footer"));
            let rp = d.styles.resolve_para(&p.props);
            assert_eq!((rp.space_before, rp.space_after), (1.0, 2.0), "{normal_id}: Normal's spacing, not the defaults' 10 pt after");
            assert_eq!(rp.line_spacing, LineSpacing::Multiple(259.0 / 240.0), "{normal_id}");
            assert_eq!(d.styles.resolve_char(Some("Footer"), &Default::default()).size, 13.0, "{normal_id}");
        }
    }
}

#[test]
fn an_undefined_style_is_written_back_as_it_was_named() {
    let d = wordcraft_docx::read(&package("Normal")).unwrap();
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(footer_para(&again).props.style.as_deref(), Some("Footer"));
    assert_eq!(again.styles.resolve_para(&footer_para(&again).props).space_after, 2.0);
}
