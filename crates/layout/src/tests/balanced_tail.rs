//! The last page of a section whose columns are balanced before a continuous break, when it holds
//! only the end of a paragraph that began on the page before, or a whole paragraph of two or three
//! lines (#6821). Measured in Word for Windows with exact 14 pt lines and no spacing, in three and
//! in two columns, one case per document, saved as compatibility mode 15 and as 14: from mode 15
//! Word keeps such lines together in the first column that can hold them (the later columns may be
//! left short or empty), keeps a paragraph with "keep lines together" or "keep with next" whole
//! there when those rules fit, including in a section that starts and ends on one page. In mode
//! 14 Word splits two lines one per column. Synthetic tests also cover a footnote and a floating
//! picture. Mode 14's other layouts (a three-line tail, two and one; the text after the section)
//! and keep with next on the last lines of a paragraph that began on the page before remain open.
use crate::*;
use wordcraft_doc::para::{Anchor, Float, InlineObject, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{LineSpacing, ParaProps};
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{PartKind, Pos, para_block};

const LINE: f32 = 14.0;

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn para_with(text: &str, keep_lines: bool, keep_next: bool) -> Arc<Block> {
    let chr = wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() };
    let mut p = Paragraph::with_text(text, chr);
    p.props = ParaProps {
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Exactly(LINE)),
        keep_lines: keep_lines.then_some(true),
        keep_next: keep_next.then_some(true),
        ..Default::default()
    };
    para_block(p)
}

fn para(text: &str) -> Arc<Block> {
    para_with(text, false, false)
}

/// A paragraph of `k` lines, each ended by a line break: "P1", "P2"…
fn lines_text(k: usize) -> String {
    (1..=k).map(|i| format!("P{i}")).collect::<Vec<_>>().join(&wordcraft_doc::para::LINE_BREAK.to_string())
}

#[derive(Clone, Copy, PartialEq)]
enum Keep {
    No,
    Lines,
    Next,
}

/// What follows the filler paragraphs: a paragraph of `k` lines ("P1"…), then optionally one more
/// line ("Q1"), all in `cols` columns, then "After" in one column.
#[derive(Clone, Copy)]
struct Case {
    cols: u32,
    /// How many one-line paragraphs come first, relative to the lines on a page: `-3` leaves room
    /// for three lines of the paragraph, `0` starts the paragraph on the next page, `1` puts one
    /// more line before it there. With `intro` it is the plain number of them.
    from_page_end: i32,
    k: usize,
    q: bool,
    compat: u32,
    /// A footnote referenced at the end of the paragraph.
    note: bool,
    /// A floating 28 pt tall picture-sized shape anchored in "Q1" (needs `q`).
    float: Option<Wrap>,
    /// This many lines of text and a one-column section break before the columns: the section
    /// starts mid-page and ends on the same page.
    intro: usize,
    /// Columns closed by a continuous break (balanced) or running to the end of the document.
    balanced: bool,
    keep: Keep,
    /// A page of this many lines of text (for a page too short for the rules).
    page_lines: Option<i32>,
}

impl Case {
    fn new(cols: u32, from_page_end: i32, k: usize) -> Case {
        Case { cols, from_page_end, k, q: false, compat: 15, note: false, float: None, intro: 0, balanced: true, keep: Keep::No, page_lines: None }
    }
}

