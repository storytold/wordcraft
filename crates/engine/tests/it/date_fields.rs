//! Unlocked `DATE` and `TIME` fields are brought up to now when a document is opened, as Word
//! does whenever it shows, prints or exports one (#1672). Packages are hand-written: the writer
//! would never produce the stale cached result this is about.

use std::io::Write;

use wordcraft_doc::{InlineObject, StoryRef};

fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(body.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A field run: its code, its cached result, and whether it carries `w:fldLock`.
fn field(instr: &str, cached: &str, locked: bool) -> String {
    let lock = if locked { r#" w:fldLock="1""# } else { "" };
    format!(
        concat!(
            r#"<w:r><w:fldChar w:fldCharType="begin"{}/></w:r>"#,
            r#"<w:r><w:instrText xml:space="preserve"> {} </w:instrText></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r>"#,
            r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        ),
        lock,
        instr.replace('"', "&quot;"),
        cached
    )
}

fn open_body(body: &str) -> wordcraft_doc::Document {
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}</w:body></w:document>"#
    );
    wordcraft_engine::io::open_bytes("d.docx", &zip(&[("_rels/.rels", rels), ("word/document.xml", &doc)])).expect("opens")
}

/// Every field of the body, as (code, result).
fn fields(d: &wordcraft_doc::Document) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in d.para_paths(StoryRef::Body) {
        let Some(p) = d.para(StoryRef::Body, &path) else { continue };
        for o in &p.objects {
            if let InlineObject::Field { instr, result, .. } = o {
                out.push((instr.clone(), result.clone()));
            }
        }
    }
    out
}

