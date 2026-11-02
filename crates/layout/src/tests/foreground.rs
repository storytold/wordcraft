use super::*;
use crate::foreground::{Foreground, order_foreground};

#[test]
fn foreground_survives_later_text() {
    use wordcraft_doc::para::{Anchor, Float, ShapeKind};
    for wrap in [Wrap::InFrontOfText, Wrap::Square, Wrap::BehindText] {
        let mut d = Document::from_text("\nLater body text");
        d.insert_object(
            &Pos::body(1, 0),
            InlineObject::Image {
                media: "original-layer-probe".into(),
                w: 120.0,
                h: 80.0,
                alt: String::new(),
                float: Default::default(),
                crop: [0.0; 4],
                ole: None,
            },
            &Default::default(),
        )
        .unwrap();
        let id = text_box(
            &mut d,
            0,
            "Foreground label",
            180.0,
            80.0,
            Float { wrap, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 90.0, ..Default::default() },
        );
        let l = lay(&d);
        let items = &l.pages[0].items;
        let later = items.iter().position(|it| matches!(it, Placed::Lines { story: StoryRef::Body, path, .. } if path.0 == [1])).unwrap();
        let frame = items.iter().position(|it| matches!(it, Placed::Shape { kind: ShapeKind::TextBox, .. })).unwrap();
        let label = items.iter().position(|it| matches!(it, Placed::Lines { story: StoryRef::Part(part), .. } if *part == id)).unwrap();
        if wrap == Wrap::BehindText {
            assert!(frame < label && label < later, "behind-text box stays below later content");
        } else {
            assert!(later < frame && frame < label, "foreground frame and label stay above later content: {wrap:?}");
        }
        assert_eq!(l.index[&(StoryRef::Body, Path::top(1))][0].1, later);
        let rect = box_rect(&l, id);
        assert!((rect.x - 72.0).abs() < 0.01 && (rect.y - 90.0).abs() < 0.01, "paint order must not change the anchor");
    }
}

#[test]
fn foreground_keeps_z_order() {
    use wordcraft_doc::para::{Anchor, Float};
    let mut d = Document::from_text("\nLater content");
    let float = |z| Float { wrap: Wrap::InFrontOfText, z, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 90.0, ..Default::default() };
    let low = text_box(&mut d, 0, "Lower", 180.0, 80.0, float(2));
    // Inserted at the same offset: the higher layer precedes the lower one in text order.
    let high = text_box(&mut d, 0, "Higher", 180.0, 80.0, float(7));
    let behind = text_box(&mut d, 0, "Behind", 180.0, 80.0, Float { wrap: Wrap::BehindText, ..float(9) });
    let l = lay(&d);
    let layers: Vec<_> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { text_box: Some(id), .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(layers, [behind, low, high]);
    for id in [low, high] {
        let marker = l.pages[0].items.iter().position(|it| matches!(it, Placed::Object { text_box: Some(part), .. } if *part == id)).unwrap();
        assert!(matches!(&l.pages[0].items[marker + 1], Placed::Lines { story: StoryRef::Part(part), .. } if *part == id));
    }
}

#[test]
fn foreground_in_header_story() {
    use wordcraft_doc::para::{Anchor, Float};
    let mut d = Document::from_text("\nLater header text");
    let id = text_box(
        &mut d,
        0,
        "Header overlay",
        180.0,
        80.0,
        Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 30.0, ..Default::default() },
    );
    let blocks = std::mem::take(&mut d.body);
    let header = d.add_part(wordcraft_doc::PartKind::Header, blocks);
    d.last_section.headers.default = Some(header);
    d.body = Document::from_text("Body").body;
    let l = lay(&d);
    let items = &l.pages[0].header;
    let later = items
        .iter()
        .position(|it| matches!(it, Placed::Lines { story: StoryRef::Part(part), path, .. } if *part == header && path.0 == [1]))
        .unwrap();
    let overlay = items.iter().position(|it| matches!(it, Placed::Object { text_box: Some(part), .. } if *part == id)).unwrap();
    assert!(later < overlay);
}