/// The document of a case, the index of the `k`-line paragraph and of "Q1", and the offsets of the
/// paragraph's lines.
fn build(c: &Case) -> (Document, usize, Option<usize>, Vec<usize>) {
    crate::test_fonts();
    let mut d = Document::new();
    d.settings.compat_mode = c.compat;
    let mut s = SectionProps { start: SectionStart::Continuous, ..Default::default() };
    s.columns.count = c.cols;
    if let Some(n) = c.page_lines {
        s.page_h = s.margin_top + s.margin_bottom + n as f32 * LINE + 0.5;
        d.last_section.page_h = s.page_h;
    }
    let per_col = ((s.page_h - s.margin_top - s.margin_bottom) / LINE).floor() as i32;
    let fillers = if c.intro > 0 { c.from_page_end.max(0) } else { (per_col * c.cols as i32 + c.from_page_end).max(0) } as usize;
    let mut body: Vec<Arc<Block>> = (0..c.intro).map(|i| para(&format!("i{i}"))).collect();
    if c.intro > 0 {
        let mut one = SectionProps { start: SectionStart::Continuous, ..Default::default() };
        one.columns.count = 1;
        let mut mark = para("");
        if let Block::Para(p) = Arc::make_mut(&mut mark) {
            p.section = Some(Box::new(one));
        }
        body.push(mark);
    }
    body.extend((0..fillers).map(|i| para(&format!("r{i}"))));
    let first = body.len();
    let text = lines_text(c.k);
    body.push(para_with(&text, c.keep == Keep::Lines, c.keep == Keep::Next));
    let q = c.q.then_some(body.len());
    if c.q {
        body.push(para("Q1"));
    }
    let mark = body.len();
    if c.balanced {
        body.push(para(""));
        body.push(para("After"));
    }
    d.body = body;
    if c.balanced {
        if let Some(Block::Para(p)) = d.body.get_mut(mark).map(Arc::make_mut) {
            p.section = Some(Box::new(s));
        }
        d.last_section.start = SectionStart::Continuous;
    } else {
        d.last_section.columns.count = c.cols;
    }
    if let (Some(w), Some(qi)) = (c.float, q) {
        let float = Float { wrap: w, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 0.0, y: 0.0, ..Default::default() };
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: if w == Wrap::TopAndBottom { 100.0 } else { 60.0 },
            h: 28.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float,
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        d.insert_object(&Pos::body(qi, 0), shape, &Default::default()).unwrap();
    }
    let breaks = text.char_indices().filter(|(_, ch)| *ch == wordcraft_doc::para::LINE_BREAK);
    let starts: Vec<usize> = std::iter::once(0).chain(breaks.map(|(i, ch)| i + ch.len_utf8())).collect();
    if c.note {
        let id = d.add_part(PartKind::Footnote, vec![para("Note")]);
        let end = text.len();
        d.insert_object(&Pos::body(first, end), InlineObject::NoteRef { kind: NoteKind::Footnote, id, custom: String::new() }, &Default::default())
            .unwrap();
    }
    (d, first, q, starts)
}

/// The columns of the lines of the `k`-line paragraph (counted from the left, 0 to `cols - 1`),
/// of "Q1" if there is one, and the number of pages.
fn columns(c: &Case) -> (Vec<usize>, Option<usize>, usize) {
    let (d, first, q, starts) = build(c);
    let l = lay(&d);
    let carets: Vec<_> = starts.iter().map(|&o| l.caret(&Pos::body(first, o)).unwrap()).collect();
    let q_caret = q.map(|qi| l.caret(&Pos::body(qi, 0)).unwrap());
    // The columns' left edges, from the section's declared geometry so that an empty column keeps
    // its place. A line beside a float starts right of its column's edge, so a position is in the
    // last edge at or left of it.
    let sp = SectionProps::default();
    let n = c.cols.max(1);
    let width = (sp.page_w - sp.margin_left - sp.margin_right - sp.gutter - sp.columns.space * (n - 1) as f32) / n as f32;
    let xs: Vec<f32> = (0..n).map(|i| sp.margin_left + i as f32 * (width + sp.columns.space)).collect();
    let col = |x: f32| xs.iter().rposition(|e| *e <= x + 1.0).unwrap_or(usize::MAX);
    (carets.iter().map(|c| col(c.x)).collect(), q_caret.map(|c| col(c.x)), l.pages.len())
}

fn of(c: &Case) -> Vec<usize> {
    columns(c).0
}

// A: the last two lines of a paragraph whose first three lines closed the page before.
#[test]
fn the_last_two_lines_of_a_continued_paragraph_stay_in_the_first_column() {
    for cols in [3, 2] {
        let (v, _, pages) = columns(&Case::new(cols, -3, 5));
        assert_eq!(pages, 2, "{cols} columns");
        assert_eq!(v[3..], [0, 0], "{cols} columns: lines 4 and 5 together");
    }
}

// B: three lines stay together too: not two and one, nor one and two.
#[test]
fn the_last_three_lines_of_a_continued_paragraph_stay_in_the_first_column() {
    for cols in [3, 2] {
        assert_eq!(of(&Case::new(cols, -3, 6))[3..], [0, 0, 0], "{cols} columns");
    }
}