#[test]
fn unlocked_date_and_time_fields_are_refreshed_on_open() {
    // The reported fixture: both cached in January 2020.
    let d = open_body(&format!(
        "<w:p>{}</w:p><w:p>{}</w:p>",
        field(r#"DATE \@ "yyyy-MM-dd""#, "2020-01-01", false),
        field(r#"TIME \@ "dddd, d MMMM yyyy""#, "Wednesday, 1 January 2020", false),
    ));
    let got = fields(&d);
    assert_eq!(got.len(), 2);
    // Compared against the same clock the refresh used, so the test does not age.
    assert_eq!(got[0].1, wordcraft_engine::cmd::insert::format_date("yyyy-MM-dd"), "DATE is today");
    assert_eq!(got[1].1, wordcraft_engine::cmd::insert::format_date("dddd, d MMMM yyyy"), "TIME is today");
    assert_ne!(got[0].1, "2020-01-01", "not the stored result");
    // The code itself is untouched, so a save writes the field back as it was.
    assert_eq!(got[0].0, r#"DATE \@ "yyyy-MM-dd""#);
}

#[test]
fn a_date_field_with_no_picture_uses_words_default() {
    let d = open_body(&format!("<w:p>{}</w:p><w:p>{}</w:p>", field("DATE", "2020-01-01", false), field("TIME", "00:00", false)));
    let got = fields(&d);
    assert_eq!(got[0].1, wordcraft_engine::cmd::insert::format_date("M/d/yyyy"));
    assert_eq!(got[1].1, wordcraft_engine::cmd::insert::format_date("h:mm am/pm"));
}

#[test]
fn a_locked_date_field_keeps_its_stored_result() {
    // `w:fldLock` is exactly the instruction not to refresh.
    let d = open_body(&format!("<w:p>{}</w:p>", field(r#"DATE \@ "yyyy-MM-dd""#, "2020-01-01", true)));
    assert_eq!(fields(&d)[0].1, "2020-01-01");
}

#[test]
fn other_date_fields_keep_their_stored_result() {
    // These record when something happened; they are not "now", and Word does not refresh them
    // on open either. Nor is any other field touched.
    let d = open_body(&format!(
        "<w:p>{}{}{}{}</w:p>",
        field(r#"CREATEDATE \@ "yyyy-MM-dd""#, "2019-05-05", false),
        field(r#"SAVEDATE \@ "yyyy-MM-dd""#, "2019-06-06", false),
        field(r#"PRINTDATE \@ "yyyy-MM-dd""#, "2019-07-07", false),
        field("AUTHOR", "Ada Lovelace", false),
    ));
    let results: Vec<String> = fields(&d).into_iter().map(|(_, r)| r).collect();
    assert_eq!(results, ["2019-05-05", "2019-06-06", "2019-07-07", "Ada Lovelace"]);
}

#[test]
fn a_date_field_in_a_header_is_refreshed_too() {
    // A letter template usually carries its date in the header, which is its own story.
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let drels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rH1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#;
    let header = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p>{}</w:p></w:hdr>"#,
        field(r#"DATE \@ "yyyy-MM-dd""#, "2020-01-01", false)
    );
    let doc = r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rH1"/></w:sectPr></w:body></w:document>"#;
    let d = wordcraft_engine::io::open_bytes(
        "d.docx",
        &zip(&[("_rels/.rels", rels), ("word/document.xml", doc), ("word/_rels/document.xml.rels", drels), ("word/header1.xml", &header)]),
    )
    .expect("opens");
    let today = wordcraft_engine::cmd::insert::format_date("yyyy-MM-dd");
    let found: Vec<String> = d
        .parts
        .values()
        .flat_map(|p| p.blocks.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.clone()))
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(found, [today], "the header's field is refreshed as well as the body's");
}

/// A minimal Word 97-2003 file whose only paragraph holds a `DATE` field cached as " 2001":
/// FIB, one compressed text piece, and the Clx that points at it.
fn binary_doc_with_stale_date() -> Vec<u8> {
    const TEXT_FC: usize = 0x600;
    let text = "\x13DATE \\@ \"yyyy\"\x14 2001\x15\r";
    let n = text.chars().count() as u32;
    let mut plc = Vec::new();
    for cp in [0u32, n] {
        plc.extend_from_slice(&cp.to_le_bytes());
    }
    plc.extend_from_slice(&[0, 0]);
    plc.extend_from_slice(&((2 * TEXT_FC as u32) | 0x4000_0000).to_le_bytes());
    plc.extend_from_slice(&0u16.to_le_bytes());
    let mut clx = vec![0x02u8];
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);
    let clx_at = 0x40usize;
    let mut table = vec![0u8; clx_at];
    table.extend_from_slice(&clx);

    let mut word = vec![0u8; TEXT_FC];
    word.extend(text.chars().map(|c| c as u8));
    let put16 = |v: &mut Vec<u8>, at: usize, x: u16| v[at..at + 2].copy_from_slice(&x.to_le_bytes());
    let put32 = |v: &mut Vec<u8>, at: usize, x: u32| v[at..at + 4].copy_from_slice(&x.to_le_bytes());
    put16(&mut word, 0x00, 0xA5EC);
    put16(&mut word, 0x02, 0x00C1);
    put16(&mut word, 0x0A, 0x1000);
    put16(&mut word, 0x0C, 0x00BF);
    put16(&mut word, 0x20, 0x000E);
    put16(&mut word, 0x3E, 0x0016);
    put32(&mut word, 0x40, 0x1000);
    put32(&mut word, 0x4C, n);
    put16(&mut word, 0x98, 0x005D);
    put32(&mut word, 0x9A + 33 * 8, clx_at as u32);
    put32(&mut word, 0x9A + 33 * 8 + 4, clx.len() as u32);

    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
    for (name, data) in [("WordDocument", &word), ("0Table", &table)] {
        let mut s = comp.create_stream(format!("/{name}")).unwrap();
        s.write_all(data).unwrap();
    }
    comp.into_inner().into_inner()
}

#[test]
fn binary_doc_opened_by_extension_refreshes_date_fields() {
    let bytes = binary_doc_with_stale_date();
    // The same bytes under a `.docx` name are read by content, which refreshes the field.
    let expected = fields(&wordcraft_engine::io::open_bytes("d.docx", &bytes).expect("opens by content"));
    assert_eq!(expected.len(), 1);
    assert!(!expected[0].1.contains("2001"), "reference should be refreshed: {expected:?}");
    for name in ["d.doc", "d.dot"] {
        let got = fields(&wordcraft_engine::io::open_bytes(name, &bytes).expect("opens"));
        assert_eq!(got, expected, "{name}: DATE field stale after opening");
    }
}

/// Regression from a synthetic Word document: a visible Brazilian long date is
/// stored in a TIME field and pt-BR is present only in docDefaults, not in field runs.
#[test]
fn a_word_time_field_inherits_pt_br_from_document_defaults() {
    const PICTURE: &str = "d' de 'MMMM' de 'yyyy";
    const CODE: &str = r#"TIME \@ "d' de 'MMMM' de 'yyyy""#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let styles = r#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:lang w:val="pt-BR"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Cidade, </w:t></w:r>{}</w:p><w:p><w:r><w:t>Cidade, </w:t></w:r>{}</w:p></w:body></w:document>"#,
        field(CODE, "22 de setembro de 2026", false),
        field(CODE, "22 de setembro de 2026", false)
    );
    let source = zip(&[("_rels/.rels", rels), ("word/document.xml", &doc), ("word/_rels/document.xml.rels", doc_rels), ("word/styles.xml", styles)]);
    let original = wordcraft_engine::io::open_bytes("pt-br-word.docx", &source).unwrap();
    let expected = wordcraft_engine::cmd::insert::format_date_in(PICTURE, Some("pt-BR"));
    let assert_correct = |document: &wordcraft_doc::Document| {
        let got = fields(document);
        assert_eq!(got.len(), 2, "both original fields survive");
        for (code, result) in got {
            assert_eq!(code, CODE, "field instruction remains editable");
            assert_eq!(result, expected, "quoted Portuguese words and month names");
            assert!(!result.contains("October") && !result.contains('\''));
            assert_eq!(result.matches(" de ").count(), 2);
        }
    };
    assert_correct(&original);
    let written = wordcraft_docx::write(&original).unwrap();
    let reopened = wordcraft_engine::io::open_bytes("reopened.docx", &written).unwrap();
    assert_correct(&reopened);
}
