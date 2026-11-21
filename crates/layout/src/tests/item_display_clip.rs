//! A warped text box whose lines are cut off by a clipped table cell: [`ItemDisplay`] draws them
//! flat in the clip, as [`page_display`] does (the clipped-cell branch of `ItemDisplay::warp_of`),
//! and the box's text after the cell bent, where its first line outside the cell is.
use crate::display::{Draw, ItemDisplay, page_display};
use crate::*;
use wordcraft_doc::props::{HeightRule, VAlign};
use wordcraft_doc::{Block, PartKind, Pos, StoryRef, Table, TextBody, para_block};

/// A body paragraph holding an arched WordArt box whose text sits in a one-cell table with an
/// exact-height row too short for it, followed by paragraphs `after`; the document and the box's
/// story.
fn clipped_wordart(after: &[&str]) -> (Document, u32) {
    let mut d = Document::from_text("q");
    let props = CharProps {
        font: Some("Source Sans 3".into()),
        size: Some(30.0),
        text_effects: Some(Box::new(wordcraft_doc::wordart::TextEffects {
            fill: Some(wordcraft_doc::wordart::TextFill::Solid { color: wordcraft_doc::Rgb::BLACK, transparency: 0.0 }),
            ..Default::default()
        })),
        ..CharProps::default()
    };
    let mut t = Table::new(1, 1, 242.25);
    t.rows[0].props.height = Some(10.0);
    t.rows[0].props.height_rule = HeightRule::Exact;
    t.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("Your Text Here", props.clone()))];
    let mut blocks = vec![std::sync::Arc::new(Block::Table(t))];
    blocks.extend(after.iter().map(|text| para_block(Paragraph::with_text(text, props.clone()))));
    let id = d.add_part(PartKind::TextBox, blocks);
    if let Some(p) = d.parts.get_mut(&id) {
        p.body = TextBody { anchor: VAlign::Center, inset: Some([0; 4]), ..Default::default() };
    }
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 242.25,
        h: if after.is_empty() { 41.25 } else { 120.0 },
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: wordcraft_doc::connector::ShapeExtra { warp: wordcraft_doc::wordart::TextWarp::new("textArchUp"), ..Default::default() },
    };
    d.insert_object(&Pos::body(0, 1), shape, &Default::default()).unwrap();
    (d, id)
}

/// The box text's glyph runs, with those drawn in a clip, in order.
fn box_glyphs(draws: &[Draw]) -> Vec<String> {
    fn walk(ds: &[Draw], clipped: bool, out: &mut Vec<String>) {
        for dr in ds {
            match dr {
                Draw::Glyphs { text, glyphs, .. } if ["Your", "Text", "Here"].iter().any(|w| text.contains(w)) => {
                    out.push(format!("{clipped} {text} {glyphs:?}"));
                }
                Draw::Clip { items, .. } => walk(items, true, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(draws, false, &mut out);
    out
}

#[test]
fn a_warped_box_in_a_clipped_cell_draws_flat_in_the_clip_as_page_display() {
    let (d, id) = clipped_wordart(&[]);
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let page = &l.pages[0];
    let ours = |s: &StoryRef| *s == StoryRef::Part(id);
    assert!(page.items.iter().any(|it| matches!(it, Placed::Cell { story, clip: true, .. } if ours(story))), "the cell clips");
    assert!(page.items.iter().any(|it| matches!(it, Placed::Lines { story, .. } if ours(story))), "the box's lines are laid out");
    let opts = Default::default();
    let whole = page_display(&d, page, &opts);
    let shown = ItemDisplay::new(&d, page);
    let draws: Vec<Draw> = page.header.iter().chain(&page.footer).chain(&page.items).flat_map(|it| shown.draws(it, &opts)).collect();
    assert!(!whole.iter().any(|dr| matches!(dr, Draw::Art { .. })), "the page draws the clipped text flat");
    assert!(!draws.iter().any(|dr| matches!(dr, Draw::Art { .. })), "so does the item-by-item path");
    let mine = box_glyphs(&draws);
    assert!(!mine.is_empty() && mine.iter().all(|g| g.starts_with("true")), "drawn in the cell's clip: {mine:?}");
    assert_eq!(mine, box_glyphs(&whole));
}

#[test]
fn text_after_a_clipped_cell_in_a_warped_box_is_drawn_bent_as_page_display() {
    let (d, id) = clipped_wordart(&["Below"]);
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let page = &l.pages[0];
    let ours = |s: &StoryRef| *s == StoryRef::Part(id);
    let lines: Vec<&Placed> = page.items.iter().filter(|it| matches!(it, Placed::Lines { story, .. } if ours(story))).collect();
    assert!(lines.len() >= 2, "the cell's line and the paragraph after the table are laid out: {}", lines.len());
    let opts = Default::default();
    let whole = page_display(&d, page, &opts);
    let shown = ItemDisplay::new(&d, page);
    let draws: Vec<Draw> = page.header.iter().chain(&page.footer).chain(&page.items).flat_map(|it| shown.draws(it, &opts)).collect();
    assert!(whole.iter().any(|dr| matches!(dr, Draw::Art { .. })), "the page draws the box's text bent after the clipped cell");
    super::item_display_warp::same_art(&draws, &whole).unwrap();
    let mine = box_glyphs(&draws);
    assert!(mine.iter().any(|g| g.starts_with("true")), "the cell's line is drawn flat in its clip: {mine:?}");
    assert_eq!(mine, box_glyphs(&whole));
}
