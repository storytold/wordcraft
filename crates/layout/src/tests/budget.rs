//! Hostile documents whose layout work or memory would otherwise grow far past their size: each
//! is bounded by a cap, and real documents stay well under it.

use crate::ctx::Outlines;
use crate::*;
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
fn a_linked_chain_lays_its_story_out_for_a_bounded_amount_of_work() {
    // Forty linked boxes, each a line deep and each a different width, over a long story: every
    // width would lay the whole story out again. Past the work cap, later boxes reuse the
    // nearest width already laid out, so every box still shows its share of the story.
    textbox::FLOW_WORK_CAP.with(|c| c.set(2000));
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
    textbox::FLOW_WORK_CAP.with(|c| c.set(textbox::MAX_FLOW_WORK));
    // The cap is spent after the first few layouts (each is far over 2000 / 8 work).
    assert!(widths.len() <= 8, "{} widths laid the story out", widths.len());
}

#[test]
fn twenty_distinct_widths_of_an_ordinary_story_all_keep_their_width() {
    // A real chain of 20 boxes, 100, 105, ... 195 pt wide: each is laid out at its own width.
    let n = 20;
    let mut d = Document::from_text(&vec!["p"; n].join("\n"));
    let story = "Words that run on and on through every box of the chain. ".repeat(200);
    let ids: Vec<u32> = (0..n).map(|i| text_box(&mut d, i, if i == 0 { &story } else { "" }, 100.0 + 5.0 * i as f32, 28.0)).collect();
    for w in ids.windows(2) {
        d.parts.get_mut(&w[0]).unwrap().body.next = Some(w[1]);
    }
    let mut widths = story_layouts(&lay(&d), ids[0]);
    assert_eq!(widths.len(), n, "every box shows the story");
    widths.sort_unstable();
    widths.dedup();
    assert_eq!(widths.len(), n, "every box has a layout at its own width");
}

/// How many times story layouts ran while `doc` was laid out with the chain work cap at `cap`.
fn chain_layouts(d: &Document, cap: usize) -> usize {
    textbox::FLOW_WORK_CAP.with(|c| c.set(cap));
    textbox::FLOW_LAYOUTS.with(|c| c.set(0));
    lay(d);
    textbox::FLOW_WORK_CAP.with(|c| c.set(textbox::MAX_FLOW_WORK));
    textbox::FLOW_LAYOUTS.with(|c| c.get())
}

/// `n` linked boxes `w0`, `w0 + 0.1`, ... pt wide (each its own layout) and a line deep, the
/// first holding `blocks`; the first box's id.
fn hairline_chain(d: &mut Document, n: usize, w0: f32, blocks: Vec<Arc<Block>>) -> u32 {
    let ids: Vec<u32> = (0..n).map(|i| text_box(d, i, "", w0 + 0.1 * i as f32, 28.0)).collect();
    for w in ids.windows(2) {
        d.parts.get_mut(&w[0]).unwrap().body.next = Some(w[1]);
    }
    d.parts.get_mut(&ids[0]).unwrap().blocks = blocks;
    ids[0]
}

#[test]
fn a_chain_whose_story_shows_little_still_pays_for_its_text() {
    // All 256 boxes a hair apart in width, each showing one short visible line of a story that
    // hides about 1 MB of text between them: each layout gives some 300 lines but works through
    // the whole story. Charged by its lines alone, every width was laid out (256 times the story).
    let n = wordcraft_doc::MAX_LINKED_BOXES;
    let mut d = Document::from_text(&vec!["p"; n].join("\n"));
    let hidden = "Hidden words that never show. ".repeat(3500 / 30);
    let hide = wordcraft_doc::props::CharProps { hidden: Some(true), ..Default::default() };
    let story = (0..300)
        .map(|k| {
            let shown = format!("Line {k}.");
            let mut p = Paragraph::with_text(&format!("{shown}{hidden}"), Default::default());
            p.runs = vec![
                wordcraft_doc::para::Run { len: shown.len(), props: Default::default() },
                wordcraft_doc::para::Run { len: hidden.len(), props: hide.clone() },
            ];
            p.touch();
            para_block(p)
        })
        .collect();
    hairline_chain(&mut d, n, 300.0, story);
    // Ordinary stories of this many lines keep all 256 widths under this cap.
    let layouts = chain_layouts(&d, 100_000);
    assert!(layouts <= 8, "{layouts} layouts of a 1 MB story");
}

