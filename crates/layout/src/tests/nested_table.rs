//! Where a table nested in a cell sits in an older-mode document (compatibility mode below 15).
//! The docxide-pdf `nested-autofit-padding-*` fixtures (Word's PDFs) put each nested level's
//! text one left cell margin further in; only the outer table moves out by its margin.

use crate::{Arc, Block, Document, LayoutCache, LayoutOptions, Placed, StoryRef, layout};
use wordcraft_doc::para::{Anchor, InlineObject, ShapeKind};
use wordcraft_doc::props::{Border, Borders, TableFloat};
use wordcraft_doc::{Paragraph, PartKind, Pos, Table, para_block};

const MARGIN: f32 = 10.8;

fn table(width: f32, margin: f32, blocks: Vec<Arc<Block>>) -> Table {
    let mut t = Table::new(1, 1, width);
    t.props.style = None;
    t.props.width = Some(width);
    t.props.fixed = true;
    t.props.cell_margins = Some([0.0, margin, 0.0, margin]);
    t.rows[0].cells[0].blocks = blocks;
    t
}

/// How far right of the text after the table the innermost cell's text starts, in `story`.
fn inset_in(d: &Document, story: StoryRef) -> f32 {
    let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
    let lines: Vec<(usize, f32)> = l
        .pages
        .iter()
        .flat_map(|p| p.items.iter().chain(&p.header))
        .filter_map(|it| match it {
            Placed::Lines { story: s, path, x, .. } if *s == story => Some((path.0.len(), *x)),
            _ => None,
        })
        .collect();
    let body = lines.iter().find(|(depth, _)| *depth == 1).unwrap().1;
    let inner = lines.iter().max_by_key(|(depth, _)| *depth).unwrap().1;
    inner - body
}

fn inset(d: &Document) -> f32 {
    inset_in(d, StoryRef::Body)
}

fn document(levels: usize) -> Document {
    let mut d = Document::new();
    d.settings.compat_mode = 12;
    let mut blocks = vec![wordcraft_doc::para_block(Paragraph::with_text("Inner", Default::default()))];
    for _ in 0..levels {
        let nested = table(300.0, MARGIN, blocks);
        blocks = vec![Arc::new(Block::Table(nested)), wordcraft_doc::para_block(Paragraph::with_text("", Default::default()))];
    }
    d.body = vec![Arc::new(Block::Table(table(400.0, 0.0, blocks))), wordcraft_doc::para_block(Paragraph::with_text("Body", Default::default()))];
    d
}

#[test]
fn a_nested_table_keeps_its_edge_at_its_cells_text() {
    for levels in 1..=3 {
        let inset = inset(&document(levels));
        assert!((inset - MARGIN * levels as f32).abs() < 0.01, "{levels} levels: inner text {inset}pt in");
    }
}

#[test]
fn a_nested_tables_indent_adds_to_its_cells_text_edge() {
    const INDENT: f32 = 15.0;
    let mut d = document(1);
    nested(&mut d).props.indent = Some(INDENT);
    let inset = inset(&d);
    assert!((inset - (MARGIN + INDENT)).abs() < 0.01, "inner text {inset}pt in");
}

#[test]
fn an_outer_table_still_moves_out_by_its_margin() {
    let mut d = document(0);
    let Some(Block::Table(t)) = d.body.first_mut().map(Arc::make_mut) else { panic!("table") };
    t.props.cell_margins = Some([0.0, MARGIN, 0.0, MARGIN]);
    assert!(inset(&d).abs() < 0.01);
}

#[test]
fn a_table_at_the_top_of_a_text_box_still_moves_out_by_its_margin() {
    let mut d = Document::new();
    d.settings.compat_mode = 12;
    let inner = vec![para_block(Paragraph::with_text("Inner", Default::default()))];
    let story = vec![Arc::new(Block::Table(table(200.0, MARGIN, inner))), para_block(Paragraph::with_text("After", Default::default()))];
    let id = d.add_part(PartKind::TextBox, story);
    let shape = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 300.0,
        h: 200.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    assert!(inset_in(&d, StoryRef::Part(id)).abs() < 0.01);
}

/// The table nested in the outer table's cell of [`document`]`(1)`.
fn nested(d: &mut Document) -> &mut Table {
    let Some(Block::Table(outer)) = d.body.first_mut().map(Arc::make_mut) else { panic!("outer table") };
    let Some(Block::Table(nested)) = outer.rows[0].cells[0].blocks.first_mut().map(Arc::make_mut) else { panic!("nested table") };
    nested
}

