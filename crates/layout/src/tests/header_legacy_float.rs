//! A header in a document of Word 2010's layout or older (compatibility mode below 15) keeps its
//! text where it is beside its own floating objects, which wrap nothing there: a top and bottom
//! object only makes the header taller when it starts above the end of the text (#6805). From
//! Word 2013's layout (mode 15) the header's text goes below such an object.
//!
//! Measured in Word for Windows 16.0 on synthetic files (A4, Arial 10 pt, a header 17.3 pt down,
//! 20 pt top margin or more, a text box 451.65 x 29.55 pt anchored to the header's paragraph and
//! set 7.2 pt below its top, top and bottom wrapping, one line of text): at mode 14 the line stays
//! at the header's top and the body's first line starts at 54.3 pt (the box's bottom, 54.05, and
//! the line's rounding); at mode 15 the line goes below the box and the body starts at 65.9.
//! A box set 78 pt below a 39 pt line changes nothing in either mode. A wrap distance below the
//! box counts at both modes (14.4 pt: the body 14.4 pt lower), the one above changes nothing. A
//! box starting in the space after the last paragraph makes the header end at its bottom. With
//! three paragraphs and a box set 26 pt down the page, anchored in the second, no paragraph moves
//! at mode 14 and the body starts at the box's bottom (55.9 pt). Square wrapping at mode 14 leaves
//! the text as it is. Mode 11 (Word 2003's layout, measured on .doc files) behaves like 14: the
//! single line stays at the top and the body starts at 54.0 pt (mode 12: 54.0, mode 14: 54.8), and
//! no paragraph of the three-paragraph header moves. A 60 pt logo with Square, Tight or Through
//! wrap leaves three short header paragraphs in place at mode 14. At mode 15 each line shifts
//! right of the logo. Footers (which differ) and header table cells are outside this test.

use super::{lay, rect_shape};
use crate::{DocLayout, Placed, StoryRef};
use wordcraft_doc::para::{Anchor, Float, Wrap};
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{Document, ParaProps, Paragraph, PartKind, Path, para_block};

const HEADER_TOP: f32 = 17.3;
const BOX_H: f32 = 29.55;

/// A page whose header holds one line of text with a text-box-sized shape anchored to it, set
/// `offset` below the paragraph's top with `wrap` (`None`: no shape), laid out as `mode` says.
fn page(mode: u32, wrap: Option<Wrap>, offset: f32) -> DocLayout {
    page_with(mode, wrap, offset, (0.0, 0.0))
}

/// [`page`] with a wrap distance below the object and a space after the header's paragraph.
fn page_with(mode: u32, wrap: Option<Wrap>, offset: f32, (dist_bottom, after): (f32, f32)) -> DocLayout {
    crate::test_fonts();
    let mut d = Document::from_text("Body text.");
    d.settings.compat_mode = mode;
    d.last_section.header = HEADER_TOP;
    d.last_section.margin_top = 20.0;
    let mut p = Paragraph::with_text("Header line", Default::default());
    p.props = ParaProps { space_before: Some(0.0), space_after: Some(after), ..Default::default() };
    if let Some(wrap) = wrap {
        let float = Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 31.6, y: offset, dist_bottom, ..Default::default() };
        p.insert_object(0, rect_shape(451.65, BOX_H, float), &Default::default()).unwrap();
    }
    let id = d.add_part(PartKind::Header, vec![para_block(p)]);
    d.last_section.headers.default = Some(id);
    lay(&d)
}

fn body_top(l: &DocLayout) -> f32 {
    l.pages[0].items.iter().find_map(|i| if let Placed::Lines { y, story: StoryRef::Body, .. } = i { Some(*y) } else { None }).unwrap()
}

fn header_line_top(l: &DocLayout) -> f32 {
    l.pages[0].header.iter().find_map(|i| if let Placed::Lines { y, story: StoryRef::Part(_), .. } = i { Some(*y) } else { None }).unwrap()
}