#[test]
fn foreground_respects_stories() {
    use wordcraft_doc::para::{Anchor, Float, ShapeKind};
    use wordcraft_doc::props::Rgb;
    let mut d = Document::from_text("");
    let header = d.add_part(wordcraft_doc::PartKind::Header, Document::from_text("").body);
    d.last_section.headers.default = Some(header);
    let logo = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 120.0,
        h: 80.0,
        fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(200, 30, 30))),
        stroke: None,
        stroke_width: 0.0,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
        float: Float { wrap: Wrap::InFrontOfText, z: u32::MAX, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 72.0, ..Default::default() },
    };
    d.insert_object(&Pos { story: StoryRef::Part(header), path: Path::top(0), off: 0 }, logo, &Default::default()).unwrap();
    let mask = InlineObject::Image {
        media: "original-body-mask".into(),
        w: 160.0,
        h: 100.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    d.insert_object(&Pos::body(0, 0), mask, &Default::default()).unwrap();
    let l = lay(&d);
    let draws = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    let logo = draws
        .iter()
        .position(|it| matches!(it, display::Draw::Shape { fill: Some(display::Paint::Solid(Rgb(200, 30, 30), alpha)), .. } if *alpha == 1.0))
        .unwrap();
    let mask = draws.iter().position(|it| matches!(it, display::Draw::Image { media, .. } if media == "original-body-mask")).unwrap();
    assert!(logo < mask, "header foreground remains below body masks, even at maximum relativeHeight");
}

