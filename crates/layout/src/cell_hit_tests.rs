use super::*;
use wordcraft_doc::{Pos, Table};

const GUARD_FONT: &str = "Source Serif 4";

fn guard_props() -> CharProps {
    CharProps { font: Some(GUARD_FONT.into()), size: Some(9.0), ..Default::default() }
}

fn cell_document(height: f32, text: &str) -> Document {
    let mut table = Table::new(1, 1, 200.0);
    table.props.style = None;
    table.props.cell_margins = Some([0.0; 4]);
    table.props.borders = Some(Default::default());
    table.rows[0].props.height = Some(height);
    table.rows[0].props.height_rule = HeightRule::Exact;
    let mut paragraph = Paragraph::with_text(text, guard_props());
    paragraph.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
    paragraph.props.space_after = Some(0.0);
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(paragraph)];
    let mut doc = Document::from_text("AFTER ROW");
    doc.body[0] = wordcraft_doc::para_block(Paragraph::with_text("AFTER ROW", guard_props()));
    doc.insert_block(StoryRef::Body, &Path::top(0), Block::Table(table)).unwrap();
    doc
}

fn laid(doc: &Document) -> DocLayout {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let bundled = db.face(GUARD_FONT, "Regular");
    let result = layout(doc, &mut LayoutCache::new(), &LayoutOptions::default());
    for page in &result.pages {
        for item in page.items.iter().chain(&page.header).chain(&page.footer) {
            if let Placed::Lines { para, .. } = item {
                for style in &para.styles {
                    assert!(style.face.get().data() == bundled.data(), "fixture must use the bundled open face: {}", style.face.get().family);
                }
            }
        }
    }
    result
}

fn cell_rect(page: &Page) -> Rect {
    page.items
        .iter()
        .find_map(|item| match item {
            Placed::Cell { rect, covered: false, .. } => Some(*rect),
            _ => None,
        })
        .unwrap()
}

fn cell_pos(off: usize) -> Pos {
    Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, 0]), off }
}

#[test]
fn clicks_below_cells_keep_body() {
    for (height, text) in [(19.0, "ONE\nTWO\nTHIRDHIDDEN"), (16.0, "ONE\nTWO")] {
        let doc = cell_document(height, text);
        let layout = laid(&doc);
        let rect = cell_rect(&layout.pages[0]);
        let at = layout.hit(0, rect.x + 8.0, rect.bottom() + 2.0, StoryRef::Body).unwrap();
        assert_eq!(at.path, Path::top(1), "height {height}: hidden cell line stole following text");
        assert!(layout.hit(0, rect.x + 8.0, rect.y + 2.0, StoryRef::Body).is_some_and(|p| p.path == cell_pos(0).path));
    }
}

#[test]
fn cell_caret_stays_inside_clip() {
    let doc = cell_document(16.0, "ONE\nTWO\nTHIRDHIDDEN");
    let layout = laid(&doc);
    let rect = cell_rect(&layout.pages[0]);
    let caret = layout.caret(&cell_pos(4)).unwrap();
    assert_eq!(caret.width, 0.0, "upright carets must remain usable");
    assert!(caret.height > 0.0);
    assert!(caret.top >= rect.y && caret.top + caret.height <= rect.bottom() + 0.001, "{caret:?}, {rect:?}");
    assert!(layout.caret(&cell_pos(8)).is_none(), "a fully clipped line has no caret geometry");
}

#[test]
fn cell_selection_is_clipped() {
    let doc = cell_document(16.0, "ONE\nTWO\nTHIRDHIDDEN");
    let layout = laid(&doc);
    let rect = cell_rect(&layout.pages[0]);
    let selected = layout.selection_rects(&doc, &cell_pos(0), &cell_pos(19), 0);
    assert!(selected.len() >= 2, "partially visible line retains selection geometry");
    for (_, r) in selected {
        assert!(r.x >= rect.x && r.right() <= rect.right() && r.y >= rect.y && r.bottom() <= rect.bottom() + 0.001, "{r:?}, {rect:?}");
    }
}

