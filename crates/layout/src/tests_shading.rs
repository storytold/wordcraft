use super::*;
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing};
use wordcraft_doc::{Paragraph, Rgb, Table, para_block};

const SHADE: Rgb = Rgb(45, 105, 165);
const PAGE_HEIGHT: f32 = 200.0;
const PAGE_MARGIN: f32 = 20.0;
const LINE_HEIGHT: f32 = 20.0;
const TEXT_WIDTH: f32 = 168.0;

#[derive(Clone, Copy)]
enum ShadeFlow {
    Whole,
    Partial,
    Widow,
}

#[derive(Clone, Copy)]
enum ShadeKind {
    Paragraph,
    Cell,
    Both,
}

fn shade_doc(flow: ShadeFlow, kind: ShadeKind) -> Document {
    let mut d = Document::new();
    d.last_section.page_h = PAGE_HEIGHT;
    d.last_section.page_w = 400.0;
    d.last_section.margin_top = PAGE_MARGIN;
    d.last_section.margin_bottom = PAGE_MARGIN;
    d.last_section.margin_left = PAGE_MARGIN;
    d.last_section.margin_right = PAGE_MARGIN;
    let para = |text: &str, after, shaded| {
        let mut p =
            Paragraph::with_text(text, wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() });
        p.props.space_before = Some(0.0);
        p.props.space_after = Some(after);
        p.props.line_spacing = Some(LineSpacing::Exactly(LINE_HEIGHT));
        p.props.widow_control = Some(false);
        p.props.shading = shaded;
        para_block(p)
    };
    let mut t = Table::new(1, 2, 360.0);
    t.props.style = None;
    t.props.cell_margins = Some([4.0, 6.0, 0.0, 6.0]);
    let none = Some(Border { style: BorderStyle::None, ..Default::default() });
    t.props.borders = Some(Borders { top: none, bottom: none, left: none, right: none, between: none, inside_v: none });
    for (index, cell) in t.rows[0].cells.iter_mut().enumerate() {
        cell.props.shading = matches!(kind, ShadeKind::Cell | ShadeKind::Both).then_some(SHADE);
        let shade = (index == 0 && matches!(kind, ShadeKind::Paragraph | ShadeKind::Both)).then_some(SHADE);
        cell.blocks = match flow {
            ShadeFlow::Whole => vec![para("Kept", 0.0, None), para("Moved shade", 130.0, shade)],
            ShadeFlow::Widow => {
                let mut kept = para(&["Kept shade"; 2].join("\n"), 0.0, shade);
                let Block::Para(p) = std::sync::Arc::make_mut(&mut kept) else { panic!("expected original paragraph") };
                p.props.widow_control = Some(true);
                vec![para(&["Kept"; 6].join("\n"), 0.0, None), kept]
            }
            ShadeFlow::Partial => vec![para(&vec!["Shade"; 20].join("\n"), 0.0, shade)],
        };
    }
    d.body = vec![std::sync::Arc::new(Block::Table(t))];
    d
}

