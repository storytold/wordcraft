//! Synthetic letterhead geometry. No external documents or image assets are used.
use super::*;
use wordcraft_doc::para::{Anchor, Float, Wrap};
use wordcraft_doc::props::VAlign;
use wordcraft_doc::{Paragraph, PartKind};

fn picture(name: &str, w: f32, h: f32, x: f32, y: f32, wrap: Wrap, dist: f32) -> InlineObject {
    InlineObject::Image {
        media: name.into(),
        w,
        h,
        alt: name.into(),
        float: Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x, y, dist, ..Default::default() },
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    }
}

fn cell_rect(items: &[Placed]) -> Rect {
    items.iter().find_map(|p| if let Placed::Cell { rect, .. } = p { Some(*rect) } else { None }).unwrap()
}

/// A body-table letterhead: a central front-of-text box and two pictures close to the cell's
/// outer edges. Both picture boxes fit; neither their offsets nor the box may move inward.
#[test]
fn cell_letterhead_keeps_picture_offsets_inside_the_outer_edges() {
    let mut d = Document::from_text("Synthetic body");
    d.last_section.page_w = 612.0;
    d.last_section.page_h = 792.0;
    d.last_section.margin_left = 70.9;
    d.last_section.margin_right = 31.5;
    d.last_section.margin_top = 31.2;
    let mut t = Table::new(1, 1, 453.6);
    t.props.style = None;
    t.props.width = Some(453.6);
    t.props.indent = Some(21.05);
    let border = Some(Border { style: BorderStyle::Single, width: 0.5, ..Default::default() });
    t.props.borders = Some(Borders { top: border, left: border, bottom: border, right: border, ..Default::default() });
    t.rows[0].props.height = Some(63.75);
    let mut p = Paragraph::default();
    let box_story = d.add_part(PartKind::TextBox, vec![wordcraft_doc::para_block(Paragraph::with_text("Synthetic letterhead", Default::default()))]);
    let mut central = rect_shape(332.25, 52.5, Float { wrap: Wrap::InFrontOfText, x: 59.9, y: 5.8, dist: 9.0, ..Default::default() });
    if let InlineObject::Shape { story, .. } = &mut central {
        *story = Some(box_story);
    }
    p.insert_object(0, central, &Default::default()).unwrap();
    p.insert_object(0, picture("right", 47.25, 61.6, 395.95, 1.4, Wrap::Through, 9.0), &Default::default()).unwrap();
    p.insert_object(0, picture("left", 63.75, 56.25, -5.15, 2.0, Wrap::Through, 9.0), &Default::default()).unwrap();
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(p)];
    d.insert_block(StoryRef::Body, &Path::top(0), Block::Table(t)).unwrap();
    let layout = lay(&d);
    let items = &layout.pages[0].items;
    let cell = cell_rect(items);
    let mut checked = 0;
    for item in items {
        if let Placed::Object { rect, origin, text_box, .. } = item {
            let (x, y) = if text_box.is_some() {
                (59.9, 5.8)
            } else if rect.w > 60.0 {
                (-5.15, 2.0)
            } else {
                (395.95, 1.4)
            };
            assert!(rect.x >= cell.x - 0.01 && rect.right() <= cell.right() + 0.01, "the original box fits the outer cell: {rect:?}, {cell:?}");
            assert!((rect.x - origin.x - x).abs() < 0.5, "offset {x} was moved: {rect:?}, origin {origin:?}");
            assert!((rect.y - origin.y - y).abs() < 0.01, "vertical placement is unchanged");
            checked += 1;
        }
    }
    assert_eq!(checked, 3, "two pictures and one central text box");
}

/// A centred header table wider than the text column, with a picture anchored far to its left.
/// The picture touches the outer cell edge, not the text inset or the wrap-distance boundary.
#[test]
fn cell_letterhead_pulls_an_outside_picture_to_the_outer_left_edge() {
    for dist in [0.0, 9.0, 24.0] {
        let mut d = Document::from_text("Synthetic body");
        d.last_section.page_w = 612.0;
        d.last_section.page_h = 792.0;
        d.last_section.margin_left = 70.85;
        d.last_section.margin_right = 70.85;
        d.last_section.header = 5.65;
        let mut t = Table::new(1, 1, 498.05);
        t.props.style = None;
        t.props.width_pct = Some(105.9);
        t.props.align = Some(Align::Center);
        t.rows[0].props.height = Some(67.95);
        t.rows[0].cells[0].props.valign = Some(VAlign::Center);
        let mut p = Paragraph::with_text("Synthetic header", Default::default());
        p.insert_object(0, picture("logo", 79.0, 47.55, -94.7, 2.3, Wrap::BehindText, dist), &Default::default()).unwrap();
        t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(p)];
        let header = d.add_part(PartKind::Header, vec![Arc::new(Block::Table(t))]);
        d.last_section.headers.default = Some(header);
        let layout = lay(&d);
        let items = &layout.pages[0].header;
        let cell = cell_rect(items);
        assert!((cell.x - 56.976).abs() < 0.5, "the centred outer cell begins at {cell:?}");
        let objects: Vec<_> =
            items.iter().filter_map(|p| if let Placed::Object { rect, origin, .. } = p { Some((*rect, *origin)) } else { None }).collect();
        assert_eq!(objects.len(), 1);
        let (rect, origin) = objects[0];
        assert!((rect.x - cell.x).abs() < 0.5, "wrap distance {dist}: picture {rect:?}, outer cell {cell:?}");
        assert!((rect.y - origin.y - 2.3).abs() < 0.01, "vertical placement is unchanged");
    }
}

/// Confinement follows the particular cell, including asymmetric margins mirrored for RTL.
#[test]
fn cell_float_outer_edges_include_asymmetric_and_mirrored_margins() {
    for rtl in [false, true] {
        for (offset, at_left) in [(-100.0, true), (300.0, false)] {
            let mut d = Document::from_text("Body");
            let mut t = Table::new(1, 2, 468.0);
            t.props.bidi_visual = rtl;
            t.props.cell_margins = Some([0.0, 12.0, 0.0, 3.0]);
            let mut p = Paragraph::default();
            p.insert_object(0, picture("logo", 40.0, 20.0, offset, 2.0, Wrap::BehindText, 9.0), &Default::default()).unwrap();
            t.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(p)];
            d.insert_block(StoryRef::Body, &Path::top(0), Block::Table(t)).unwrap();
            let layout = lay(&d);
            let items = &layout.pages[0].items;
            let cell = items.iter().find_map(|p| if let Placed::Cell { rect, cell: 1, .. } = p { Some(*rect) } else { None }).unwrap();
            let picture = items.iter().find_map(|p| if let Placed::Image { rect, .. } = p { Some(*rect) } else { None }).unwrap();
            let edge = if at_left { picture.x - cell.x } else { picture.right() - cell.right() };
            assert!(edge.abs() < 0.5, "rtl {rtl}, offset {offset}: picture {picture:?}, cell {cell:?}");
        }
    }
}