#[test]
fn mode_14_header_text_stays_beside_its_own_top_and_bottom_object() {
    let (old, new, none) = (page(14, Some(Wrap::TopAndBottom), 7.2), page(15, Some(Wrap::TopAndBottom), 7.2), page(14, None, 0.0));
    // The line stays at the header's top at mode 14, below the box at mode 15.
    assert!((header_line_top(&old) - header_line_top(&none)).abs() < 0.5, "line at {}", header_line_top(&old));
    assert!(header_line_top(&new) > HEADER_TOP + 7.2 + BOX_H - 0.5, "line at {}", header_line_top(&new));
    // The body starts below the box at mode 14 (it reaches past the text), a line lower at 15.
    assert!((body_top(&old) - (HEADER_TOP + 7.2 + BOX_H)).abs() < 0.5, "body at {}", body_top(&old));
    assert!(body_top(&new) > body_top(&old) + 5.0, "mode 15 body at {}, mode 14 at {}", body_top(&new), body_top(&old));
}

#[test]
fn mode_14_object_set_below_the_header_text_adds_no_height() {
    // Starting 78 pt down, under the line: no room taken in either mode.
    let none = body_top(&page(14, None, 0.0));
    for mode in [14, 15] {
        let t = body_top(&page(mode, Some(Wrap::TopAndBottom), 78.0));
        assert!((t - none).abs() < 0.5, "mode {mode}: body at {t}, {none} without the object");
    }
}

#[test]
fn mode_14_square_object_leaves_header_text_and_height() {
    let (sq, none) = (page(14, Some(Wrap::Square), 7.2), page(14, None, 0.0));
    assert!((header_line_top(&sq) - header_line_top(&none)).abs() < 0.5, "line at {}", header_line_top(&sq));
    assert!((body_top(&sq) - body_top(&none)).abs() < 0.5, "body at {}, {} without", body_top(&sq), body_top(&none));
}