#[test]
fn foreground_across_cells() {
    use wordcraft_doc::para::{Anchor, Float};
    let mut d = Document::from_text("Cell high anchor\nCell mid anchor\nBody anchor");
    let at = |z, x| Float { wrap: Wrap::InFrontOfText, z, h_rel: Anchor::Page, v_rel: Anchor::Page, x, y: 0.0, ..Default::default() };
    let high = text_box_at(&mut d, &Pos::body(0, 0), "Cell high", 100.0, 80.0, at(7, 0.0));
    let mid = text_box_at(&mut d, &Pos::body(1, 0), "Cell mid", 100.0, 80.0, at(3, -200.0));
    let low = text_box_at(&mut d, &Pos::body(2, 0), "Body low", 100.0, 80.0, Float { y: 72.0, ..at(1, 72.0) });
    let cells: Vec<_> = d.body.drain(..2).collect();
    let mut table = Table::new(1, 2, 400.0);
    table.rows[0].cells[0].blocks = vec![cells[0].clone()];
    table.rows[0].cells[1].blocks = vec![cells[1].clone()];
    d.body.insert(0, Arc::new(Block::Table(table)));
    let l = lay(&d);
    let layers: Vec<_> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { text_box: Some(id), .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert!(box_rect(&l, high).intersects(&box_rect(&l, low)));
    assert!(box_rect(&l, high).intersects(&box_rect(&l, mid)));
    assert_eq!(layers, [low, mid, high], "table cells share their owning body's foreground stack");
}

#[test]
fn split_row_keeps_foreground() {
    use crate::table::{CellPart, RowLayout, split_row};
    let l = lay(&Document::from_text("Line"));
    let mut line = l.pages[0].items.iter().find(|it| matches!(it, Placed::Lines { .. })).unwrap().clone();
    let Placed::Lines { y, .. } = &line else { panic!("line") };
    line.translate(0.0, -*y);
    let mut second = line.clone();
    second.translate(0.0, 150.0);
    let row = RowLayout {
        height: 200.0,
        items: vec![line, second, Placed::Fill { rect: Rect::new(0.0, 150.0, 100.0, 20.0), color: Rgb(200, 0, 0) }],
        foreground: vec![Foreground { range: 1..3, z: 7, floor: 7 }],
        bottom: 0.0,
        edge: 0.0,
        cells: vec![CellPart { top: 0.0, items: 0..3, end: Some(170.0), align: None }],
    };
    let (a, b) = split_row(&row, 100.0).unwrap();
    assert!(a.foreground.is_empty());
    assert_eq!(b.foreground.len(), 1);
    assert_eq!((b.foreground[0].range.clone(), b.foreground[0].z), (0..2, 7));
    assert_eq!(b.foreground[0].floor, 7);
}

#[test]
fn pinned_cells_keep_z_order() {
    use wordcraft_doc::para::{Anchor, Float};
    for kind in [wordcraft_doc::PartKind::Header, wordcraft_doc::PartKind::Footer] {
        let mut d = Document::from_text("High cell\nLow cell");
        let float = |z| Float {
            wrap: Wrap::InFrontOfText,
            z,
            h_rel: Anchor::Page,
            v_rel: Anchor::Page,
            x: 90.0,
            y: 100.0,
            outside_cell: true,
            ..Default::default()
        };
        let high = text_box_at(&mut d, &Pos::body(0, 0), "High", 100.0, 60.0, float(8));
        let low = text_box_at(&mut d, &Pos::body(1, 0), "Low", 100.0, 60.0, float(2));
        let cells = std::mem::take(&mut d.body);
        let mut table = Table::new(1, 2, 400.0);
        table.rows[0].cells[0].blocks = vec![cells[0].clone()];
        table.rows[0].cells[1].blocks = vec![cells[1].clone()];
        let id = d.add_part(kind, vec![Arc::new(Block::Table(table))]);
        if kind == wordcraft_doc::PartKind::Header {
            d.last_section.headers.default = Some(id);
        } else {
            d.last_section.footers.default = Some(id);
        }
        d.body = Document::from_text(&"Body text. ".repeat(600)).body;
        let l = lay(&d);
        assert!(l.pages.len() > 1);
        for page in &l.pages {
            let items = if kind == wordcraft_doc::PartKind::Header { &page.header } else { &page.footer };
            let layers: Vec<_> = items
                .iter()
                .filter_map(|it| match it {
                    Placed::Object { text_box: Some(part), rect, .. } if *part == low || *part == high => {
                        assert!((rect.x - 90.0).abs() < 0.01 && (rect.y - 100.0).abs() < 0.01);
                        Some(*part)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(layers, [low, high]);
            for part in [low, high] {
                let at = items.iter().position(|it| matches!(it, Placed::Object { text_box: Some(p), .. } if *p == part)).unwrap();
                assert!(matches!(&items[at + 1], Placed::Lines { story: StoryRef::Part(p), .. } if *p == part));
            }
        }
    }
}

#[test]
fn unranked_follows_known_rank() {
    use wordcraft_doc::para::{Anchor, Float};
    let mut d = Document::from_text("First\nSecond\nThird");
    let float = |z| Float { z, wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 90.0, ..Default::default() };
    let high = text_box_at(&mut d, &Pos::body(0, 0), "High", 80.0, 40.0, float(7));
    let unknown = text_box_at(&mut d, &Pos::body(1, 0), "Unranked", 80.0, 40.0, float(0));
    let low = text_box_at(&mut d, &Pos::body(2, 0), "Low", 80.0, 40.0, float(2));
    let layout = lay(&d);
    let order: Vec<_> = layout.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { text_box: Some(id), .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(order, [low, high, unknown]);
}

#[test]
fn behind_rank_sets_fallback() {
    use wordcraft_doc::para::{Anchor, Float};
    let mut d = Document::from_text("Anchor");
    let float = |z, wrap| Float { z, wrap, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 90.0, ..Default::default() };
    // The helper inserts at offset zero, so insert in reverse anchor order.
    let low = text_box(&mut d, 0, "Low", 80.0, 40.0, float(2, Wrap::InFrontOfText));
    let unknown = text_box(&mut d, 0, "Unranked", 80.0, 40.0, float(0, Wrap::InFrontOfText));
    let back = text_box(&mut d, 0, "Behind", 80.0, 40.0, float(7, Wrap::BehindText));
    let layout = lay(&d);
    let order: Vec<_> = layout.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { text_box: Some(id), .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(order, [back, low, unknown]);
}
