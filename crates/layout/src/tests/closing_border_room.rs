//! The room the border a table closes with needs where a page ends inside one of its rows: under
//! the part of the row left there, and under the last row left there of a table in its cell.
//! Measured in Word for Windows on synthetic documents, the room on the page moved in steps of
//! 0.1 pt: a line stays where the table's closing border fits under it too, whether its row is
//! the table's last or not; a nested row stays where its bottom, the border its table closes with
//! and the outer row's bottom band fit; a row with nothing to split between, of an exact height or
//! short of its least height goes on whole, and so does the row around it when its cell would keep
//! nothing.
use crate::*;
use wordcraft_doc::para::{Anchor, Float, Wrap};
use wordcraft_doc::props::{Borders, CellPicture, HeightRule, PictureRepeat, TextDirection};
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

/// A one-row table of `cols` cells, its borders `border` wide, whose first cell holds `inner`
/// and an empty paragraph.
fn around(inner: &Table, border: f32, cols: usize) -> Table {
    let mut outer = Table::new(1, cols, 400.0);
    outer.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    outer.props.borders = Some(Borders::all(Border::single(border)));
    outer.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(inner.clone())), one_line("")];
    outer
}

/// The one paragraph of `d` pushed `push` points down, then the table `outer`.
fn lay_in(d: &Document, push: f32, outer: &Table) -> DocLayout {
    let mut d = d.clone();
    if let Some(Block::Para(p)) = d.body.first_mut().map(Arc::make_mut) {
        p.props.space_before = Some(push);
        p.props.space_after = Some(0.0);
    }
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(outer.clone())).unwrap();
    let mut c = LayoutCache::new();
    layout(&d, &mut c, &LayoutOptions::default())
}

/// A filler paragraph pushed `push` points down, then the table `outer`.
fn lay_table(push: f32, outer: &Table) -> DocLayout {
    lay_in(&Document::from_text("Filler"), push, outer)
}

/// A filler paragraph pushed `push` points down, then a one-row table, its borders `border`
/// wide, whose cell holds `inner` and an empty paragraph.
fn lay(push: f32, inner: &Table, border: f32) -> DocLayout {
    lay_table(push, &around(inner, border, 1))
}