#[test]
fn nested_cell_hit_is_clipped() {
    let mut doc = cell_document(50.0, "ONE\nTWO\nTHIRDHIDDEN");
    let inner = doc.body[0].clone();
    let mut outer = Table::new(1, 1, 220.0);
    outer.props.style = None;
    outer.props.cell_margins = Some([0.0; 4]);
    outer.props.borders = Some(Default::default());
    outer.rows[0].props.height = Some(16.0);
    outer.rows[0].props.height_rule = HeightRule::Exact;
    outer.rows[0].cells[0].blocks = vec![inner];
    doc.body[0] = Arc::new(Block::Table(outer));
    let layout = laid(&doc);
    let rect = layout.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Cell { rect, table, .. } if table.0.len() == 1 => Some(*rect),
            _ => None,
        })
        .unwrap();
    let at = layout.hit(0, rect.x + 8.0, rect.bottom() + 2.0, StoryRef::Body).unwrap();
    assert_eq!(at.path, Path::top(1));
    let pos = Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, 0, 0, 0, 0]), off: 8 };
    assert!(layout.caret(&pos).is_none());
}

#[test]
fn story_slices_keep_own_clips() {
    let doc = cell_document(16.0, "ONE\nTWO\nTHIRDHIDDEN");
    let mut layout = laid(&doc);
    let page = &mut layout.pages[0];
    let source: Vec<_> = page
        .items
        .iter()
        .filter(|i| match i {
            Placed::Lines { path, .. } => path.0.len() > 1,
            Placed::Cell { .. } => true,
            _ => false,
        })
        .cloned()
        .collect();
    for (story, shift, header) in [(StoryRef::Part(11), -40.0, true), (StoryRef::Part(22), 650.0, false)] {
        let mut items = source.clone();
        for item in &mut items {
            match item {
                Placed::Lines { story: s, .. } | Placed::Cell { story: s, .. } => *s = story,
                _ => {}
            }
            item.translate(0.0, shift);
        }
        if header {
            page.header = items;
        } else {
            page.footer = items;
        }
    }
    let original = cell_rect(&layout.pages[0]);
    for (story, shift) in [(StoryRef::Body, 0.0), (StoryRef::Part(11), -40.0), (StoryRef::Part(22), 650.0)] {
        let pos = Pos { story, ..cell_pos(4) };
        let rect = Rect::new(original.x, original.y + shift, original.w, original.h);
        assert!(layout.hit(0, rect.x + 8.0, rect.y + 2.0, story).is_some());
        let caret = layout.caret(&pos).unwrap();
        assert!(caret.top >= rect.y && caret.top + caret.height <= rect.bottom() + 0.001, "{story:?}: {caret:?}, {rect:?}");
        if story != StoryRef::Body {
            assert!(layout.hit(0, rect.x + 8.0, rect.bottom() + 2.0, story).is_none());
        }
    }
}

#[test]
fn empty_ancestor_stays_clipped() {
    let mut doc = cell_document(16.0, "ONE\nTWO\nTHIRDHIDDEN");
    let inner = doc.body[0].clone();
    let mut outer = Table::new(1, 1, 220.0);
    outer.props.cell_margins = Some([0.0; 4]);
    outer.rows[0].props.height = Some(16.0);
    outer.rows[0].props.height_rule = HeightRule::Exact;
    outer.rows[0].cells[0].blocks = vec![inner];
    doc.body[0] = Arc::new(Block::Table(outer));
    let mut layout = laid(&doc);
    for item in &mut layout.pages[0].items {
        if let Placed::Cell { rect, table, .. } = item
            && table.0.len() == 1
        {
            rect.y += 200.0;
        }
    }
    let pos = Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, 0, 0, 0, 0]), off: 0 };
    assert!(layout.caret(&pos).is_none());
    assert!(layout.selection_rects(&doc, &pos, &Pos { off: 19, ..pos.clone() }, 0).is_empty());
    assert_ne!(layout.hit(0, 80.0, 74.0, StoryRef::Body).unwrap().path, pos.path);
}

#[test]
fn repeated_header_hits_own_row() {
    for columns in [1, 2] {
        let mut doc = cell_document(16.0, "HEAD\nPART\nHIDDEN");
        doc.last_section.columns.count = columns;
        let Block::Table(table) = Arc::make_mut(&mut doc.body[0]) else { panic!("table") };
        table.rows[0].props.header = true;
        for _ in 0..14 {
            let mut row = Table::new(1, 1, 200.0).rows.remove(0);
            row.props.height = Some(110.0);
            row.props.height_rule = HeightRule::AtLeast;
            row.cells[0].blocks = vec![wordcraft_doc::para_block(Paragraph::with_text("Body row", guard_props()))];
            table.rows.push(row);
        }
        let layout = laid(&doc);
        assert!(layout.pages.len() > 1);
        for (page_index, page) in layout.pages.iter().enumerate() {
            let rects: Vec<_> = page
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Cell { rect, row: 0, .. } => Some(*rect),
                    _ => None,
                })
                .collect();
            assert!(!rects.is_empty());
            for rect in rects {
                let hit = layout.hit(page_index, rect.x + 8.0, rect.bottom() + 2.0, StoryRef::Body).unwrap();
                assert_ne!(hit.path, cell_pos(0).path, "page {page_index}: repeated header stole next row");
            }
        }
    }
}

