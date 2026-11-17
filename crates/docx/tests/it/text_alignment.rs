//! A paragraph's vertical text alignment (`w:textAlignment`, ECMA-376 Part 1 §17.3.1.39, values
//! from ST_TextAlignment): read and written back as read, in the schema's place after `w:jc`.
//! Word sizes a line by it when a list label is larger than the text, so a save that drops it
//! moves every line after. Fixtures are hand-written from the spec.

use wordcraft_doc::props::TextAlignment;

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

#[test]
fn text_alignment_is_written_back_as_read() {
    let para = |ppr: &str, text: &str| format!(r#"<w:p><w:pPr>{ppr}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#);
    let body = [
        para(r#"<w:jc w:val="center"/><w:textAlignment w:val="top"/><w:outlineLvl w:val="1"/>"#, "A"),
        para(r#"<w:textAlignment w:val="center"/>"#, "B"),
        para(r#"<w:textAlignment w:val="baseline"/>"#, "C"),
        para(r#"<w:textAlignment w:val="bottom"/>"#, "D"),
        para(r#"<w:textAlignment w:val="auto"/>"#, "E"),
        para(r#"<w:textAlignment w:val="sideways"/>"#, "F"),
        para("", "G"),
    ]
    .concat();
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let got: Vec<Option<TextAlignment>> = d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.text_alignment).collect();
    let want = vec![
        Some(TextAlignment::Top),
        Some(TextAlignment::Center),
        Some(TextAlignment::Baseline),
        Some(TextAlignment::Bottom),
        Some(TextAlignment::Auto),
        None,
        None,
    ];
    assert_eq!(got, want);
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    assert!(xml.contains(r#"<w:jc w:val="center"/><w:textAlignment w:val="top"/><w:outlineLvl w:val="1"/>"#), "{xml}");
    for v in ["center", "baseline", "bottom", "auto"] {
        assert_eq!(xml.matches(&format!(r#"<w:textAlignment w:val="{v}"/>"#)).count(), 1, "{v}: {xml}");
    }
    assert_eq!(xml.matches("w:textAlignment").count(), 5, "an unknown value and none write nothing: {xml}");
    let back = wordcraft_docx::read(&out).unwrap();
    assert_eq!(back.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.text_alignment).collect::<Vec<_>>(), want);
}
