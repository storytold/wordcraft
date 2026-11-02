//! Word's paragraph ids (`w14:paraId`, [MS-DOCX]) on body paragraphs: read, written back as
//! read, fresh ones for new paragraphs and copies, never two paragraphs with one id. Fixtures are
//! hand-written.

use std::collections::HashSet;
use std::io::{Read, Write};

use wordcraft_doc::Pos;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml""#;

fn docx(body: &str) -> Vec<u8> {
    package(body, &[])
}

/// A package of `body` and `parts` (file name under `word/`, relationship type, XML).
fn package(body: &str, parts: &[(&str, &str, String)]) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}</w:body></w:document>"#);
    let links: String =
        parts.iter().enumerate().map(|(i, (name, rel, _))| format!(r#"<Relationship Id="rId{}" Type="{rel}" Target="{name}"/>"#, i + 1)).collect();
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{links}</Relationships>"#
    );
    let mut files = vec![
        ("_rels/.rels".to_string(), root.to_string()),
        ("word/document.xml".to_string(), doc),
        ("word/_rels/document.xml.rels".to_string(), rels),
    ];
    files.extend(parts.iter().map(|(name, _, xml)| (format!("word/{name}"), xml.clone())));
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in &files {
        zw.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn part_xml(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    z.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
    xml
}

/// Two comments with Word's ids on their paragraphs, the second a reply to the first, on a body
/// paragraph with id `body_id` (or none).
fn commented(body_id: Option<&str>) -> Vec<u8> {
    let anchor = |id: u32| {
        format!(
            r#"<w:commentRangeStart w:id="{id}"/><w:r><w:t>Body</w:t></w:r><w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r>"#
        )
    };
    let open = body_id.map(|id| format!(r#"<w:p w14:paraId="{id}">"#)).unwrap_or_else(|| "<w:p>".into());
    let body = format!("{open}{}{}</w:p>", anchor(0), anchor(1));
    let comment = |id: u32, pid: &str, text: &str| {
        format!(r#"<w:comment w:id="{id}" w:author="A"><w:p w14:paraId="{pid}"><w:r><w:t>{text}</w:t></w:r></w:p></w:comment>"#)
    };
    let comments = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:comments {NS}>{}{}</w:comments>"#,
        comment(0, "2A000001", "First"),
        comment(1, "2A000002", "Reply")
    );
    let ex = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w15:commentsEx {NS}><w15:commentEx w15:paraId="2A000001" w15:done="0"/><w15:commentEx w15:paraId="2A000002" w15:paraIdParent="2A000001" w15:done="0"/></w15:commentsEx>"#
    );
    package(
        &body,
        &[
            ("comments.xml", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments", comments),
            ("commentsExtended.xml", "http://schemas.microsoft.com/office/2011/relationships/commentsExtended", ex),
        ],
    )
}

fn para(id: Option<&str>, text: &str) -> String {
    match id {
        Some(id) => format!(r#"<w:p w14:paraId="{id}" w14:textId="0A0B0C0D"><w:r><w:t>{text}</w:t></w:r></w:p>"#),
        None => format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"),
    }
}

/// Each body paragraph of the saved file: (its `w14:paraId`, its text), in order.
fn saved_paras(bytes: &[u8]) -> Vec<(Option<String>, String)> {
    let xml = part_xml(bytes, "word/document.xml");
    let mut out = Vec::new();
    let mut rest = xml.as_str();
    while let Some(at) = rest.find("<w:p") {
        rest = &rest[at + 4..];
        if !(rest.starts_with('>') || rest.starts_with(' ')) {
            continue;
        }
        let tag = &rest[..rest.find('>').unwrap()];
        let id = tag.split("w14:paraId=\"").nth(1).map(|s| s[..s.find('"').unwrap()].to_string());
        let end = rest.find("</w:p>").unwrap();
        let text: String = rest[..end].split("<w:t").skip(1).filter_map(|t| Some(t.split_once('>')?.1.split("</w:t>").next()?.to_string())).collect();
        out.push((id, text));
        rest = &rest[end..];
    }
    out
}

fn assert_unique(paras: &[(Option<String>, String)]) {
    let mut seen = HashSet::new();
    for (id, text) in paras {
        let id = id.as_deref().unwrap_or_else(|| panic!("no id on {text:?}: {paras:?}"));
        assert_eq!(id.len(), 8, "{id}");
        assert!(u32::from_str_radix(id, 16).unwrap() < 0x8000_0000, "{id}");
        assert!(seen.insert(id.to_string()), "{id} twice: {paras:?}");
    }
}

fn id_of<'a>(paras: &'a [(Option<String>, String)], text: &str) -> &'a str {
    paras.iter().find(|(_, t)| t == text).and_then(|(id, _)| id.as_deref()).unwrap_or_else(|| panic!("{text:?} not found: {paras:?}"))
}

#[test]
fn paragraph_ids_are_written_back_as_read() {
    let body = [para(Some("1A2B3C4D"), "Alpha"), para(Some("00000001"), "Beta"), para(Some("7FFFFFFF"), "Gamma")].concat();
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let ids: Vec<Option<u32>> = d.body.iter().filter_map(|b| b.as_para()).map(|p| p.para_id).collect();
    assert_eq!(ids, vec![Some(0x1A2B_3C4D), Some(1), Some(0x7FFF_FFFF)]);
    let saved = saved_paras(&wordcraft_docx::write(&d).unwrap());
    assert_eq!(id_of(&saved, "Alpha"), "1A2B3C4D");
    assert_eq!(id_of(&saved, "Beta"), "00000001");
    assert_eq!(id_of(&saved, "Gamma"), "7FFFFFFF");
    assert_unique(&saved);
    // And again, unchanged, from the saved file.
    let again = saved_paras(&wordcraft_docx::write(&wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap()).unwrap());
    assert_eq!(again, saved);
}

#[test]
fn missing_invalid_and_repeated_ids_get_fresh_ones() {
    let body = [
        para(Some("00000001"), "One"),
        para(None, "Missing"),
        para(Some("80000000"), "Too big"),
        para(Some("NOTHEX!!"), "Not hex"),
        para(Some("00000001"), "Repeated"),
        para(Some("00000002"), "Two"),
    ]
    .concat();
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let saved = saved_paras(&wordcraft_docx::write(&d).unwrap());
    assert_eq!(saved.len(), 6, "{saved:?}");
    assert_unique(&saved);
    assert_eq!(id_of(&saved, "One"), "00000001");
    assert_eq!(id_of(&saved, "Two"), "00000002", "a fresh id never takes one a later paragraph carries");
}

#[test]
fn a_document_without_ids_is_written_without_them() {
    let body = [para(None, "Alpha"), para(None, "Beta")].concat();
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let saved = saved_paras(&wordcraft_docx::write(&d).unwrap());
    assert!(saved.iter().all(|(id, _)| id.is_none()), "{saved:?}");
}

#[test]
fn new_split_and_pasted_paragraphs_get_fresh_ids_and_the_others_keep_theirs() {
    let body = [para(Some("0000000A"), "Alpha one"), para(Some("0000000B"), "Beta"), para(Some("0000000C"), "Gamma")].concat();
    let mut d = wordcraft_docx::read(&docx(&body)).unwrap();
    // Split "Alpha one" after "Alpha": the first half keeps the id.
    d.split_paragraph(&Pos::body(0, 5)).unwrap();
    // Enter at the start of "Gamma" (now block 3): the id stays with its text.
    d.split_paragraph(&Pos::body(3, 0)).unwrap();
    // Copy "Beta" whole (with its mark) and paste it in front of "Gamma" (now block 4).
    let copy = d.copy_range(&Pos::body(2, 0), &Pos::body(3, 0));
    d.insert_fragment(&Pos::body(4, 0), &copy).unwrap();
    // Paste a word inside "Beta": one paragraph, pasted inline.
    let word = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5));
    d.insert_fragment(&Pos::body(2, 0), &word).unwrap();
    let saved = saved_paras(&wordcraft_docx::write(&d).unwrap());
    let texts: Vec<&str> = saved.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts, vec!["Alpha", " one", "AlphaBeta", "", "Beta", "Gamma"]);
    assert_unique(&saved);
    assert_eq!(id_of(&saved, "Alpha"), "0000000A");
    assert_eq!(id_of(&saved, "AlphaBeta"), "0000000B");
    assert_eq!(id_of(&saved, "Gamma"), "0000000C");
}