#[test]
fn mode_14_header_text_ignores_a_narrow_logo_with_square_tight_or_through_wrap() {
    crate::test_fonts();
    let layout = |mode, wrap| {
        let mut d = Document::from_text("Body text.");
        d.settings.compat_mode = mode;
        d.last_section.header = 30.0;
        d.last_section.margin_top = 72.0;
        let mut blocks = Vec::new();
        for (i, text) in ["First line", "Second line", "Third line"].into_iter().enumerate() {
            let mut p = Paragraph::with_text(text, Default::default());
            p.props =
                ParaProps { space_before: Some(0.0), space_after: Some(0.0), line_spacing: Some(LineSpacing::Exactly(12.0)), ..Default::default() };
            if i == 0
                && let Some(wrap) = wrap
            {
                let float = Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, dist: 8.0, ..Default::default() };
                p.insert_object(0, rect_shape(60.0, 60.0, float), &Default::default()).unwrap();
            }
            blocks.push(para_block(p));
        }
        let id = d.add_part(PartKind::Header, blocks);
        d.last_section.headers.default = Some(id);
        lay(&d)
    };
    let header_positions = |layout: &DocLayout| {
        layout.pages[0]
            .header
            .iter()
            .filter_map(|item| match item {
                Placed::Lines { x, y, para, l0, .. } => para.lines.get(*l0).map(|line| (x + line.left, *y)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for mode in [11, 12, 13, 14, 15] {
        let plain = layout(mode, None);
        let control = header_positions(&plain);
        assert_eq!(control.len(), 3);
        for wrap in [Wrap::Square, Wrap::Tight, Wrap::Through] {
            let with_logo = layout(mode, Some(wrap));
            let positions = header_positions(&with_logo);
            assert_eq!(positions.len(), control.len(), "mode {mode}, {wrap:?}: {positions:?}");
            for (&(x, y), &(plain_x, plain_y)) in positions.iter().zip(&control) {
                assert!((y - plain_y).abs() < 0.5, "mode {mode}, {wrap:?}: line moved vertically from {plain_y} to {y}");
                if mode == 14 || (mode < 14 && wrap == Wrap::Square) {
                    assert!((x - plain_x).abs() < 0.5, "mode {mode}, {wrap:?}: line moved from {plain_x} to {x}");
                } else {
                    assert!(x > plain_x + 59.5, "mode {mode}, {wrap:?}: line at {x} does not clear the logo");
                }
            }
            assert!((body_top(&with_logo) - body_top(&plain)).abs() < 0.5, "mode {mode}, {wrap:?}: body moved vertically");
        }
    }
}

#[test]
fn mode_14_body_text_still_goes_below_its_own_top_and_bottom_object() {
    crate::test_fonts();
    let top = |mode| {
        let mut d = Document::from_text("Body text.");
        d.settings.compat_mode = mode;
        d.last_section.margin_top = 20.0;
        let float = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 31.6, y: 7.2, ..Default::default() };
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.insert_object(0, rect_shape(451.65, BOX_H, float), &Default::default()).unwrap();
        body_top(&lay(&d))
    };
    let (old, new) = (top(14), top(15));
    assert!((old - new).abs() < 0.5 && old > 20.0 + 7.2 + BOX_H - 0.5, "mode 14 body line at {old}, mode 15 at {new}");
}

#[test]
fn mode_14_distance_below_the_object_counts_and_a_start_in_the_space_after_extends() {
    // Word: 14.4 pt below the box make the body 14.4 pt lower.
    let (plain, spaced) = (body_top(&page(14, Some(Wrap::TopAndBottom), 7.2)), body_top(&page_with(14, Some(Wrap::TopAndBottom), 7.2, (14.4, 0.0))));
    assert!((spaced - plain - 14.4).abs() < 0.5, "body at {spaced}, {plain} without the distance");
    // A box starting 15 pt down, past the 11.5 pt line but inside its 10 pt space after: the
    // header ends at the box's bottom (Word: 62.1 from the page's top).
    let inside = page_with(14, Some(Wrap::TopAndBottom), 15.0, (0.0, 10.0));
    assert!((body_top(&inside) - (HEADER_TOP + 15.0 + BOX_H)).abs() < 0.5, "body at {}", body_top(&inside));
}

#[test]
fn mode_14_no_header_paragraph_moves_for_an_object_set_down_the_page_from_the_second() {
    crate::test_fonts();
    let lines = |mode: u32, object: bool| {
        let mut d = Document::from_text("Body text.");
        d.settings.compat_mode = mode;
        d.last_section.header = HEADER_TOP;
        d.last_section.margin_top = 20.0;
        let tight = ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
        let mut blocks = Vec::new();
        for (i, t) in ["First line", "Second line", "Third line"].into_iter().enumerate() {
            let mut p = Paragraph::with_text(t, Default::default());
            p.props = tight.clone();
            if object && i == 1 {
                let float = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 60.0, ..Default::default() };
                p.insert_object(0, rect_shape(451.65, BOX_H, float), &Default::default()).unwrap();
            }
            blocks.push(para_block(p));
        }
        let id = d.add_part(PartKind::Header, blocks);
        d.last_section.headers.default = Some(id);
        let l = lay(&d);
        let tops: Vec<f32> = l.pages[0].header.iter().filter_map(|i| if let Placed::Lines { y, .. } = i { Some(*y) } else { None }).collect();
        (tops, body_top(&l))
    };
    let ((plain, _), (with, body)) = (lines(14, false), lines(14, true));
    assert_eq!(plain.len(), with.len());
    for (a, b) in plain.iter().zip(&with) {
        assert!((a - b).abs() < 0.5, "header lines at {with:?}, {plain:?} without the object");
    }
    // The box is 60 pt down the page, over the third line (the test font's lines are taller than
    // Word's 11.5 pt, so the box is set lower than the 26 pt measured): the body starts at its bottom.
    assert!((body - (60.0 + BOX_H)).abs() < 0.5, "body at {body}");
}

#[test]
fn mode_11_header_text_stays_beside_its_own_top_and_bottom_object_like_mode_14() {
    // Word: modes 11, 12 and 14 keep the line at the header's top; mode 15 moves it below the box.
    let (m11, m12, m14, m15) = (
        page(11, Some(Wrap::TopAndBottom), 7.2),
        page(12, Some(Wrap::TopAndBottom), 7.2),
        page(14, Some(Wrap::TopAndBottom), 7.2),
        page(15, Some(Wrap::TopAndBottom), 7.2),
    );
    for l in [&m11, &m12] {
        assert!((header_line_top(l) - header_line_top(&m14)).abs() < 0.5, "line at {}, {} at mode 14", header_line_top(l), header_line_top(&m14));
        assert!((body_top(l) - body_top(&m14)).abs() < 0.5, "body at {}, {} at mode 14", body_top(l), body_top(&m14));
    }
    assert!(header_line_top(&m15) > header_line_top(&m11) + 5.0, "mode 15 line at {}, mode 11 at {}", header_line_top(&m15), header_line_top(&m11));
}