#[test]
fn a_chain_of_wide_boxes_pays_for_a_long_paragraph() {
    // 256 wide boxes a hair apart over one long paragraph: few lines per layout, but each
    // layout shapes all of it.
    let n = wordcraft_doc::MAX_LINKED_BOXES;
    let mut d = Document::from_text(&vec!["p"; n].join("\n"));
    let story = "Words that run on and on through every box of the chain. ".repeat(64 * 1024 / 58);
    hairline_chain(&mut d, n, 1500.0, vec![para_block(Paragraph::with_text(&story, Default::default()))]);
    let layouts = chain_layouts(&d, 2000);
    assert!(layouts <= 4, "{layouts} layouts of a 64 KB story");
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
    textbox::FLOW_WORK_CAP.with(|c| c.set(2000));
    let n = 17;
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
    assert!(widths.len() > 1 && widths.len() < n, "{} layouts: the vertical box has its own, the rest are shared", widths.len());
    // The vertical box's layout is none of the horizontal boxes', still past the cap.
    let mut all = story_layouts(&lay(&d), ids[0]);
    textbox::FLOW_WORK_CAP.with(|c| c.set(textbox::MAX_FLOW_WORK));
    let vertical = all.pop();
    assert!(vertical.is_some_and(|v| !all.contains(&v)), "the vertical box shares a horizontal layout");
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
    assert!(outlines.get(&key).is_some_and(|(_, o, _)| o.is_some()), "the outline is kept");
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
    let Some((weak, ..)) = outlines.get(&key) else { panic!("the outline is kept") };
    assert!(std::ptr::eq(weak.as_ptr(), Arc::as_ptr(&disc)), "the entry refers to its own picture's bytes");
    assert_eq!((Arc::strong_count(&disc), Arc::weak_count(&disc)), (1, 1), "a weak reference: the document still owns the bytes");
    drop(disc);
    assert!(weak.upgrade().is_none(), "the bytes are freed with the document; only their allocation is kept");
}

/// A floating Drawing Canvas in a header cell is one `Clip` holding its members, and re-placing
/// the header clones every one of them: they all count against the repeat budget.
#[test]
fn a_clipped_group_costs_its_members_when_a_header_repeats() {
    let rule = || Placed::Rule { x0: 0.0, y0: 0.0, x1: 1.0, y1: 0.0, border: Default::default() };
    let clip = |items: Vec<Placed>| Placed::Clip { rect: Rect::new(0.0, 0.0, 10.0, 10.0), spin: Default::default(), items };
    let big = usize::MAX;
    assert_eq!(placed_cost(&rule(), big), 1);
    assert_eq!(placed_cost(&clip((0..1000).map(|_| rule()).collect()), big), 1001);
    // Nested groups count too, however deep: every descendant is charged.
    assert_eq!(placed_cost(&clip(vec![clip(vec![rule(), rule()]), rule()]), big), 5);
    let mut deep = clip((0..1000).map(|_| rule()).collect());
    for _ in 0..20 {
        deep = clip(vec![deep]);
    }
    assert_eq!(placed_cost(&deep, big), 1021, "members below the eighth level still cost one each");
    // Counting stops once past the limit, so a huge or very deep group is cheap to count.
    assert_eq!(placed_cost(&deep, 10), 11);
    let mut deeper = rule();
    for _ in 0..100_000 {
        deeper = clip(vec![deeper]);
    }
    assert_eq!(placed_cost(&deeper, 50), 51);
    assert_eq!(placed_cost(&deeper, big), 100_001);
    std::mem::forget(deeper);
}

/// The linked-box work budget counts a clipped group's members, nested groups included.
#[test]
fn flow_work_counts_clip_members() {
    let rule = || Placed::Rule { x0: 0.0, y0: 0.0, x1: 1.0, y1: 0.0, border: Default::default() };
    let clip = |items: Vec<Placed>| Placed::Clip { rect: Rect::new(0.0, 0.0, 10.0, 10.0), spin: Default::default(), items };
    let mut deep = clip((0..100).map(|_| rule()).collect());
    for _ in 0..12 {
        deep = clip(vec![deep, rule()]);
    }
    assert_eq!(textbox::flow_work(&[rule(), deep]), 1 + 100 + 1 + 12 * 2);
}

/// Lines inside a clipped group (a text box on a canvas) cost each line, as they do outside one.
#[test]
fn flow_work_counts_each_line_in_a_clip() {
    let l = lay(&Document::from_text(&"Words that run on and on. ".repeat(200)));
    let lines = l.pages.iter().flat_map(|p| &p.items).find(|it| matches!(it, Placed::Lines { .. })).cloned().unwrap();
    let (l0, l1) = match &lines {
        Placed::Lines { l0, l1, .. } => (*l0, *l1),
        _ => (0, 0),
    };
    assert!(l1 - l0 > 1);
    let clip = Placed::Clip { rect: Rect::new(0.0, 0.0, 10.0, 10.0), spin: Default::default(), items: vec![lines] };
    assert_eq!(textbox::flow_work(&[clip]), 1 + (l1 - l0));
}