/// Characterization, unverified in Word: a floating table nested in a cell (`w:tblpPr`, no
/// horizontal alignment) still moves out by its first cell's margin before mode 15.
#[test]
fn a_floating_nested_table_still_moves_out_by_its_margin() {
    let mut d = document(1);
    nested(&mut d).props.float = Some(TableFloat { h_rel: Anchor::Column, v_rel: Anchor::Paragraph, ..Default::default() });
    let inset = inset(&d);
    assert!(inset.abs() < 0.01, "inner text {inset}pt in");
}

/// Mode 15 never moved a table out, nested or not: each level's margin insets its text.
#[test]
fn in_mode_15_a_nested_tables_text_is_inset_by_its_margin() {
    for levels in 1..=3 {
        let mut d = document(levels);
        d.settings.compat_mode = 15;
        let inset = inset(&d);
        assert!((inset - MARGIN * levels as f32).abs() < 0.01, "{levels} levels: inner text {inset}pt in");
    }
}

/// A right-to-left nested table (`w:bidiVisual`) mirrors: its edge sits at the cell's text on
/// the right, so its 300pt column starts 100pt into the 400pt cell, plus its end margin.
#[test]
fn a_right_to_left_nested_table_keeps_its_edge_at_its_cells_text_on_the_right() {
    let mut d = document(1);
    nested(&mut d).props.bidi_visual = true;
    let inset = inset(&d);
    assert!((inset - (100.0 + MARGIN)).abs() < 0.01, "inner text {inset}pt in");
}

#[test]
fn a_table_at_the_top_of_a_header_still_moves_out_by_its_margin() {
    let mut d = document(0);
    let inner = vec![para_block(Paragraph::with_text("Inner", Default::default()))];
    let header = vec![Arc::new(Block::Table(table(200.0, MARGIN, inner))), para_block(Paragraph::with_text("After", Default::default()))];
    let id = d.add_part(PartKind::Header, header);
    d.last_section.headers.default = Some(id);
    assert!(inset_in(&d, StoryRef::Part(id)).abs() < 0.01);
}

/// Where the leftmost vertical line sits right of the body text: in [`document`]`(1)` the outer
/// table has no margin and no lines, so that is the nested table's left line from its cell's text.
fn line_in(d: &Document) -> f32 {
    let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
    let items = || l.pages.iter().flat_map(|p| &p.items);
    let body = items().find_map(|it| match it {
        Placed::Lines { path, x, .. } if path.0.len() == 1 => Some(*x),
        _ => None,
    });
    let line = items()
        .filter_map(|it| match it {
            Placed::Rule { x0, x1, .. } if (x0 - x1).abs() < 0.01 => Some(*x0),
            _ => None,
        })
        .reduce(f32::min);
    line.unwrap() - body.unwrap()
}

/// [`document`]`(1)` in `mode`, its nested table with a 1.5pt left line.
fn lined(mode: u32) -> Document {
    let mut d = document(1);
    d.settings.compat_mode = mode;
    nested(&mut d).props.borders = Some(Borders { left: Some(Border::single(1.5)), ..Default::default() });
    d
}

/// Word for Windows, Nov 2026 (mode 14, then mode 15): cell text at 72.02pt, the nested
/// table's 1.5pt left line at 72.75pt; text at 76.58pt, line at 77.31pt.
#[test]
fn a_nested_tables_left_line_sits_half_its_width_in_from_its_cells_text() {
    for mode in [12, 14, 15] {
        let line = line_in(&lined(mode));
        assert!((line - 0.75).abs() < 0.01, "mode {mode}: line {line}pt in");
    }
}

/// Word for Windows, Nov 2026 (mode 14): `w:tblInd` 240 puts the line at 84.75pt, 12.75pt in.
#[test]
fn a_nested_tables_indent_adds_to_its_left_lines_half_width() {
    let mut d = lined(14);
    nested(&mut d).props.indent = Some(12.0);
    let line = line_in(&d);
    assert!((line - 12.75).abs() < 0.01, "line {line}pt in");
}

/// Word for Windows, Nov 2026 (mode 14): the nested table's own margins (10pt) or its first
/// cell's own left margin (15pt) leave its line at 72.75pt.
#[test]
fn a_nested_tables_own_margins_leave_its_edge() {
    let mut d = lined(14);
    let t = nested(&mut d);
    t.props.cell_margins = Some([0.0, 10.0, 0.0, 10.0]);
    t.rows[0].cells[0].props.margins = Some([0.0, 15.0, 0.0, 15.0]);
    let line = line_in(&d);
    assert!((line - 0.75).abs() < 0.01, "line {line}pt in");
}
