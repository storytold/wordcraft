//! #5676: Remove Header / Remove Footer clear every section and the parts they referenced.
//! Before, `insert.removeFooter` cleared only the last section's footers and the cleared parts
//! stayed in `Document::parts`, so a footer in an earlier section was still there and the
//! Document Inspector kept counting them after the dialog said "Removed.".

use std::sync::Arc;

use serde_json::json;
use wordcraft_doc::section::SectionProps;
use wordcraft_doc::{Block, Document, Paragraph, PartKind, StoryRef, para_block};
use wordcraft_engine::Session;

/// A document with two sections — a section break on the first paragraph's mark — and a footer
/// in each: the first one's named by its break, the last one's in `last_section`. Returns the
/// session and the two footer part ids.
fn two_sections_with_footers() -> (Session, u32, u32) {
    let mut doc = Document::from_text("First section.\nLast section.");
    if let Some(Block::Para(p)) = doc.body.first_mut().map(Arc::make_mut) {
        p.section = Some(Box::new(SectionProps::default()));
    }
    let first = doc.add_part(PartKind::Footer, vec![para_block(Paragraph::new().styled("Footer"))]);
    doc.section_mut(0).footers.default = Some(first);
    let last = doc.add_part(PartKind::Footer, vec![para_block(Paragraph::new().styled("Footer"))]);
    doc.last_section.footers.default = Some(last);
    (Session::new(doc), first, last)
}

#[test]
fn remove_footer_clears_every_section_and_its_part() {
    let (mut s, first, _last) = two_sections_with_footers();
    s.run("insert.removeFooter", &json!({})).unwrap();
    // The first section's break no longer names a footer, and the part is gone from the
    // document: before, only the last section's footers were cleared and the part stayed.
    let Some(Block::Para(p)) = s.doc.body.first().map(|b| &**b) else { panic!("first block") };
    assert!(p.section.as_ref().is_some_and(|sec| sec.footers.default.is_none()), "the first section's footer is cleared");
    assert!(s.doc.parts.get(&first).is_none(), "the footer part the first section named is removed");
    assert!(s.doc.last_section.footers.default.is_none());
    // The inspector counts none, so its row drops its Remove All button.
    assert_eq!(s.run("file.inspect", &json!({})).unwrap()["found"]["headers"], json!(0));
}

#[test]
fn remove_header_clears_every_section_and_its_part() {
    let mut doc = Document::from_text("First section.\nLast section.");
    if let Some(Block::Para(p)) = doc.body.first_mut().map(Arc::make_mut) {
        p.section = Some(Box::new(SectionProps::default()));
    }
    let first = doc.add_part(PartKind::Header, vec![para_block(Paragraph::new().styled("Header"))]);
    doc.section_mut(0).headers.default = Some(first);
    let (mut s, _) = (Session::new(doc), ());
    s.run("insert.removeHeader", &json!({})).unwrap();
    let Some(Block::Para(p)) = s.doc.body.first().map(|b| &**b) else { panic!("first block") };
    assert!(p.section.as_ref().is_some_and(|sec| sec.headers.default.is_none()), "the first section's header is cleared");
    assert!(s.doc.parts.get(&first).is_none(), "the header part the first section named is removed");
    assert_eq!(s.run("file.inspect", &json!({})).unwrap()["found"]["headers"], json!(0));
}

#[test]
fn the_caret_in_a_removed_footer_leaves_a_valid_selection() {
    // Remove Footer while editing the footer it removes: the part goes, so the selection is
    // clamped back into the document instead of naming a story that is gone.
    let (mut s, first, _last) = two_sections_with_footers();
    s.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos { story: StoryRef::Part(first), path: wordcraft_doc::Path(vec![0, 0]), off: 0 });
    s.run("insert.removeFooter", &json!({})).unwrap();
    assert_eq!(s.sel.focus.story, StoryRef::Body, "the caret in the removed footer is clamped into the body");
}

#[test]
fn inspect_document_remove_headers_clears_every_section() {
    // Remove All in the Document Inspector runs removeHeader and removeFooter, so a header or
    // footer in an earlier section is gone too.
    let (mut s, first, _last) = two_sections_with_footers();
    let _ = s.run("file.inspect", &json!({"remove": ["headers"]})).unwrap();
    // The report `found` counts what was found; the count the inspector reads afresh after the
    // removal is none.
    assert_eq!(s.run("file.inspect", &json!({})).unwrap()["found"]["headers"], json!(0));
    assert!(s.doc.parts.get(&first).is_none(), "the footer part the first section named is removed");
    assert!(s.doc.parts.values().all(|p| !matches!(p.kind, PartKind::Header | PartKind::Footer)));
}
