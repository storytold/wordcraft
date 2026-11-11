//! Tracked table-row insertions and deletions (`w:trPr/w:ins` and `w:trPr/w:del`, ECMA-376
//! Part 1 §17.13.5.17 and §17.13.5.13). Fixtures are hand-written from the spec.

use std::io::{Read, Write};
use std::sync::Arc;

use wordcraft_doc::table::{Cell, Table};
use wordcraft_doc::{Block, Document, Revision, RevisionKind};

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

fn table(d: &Document) -> &Table {
    d.body.iter().find_map(|b| b.as_table()).expect("a table")
}

/// A three-row table: row 1 kept, row 2 a tracked insertion, row 3 a tracked deletion.
fn marked_doc() -> Document {
    let mut d = Document::from_text("Intro");
    d.revisions.push(Revision::new(RevisionKind::Insert, "Ann", "2026-01-02T03:04:05Z"));
    d.revisions.push(Revision::new(RevisionKind::Delete, "Bob", "2026-02-03T04:05:06Z"));
    let mut t = Table::new(3, 1, 200.0);
    for (i, r) in t.rows.iter_mut().enumerate() {
        r.cells[0] = Cell::with_text(&format!("row {i}"));
    }
    t.rows[1].props.ins = Some(0);
    t.rows[2].props.del = Some(1);
    t.rows[2].props.cant_split = true;
    d.body.push(Arc::new(Block::Table(t)));
    d.body.push(Arc::new(Block::Para(wordcraft_doc::Paragraph::default())));
    d
}

/// (a) Saving writes the row marks inside `w:trPr`, after the row's own properties, with id,
/// author and date from the revision table.
#[test]
fn row_insertions_and_deletions_are_written_in_trpr() {
    let xml = document_xml(&wordcraft_docx::write(&marked_doc()).unwrap());
    assert!(regex_like(&xml, r#"<w:trPr><w:ins w:id=""#, r#"" w:author="Ann" w:date="2026-01-02T03:04:05Z"/></w:trPr>"#), "inserted row mark: {xml}");
    assert!(
        regex_like(&xml, r#"<w:trPr><w:cantSplit/><w:del w:id=""#, r#"" w:author="Bob" w:date="2026-02-03T04:05:06Z"/></w:trPr>"#),
        "deleted row mark after the row's own properties: {xml}"
    );
}

/// `prefix` + some digits + `suffix` occurs in `s`.
fn regex_like(s: &str, prefix: &str, suffix: &str) -> bool {
    s.match_indices(prefix).any(|(i, _)| {
        let rest = &s[i + prefix.len()..];
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        digits > 0 && rest[digits..].starts_with(suffix)
    })
}

/// (b) Reading `w:trPr/w:ins` and `w:trPr/w:del` marks the rows, with revisions registered.
#[test]
fn row_insertions_and_deletions_are_read() {
    let row = |pr: &str, text: &str| {
        format!(
            r#"<w:tr><w:trPr>{pr}</w:trPr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc></w:tr>"#
        )
    };
    let body = format!(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>{}{}{}</w:tbl><w:p/>"#,
        row("", "kept"),
        row(r#"<w:ins w:id="7" w:author="Ann" w:date="2026-01-02T03:04:05Z"/>"#, "new"),
        row(r#"<w:cantSplit/><w:del w:id="8" w:author="Bob" w:date="2026-02-03T04:05:06Z"/>"#, "gone"),
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let t = table(&d);
    assert_eq!((t.rows[0].props.ins, t.rows[0].props.del), (None, None));
    let ins = t.rows[1].props.ins.expect("the inserted row is marked");
    let del = t.rows[2].props.del.expect("the deleted row is marked");
    assert!(t.rows[2].props.cant_split, "the row's own properties are still read");
    let rev = |i: u32| d.revisions.get(i as usize).cloned().expect("a registered revision");
    assert_eq!((rev(ins).kind, rev(ins).author.as_str(), rev(ins).date.as_str()), (RevisionKind::Insert, "Ann", "2026-01-02T03:04:05Z"));
    assert_eq!((rev(del).kind, rev(del).author.as_str(), rev(del).date.as_str()), (RevisionKind::Delete, "Bob", "2026-02-03T04:05:06Z"));
}

/// Save → open keeps the row marks, their authors and dates.
#[test]
fn row_marks_round_trip() {
    let back = wordcraft_docx::read(&wordcraft_docx::write(&marked_doc()).unwrap()).unwrap();
    let t = table(&back);
    let author = |i: Option<u32>| i.and_then(|i| back.revisions.get(i as usize)).map(|r| (r.kind, r.author.clone()));
    assert_eq!(author(t.rows[0].props.ins.or(t.rows[0].props.del)), None);
    assert_eq!(author(t.rows[1].props.ins), Some((RevisionKind::Insert, "Ann".into())));
    assert_eq!(author(t.rows[2].props.del), Some((RevisionKind::Delete, "Bob".into())));
    assert!(t.rows[2].props.cant_split);
    // And a second save writes the same marks again.
    let xml = document_xml(&wordcraft_docx::write(&back).unwrap());
    assert_eq!(xml.matches("<w:trPr><w:ins ").count(), 1, "{xml}");
    assert_eq!(xml.matches("<w:cantSplit/><w:del ").count(), 1, "{xml}");
}

/// (d) Row marks with missing or junk attributes, or nested in odd places, don't crash and
/// still mark the row.
#[test]
fn hostile_row_marks_never_panic() {
    let cell = r#"<w:tc><w:p><w:r><w:t>x</w:t></w:r></w:p></w:tc>"#;
    for pr in [
        "<w:ins/>",
        "<w:del/>",
        r#"<w:ins w:id="-99999999999999999999" w:author="" w:date="not a date"/>"#,
        r#"<w:del w:id="x"><w:del w:id="y"/></w:del>"#,
        "<w:ins/><w:del/><w:ins/>",
        "<w:trPrChange><w:trPr><w:ins/></w:trPr></w:trPrChange>",
    ] {
        let body = format!(r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:trPr>{pr}</w:trPr>{cell}</w:tr></w:tbl><w:p/>"#);
        let d = wordcraft_docx::read(&docx(&body)).unwrap();
        let _ = wordcraft_docx::write(&d).unwrap();
    }
    let body = format!(r#"<w:tbl><w:tr><w:trPr><w:ins/></w:trPr>{cell}</w:tr></w:tbl><w:p/>"#);
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    assert!(table(&d).rows[0].props.ins.is_some(), "an author-less mark still marks the row");
}