/// The pages each row of the table at `table` has an area on, and the pages its text is on.
fn pages_of(l: &DocLayout, table: &[u32], rows: usize) -> Vec<(Vec<usize>, Vec<usize>)> {
    (0..rows)
        .map(|r| {
            let on = |hit: &dyn Fn(&Placed) -> bool| (0..l.pages.len()).filter(|&pg| l.pages[pg].items.iter().any(hit)).collect::<Vec<_>>();
            let areas = on(&|it| matches!(it, Placed::Cell { table: t, row, .. } if t.0 == table && *row == r));
            let text = on(&|it| matches!(it, Placed::Lines { path, .. } if path.0.len() > table.len() && path.0[..table.len()] == *table && path.0[table.len()] as usize == r));
            (areas, text)
        })
        .collect()
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

/// The last paragraph of each cell of `t` with `after` points of space after it.
fn space_after(mut t: Table, after: f32) -> Table {
    for cell in t.rows.iter_mut().flat_map(|r| &mut r.cells) {
        if let Some(Block::Para(p)) = cell.blocks.last_mut().map(Arc::make_mut) {
            p.props.space_after = Some(after);
        }
    }
    t
}

/// How many lines of the only row of `inner` are on each page with `room` points left for the
/// row around it.
fn lines_with_room(inner: &Table, room: f32) -> Vec<usize> {
    // The outer row starts its top border above the nested one.
    let top = area(&lay(0.0, inner, 0.5), 0, 0).expect("the row is on the page").y - 0.5;
    lines_of(&lay(BOTTOM - top - room, inner, 0.5), 0)
}

#[test]
fn the_last_paragraph_of_a_nested_cell_needs_its_space_after() {
    // Word, with 120 pt of room: of two one-line paragraphs in a nested cell, the second with
    // 280 pt after it, the first stays and the second goes on; of five, four stay.
    for lines in [2, 5] {
        let on = lines_with_room(&space_after(nested(1, lines, 0.5, None), 280.0), 120.0);
        assert_eq!(on[..2], [lines - 1, 1], "{lines} lines: {on:?}");
    }
    // One paragraph with that space after it has nothing to leave behind: its row goes on whole.
    let on = lines_with_room(&space_after(nested(1, 1, 0.5, None), 280.0), 120.0);
    assert_eq!(on[..2], [0, 1], "{on:?}");
    // And so does a row of two lines above a bottom cell margin of 280 pt: the margin is the
    // row's, under whichever line is its last on the page.
    let mut inner = nested(1, 2, 0.5, None);
    inner.props.cell_margins = Some([0.0, 5.4, 280.0, 5.4]);
    let on = lines_with_room(&inner, 120.0);
    assert_eq!(on[..2], [0, 2], "{on:?}");
}

#[test]
fn a_nested_at_least_row_needs_its_least_height_with_the_cell_margins() {
    // Word: a nested row of at least 60 pt with cell margins of 10 pt above and below it and
    // twelve lines goes on whole with up to 81 pt of room and splits from 83 pt: its least height
    // is the 60 pt, the margins and its borders, and the outer borders take 1 pt more.
    let mut inner = nested(1, 12, 0.5, Some((60.0, HeightRule::AtLeast)));
    inner.props.cell_margins = Some([10.0, 5.4, 10.0, 5.4]);
    for room in [62.0, 70.0, 81.0] {
        let on = lines_with_room(&inner, room);
        assert_eq!(on[..2], [0, 12], "{room} pt of room: {on:?}");
    }
    let on = lines_with_room(&inner, 83.0);
    assert!((1..12).contains(&on[0]) && on.iter().sum::<usize>() == 12, "83 pt of room: {on:?}");
}

#[test]
fn a_row_goes_on_whole_when_the_nested_row_its_cell_starts_with_does() {
    // Word: beside a cell of long text, a cell that starts with a nested row of 300 pt, exact or
    // at least, takes the whole row on with 120 pt of room: none of the text stays beside an
    // empty cell.
    for rule in [HeightRule::Exact, HeightRule::AtLeast] {
        let inner = nested(1, 1, 0.5, Some((300.0, rule)));
        let mut outer = around(&inner, 0.5, 2);
        outer.rows[0].cells[1].blocks = (0..12).map(|l| one_line(&format!("Beside {l}"))).collect();
        let top = area(&lay_table(0.0, &outer), 0, 0).expect("the row is on the page").y - 0.5;
        let l = lay_table(BOTTOM - top - 120.0, &outer);
        let beside: Vec<usize> = l
            .pages
            .iter()
            .map(|pg| pg.items.iter().filter(|it| matches!(it, Placed::Lines { path, .. } if path.0.len() == 4 && path.0[..3] == [1, 0, 1])).count())
            .collect();
        assert_eq!(beside[..2], [0, 12], "{rule:?}: the text beside it");
        assert_eq!(lines_of(&l, 0)[..2], [0, 1], "{rule:?}: its own line");
    }
}

/// A paragraph that takes `lines` lines in a cell of [`nested`], with widow control on or off.
fn wrapped(lines: usize, widow: bool) -> Arc<Block> {
    let para = |words: usize| {
        let mut p = Paragraph::with_text(&"word ".repeat(words), Default::default());
        (p.props.space_before, p.props.space_after, p.props.widow_control) = (Some(0.0), Some(0.0), Some(widow));
        para_block(p)
    };
    let taken = |words: usize| {
        let mut t = nested(1, 1, 0.5, None);
        t.rows[0].cells[0].blocks = vec![para(words)];
        lay(0.0, &t, 0.5).pages[0].items.iter().find_map(|it| match it {
            Placed::Lines { path, para, .. } if path.0.len() == 7 => Some(para.lines.len()),
            _ => None,
        })
    };
    para((1..200).find(|&words| taken(words) == Some(lines)).expect("some number of words takes that many lines"))
}

#[test]
fn widow_control_takes_a_nested_row_on_whole() {
    // Word, a nested row of one line and under it a row of one paragraph: with room for the
    // first line of two, or for two of three, the paragraph goes on whole with widow control, and
    // its row with it, nothing of it left behind; without, the lines that fit stay.
    for (lines, widow, kept) in [(2, true, 0), (2, false, 1), (3, true, 0), (3, false, 2)] {
        let mut inner = nested(2, 1, 0.5, None);
        inner.rows[1].cells[0].blocks = vec![wrapped(lines, widow)];
        let free = lay(0.0, &inner, 0.5);
        let row = area(&free, 0, 1).expect("the row is on the page");
        let line = (row.h - 1.0) / lines as f32;
        // Room for all its lines but the last, the nested table's border and the outer one, and
        // 0.3 pt to spare.
        let l = lay(BOTTOM - (row.y + 0.5 + line * (lines - 1) as f32) - 1.0 - 0.3, &inner, 0.5);
        let on = lines_of(&l, 1);
        assert_eq!(on[..2], [kept, lines - kept], "{lines} lines, widow control {widow}");
        assert_eq!(lines_of(&l, 0).first(), Some(&1), "the row above stays");
        assert_eq!(area(&l, 0, 1).is_some(), kept > 0, "{lines} lines, widow control {widow}: what stays of its area");
    }
}

/// Lay out `inner`, a table of three rows the third of which can't split, in a cell, first clear
/// of the page's bottom and then with the bottom 5 pt above the third row's, which goes on whole:
/// what `things` finds on the first page when clear, then left there and gone on, with the third
/// row's area on the next page.
fn with_the_third_row(d: &Document, inner: &Table, things: impl Fn(&Placed) -> Option<Rect>) -> (Vec<Rect>, Vec<Rect>, Vec<Rect>, Rect) {
    let on = |l: &DocLayout, pg: usize| l.pages[pg].items.iter().filter_map(&things).collect::<Vec<_>>();
    let outer = around(inner, 0.5, 1);
    let free = lay_in(d, 0.0, &outer);
    let whole = area(&free, 0, 2).expect("the row is on the page");
    let l = lay_in(d, BOTTOM - whole.bottom() + 5.0, &outer);
    (on(&free, 0), on(&l, 0), on(&l, 1), area(&l, 1, 2).expect("the row goes on"))
}

#[test]
fn what_a_nested_row_holds_goes_on_with_it() {
    let d = Document::from_text("Filler");
    let rows = || nested(3, 1, 0.5, Some((40.0, HeightRule::Exact)));
    // A floating picture at the row's top that would fit above the page's bottom by itself, and
    // its frame.
    let mut inner = rows();
    let mut p = Paragraph::with_text("Row 2 line 0", Default::default());
    (p.props.space_before, p.props.space_after) = (Some(0.0), Some(0.0));
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 100.0, y: 2.0, ..Default::default() };
    p.insert_object(0, super::picture(10.0, 10.0, float), &Default::default()).unwrap();
    inner.rows[2].cells[0].blocks = vec![para_block(p)];
    let picture = |it: &Placed| match it {
        Placed::Shape { rect, .. } | Placed::Object { rect, .. } => Some(*rect),
        _ => None,
    };
    let (free, left, moved, row) = with_the_third_row(&d, &inner, picture);
    assert_eq!((free.len(), left.len(), moved.len()), (2, 0, 2), "the picture and its frame: {left:?} stay, {moved:?} go on");
    assert!(moved.iter().all(|r| r.y >= row.y && r.bottom() <= row.bottom()), "inside its row: {moved:?} in {row:?}");
    // The cell's background picture: with its cell, not cut where the page ends.
    let mut d = d;
    let media = d.add_media(vec![0u8; 8], "png");
    let mut inner = rows();
    inner.rows[2].cells[0].props.picture = Some(Box::new(CellPicture { media, size: [30.0, 6.0], repeat: PictureRepeat::Tile, align: [0.0, 0.0] }));
    let clip = |it: &Placed| if let Placed::Clip { rect, .. } = it { Some(*rect) } else { None };
    let (free, left, moved, row) = with_the_third_row(&d, &inner, clip);
    assert_eq!((free.len(), left.len(), moved.len()), (1, 0, 1), "the background: {left:?} stays, {moved:?} goes on");
    assert!((moved[0].y - row.y).abs() < 0.01 && (moved[0].h - row.h).abs() < 0.01, "as big as its cell: {moved:?} in {row:?}");
}

