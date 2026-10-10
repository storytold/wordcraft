//! Hostile input: every case must return `Err` or a document, never panic.

use std::io::Write;

use proptest::prelude::*;
use wordcraft_docx::{DocxError, read, write};

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn doc_zip(document_xml: &[u8]) -> Vec<u8> {
    zip(&[("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", document_xml)])
}

fn sample() -> Vec<u8> {
    let mut d = wordcraft_doc::Document::from_text("Hello world\nSecond paragraph with more text\nThird");
    let t = wordcraft_doc::Table::new(2, 2, 200.0);
    d.body.insert(1, std::sync::Arc::new(wordcraft_doc::Block::Table(t)));
    write(&d).unwrap()
}

#[test]
fn random_bytes_and_empty_input() {
    assert!(matches!(read(&[]), Err(DocxError::Zip(_))));
    assert!(read(b"PK\x03\x04garbage").is_err());
    assert!(read(&[0u8; 4096]).is_err());
    let noise: Vec<u8> = (0..10_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    assert!(read(&noise).is_err());
}

#[test]
fn truncated_zip_never_panics() {
    let good = sample();
    assert!(read(&good).is_ok());
    for cut in (0..good.len()).step_by(37) {
        let _ = read(&good[..cut]);
    }
    // Flipped bytes throughout.
    for i in (0..good.len()).step_by(53) {
        let mut b = good.clone();
        b[i] ^= 0xA5;
        let _ = read(&b);
    }
}

#[test]
fn missing_or_wrong_main_part() {
    let z = zip(&[("_rels/.rels", ROOT_RELS.as_bytes()), ("word/other.xml", b"<x/>")]);
    assert!(matches!(read(&z), Err(DocxError::MissingPart(_))));
    let z = doc_zip(b"<notword/>");
    assert!(matches!(read(&z), Err(DocxError::NotWord(_))));
    let z = doc_zip(b"<w:document xmlns:w=\"x\"><unclosed");
    let _ = read(&z);
    let z = doc_zip(b"\xff\xfe<\0w\0");
    assert!(read(&z).is_err());
    // No root rels at all: falls back to word/document.xml.
    let z = zip(&[("word/document.xml", format!("<w:document {W_NS}><w:body><w:p><w:r><w:t>ok</w:t></w:r></w:p></w:body></w:document>").as_bytes())]);
    let d = read(&z).unwrap();
    assert_eq!(d.plain_text(wordcraft_doc::StoryRef::Body), "ok");
}

#[test]
fn deep_nesting_is_rejected() {
    let mut x = format!("<w:document {W_NS}><w:body>");
    for _ in 0..5_000 {
        x.push_str("<w:sdt><w:sdtContent>");
    }
    x.push_str("</w:body></w:document>");
    assert!(matches!(read(&doc_zip(x.as_bytes())), Err(DocxError::Limit(_))));
    // Nested tables just under the XML depth limit build without blowing the stack.
    let mut y = format!("<w:document {W_NS}><w:body>");
    for _ in 0..60 {
        y.push_str("<w:tbl><w:tr><w:tc>");
    }
    y.push_str("<w:p><w:r><w:t>deep</w:t></w:r></w:p>");
    for _ in 0..60 {
        y.push_str("</w:tc></w:tr></w:tbl>");
    }
    y.push_str("</w:body></w:document>");
    let d = read(&doc_zip(y.as_bytes())).unwrap();
    fn all_text(bl: &wordcraft_doc::Blocks, out: &mut String) {
        for b in bl {
            match &**b {
                wordcraft_doc::Block::Para(p) => out.push_str(&p.plain_text()),
                wordcraft_doc::Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).for_each(|c| all_text(&c.blocks, out)),
            }
        }
    }
    let mut s = String::new();
    all_text(&d.body, &mut s);
    assert!(s.contains("deep"), "{s}");
    write(&d).unwrap();
}

