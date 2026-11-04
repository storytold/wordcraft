//! East Asian language attribute of a run (`w:lang/@w:eastAsia`, ISO/IEC 29500-1 §17.3.2.20).
//! Verifies that saving DOCX documents preserves the East Asian language of `w:lang`.

use std::io::{Read, Write};

fn docx(body: &str, styles: Option<&str>) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut parts = vec![("_rels/.rels", root.to_string()), ("word/document.xml", doc)];
    if let Some(s) = styles {
        parts.push(("word/_rels/document.xml.rels", doc_rels.to_string()));
        parts.push((
            "word/styles.xml",
            format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{s}</w:styles>"#),
        ));
    }
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in &parts {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn run(text: &str, rpr: &str) -> String {
    format!(r#"<w:r><w:rPr>{rpr}</w:rPr><w:t>{text}</w:t></w:r>"#)
}

#[test]
fn east_asian_lang_survives_a_save() {
    let body = format!(
        r#"<w:p>{}{}</w:p>"#,
        run("Hello", r#"<w:lang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>"#),
        run("中文", r#"<w:lang w:eastAsia="zh-CN"/>"#)
    );
    let d = wordcraft_docx::read(&docx(&body, None)).unwrap();
    let p = d.body[0].as_para().unwrap();

    let r0 = &p.runs[0];
    assert_eq!(r0.props.lang.as_deref(), Some("en-US"));
    assert_eq!(r0.props.lang_ea.as_deref(), Some("ja-JP"));
    assert_eq!(r0.props.lang_bidi.as_deref(), Some("ar-SA"));

    let r1 = &p.runs[1];
    assert_eq!(r1.props.lang, None);
    assert_eq!(r1.props.lang_ea.as_deref(), Some("zh-CN"));
    assert_eq!(r1.props.lang_bidi, None);

    let out = wordcraft_docx::write(&d).unwrap();
    let xml = part(&out, "word/document.xml");
    assert!(xml.contains(r#"w:eastAsia="ja-JP""#), "{xml}");
    assert!(xml.contains(r#"w:eastAsia="zh-CN""#), "{xml}");

    let back = wordcraft_docx::read(&out).unwrap();
    let back_p = back.body[0].as_para().unwrap();
    assert_eq!(back_p.runs[0].props.lang.as_deref(), Some("en-US"));
    assert_eq!(back_p.runs[0].props.lang_ea.as_deref(), Some("ja-JP"));
    assert_eq!(back_p.runs[0].props.lang_bidi.as_deref(), Some("ar-SA"));
    assert_eq!(back_p.runs[1].props.lang_ea.as_deref(), Some("zh-CN"));
}

#[test]
fn east_asian_lang_style_and_whitespace_trim() {
    let styles =
        r#"<w:style w:type="character" w:styleId="CJKLang"><w:name w:val="CJK Language"/><w:rPr><w:lang w:eastAsia="  ko-KR  "/></w:rPr></w:style>"#;
    let body = format!(r#"<w:p>{}</w:p>"#, run("한국어", r#"<w:rStyle w:val="CJKLang"/>"#));
    let d = wordcraft_docx::read(&docx(&body, Some(styles))).unwrap();

    let style = d.styles.get("CJKLang").unwrap();
    assert_eq!(style.chr.lang_ea.as_deref(), Some("ko-KR"));

    let out = wordcraft_docx::write(&d).unwrap();
    let styles_xml = part(&out, "word/styles.xml");
    assert!(styles_xml.contains(r#"w:eastAsia="ko-KR""#), "{styles_xml}");
}
