//! The room the border a table closes with needs where a page ends inside one of its rows: under
//! the part of the row left there, and under the last row left there of a table in its cell.
//! Measured in Word for Windows on synthetic documents, the room on the page moved in steps of
//! 0.1 pt: a line stays where the table's closing border fits under it too, whether its row is
//! the table's last or not; a nested row stays where its bottom, the border its table closes with
//! and the outer row's bottom band fit; a row with nothing to split between, of an exact height or
//! short of its least height goes on whole.
use crate::*;
use wordcraft_doc::props::{Borders, HeightRule};
use wordcraft_doc::{Table, para_block};

/// The bottom of the text area of a default page.
const BOTTOM: f32 = 720.0;

fn one_line(text: &str) -> Arc<Block> {
    let mut p = Paragraph::with_text(text, Default::default());
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    para_block(p)
}

/// A table of `rows` rows with `lines` one-line paragraphs each, without cell margins above and
/// below, all its borders `border` wide.
fn nested(rows: usize, lines: usize, border: f32, height: Option<(f32, HeightRule)>) -> Table {
    let mut t = Table::new(rows, 1, 200.0);
    t.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    t.props.borders = Some(Borders::all(Border::single(border)));
    for (i, row) in t.rows.iter_mut().enumerate() {
        row.cells[0].blocks = (0..lines).map(|l| one_line(&format!("Row {i} line {l}"))).collect();
        if let Some((h, rule)) = height {
            (row.props.height, row.props.height_rule) = (Some(h), rule);
        }
    }
    t
}

/// A filler paragraph pushed `push` points down, then a one-row table, its borders `border`
/// wide, whose cell holds `inner` and an empty paragraph.
fn lay(push: f32, inner: &Table, border: f32) -> DocLayout {
    let mut d = Document::from_text("Filler");
    if let Some(Block::Para(p)) = d.body.first_mut().map(Arc::make_mut) {
        p.props.space_before = Some(push);
        p.props.space_after = Some(0.0);
    }
    let mut outer = Table::new(1, 1, 400.0);
    outer.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    outer.props.borders = Some(Borders::all(Border::single(border)));
    outer.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(inner.clone())), one_line("")];
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(outer)).unwrap();
    let mut c = LayoutCache::new();
    layout(&d, &mut c, &LayoutOptions::default())
}

/// The lines of nested row `r` on each page.
fn lines_of(l: &DocLayout, r: u32) -> Vec<usize> {
    l.pages
        .iter()
        .map(|pg| {
            pg.items
                .iter()
                .filter_map(|it| match it {
                    Placed::Lines { path, l0, l1, .. } => (path.0.len() == 7 && path.0[..5] == [1, 0, 0, 0, r]).then_some(l1 - l0),
                    _ => None,
                })
                .sum()
        })
        .collect()
}

/// The area of nested row `r` on page `pg`.
fn area(l: &DocLayout, pg: usize, r: usize) -> Option<Rect> {
    l.pages.get(pg)?.items.iter().find_map(|it| match it {
        Placed::Cell { rect, table, row, .. } if table.0.len() == 4 && *row == r => Some(*rect),
        _ => None,
    })
}

/// Push the table down in steps of 0.1 pt from where nested row `r` is well clear of the page's
/// bottom to where it has left the page: how far above the bottom of the text area it ends in
/// the last layout that keeps it there whole, and whether any layout leaves it on both pages.
fn last_stand(inner: &Table, border: f32, r: usize) -> (f32, bool) {
    let free = lay(0.0, inner, border);
    let bottom = area(&free, 0, r).expect("the row fits on the page of the unpushed table").bottom();
    let (mut gap, mut torn) = (f32::MAX, false);
    for step in 0..80 {
        let l = lay(BOTTOM - bottom - 6.0 + step as f32 * 0.1, inner, border);
        let on = lines_of(&l, r as u32);
        torn |= on.iter().filter(|&&n| n > 0).count() > 1;
        if on.first().is_some_and(|&n| n > 0) && on.iter().skip(1).all(|&n| n == 0) {
            gap = BOTTOM - area(&l, 0, r).expect("its area is where its text is").bottom();
        }
    }
    (gap, torn)
}

#[test]
fn a_nested_row_stays_where_its_tables_closing_border_fits_too() {
    // Word: with borders of 1.5 pt in the nested table and of 0.5 pt round the outer one, the
    // third row is the last one on the page from 2 pt of room under it, and with 0.5 pt borders
    // in both from 1 pt.
    for (inner_border, need) in [(1.5, 2.0), (0.5, 1.0)] {
        let (gap, torn) = last_stand(&nested(6, 1, inner_border, None), 0.5, 2);
        assert!((need - 0.01..need + 0.11).contains(&gap), "borders of {inner_border} pt: the row's last stand is {gap} pt above the bottom");
        assert!(!torn, "a one-line row is never on two pages");
    }
    // The table's last row has that border in its own height: under it only the outer band.
    let (gap, _) = last_stand(&nested(3, 1, 1.5, None), 0.5, 2);
    assert!((0.5 - 0.01..2.0 - 0.01).contains(&gap), "the last row ends {gap} pt above the bottom");
}