#[test]
fn fragment_selection_is_local() {
    for columns in [1, 2] {
        let text = vec!["Fragment"; 140].join("\n");
        let mut doc = cell_document(1000.0, &text);
        doc.last_section.columns.count = columns;
        let inner = doc.body[0].clone();
        let mut outer = Table::new(1, 1, 220.0);
        outer.props.style = None;
        outer.props.cell_margins = Some([0.0; 4]);
        outer.props.borders = Some(Default::default());
        outer.rows[0].cells[0].blocks = vec![inner];
        doc.body[0] = Arc::new(Block::Table(outer));
        let layout = laid(&doc);
        assert_eq!(layout.pages.len(), if columns == 1 { 2 } else { 1 });
        let pos = Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, 0, 0, 0, 0]), off: 0 };
        let selected = layout.selection_rects(&doc, &pos, &Pos { off: text.len(), ..pos.clone() }, 0);
        for (page_index, page) in layout.pages.iter().enumerate() {
            let rects: Vec<_> = page
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Cell { rect, table, clip: true, .. } if table.0.len() > 1 => Some(*rect),
                    _ => None,
                })
                .collect();
            assert_eq!(rects.len(), if columns == 1 { 1 } else { 2 }, "each physical fragment keeps its own clip");
            assert!(selected.iter().any(|(pi, _)| *pi == page_index));
            for (_, r) in selected.iter().filter(|(pi, _)| *pi == page_index) {
                assert!(
                    rects.iter().any(|c| r.x >= c.x && r.right() <= c.right() && r.y >= c.y && r.bottom() <= c.bottom() + 0.001),
                    "page {page_index}: {r:?}, {rects:?}"
                );
            }
        }
    }
}

#[test]
fn leading_only_line_is_skipped() {
    use kurbo::Shape;
    const TEXT: &str = "ONE\nTWO\nTHIRDHIDDEN";
    let probe = laid(&cell_document(100.0, TEXT));
    let para = probe.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { para, path, .. } if path == &cell_pos(0).path => Some(para),
            _ => None,
        })
        .unwrap();
    let third = &para.lines[2];
    let mut ink_top = f32::INFINITY;
    for cluster in &para.clusters[third.c0..third.c1] {
        let style = &para.styles[cluster.style as usize];
        let scale = style.size / style.face.get().units_per_em() as f32;
        for glyph in &para.glyphs[cluster.g0 as usize..cluster.g1 as usize] {
            let outline = style.face.get().glyph_outline(glyph.gid);
            if !outline.elements().is_empty() {
                ink_top = ink_top.min(third.baseline - style.shift - glyph.dy + outline.bounding_box().y0 as f32 * scale);
            }
        }
    }
    assert!(ink_top.is_finite() && ink_top > third.top);
    let height = (third.top + ink_top) / 2.0;
    let doc = cell_document(height, TEXT);
    let layout = laid(&doc);
    let placed = layout.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { para, path, l0, l1, .. } if path == &cell_pos(0).path => Some((para, l0, l1)),
            _ => None,
        })
        .unwrap();
    assert_eq!((*placed.1, *placed.2), (0, 3), "all source lines remain retained for paint and PDF extraction");
    assert!(placed.0.lines[2].top < height);
    assert!(ink_top > height);
    let hits = crate::hit::page_lines(&layout.pages[0], StoryRef::Body);
    assert!(!hits.iter().any(|l| l.path == &cell_pos(0).path && l.li == 2));
    let second = hits.iter().find(|l| l.path == &cell_pos(0).path && l.li == 1).unwrap();
    let rect = cell_rect(&layout.pages[0]);
    assert!((second.top - rect.y - placed.0.lines[1].top).abs() < 0.001, "public line coordinates stay intrinsic");
    assert!((second.bottom - second.top - placed.0.lines[1].height).abs() < 0.001);
    assert!(layout.caret(&cell_pos(8)).is_none(), "leading alone must not expose hidden third-line caret");
    let (next, _) = layout.vertical(&cell_pos(4), None, 1, 0).unwrap();
    assert_eq!(next.path, Path::top(1), "down must skip a leading-only line");
    assert!(layout.caret(&next).is_some());
}

