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
