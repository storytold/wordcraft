//! A row of a nested table marked "can't split" moves on with its outer row, as in Word
//! (docxide-pdf `nested-first-row-*-cant1`): the outer row splits only above it.
use crate::*;
use wordcraft_doc::{Table, para_block};

fn one_line_paras(what: &str, n: usize) -> Vec<Arc<Block>> {
    (0..n)
        .map(|i| {
            let mut p = Paragraph::with_text(&format!("{what} {i}"), Default::default());
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
            para_block(p)
        })
        .collect()
}

/// A filler paragraph pushed `push` points down, then a one-row table whose cells each hold
/// `prefix` one-line paragraphs and a nested table of one row with `nested` of them. Returns,
/// per cell, the prefix lines and the nested lines on each page.
fn lines_per_page(push: f32, cells: &[(usize, usize)], cant_split: bool) -> Vec<(Vec<usize>, Vec<usize>)> {
    let mut d = Document::from_text("Filler");
    if let Some(Block::Para(p)) = d.body.first_mut().map(Arc::make_mut) {
        p.props.space_before = Some(push);
    }
    let mut outer = Table::new(1, cells.len(), 440.0);
    for (cell, &(prefix, nested)) in outer.rows[0].cells.iter_mut().zip(cells) {
        let mut inner = Table::new(1, 1, 200.0);
        inner.rows[0].props.cant_split = cant_split;
        inner.rows[0].cells[0].blocks = one_line_paras("Nested", nested);
        cell.blocks = one_line_paras("Prefix", prefix);
        cell.blocks.push(Arc::new(Block::Table(inner)));
        cell.blocks.push(para_block(Paragraph::with_text("", Default::default())));
    }
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(outer)).unwrap();
    let mut c = LayoutCache::new();
    let l = layout(&d, &mut c, &LayoutOptions::default());
    let count = |cell: u32, nested: bool| -> Vec<usize> {
        l.pages
            .iter()
            .map(|pg| {
                pg.items
                    .iter()
                    .filter_map(|it| match it {
                        Placed::Lines { path, l0, l1, .. } => {
                            let p = &path.0;
                            let hit = if nested {
                                p.len() == 7 && p[..3] == [1, 0, cell] && p[4..6] == [0, 0]
                            } else {
                                p.len() == 4 && p[..3] == [1, 0, cell]
                            };
                            hit.then_some(l1 - l0)
                        }
                        _ => None,
                    })
                    .sum()
            })
            .collect()
    };
    (0..cells.len() as u32).map(|c| (count(c, false), count(c, true))).collect()
}

#[test]
fn nested_cant_split_row_moves_on_with_its_outer_row() {
    let split = &lines_per_page(520.0, &[(0, 20)], false)[0].1;
    assert!(split[0] > 0 && split[1] > 0, "a splittable nested row splits with its outer row: {split:?}");
    assert_eq!(lines_per_page(520.0, &[(0, 20)], true)[0].1[..2], [0, 20], "a can't-split nested row moves on whole");
}

#[test]
fn the_outer_row_splits_above_a_nested_cant_split_row() {
    // About 22 lines of room: the nested row (lines 10 to 40) runs through it.
    let cell = &lines_per_page(340.0, &[(10, 30)], true)[0];
    assert!(cell.0[0] > 0, "the text above the nested row stays: {cell:?}");
    assert_eq!(cell.1[..2], [0, 30], "the nested row moves on whole: {cell:?}");
}

#[test]
fn the_cut_moves_up_until_no_nested_cant_split_row_runs_through_it() {
    // Room for about 22 lines. Only cell 0's nested row (lines 10 to 40) runs through it; the
    // cut at its top then runs through cell 1's (lines 5 to 13), which moves on whole too.
    let cells = lines_per_page(340.0, &[(10, 30), (5, 8)], true);
    assert_eq!(cells[0].1[..2], [0, 30], "{cells:?}");
    assert_eq!(cells[1].1[..2], [0, 8], "{cells:?}");
    assert!(cells[0].0[0] > 0 && cells[1].0[0] > 0, "the text above both stays: {cells:?}");
}

#[test]
fn nested_cant_split_row_taller_than_a_page_still_splits() {
    let pages = &lines_per_page(520.0, &[(0, 60)], true)[0].1;
    assert_eq!(pages[0], 0, "it leaves the partly filled page: {pages:?}");
    assert_eq!(pages.iter().sum::<usize>(), 60, "every line is placed once: {pages:?}");
    assert!(pages.iter().filter(|&&n| n > 0).count() >= 2, "it runs on over pages instead of into the margin: {pages:?}");
}

#[test]
fn the_outer_row_breaks_between_nested_cant_split_rows_above_its_bottom_band() {
    let mut nested = Table::new(40, 1, 200.0);
    nested.props.cell_margins = Some([0.0; 4]);
    for (i, row) in nested.rows.iter_mut().enumerate() {
        row.props.cant_split = true;
        row.cells[0].blocks = one_line_paras(&format!("Row {i}"), 2);
    }
    let mut outer = Table::new(1, 1, 400.0);
    // A bottom margin: the part left on the page keeps it below the last row there.
    outer.props.cell_margins = Some([0.0, 5.4, 4.0, 5.4]);
    outer.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(nested)), empty()];
    let l = lay_pushed(520.0, outer);
    let pages = per_page(&l, |p| p.len() == 7 && p[..4] == [1, 0, 0, 0]);
    assert!(pages[0] > 0, "the rows that fit stay on the page: {pages:?}");
    assert_eq!(pages.iter().sum::<usize>(), 80, "every line is placed once: {pages:?}");
    for r in 0..40u32 {
        let on: Vec<usize> = per_page(&l, |p| p.len() == 7 && p[..5] == [1, 0, 0, 0, r]);
        assert_eq!(on.iter().filter(|&&n| n > 0).count(), 1, "row {r} stays on one page: {on:?}");
    }
}

