//! Hostile documents whose layout work or memory would otherwise grow far past their size: each
//! is bounded by a cap, and real documents stay well under it.

use super::*;
use wordcraft_doc::props::HeightRule;
use wordcraft_doc::table::{MAX_COLS, Table};
use wordcraft_doc::{Block, PartKind, Pos, TextBody, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// A `w` × `h` text box showing `text`, inline at the end of body paragraph `para`; its story id.
fn text_box(d: &mut Document, para: usize, text: &str, w: f32, h: f32) -> u32 {
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(text, Default::default()))]);
    if let Some(p) = d.parts.get_mut(&id) {
        p.body = TextBody::default();
    }
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let off = d.para(StoryRef::Body, &Path::top(para)).map_or(0, |p| p.len());
    d.insert_object(&Pos::body(para, off), shape, &Default::default()).unwrap();
    id
}

#[test]
fn a_linked_chain_lays_its_story_out_at_a_bounded_number_of_widths() {
    // Forty linked boxes, each a line deep and each a different width, over a long story: every
    // width would lay the whole story out again. Past the cap, later boxes reuse the nearest
    // width already laid out, so every box still shows its share of the story.
    let n = 40;
    let mut d = Document::from_text(&vec!["p"; n].join("\n"));
    let story = "Words that run on and on through every box of the chain. ".repeat(200);
    let ids: Vec<u32> = (0..n).map(|i| text_box(&mut d, i, if i == 0 { &story } else { "" }, 100.0 + 3.0 * i as f32, 28.0)).collect();
    for w in ids.windows(2) {
        d.parts.get_mut(&w[0]).unwrap().body.next = Some(w[1]);
    }
    let l = lay(&d);
    let lines: Vec<&Placed> =
        l.pages.iter().flat_map(|p| &p.items).filter(|it| matches!(it, Placed::Lines { story: StoryRef::Part(s), .. } if *s == ids[0])).collect();
    assert_eq!(lines.len(), n, "every box shows the story");
    // Each width the story was laid out at is its own paragraph layout.
    let mut widths: Vec<usize> =
        lines.iter().filter_map(|it| if let Placed::Lines { para, .. } = it { Some(Arc::as_ptr(para) as usize) } else { None }).collect();
    widths.sort_unstable();
    widths.dedup();
    assert!(widths.len() <= textbox::MAX_FLOW_WIDTHS, "{} widths laid the story out", widths.len());
}

/// The paragraph layouts (by address) showing story `head` across the document.
fn story_layouts(l: &DocLayout, head: u32) -> Vec<usize> {
    l.pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), para, .. } if *s == head => Some(Arc::as_ptr(para) as usize),
            _ => None,
        })
        .collect()
}

#[test]
fn stacked_boxes_of_any_width_share_a_layout() {
    // Stacked text is laid out one letter to a line whatever the box's width.
    let n = 20;
    let mut d = Document::from_text(&vec!["p"; n].join("\n"));
    let story = "Stacked letters run on through every box. ".repeat(20);
    let ids: Vec<u32> = (0..n).map(|i| text_box(&mut d, i, if i == 0 { &story } else { "" }, 100.0 + 3.0 * i as f32, 28.0)).collect();
    for id in &ids {
        d.parts.get_mut(id).unwrap().body.vert = wordcraft_doc::props::TextVert::Stacked;
    }
    for w in ids.windows(2) {
        d.parts.get_mut(&w[0]).unwrap().body.next = Some(w[1]);
    }
    let mut widths = story_layouts(&lay(&d), ids[0]);
    assert_eq!(widths.len(), n, "every box shows the story");
    widths.sort_unstable();
    widths.dedup();
    assert_eq!(widths.len(), 1, "one layout for every stacked box");
}

#[test]
fn past_the_cap_a_box_takes_a_layout_in_its_own_direction() {
    // The cap is full of horizontal widths when a vertical box comes: it lays the story out
    // its own way rather than showing a horizontal layout.
    let n = textbox::MAX_FLOW_WIDTHS + 1;
    let mut d = Document::from_text(&vec!["p"; n].join("\n"));
    let story = "Words that run on and on through every box of the chain. ".repeat(200);
    let ids: Vec<u32> = (0..n).map(|i| text_box(&mut d, i, if i == 0 { &story } else { "" }, 100.0 + 3.0 * i as f32, 28.0)).collect();
    if let Some(last) = ids.last() {
        d.parts.get_mut(last).unwrap().body.vert = wordcraft_doc::props::TextVert::Vert;
    }
    for w in ids.windows(2) {
        d.parts.get_mut(&w[0]).unwrap().body.next = Some(w[1]);
    }
    let mut widths = story_layouts(&lay(&d), ids[0]);
    assert_eq!(widths.len(), n, "every box shows the story");
    widths.sort_unstable();
    widths.dedup();
    assert_eq!(widths.len(), n, "the vertical box has a layout of its own");
}

