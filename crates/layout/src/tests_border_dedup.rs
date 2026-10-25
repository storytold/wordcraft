use super::*;
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing, TextFrame, VMerge};
use wordcraft_doc::{Paragraph, Rgb, Table, para_block};

const CELL_WIDTH: f32 = 150.0;
const TABLE_WIDTH: f32 = 2.0 * CELL_WIDTH;
const PAGE_MARGIN: f32 = 20.0;
const PAGE_BODY: f32 = 24.0;
const LINE_HEIGHT: f32 = 20.0;
const RULE_WIDTH: f32 = 1.0;
const FRAME_WRAP_NONE: &str = "none";
const CELL_COLOR: Rgb = Rgb(200, 30, 40);
const PARA_COLOR: Rgb = Rgb(30, 60, 200);

type EdgeBits = (u32, u32, u32, u32);

#[derive(Clone, Copy, Debug)]
enum EdgeRoute {
    CloseBottom,
    RepeatTop,
}

#[derive(Clone, Copy)]
enum MergeMode {
    Joined,
    Separate,
}

#[derive(Clone, Copy)]
enum BorderCase {
    Same,
    DifferentColor,
}

fn cell_border(route: EdgeRoute) -> Border {
    Border {
        style: BorderStyle::Single,
        width: RULE_WIDTH,
        color: Some(CELL_COLOR),
        // Cell border spacing is retained in Border but does not change table rule geometry.
        space: match route {
            EdgeRoute::CloseBottom => 0.0,
            EdgeRoute::RepeatTop => RULE_WIDTH,
        },
    }
}

fn paragraph(text: &str) -> Paragraph {
    let mut p = Paragraph::with_text(text, wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(9.0), ..Default::default() });
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    p.props.line_spacing = Some(LineSpacing::Exactly(LINE_HEIGHT));
    p.props.widow_control = Some(false);
    p.props.keep_next = Some(false);
    p.props.keep_lines = Some(false);
    p
}

fn collision_doc(route: EdgeRoute, merge: MergeMode, case: BorderCase) -> Document {
    let mut d = Document::new();
    d.settings.compat_mode = 15;
    d.last_section.page_w = TABLE_WIDTH + 2.0 * PAGE_MARGIN;
    d.last_section.page_h = PAGE_BODY + 2.0 * PAGE_MARGIN;
    d.last_section.margin_top = PAGE_MARGIN;
    d.last_section.margin_bottom = PAGE_MARGIN;
    d.last_section.margin_left = PAGE_MARGIN;
    d.last_section.margin_right = PAGE_MARGIN;
    let border = cell_border(route);
    let none = Some(Border::default());
    let mut t = Table::new(2, 2, TABLE_WIDTH);
    t.props.style = None;
    t.props.fixed = true;
    t.props.cell_margins = Some([0.0; 4]);
    t.props.borders = Some(Borders { top: none, bottom: none, left: none, right: none, between: Some(border), inside_v: none });
    for row in &mut t.rows {
        row.props.cant_split = true;
        for cell in &mut row.cells {
            cell.blocks = vec![para_block(paragraph("Row"))];
        }
    }
    if matches!(merge, MergeMode::Joined) {
        t.rows[0].cells[0].props.vmerge = VMerge::Restart;
        t.rows[1].cells[0].props.vmerge = VMerge::Continue;
    }
    let mut p = paragraph("Rule");
    let para_border = Border {
        color: Some(match case {
            BorderCase::Same => CELL_COLOR,
            BorderCase::DifferentColor => PARA_COLOR,
        }),
        ..border
    };
    match route {
        EdgeRoute::CloseBottom => {
            p.props.borders = Some(Borders { bottom: Some(para_border), ..Default::default() });
            t.rows[0].cells[0].blocks = vec![para_block(p)];
        }
        EdgeRoute::RepeatTop => {
            // The second cell's real paragraph rule spans the first cell through explicit indents.
            p.props.indent_left = Some(-CELL_WIDTH);
            p.props.indent_right = Some(CELL_WIDTH);
            p.props.indent_first = Some(0.0);
            p.props.borders = Some(Borders { top: Some(para_border), ..Default::default() });
            t.rows[1].cells[1].blocks = vec![para_block(p)];
        }
    }
    d.body = vec![std::sync::Arc::new(Block::Table(t))];
    d
}

