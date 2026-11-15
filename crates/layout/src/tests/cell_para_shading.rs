//! #6627: a cell paragraph's shading covers the cell's left and right margins, so the cell's
//! own fill does not show beside it.

use crate::{Arc, Block, Document, LayoutCache, LayoutOptions, Placed, layout};
use wordcraft_doc::props::{CharProps, Highlight, Rgb};
use wordcraft_doc::{Paragraph, Table};
use wordcraft_geom::Rect;

const CELL: Rgb = Rgb(0xFF, 0, 0);
const PARA: Rgb = Rgb(0xBF, 0xBF, 0xBF);

/// The page's fills, laid out from a one-cell red table holding `p`.
fn page_fills(p: Paragraph) -> Vec<(Rgb, Rect)> {
    let mut d = Document::from_text("Before");
    let mut t = Table::new(1, 2, 200.0);
    t.props.style = None;
    t.props.fixed = true;
    t.props.cell_margins = Some([0.0, 1.4, 0.0, 1.4]);
    let cell = &mut t.rows[0].cells[0];
    cell.props.shading = Some(CELL);
    cell.blocks = vec![wordcraft_doc::para_block(p)];
    d.body.push(Arc::new(Block::Table(t)));
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    l.pages[0].items.iter().filter_map(|i| if let Placed::Fill { rect, color } = i { Some((*color, *rect)) } else { None }).collect()
}

fn fill(fills: &[(Rgb, Rect)], c: Rgb) -> Rect {
    fills.iter().find(|(color, _)| *color == c).map(|(_, r)| *r).unwrap()
}

fn fills(text: &str, indent: f32) -> (Rect, Rect) {
    let mut p = Paragraph::with_text(text, Default::default());
    p.props.shading = Some(PARA);
    p.props.indent_left = Some(indent);
    let f = page_fills(p);
    (fill(&f, CELL), fill(&f, PARA))
}

#[test]
fn paragraph_shading_covers_the_cell_margins() {
    let (cell, para) = fills("Header", 0.0);
    assert!((para.x - cell.x).abs() < 0.01, "cell {cell:?} paragraph {para:?}");
    assert!((para.right() - cell.right()).abs() < 0.01, "cell {cell:?} paragraph {para:?}");
}

#[test]
fn wrapped_paragraph_shading_covers_the_cell_margins_on_every_line() {
    let mut p = Paragraph::with_text(&"Header text that wraps ".repeat(8), Default::default());
    p.props.shading = Some(PARA);
    let f = page_fills(p);
    let cell = fill(&f, CELL);
    let paras: Vec<Rect> = f.iter().filter(|(c, _)| *c == PARA).map(|(_, r)| *r).collect();
    assert!(!paras.is_empty());
    // The shading reaches over at least two lines' height.
    assert!(paras.iter().map(|r| r.h).sum::<f32>() > 20.0, "paragraph {paras:?}");
    for para in &paras {
        assert!((para.x - cell.x).abs() < 0.01, "cell {cell:?} paragraph {para:?}");
        assert!((para.right() - cell.right()).abs() < 0.01, "cell {cell:?} paragraph {para:?}");
    }
}

#[test]
fn indented_paragraph_shading_starts_at_its_indent() {
    let (cell, para) = fills("Header", 20.0);
    assert!((para.x - (cell.x + 1.4 + 20.0)).abs() < 0.01, "cell {cell:?} paragraph {para:?}");
    assert!((para.right() - cell.right()).abs() < 0.01, "cell {cell:?} paragraph {para:?}");
}

#[test]
fn unshaded_paragraph_with_a_highlight_adds_no_fill_over_the_cell_margins() {
    let p = Paragraph::with_text("Header", CharProps { highlight: Some(Highlight::Yellow), ..Default::default() });
    let f = page_fills(p);
    let cell = fill(&f, CELL);
    for (c, r) in f.iter().filter(|(c, _)| *c != CELL) {
        assert!(r.x > cell.x + 1.0, "fill {c:?} {r:?} starts at the cell's edge {cell:?}");
    }
}
