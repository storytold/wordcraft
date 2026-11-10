//! The PDF export draws a page one placed item at a time ([`ItemDisplay`]); warped text boxes
//! (Text Effects › Transform) must come out as [`page_display`] draws them, not flat.
use crate::display::{Draw, ItemDisplay, page_display};
use crate::*;
use wordcraft_doc::props::VAlign;
use wordcraft_doc::{PartKind, Path, Pos, StoryRef, TextBody, para_block};

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

// ---- PR #5302 review ----

/// The points of each path element, in order.
fn path_points(path: &kurbo::BezPath) -> Vec<Vec<kurbo::Point>> {
    use kurbo::PathEl;
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
            PathEl::QuadTo(a, b) => vec![a, b],
            PathEl::CurveTo(a, b, c) => vec![a, b, c],
            PathEl::ClosePath => Vec::new(),
        })
        .collect()
}

/// Every `Draw::Art` of `a` and `b`, in order: same count, colour, alpha and path points (0.01 pt).
fn same_art(a: &[Draw], b: &[Draw]) -> Result<(), String> {
    let arts = |d: &[Draw]| -> Vec<(Vec<Vec<kurbo::Point>>, wordcraft_doc::Rgb, f32)> {
        d.iter()
            .filter_map(|dr| match dr {
                Draw::Art { path, color, alpha, .. } => Some((path_points(path), *color, *alpha)),
                _ => None,
            })
            .collect()
    };
    let (x, y) = (arts(a), arts(b));
    if x.len() != y.len() {
        return Err(format!("{} art items vs {}", x.len(), y.len()));
    }
    for (k, (p, q)) in x.iter().zip(&y).enumerate() {
        if p.1 != q.1 || (p.2 - q.2).abs() > 1e-6 {
            return Err(format!("art {k}: colour/alpha {:?}/{} vs {:?}/{}", p.1, p.2, q.1, q.2));
        }
        if p.0.len() != q.0.len() {
            return Err(format!("art {k}: {} path elements vs {}", p.0.len(), q.0.len()));
        }
        for (e, (u, v)) in p.0.iter().zip(&q.0).enumerate() {
            let close = u.len() == v.len() && u.iter().zip(v).all(|(s, t)| (s.x - t.x).abs() <= 0.01 && (s.y - t.y).abs() <= 0.01);
            if !close {
                return Err(format!("art {k} element {e}: {u:?} vs {v:?}"));
            }
        }
    }
    Ok(())
}

/// Visible glyph runs of any text.
fn visible_runs(draws: &[Draw]) -> usize {
    draws.iter().filter(|dr| matches!(dr, Draw::Glyphs { alpha, text, .. } if *alpha > 0.0 && !text.trim().is_empty())).count()
}

#[test]
fn art_paths_match_page_display_exactly() {
    for warp in ["textArchUp", "textPlain", "textWave1"] {
        let (d, _) = wordart(warp, &["Your Text Here"]);
        let l = lay(&d);
        let page = &l.pages[0];
        let whole = page_display(&d, page, &Default::default());
        if let Err(e) = same_art(&one_by_one(&d, page), &whole) {
            panic!("{warp}: {e}");
        }
    }
}

#[test]
fn a_warped_box_with_two_paragraphs_draws_no_flat_text() {
    let (d, _) = wordart("textArchUp", &["Your Text", "Here"]);
    let l = lay(&d);
    let page = &l.pages[0];
    let whole = page_display(&d, page, &Default::default());
    let draws = one_by_one(&d, page);
    same_art(&draws, &whole).unwrap();
    assert_eq!(visible_box_glyphs(&draws, "Your Text Here"), 0, "the bent text isn't also drawn flat");
    assert_eq!(visible_runs(&draws), visible_runs(&whole), "the same visible glyph runs as the page");
}