fn equation_document(height: f32) -> Document {
    let mut doc = cell_document(height, "ONE\n");
    let math = wordcraft_doc::math::Math { nodes: wordcraft_doc::math::parse_linear("x/(y+z)"), ..Default::default() };
    doc.para_mut(StoryRef::Body, &cell_pos(0).path)
        .unwrap()
        .insert_object(4, wordcraft_doc::para::InlineObject::Equation { linear: "x/(y+z)".into(), display: false, math }, &guard_props())
        .unwrap();
    doc
}

fn equation_bounds(layout: &DocLayout) -> (Rect, Arc<crate::math::MathLayout>) {
    for item in &layout.pages[0].items {
        let Placed::Lines { para, x, y, l0, .. } = item else { continue };
        let Some((oi, math)) = para.maths.first() else { continue };
        let (ci, _) = para.clusters.iter().enumerate().find(|(_, c)| c.kind == crate::para::ClKind::Object(*oi)).unwrap();
        let line = para.lines.iter().find(|l| l.c0 <= ci && ci < l.c1).unwrap();
        let base = y + line.baseline - para.lines[*l0].top;
        return (Rect::new(x + line.cl_left(ci).unwrap(), base - math.ascent, math.width, math.ascent + math.descent), math.clone());
    }
    panic!("original equation must be laid out");
}

fn partial_equation() -> DocLayout {
    let probe = laid(&equation_document(100.0));
    let (bounds, _) = equation_bounds(&probe);
    laid(&equation_document(bounds.y - cell_rect(&probe.pages[0]).y + bounds.h / 2.0))
}

#[test]
fn partial_math_keeps_clip() {
    let layout = partial_equation();
    let (bounds, original) = equation_bounds(&layout);
    let cell = cell_rect(&layout.pages[0]);
    let (page, _, _, math) = layout.equation_geom(&cell_pos(4), 0).unwrap();
    assert_eq!(page, 0);
    assert!(Arc::ptr_eq(&math, &original));
    assert_eq!(layout.equation_clip(&cell_pos(4), 0), Some(cell));
    assert!(bounds.y < cell.bottom() && bounds.bottom() > cell.bottom());
    assert!(layout.equation_hit(0, bounds.x + bounds.w / 2.0, (bounds.y + cell.bottom()) / 2.0, StoryRef::Body).is_some());
    assert!(layout.equation_hit(0, bounds.x + bounds.w / 2.0, cell.bottom() + 1.0, StoryRef::Body).is_none());
}

#[test]
fn hidden_math_has_no_editor() {
    let probe = laid(&equation_document(100.0));
    let (bounds, _) = equation_bounds(&probe);
    let layout = laid(&equation_document(bounds.y - cell_rect(&probe.pages[0]).y - 1.0));
    let (bounds, _) = equation_bounds(&layout);
    let cell = cell_rect(&layout.pages[0]);
    assert!(bounds.y > cell.bottom());
    assert!(layout.equation_geom(&cell_pos(4), 0).is_none());
    assert_eq!(layout.equation_clip(&cell_pos(4), 0), Some(cell));
    assert!(layout.equation_hit(0, bounds.x + bounds.w / 2.0, cell.bottom() - 0.1, StoryRef::Body).is_none());
}

#[test]
fn whole_math_keeps_editor() {
    let mut doc = equation_document(100.0);
    let paragraph = doc.para_mut(StoryRef::Body, &cell_pos(0).path).unwrap();
    paragraph.insert_text(paragraph.len(), &format!("\n{}", "TAIL\n".repeat(20)), &guard_props()).unwrap();
    let layout = laid(&doc);
    let (bounds, original) = equation_bounds(&layout);
    let (page, x, base, math) = layout.equation_geom(&cell_pos(4), 0).unwrap();
    assert_eq!(page, 0);
    assert_eq!(x, bounds.x);
    assert!((base - bounds.y - original.ascent).abs() < 0.001);
    assert!(Arc::ptr_eq(&math, &original));
    assert_eq!(layout.equation_clip(&cell_pos(4), 0), Some(cell_rect(&layout.pages[0])));
    assert_eq!(layout.equation_hit(0, bounds.x + bounds.w / 2.0, bounds.y + bounds.h / 2.0, StoryRef::Body).unwrap().0, cell_pos(4));
    let mut unclipped = doc.clone();
    let Block::Table(table) = Arc::make_mut(&mut unclipped.body[0]) else { panic!("table") };
    table.rows[0].props.height_rule = HeightRule::AtLeast;
    let layout = laid(&unclipped);
    assert!(layout.equation_geom(&cell_pos(4), 0).is_some());
    assert!(layout.equation_clip(&cell_pos(4), 0).is_none());
}

