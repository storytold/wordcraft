//! `w:dirty` on a field's begin marker: a generator that can't paginate writes a placeholder result
//! and sets it so Word rebuilds the field on open. Saving must not drop it, and must not add it.
use std::io::{Read, Write};

fn package(body: &str) -> Vec<u8> {
    let xml = format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, value) in [
        ("word/document.xml", xml.as_str()),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A complex field whose begin marker carries `attrs`.
fn field(attrs: &str, code: &str, result: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"{attrs}/></w:r><w:r><w:instrText xml:space="preserve"> {code} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

/// The `w:fldChar` begin markers of the document part, in order.
fn begins(bytes: &[u8]) -> Vec<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml.split("<w:fldChar ")
        .skip(1)
        .map(|s| s.split("/>").next().unwrap_or_default().to_string())
        .filter(|a| a.contains(r#"w:fldCharType="begin""#))
        .collect()
}

/// Reads `body`, saves it, reads that and saves again: the begin markers after each save.
fn saves(body: &str) -> [Vec<String>; 2] {
    let first = wordcraft_docx::write(&wordcraft_docx::read(&package(body)).unwrap()).unwrap();
    let second = wordcraft_docx::write(&wordcraft_docx::read(&first).unwrap()).unwrap();
    [begins(&first), begins(&second)]
}

#[test]
fn a_dirty_toc_field_stays_dirty_through_saves() {
    let body = format!(
        "<w:p>{}</w:p><w:p><w:r><w:t>Body.</w:t></w:r></w:p>",
        field(r#" w:dirty="true""#, r#"TOC \o "1-3" \h \z \u"#, "Placeholder: update fields (F9).")
    );
    for markers in saves(&body) {
        assert_eq!(markers.len(), 1, "{markers:?}");
        assert!(markers[0].contains(r#"w:dirty="true""#), "{markers:?}");
    }
}

#[test]
fn a_dirty_toc_under_a_heading_with_entries_stays_dirty() {
    let heading = r#"<w:p><w:pPr><w:pStyle w:val="TOCHeading"/></w:pPr><w:r><w:t>Contents</w:t></w:r></w:p>"#;
    let entries = r#"<w:p><w:pPr><w:pStyle w:val="TOC1"/></w:pPr><w:r><w:fldChar w:fldCharType="begin" w:dirty="1"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Alpha</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="TOC1"/></w:pPr><w:r><w:t>Beta</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    for markers in saves(&format!("{heading}{entries}<w:p><w:r><w:t>Body.</w:t></w:r></w:p>")) {
        assert_eq!(markers.len(), 1, "{markers:?}");
        assert!(markers[0].contains(r#"w:dirty="true""#), "{markers:?}");
    }
}

#[test]
fn dirty_survives_on_simple_and_complex_fields_with_a_lock() {
    let simple = r#"<w:fldSimple w:instr=" DATE " w:dirty="true"><w:r><w:t>1/1/2020</w:t></w:r></w:fldSimple>"#;
    let body = format!("<w:p>{simple}</w:p><w:p>{}</w:p>", field(r#" w:fldLock="true" w:dirty="true""#, "PAGE", "3"));
    for markers in saves(&body) {
        assert_eq!(markers.len(), 2, "{markers:?}");
        assert!(markers.iter().all(|m| m.contains(r#"w:dirty="true""#)), "{markers:?}");
        assert!(markers[1].contains("w:fldLock"), "{markers:?}");
        assert!(!markers[0].contains("w:fldLock"), "{markers:?}");
    }
}

#[test]
fn fields_without_the_flag_do_not_gain_it() {
    let body = format!(
        "<w:p>{}</w:p><w:p>{}</w:p><w:p>{}</w:p>",
        field("", r#"TOC \o "1-3""#, "Entries"),
        field(r#" w:dirty="false""#, "PAGE", "3"),
        field(r#" w:fldLock="true""#, "NUMPAGES", "9"),
    );
    for markers in saves(&body) {
        assert_eq!(markers.len(), 3, "{markers:?}");
        assert!(markers.iter().all(|m| !m.contains("w:dirty")), "{markers:?}");
    }
}
