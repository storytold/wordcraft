//! `w:documentProtection` (ECMA-376 §17.15.1.29): Word enforces protection when
//! `w:enforcement` is missing ([MS-OI29500] §2.1.402 c), so the reader must keep the
//! restriction — and, on save, its hash — rather than unlocking the document. Synthetic
//! fixture written for this test.

use std::io::{Cursor, Write};

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

fn fixture(protection: &str) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let parts: [(&str, String); 3] = [
        ("word/document.xml", format!(r#"<w:document {W_NS}><w:body><w:p><w:r><w:t>Locked</w:t></w:r></w:p></w:body></w:document>"#)),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}settings" Target="settings.xml"/></Relationships>"#
            ),
        ),
        ("word/settings.xml", format!(r#"<w:settings {W_NS}>{protection}</w:settings>"#)),
    ];
    for (name, text) in parts {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(text.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn protection_without_enforcement_is_enforced_like_word() {
    let no_enforcement = r#"<w:documentProtection w:edit="readOnly" w:cryptAlgorithmSid="4" w:cryptSpinCount="100000" w:hash="AAAA" w:salt="BBBB"/>"#;
    let read = wordcraft_docx::read(&fixture(no_enforcement)).unwrap();
    assert_eq!(read.settings.protection.as_deref(), Some("readOnly"), "Word enforces protection with no w:enforcement");
    assert!(read.settings.protect_hash.iter().any(|(k, _)| k == "w:hash"), "the password hash is kept");
    let again = wordcraft_docx::read(&wordcraft_docx::write(&read).unwrap()).unwrap();
    assert_eq!(again.settings.protection.as_deref(), Some("readOnly"), "saving keeps the protection");
    assert!(again.settings.protect_hash.iter().any(|(k, _)| k == "w:hash"), "saving keeps the hash");
    // Explicitly off stays off (ignored but persisted, as the standard says).
    let off = wordcraft_docx::read(&fixture(r#"<w:documentProtection w:edit="readOnly" w:enforcement="0"/>"#)).unwrap();
    assert!(off.settings.protection.is_none());
}