#[test]
fn cloned_items_draw_the_same_bent_text() {
    let (d, _) = wordart("textArchUp", &["Your Text", "Here"]);
    let l = lay(&d);
    let page = &l.pages[0];
    let whole = page_display(&d, page, &Default::default());
    let shown = ItemDisplay::new(&d, page);
    let opts = Default::default();
    let cloned: Vec<Placed> = page.header.iter().chain(&page.footer).chain(&page.items).cloned().collect();
    let draws: Vec<Draw> = cloned.iter().flat_map(|it| shown.draws(it, &opts)).collect();
    assert_eq!(visible_box_glyphs(&draws, "Your Text Here"), 0, "a cloned line isn't drawn flat");
    same_art(&draws, &whole).unwrap();
}

/// The arch WordArt box of [`wordart`] anchored in the default header and the default footer.
fn wordart_in_header_and_footer() -> Document {
    let (mut d, _) = wordart("textArchUp", &["Your Text Here"]);
    let (shape, story) = {
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        let shape = p.objects.first().cloned().expect("the WordArt shape");
        let InlineObject::Shape { story, .. } = &shape else { panic!("a shape") };
        (shape.clone(), story.expect("a story"))
    };
    let body = d.parts.get(&story).map(|p| p.blocks.clone()).unwrap();
    let mut sect = wordcraft_doc::section::SectionProps::default();
    for kind in [PartKind::Header, PartKind::Footer] {
        let id = d.add_part(kind, vec![para_block(Paragraph::with_text("q", Default::default()))]);
        // Each one shows its own copy of the box's story.
        let own = d.add_part(PartKind::TextBox, body.clone());
        if let (Some(src), Some(dst)) = (d.parts.get(&story).map(|p| p.body), d.parts.get_mut(&own)) {
            dst.body = src;
        }
        let mut s = shape.clone();
        if let InlineObject::Shape { story, .. } = &mut s {
            *story = Some(own);
        }
        d.insert_object(&Pos::new(StoryRef::Part(id), Path::top(0), 1), s, &Default::default()).unwrap();
        match kind {
            PartKind::Header => sect.headers.default = Some(id),
            _ => sect.footers.default = Some(id),
        }
    }
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(sect));
    d
}

#[test]
fn warped_boxes_in_header_and_footer_match_page_display() {
    let d = wordart_in_header_and_footer();
    let l = lay(&d);
    let page = &l.pages[0];
    assert!(!page.header.is_empty() && !page.footer.is_empty(), "header and footer laid out");
    // As the PDF export draws them: headers and footers at full strength.
    let opts = crate::display::DisplayOptions { dim_header: false, ..Default::default() };
    let whole = page_display(&d, page, &opts);
    let arts = whole.iter().filter(|dr| matches!(dr, Draw::Art { .. })).count();
    assert!(arts >= 3, "body, header and footer arches drawn as art by the page ({arts})");
    let shown = ItemDisplay::new(&d, page);
    let draws: Vec<Draw> = page.header.iter().chain(&page.footer).chain(&page.items).flat_map(|it| shown.draws(it, &opts)).collect();
    same_art(&draws, &whole).unwrap();
    assert_eq!(visible_box_glyphs(&draws, "Your Text Here"), 0);
}

#[test]
fn a_hidden_warped_box_draws_nothing() {
    let (d, _) = wordart("textArchUp", &["Your Text Here"]);
    let l = lay(&d);
    let page = &l.pages[0];
    let opts = crate::display::DisplayOptions { hide_object: Some(Pos::body(0, 1)), ..Default::default() };
    let shown = ItemDisplay::new(&d, page);
    let draws: Vec<Draw> = page.header.iter().chain(&page.footer).chain(&page.items).flat_map(|it| shown.draws(it, &opts)).collect();
    let whole = page_display(&d, page, &opts);
    assert!(!draws.iter().any(|dr| matches!(dr, Draw::Art { .. })), "no art for a hidden box");
    assert_eq!(visible_box_glyphs(&draws, "Your Text Here"), 0, "no flat text either");
    same_art(&draws, &whole).unwrap();
}