// L: five lines are three and two (not two, one and two).
#[test]
fn the_last_five_lines_of_a_continued_paragraph_are_three_and_two() {
    for cols in [3, 2] {
        assert_eq!(of(&Case::new(cols, -3, 8))[3..], [0, 0, 0, 1, 1], "{cols} columns");
    }
}

// N: a one-line paragraph after the two lines goes to the second column.
#[test]
fn a_line_after_the_continued_two_lines_goes_to_the_next_column() {
    for cols in [3, 2] {
        let (v, q, _) = columns(&Case { q: true, ..Case::new(cols, -3, 5) });
        assert_eq!((v[3..].to_vec(), q), (vec![0, 0], Some(1)), "{cols} columns");
    }
}

// K, E: a paragraph of three lines (or two) that begins the last page stays together.
#[test]
fn a_whole_paragraph_of_two_or_three_lines_at_the_top_of_the_last_page_stays_together() {
    for cols in [3, 2] {
        assert_eq!(of(&Case::new(cols, 0, 3)), [0, 0, 0], "{cols} columns, three lines");
        assert_eq!(of(&Case::new(cols, 0, 2)), [0, 0], "{cols} columns, two lines");
    }
}

// K2: the same after one line of another paragraph in the first column: it starts the second.
#[test]
fn a_three_line_paragraph_after_one_line_on_the_last_page_stays_together() {
    for cols in [3, 2] {
        let (v, _, pages) = columns(&Case::new(cols, 1, 3));
        assert_eq!((v, pages), (vec![1, 1, 1], 2), "{cols} columns");
    }
}

// F: with a footnote on the last page, referenced at the end of the paragraph.
#[test]
fn a_footnote_on_the_last_page_leaves_the_tail_together() {
    for cols in [3, 2] {
        let n = |off, k| of(&Case { note: true, ..Case::new(cols, off, k) });
        assert_eq!(n(-3, 5)[3..], [0, 0], "{cols} columns, two lines");
        assert_eq!(n(-3, 6)[3..], [0, 0, 0], "{cols} columns, three lines");
        assert_eq!(n(0, 2), [0, 0], "{cols} columns, whole paragraph of two lines");
        assert_eq!(n(0, 3), [0, 0, 0], "{cols} columns, whole paragraph of three lines");
    }
}

// L2: beside a square-wrapped float in the next paragraph; a top-and-bottom one is the guard.
#[test]
fn a_square_wrapped_float_after_the_tail_leaves_it_together() {
    for cols in [3, 2] {
        let sq = |off, k| columns(&Case { q: true, float: Some(Wrap::Square), ..Case::new(cols, off, k) });
        let (v, q, _) = sq(-3, 5);
        assert_eq!((v[3..].to_vec(), q), (vec![0, 0], Some(1)), "{cols} columns, two lines");
        let (v, q, _) = sq(0, 3);
        assert_eq!((v, q), (vec![0, 0, 0], Some(1)), "{cols} columns, whole paragraph of three lines");
    }
}

#[test]
fn a_top_and_bottom_float_after_the_tail_is_laid_out_as_before() {
    for cols in [3, 2] {
        let tb = |off, k| columns(&Case { q: true, float: Some(Wrap::TopAndBottom), ..Case::new(cols, off, k) });
        let (v, q, _) = tb(-3, 5);
        assert_eq!((v[3..].to_vec(), q), (vec![0, 0], Some(1)), "{cols} columns, two lines");
        let (v, q, _) = tb(0, 3);
        assert_eq!((v, q), (vec![0, 0, 0], Some(1)), "{cols} columns, whole paragraph of three lines");
    }
}

// M: a section that starts and ends on one page (after two lines of text in one column).
#[test]
fn a_five_line_paragraph_balanced_mid_page_is_three_and_two() {
    assert_eq!(of(&Case { intro: 2, ..Case::new(3, 0, 5) }), [0, 0, 0, 1, 1]);
}

