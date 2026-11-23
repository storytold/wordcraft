//! Compound border lines of thin and thick strokes (`thinThickSmallGap` to
//! `thinThickThinLargeGap`, ECMA-376 Part 1 §17.18.2 ST_Border) on paragraphs, table cells and
//! pages: written back as read, not as a single line. Fixtures are hand-written from the spec.

use wordcraft_doc::props::BorderStyle;

const LINES: [(&str, BorderStyle); 9] = [
    ("thinThickSmallGap", BorderStyle::ThinThickSmallGap),
    ("thickThinSmallGap", BorderStyle::ThickThinSmallGap),
    ("thinThickThinSmallGap", BorderStyle::ThinThickThinSmallGap),
    ("thinThickMediumGap", BorderStyle::ThinThickMediumGap),
    ("thickThinMediumGap", BorderStyle::ThickThinMediumGap),
    ("thinThickThinMediumGap", BorderStyle::ThinThickThinMediumGap),
    ("thinThickLargeGap", BorderStyle::ThinThickLargeGap),
    ("thickThinLargeGap", BorderStyle::ThickThinLargeGap),
    ("thinThickThinLargeGap", BorderStyle::ThinThickThinLargeGap),
];

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = craft_zip::Writer::new();
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.add(name, bytes, craft_zip::Compression::Deflated).unwrap();
    }
    zw.finish().unwrap()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 }).unwrap();

    String::from_utf8(z.read("word/document.xml", 16 << 20).unwrap().unwrap()).unwrap()
}

fn side(name: &str, val: &str) -> String {
    format!(r#"<w:{name} w:val="{val}" w:sz="36" w:space="1" w:color="2F5496"/>"#)
}

#[test]
fn compound_paragraph_borders_are_written_back_as_read() {
    let body: String = LINES
        .iter()
        .map(|(val, _)| {
            format!(r#"<w:p><w:pPr><w:pBdr>{}{}</w:pBdr></w:pPr><w:r><w:t>{val}</w:t></w:r></w:p>"#, side("top", val), side("bottom", val))
        })
        .collect();
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let got: Vec<Option<BorderStyle>> =
        d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.borders.and_then(|b| b.top).map(|t| t.style)).collect();
    assert_eq!(got, LINES.iter().map(|(_, s)| Some(*s)).collect::<Vec<_>>());
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    for (val, _) in LINES {
        assert_eq!(xml.matches(&format!(r#"w:val="{val}""#)).count(), 2, "{val}: {xml}");
    }
    assert!(!xml.contains(r#"w:val="single""#), "{xml}");
}

#[test]
fn compound_cell_and_page_borders_are_written_back_as_read() {
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>{}</w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:tcBorders>{}</w:tcBorders></w:tcPr><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/><w:sectPr><w:pgBorders w:offsetFrom="text">{}</w:pgBorders></w:sectPr>"#,
        side("top", "thickThinLargeGap"),
        side("left", "thinThickSmallGap"),
        side("right", "thinThickThinMediumGap")
    );
    let xml = document_xml(&wordcraft_docx::write(&wordcraft_docx::read(&docx(&body)).unwrap()).unwrap());
    for val in ["thickThinLargeGap", "thinThickSmallGap", "thinThickThinMediumGap"] {
        assert_eq!(xml.matches(&format!(r#"w:val="{val}""#)).count(), 1, "{val}: {xml}");
    }
}
