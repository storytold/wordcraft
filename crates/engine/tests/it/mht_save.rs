//! Gap 26: Save As and Export write a Single File Web Page (`.mht`, `.mhtml`), and WordCraft
//! opens it again with its text, title, picture and, with Print Markup, the comment list.

use serde_json::json;
use wordcraft_doc::{CharProps, Comment, Document, InlineObject, Pos, StoryRef};
use wordcraft_engine::{Session, io};

use crate::safe_save::Scratch;

/// A 1×1 PNG.
const DOT: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xFC, 0xCF, 0xC0, 0x50,
    0x0F, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xA9, 0x8C, 0x21, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Two paragraphs (one with letters outside ASCII), a title and a picture after them.
fn report() -> Document {
    let mut doc = Document::from_text("Quarterly report\nRezultati tromjesečja: 12 % više");
    doc.core.title = "Izvještaj".into();
    let key = doc.add_media(DOT.to_vec(), "png");
    let at = doc.body.len();
    doc.body.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::new()));
    let image = InlineObject::Image { media: key, w: 30.0, h: 30.0, alt: "Logo".into(), float: Default::default(), crop: [0.0; 4], ole: None };
    doc.insert_object(&Pos::body(at, 0), image, &CharProps::default()).unwrap();
    doc
}

/// The body's paragraphs with text, trimmed.
fn lines(doc: &Document) -> Vec<String> {
    doc.plain_text(StoryRef::Body).lines().map(str::trim).filter(|l| !l.is_empty() && *l != "\u{FFFC}").map(str::to_string).collect()
}

#[test]
fn a_document_saved_as_a_single_file_web_page_opens_again() {
    assert!(io::SAVE_EXTS.contains(&"mht"), "Save As offers the format");
    let doc = report();
    for name in ["report.mht", "report.mhtml", "REPORT.MHT"] {
        let bytes = io::save_bytes(name, &doc).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(bytes.starts_with(b"MIME-Version: 1.0\r\n"), "{name}");
        assert!(wordcraft_formats::mht::is_mht(&bytes), "{name}");
        let back = io::open_bytes(name, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(lines(&back), lines(&doc), "{name}");
        assert_eq!(back.core.title, "Izvještaj", "{name}");
        assert_eq!(back.media.values().map(|m| m.as_slice()).collect::<Vec<_>>(), [DOT], "{name}: the picture, byte for byte");
    }
}

#[test]
fn save_as_writes_the_file_and_print_markup_adds_the_comment_list() {
    let dir = Scratch::new();
    let mut doc = report();
    doc.comments.insert(1, Comment { author: "Reviewer Ana".into(), ..Default::default() });
    let mut s = Session::new(doc);
    let own = dir.path("report.docx");
    s.path = Some(own.clone());
    let plain = dir.path("copy.mht");
    s.run("file.saveAs", &json!({"path": plain, "markup": false})).unwrap();
    let back = io::open_path(&plain).unwrap();
    assert_eq!(lines(&back), lines(&s.doc));
    assert!(!back.plain_text(StoryRef::Body).contains("Reviewer Ana"));
    // Like a web page, the copy doesn't become the document's file.
    assert_eq!(s.path.as_deref(), Some(own.as_path()));
    let marked = dir.path("marked.mhtml");
    s.run("file.saveAs", &json!({"path": marked, "markup": true})).unwrap();
    let text = io::open_path(&marked).unwrap().plain_text(StoryRef::Body);
    assert!(text.contains("Reviewer Ana"), "{text}");
}