#[test]
fn a_section_balanced_mid_page_is_laid_out_as_before() {
    let mid = |cols, filler, k| of(&Case { intro: 2, ..Case::new(cols, filler, k) });
    assert_eq!(mid(3, 0, 2), [0, 0]);
    assert_eq!(mid(3, 0, 3), [0, 0, 0]);
    assert_eq!(mid(3, 0, 4), [0, 0, 1, 1]);
    assert_eq!(mid(3, 0, 6), [0, 0, 1, 1, 2, 2]);
    assert_eq!(mid(2, 0, 3), [0, 0, 0]);
    assert_eq!(mid(2, 0, 5), [0, 0, 0, 1, 1]);
    assert_eq!(mid(2, 0, 6), [0, 0, 0, 1, 1, 1]);
    // After one more line.
    assert_eq!(mid(3, 1, 3), [1, 1, 1]);
    assert_eq!(mid(3, 1, 4), [1, 1, 2, 2]);
    assert_eq!(mid(3, 1, 5), [0, 0, 1, 1, 1]);
    assert_eq!(mid(2, 1, 4), [0, 0, 1, 1]);
    assert_eq!(mid(2, 1, 6), [0, 0, 0, 1, 1, 1]);
}

// X: keep lines together, and keep with next, hold the paragraph whole in the first column.
#[test]
fn a_paragraph_kept_together_stays_whole_in_the_first_balanced_column() {
    for cols in [3, 2] {
        let kept = |off, k| of(&Case { keep: Keep::Lines, ..Case::new(cols, off, k) });
        assert_eq!(kept(0, 3), [0, 0, 0], "{cols} columns, three lines at the top");
        // Keep lines together moved it whole onto the last page.
        assert_eq!(kept(-3, 5), [0, 0, 0, 0, 0], "{cols} columns, five lines");
        assert_eq!(kept(-3, 6), [0, 0, 0, 0, 0, 0], "{cols} columns, six lines");
    }
}

#[test]
fn a_paragraph_kept_with_the_next_stays_with_it_in_the_first_balanced_column() {
    for cols in [3, 2] {
        let (v, q, _) = columns(&Case { keep: Keep::Next, q: true, ..Case::new(cols, 0, 3) });
        assert_eq!((v, q), (vec![0, 0, 0], Some(0)), "{cols} columns");
    }
}

// Guards, the same in Word and before the change.

// C, M: four lines are two and two, six are two, two and two.
#[test]
fn longer_tails_are_balanced_as_before() {
    for cols in [3, 2] {
        assert_eq!(of(&Case::new(cols, -3, 7))[3..], [0, 0, 1, 1], "{cols} columns, four lines");
    }
    assert_eq!(of(&Case::new(3, -3, 9))[3..], [0, 0, 1, 1, 2, 2], "three columns, six lines");
}

// O: a four-line paragraph that begins the last page is two and two.
#[test]
fn a_four_line_paragraph_at_the_top_of_the_last_page_is_two_and_two() {
    for cols in [3, 2] {
        assert_eq!(of(&Case::new(cols, 0, 4)), [0, 0, 1, 1], "{cols} columns");
    }
}

// Columns not balanced (nothing follows them): the lines are not split without need.
#[test]
fn unbalanced_columns_keep_the_tail_in_the_first_column() {
    for cols in [3, 2] {
        let c = Case { balanced: false, ..Case::new(cols, -3, 5) };
        assert_eq!(of(&c)[3..], [0, 0], "{cols} columns");
    }
}

// Before Word 2013's layout (mode 14) the two lines are split one per column, as Word does.
#[test]
fn mode_14_still_splits_the_last_two_lines_of_a_continued_paragraph() {
    for cols in [3, 2] {
        let c = Case { compat: 14, ..Case::new(cols, -3, 5) };
        assert_eq!(of(&c)[3..], [0, 1], "{cols} columns");
    }
}

/// How many lines of body text are laid out, and on how many pages.
fn shape(c: &Case) -> (usize, usize) {
    let (d, ..) = build(c);
    let l = lay(&d);
    let lines = l
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .map(|it| match it {
            Placed::Lines { story: StoryRef::Body, l0, l1, .. } => l1 - l0,
            _ => 0,
        })
        .sum();
    (l.pages.len(), lines)
}

// A page whose columns hold two lines is too short for the rules that keep three lines together:
// not even the full column holds the paragraph with them, so the columns are laid out as before
// (mode 14), with every line placed and no more pages.
#[test]
fn a_page_too_short_for_the_strict_rules_is_laid_out_as_in_mode_14() {
    let tiny = |compat| Case { compat, page_lines: Some(2), ..Case::new(2, 0, 3) };
    let (pages, lines) = shape(&tiny(14));
    assert!(lines >= 4, "the paragraph's three lines and After: {lines}");
    assert_eq!(shape(&tiny(15)), (pages, lines));
}