fn empty() -> Arc<Block> {
    para_block(Paragraph::with_text("", Default::default()))
}

/// A nested table of one row, marked "can't split", with `lines` one-line paragraphs.
fn kept_nested(lines: usize) -> Arc<Block> {
    let mut t = Table::new(1, 1, 200.0);
    t.rows[0].props.cant_split = true;
    t.rows[0].cells[0].blocks = one_line_paras("Nested", lines);
    Arc::new(Block::Table(t))
}

/// Lay out a filler paragraph pushed `push` points down, then `outer`.
fn lay_pushed(push: f32, outer: Table) -> DocLayout {
    let mut d = Document::from_text("Filler");
    if let Some(Block::Para(p)) = d.body.first_mut().map(Arc::make_mut) {
        p.props.space_before = Some(push);
    }
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(outer)).unwrap();
    let mut c = LayoutCache::new();
    layout(&d, &mut c, &LayoutOptions::default())
}

/// The lines on each page of the body paragraphs whose path `hit` picks.
fn per_page(l: &DocLayout, hit: impl Fn(&[u32]) -> bool) -> Vec<usize> {
    l.pages
        .iter()
        .map(|pg| {
            pg.items
                .iter()
                .filter_map(|it| match it {
                    Placed::Lines { path, l0, l1, .. } => hit(&path.0).then_some(l1 - l0),
                    _ => None,
                })
                .sum()
        })
        .collect()
}

#[test]
fn a_bottom_aligned_cell_keeps_its_nested_cant_split_row_whole() {
    // Cell 1 sits at the bottom of the row as laid out, but the split runs it from its top,
    // where its nested row (lines 2 to 32) runs through the room of about 22 lines.
    let mut outer = Table::new(1, 2, 440.0);
    outer.rows[0].cells[0].blocks = one_line_paras("Tall", 50);
    outer.rows[0].cells[1].props.valign = Some(wordcraft_doc::props::VAlign::Bottom);
    let mut blocks = one_line_paras("Prefix", 2);
    blocks.extend([kept_nested(30), empty()]);
    outer.rows[0].cells[1].blocks = blocks;
    let l = lay_pushed(340.0, outer);
    let nested = per_page(&l, |p| p.len() == 7 && p[..4] == [1, 0, 1, 2]);
    assert_eq!(nested[..2], [0, 30], "the nested row moves on whole: {nested:?}");
}

#[test]
fn header_rows_go_on_with_a_first_row_whose_nested_cant_split_row_comes_first() {
    let mut outer = Table::new(2, 1, 400.0);
    outer.rows[0].props.header = true;
    outer.rows[0].cells[0].blocks = one_line_paras("Header", 1);
    outer.rows[1].cells[0].blocks = vec![kept_nested(20), empty()];
    let l = lay_pushed(520.0, outer);
    let header = per_page(&l, |p| p == [1, 0, 0, 0]);
    assert_eq!(header[0], 0, "the header row doesn't stay alone at the page bottom: {header:?}");
    assert_eq!(per_page(&l, |p| p.len() == 7 && p[..4] == [1, 1, 0, 0])[..2], [0, 20]);
}

#[test]
fn a_kept_row_goes_on_with_a_row_whose_nested_cant_split_row_cant_start_below_it() {
    let mut outer = Table::new(2, 1, 400.0);
    let mut kept = one_line_paras("Kept", 1);
    if let Some(Block::Para(p)) = kept.first_mut().map(Arc::make_mut) {
        p.props.keep_next = Some(true);
    }
    outer.rows[0].cells[0].blocks = kept;
    outer.rows[1].cells[0].blocks = vec![kept_nested(20), empty()];
    let l = lay_pushed(520.0, outer);
    let kept = per_page(&l, |p| p == [1, 0, 0, 0]);
    assert_eq!(kept[..2], [0, 1], "the kept row moves on with the next one: {kept:?}");
}

#[test]
fn a_merged_cell_doesnt_stretch_its_cant_split_row() {
    // Row 0 can't split; its first cell is merged down into row 1, which can and runs through
    // the room. The cut stays in row 1.
    let mut nested = Table::new(2, 2, 300.0);
    nested.rows[0].props.cant_split = true;
    nested.rows[0].cells[0].props.vmerge = wordcraft_doc::props::VMerge::Restart;
    nested.rows[1].cells[0].props.vmerge = wordcraft_doc::props::VMerge::Continue;
    nested.rows[0].cells[0].blocks = one_line_paras("Merged", 1);
    nested.rows[0].cells[1].blocks = one_line_paras("Short", 1);
    nested.rows[1].cells[1].blocks = one_line_paras("Long", 20);
    let mut outer = Table::new(1, 1, 400.0);
    outer.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(nested)), empty()];
    let l = lay_pushed(520.0, outer);
    let long = per_page(&l, |p| p.len() == 7 && p[..6] == [1, 0, 0, 0, 1, 1]);
    assert!(long[0] > 0 && long[1] > 0, "the splittable row splits: {long:?}");
}
