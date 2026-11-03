//! Where a table nested in a cell sits in an older-mode document (compatibility mode below 15).
//! The docxide-pdf `nested-autofit-padding-*` fixtures (Word's PDFs) put each nested level's
//! text one left cell margin further in; only the outer table moves out by its margin.

use crate::{Arc, Block, Document, LayoutCache, LayoutOptions, Placed, StoryRef, layout};
use wordcraft_doc::para::{InlineObject, ShapeKind};
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
        .flat_map(|p| &p.items)
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
    let Some(Block::Table(outer)) = d.body.first_mut().map(Arc::make_mut) else { panic!("outer table") };
    let Some(Block::Table(nested)) = outer.rows[0].cells[0].blocks.first_mut().map(Arc::make_mut) else { panic!("nested table") };
    nested.props.indent = Some(INDENT);
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
