//! The PDF export draws a page one placed item at a time ([`ItemDisplay`]); warped text boxes
//! (Text Effects › Transform) must come out as [`page_display`] draws them, not flat.
use crate::display::{Draw, ItemDisplay, page_display};
use crate::*;
use wordcraft_doc::props::VAlign;
use wordcraft_doc::{PartKind, Pos, TextBody, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// A body paragraph holding a 242.25 x 41.25 pt WordArt box with `paras` paragraphs of text,
/// bent by `warp`; the document and the box's story.
fn wordart(warp: &str, paras: &[&str]) -> (Document, u32) {
    // The anchor text must not be part of "Your Text Here": it is drawn visibly.
    let mut d = Document::from_text("q");
    let props = CharProps {
        font: Some("Source Sans 3".into()),
        size: Some(41.25 * 0.75),
        color: Some(wordcraft_doc::TextColor::Rgb(wordcraft_doc::Rgb::WHITE)),
        text_effects: Some(Box::new(wordcraft_doc::wordart::TextEffects {
            fill: Some(wordcraft_doc::wordart::TextFill::Solid { color: wordcraft_doc::Rgb::WHITE, transparency: 0.0 }),
            outline: Some(Default::default()),
            ..Default::default()
        })),
        ..CharProps::default()
    };
    let blocks = paras.iter().map(|t| para_block(Paragraph::with_text(t, props.clone()))).collect();
    let id = d.add_part(PartKind::TextBox, blocks);
    if let Some(p) = d.parts.get_mut(&id) {
        p.body = TextBody { anchor: VAlign::Center, inset: Some([0; 4]), ..Default::default() };
    }
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 242.25,
        h: if paras.len() > 1 { 90.0 } else { 41.25 },
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: wordcraft_doc::connector::ShapeExtra { warp: wordcraft_doc::wordart::TextWarp::new(warp), ..Default::default() },
    };
    d.insert_object(&Pos::body(0, 1), shape, &Default::default()).unwrap();
    (d, id)
}

/// The page's draw items as the PDF export gets them: each placed item on its own.
fn one_by_one(d: &Document, page: &Page) -> Vec<Draw> {
    let shown = ItemDisplay::new(d, page);
    let opts = Default::default();
    page.header.iter().chain(&page.footer).chain(&page.items).flat_map(|it| shown.draws(it, &opts)).collect()
}

fn art_boxes(draws: &[Draw]) -> Vec<kurbo::Rect> {
    draws
        .iter()
        .filter_map(|dr| match dr {
            Draw::Art { path, .. } => crate::wordart::path_box(path),
            _ => None,
        })
        .collect()
}

fn same_boxes(a: &[kurbo::Rect], b: &[kurbo::Rect]) -> bool {
    let close = |x: f64, y: f64| (x - y).abs() <= 0.5;
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| close(p.x0, q.x0) && close(p.y0, q.y0) && close(p.x1, q.x1) && close(p.y1, q.y1))
}

/// Visible glyph runs whose text is part of the WordArt (the box text drawn flat as well).
fn visible_box_glyphs(draws: &[Draw], words: &str) -> usize {
    draws
        .iter()
        .filter(|dr| matches!(dr, Draw::Glyphs { alpha, text, .. } if *alpha > 0.0 && !text.trim().is_empty() && words.contains(text.trim())))
        .count()
}

#[test]
fn bent_wordart_is_bent_when_drawn_item_by_item() {
    let (d, _) = wordart("textArchUp", &["Your Text Here"]);
    let l = lay(&d);
    let page = &l.pages[0];
    let whole = art_boxes(&page_display(&d, page, &Default::default()));
    assert!(!whole.is_empty(), "the page draws the arch as art");
    let draws = one_by_one(&d, page);
    let items = art_boxes(&draws);
    assert!(same_boxes(&items, &whole), "item by item {items:?} vs page {whole:?}");
    assert_eq!(visible_box_glyphs(&draws, "Your Text Here"), 0, "the bent text isn't also drawn flat");
}

#[test]
fn a_warped_box_with_two_paragraphs_is_drawn_once() {
    let (d, _) = wordart("textArchUp", &["Your Text", "Here"]);
    let l = lay(&d);
    let page = &l.pages[0];
    let whole = art_boxes(&page_display(&d, page, &Default::default()));
    let items = art_boxes(&one_by_one(&d, page));
    assert!(!whole.is_empty());
    assert_eq!(items.len(), whole.len(), "each line's letters once: {items:?} vs {whole:?}");
    assert!(same_boxes(&items, &whole));
}
