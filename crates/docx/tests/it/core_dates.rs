//! Core properties dates (ECMA-376 Part 2 §11, `dcterms:created` / `dcterms:modified` typed
//! `dcterms:W3CDTF`): a value that isn't a W3C date-time is not written back, so Word doesn't
//! report the saved file as corrupted and repair it. Synthetic fixtures written for these tests.
use std::io::{Cursor, Read, Write};

use wordcraft_doc::Document;

const CORE_NS: &str = concat!(
    r#"xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" "#,
    r#"xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" "#,
    r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#
);

/// Values Word and W3CDTF accept: kept.
const VALID: &[&str] = &[
    "2026-01-01T00:00:00Z",
    "2014-10-28T11:34:00Z",
    "2014-10-28T11:34Z",
    "2014-10-28T11:34:00.5+01:00",
    "2014-10-28T11:34:00-08:00",
    "2014-10-28T11:34:00",
    "2014-10-28",
    "2014-10",
    "2014",
];

/// Values that aren't dates: dropped.
const INVALID: &[&str] = &[
    "2014aaa-10-28T11:34:00Z",
    "2015sss-06-20T07:40:00Z",
    "2014-13-01T00:00:00Z",
    "2014-00-10T00:00:00Z",
    "2014-10-32T00:00:00Z",
    "2014-10-28T24:00:00Z",
    "2014-10-28T11:60:00Z",
    "2014-10-28T11:34:00+25:00",
    "2014-10-28 11:34:00Z",
    "14-10-28",
    "yesterday",
];

fn part(bytes: &[u8], name: &str) -> Option<String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut f = z.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    Some(s)
}

fn replace_part(bytes: &[u8], name: &str, data: &str) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let n = f.name().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        out.start_file(n.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(if n == name { data.as_bytes() } else { &buf }).unwrap();
    }
    out.finish().unwrap().into_inner()
}

fn core_xml_of(created: &str, modified: &str) -> String {
    let mut d = Document::new();
    d.core.creator = "Someone".into();
    d.core.created = created.into();
    d.core.modified = modified.into();
    part(&wordcraft_docx::write(&d).unwrap(), "docProps/core.xml").expect("core.xml written")
}

#[test]
fn the_writer_keeps_w3cdtf_dates() {
    for v in VALID {
        let xml = core_xml_of(v, v);
        assert!(xml.contains(&format!(r#"<dcterms:created xsi:type="dcterms:W3CDTF">{v}</dcterms:created>"#)), "{v}: {xml}");
        assert!(xml.contains(&format!(r#"<dcterms:modified xsi:type="dcterms:W3CDTF">{v}</dcterms:modified>"#)), "{v}: {xml}");
    }
}

#[test]
fn the_writer_leaves_out_dates_that_are_not_w3cdtf() {
    for v in INVALID {
        let xml = core_xml_of(v, "2015-06-20T07:40:00Z");
        assert!(!xml.contains("dcterms:created"), "{v}: {xml}");
        assert!(!xml.contains(v), "{v}: {xml}");
        // The other date and the other properties are still written.
        assert!(xml.contains(r#"<dcterms:modified xsi:type="dcterms:W3CDTF">2015-06-20T07:40:00Z</dcterms:modified>"#), "{v}: {xml}");
        assert!(xml.contains("<dc:creator>Someone</dc:creator>"), "{v}: {xml}");
        let xml = core_xml_of("2014-10-28T11:34:00Z", v);
        assert!(!xml.contains("dcterms:modified"), "{v}: {xml}");
        assert!(xml.contains(r#"<dcterms:created xsi:type="dcterms:W3CDTF">2014-10-28T11:34:00Z</dcterms:created>"#), "{v}: {xml}");
    }
}

/// The reader doesn't take a date that isn't one into the document (File › Info would show it),
/// and a file with one saves clean: the BadDocProps shape (bad `created` and `modified`, empty
/// title and subject elements).
#[test]
fn reading_drops_dates_that_are_not_w3cdtf() {
    let base = wordcraft_docx::write(&Document::new()).unwrap();
    for (created, modified) in [("2014aaa-10-28T11:34:00Z", "2015sss-06-20T07:40:00Z"), ("2014-10-28T11:34:00Z", "not a date")] {
        let core = format!(
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties {ns}>"#,
                "<dc:title></dc:title><dc:subject></dc:subject><dc:creator>Someone</dc:creator>",
                "<cp:revision>6</cp:revision>",
                r#"<dcterms:created xsi:type="dcterms:W3CDTF">{c}</dcterms:created>"#,
                r#"<dcterms:modified xsi:type="dcterms:W3CDTF">{m}</dcterms:modified></cp:coreProperties>"#
            ),
            ns = CORE_NS,
            c = created,
            m = modified
        );
        let file = replace_part(&base, "docProps/core.xml", &core);
        let d = wordcraft_docx::read(&file).unwrap();
        let ok = |v: &str| VALID.contains(&v);
        assert_eq!(d.core.created, if ok(created) { created } else { "" }, "{created}");
        assert_eq!(d.core.modified, if ok(modified) { modified } else { "" }, "{modified}");
        assert_eq!(d.core.creator, "Someone");
        assert_eq!(d.core.revision, 6);
        let xml = part(&wordcraft_docx::write(&d).unwrap(), "docProps/core.xml").unwrap();
        assert!(!xml.contains("aaa") && !xml.contains("sss") && !xml.contains("not a date"), "{xml}");
    }
}
