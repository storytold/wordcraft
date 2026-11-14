//! The blank page inserted so a section starts on the side its break asks for (odd-page and
//! even-page breaks, or a next-page break restarting at the wrong parity with different odd and
//! even headers): Word draws no header or footer on it (Microsoft 365 Word's PDF output).

#[allow(unused_imports)]
use crate::*;
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{PartKind, Path, StoryRef, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn part(d: &mut Document, kind: PartKind, text: &str) -> u32 {
    d.add_part(kind, vec![para_block(Paragraph::with_text(text, Default::default()))])
}

/// "One" ends the first section on page 1; the second section ("Two") starts as `start`. Both
/// sections show the first section's default header and footer (the second links to them); with
/// `even_odd` the first section also names an even header and footer.
fn doc(start: SectionStart, restart: Option<u32>, even_odd: bool) -> Document {
    let mut d = Document::from_text("One\nTwo");
    let head = part(&mut d, PartKind::Header, "Head");
    let foot = part(&mut d, PartKind::Footer, "Foot");
    let mut first = SectionProps::default();
    first.headers.default = Some(head);
    first.footers.default = Some(foot);
    if even_odd {
        first.headers.even = Some(part(&mut d, PartKind::Header, "Even head"));
        first.footers.even = Some(part(&mut d, PartKind::Footer, "Even foot"));
    }
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(first));
    d.last_section.start = start;
    d.last_section.page_num_start = restart;
    d.settings.even_odd_headers = even_odd;
    d
}

/// Per page: (number, has header items, has footer items, has body items, blank flag).
fn shape(l: &DocLayout) -> Vec<(u32, bool, bool, bool, bool)> {
    l.pages.iter().map(|p| (p.number, !p.header.is_empty(), !p.footer.is_empty(), !p.items.is_empty(), p.blank)).collect()
}

#[test]
fn an_odd_page_break_leaves_its_blank_page_without_header_or_footer() {
    let l = lay(&doc(SectionStart::OddPage, None, false));
    assert_eq!(shape(&l), [(1, true, true, true, false), (2, false, false, false, true), (3, true, true, true, false)]);
    assert_eq!((l.pages[1].header_story, l.pages[1].footer_story), (None, None));
}

#[test]
fn the_blank_page_stays_empty_with_different_odd_and_even_headers() {
    // Page 2 is even and the first section names an even header: Word still leaves it empty.
    let l = lay(&doc(SectionStart::OddPage, None, true));
    assert_eq!(shape(&l), [(1, true, true, true, false), (2, false, false, false, true), (3, true, true, true, false)]);
}

#[test]
fn a_next_page_restart_on_the_wrong_side_leaves_its_blank_page_empty() {
    // Restarting at 1 with different odd and even headers starts on an odd sheet: blank page 2.
    let l = lay(&doc(SectionStart::NextPage, Some(1), true));
    assert_eq!(shape(&l), [(1, true, true, true, false), (2, false, false, false, true), (1, true, true, true, false)]);
}

#[test]
fn a_section_already_on_its_side_inserts_no_blank_page() {
    // Guard: an even-page break after page 1 needs no filler; page 2 keeps its header and footer.
    let l = lay(&doc(SectionStart::EvenPage, None, false));
    assert_eq!(shape(&l), [(1, true, true, true, false), (2, true, true, true, false)]);
}