#[test]
fn turned_text_goes_on_with_its_nested_row() {
    // Turned text is as long as its row is tall, and ends above the row's bottom border: where
    // the row itself would fit but the borders that close the tables under it don't, the row goes
    // on whole, its turned text too.
    let mut inner = Table::new(3, 2, 200.0);
    inner.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    inner.props.borders = Some(Borders::all(Border::single(1.5)));
    for cell in inner.rows.iter_mut().flat_map(|r| &mut r.cells) {
        cell.blocks = vec![one_line("Cell")];
    }
    inner.rows[1].cells[0].props.text_direction = TextDirection::Down;
    inner.rows[1].cells[0].blocks = vec![one_line("T")];
    let outer = around(&inner, 0.5, 1);
    let bottom = area(&lay_table(0.0, &outer), 0, 1).expect("the row is on the page").bottom();
    let mut gone = 0;
    for step in 0..80 {
        let push = BOTTOM - bottom - 6.0 + step as f32 * 0.1;
        let rows = pages_of(&lay_table(push, &outer), &[1, 0, 0, 0], 3);
        let (areas, text) = &rows[1];
        assert_eq!(areas, text, "pushed {push}: the row and its text");
        gone += usize::from(areas == &[1] && rows[0].0 == [0]);
    }
    assert!(gone > 10, "the row leaves the one above it behind in {gone} of the layouts");
}

