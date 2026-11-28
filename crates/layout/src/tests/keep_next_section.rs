//! Keep with next on the paragraph that closes a section (#6834). When the next section starts
//! on a new page, the paragraph after it is not on this page, so Keep with next has nothing to
//! keep it with: it stays at the bottom of its page. Moving it over for want of the next
//! section's first line left a page holding only that section's paragraph. Only a next-page
//! break was measured in Word; odd-page and even-page breaks follow by analogy (not measured).
//! Fixed page size and exact line heights, so no font metrics are assumed. Invented text only.

use crate::*;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{ParaProps, Paragraph};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// A 400 x 300 pt page with 50 pt margins: 200 pt of body, ten 20 pt lines.
fn small_page(start: SectionStart) -> SectionProps {
    SectionProps {
        start,
        page_w: 400.0,
        page_h: 300.0,
        margin_top: 50.0,
        margin_bottom: 50.0,
        margin_left: 50.0,
        margin_right: 50.0,
        ..Default::default()
    }
}

fn exact(text: &str, height: f32, keep: bool) -> Paragraph {
    let mut p = Paragraph::with_text(text, Default::default());
    p.props = ParaProps {
        line_spacing: Some(LineSpacing::Exactly(height)),
        space_before: Some(0.0),
        space_after: Some(0.0),
        keep_next: keep.then_some(true),
        ..Default::default()
    };
    p
}

/// `filler` exact 20 pt paragraphs, then the closing paragraph (15 pt, block index `filler`,
/// kept with the next when `keep`), then one 20 pt paragraph. When `closes`, the closing
/// paragraph ends section one and the second section starts as `start`; otherwise the second
/// paragraph is in the same section.
fn build(filler: usize, keep: bool, start: SectionStart, closes: bool, empty: bool) -> Document {
    let mut d = Document::new();
    d.body.clear();
    for i in 0..filler {
        d.body.push(wordcraft_doc::para_block(exact(&format!("Filler line {i}"), 20.0, false)));
    }
    let mut last = exact(if empty { "" } else { "Closing paragraph of the first part" }, 15.0, keep);
    if closes {
        last.section = Some(Box::new(small_page(SectionStart::NextPage)));
    }
    d.body.push(wordcraft_doc::para_block(last));
    d.body.push(wordcraft_doc::para_block(exact("First paragraph of the second part", 20.0, false)));
    d.last_section = small_page(if closes { start } else { SectionStart::NextPage });
    d
}

/// The page (0-based) that holds the body paragraph `block`.
fn page_of(l: &DocLayout, block: u32) -> Option<usize> {
    l.pages.iter().position(|p| p.items.iter().any(|it| matches!(it, Placed::Lines { story: StoryRef::Body, path, .. } if path.0 == [block])))
}

const NEXT_BREAKS: [SectionStart; 3] = [SectionStart::NextPage, SectionStart::OddPage, SectionStart::EvenPage];

/// The issue's case: nine 20 pt lines, an empty 15 pt kept closing paragraph that fits at the
/// bottom (195 of 200 pt) while the next section's first line (20 pt) would not, then one
/// paragraph of the next section. Two pages, none empty.
#[test]
fn an_empty_kept_closing_paragraph_leaves_no_empty_page() {
    crate::test_fonts();
    let l = lay(&build(9, true, SectionStart::NextPage, true, true));
    assert_eq!(l.pages.len(), 2);
    assert_eq!(page_of(&l, 9), Some(0), "the closing paragraph moved to the next page");
    assert_eq!(page_of(&l, 10), Some(1));
}

#[test]
fn a_kept_closing_paragraph_stays_on_its_page_and_adds_no_page() {
    crate::test_fonts();
    for start in NEXT_BREAKS {
        for empty in [true, false] {
            for n in 0..25 {
                let kept = lay(&build(n, true, start, true, empty));
                let plain = lay(&build(n, false, start, true, empty));
                assert_eq!(kept.pages.len(), plain.pages.len(), "{start:?}, {n} filler paragraphs, empty {empty}: keep with next added a page");
                assert_eq!(page_of(&kept, n as u32), page_of(&plain, n as u32), "{start:?}, {n} filler paragraphs, empty {empty}");
            }
        }
    }
}

#[test]
fn keep_with_next_still_moves_an_ordinary_paragraph_with_its_next_line() {
    crate::test_fonts();
    // No section ends here: the 15 pt paragraph fits (195 of 200 pt) but the 20 pt one after it
    // does not, so the pair goes to page 2 together.
    let kept = lay(&build(9, true, SectionStart::NextPage, false, false));
    let plain = lay(&build(9, false, SectionStart::NextPage, false, false));
    assert_eq!(page_of(&plain, 9), Some(0));
    assert_eq!(page_of(&kept, 9), Some(1));
}

#[test]
fn keep_with_next_before_a_continuous_section_still_keeps_with_its_next_line() {
    crate::test_fonts();
    // The next section continues on this page, so its first line is what the paragraph keeps
    // with (not measured in Word; the earlier behaviour stays).
    let kept = lay(&build(9, true, SectionStart::Continuous, true, false));
    let plain = lay(&build(9, false, SectionStart::Continuous, true, false));
    assert_eq!(page_of(&plain, 9), Some(0));
    assert_eq!(page_of(&kept, 9), Some(1));
}