#[test]
fn broken_secondary_parts_are_ignored() {
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="a" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="b" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/><Relationship Id="c" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#;
    let doc = format!("<w:document {W_NS}><w:body><w:p><w:r><w:t>body</w:t></w:r></w:p></w:body></w:document>");
    let z = zip(&[
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/styles.xml", b"\xff\xfe"),
        ("word/numbering.xml", b"<w:numbering><w:num w:numId=\"1\"><w:abstractNumId w:val=\"999\"/></w:num></w:numbering>"),
        ("word/comments.xml", b"<<<"),
    ]);
    let d = read(&z).unwrap();
    assert_eq!(d.plain_text(wordcraft_doc::StoryRef::Body), "body");
    // The unresolvable list still writes.
    write(&d).unwrap();
}

#[test]
fn many_fields_and_huge_attribute_values() {
    let mut x = format!("<w:document {W_NS}><w:body><w:p>");
    for _ in 0..200 {
        x.push_str(r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r>"#);
    }
    x.push_str(&format!(r#"<w:r><w:rPr><w:rFonts w:ascii="{}"/></w:rPr><w:t>x</w:t></w:r>"#, "F".repeat(100_000)));
    x.push_str("</w:p><w:p><w:r><w:t>later</w:t></w:r></w:p></w:body></w:document>");
    let d = read(&doc_zip(x.as_bytes())).unwrap();
    assert!(d.plain_text(wordcraft_doc::StoryRef::Body).contains("later"));
}

/// A small valid document.xml to mutate.
fn base_xml() -> String {
    format!(
        r#"<w:document {W_NS}><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="24"/></w:rPr><w:t>Hi</w:t></w:r><w:hyperlink w:anchor="x"><w:r><w:t>link</w:t></w:r></w:hyperlink><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>PAGE</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:bookmarkStart w:id="0" w:name="x"/><w:bookmarkEnd w:id="0"/></w:p><w:tbl><w:tblPr><w:tblLook w:val="04A0"/></w:tblPr><w:tblGrid><w:gridCol w:w="100"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:cols w:num="2"/></w:sectPr></w:body></w:document>"#
    )
}

const SNIPPETS: &[&str] = &[
    "<w:p>",
    "</w:p>",
    "<w:tbl>",
    "<w:tr>",
    "<w:tc>",
    "</w:tc>",
    r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
    r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r>"#,
    r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>ADDIN ZOTERO_ITEM {}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#,
    r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r>"#,
    r#"<w:br w:type="page"/>"#,
    r#"<w:sz w:val="-99999999999"/>"#,
    r#"<w:gridSpan w:val="0"/>"#,
    "&amp;",
    "&#0;",
    "&#xFFFF;",
    "<![CDATA[x]]>",
    "\u{FFFC}",
    r#"<w:footnoteReference w:id="1"/>"#,
    r#"<w:drawing><wp:inline/></w:drawing>"#,
    r#"<mc:AlternateContent><mc:Choice/></mc:AlternateContent>"#,
    "<w:sectPr/>",
];

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    #[test]
    fn mutated_document_xml_never_panics(ops in proptest::collection::vec((0u8..4, any::<prop::sample::Index>(), any::<u8>(), 0usize..SNIPPETS.len()), 0..12)) {
        let mut x = base_xml().into_bytes();
        for (k, idx, byte, snip) in ops {
            let at = idx.index(x.len() + 1);
            match k {
                0 => { if at < x.len() { x[at] = byte; } }
                1 => { x.truncate(at); }
                2 => { let s = SNIPPETS[snip].as_bytes(); let at = at.min(x.len()); x.splice(at..at, s.iter().copied()); }
                _ => { if at < x.len() { x.remove(at); } }
            }
        }
        if let Ok(d) = read(&doc_zip(&x)) {
            // Whatever we managed to read must write back and read again.
            let out = write(&d).unwrap();
            read(&out).unwrap();
        }
    }

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = read(&bytes);
        let mut z = b"PK\x03\x04".to_vec();
        z.extend(&bytes);
        let _ = read(&z);
    }
}
