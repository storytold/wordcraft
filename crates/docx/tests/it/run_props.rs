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
fn paragraph_adjust_right_indent_inherits_from_style() {
    let styles =
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:pPr><w:adjustRightInd/></w:pPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:adjustRightInd w:val="0"/></w:pPr><w:r><w:t>off</w:t></w:r></w:p>
        <w:p><w:r><w:t>inherited</w:t></w:r></w:p>"#;
    let mut d = wordcraft_docx::read(&docx(body, styles)).unwrap();
    let paragraphs: Vec<_> = d.body.iter().filter_map(|b| b.as_para()).collect();
    assert_eq!(paragraphs[0].props.adjust_right_indent, Some(false));
    assert!(!d.styles.resolve_para(&paragraphs[0].props).adjust_right_indent);
    assert!(d.styles.resolve_para(&paragraphs[1].props).adjust_right_indent);

    for _ in 0..2 {
        let out = wordcraft_docx::write(&d).unwrap();
        assert!(part(&out, "word/document.xml").contains(r#"<w:adjustRightInd w:val="0""#));
        assert!(part(&out, "word/styles.xml").contains("<w:adjustRightInd/>"));
        d = wordcraft_docx::read(&out).unwrap();
        let paragraphs: Vec<_> = d.body.iter().filter_map(|b| b.as_para()).collect();
        assert!(!d.styles.resolve_para(&paragraphs[0].props).adjust_right_indent);
        assert!(d.styles.resolve_para(&paragraphs[1].props).adjust_right_indent);
        assert_eq!(paragraphs[0].props.adjust_right_indent, Some(false));
    }
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
fn conditional_table_flags_survive_two_saves_in_all_property_locations() {
    let styles = r#"
        <w:style w:type="character" w:styleId="RunFlags"><w:rPr><w:cnfStyle w:val="100000000000" w:firstRow="1" w:oddVBand="0"/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="ParaFlags"><w:pPr><w:cnfStyle w:val="010000000000" w:lastRow="0"/></w:pPr></w:style>
    "#;
    let body = r#"
        <w:p><w:pPr><w:pStyle w:val="ParaFlags"/><w:outlineLvl w:val="2"/></w:pPr>
          <w:r><w:rPr><w:rStyle w:val="RunFlags"/><w:cnfStyle w:val="000100000000" w:lastColumn="0"/></w:rPr><w:t>flags</w:t></w:r>
        </w:p>
        <w:p><w:pPr><w:outlineLvl w:val="2"/><w:cnfStyle w:val="101010101010" w:firstRow="1" w:lastRow="0"/></w:pPr><w:r><w:t>direct flags</w:t></w:r></w:p>
        <w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1000"/></w:tblGrid>
          <w:tr><w:trPr><w:cnfStyle w:val="000010000000" w:evenVBand="1"/></w:trPr>
            <w:tc><w:tcPr><w:tcW w:w="1000" w:type="dxa"/><w:cnfStyle w:val="000001000000" w:firstRowLastColumn="0"/></w:tcPr>
              <w:p><w:r><w:t>cell</w:t></w:r></w:p>
            </w:tc>
          </w:tr>
        </w:tbl>
    "#;
    let mut doc = wordcraft_docx::read(&docx(body, styles)).unwrap();
    let mut paragraphs = doc.body.iter().filter_map(|b| b.as_para());
    let paragraph = paragraphs.next().unwrap();
    let direct_paragraph = paragraphs.next().unwrap();
    assert_eq!(paragraph.props.cnf_style, None);
    assert_eq!(direct_paragraph.props.cnf_style.as_ref().and_then(|c| c.first_row), Some(true));
    assert_eq!(direct_paragraph.props.cnf_style.as_ref().and_then(|c| c.last_row), Some(false));
    assert_eq!(paragraph.runs[0].props.style.as_deref(), Some("RunFlags"));
    assert_eq!(paragraph.runs[0].props.cnf_style.as_ref().and_then(|c| c.last_column), Some(false));
    assert_eq!(doc.styles.get("RunFlags").unwrap().chr.cnf_style.as_ref().and_then(|c| c.first_row), Some(true));
    assert_eq!(doc.styles.get("ParaFlags").unwrap().para.cnf_style.as_ref().and_then(|c| c.last_row), Some(false));
    let table = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(table.rows[0].props.cnf_style.as_ref().and_then(|c| c.even_v_band), Some(true));
    assert_eq!(table.rows[0].cells[0].props.cnf_style.as_ref().and_then(|c| c.first_row_last_column), Some(false));

    let mut inherited = doc.styles.get("RunFlags").unwrap().chr.clone();
    inherited.overlay(&paragraph.runs[0].props);
    assert_eq!(inherited.cnf_style.as_ref().and_then(|c| c.first_row), Some(true));
    assert_eq!(inherited.cnf_style.as_ref().and_then(|c| c.last_column), Some(false));

    for pass in 0..2 {
        let out = wordcraft_docx::write(&doc).unwrap();
        let style_xml = part(&out, "word/styles.xml");
        assert!(style_xml.contains(r#"w:cnfStyle w:val="100000000000" w:firstRow="1" w:oddVBand="0""#), "pass {pass}: {style_xml}");
        assert!(style_xml.contains(r#"w:cnfStyle w:val="010000000000" w:lastRow="0""#), "pass {pass}: {style_xml}");
        let document_xml = part(&out, "word/document.xml");
        let direct_ppr = document_xml
            .split("<w:p>")
            .find(|paragraph| paragraph.contains("direct flags"))
            .unwrap()
            .split_once("<w:pPr>")
            .unwrap()
            .1
            .split_once("</w:pPr>")
            .unwrap()
            .0;
        assert!(direct_ppr.find("w:outlineLvl").unwrap() < direct_ppr.find("w:cnfStyle").unwrap(), "pass {pass}: {direct_ppr}");
        assert!(direct_ppr.contains(r#"w:cnfStyle w:val="101010101010" w:firstRow="1" w:lastRow="0""#), "pass {pass}: {direct_ppr}");
        for flag in [
            r#"w:cnfStyle w:val="000100000000" w:lastColumn="0""#,
            r#"w:cnfStyle w:val="000010000000" w:evenVBand="1""#,
            r#"w:cnfStyle w:val="000001000000" w:firstRowLastColumn="0""#,
        ] {
            assert!(document_xml.contains(flag), "pass {pass}: missing {flag} in {document_xml}");
        }
        doc = wordcraft_docx::read(&out).unwrap();
        let direct_paragraph = doc.body.iter().filter_map(|b| b.as_para()).nth(1).unwrap();
        assert_eq!(direct_paragraph.props.cnf_style.as_ref().and_then(|c| c.first_row), Some(true), "pass {pass}");
        assert_eq!(direct_paragraph.props.cnf_style.as_ref().and_then(|c| c.last_row), Some(false), "pass {pass}");
    }
}
