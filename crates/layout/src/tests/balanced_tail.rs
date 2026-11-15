//! The last page of a section whose columns are balanced before a continuous break, when it holds
//! only the end of a paragraph that began on the page before, or a whole paragraph of two or three
//! lines (#6821). Measured in Word for Windows with exact 14 pt lines and no spacing, in three and
//! in two columns, one case per document, saved as compatibility mode 15 and as 14: from mode 15
//! Word keeps such lines together in the first column that can hold them (the later columns may be
//! left short or empty); in mode 14 it splits two lines one per column. Not measured: balanced
//! columns beside footnotes or floating objects, and an unbalanced column left with room for one
//! line (it takes a note or a float to make it that short).
use crate::*;
use wordcraft_doc::props::{LineSpacing, ParaProps};
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{Pos, para_block};

const LINE: f32 = 14.0;

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn para(text: &str) -> Arc<Block> {
    let chr = wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() };
    let mut p = Paragraph::with_text(text, chr);
    p.props = ParaProps { space_before: Some(0.0), space_after: Some(0.0), line_spacing: Some(LineSpacing::Exactly(LINE)), ..Default::default() };
    para_block(p)
}

/// A paragraph of `k` lines, each ended by a line break: "P1", "P2"…
fn lines_para(k: usize) -> String {
    (1..=k).map(|i| format!("P{i}")).collect::<Vec<_>>().join(&wordcraft_doc::para::LINE_BREAK.to_string())
}

/// One-line paragraphs, then a paragraph of `k` lines, then optionally one more line ("Q1"), all
/// in `cols` columns balanced before a continuous break, then "After" in one column.
struct Case {
    cols: u32,
    /// How many one-line paragraphs come first, relative to the lines on a page: `-3` leaves room
    /// for three lines of the paragraph, `0` starts the paragraph on the next page, `1` puts one
    /// more line before it there.
    from_page_end: i32,
    k: usize,
    q: bool,
    compat: u32,
}

impl Case {
    fn new(cols: u32, from_page_end: i32, k: usize) -> Case {
        Case { cols, from_page_end, k, q: false, compat: 15 }
    }
}

/// The column (counted from the left, 0 to `cols - 1`) of each line of the `k`-line paragraph,
/// and the number of pages.
fn columns(c: &Case) -> (Vec<usize>, usize) {
    crate::test_fonts();
    let mut d = Document::new();
    d.settings.compat_mode = c.compat;
    let mut s = SectionProps { start: SectionStart::Continuous, ..Default::default() };
    s.columns.count = c.cols;
    let per_col = ((s.page_h - s.margin_top - s.margin_bottom) / LINE).floor() as i32;
    let fillers = (per_col * c.cols as i32 + c.from_page_end).max(0) as usize;
    let mut body: Vec<Arc<Block>> = (0..fillers).map(|i| para(&format!("r{i}"))).collect();
    let first = body.len();
    body.push(para(&lines_para(c.k)));
    if c.q {
        body.push(para("Q1"));
    }
    let mark = body.len();
    body.push(para(""));
    body.push(para("After"));
    d.body = body;
    if let Some(Block::Para(p)) = d.body.get_mut(mark).map(Arc::make_mut) {
        p.section = Some(Box::new(s));
    }
    d.last_section.start = SectionStart::Continuous;
    let l = lay(&d);
    let text = match d.body.get(first).map(|b| &**b) {
        Some(Block::Para(p)) => p.text.clone(),
        _ => String::new(),
    };
    let starts = std::iter::once(0).chain(text.char_indices().filter(|(_, ch)| *ch == wordcraft_doc::para::LINE_BREAK).map(|(i, ch)| i + ch.len_utf8()));
    let carets: Vec<_> = starts.map(|o| l.caret(&Pos::body(first, o)).unwrap()).collect();
    // Columns by the order of their left edges, with the one-column "After" at the first's.
    let mut xs = vec![l.caret(&Pos::body(mark + 1, 0)).unwrap().x];
    for x in carets.iter().map(|c| c.x) {
        if !xs.iter().any(|e| (e - x).abs() < 1.0) {
            xs.push(x);
        }
    }
    xs.sort_by(f32::total_cmp);
    let cols = carets.iter().map(|c| xs.iter().position(|e| (e - c.x).abs() < 1.0).unwrap_or(usize::MAX)).collect();
    (cols, l.pages.len())
}

