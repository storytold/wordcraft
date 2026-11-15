//! A header in a document of Word 2010's layout or older (compatibility mode below 15) keeps its
//! text where it is beside its own floating objects, which wrap nothing there: a top and bottom
//! object only makes the header taller when it starts above the end of the text (#6805). From
//! Word 2013's layout (mode 15) the header's text goes below such an object.
//!
//! Measured in Word for Windows 16.0 on synthetic files (A4, Arial 10 pt, header 17.3 pt down,
//! a text box 451.65 x 29.55 pt anchored to the header's paragraph and set 7.2 pt below its top,
//! top and bottom wrapping, one line of text): mode 14 puts the body 38.3 pt below the header's
//! top (the box's bottom, 36.7 pt, and a line's rounding), mode 15 puts it 76.1 pt below (the
//! box, then the line). A box set 78 pt below the paragraph's top, under its text, changes
//! nothing in either mode. Square wrapping at mode 14 leaves the text and the header's height as
//! they are.

use super::{lay, rect_shape};
use crate::{DocLayout, Placed, StoryRef};
use wordcraft_doc::para::{Anchor, Float, Wrap};
use wordcraft_doc::{Document, ParaProps, Paragraph, PartKind, Path, para_block};

const HEADER_TOP: f32 = 17.3;
const BOX_H: f32 = 29.55;

/// A page whose header holds one line of text with a text-box-sized shape anchored to it, set
/// `offset` below the paragraph's top with `wrap` (`None`: no shape), laid out as `mode` says.
fn page(mode: u32, wrap: Option<Wrap>, offset: f32) -> DocLayout {
    crate::test_fonts();
    let mut d = Document::from_text("Body text.");
    d.settings.compat_mode = mode;
    d.last_section.header = HEADER_TOP;
    d.last_section.margin_top = 20.0;
    let mut p = Paragraph::with_text("Header line", Default::default());
    p.props = ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
    if let Some(wrap) = wrap {
        let float = Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 31.6, y: offset, ..Default::default() };
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
    l.pages[0].items.iter().find_map(|i| if let Placed::Lines { y, story: StoryRef::Part(_), .. } = i { Some(*y) } else { None }).unwrap()
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