fn framed_collision_doc(route: EdgeRoute, merge: MergeMode, case: BorderCase) -> Document {
    let mut d = collision_doc(route, merge, case);
    let path = match route {
        EdgeRoute::CloseBottom => Path(vec![0, 0, 0, 0]),
        EdgeRoute::RepeatTop => Path(vec![0, 1, 1, 0]),
    };
    let p = d.para_mut(StoryRef::Body, &path).unwrap();
    // Frame the rule independently of border flow padding so it meets the physical cell edge.
    p.props.frame = Some(TextFrame {
        w: Some(CELL_WIDTH),
        y: match route {
            EdgeRoute::CloseBottom => -RULE_WIDTH / 2.0,
            EdgeRoute::RepeatTop => RULE_WIDTH / 2.0,
        },
        wrap: FRAME_WRAP_NONE.into(),
        ..Default::default()
    });
    d
}

fn model_layout(d: &Document) -> DocLayout {
    layout(d, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn reference_geometry(l: &DocLayout, route: EdgeRoute) -> EdgeBits {
    let (page, row) = match route {
        EdgeRoute::CloseBottom => (0, 0),
        EdgeRoute::RepeatTop => (1, 1),
    };
    let rect = l.pages[page]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Cell { table, row: r, cell: 0, rect, story: StoryRef::Body, .. } if table.0 == [0] && *r == row => Some(*rect),
            _ => None,
        })
        .unwrap();
    let y = match route {
        EdgeRoute::CloseBottom => rect.bottom(),
        EdgeRoute::RepeatTop => rect.y,
    };
    (rect.x.to_bits(), rect.right().to_bits(), y.to_bits(), y.to_bits())
}

fn rules_at(l: &DocLayout, page: usize, geometry: EdgeBits) -> Vec<Border> {
    l.pages[page]
        .items
        .iter()
        .filter_map(|item| match item {
            Placed::Rule { x0, x1, y0, y1, border } if (x0.to_bits(), x1.to_bits(), y0.to_bits(), y1.to_bits()) == geometry => Some(*border),
            _ => None,
        })
        .collect()
}

fn assert_border_collision(route: EdgeRoute, case: BorderCase) {
    let reference = model_layout(&framed_collision_doc(route, MergeMode::Separate, case));
    let joined = model_layout(&framed_collision_doc(route, MergeMode::Joined, case));
    assert_eq!(reference.pages.len(), 2);
    assert_eq!(joined.pages.len(), 2);
    let page = match route {
        EdgeRoute::CloseBottom => 0,
        EdgeRoute::RepeatTop => 1,
    };
    let geometry = reference_geometry(&reference, route);
    let expected_cell = cell_border(route);
    let expected_para = Border {
        color: Some(match case {
            BorderCase::Same => CELL_COLOR,
            BorderCase::DifferentColor => PARA_COLOR,
        }),
        ..expected_cell
    };
    let emitted = rules_at(&reference, page, geometry);
    let actual = rules_at(&joined, page, geometry);
    eprintln!("BORDER_MODEL route={route:?} geometry={geometry:?} emitter={emitted:?} merged={actual:?}");
    assert_eq!(emitted.len(), 2, "the unmerged Document/Table emitters must establish the exact collision first");
    match case {
        BorderCase::DifferentColor => {
            assert_ne!(expected_para, expected_cell);
            assert_eq!(emitted.iter().filter(|b| **b == expected_para).count(), 1);
            assert_eq!(emitted.iter().filter(|b| **b == expected_cell).count(), 1);
            assert_eq!(actual.iter().filter(|b| **b == expected_para).count(), 1, "the emitted paragraph rule survives");
            assert_eq!(actual.iter().filter(|b| **b == expected_cell).count(), 1, "a distinct merged border must not be suppressed");
            assert_eq!(actual.len(), 2);
        }
        BorderCase::Same => {
            assert_eq!(expected_para, expected_cell);
            assert!(emitted.iter().all(|b| *b == expected_cell));
            assert_eq!(actual, [expected_cell], "identical full Borders remain deduplicated");
        }
    }
}

#[test]
fn merge_close_keeps_border() {
    assert_border_collision(EdgeRoute::CloseBottom, BorderCase::DifferentColor);
}

#[test]
fn merge_repeat_keeps_border() {
    assert_border_collision(EdgeRoute::RepeatTop, BorderCase::DifferentColor);
}

#[test]
fn merge_equal_border_once() {
    for route in [EdgeRoute::CloseBottom, EdgeRoute::RepeatTop] {
        assert_border_collision(route, BorderCase::Same);
    }
}

