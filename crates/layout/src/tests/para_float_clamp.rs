//! Synthetic regressions for body paragraph/line-relative wrapping float clamping against
//! `page_top` in Word 2013+ (`compat_mode >= 15`) vs legacy compatibility modes.

use super::lay;
use crate::Placed;
use wordcraft_doc::para::{Anchor, Float, InlineObject, Wrap};
use wordcraft_doc::{Document, Paragraph, para_block};

fn float_picture(w: f32, h: f32, float: Float) -> InlineObject {
    InlineObject::Image { media: "synthetic-float".into(), w, h, alt: "synthetic".into(), float, crop: [0.0; 4], fx: Default::default(), ole: None }
}

/// In Word 2013+ (`compat_mode >= 15`), a body wrapping float positioned relative to its
/// paragraph with a negative offset is clamped to `page_top` (72 pt), both on its anchor
/// paragraph's first piece and when placed as a late float after an explicit page break.
#[test]
fn body_wrapping_paragraph_float_clamps_to_page_top_in_modern_compat() {
    let margin_top = 72.0;

    // 1. First-piece anchor on page 1.
    let mut doc = Document::new();
    doc.settings.compat_mode = 15;
    let mut p = Paragraph::with_text("Body", Default::default());
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 0.0, y: -72.0, ..Default::default() };
    p.insert_object(0, float_picture(72.0, 72.0, float), &Default::default()).unwrap();
    doc.body = vec![para_block(p)];

    let laid = lay(&doc);
    let img_y = laid.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Image { rect, .. } => Some(rect.y),
            _ => None,
        })
        .expect("image placed on page 1");
    assert!((img_y - margin_top).abs() < 0.01, "expected clamped y={margin_top}, got {img_y}");

    // 2. Late float anchored after an explicit page break on page 2.
    let mut doc2 = Document::new();
    doc2.settings.compat_mode = 15;
    let mut p2 = Paragraph::with_text(&format!("{}A", wordcraft_doc::para::PAGE_BREAK), Default::default());
    let float2 = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 0.0, y: -72.0, ..Default::default() };
    p2.insert_object(p2.text.len(), float_picture(72.0, 72.0, float2), &Default::default()).unwrap();
    doc2.body = vec![para_block(p2)];

    let laid2 = lay(&doc2);
    assert!(laid2.pages.len() >= 2, "page break should create page 2");
    let img2_y = laid2.pages[1]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Image { rect, .. } => Some(rect.y),
            _ => None,
        })
        .expect("late image placed on page 2");
    assert!((img2_y - margin_top).abs() < 0.01, "expected clamped late y={margin_top}, got {img2_y}");
}

/// In Word 2013+ (`compat_mode >= 15`), a body wrapping float positioned relative to its line
/// (`Anchor::Line`) with a negative offset is also clamped to `page_top`.
#[test]
fn body_wrapping_line_float_clamps_to_page_top_in_modern_compat() {
    let margin_top = 72.0;
    let mut doc = Document::new();
    doc.settings.compat_mode = 15;
    let mut p = Paragraph::with_text("Body", Default::default());
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Line, x: 0.0, y: -72.0, ..Default::default() };
    p.insert_object(0, float_picture(72.0, 72.0, float), &Default::default()).unwrap();
    doc.body = vec![para_block(p)];

    let laid = lay(&doc);
    let img_y = laid.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Image { rect, .. } => Some(rect.y),
            _ => None,
        })
        .expect("image placed on page 1");
    assert!((img_y - margin_top).abs() < 0.01, "expected clamped y={margin_top}, got {img_y}");
}

/// In legacy Word compatibility modes (`compat_mode < 15`, e.g. Word 2007 compat 12), a
/// paragraph-anchored wrapping float with a negative offset is NOT clamped to `page_top`.
#[test]
fn legacy_compat_paragraph_float_is_not_clamped_to_page_top() {
    let mut doc = Document::new();
    doc.settings.compat_mode = 12;
    let mut p = Paragraph::with_text("Body", Default::default());
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 0.0, y: -72.0, ..Default::default() };
    p.insert_object(0, float_picture(72.0, 72.0, float), &Default::default()).unwrap();
    doc.body = vec![para_block(p)];

    let laid = lay(&doc);
    let img_y = laid.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Image { rect, .. } => Some(rect.y),
            _ => None,
        })
        .expect("image placed on page 1");
    assert!(img_y.abs() < 0.01, "expected unclamped y=0.0 in compat 12, got {img_y}");
}

/// Margin-anchored wrapping floats and BehindText/InFrontOfText paragraph floats are NOT clamped
/// to `page_top` even in compat 15, and may extend into the top margin.
#[test]
fn margin_and_non_wrapping_floats_are_not_clamped_to_page_top() {
    for (v_rel, wrap) in [
        (Anchor::Margin, Wrap::Square),
        (Anchor::Margin, Wrap::TopAndBottom),
        (Anchor::Paragraph, Wrap::BehindText),
        (Anchor::Paragraph, Wrap::InFrontOfText),
    ] {
        let mut doc = Document::new();
        doc.settings.compat_mode = 15;
        let mut p = Paragraph::with_text("Body", Default::default());
        let float = Float { wrap, h_rel: Anchor::Margin, v_rel, x: 0.0, y: -72.0, ..Default::default() };
        p.insert_object(0, float_picture(72.0, 72.0, float), &Default::default()).unwrap();
        doc.body = vec![para_block(p)];

        let laid = lay(&doc);
        let img_y = laid.pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Image { rect, .. } => Some(rect.y),
                _ => None,
            })
            .expect("image placed");
        assert!(img_y.abs() < 0.01, "expected unclamped y=0.0 for ({v_rel:?}, {wrap:?}), got {img_y}");
    }
}
