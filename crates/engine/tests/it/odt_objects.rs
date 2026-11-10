//! An embedded spreadsheet in an ODT file (ODF 1.3 §10.4.6) opens as a picture WordCraft keeps
//! the object behind: saved as ODT it is the spreadsheet again; saved as DOCX it is the picture
//! alone, never ODF markup. The ODT is original and synthetic, shaped like LibreOffice's.

use std::io::{Read, Write};

use wordcraft_doc::para::InlineObject;
use wordcraft_engine::io;

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;

fn odt() -> Vec<u8> {
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text><text:p>Costs <draw:frame svg:width="3in" svg:height="1in"><draw:object xlink:href="./Object 1"/><draw:image xlink:href="./ObjectReplacements/Object 1"/></draw:frame></text:p></office:text></office:body></office:document-content>"#
    );
    let sheet = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:spreadsheet/></office:body></office:document-content>"#;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let entries: [(&str, &[u8]); 4] = [
        ("mimetype", b"application/vnd.oasis.opendocument.text"),
        ("content.xml", content.as_bytes()),
        ("Object 1/content.xml", sheet.as_bytes()),
        ("ObjectReplacements/Object 1", b"VCLMTF not a picture WordCraft draws"),
    ];
    for (name, data) in entries {
        z.start_file(name, o).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn entry(pkg: &[u8], name: &str) -> Option<String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(pkg)).unwrap();
    let mut s = String::new();
    z.by_name(name).ok()?.read_to_string(&mut s).ok()?;
    Some(s)
}

#[test]
fn an_embedded_spreadsheet_survives_odt_and_becomes_a_picture_in_docx() {
    let doc = io::open_bytes("sheet.odt", &odt()).unwrap();
    let pictures =
        doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).filter(|o| matches!(o, InlineObject::Image { .. })).count();
    assert_eq!(pictures, 1, "shown, by a placeholder");
    let odt = io::save_bytes("sheet.odt", &doc).unwrap();
    assert!(entry(&odt, "Object 1/content.xml").is_some_and(|c| c.contains("office:spreadsheet")));
    let docx = io::save_bytes("sheet.docx", &doc).unwrap();
    let body = entry(&docx, "word/document.xml").unwrap();
    assert!(!body.contains("odf-object") && body.contains("<w:drawing>"), "{body}");
    // The DOCX opens with its picture.
    let again = io::open_bytes("sheet.docx", &docx).unwrap();
    assert!(again.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).any(|o| matches!(o, InlineObject::Image { .. })));
}

#[test]
fn docx_layout_in_cell_survives_odt_round_trip() {
    // Original synthetic frame in a cell; the DOCX writer supplies the anchor markup.
    let content = format!(
        r#"<office:document-content {NS} xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:text>
        <table:table><table:table-row><table:table-cell><text:p><draw:frame text:anchor-type="page" svg:width="1in" svg:height="1in"><draw:text-box><text:p>Box</text:p></draw:text-box></draw:frame></text:p></table:table-cell></table:table-row></table:table>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file("content.xml", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(content.as_bytes()).unwrap();
    let seed = io::open_bytes("frame.odt", &z.finish().unwrap().into_inner()).unwrap();
    for outside in [false, true] {
        let mut doc = seed.clone();
        let wordcraft_doc::Block::Table(t) = std::sync::Arc::make_mut(&mut doc.body[0]) else { panic!("table") };
        let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut t.rows[0].cells[0].blocks[0]) else { panic!("paragraph") };
        let InlineObject::Shape { float, .. } = &mut p.objects[0] else { panic!("frame") };
        float.outside_cell = outside;
        let docx = io::save_bytes("frame.docx", &doc).unwrap();
        let flag = if outside { "0" } else { "1" };
        assert!(entry(&docx, "word/document.xml").unwrap().contains(&format!("layoutInCell=\"{flag}\"")));
        let doc = io::open_bytes("frame.docx", &docx).unwrap();
        let odt = io::save_bytes("frame.odt", &doc).unwrap();
        assert!(entry(&odt, "content.xml").unwrap().contains(&format!("style:flow-with-text=\"{}\"", !outside)));
        let back = io::open_bytes("frame.odt", &odt).unwrap();
        let wordcraft_doc::Block::Table(t) = back.body[0].as_ref() else { panic!("table") };
        let p = t.rows[0].cells[0].blocks[0].as_para().unwrap();
        let InlineObject::Shape { float, .. } = &p.objects[0] else { panic!("frame") };
        assert_eq!(float.outside_cell, outside);
        let saved = io::save_bytes("frame.docx", &back).unwrap();
        assert!(entry(&saved, "word/document.xml").unwrap().contains(&format!("layoutInCell=\"{flag}\"")));
    }
}