// A: the last two lines of a paragraph whose first three lines closed the page before.
#[test]
fn the_last_two_lines_of_a_continued_paragraph_stay_in_the_first_column() {
    for cols in [3, 2] {
        let (v, pages) = columns(&Case::new(cols, -3, 5));
        assert_eq!(pages, 2, "{cols} columns");
        assert_eq!(v[3..], [0, 0], "{cols} columns: lines 4 and 5 together");
    }
}

// B: three lines stay together too: not two and one, nor one and two.
#[test]
fn the_last_three_lines_of_a_continued_paragraph_stay_in_the_first_column() {
    for cols in [3, 2] {
        let (v, _) = columns(&Case::new(cols, -3, 6));
        assert_eq!(v[3..], [0, 0, 0], "{cols} columns");
    }
}

// L: five lines are three and two (not two, one and two).
#[test]
fn the_last_five_lines_of_a_continued_paragraph_are_three_and_two() {
    for cols in [3, 2] {
        let (v, _) = columns(&Case::new(cols, -3, 8));
        assert_eq!(v[3..], [0, 0, 0, 1, 1], "{cols} columns");
    }
}

// N: a one-line paragraph after the two lines goes to the second column.
#[test]
fn a_line_after_the_continued_two_lines_goes_to_the_next_column() {
    for cols in [3, 2] {
        let c = Case { q: true, ..Case::new(cols, -3, 5) };
        assert_eq!(columns(&c).0[3..], [0, 0], "{cols} columns");
    }
}

// K, E: a paragraph of three lines (or two) that begins the last page stays together.
#[test]
fn a_whole_paragraph_of_two_or_three_lines_at_the_top_of_the_last_page_stays_together() {
    for cols in [3, 2] {
        assert_eq!(columns(&Case::new(cols, 0, 3)).0, [0, 0, 0], "{cols} columns, three lines");
        assert_eq!(columns(&Case::new(cols, 0, 2)).0, [0, 0], "{cols} columns, two lines");
    }
}

// K2: the same after one line of another paragraph in the first column: it starts the second.
#[test]
fn a_three_line_paragraph_after_one_line_on_the_last_page_stays_together() {
    for cols in [3, 2] {
        let (v, pages) = columns(&Case::new(cols, 1, 3));
        assert_eq!(pages, 2, "{cols} columns");
        assert_eq!(v, [1, 1, 1], "{cols} columns");
    }
}

// Guards, the same in Word and before the change.

// C, M: four lines are two and two, six are two, two and two.
#[test]
fn longer_tails_are_balanced_as_before() {
    for cols in [3, 2] {
        assert_eq!(columns(&Case::new(cols, -3, 7)).0[3..], [0, 0, 1, 1], "{cols} columns, four lines");
    }
    assert_eq!(columns(&Case::new(3, -3, 9)).0[3..], [0, 0, 1, 1, 2, 2], "three columns, six lines");
}

// O: a four-line paragraph that begins the last page is two and two.
#[test]
fn a_four_line_paragraph_at_the_top_of_the_last_page_is_two_and_two() {
    for cols in [3, 2] {
        assert_eq!(columns(&Case::new(cols, 0, 4)).0, [0, 0, 1, 1], "{cols} columns");
    }
}

// Before Word 2013's layout (mode 14) the two lines are split one per column, as Word does.
#[test]
fn mode_14_still_splits_the_last_two_lines_of_a_continued_paragraph() {
    for cols in [3, 2] {
        let c = Case { compat: 14, ..Case::new(cols, -3, 5) };
        assert_eq!(columns(&c).0[3..], [0, 1], "{cols} columns");
    }
}
