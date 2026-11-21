//! PR #6425 review: the blank page inserted before an odd-page section draws no header or footer,
//! but still uses the section's parts, so editing its header or footer edits the existing part
//! instead of putting a new empty one in the first section's slot.

use crate::Session;
use serde_json::json;
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{Document, PartKind, Path, StoryRef, para_block};

/// "One" ends the first section (default header "Head", footer "Foot") on page 1; "Two" starts
/// an odd-page section, so page index 1 is the blank page. Returns (doc, head, foot).
fn doc() -> (Document, u32, u32) {
    let mut d = Document::from_text("One\nTwo");
    let head = d.add_part(PartKind::Header, vec![para_block(wordcraft_doc::Paragraph::with_text("Head", Default::default()))]);
    let foot = d.add_part(PartKind::Footer, vec![para_block(wordcraft_doc::Paragraph::with_text("Foot", Default::default()))]);
    let mut first = SectionProps::default();
    first.headers.default = Some(head);
    first.footers.default = Some(foot);
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(first));
    d.last_section.start = SectionStart::OddPage;
    (d, head, foot)
}

fn first_section(s: &Session) -> SectionProps {
    s.doc.para(StoryRef::Body, &Path::top(0)).and_then(|p| p.section.as_deref().cloned()).expect("the first section's properties")
}

fn part_text(s: &Session, id: u32) -> String {
    let part = s.doc.parts.get(&id).expect("the part is still there");
    part.blocks.iter().filter_map(|b| b.as_para().map(|p| p.text.clone())).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_blank_page_uses_the_sections_parts_but_draws_nothing() {
    let (d, head, foot) = doc();
    let mut s = Session::new(d);
    let l = s.layout();
    assert_eq!(l.pages.len(), 3);
    assert!(l.pages[1].blank, "page 2 is the blank page");
    assert!(l.pages[1].header.is_empty() && l.pages[1].footer.is_empty(), "the blank page draws no header or footer");
    assert_eq!((l.pages[1].header_story, l.pages[1].footer_story), (Some(head), Some(foot)));
}

#[test]
fn editing_the_blank_pages_header_or_footer_keeps_the_sections_part() {
    for header in [true, false] {
        let (d, head, foot) = doc();
        let (cmd, id, text) = if header { ("insert.editHeader", head, "Head") } else { ("insert.editFooter", foot, "Foot") };
        let mut s = Session::new(d);
        let kinds = |s: &Session| s.doc.parts.values().filter(|p| matches!(p.kind, PartKind::Header | PartKind::Footer)).count();
        let parts_before = kinds(&s);
        let r = s.run(cmd, &json!({"page": 1})).unwrap();
        assert_eq!(r["story"].as_u64(), Some(u64::from(id)), "{cmd} on the blank page edits the existing part");
        assert_eq!(kinds(&s), parts_before, "{cmd}: no new part");
        let sect = first_section(&s);
        assert_eq!((sect.headers.default, sect.footers.default), (Some(head), Some(foot)), "{cmd}: the slots are unchanged");
        s.run("insert.closeHeader", &json!({})).unwrap();
        let sect = first_section(&s);
        assert_eq!((sect.headers.default, sect.footers.default), (Some(head), Some(foot)), "{cmd} then close: the slots are unchanged");
        assert_eq!(part_text(&s, id), text);
        let l = s.layout();
        for page in [0, 2] {
            let story = if header { l.pages[page].header_story } else { l.pages[page].footer_story };
            let drawn = if header { &l.pages[page].header } else { &l.pages[page].footer };
            assert_eq!(story, Some(id), "{cmd}: page {page} still uses the part");
            assert!(!drawn.is_empty(), "{cmd}: page {page} still draws it");
        }
    }
}
