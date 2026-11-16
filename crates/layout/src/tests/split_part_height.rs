//! The part of a split at-least row left on its page: its cells are lined up within the row's
//! least height, not only against the text the other cells keep. Measured in Word for Windows on
//! synthetic documents: a row of at least 12.75 pt that keeps one line of 9.2 pt has that line
//! 1.8 pt down in a cell aligned to the middle and 3.6 pt down in one aligned to the bottom,
//! whether the cell's text goes on to the next page or not; rows of at least 20, 30 and 50 pt
//! keeping two, three and five lines the same way; a row without a height keeps them at the top.
use crate::*;
use wordcraft_doc::props::{HeightRule, VAlign};
use wordcraft_doc::{Table, para_block};

/// The bottom of the text area of a default page.
const BOTTOM: f32 = 720.0;

fn lines(what: &str, n: usize) -> Vec<Arc<Block>> {
    (0..n)
        .map(|i| {
            let mut p = Paragraph::with_text(&format!("{what} {i}"), Default::default());
            (p.props.space_before, p.props.space_after) = (Some(0.0), Some(0.0));
            para_block(p)
        })
        .collect()
}

/// A table whose first row holds four lines in a top and a middle cell and one line in a middle
/// and a bottom cell, `push` points below a filler paragraph; the row is at least `least` tall.
fn lay(push: f32, least: Option<f32>) -> DocLayout {
    let mut d = Document::from_text("Filler");
    if let Some(Block::Para(p)) = d.body.first_mut().map(Arc::make_mut) {
        (p.props.space_before, p.props.space_after) = (Some(push), Some(0.0));
    }
    let mut t = Table::new(2, 4, 440.0);
    t.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    (t.rows[0].props.height, t.rows[0].props.height_rule) = (least, HeightRule::AtLeast);
    for (cell, (n, valign)) in t.rows[0].cells.iter_mut().zip([(4, VAlign::Top), (4, VAlign::Center), (1, VAlign::Center), (1, VAlign::Bottom)]) {
        cell.blocks = lines("Line", n);
        cell.props.valign = Some(valign);
    }
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
    let mut c = LayoutCache::new();
    layout(&d, &mut c, &LayoutOptions::default())
}

/// The tops of the lines of cell `c` of the first row on page `pg`.
fn tops(l: &DocLayout, pg: usize, c: u32) -> Vec<f32> {
    let mut ys: Vec<f32> = l.pages[pg]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { path, y, .. } if path.0.len() == 4 && path.0[..3] == [1, 0, c] => Some(*y),
            _ => None,
        })
        .collect();
    ys.sort_by(f32::total_cmp);
    ys
}

#[test]
fn the_part_of_an_at_least_row_lines_its_cells_up_within_the_rows_least_height() {
    crate::test_fonts();
    // The row's top and its line height, from the table where it fits.
    let free = lay(0.0, None);
    let whole = tops(&free, 0, 0);
    let (top, line) = (whole[0], whole[1] - whole[0]);
    // Room for the least height and three lines, not for four.
    for (least, slack) in [(Some(3.0 * line + 6.0), 6.0), (None, 0.0)] {
        let l = lay(BOTTOM - top - 3.0 * line - 8.0, least);
        let cell = |c: u32| tops(&l, 0, c);
        let a = cell(0);
        assert_eq!((a.len(), cell(1).len()), (3, 3), "{least:?}: three of the four lines stay");
        let down = |c: u32| cell(c).first().map_or(f32::NAN, |y| y - a[0]);
        // A middle cell whose text goes on: half of what the least height leaves over its lines.
        assert!((down(1) - slack / 2.0).abs() < 0.05, "{least:?}: the middle cell that goes on is {} pt down", down(1));
        // The one-line cells: in the middle and at the bottom of the three lines and the rest.
        assert!((down(2) - (line + slack / 2.0)).abs() < 0.05, "{least:?}: the middle cell is {} pt down, a line is {line}", down(2));
        assert!((down(3) - (2.0 * line + slack)).abs() < 0.05, "{least:?}: the bottom cell is {} pt down, a line is {line}", down(3));
    }
}