#[test]
fn repeated_header_items_are_capped_across_all_the_tables_of_a_layout() {
    // Each table's repeats used to have a budget of their own: a hundred tables, a hundred times
    // the cap.
    let table = || {
        let mut t = Table::new(201, 1, 468.0);
        t.rows[0].props.header = true;
        t.rows[0].props.height = Some(20.0);
        t.rows[0].props.height_rule = HeightRule::Exact;
        t.rows[0].cells[0].blocks = (0..3000).map(|_| para_block(Paragraph::with_text("x", Default::default()))).collect();
        for row in t.rows.iter_mut().skip(1) {
            row.props.height = Some(600.0);
            row.props.height_rule = HeightRule::Exact;
        }
        t
    };
    let mut d = Document::from_text("");
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(table())).unwrap();
    d.insert_block(StoryRef::Body, &Path::top(2), Block::Table(table())).unwrap();
    d.insert_block(StoryRef::Body, &Path::top(3), Block::Table(table())).unwrap();
    let l = lay(&d);
    let items: usize = l.pages.iter().map(|p| p.items.len()).sum();
    assert!(items <= MAX_HEADER_ITEMS + 3 * 3000 + 50 * l.pages.len(), "{items} items placed");
}

/// Which pictures wrap by their outline must not depend on what the cache already holds: a
/// picture's outline kept from an earlier layout is charged to the budget like a decoded one.
#[test]
fn cold_and_warm_caches_give_the_same_outlines() {
    let (a, b) = (Arc::new(disc_png(64)), Arc::new(disc_png(64)));
    let key = |bytes: &Arc<Vec<u8>>, n: &str| (n.to_string(), Arc::as_ptr(bytes) as usize, bytes.len(), [0; 4]);
    // Room for one of the two pictures.
    let pass = |outlines: &mut Outlines| {
        let mut alphas = contour::Alphas::default();
        alphas.decoded = contour::MAX_DECODED_PIXELS - 64 * 64;
        let got = [(&a, "a"), (&b, "b")].map(|(p, n)| picture_outline(outlines, &mut alphas, key(p, n), p, [0.0; 4]).is_some());
        (got, alphas.decoded)
    };
    let mut roomy = contour::Alphas::default();
    let mut warm = Outlines::new();
    for (p, n) in [(&a, "a"), (&b, "b")] {
        assert!(picture_outline(&mut warm, &mut roomy, key(p, n), p, [0.0; 4]).is_some());
    }
    assert_eq!(warm.len(), 2, "both outlines are kept");
    let (cold, cold_spent) = pass(&mut Outlines::new());
    assert_eq!(cold, [true, false], "the first picture fits, the second wraps as its rectangle");
    let (warmed, warm_spent) = pass(&mut warm);
    assert_eq!(warmed, cold, "the same pictures get outlines with the cache warm");
    assert_eq!(warm_spent, cold_spent);
    // And again: a layout after a layout.
    assert_eq!(pass(&mut warm).0, cold);
}

