//! Right-to-left / left-to-right embeddings and overrides: `w:dir` (ECMA-376 Part 1 §17.3.2.8) and
//! `w:bdo` (§17.3.2.3) are read as the Unicode explicit formatting characters (U+202A..U+202E,
//! which the layout's bidi algorithm honours) and written back as the elements. Fixtures are
//! hand-written from the spec.

use std::io::{Read, Write};

use wordcraft_doc::{Block, Document};

const RLE: char = '\u{202B}';
const LRE: char = '\u{202A}';
const RLO: char = '\u{202E}';
const LRO: char = '\u{202D}';
const PDF: char = '\u{202C}';

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

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

fn r(t: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{t}</w:t></w:r>"#)
}

fn read(body: &str) -> Document {
    wordcraft_docx::read(&docx(body)).unwrap()
}

fn first_text(d: &Document) -> String {
    d.body.iter().find_map(|b| if let Block::Para(p) = &**b { Some(p.text.clone()) } else { None }).unwrap()
}

/// `x`, then `<tag w:val=val>abc</tag>`, then `y`, in one paragraph.
fn wrapped(tag: &str, val: &str) -> String {
    format!(r#"<w:p>{}<w:{tag} w:val="{val}">{}</w:{tag}>{}</w:p>"#, r("x"), r("abc"), r("y"))
}

/// The texts of all `w:t` elements in `xml`.
fn t_texts(xml: &str) -> Vec<String> {
    xml.split("<w:t")
        .skip(1)
        .filter(|s| s.starts_with('>') || s.starts_with(' '))
        .map(|s| s.split_once('>').unwrap().1.split("</w:t>").next().unwrap().to_string())
        .collect()
}

#[test]
fn each_embedding_and_override_reads_as_its_formatting_characters() {
    for (tag, val, open) in [("dir", "rtl", RLE), ("dir", "ltr", LRE), ("bdo", "rtl", RLO), ("bdo", "ltr", LRO)] {
        let text = first_text(&read(&wrapped(tag, val)));
        assert_eq!(text, format!("x{open}abc{PDF}y"), "w:{tag} {val}");
    }
}

#[test]
fn an_unknown_direction_reads_the_text_only() {
    assert_eq!(first_text(&read(&wrapped("dir", "up"))), "xabcy");
}

#[test]
fn nested_wrappers_nest_their_characters() {
    let body = format!(r#"<w:p><w:dir w:val="rtl">{}<w:bdo w:val="ltr">{}</w:bdo></w:dir></w:p>"#, r("a"), r("b"));
    assert_eq!(first_text(&read(&body)), format!("{RLE}a{LRO}b{PDF}{PDF}"));
}

#[test]
fn each_embedding_and_override_is_saved_as_its_element() {
    for (tag, val) in [("dir", "rtl"), ("dir", "ltr"), ("bdo", "rtl"), ("bdo", "ltr")] {
        let d = read(&wrapped(tag, val));
        let out = wordcraft_docx::write(&d).unwrap();
        let xml = document_xml(&out);
        let open = format!(r#"<w:{tag} w:val="{val}">"#);
        let start = xml.find(&open).unwrap_or_else(|| panic!("w:{tag} {val} not written: {xml}"));
        let end = xml[start..].find(&format!("</w:{tag}>")).expect("element closed") + start;
        assert_eq!(t_texts(&xml[start..end]).concat(), "abc", "{xml}");
        assert!(t_texts(&xml).iter().all(|t| !t.chars().any(|c| ('\u{202A}'..='\u{202E}').contains(&c))), "no formatting characters as text: {xml}");
        assert_eq!(t_texts(&xml).concat(), "xabcy", "{xml}");
        assert_eq!(first_text(&wordcraft_docx::read(&out).unwrap()), first_text(&d), "w:{tag} {val} reads back the same");
    }
}

#[test]
fn nested_wrappers_are_saved_nested() {
    let body = format!(r#"<w:p><w:dir w:val="rtl">{}<w:bdo w:val="ltr">{}</w:bdo></w:dir></w:p>"#, r("a"), r("b"));
    let d = read(&body);
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    let dir = xml.find(r#"<w:dir w:val="rtl">"#).expect("w:dir written");
    let bdo = xml.find(r#"<w:bdo w:val="ltr">"#).expect("w:bdo written");
    let bdo_end = xml.find("</w:bdo>").unwrap();
    let dir_end = xml.find("</w:dir>").unwrap();
    assert!(dir < bdo && bdo < bdo_end && bdo_end < dir_end, "{xml}");
    assert_eq!(t_texts(&xml[bdo..bdo_end]).concat(), "b", "{xml}");
    assert_eq!(t_texts(&xml[dir..dir_end]).concat(), "ab", "{xml}");
}

#[test]
fn an_unpaired_embedding_character_stays_text() {
    let body = format!("<w:p>{}</w:p>", r(&format!("x{RLE}abc")));
    let d = read(&body);
    assert_eq!(first_text(&d), format!("x{RLE}abc"));
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(!xml.contains("<w:dir"), "{xml}");
    assert_eq!(t_texts(&xml).concat(), format!("x{RLE}abc"), "{xml}");
}
