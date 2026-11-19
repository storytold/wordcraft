//! Emphasis marks (`w:em`, ISO/IEC 29500-1 §17.3.2.12: Font › Emphasis mark) are read and
//! saved again, in their place in `w:rPr`. Fixtures are hand-written from the spec.

use wordcraft_doc::props::EmphasisMark;

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = craft_zip::Writer::new();
    for (name, xml) in [("_rels/.rels", root.to_string()), ("word/document.xml", doc)] {
        zw.add(name, xml.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    }
    zw.finish().unwrap()
}

fn marks(d: &wordcraft_doc::Document) -> Vec<Option<EmphasisMark>> {
    d.body[0].as_para().unwrap().runs.iter().map(|r| r.props.emphasis).collect()
}

#[test]
fn emphasis_marks_are_read_and_saved() {
    let run = |t: &str, em: &str| format!(r#"<w:r><w:rPr><w:cs/>{em}<w:lang w:val="ja-JP"/></w:rPr><w:t>{t}</w:t></w:r>"#);
    let body = format!(
        "<w:p>{}{}{}{}{}{}</w:p>",
        run("a", r#"<w:em w:val="dot"/>"#),
        run("b", r#"<w:em w:val="comma"/>"#),
        run("c", r#"<w:em w:val="circle"/>"#),
        run("d", r#"<w:em w:val="underDot"/>"#),
        run("e", r#"<w:em w:val="none"/>"#),
        run("f", r#"<w:em w:val="sparkle"/>"#),
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    // An unknown mark is none (and the run with it joins the one before, now alike).
    let want = [EmphasisMark::Dot, EmphasisMark::Comma, EmphasisMark::Circle, EmphasisMark::UnderDot, EmphasisMark::None].map(Some);
    assert_eq!(marks(&d), want);
    let saved = wordcraft_docx::write(&d).unwrap();
    let mut z = craft_zip::Archive::open(&saved, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 }).unwrap();
    let xml = String::from_utf8(z.read("word/document.xml", 16 << 20).unwrap().unwrap()).unwrap();
    // In its place: after w:cs, before w:lang.
    assert!(xml.contains(r#"<w:cs/><w:em w:val="underDot"/><w:lang"#), "{xml}");
    assert_eq!(marks(&wordcraft_docx::read(&saved).unwrap()), want);
}