#[test]
fn boxes_a_hair_apart_in_width_share_a_layout() {
    // Resized boxes differ by fractions of a point; those within a twentieth of a point of each
    // other share one layout of the story.
    let n = 30;
    let mut d = Document::from_text(&vec!["p"; n].join("\n"));
    let story = "Words that run on and on through every box of the chain. ".repeat(100);
    let ids: Vec<u32> = (0..n).map(|i| text_box(&mut d, i, if i == 0 { &story } else { "" }, 150.0 + 0.0005 * i as f32, 28.0)).collect();
    for w in ids.windows(2) {
        d.parts.get_mut(&w[0]).unwrap().body.next = Some(w[1]);
    }
    let l = lay(&d);
    let mut widths: Vec<usize> = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), para, .. } if *s == ids[0] => Some(Arc::as_ptr(para) as usize),
            _ => None,
        })
        .collect();
    assert_eq!(widths.len(), n, "every box shows the story");
    widths.sort_unstable();
    widths.dedup();
    assert_eq!(widths.len(), 1, "one layout for every box");
}

#[test]
fn repeated_header_items_are_capped_across_the_table() {
    // An exact-height header row keeps thousands of overflowing paragraphs without growing, and
    // repeats on each of hundreds of pages: every page would copy all of them.
    let mut t = Table::new(201, 1, 468.0);
    t.rows[0].props.header = true;
    t.rows[0].props.height = Some(20.0);
    t.rows[0].props.height_rule = HeightRule::Exact;
    t.rows[0].cells[0].blocks = (0..3000).map(|_| para_block(Paragraph::with_text("x", Default::default()))).collect();
    for row in t.rows.iter_mut().skip(1) {
        row.props.height = Some(600.0);
        row.props.height_rule = HeightRule::Exact;
    }
    let mut d = Document::from_text("");
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 200, "{} pages", l.pages.len());
    let items: usize = l.pages.iter().map(|p| p.items.len()).sum();
    // The first page's header and each page's body row come on top of the repeats.
    assert!(items <= MAX_HEADER_ITEMS + 3000 + 50 * l.pages.len(), "{items} items placed");
    // An ordinary table's header still repeats on every page.
    let mut t = Table::new(80, 2, 468.0);
    t.rows[0].props.header = true;
    let mut d = Document::from_text("");
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 2);
    for p in &l.pages {
        assert!(p.items.iter().any(|i| matches!(i, Placed::Cell { row: 0, .. })), "the header repeats");
    }
}

#[test]
fn a_huge_grid_after_is_bounded_like_the_docx_readers() {
    // A native document can say a row leaves four billion columns empty after its cells.
    let mut t = Table::new(1, 2, 468.0);
    t.rows[0].props.grid_after = u32::MAX;
    t.grid.clear();
    assert!(t.cols() <= 2 + MAX_COLS, "{} columns", t.cols());
    let mut d = Document::from_text("");
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
    let l = lay(&d);
    assert!(l.pages[0].items.iter().any(|i| matches!(i, Placed::Cell { row: 0, cell: 1, .. })));
}

/// A `size` × `size` PNG, transparent but for an opaque disc in the middle.
fn disc_png(size: u32) -> Vec<u8> {
    let c = size as f32 / 2.0;
    let img = image::RgbaImage::from_fn(size, size, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
        image::Rgba([0, 0, 0, if dx * dx + dy * dy < c * c * 0.5 { 255 } else { 0 }])
    });
    let mut buf = Vec::new();
    image::DynamicImage::from(img).write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

#[test]
fn a_picture_cropped_many_ways_is_decoded_once() {
    let png = disc_png(64);
    let mut alphas = contour::Alphas::default();
    for i in 0..300 {
        let f = i as f32 / 1000.0;
        assert!(alphas.outline(&png, [f, f, 0.0, 0.0]).is_ok_and(|o| o.is_some()), "crop {i} follows the disc");
    }
    assert_eq!(alphas.decoded, 64 * 64, "one decode for every crop");
    // A crop's outline is the one the picture alone gives.
    let mut fresh = contour::Alphas::default();
    assert_eq!(alphas.outline(&png, [0.1, 0.0, 0.2, 0.0]), fresh.outline(&png, [0.1, 0.0, 0.2, 0.0]));
}

