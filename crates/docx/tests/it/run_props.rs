//! Run and style properties that must survive repeated DOCX saves.

use std::io::{Read, Write};

fn docx(body: &str, styles: &str) -> Vec<u8> {
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr/></w:body></w:document>"#
    );
    let styles = format!(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{styles}</w:styles>"#);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in [
        ("_rels/.rels", root.to_string()),
        ("word/document.xml", document),
        ("word/_rels/document.xml.rels", rels.to_string()),
        ("word/styles.xml", styles),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
    xml
}

#[test]
fn run_and_style_snap_to_grid_and_web_hidden_survive_two_saves() {
    let styles = r#"
        <w:style w:type="character" w:styleId="On"><w:rPr><w:snapToGrid/><w:webHidden/></w:rPr></w:style>
        <w:style w:type="character" w:styleId="Off"><w:rPr><w:snapToGrid w:val="0"/><w:webHidden w:val="0"/></w:rPr></w:style>
    "#;
    let body = r#"
        <w:p>
          <w:r><w:rPr><w:rStyle w:val="On"/></w:rPr><w:t>inherit</w:t></w:r>
          <w:r><w:rPr><w:snapToGrid/><w:webHidden/></w:rPr><w:t>direct true</w:t></w:r>
          <w:r><w:rPr><w:rStyle w:val="On"/><w:snapToGrid w:val="0"/><w:webHidden w:val="0"/></w:rPr><w:t>override</w:t></w:r>
          <w:r><w:rPr><w:rStyle w:val="Off"/><w:vanish/></w:rPr><w:t>visible in print</w:t></w:r>
          <w:r><w:t>absent</w:t></w:r>
        </w:p>
    "#;
    let mut doc = wordcraft_docx::read(&docx(body, styles)).unwrap();
    let p = doc.body.iter().find_map(|b| b.as_para()).unwrap();
    assert_eq!(p.runs[0].props.snap_to_grid, None);
    assert_eq!(p.runs[0].props.web_hidden, None);
    assert_eq!(p.runs[1].props.snap_to_grid, Some(true));
    assert_eq!(p.runs[1].props.web_hidden, Some(true));
    assert_eq!(p.runs[2].props.snap_to_grid, Some(false));
    assert_eq!(p.runs[2].props.web_hidden, Some(false));
    assert_eq!(p.runs[3].props.hidden, Some(true));
    assert_eq!(p.runs[3].props.web_hidden, None);
    assert_eq!(p.runs[4].props.snap_to_grid, None);
    assert_eq!(p.runs[4].props.web_hidden, None);
    assert!(!doc.styles.resolve_char(None, &p.runs[0].props).hidden, "w:webHidden does not hide Print Layout text");
    assert!(doc.styles.resolve_char(None, &p.runs[3].props).hidden, "w:vanish still controls hidden text");
    for pass in 0..2 {
        let out = wordcraft_docx::write(&doc).unwrap();
        let style_xml = part(&out, "word/styles.xml");
        assert!(style_xml.contains(r#"<w:snapToGrid/><w:webHidden/>"#), "pass {pass}: {style_xml}");
        assert!(style_xml.contains(r#"<w:snapToGrid w:val="0"/><w:webHidden w:val="0"/>"#), "pass {pass}: {style_xml}");
        let document_xml = part(&out, "word/document.xml");
        assert!(document_xml.contains(r#"<w:snapToGrid w:val="0"/>"#), "pass {pass}: {document_xml}");
        assert!(document_xml.contains(r#"<w:webHidden w:val="0"/>"#), "pass {pass}: {document_xml}");
        assert!(document_xml.contains("<w:snapToGrid/><w:webHidden/>"), "pass {pass}: {document_xml}");
        assert!(document_xml.contains("<w:vanish/>"), "pass {pass}: {document_xml}");
        assert!(document_xml.contains("visible in print"));
        doc = wordcraft_docx::read(&out).unwrap();
    }
}

#[test]
fn run_and_style_emphasis_marks_survive_two_saves() {
    use wordcraft_doc::props::EmphasisMark;

    let styles = r#"
        <w:style w:type="character" w:styleId="Dot"><w:rPr><w:em/></w:rPr></w:style>
        <w:style w:type="character" w:styleId="Comma"><w:rPr><w:em w:val="comma"/></w:rPr></w:style>
        <w:style w:type="character" w:styleId="Circle"><w:rPr><w:em w:val="circle"/></w:rPr></w:style>
        <w:style w:type="character" w:styleId="UnderDot"><w:rPr><w:em w:val="underDot"/></w:rPr></w:style>
        <w:style w:type="character" w:styleId="NoMark"><w:rPr><w:em w:val="none"/></w:rPr></w:style>
    "#;
    let body = r#"
        <w:p>
          <w:r><w:rPr><w:rStyle w:val="Dot"/></w:rPr><w:t>default dot</w:t></w:r>
          <w:r><w:rPr><w:rStyle w:val="Comma"/></w:rPr><w:t>comma</w:t></w:r>
          <w:r><w:rPr><w:rStyle w:val="Circle"/></w:rPr><w:t>circle</w:t></w:r>
          <w:r><w:rPr><w:rStyle w:val="UnderDot"/></w:rPr><w:t>under dot</w:t></w:r>
          <w:r><w:rPr><w:rStyle w:val="Dot"/><w:em w:val="none"/></w:rPr><w:t>explicit none</w:t></w:r>
          <w:r><w:rPr><w:rStyle w:val="NoMark"/></w:rPr><w:t>style none</w:t></w:r>
          <w:r><w:rPr><w:em w:val="dot"/></w:rPr><w:t>run dot</w:t></w:r>
          <w:r><w:rPr><w:em w:val="comma"/></w:rPr><w:t>run comma</w:t></w:r>
          <w:r><w:rPr><w:em w:val="circle"/></w:rPr><w:t>run circle</w:t></w:r>
          <w:r><w:rPr><w:em w:val="underDot"/></w:rPr><w:t>run under dot</w:t></w:r>
          <w:r><w:rPr><w:em w:val="none"/></w:rPr><w:t>run none</w:t></w:r>
          <w:r><w:t>inherit absent</w:t></w:r>
        </w:p>
    "#;
    let mut doc = wordcraft_docx::read(&docx(body, styles)).unwrap();
    let p = doc.body.iter().find_map(|b| b.as_para()).unwrap();
    let expected_styles = [
        ("Dot", Some(EmphasisMark::Dot)),
        ("Comma", Some(EmphasisMark::Comma)),
        ("Circle", Some(EmphasisMark::Circle)),
        ("UnderDot", Some(EmphasisMark::UnderDot)),
        ("NoMark", Some(EmphasisMark::None)),
    ];
    for (id, mark) in expected_styles {
        assert_eq!(doc.styles.get(id).and_then(|style| style.chr.emphasis_mark), mark);
    }
    assert_eq!(p.runs[4].props.emphasis_mark, Some(EmphasisMark::None));
    let expected_runs = [
        Some(EmphasisMark::Dot),
        Some(EmphasisMark::Comma),
        Some(EmphasisMark::Circle),
        Some(EmphasisMark::UnderDot),
        Some(EmphasisMark::None),
        None,
    ];
    for (run, mark) in p.runs.iter().skip(6).zip(expected_runs) {
        assert_eq!(run.props.emphasis_mark, mark);
    }

    for pass in 0..2 {
        let out = wordcraft_docx::write(&doc).unwrap();
        let style_xml = part(&out, "word/styles.xml");
        for value in ["dot", "comma", "circle", "underDot", "none"] {
            assert!(style_xml.contains(&format!(r#"<w:em w:val="{value}"/>"#)), "pass {pass}: {style_xml}");
        }
        let document_xml = part(&out, "word/document.xml");
        assert!(document_xml.contains(r#"<w:em w:val="none"/>"#), "pass {pass}: {document_xml}");
        assert!(document_xml.contains("default dot"));
        doc = wordcraft_docx::read(&out).unwrap();
        for (id, mark) in expected_styles {
            assert_eq!(doc.styles.get(id).and_then(|style| style.chr.emphasis_mark), mark, "pass {pass}");
        }
        let p = doc.body.iter().find_map(|b| b.as_para()).unwrap();
        for (run, mark) in p.runs.iter().skip(6).zip(expected_runs) {
            assert_eq!(run.props.emphasis_mark, mark, "pass {pass}");
        }
    }
}
