//! Section text direction (`w:textDirection` in `w:sectPr`, ECMA-376 Part 1 §17.6.20). Fixtures
//! are hand-written from the spec.

use std::io::{Read, Write};

use wordcraft_doc::props::TextDirection;

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn docx_with_styles(body: &str, styles: &str) -> Vec<u8> {
    let mut bytes = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let document_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    for (name, value) in [
        ("_rels/.rels", root.as_bytes()),
        ("word/_rels/document.xml.rels", document_rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
    ] {
        bytes.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        bytes.write_all(value).unwrap();
    }
    bytes.finish().unwrap().into_inner()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn section_text_direction_round_trips_in_schema_order() {
    // Three sections: vertical East Asian (tbRl), bottom-to-top (btLr) and a value no one knows.
    let sect = |dir: &str| format!(r#"<w:sectPr><w:titlePg/><w:textDirection w:val="{dir}"/><w:bidi/></w:sectPr>"#);
    let body = format!(
        r#"<w:p><w:pPr>{}</w:pPr><w:r><w:t>縦書き</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr><w:r><w:t>Up</w:t></w:r></w:p><w:p><w:r><w:t>Odd</w:t></w:r></w:p>{}"#,
        sect("tbRl"),
        sect("btLr"),
        sect("diagonal")
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let dirs: Vec<TextDirection> = d.sections().iter().map(|(_, s)| s.text_direction).collect();
    assert_eq!(dirs, vec![TextDirection::Down, TextDirection::Up, TextDirection::Horizontal]);
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    // After w:titlePg and before w:bidi (CT_SectPr's sequence); a horizontal section writes none.
    assert!(xml.contains(r#"<w:titlePg/><w:textDirection w:val="tbRl"/><w:bidi/>"#), "{xml}");
    assert!(xml.contains(r#"<w:textDirection w:val="btLr"/>"#));
    assert_eq!(xml.matches("w:textDirection").count(), 2);
    let back = wordcraft_docx::read(&out).unwrap();
    let again: Vec<TextDirection> = back.sections().iter().map(|(_, s)| s.text_direction).collect();
    assert_eq!(again, dirs);
}

#[test]
fn explicit_horizontal_direction_is_kept_in_sections_and_cells() {
    let body = r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:textDirection w:val="lrTb"/></w:tcPr><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:textDirection w:val="lrTb"/></w:sectPr>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert_eq!(xml.matches(r#"<w:textDirection w:val="lrTb"/>"#).count(), 2, "{xml}");
    // A horizontal section or cell that never said so still writes nothing.
    let plain = wordcraft_docx::read(&docx(r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr/>"#)).unwrap();
    assert!(!document_xml(&wordcraft_docx::write(&plain).unwrap()).contains("textDirection"));
}

#[test]
fn paragraph_text_direction_round_trips_through_two_saves_and_json() {
    use wordcraft_doc::props::ParaProps;
    use wordcraft_doc::props::ParagraphTextDirection;

    let values = ["lrTb", "lrTbV", "tb", "tbRl", "tbRlV", "tbLr", "tbLrV", "btLr", "tbV", "rl", "rlV", "lr", "lrV"];
    let mut body: String =
        values.iter().map(|value| format!(r#"<w:p><w:pPr><w:textDirection w:val="{value}"/></w:pPr><w:r><w:t>{value}</w:t></w:r></w:p>"#)).collect();
    body.push_str(r#"<w:p><w:r><w:t>Inherited</w:t></w:r></w:p>"#);
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let paras: Vec<_> = d.body.iter().filter_map(|b| b.as_para()).collect();
    for (para, value) in paras.iter().zip(values) {
        assert_eq!(para.props.text_direction, ParagraphTextDirection::from_ooxml(value), "{value}");
    }
    assert_eq!(paras[values.len()].props.text_direction, None);
    let json = serde_json::to_value(&paras[0].props).unwrap();
    assert_eq!(json["textDirection"], "lrTb");
    let decoded: ParaProps = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.text_direction, Some(ParagraphTextDirection::LrTb));

    let first = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&first);
    for value in values {
        assert!(xml.contains(&format!(r#"<w:textDirection w:val="{value}"/>"#)), "{xml}");
    }
    assert_eq!(xml.matches("w:textDirection").count(), values.len(), "inherited direction stays absent: {xml}");
    let second = wordcraft_docx::write(&wordcraft_docx::read(&first).unwrap()).unwrap();
    let twice = document_xml(&second);
    assert_eq!(twice.matches("w:textDirection").count(), values.len(), "{twice}");
    for value in values {
        assert!(twice.contains(&format!(r#"<w:textDirection w:val="{value}"/>"#)), "{twice}");
    }
}

#[test]
fn paragraph_style_text_direction_is_preserved_and_direct_value_overrides_it() {
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Vertical"><w:name w:val="Vertical"/><w:pPr><w:textDirection w:val="tbRl"/></w:pPr></w:style></w:styles>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="Vertical"/></w:pPr><w:r><w:t>Style</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Vertical"/><w:textDirection w:val="lrTb"/></w:pPr><w:r><w:t>Override</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx_with_styles(body, styles)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert_eq!(xml.matches(r#"<w:pStyle w:val="Vertical"/>"#).count(), 2, "both paragraphs keep their style: {xml}");
    assert!(xml.contains(r#"<w:pStyle w:val="Vertical"/><w:textDirection w:val="lrTb"/>"#), "direct horizontal is explicit: {xml}");
    assert_eq!(xml.matches("w:textDirection").count(), 1, "only the direct paragraph writes a direction: {xml}");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(wordcraft_docx::write(&d).unwrap())).unwrap();
    let mut styles_xml = String::new();
    z.by_name("word/styles.xml").unwrap().read_to_string(&mut styles_xml).unwrap();
    assert!(styles_xml.contains(r#"<w:textDirection w:val="tbRl"/>"#), "style value survives: {styles_xml}");
}