#[test]
fn comment_paragraphs_never_share_a_body_paragraphs_id() {
    // Body ids where comments' fresh ones start.
    let body = [para(Some("10000001"), "Alpha"), para(Some("10000002"), "Beta")].concat();
    let mut d = wordcraft_docx::read(&docx(&body)).unwrap();
    let part = d
        .add_part(wordcraft_doc::PartKind::Comment, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Note", Default::default()))]);
    d.comments.insert(0, wordcraft_doc::Comment { part, ..Default::default() });
    let bytes = wordcraft_docx::write(&d).unwrap();
    let saved = saved_paras(&bytes);
    assert_eq!(id_of(&saved, "Alpha"), "10000001");
    assert_eq!(id_of(&saved, "Beta"), "10000002");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut ex = String::new();
    z.by_name("word/commentsExtended.xml").unwrap().read_to_string(&mut ex).unwrap();
    assert!(ex.contains(r#"w15:paraId="10000003""#), "{ex}");
}

#[test]
fn paragraphs_pasted_above_their_source_leave_it_its_ids() {
    let body = [para(Some("00000001"), "A"), para(Some("00000002"), "B"), para(Some("00000003"), "C"), para(Some("00000004"), "D")].concat();
    let mut d = wordcraft_docx::read(&docx(&body)).unwrap();
    // B and C whole (C is a whole block of the fragment, inserted as it is) pasted in front of A.
    let copy = d.copy_range(&Pos::body(1, 0), &Pos::body(3, 0));
    d.insert_fragment(&Pos::body(0, 0), &copy).unwrap();
    let saved = saved_paras(&wordcraft_docx::write(&d).unwrap());
    let texts: Vec<&str> = saved.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts, vec!["B", "C", "A", "B", "C", "D"]);
    assert_unique(&saved);
    let ids: Vec<&str> = saved[2..].iter().map(|(id, _)| id.as_deref().unwrap()).collect();
    assert_eq!(ids, vec!["00000001", "00000002", "00000003", "00000004"], "{saved:?}");
}

#[test]
fn comments_keep_their_ids_and_replies() {
    let d = wordcraft_docx::read(&commented(Some("00000010"))).unwrap();
    let reply = d.comments.values().find(|c| c.parent.is_some()).expect("the reply is read as one");
    assert!(reply.parent.is_some_and(|p| d.comments.contains_key(&p)));
    let bytes = wordcraft_docx::write(&d).unwrap();
    assert_eq!(id_of(&saved_paras(&bytes), "BodyBody"), "00000010");
    let comments = part_xml(&bytes, "word/comments.xml");
    assert!(comments.contains(r#"w14:paraId="2A000001""#) && comments.contains(r#"w14:paraId="2A000002""#), "{comments}");
    let ex = part_xml(&bytes, "word/commentsExtended.xml");
    assert!(ex.contains(r#"w15:paraId="2A000002" w15:paraIdParent="2A000001""#), "{ex}");
    let back = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(back.comments.values().filter(|c| c.parent.is_some()).count(), 1);
}

#[test]
fn ids_on_comments_alone_put_none_on_the_body() {
    let d = wordcraft_docx::read(&commented(None)).unwrap();
    let bytes = wordcraft_docx::write(&d).unwrap();
    assert!(saved_paras(&bytes).iter().all(|(id, _)| id.is_none()), "{:?}", saved_paras(&bytes));
    assert!(part_xml(&bytes, "word/comments.xml").contains(r#"w14:paraId="2A000001""#));
}
