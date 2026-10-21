use super::*;
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing, VMerge};
use wordcraft_doc::{Paragraph, Rgb, Table, para_block};

const CELL_WIDTH: f32 = 150.0;
const TABLE_WIDTH: f32 = 2.0 * CELL_WIDTH;
const PAGE_MARGIN: f32 = 20.0;
const PAGE_BODY: f32 = 24.0;
const LINE_HEIGHT: f32 = 20.0;
const RULE_WIDTH: f32 = 1.0;
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
    let reference = model_layout(&collision_doc(route, MergeMode::Separate, case));
    let joined = model_layout(&collision_doc(route, MergeMode::Joined, case));
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