#[test]
fn a_nested_row_of_a_set_height_goes_on_whole() {
    // Word: rows of 15 pt, at least or exactly, with a line of less in them: the third row leaves
    // the page whole as soon as it and the two borders no longer fit.
    for rule in [HeightRule::AtLeast, HeightRule::Exact] {
        let inner = nested(6, 1, 0.5, Some((30.0, rule)));
        let (gap, torn) = last_stand(&inner, 0.5, 2);
        assert!((1.0 - 0.01..1.0 + 0.11).contains(&gap), "{rule:?}: the row's last stand is {gap} pt above the bottom");
        assert!(!torn, "{rule:?}: the row is never on two pages");
        // Where the page's bottom runs through it, neither its text nor its area stays, and it
        // goes on as tall as it is.
        let free = lay(0.0, &inner, 0.5);
        let whole = area(&free, 0, 2).expect("the row is on the page");
        let l = lay(BOTTOM - whole.bottom() + 5.0, &inner, 0.5);
        assert_eq!(lines_of(&l, 2)[..2], [0, 1], "{rule:?}: its line goes on");
        assert_eq!(lines_of(&l, 1).first(), Some(&1), "{rule:?}: the row above stays");
        assert!(area(&l, 0, 1).is_some() && area(&l, 0, 2).is_none(), "{rule:?}: {:?} of it stays", area(&l, 0, 2));
        assert!(area(&l, 1, 2).is_some_and(|a| (a.h - whole.h).abs() < 0.01), "{rule:?}: it goes on {:?} tall, of {}", area(&l, 1, 2), whole.h);
        // The border between it and the row above closes the table on the first page and opens
        // the row on the next.
        let rules = |pg: usize, y: f32| {
            l.pages[pg]
                .items
                .iter()
                .filter(|it| matches!(it, Placed::Rule { y0, y1, x1, .. } if (y0 - y1).abs() < 0.01 && (y0 - y).abs() < 0.01 && *x1 < 300.0))
                .count()
        };
        let (above, here) = (area(&l, 0, 1).map_or(0.0, |a| a.bottom()), area(&l, 1, 2).map_or(0.0, |a| a.y));
        assert!(rules(0, above) >= 1 && rules(1, here) >= 1, "{rule:?}: the border at {above} and at {here}");
    }
    // Word: a row of 300 pt with 120 pt of room goes on whole, with one line in it or with many.
    for (rule, lines) in [(HeightRule::Exact, 1), (HeightRule::AtLeast, 1), (HeightRule::Exact, 12), (HeightRule::AtLeast, 12)] {
        let inner = nested(1, lines, 0.5, Some((300.0, rule)));
        let top = area(&lay(0.0, &inner, 0.5), 0, 0).expect("the row is on the page").y;
        let on = lines_of(&lay(BOTTOM - top - 120.0, &inner, 0.5), 0);
        assert_eq!(on.first(), Some(&0), "{rule:?} with {lines} lines: {on:?}");
        assert_eq!(on.iter().sum::<usize>(), lines, "{rule:?} with {lines} lines: {on:?}");
    }
}

#[test]
fn a_nested_row_of_several_lines_splits_between_them_above_the_closing_border() {
    // Word: a row of two lines keeps its first line on the page from where that line, the nested
    // table's border and the outer one fit under it, and goes on whole before that.
    let inner = nested(6, 2, 1.5, None);
    let free = lay(0.0, &inner, 0.5);
    let top = area(&free, 0, 2).expect("the row is on the page").y;
    let line = (area(&free, 0, 2).map_or(0.0, |a| a.h) - 1.5) / 2.0;
    // The first line ends `line` below the row's top border; the borders need 2 pt under it.
    for (spare, kept) in [(-0.3, 0), (0.3, 1)] {
        let l = lay(BOTTOM - top - 1.5 - line - 2.0 - spare, &inner, 0.5);
        let on = lines_of(&l, 2);
        assert_eq!(on.first(), Some(&kept), "{spare} pt to spare: {on:?}");
        assert_eq!(on.iter().sum::<usize>(), 2, "{on:?}");
        assert_eq!(lines_of(&l, 1).first(), Some(&2), "the row above stays whole");
    }
}

#[test]
fn the_part_of_a_split_row_keeps_room_for_the_closing_border() {
    // Word: a row of six lines, the first of two rows, with borders of 1.5 pt keeps its third
    // line from 1.5 pt of room under it, as the table's only row does.
    for rows in [2, 1] {
        let mut t = Table::new(rows, 1, 400.0);
        t.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
        t.props.borders = Some(Borders::all(Border::single(1.5)));
        t.rows[0].cells[0].blocks = (0..6).map(|l| one_line(&format!("Line {l}"))).collect();
        let lay = |push: f32| {
            let mut d = Document::from_text("Filler");
            if let Some(Block::Para(p)) = d.body.first_mut().map(Arc::make_mut) {
                (p.props.space_before, p.props.space_after) = (Some(push), Some(0.0));
            }
            d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t.clone())).unwrap();
            let mut c = LayoutCache::new();
            layout(&d, &mut c, &LayoutOptions::default())
        };
        // Where the third line ends, on the page it is on.
        let third = |l: &DocLayout| {
            l.pages.iter().enumerate().find_map(|(pg, page)| {
                page.items.iter().find_map(|it| match it {
                    Placed::Lines { path, para, y, .. } if path.0 == [1, 0, 0, 2] => Some((pg, y + para.lines.first().map_or(0.0, |l| l.height))),
                    _ => None,
                })
            })
        };
        let free = third(&lay(0.0)).expect("the line is laid out").1;
        let mut gap = f32::MAX;
        for step in 0..80 {
            let l = lay(BOTTOM - free - 6.0 + step as f32 * 0.1);
            if let Some((0, bottom)) = third(&l) {
                gap = BOTTOM - bottom;
            }
        }
        assert!((1.5 - 0.01..1.5 + 0.11).contains(&gap), "{rows} row(s): the line's last stand is {gap} pt above the bottom");
    }
}