#[test]
fn math_clip_tracks_occurrence() {
    let mut layout = partial_equation();
    let source = layout.pages[0].items.clone();
    for (story, shift, header) in [(StoryRef::Part(11), -40.0, true), (StoryRef::Part(22), 500.0, false)] {
        let mut items = source.clone();
        for item in &mut items {
            match item {
                Placed::Lines { story: s, .. } | Placed::Cell { story: s, .. } => *s = story,
                _ => {}
            }
            item.translate(0.0, shift);
        }
        if header {
            layout.pages[0].header = items;
        } else {
            layout.pages[0].footer = items;
        }
    }
    let mut second = layout.pages[0].clone();
    for item in second.header.iter_mut().chain(&mut second.footer) {
        item.translate(30.0, 20.0);
    }
    layout.pages.push(second);
    for page in 0..2 {
        for story in [StoryRef::Part(11), StoryRef::Part(22)] {
            let pos = Pos { story, ..cell_pos(4) };
            let (pi, x, _, _) = layout.equation_geom(&pos, page).unwrap();
            assert_eq!(pi, page);
            let items = if story == StoryRef::Part(11) { &layout.pages[page].header } else { &layout.pages[page].footer };
            let expected = items
                .iter()
                .find_map(|it| match it {
                    Placed::Cell { rect, .. } => Some(*rect),
                    _ => None,
                })
                .unwrap();
            assert_eq!(x, expected.x);
            assert_eq!(layout.equation_clip(&pos, page), Some(expected));
        }
    }
    assert_eq!(
        layout.equation_clip(&cell_pos(4), 1),
        Some(cell_rect(&layout.pages[0])),
        "body clip follows the occurrence selected by existing geometry lookup"
    );
}

#[test]
fn empty_math_clip_has_no_editor() {
    let mut layout = partial_equation();
    for item in &mut layout.pages[0].items {
        if let Placed::Cell { rect, .. } = item {
            rect.h = 0.0;
        }
    }
    let cell = cell_rect(&layout.pages[0]);
    assert_eq!(layout.equation_clip(&cell_pos(4), 0), Some(cell));
    assert!(layout.equation_geom(&cell_pos(4), 0).is_none());
    assert!(layout.equation_hit(0, cell.x + 1.0, cell.y, StoryRef::Body).is_none());
}

#[test]
fn exact_cell_keeps_box_fitting() {
    use wordcraft_doc::para::{Float, InlineObject, ShapeKind, Wrap};
    const TEXT: &str = "BOXFIRST\nBOXSECOND\nBOXTHIRD\nBOXFOURTH";
    for wrap in [Wrap::Inline, Wrap::InFrontOfText] {
        let mut doc = cell_document(19.0, "HOST");
        let mut text = Paragraph::with_text(TEXT, guard_props());
        text.props.space_after = Some(0.0);
        let id = doc.add_part(wordcraft_doc::PartKind::TextBox, vec![wordcraft_doc::para_block(text)]);
        doc.parts.get_mut(&id).unwrap().body.inset = Some([0; 4]);
        let shape = InlineObject::Shape {
            kind: ShapeKind::TextBox,
            w: 140.0,
            h: 60.0,
            fill: None,
            stroke: None,
            stroke_width: 0.75,
            float: Float { wrap, ..Default::default() },
            story: Some(id),
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        doc.insert_object(&cell_pos(4), shape, &guard_props()).unwrap();
        let layout = laid(&doc);
        let kept: Vec<_> = layout.pages[0]
            .items
            .iter()
            .filter_map(|item| match item {
                Placed::Lines { story: StoryRef::Part(part), l0, l1, .. } if *part == id => Some(*l0..*l1),
                _ => None,
            })
            .collect();
        assert_eq!(kept, vec![0..1], "embedded {wrap:?} text retains complete-line fitting");
    }
}
