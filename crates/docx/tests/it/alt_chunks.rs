//! Alternative format chunks (`w:altChunk`, ECMA-376 §17.17.2.1): content in another format that
//! is brought in where the chunk stands. All markup here is synthetic.

use std::io::Write;

use wordcraft_doc::{Block, Document, InlineObject, StoryRef};
use wordcraft_docx::{AltChunk, ReadOptions, read, read_with};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOCX_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";

fn p(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

/// A package whose body is `body`, with the chunk `afchunk.{ext}` (content type `ct`) as `rIdC`.
fn package(body: &str, chunk: Option<(&str, &str, &[u8])>) -> Vec<u8> {
    let doc = format!(r#"<w:document {NS}><w:body>{body}<w:sectPr/></w:body></w:document>"#);
    let mut files: Vec<(String, Vec<u8>)> = vec![("_rels/.rels".into(), ROOT_RELS.into()), ("word/document.xml".into(), doc.into_bytes())];
    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#,
    );
    if let Some((ext, ct, bytes)) = chunk {
        types.push_str(&format!(r#"<Default Extension="{ext}" ContentType="{ct}"/>"#));
        files.push((
            "word/_rels/document.xml.rels".into(),
            format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk" Target="/word/afchunk.{ext}"/></Relationships>"#).into_bytes(),
        ));
        files.push((format!("word/afchunk.{ext}"), bytes.to_vec()));
    }
    types.push_str(r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#);
    files.push(("[Content_Types].xml".into(), types.into_bytes()));
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(&bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

const CHUNK: &str = r#"<w:altChunk r:id="rIdC"/>"#;

fn body_lines(d: &Document) -> Vec<String> {
    d.plain_text(StoryRef::Body).split('\n').map(str::to_string).collect()
}

#[test]
fn a_chunk_comes_in_through_the_importer_where_it_stands() {
    let html = b"<p>Chunk one</p><p>Chunk two</p>";
    let bytes = package(&format!("{}{CHUNK}{}", p("Before"), p("After")), Some(("htm", "application/xhtml+xml", html)));
    let seen = std::cell::RefCell::new(Vec::new());
    let import = |c: &AltChunk| {
        seen.borrow_mut().push((c.content_type.to_string(), c.part.to_string(), c.bytes.to_vec()));
        Some(Document::from_text("Chunk one\nChunk two"))
    };
    let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
    assert_eq!(body_lines(&d), ["Before", "Chunk one", "Chunk two", "After"]);
    assert_eq!(*seen.borrow(), [("application/xhtml+xml".to_string(), "word/afchunk.htm".to_string(), html.to_vec())]);
    // Without an importer the chunk is left out.
    assert_eq!(body_lines(&read(&bytes).unwrap()), ["Before", "After"]);
}

#[test]
fn a_bookmark_opened_before_a_chunk_at_the_start_of_the_body_is_kept() {
    let body = format!(r#"<w:bookmarkStart w:id="0" w:name="Here"/>{CHUNK}<w:bookmarkEnd w:id="0"/>{}"#, p("After"));
    let bytes = package(&body, Some(("htm", "application/xhtml+xml", b"<p>Chunk</p>")));
    let import = |_: &AltChunk| Some(Document::from_text("Chunk"));
    let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
    assert_eq!(body_lines(&d), ["Chunk", "After"]);
    let marks = d.bookmarks();
    assert_eq!(marks.len(), 1);
    assert_eq!((marks[0].0.as_str(), marks[0].1.path.last(), marks[0].1.off), ("Here", 0, 0));
}

#[test]
fn a_bookmark_opened_between_a_table_and_a_chunk_starts_the_chunk() {
    let table = format!("<w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>", p("Cell"));
    let body = format!(r#"{table}<w:bookmarkStart w:id="0" w:name="Here"/>{CHUNK}<w:bookmarkEnd w:id="0"/>{}"#, p("After"));
    let bytes = package(&body, Some(("htm", "application/xhtml+xml", b"<p>Chunk</p>")));
    let import = |_: &AltChunk| Some(Document::from_text("Chunk"));
    let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
    let marks = d.bookmarks();
    assert_eq!(marks.len(), 1);
    assert_eq!((marks[0].1.path.clone(), marks[0].1.off), (wordcraft_doc::Path::top(1), 0), "not in the table's cell");
}

#[test]
fn permission_ranges_of_chunks_and_their_host_stay_apart() {
    let inner = package(r#"<w:p><w:permStart w:id="0" w:ed="them"/><w:r><w:t>Inner</w:t></w:r><w:permEnd w:id="0"/></w:p>"#, None);
    // The host's range 0 is open across two chunks whose own ranges are 0 too.
    let body = format!(
        r#"<w:p><w:permStart w:id="0" w:edGrp="everyone"/><w:r><w:t>Host</w:t></w:r></w:p>{CHUNK}{CHUNK}<w:p><w:r><w:t>After</w:t></w:r><w:permEnd w:id="0"/></w:p>"#
    );
    let d = read(&package(&body, Some(("docx", DOCX_MAIN, &inner)))).unwrap();
    assert_eq!(body_lines(&d), ["Host", "Inner", "Inner", "After"]);
    let check = |d: &Document| {
        let ranges = d.perm_ranges();
        let mut ids: Vec<u32> = ranges.iter().map(|r| r.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3, "{ranges:?}");
        let host = ranges.iter().find(|r| r.group == "everyone").unwrap();
        assert_eq!((host.start.path.last(), host.end.path.last()), (0, 3));
        assert!(ranges.iter().filter(|r| r.editor == "them").all(|r| r.start.path == r.end.path));
    };
    check(&d);
    check(&read(&wordcraft_docx::write(&d).unwrap()).unwrap());
}

/// A package whose body is `body`, with story parts (relationship type, part name, XML) and the
/// Word document `chunk` as `rIdC` from the main part and from each story part.
fn package_with_stories(body: &str, stories: &[(&str, &str, String)], chunk: Option<&[u8]>) -> Vec<u8> {
    let rel = |id: &str, ty: &str, target: &str| {
        format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"/>"#)
    };
    let rels =
        |inner: String| format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{inner}</Relationships>"#);
    let chunk_rel = if chunk.is_some() { rel("rIdC", "aFChunk", "/word/afchunk.docx") } else { String::new() };
    let doc = format!(r#"<w:document {NS}><w:body>{body}<w:sectPr/></w:body></w:document>"#);
    let mut main_rels = chunk_rel.clone();
    let mut files: Vec<(String, Vec<u8>)> = vec![("_rels/.rels".into(), ROOT_RELS.into()), ("word/document.xml".into(), doc.into_bytes())];
    for (i, (ty, name, xml)) in stories.iter().enumerate() {
        main_rels.push_str(&rel(&format!("rIdS{i}"), ty, name));
        files.push((format!("word/{name}"), xml.clone().into_bytes()));
        files.push((format!("word/_rels/{name}.rels"), rels(chunk_rel.clone()).into_bytes()));
    }
    files.push(("word/_rels/document.xml.rels".into(), rels(main_rels).into_bytes()));
    if let Some(c) = chunk {
        files.push(("word/afchunk.docx".into(), c.to_vec()));
    }
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(&bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A paragraph `text` that comment `id` (in the file) is anchored around.
fn commented(id: u32, text: &str) -> String {
    format!(
        r#"<w:p><w:commentRangeStart w:id="{id}"/><w:r><w:t>{text}</w:t></w:r><w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r></w:p>"#
    )
}

/// A comments part: (file id, author, content).
fn comments_part(list: &[(u32, &str, String)]) -> (&'static str, &'static str, String) {
    let body: String = list.iter().map(|(id, who, content)| format!(r#"<w:comment w:id="{id}" w:author="{who}">{content}</w:comment>"#)).collect();
    ("comments", "comments.xml", format!("<w:comments {NS}>{body}</w:comments>"))
}

/// The authors of the comments anchored in the stories whose text contains `text`, and whether
/// every comment has text of its own and a distinct id.
fn anchored(d: &Document, text: &str) -> Vec<String> {
    let mut stories: Vec<&[std::sync::Arc<Block>]> = vec![&d.body];
    stories.extend(d.parts.values().map(|p| p.blocks.as_slice()));
    let mut out = Vec::new();
    for blocks in stories {
        for b in blocks {
            let Block::Para(p) = &**b else { continue };
            if !p.text.contains(text) {
                continue;
            }
            for o in &p.objects {
                if let InlineObject::CommentStart { id } = o {
                    out.push(d.comments.get(id).map_or("?".to_string(), |c| c.author.clone()));
                }
            }
        }
    }
    out
}

/// A Word document with one comment by "Them" around "Inner".
fn commented_chunk() -> Vec<u8> {
    package_with_stories(&commented(0, "Inner"), &[comments_part(&[(0, "Them", p("Theirs"))])], None)
}

#[test]
fn a_chunk_in_a_footnote_keeps_its_comments_beside_the_documents() {
    let notes = format!(r#"<w:footnotes {NS}><w:footnote w:id="1"><w:altChunk r:id="rIdC"/></w:footnote></w:footnotes>"#);
    let body = format!(r#"{}<w:p><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#, commented(0, "Host"));
    let stories = [("footnotes", "footnotes.xml", notes), comments_part(&[(0, "Us", p("Ours"))])];
    let d = read(&package_with_stories(&body, &stories, Some(&commented_chunk()))).unwrap();
    assert_eq!(d.comments.len(), 2, "{:?}", d.comments);
    assert_eq!(anchored(&d, "Host"), ["Us"]);
    assert_eq!(anchored(&d, "Inner"), ["Them"]);
}

#[test]
fn a_chunk_in_a_comment_keeps_its_comments_beside_the_documents() {
    let inside = format!(r#"{}<w:altChunk r:id="rIdC"/>"#, p("Ours"));
    let body = format!("{}{}", commented(0, "Host"), commented(1, "Second"));
    let stories = [comments_part(&[(0, "Us", inside), (1, "Us too", p("Ours too"))])];
    let d = read(&package_with_stories(&body, &stories, Some(&commented_chunk()))).unwrap();
    assert_eq!(d.comments.len(), 3, "{:?}", d.comments);
    assert_eq!(anchored(&d, "Host"), ["Us"]);
    assert_eq!(anchored(&d, "Second"), ["Us too"]);
    assert_eq!(anchored(&d, "Inner"), ["Them"]);
}

#[test]
fn a_wordprocessing_chunk_is_read_without_an_importer_and_flows_in_its_section() {
    let inner = package(&format!("{}{}", p("Inner one"), p("Inner two")), None);
    let bytes = package(&format!("{}{CHUNK}{}", p("Before"), p("After")), Some(("docx", DOCX_MAIN, &inner)));
    let d = read(&bytes).unwrap();
    assert_eq!(body_lines(&d), ["Before", "Inner one", "Inner two", "After"]);
    // The inner document's own section doesn't come along.
    assert_eq!(d.sections().len(), 1);
}

#[test]
fn nested_wordprocessing_chunks_stop_at_a_depth() {
    let mut bytes = package(&p("Deepest"), None);
    for level in 0..8 {
        bytes = package(&format!("{}{CHUNK}", p(&format!("Level {level}"))), Some(("docx", DOCX_MAIN, &bytes)));
    }
    let lines = body_lines(&read(&bytes).unwrap());
    assert_eq!(lines.first().map(String::as_str), Some("Level 7"));
    assert!(!lines.iter().any(|l| l == "Deepest"), "{lines:?}");
    assert!(lines.len() >= 4, "{lines:?}");
}

#[test]
fn a_chunks_pictures_come_with_it() {
    let bytes = package(&format!("{}{CHUNK}", p("Before")), Some(("htm", "text/html", b"<p>x</p>")));
    let import = |_: &AltChunk| {
        let mut d = Document::new();
        let key = d.add_media(vec![1, 2, 3], "png");
        let mut para = wordcraft_doc::Paragraph::new();
        let image = InlineObject::Image { media: key, w: 10.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
        para.insert_object(0, image, &Default::default()).unwrap();
        d.body = vec![std::sync::Arc::new(Block::Para(para))];
        Some(d)
    };
    let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
    let keys: Vec<String> = d
        .body
        .iter()
        .filter_map(|b| match &**b {
            Block::Para(p) => p.objects.iter().find_map(|o| match o {
                InlineObject::Image { media, .. } => Some(media.clone()),
                _ => None,
            }),
            _ => None,
        })
        .collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(d.media.get(&keys[0]).map(|m| m.as_slice()), Some(&[1u8, 2, 3][..]));
}

#[test]
fn a_file_brings_in_a_bounded_number_of_chunks() {
    let bytes = package(&CHUNK.repeat(300), Some(("htm", "text/html", b"<p>x</p>")));
    let calls = std::cell::Cell::new(0);
    let import = |_: &AltChunk| {
        calls.set(calls.get() + 1);
        Some(Document::from_text("x"))
    };
    let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
    assert_eq!(calls.get(), 256);
    assert_eq!(body_lines(&d).len(), 256);
}

#[test]
fn hostile_chunks_never_panic() {
    for (ext, ct, bytes) in [("docx", DOCX_MAIN, &b"PK\x03\x04 not a zip"[..]), ("htm", "text/html", b""), ("bin", "", b"\x00\xff")] {
        let bytes = package(&format!("{CHUNK}{CHUNK}{}", p("x")), Some((ext, ct, bytes)));
        let import = |_: &AltChunk| None;
        let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
        assert_eq!(body_lines(&d), ["x"]);
    }
    // A chunk naming a missing part or another relationship type is left out.
    let bytes = package(&format!(r#"<w:altChunk r:id="nope"/><w:altChunk/>{}"#, p("y")), None);
    assert_eq!(body_lines(&read(&bytes).unwrap()), ["y"]);
}
