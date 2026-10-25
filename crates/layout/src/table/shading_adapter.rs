use super::*;
use std::sync::Arc;
use wordcraft_doc::props::{BorderStyle, LineSpacing};
use wordcraft_doc::{Document, para_block};

const LINE_HEIGHT: f32 = 20.0;
const CELL_PAD: f32 = 4.0;
const CUT: f32 = 160.0;

fn emitted_row() -> RowLayout {
    let mut d = Document::new();
    d.last_section.page_h = 1000.0;
    let mut p = Paragraph::with_text(&["Original shade"; 20].join("\n"), Default::default());
    p.props.shading = Some(Rgb(45, 105, 165));
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    p.props.line_spacing = Some(LineSpacing::Exactly(LINE_HEIGHT));
    p.props.widow_control = Some(false);
    let mut t = Table::new(1, 1, 200.0);
    t.props.style = None;
    t.props.cell_margins = Some([CELL_PAD, 6.0, CELL_PAD, 6.0]);
    let none = Some(Border { style: BorderStyle::None, ..Default::default() });
    t.props.borders = Some(Borders { top: none, bottom: none, left: none, right: none, between: none, inside_v: none });
    t.rows[0].cells[0].blocks = vec![para_block(p)];
    d.body = vec![Arc::new(Block::Table(t))];
    let mut l = crate::layout(&d, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default());
    assert_eq!(l.pages.len(), 1);
    let mut items = l.pages.remove(0).items;
    let cells: Vec<_> = items
        .iter()
        .filter_map(|it| match it {
            Placed::Cell { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 1);
    let frame = cells[0];
    for item in &mut items {
        item.translate(0.0, -frame.y);
    }
    assert!(items.iter().any(|it| matches!(it, Placed::Lines { para, .. } if para.lines.len() == 20)));
    let part = CellPart { top: CELL_PAD, items: 0..items.len(), end: Some(frame.h - CELL_PAD), center: None, merge: None };
    RowLayout { edge: 0.0, height: frame.h, min_height: 0.0, bottom: CELL_PAD, items, cells: vec![part] }
}

fn fill_pair(pair: &(RowLayout, RowLayout)) -> [Vec<Rect>; 2] {
    [&pair.0, &pair.1].map(|row| {
        row.items
            .iter()
            .filter_map(|it| match it {
                Placed::Fill { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect()
    })
}

fn assert_same_pair(a: &(RowLayout, RowLayout), b: &(RowLayout, RowLayout)) {
    for (a, b) in [(&a.0, &b.0), (&a.1, &b.1)] {
        assert_eq!((a.height, a.bottom, a.min_height), (b.height, b.bottom, b.min_height));
        assert_eq!(format!("{:?}", a.items), format!("{:?}", b.items));
        assert_eq!(a.cells.len(), b.cells.len());
        for (a, b) in a.cells.iter().zip(&b.cells) {
            assert_eq!((&a.items, a.top, a.end), (&b.items, b.top, b.end));
        }
    }
}

#[test]
fn cleared_center_keeps_fallback() {
    let row = emitted_row();
    let fallback = split_row_origin(&row, CUT, &[]).unwrap();
    let repaired = split_row_inner(&row, CUT).unwrap();
    assert_ne!(fill_pair(&repaired), fill_pair(&fallback), "the plain owner must exercise shading routing");
    for kind in [CenterKind::Fragment, CenterKind::Merge] {
        let mut original = row.cells.clone();
        original[0].center = Some(CellCenter { kind, offset: 0.0, height: row.height, content: original[0].items.clone() });
        assert!(row.cells[0].center.is_none());
        assert!(!shade_origin(&row.cells[0], original.first()));
        let actual = split_row_origin(&row, CUT, &original).unwrap();
        assert_same_pair(&actual, &fallback);
        let mut active = row.clone();
        active.cells[0].center = original[0].center.clone();
        assert!(!shade_origin(&active.cells[0], row.cells.first()));
        if kind == CenterKind::Fragment {
            assert_same_pair(&split_row(&active, CUT).unwrap(), &fallback);
        }
    }
}

#[test]
fn shade_merge_keeps_fallback() {
    let row = emitted_row();
    let fallback = split_row_origin(&row, CUT, &[]).unwrap();
    let mut original = row.cells.clone();
    original[0].merge = Some(MergeEdge { key: (0, 0), x0: 0.0, x1: 200.0, top: None, bottom: None });
    assert!(!shade_origin(&row.cells[0], original.first()));
    assert_same_pair(&split_row_origin(&row, CUT, &original).unwrap(), &fallback);
    let mut active = row.clone();
    active.cells[0].merge = original[0].merge.clone();
    assert!(!shade_origin(&active.cells[0], row.cells.first()));
}

#[test]
fn shade_origin_requires_range() {
    let row = emitted_row();
    let fallback = split_row_origin(&row, CUT, &[]).unwrap();
    assert!(!shade_origin(&row.cells[0], None));
    let mut original = row.cells.clone();
    original[0].items.end -= 1;
    assert!(!shade_origin(&row.cells[0], original.first()));
    assert_same_pair(&split_row_origin(&row, CUT, &original).unwrap(), &fallback);
    let original = [row.cells[0].clone(), row.cells[0].clone()];
    assert_same_pair(&split_row_origin(&row, CUT, &original).unwrap(), &fallback);
}

#[test]
fn mixed_origin_is_per_cell() {
    let row = emitted_row();
    let plain = row.cells[0].clone();
    let mut merged = plain.clone();
    merged.merge = Some(MergeEdge { key: (0, 1), x0: 200.0, x1: 400.0, top: None, bottom: None });
    assert!(shade_origin(&plain, Some(&plain)));
    assert!(!shade_origin(&plain, Some(&merged)));
    assert!(!shade_origin(&merged, Some(&plain)));
    // A sibling's merge does not erase an unambiguous plain cell's source identity.
    let original = [plain.clone(), merged];
    assert!(shade_origin(&plain, original.first()));
    assert!(!shade_origin(&original[1], original.get(1)));
}