#[test]
fn unframed_rules_stay_separate() {
    // Border flow padding keeps default paragraph rules away from the cell edge.
    // Keep this model unframed rather than inheriting the collision fixture's offsets.
    for route in [EdgeRoute::CloseBottom, EdgeRoute::RepeatTop] {
        for case in [BorderCase::Same, BorderCase::DifferentColor] {
            let d = collision_doc(route, MergeMode::Separate, case);
            let path = match route {
                EdgeRoute::CloseBottom => Path(vec![0, 0, 0, 0]),
                EdgeRoute::RepeatTop => Path(vec![0, 1, 1, 0]),
            };
            assert!(d.para(StoryRef::Body, &path).unwrap().props.frame.is_none());
            let l = model_layout(&d);
            assert_eq!(l.pages.len(), 2);
            let page = match route {
                EdgeRoute::CloseBottom => 0,
                EdgeRoute::RepeatTop => 1,
            };
            let geometry = reference_geometry(&l, route);
            let edge_y = f32::from_bits(geometry.2);
            let para_y = match route {
                EdgeRoute::CloseBottom => edge_y - RULE_WIDTH / 2.0,
                EdgeRoute::RepeatTop => edge_y + RULE_WIDTH + RULE_WIDTH / 2.0,
            };
            let para_geometry = (geometry.0, geometry.1, para_y.to_bits(), para_y.to_bits());
            assert_ne!(para_geometry, geometry, "default flow must not manufacture a collision");
            let expected_cell = cell_border(route);
            let expected_para = Border {
                color: Some(match case {
                    BorderCase::Same => CELL_COLOR,
                    BorderCase::DifferentColor => PARA_COLOR,
                }),
                ..expected_cell
            };
            assert_eq!(rules_at(&l, page, geometry), [expected_cell]);
            assert_eq!(rules_at(&l, page, para_geometry), [expected_para]);
            for page in &l.pages {
                for item in &page.items {
                    if let Placed::Cell { rect, .. } = item {
                        assert!(rect.y >= PAGE_MARGIN);
                        assert!(rect.bottom() <= PAGE_MARGIN + PAGE_BODY);
                    }
                }
            }
        }
    }
}

#[test]
fn merged_rules_are_ordered() {
    const MERGED_COLUMNS: usize = 12;
    let columns = MERGED_COLUMNS + 1;
    let width = columns as f32 * CELL_WIDTH;
    let border = cell_border(EdgeRoute::CloseBottom);
    let mut d = Document::new();
    d.settings.compat_mode = 15;
    d.last_section.page_w = width + 2.0 * PAGE_MARGIN;
    d.last_section.page_h = PAGE_BODY + 2.0 * PAGE_MARGIN;
    d.last_section.margin_top = PAGE_MARGIN;
    d.last_section.margin_bottom = PAGE_MARGIN;
    d.last_section.margin_left = PAGE_MARGIN;
    d.last_section.margin_right = PAGE_MARGIN;
    let mut table = Table::new(2, columns, width);
    table.props.style = None;
    table.props.fixed = true;
    table.props.cell_margins = Some([0.0; 4]);
    let none = Some(Border::default());
    table.props.borders = Some(Borders { top: none, bottom: Some(border), left: none, right: none, between: Some(border), inside_v: none });
    for (index, row) in table.rows.iter_mut().enumerate() {
        row.props.cant_split = true;
        for (column, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = vec![para_block(paragraph("Row"))];
            if column < MERGED_COLUMNS {
                cell.props.vmerge = if index == 0 { VMerge::Restart } else { VMerge::Continue };
            }
        }
    }
    d.body = vec![std::sync::Arc::new(Block::Table(table))];
    for _ in 0..3 {
        let l = model_layout(&d);
        assert_eq!(l.pages.len(), 2);
        let page = &l.pages[0];
        let cells: Vec<_> = page
            .items
            .iter()
            .filter_map(|item| match item {
                Placed::Cell { table, row: 0, cell, rect, .. } if table.0 == [0] && *cell < MERGED_COLUMNS => Some((*cell, *rect)),
                _ => None,
            })
            .collect();
        assert_eq!(cells.len(), MERGED_COLUMNS);
        let close = page
            .items
            .iter()
            .find_map(|item| match item {
                Placed::Cell { table, row: 0, cell: MERGED_COLUMNS, rect, .. } if table.0 == [0] => Some(rect.bottom()),
                _ => None,
            })
            .unwrap();
        let expected: Vec<_> = cells
            .iter()
            .map(|(column, rect)| {
                assert_eq!(rect.x, PAGE_MARGIN + *column as f32 * CELL_WIDTH);
                (rect.x, rect.right(), close, close, border)
            })
            .collect();
        let actual: Vec<_> = page
            .items
            .iter()
            .filter_map(|item| match item {
                Placed::Rule { x0, x1, y0, y1, border: actual }
                    if *actual == border && y0 == y1 && *y0 == close && *x0 < PAGE_MARGIN + MERGED_COLUMNS as f32 * CELL_WIDTH =>
                {
                    Some((*x0, *x1, *y0, *y1, *actual))
                }
                _ => None,
            })
            .collect();
        let mut by_column = actual.clone();
        by_column.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(by_column, expected, "all closures keep their exact physical-fragment geometry and style");
        assert_eq!(actual, expected, "merged closures retain source-column order");
    }
}