#[test]
fn no_fragment_of_a_row_stays_in_the_outer_rows_bottom_band() {
    // The outer cell has a bottom margin of 10 pt: text ends that far above where the part of
    // the row left on the page does. Under a paragraph with 8 pt of space after it, the first
    // row of a nested table can start in between: it has no room there either and goes on whole,
    // like a row starting higher.
    let inner = nested(3, 1, 0.5, Some((30.0, HeightRule::Exact)));
    let mut outer = around(&inner, 0.5, 1);
    outer.props.cell_margins = Some([0.0, 5.4, 10.0, 5.4]);
    let mut above = Paragraph::with_text("Above", Default::default());
    (above.props.space_before, above.props.space_after) = (Some(0.0), Some(8.0));
    outer.rows[0].cells[0].blocks.insert(0, para_block(above));
    let table: &[u32] = &[1, 0, 0, 1];
    let top = |l: &DocLayout| {
        l.pages[0].items.iter().find_map(|it| match it {
            Placed::Cell { rect, table: t, row: 0, .. } if t.0 == table => Some(rect.y),
            _ => None,
        })
    };
    let free = top(&lay_table(0.0, &outer)).expect("the row is on the page");
    let mut between = 0;
    for step in 0..300 {
        // The page's bottom from 5 pt above the nested table's top to 25 pt below it.
        let push = BOTTOM - free - 25.0 + step as f32 * 0.1;
        let l = lay_table(push, &outer);
        for (r, (areas, text)) in pages_of(&l, table, 3).into_iter().enumerate() {
            assert_eq!(areas.len(), 1, "pushed {push}: row {r} has an area on pages {areas:?}");
            assert_eq!(areas, text, "pushed {push}: row {r} and its text");
        }
        // The paragraph above stays while the table has gone on.
        let stays = l.pages[0].items.iter().any(|it| matches!(it, Placed::Lines { path, .. } if path.0 == [1, 0, 0, 0]));
        between += usize::from(stays && top(&l).is_none());
    }
    assert!(between > 50, "the table goes on from under the paragraph in {between} of the layouts");
}

#[test]
fn tables_side_by_side_in_a_cell_break_each_at_its_own_row() {
    // A table of one row whose two cells hold a table each, rows of 60 pt in one and of 25 pt
    // in the other: where the page ends, each keeps the rows that fit, whatever the other one
    // does beside it, and every row is where its text is.
    let (tall, short) = (nested(4, 1, 0.5, Some((60.0, HeightRule::Exact))), nested(8, 1, 0.5, Some((25.0, HeightRule::Exact))));
    let mut pair = Table::new(1, 2, 390.0);
    pair.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    pair.props.borders = Some(Borders::all(Border::single(0.5)));
    pair.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(tall)), one_line("")];
    pair.rows[0].cells[1].blocks = vec![Arc::new(Block::Table(short)), one_line("")];
    let (left, right): (&[u32], &[u32]) = (&[1, 0, 0, 0, 0, 0, 0], &[1, 0, 0, 0, 0, 1, 0]);
    let free = lay(0.0, &pair, 0.5);
    let top = area(&free, 0, 0).expect("the row is on the page").y;
    let mut kept = std::collections::BTreeSet::new();
    for step in 0..900 {
        // The page's bottom from 30 pt to 120 pt below the top of the two tables.
        let room = 30.0 + step as f32 * 0.1;
        let l = lay(BOTTOM - top - room, &pair, 0.5);
        let rows = |table: &[u32], n: usize| {
            let pages = pages_of(&l, table, n);
            for (r, (areas, text)) in pages.iter().enumerate() {
                assert_eq!(areas.len(), 1, "{room} pt of room: row {r} of {table:?} has an area on pages {areas:?}");
                assert_eq!(areas, text, "{room} pt of room: row {r} of {table:?} and its text");
            }
            pages.iter().filter(|(areas, _)| areas == &[0]).count()
        };
        kept.insert((rows(left, 4), rows(right, 8)));
    }
    // One keeps its first row while the other keeps two and three of its own.
    assert!(kept.contains(&(1, 2)) && kept.contains(&(1, 3)), "{kept:?}");
}
