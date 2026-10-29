//! East Asian typography settings of a run (`w:eastAsianLayout`, ISO/IEC 29500-1 §17.3.2.10):
//! two lines in one and horizontal in vertical are read and saved again. Fixtures are
//! hand-written from the spec.

use std::io::{Read, Write};

use wordcraft_doc::props::{CombineBrackets, EastAsianLayout};

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

/// The East Asian layout of the run holding `text` in the first paragraph.
fn layout_of(d: &wordcraft_doc::Document, text: &str) -> Option<EastAsianLayout> {
    let p = d.body.iter().find_map(|b| b.as_para()).unwrap();
    let start = p.text.find(text).unwrap();
    let mut at = 0;
    for r in &p.runs {
        if at == start {
            return r.props.east_asian_layout.clone();
        }
        at += r.len;
    }
    panic!("no run starts at {text:?}");
}

#[test]
fn two_lines_in_one_and_horizontal_in_vertical_survive_a_save() {
    // Warichu with round brackets, tate-chu-yoko compressed to the line, and a plain run beside them.
    let body = format!(
        r#"<w:p>{}{}{}{}</w:p><w:sectPr><w:textDirection w:val="tbRl"/></w:sectPr>"#,
        run("本文", ""),
        run("二行の注", r#"<w:eastAsianLayout w:id="1" w:combine="on" w:combineBrackets="round"/>"#),
        run("12", r#"<w:eastAsianLayout w:id="2" w:vert="on" w:vertCompress="true"/>"#),
        run("月", "")
    );
    let d = wordcraft_docx::read(&docx(&body, None)).unwrap();
    let warichu = EastAsianLayout { id: Some(1), combine: Some(true), combine_brackets: Some(CombineBrackets::Round), ..Default::default() };
    let tcy = EastAsianLayout { id: Some(2), vert: Some(true), vert_compress: Some(true), ..Default::default() };
    assert_eq!(layout_of(&d, "二行の注"), Some(warichu.clone()));
    assert_eq!(layout_of(&d, "12"), Some(tcy.clone()));
    assert_eq!(layout_of(&d, "本文"), None);
    assert_eq!(layout_of(&d, "月"), None);

    let out = wordcraft_docx::write(&d).unwrap();
    let xml = part(&out, "word/document.xml");
    assert_eq!(xml.matches("<w:eastAsianLayout ").count(), 2, "{xml}");
    assert!(xml.contains(r#"<w:eastAsianLayout w:id="1" w:combine="1" w:combineBrackets="round"/>"#), "{xml}");
    assert!(xml.contains(r#"<w:eastAsianLayout w:id="2" w:vert="1" w:vertCompress="1"/>"#), "{xml}");
    let back = wordcraft_docx::read(&out).unwrap();
    assert_eq!(layout_of(&back, "二行の注"), Some(warichu));
    assert_eq!(layout_of(&back, "12"), Some(tcy));
}

#[test]
fn east_asian_layout_is_written_after_lang() {
    // CT_RPr's sequence: … w:lang, w:eastAsianLayout, w:specVanish.
    let body = format!(r#"<w:p>{}</w:p>"#, run("12", r#"<w:eastAsianLayout w:vert="1"/><w:lang w:val="ja-JP"/><w:b/>"#));
    let d = wordcraft_docx::read(&docx(&body, None)).unwrap();
    let xml = part(&wordcraft_docx::write(&d).unwrap(), "word/document.xml");
    let lang = xml.find("<w:lang ").unwrap();
    let eal = xml.find("<w:eastAsianLayout ").unwrap();
    assert!(lang < eal, "{xml}");
    assert!(xml.contains(r#"<w:eastAsianLayout w:vert="1"/>"#), "{xml}");
}

#[test]
fn a_character_style_keeps_its_east_asian_layout() {
    let styles = r#"<w:style w:type="character" w:styleId="TCY"><w:name w:val="Tate Chu Yoko"/><w:rPr><w:eastAsianLayout w:id="7" w:vert="on"/></w:rPr></w:style>"#;
    let body = format!(r#"<w:p>{}</w:p>"#, run("12", r#"<w:rStyle w:val="TCY"/>"#));
    let d = wordcraft_docx::read(&docx(&body, Some(styles))).unwrap();
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = part(&out, "word/styles.xml");
    assert!(xml.contains(r#"<w:eastAsianLayout w:id="7" w:vert="1"/>"#), "{xml}");
}

#[test]
fn values_outside_the_schema_are_dropped_not_guessed() {
    // An id past xsd:int, an unknown bracket and an on/off that isn't one: those attributes are
    // unset; the valid one beside them is kept. An element with nothing valid writes nothing.
    let body = format!(
        r#"<w:p>{}{}</w:p>"#,
        run("注", r#"<w:eastAsianLayout w:id="99999999999" w:combine="yes" w:combineBrackets="wavy" w:vert="off"/>"#),
        run("x", r#"<w:eastAsianLayout w:combineBrackets="wavy"/>"#)
    );
    let d = wordcraft_docx::read(&docx(&body, None)).unwrap();
    assert_eq!(layout_of(&d, "注"), Some(EastAsianLayout { vert: Some(false), ..Default::default() }));
    assert_eq!(layout_of(&d, "x"), None);
    let xml = part(&wordcraft_docx::write(&d).unwrap(), "word/document.xml");
    assert_eq!(xml.matches("<w:eastAsianLayout ").count(), 1, "{xml}");
    assert!(xml.contains(r#"<w:eastAsianLayout w:vert="0"/>"#), "{xml}");
}