#[test]
fn pictures_cropped_in_turn_are_each_decoded_once() {
    // Crops alternating between two pictures don't decode either again: enough of them would
    // otherwise spend the budget on pictures that fit in it.
    let (a, b) = (disc_png(64), disc_png(32));
    let mut alphas = contour::Alphas::default();
    for i in 0..300 {
        let f = i as f32 / 1000.0;
        assert!(alphas.outline(&a, [f, 0.0, 0.0, 0.0]).is_ok_and(|o| o.is_some()), "crop {i} of a");
        assert!(alphas.outline(&b, [0.0, f, 0.0, 0.0]).is_ok_and(|o| o.is_some()), "crop {i} of b");
    }
    assert_eq!(alphas.decoded, 64 * 64 + 32 * 32, "one decode per picture");
}

#[test]
fn decoding_pictures_stops_at_the_layouts_budget() {
    let (a, b) = (disc_png(64), disc_png(32));
    let mut alphas = contour::Alphas::default();
    alphas.decoded = contour::MAX_DECODED_PIXELS - 64 * 64 + 1;
    // Too many pixels left to decode: the picture wraps as its rectangle, and says why.
    assert_eq!(alphas.outline(&a, [0.0; 4]), Err(contour::OverBudget));
    // A smaller one still fits.
    assert!(alphas.outline(&b, [0.0; 4]).is_ok_and(|o| o.is_some()));
    assert!(alphas.decoded <= contour::MAX_DECODED_PIXELS);
    // Pictures alternating past the budget aren't decoded again.
    for _ in 0..10 {
        assert_eq!(alphas.outline(&a, [0.0; 4]), Err(contour::OverBudget));
        let _ = alphas.outline(&b, [0.0; 4]);
    }
    assert!(alphas.decoded <= contour::MAX_DECODED_PIXELS);
}

/// A `size` × `size` PNG with no transparency.
fn opaque_png(size: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(size, size, image::Rgb([0, 0, 0]));
    let mut buf = Vec::new();
    image::DynamicImage::from(img).write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

#[test]
fn opaque_pictures_take_none_of_the_decoding_budget() {
    // Pictures whose header says they have no transparency have no outline to find: decoding
    // them would only spend the budget a transparent picture after them needs.
    let (opaque, disc) = (opaque_png(64), disc_png(64));
    let mut alphas = contour::Alphas::default();
    alphas.decoded = contour::MAX_DECODED_PIXELS - 64 * 64;
    for _ in 0..3 {
        assert_eq!(alphas.outline(&opaque, [0.0; 4]), Ok(None));
    }
    assert!(alphas.outline(&disc, [0.0; 4]).is_ok_and(|o| o.is_some()), "the transparent picture still has its outline");
    assert_eq!(alphas.decoded, contour::MAX_DECODED_PIXELS);
}

#[test]
fn a_picture_refused_for_the_budget_is_not_cached_across_layouts() {
    // One layout ran out of budget before this picture; the next has a budget of its own.
    let disc = Arc::new(disc_png(64));
    let mut outlines = HashMap::new();
    let key = ("p".to_string(), 1, 2, [0; 4]);
    let mut spent = contour::Alphas::default();
    spent.decoded = contour::MAX_DECODED_PIXELS;
    assert!(picture_outline(&mut outlines, &mut spent, key.clone(), &disc, [0.0; 4]).is_none());
    assert!(outlines.is_empty(), "the refusal isn't kept");
    let mut fresh = contour::Alphas::default();
    assert!(picture_outline(&mut outlines, &mut fresh, key.clone(), &disc, [0.0; 4]).is_some());
    assert!(outlines.get(&key).is_some_and(|(_, o)| o.is_some()), "the outline is kept");
}

/// The outline cache outlives the documents it is used with and tells pictures apart by their
/// bytes' address: each entry keeps that allocation (a weak reference, not the bytes' owner), so
/// it can't be freed and the address given to another picture's bytes while the entry is kept.
#[test]
fn a_kept_outline_holds_on_to_its_pictures_allocation() {
    let disc = Arc::new(disc_png(64));
    let mut outlines = HashMap::new();
    let key = ("p".to_string(), Arc::as_ptr(&disc) as usize, disc.len(), [0; 4]);
    assert!(picture_outline(&mut outlines, &mut contour::Alphas::default(), key.clone(), &disc, [0.0; 4]).is_some());
    let Some((weak, _)) = outlines.get(&key) else { panic!("the outline is kept") };
    assert!(std::ptr::eq(weak.as_ptr(), Arc::as_ptr(&disc)), "the entry refers to its own picture's bytes");
    assert_eq!((Arc::strong_count(&disc), Arc::weak_count(&disc)), (1, 1), "a weak reference: the document still owns the bytes");
    drop(disc);
    assert!(weak.upgrade().is_none(), "the bytes are freed with the document; only their allocation is kept");
}