/// A PNG whose header says `side` × `side` RGBA and whose body is missing: it has transparency
/// to look at, costs its pixels, and then fails to decode.
fn truncated_png(side: u32) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for b in data {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend(side.to_be_bytes());
    ihdr.extend(side.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    png.extend(&ihdr);
    png.extend(crc(&ihdr).to_be_bytes());
    // An image data chunk of junk: the header is read, the pixels aren't there.
    let mut idat = b"IDAT".to_vec();
    idat.extend([0x78, 0x9c, 1, 2, 3, 4]);
    png.extend(6u32.to_be_bytes());
    png.extend(&idat);
    png.extend(crc(&idat).to_be_bytes());
    png
}

#[test]
fn a_truncated_picture_costs_the_same_cold_and_warm() {
    // Its header promises transparency, so a layout spends its pixels before the body fails to
    // decode; a layout finding its (empty) outline kept spends them too, or pictures after it
    // would get outlines only with the cache warm.
    let (big, disc) = (Arc::new(truncated_png(8000)), Arc::new(disc_png(64)));
    let key = |bytes: &Arc<Vec<u8>>, n: &str| (n.to_string(), Arc::as_ptr(bytes) as usize, bytes.len(), [0; 4]);
    let pass = |outlines: &mut Outlines| {
        let mut alphas = contour::Alphas::default();
        alphas.decoded = contour::MAX_DECODED_PIXELS - 8000 * 8000;
        let got = [(&big, "big"), (&disc, "disc")].map(|(p, n)| picture_outline(outlines, &mut alphas, key(p, n), p, [0.0; 4]).is_some());
        (got, alphas.decoded)
    };
    let mut warm = Outlines::new();
    let (cold, spent) = pass(&mut warm);
    assert_eq!(spent, contour::MAX_DECODED_PIXELS, "the truncated picture spent its pixels");
    assert_eq!(cold, [false, false], "no budget is left for the disc");
    assert_eq!(warm.get(&key(&big, "big")).map(|e| e.2), Some(8000 * 8000), "the entry holds the cost");
    // Also with a roomy first layout filling the cache, then a tight one.
    let mut roomy_cache = Outlines::new();
    let mut roomy = contour::Alphas::default();
    for (p, n) in [(&big, "big"), (&disc, "disc")] {
        let _ = picture_outline(&mut roomy_cache, &mut roomy, key(p, n), p, [0.0; 4]);
    }
    assert_eq!(pass(&mut roomy_cache), (cold, spent), "warm matches cold");
}

#[test]
fn a_header_row_holding_a_canvas_repeats_within_the_budget() {
    use wordcraft_doc::para::{Anchor, CanvasStyle, Float, GroupChild, ShapeKind, Wrap};
    // A canvas of 800 members in an exact-height header row, over two thousand pages: each page
    // would clone all 800 members.
    let member = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 4.0,
        h: 4.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float::default(),
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let canvas = InlineObject::Group {
        w: 40.0,
        h: 40.0,
        float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 5.0, y: 5.0, ..Default::default() },
        ch_w: 40.0,
        ch_h: 40.0,
        children: (0..800).map(|_| GroupChild { x: 1.0, y: 1.0, obj: member.clone() }).collect(),
        canvas: Some(CanvasStyle { fill: None, stroke: None, stroke_width: 0.0 }),
    };
    let mut t = Table::new(2001, 1, 468.0);
    t.rows[0].props.header = true;
    t.rows[0].props.height = Some(60.0);
    t.rows[0].props.height_rule = HeightRule::Exact;
    let mut p = Paragraph::with_text("", Default::default());
    p.insert_object(0, canvas, &Default::default()).unwrap();
    t.rows[0].cells[0].blocks = vec![para_block(p)];
    for row in t.rows.iter_mut().skip(1) {
        row.props.height = Some(600.0);
        row.props.height_rule = HeightRule::Exact;
    }
    fn count(it: &Placed) -> usize {
        match it {
            Placed::Clip { items, .. } => items.iter().map(count).fold(1, usize::saturating_add),
            _ => 1,
        }
    }
    let mut d = Document::from_text("");
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 1000, "{} pages", l.pages.len());
    let items: usize = l.pages.iter().flat_map(|p| &p.items).map(count).sum();
    assert!(items <= MAX_HEADER_ITEMS + 2000 + 50 * l.pages.len(), "{items} items placed");
    assert!(items > 100_000, "the header still repeats on early pages: {items}");
}

/// A table with a repeating header row in a two-column section ended by a continuous break, so
/// its columns are balanced: laid out once, then again from a snapshot for each balancing try.
fn balanced_header_table(rows: usize) -> Document {
    use wordcraft_doc::section::{SectionProps, SectionStart};
    let mut d = Document::from_text("Before\nEnd of the section\nAfter the break");
    let mut s = SectionProps::default();
    s.columns.count = 2;
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().section = Some(Box::new(s));
    d.last_section.start = SectionStart::Continuous;
    let mut t = Table::new(rows, 1, 200.0);
    t.rows[0].props.header = true;
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
    d
}

/// Header rows placed (each repeat of the one-column header row is one cell of row 0).
fn header_cells(l: &DocLayout) -> usize {
    l.pages.iter().flat_map(|p| &p.items).filter(|i| matches!(i, Placed::Cell { row: 0, .. })).count()
}

fn with_header_cap<T>(cap: usize, f: impl FnOnce() -> T) -> T {
    headers::HEADER_ITEMS_CAP.with(|c| c.set(cap));
    let out = f();
    headers::HEADER_ITEMS_CAP.with(|c| c.set(MAX_HEADER_ITEMS));
    out
}

#[test]
fn balancing_columns_gives_back_the_header_repeats_it_throws_away() {
    // The table runs over both columns of a page and onto a second one.
    let d = balanced_header_table(120);
    let full = lay(&d);
    let shown = header_cells(&full);
    assert!(shown >= 3, "the header repeats in each later column: {shown} header rows");
    // What one repeat costs: the smallest cap at which a plain two-page table repeats its header.
    let mut plain = Table::new(80, 1, 200.0);
    plain.rows[0].props.header = true;
    let mut p = Document::from_text("");
    p.insert_block(StoryRef::Body, &Path::top(1), Block::Table(plain)).unwrap();
    let cost = (1..=200).find(|cap| with_header_cap(*cap, || header_cells(&lay(&p))) >= 2).expect("a header repeat costs a few items");
    // A budget holding exactly the repeats shown is enough: the balancing tries thrown away don't
    // spend it.
    let capped = with_header_cap((shown - 1) * cost, || lay(&d));
    assert_eq!(header_cells(&capped), shown, "the final layout repeats the header as often");
}