fn shade_layout(d: &Document) -> DocLayout {
    layout(d, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn same_rect(a: Rect, b: Rect) -> bool {
    [(a.x, b.x), (a.y, b.y), (a.w, b.w), (a.h, b.h)].iter().all(|(a, b)| (a - b).abs() < 0.01)
}

fn assert_paragraph_fill(l: &DocLayout) {
    let mut fragments = 0;
    for (page, p) in l.pages.iter().enumerate() {
        for item in &p.items {
            let Placed::Lines { para, l0, l1, x, y, path, .. } = item else { continue };
            if para.rp.shading != Some(SHADE) {
                continue;
            }
            fragments += 1;
            let h = para.lines[*l1 - 1].top + para.lines[*l1 - 1].height - para.lines[*l0].top;
            let expected = Rect::new(*x, *y, TEXT_WIDTH, h);
            let fills: Vec<_> = p
                .items
                .iter()
                .filter_map(|it| match it {
                    Placed::Fill { rect, color } if *color == SHADE => Some(*rect),
                    _ => None,
                })
                .collect();
            eprintln!("SHADE_MODEL page={page} path={path:?} lines={l0}..{l1} expected={expected:?} fills={fills:?}");
            assert_eq!(
                fills.iter().filter(|r| same_rect(**r, expected)).count(),
                1,
                "paragraph shading must follow its exact physical line fragment: page={page} path={path:?}"
            );
        }
    }
    assert!(fragments > 0);
}

#[test]
fn shading_moves_with_after_space() {
    let d = shade_doc(ShadeFlow::Whole, ShadeKind::Paragraph);
    let l = shade_layout(&d);
    assert_eq!(l.pages.len(), 2);
    assert!(!l.pages[0].items.iter().any(|it| matches!(it,Placed::Lines {para,..} if para.rp.shading==Some(SHADE))));
    assert!(l.pages[1].items.iter().any(|it| matches!(it,Placed::Lines {para,..} if para.rp.shading==Some(SHADE))));
    assert_paragraph_fill(&l);
    assert!(
        !l.pages[0].items.iter().any(|it| matches!(it,Placed::Fill {color,..} if *color==SHADE)),
        "moved paragraph leaves no shade on first page"
    );
}

#[test]
fn shading_tracks_partial_lines() {
    let l = shade_layout(&shade_doc(ShadeFlow::Partial, ShadeKind::Paragraph));
    assert!(l.pages.len() > 1);
    assert_paragraph_fill(&l);
    let mut next = 0;
    for item in l.pages.iter().flat_map(|page| &page.items) {
        if let Placed::Lines { para, l0, l1, .. } = item
            && para.rp.shading == Some(SHADE)
        {
            assert_eq!(*l0, next, "shaded ranges stay consecutive");
            assert!(l1 > l0);
            next = *l1;
        }
    }
    assert_eq!(next, 20);
}

#[test]
fn shading_keeps_cell_background() {
    for flow in [ShadeFlow::Whole, ShadeFlow::Partial] {
        let l = shade_layout(&shade_doc(flow, ShadeKind::Cell));
        assert!(l.pages.len() > 1);
        for (page, p) in l.pages.iter().enumerate() {
            let cells: Vec<_> = p
                .items
                .iter()
                .filter_map(|it| match it {
                    Placed::Cell { rect, .. } => Some(*rect),
                    _ => None,
                })
                .collect();
            let fills: Vec<_> = p
                .items
                .iter()
                .filter_map(|it| match it {
                    Placed::Fill { rect, color } if *color == SHADE => Some(*rect),
                    _ => None,
                })
                .collect();
            assert_eq!(fills.len(), cells.len(), "page={page}");
            for r in cells {
                assert_eq!(fills.iter().filter(|f| same_rect(**f, r)).count(), 1, "cell fill remains its cell frame: page={page} {r:?}");
            }
        }
    }
}

#[test]
fn shading_same_color_owners() {
    for flow in [ShadeFlow::Whole, ShadeFlow::Partial] {
        let l = shade_layout(&shade_doc(flow, ShadeKind::Both));
        assert_paragraph_fill(&l);
    }
}

#[test]
fn shading_moves_with_widow() {
    let l = shade_layout(&shade_doc(ShadeFlow::Widow, ShadeKind::Paragraph));
    assert_eq!(l.pages.len(), 2);
    assert!(!l.pages[0].items.iter().any(|it| matches!(it,Placed::Lines {para,..} if para.rp.shading==Some(SHADE))));
    assert!(l.pages[1].items.iter().any(|it| matches!(it,Placed::Lines {para,..} if para.rp.shading==Some(SHADE))));
    assert_paragraph_fill(&l);
    assert!(!l.pages[0].items.iter().any(|it| matches!(it,Placed::Fill {color,..} if *color==SHADE)));
}

#[test]
fn shading_fitting_stays_whole() {
    for flow in [ShadeFlow::Whole, ShadeFlow::Partial, ShadeFlow::Widow] {
        for kind in [ShadeKind::Paragraph, ShadeKind::Both] {
            let mut d = shade_doc(flow, kind);
            d.last_section.page_h = 1000.0;
            let l = shade_layout(&d);
            assert_eq!(l.pages.len(), 1);
            assert_paragraph_fill(&l);
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ShadeFallback {
    Nested,
    Turned,
    Clipped,
    Merged,
    Header,
}

fn fallback_doc(kind: ShadeFallback) -> Document {
    use wordcraft_doc::props::{HeightRule, TextDirection, VMerge};
    let mut d = shade_doc(ShadeFlow::Partial, ShadeKind::Both);
    if matches!(kind, ShadeFallback::Header) {
        let id = d.add_part(wordcraft_doc::PartKind::Header, d.body.clone());
        d.last_section.headers.default = Some(id);
        d.body = vec![para_block(Paragraph::with_text("Body", Default::default()))];
        return d;
    }
    let Block::Table(table) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!() };
    match kind {
        ShadeFallback::Nested => {
            let mut outer = Table::new(1, 1, 360.0);
            outer.props.style = None;
            outer.rows[0].cells[0].blocks = vec![std::sync::Arc::new(Block::Table(table.clone()))];
            *table = outer;
        }
        ShadeFallback::Turned => {
            for cell in &mut table.rows[0].cells {
                cell.props.text_direction = TextDirection::Down;
            }
        }
        ShadeFallback::Clipped => {
            table.rows[0].props.height = Some(60.0);
            table.rows[0].props.height_rule = HeightRule::Exact;
        }
        ShadeFallback::Merged => {
            table.rows.push(table.rows[0].clone());
            for cell in &mut table.rows[0].cells {
                cell.props.vmerge = VMerge::Restart;
            }
            for cell in &mut table.rows[1].cells {
                cell.props.vmerge = VMerge::Continue;
            }
        }
        ShadeFallback::Header => panic!("header returned before body layout"),
    }
    d
}

fn shade_snapshot(item: &Placed) -> serde_json::Value {
    use serde_json::json;
    let rect = |r: &Rect| [r.x, r.y, r.w, r.h];
    match item {
        Placed::Fill { rect: r, color } => json!(["Fill", rect(r), color]),
        Placed::Lines { story, path, para, l0, l1, x, y, turn } => json!([
            "Lines",
            format!("{story:?}"),
            path.0,
            l0,
            l1,
            x,
            y,
            format!("{turn:?}"),
            para.text_len,
            para.rp.shading,
            para.lines[*l0..*l1].iter().map(|line| [line.top, line.height, line.right]).collect::<Vec<_>>()
        ]),
        Placed::Cell { rect: r, table, row, cell, story, covered, clip } => {
            json!(["Cell", rect(r), table.0, row, cell, format!("{story:?}"), covered, clip])
        }
        Placed::Rule { x0, y0, x1, y1, border } => json!(["Rule", x0, y0, x1, y1, format!("{border:?}")]),
        _ => panic!("unrecorded fallback item: {item:?}"),
    }
}

#[test]
fn shading_main_fallback_models() {
    for kind in [ShadeFallback::Nested, ShadeFallback::Turned, ShadeFallback::Clipped, ShadeFallback::Merged, ShadeFallback::Header] {
        let d = fallback_doc(kind);
        let l = shade_layout(&d);
        assert!(!l.pages.is_empty());
        let items: Vec<_> = l.pages.iter().flat_map(|page| page.items.iter().chain(&page.header)).collect();
        match kind {
            ShadeFallback::Nested => assert!(items.iter().any(|it| matches!(it, Placed::Cell { table, .. } if table.0.len() > 1))),
            ShadeFallback::Turned => assert!(items.iter().any(|it| matches!(it, Placed::Lines { turn, .. } if turn.is_turned()))),
            ShadeFallback::Clipped => assert!(items.iter().any(|it| matches!(it, Placed::Cell { clip: true, .. }))),
            ShadeFallback::Merged => assert!(items.iter().any(|it| matches!(it, Placed::Cell { covered: true, .. }))),
            ShadeFallback::Header => assert!(items.iter().any(|it| matches!(it, Placed::Lines { story: StoryRef::Part(_), .. }))),
        }
        let snapshot: Vec<_> = l
            .pages
            .iter()
            .map(|page| {
                serde_json::json!({
                    "body":page.items.iter().map(shade_snapshot).collect::<Vec<_>>(),
                    "header":page.header.iter().map(shade_snapshot).collect::<Vec<_>>()
                })
            })
            .collect();
        eprintln!("SHADE_FALLBACK {kind:?} {}", serde_json::to_string(&snapshot).unwrap());
        assert_fallback_route(kind, &d, &l);
    }
}

const MODEL_LINE_COUNT: usize = 20;
const CELL_TOP_MARGIN: f32 = 4.0;
const CELL_SIDE_MARGIN: f32 = 6.0;
const SHADE_EPS: f32 = 0.01;

// Pin source placement and legacy geometric paint, not a new native Word policy.
fn assert_fallback_route(kind: ShadeFallback, d: &Document, l: &DocLayout) {
    use wordcraft_doc::props::TextDirection;
    let (items, story) = if matches!(kind, ShadeFallback::Header) {
        (&l.pages[0].header, StoryRef::Part(d.last_section.headers.default.unwrap()))
    } else {
        (&l.pages[0].items, StoryRef::Body)
    };
    let table_path = if matches!(kind, ShadeFallback::Nested) { vec![0, 0, 0, 0] } else { vec![0] };
    let mut para_path = table_path.clone();
    para_path.extend([0, 0, 0]);
    let cells: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            Placed::Cell { rect, table, row: 0, cell: 0, story: actual, covered, clip } if table.0 == table_path && *actual == story => {
                Some((*rect, *covered, *clip))
            }
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 1, "one source cell for {kind:?}");
    let (cell, covered, clip) = cells[0];
    assert!(!covered);
    assert_eq!(clip, matches!(kind, ShadeFallback::Clipped));
    let owners: Vec<_> = items
        .iter()
        .filter(|item| {
            matches!(item,
                Placed::Lines { story: actual, path, para, .. }
                if *actual == story && path.0 == para_path && para.rp.shading == Some(SHADE)
            )
        })
        .collect();
    assert_eq!(owners.len(), 1, "one source paragraph for {kind:?}");
    let Placed::Lines { l0, l1, x, y, turn, .. } = owners[0] else { panic!("expected source lines") };
    assert_eq!(*l0, 0);
    assert!((*y - cell.y - CELL_TOP_MARGIN).abs() < SHADE_EPS);
    let expected = match kind {
        ShadeFallback::Nested => {
            assert_eq!((*l0, *l1), (0, 7));
            assert_eq!(*turn, TextDirection::Horizontal);
            assert!((cell.y - PAGE_MARGIN).abs() < SHADE_EPS);
            assert!((cell.h - (PAGE_HEIGHT - 2.0 * PAGE_MARGIN)).abs() < SHADE_EPS);
            assert!((*x - cell.x - CELL_SIDE_MARGIN).abs() < SHADE_EPS);
            // The generic rectangle reaches the cut, beyond the retained line ink.
            let h = cell.bottom() - y;
            assert!(h > LINE_HEIGHT * *l1 as f32);
            Rect::new(*x, *y, TEXT_WIDTH, h)
        }
        ShadeFallback::Turned => {
            assert_eq!((*l0, *l1), (0, MODEL_LINE_COUNT));
            assert_eq!(*turn, TextDirection::Down);
            assert!((*x - (cell.right() - CELL_SIDE_MARGIN)).abs() < SHADE_EPS);
            assert_eq!(l.pages.len(), 1);
            let bounds = owners[0].turned_bounds().unwrap();
            assert!((bounds.w - LINE_HEIGHT * MODEL_LINE_COUNT as f32).abs() < SHADE_EPS);
            bounds
        }
        ShadeFallback::Clipped | ShadeFallback::Merged | ShadeFallback::Header => {
            assert_eq!((*l0, *l1), (0, MODEL_LINE_COUNT));
            assert_eq!(*turn, TextDirection::Horizontal);
            assert!((*x - cell.x - CELL_SIDE_MARGIN).abs() < SHADE_EPS);
            match kind {
                ShadeFallback::Clipped => {
                    assert_eq!(l.pages.len(), 1);
                    assert!((cell.h - 60.0).abs() < SHADE_EPS);
                    assert!(y + LINE_HEIGHT * MODEL_LINE_COUNT as f32 > cell.bottom());
                }
                ShadeFallback::Merged => {
                    assert_eq!(l.pages.len(), 2);
                    assert!((cell.h - (CELL_TOP_MARGIN + LINE_HEIGHT * MODEL_LINE_COUNT as f32)).abs() < SHADE_EPS);
                    assert!(l.pages[1].items.iter().any(|item| matches!(item,
                        Placed::Cell { story: StoryRef::Body, table, row: 1, cell: 0, covered: true, clip: false, rect }
                        if table.0 == [0] && (rect.y - PAGE_MARGIN).abs() < SHADE_EPS
                            && (rect.h - LINE_HEIGHT * MODEL_LINE_COUNT as f32).abs() < SHADE_EPS
                    )));
                    assert!(!l.pages[1].items.iter().any(|item| matches!(item, Placed::Lines { .. } | Placed::Fill { .. })));
                }
                ShadeFallback::Header => {
                    assert_eq!(l.pages.len(), 1);
                    assert!((cell.y - d.last_section.header).abs() < SHADE_EPS);
                    assert!((cell.h - (CELL_TOP_MARGIN + LINE_HEIGHT * MODEL_LINE_COUNT as f32)).abs() < SHADE_EPS);
                    assert!(!l.pages[0].items.iter().any(|item| matches!(item,
                        Placed::Lines { story: actual, path, .. } if *actual == story && path.0 == para_path
                    )));
                }
                _ => panic!("expected horizontal fallback"),
            }
            Rect::new(*x, *y, TEXT_WIDTH, LINE_HEIGHT * MODEL_LINE_COUNT as f32)
        }
    };
    assert_eq!(
        items
            .iter()
            .filter(|item| matches!(item,
                Placed::Fill { rect, color } if *color == SHADE && same_rect(*rect, expected)
            ))
            .count(),
        1,
        "exact fallback paragraph paint for {kind:?}: {expected:?}"
    );
}

#[test]
fn shade_continues_above_rule() {
    const BOTTOM_MARGIN: f32 = 6.0;
    const BOTTOM_WIDTH: f32 = 2.0;
    for page_height in [PAGE_HEIGHT, 1000.0] {
        let mut d = shade_doc(ShadeFlow::Partial, ShadeKind::Paragraph);
        d.last_section.page_h = page_height;
        let Block::Table(t) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!("expected original table") };
        assert_eq!(t.rows.len(), 1, "the split row is the table's last row");
        t.props.cell_margins = Some([CELL_TOP_MARGIN, CELL_SIDE_MARGIN, BOTTOM_MARGIN, CELL_SIDE_MARGIN]);
        t.props.borders.as_mut().unwrap().bottom = Some(Border::single(BOTTOM_WIDTH));
        let l = shade_layout(&d);
        if page_height == PAGE_HEIGHT {
            assert!(l.pages.len() > 1);
        } else {
            assert_eq!(l.pages.len(), 1);
        }
        assert_paragraph_fill(&l);
        let mut next = 0;
        for (page_index, page) in l.pages.iter().enumerate() {
            let owners: Vec<_> = page
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Lines { story: StoryRef::Body, path, l0, l1, x, y, .. } if path.0 == [0, 0, 0, 0] => Some((*l0, *l1, *x, *y)),
                    _ => None,
                })
                .collect();
            assert_eq!(owners.len(), 1);
            let (l0, l1, x, y) = owners[0];
            assert_eq!(l0, next);
            assert!(l1 > l0);
            if page_index > 0 {
                assert!(l0 > 0, "real paragraph continuation");
            }
            next = l1;
            let cells: Vec<_> = page
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Cell { story: StoryRef::Body, table, row: 0, cell: 0, covered: false, clip: false, rect } if table.0 == [0] => {
                        Some(*rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(cells.len(), 1);
            let cell = cells[0];
            assert!((y - cell.y - CELL_TOP_MARGIN).abs() < SHADE_EPS);
            let fills: Vec<_> = page
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Fill { rect, color } if *color == SHADE => Some(*rect),
                    _ => None,
                })
                .collect();
            assert_eq!(fills.len(), 1);
            let fill = fills[0];
            assert!(same_rect(fill, Rect::new(x, y, TEXT_WIDTH, LINE_HEIGHT * (l1 - l0) as f32)));
            assert!(fill.x >= cell.x && fill.right() <= cell.right() + SHADE_EPS);
            assert!(fill.y >= cell.y && fill.bottom() + BOTTOM_MARGIN <= cell.bottom() + SHADE_EPS);
            let closing: Vec<_> = page
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Rule { x0, y0, x1, y1, border }
                        if (x0 - cell.x).abs() < SHADE_EPS
                            && (x1 - cell.right()).abs() < SHADE_EPS
                            && (y0 - y1).abs() < SHADE_EPS
                            && (y0 - cell.bottom()).abs() < SHADE_EPS
                            && (border.width - BOTTOM_WIDTH).abs() < SHADE_EPS
                            && border.is_visible() =>
                    {
                        Some(*y0)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(closing.len(), 1, "closing rule on every physical fragment");
            assert!(fill.bottom() < closing[0]);
        }
        assert_eq!(next, MODEL_LINE_COUNT);
    }
}
