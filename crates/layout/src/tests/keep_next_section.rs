//! Keep with next on the paragraph that closes a section (#6834). When the next section starts
//! on a new page, the paragraph after it is not on this page, so Keep with next has nothing to
//! keep it with: it stays at the bottom of its page. Moving it over for want of the next
//! section's first line left a page holding only that section's paragraph. Page counts are
//! compared with and without the flag over a sweep of filler lengths, so the case where the
//! closing paragraph fits and its successor's line would not is always hit (no font metrics
//! assumed). Invented text only.

use crate::*;
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{ParaProps, Paragraph};

/// `filler` short paragraphs, a closing paragraph (the end of section one, kept with the next
/// when `keep`), then a paragraph of section two, which starts as `start`. With `closes` false
/// the closing paragraph ends no section (a plain paragraph before one more of the same section).
fn build(filler: usize, keep: bool, start: SectionStart, closes: bool) -> Document {
    let mut d = Document::new();
    d.body.clear();
    for i in 0..filler {
        d.body.push(wordcraft_doc::para_block(Paragraph::with_text(&format!("Filler line {i}"), Default::default())));
    }
    let mut last = Paragraph::with_text("Closing paragraph of the first part", Default::default());
    if keep {
        last.props = ParaProps { keep_next: Some(true), ..Default::default() };
    }
    if closes {
        last.section = Some(Box::new(SectionProps::default()));
    }
    d.body.push(wordcraft_doc::para_block(last));
    d.body.push(wordcraft_doc::para_block(Paragraph::with_text("First paragraph of the second part", Default::default())));
    d.last_section.start = start;
    d
}

fn page_count(d: &Document) -> usize {
    lay(d).pages.len()
}

/// Filler counts that go through the page boundary: the closing paragraph first fits, then not.
const SWEEP: std::ops::Range<usize> = 10..90;

#[test]
fn a_kept_paragraph_closing_a_next_page_section_stays_on_its_page() {
    crate::test_fonts();
    for start in [SectionStart::NextPage, SectionStart::OddPage, SectionStart::EvenPage] {
        for n in SWEEP {
            let kept = page_count(&build(n, true, start, true));
            let plain = page_count(&build(n, false, start, true));
            assert_eq!(kept, plain, "{start:?}, {n} filler paragraphs: keep with next added a page");
        }
    }
    // The sweep does go past one page of filler, where the closing paragraph is last on page 1.
    assert!(SWEEP.any(|n| page_count(&build(n, false, SectionStart::NextPage, true)) > 2));
}

#[test]
fn keep_with_next_still_pulls_an_ordinary_paragraph_over_with_its_next_line() {
    crate::test_fonts();
    // No section ends here: the paragraph must travel with the next one, which costs a page at
    // least for one filler count.
    let pulled = SWEEP.any(|n| page_count(&build(n, true, SectionStart::NextPage, false)) > page_count(&build(n, false, SectionStart::NextPage, false)));
    assert!(pulled, "keep with next never moved the paragraph");
}

#[test]
fn keep_with_next_before_a_continuous_section_still_keeps_with_its_next_line() {
    crate::test_fonts();
    // The next section continues on this page, so its first line is what the paragraph keeps with
    // (not measured in Word; the earlier behaviour stays).
    let pulled = SWEEP.any(|n| page_count(&build(n, true, SectionStart::Continuous, true)) > page_count(&build(n, false, SectionStart::Continuous, true)));
    assert!(pulled, "keep with next before a continuous section never moved the paragraph");
}
