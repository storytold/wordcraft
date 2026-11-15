//! A paragraph that starts a cell's part of a page, in a row split across pages, is split where
//! its lines fall, as in Word (#6823): moving it on whole would gain nothing, and keeping
//! widow/orphan control left the cell with nothing on the page, so the row was placed whole and
//! ran off the bottom. A paragraph below other content of its cell keeps the rule, and so does a
//! row that starts below other content of the page. A "can't split" row taller than a page
//! splits too.
//!
//! The expectations were measured in Word for Windows 16.0 (compatibility mode 15, widow control
//! on, Letter with 1 inch margins) on synthetic documents laid out like these.
use crate::*;
use wordcraft_doc::para::{Anchor, Float, InlineObject, Wrap};
use wordcraft_doc::props::{LineSpacing, ParaProps};
use wordcraft_doc::{Table, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn exact(height: f32, widow: bool) -> ParaProps {
    ParaProps {
        line_spacing: Some(LineSpacing::Exactly(height)),
        space_before: Some(0.0),
        space_after: Some(0.0),
        widow_control: Some(widow),
        ..Default::default()
    }
}

fn lines_para(lines: usize, height: f32, widow: bool, name: &str) -> Arc<Block> {
    para_block(lines_paragraph(lines, height, widow, name))
}

fn lines_paragraph(lines: usize, height: f32, widow: bool, name: &str) -> Paragraph {
    let text: Vec<String> = (1..=lines).map(|i| format!("{name}{i}")).collect();
    let mut p = Paragraph::with_text(&text.join(&wordcraft_doc::para::LINE_BREAK.to_string()), Default::default());
    p.props = exact(height, widow);
    p
}

/// What the table is made of: `pre` one-line paragraphs of 16 pt before it, an optional repeated
/// header row, a row (can't split or not) whose second cell holds `fill` lines of `fill_height`,
/// the paragraphs `before` (lines and line height) and last a paragraph `last` of lines of a
/// height. The last two are inside a nested table when `nested` is set.
struct Case {
    pre: usize,
    header: bool,
    cant_split: bool,
    nested: bool,
    fill: usize,
    fill_height: f32,
    before: Vec<(usize, f32)>,
    last: (usize, f32),
    widow: bool,
    /// A picture floating through the text, anchored in the last paragraph and placed above it.
    float_above: bool,
}

impl Case {
    fn new(last: (usize, f32)) -> Case {
        Case {
            pre: 0,
            header: false,
            cant_split: false,
            nested: false,
            fill: 45,
            fill_height: 14.0,
            before: Vec::new(),
            last,
            widow: true,
            float_above: false,
        }
    }

    /// The lines of the last paragraph on each page.
    fn per_page(&self) -> Vec<usize> {
        crate::test_fonts();
        let w = self.widow;
        let mut d = Document::from_text("After");
        for i in 0..self.pre {
            let mut p = Paragraph::with_text(&format!("F{i}"), Default::default());
            p.props = exact(16.0, w);
            d.insert_block(StoryRef::Body, &Path::top(i), Block::Para(p)).unwrap();
        }
        let mut t = Table::new(1 + usize::from(self.header), 2, 400.0);
        t.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
        let row = usize::from(self.header);
        if self.header {
            t.rows[0].props.header = true;
            for c in &mut t.rows[0].cells {
                c.blocks = vec![lines_para(1, 14.0, w, "H")];
            }
        }
        t.rows[row].props.cant_split = self.cant_split;
        let mut inner: Vec<Arc<Block>> = self.before.iter().map(|&(n, h)| lines_para(n, h, w, "P")).collect();
        let mut last = lines_paragraph(self.last.0, self.last.1, w, "B");
        if self.float_above {
            let float = Float { wrap: Wrap::Through, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 250.0, y: -6.0, ..Default::default() };
            let picture = InlineObject::Image {
                media: "float".into(),
                w: 20.0,
                h: 20.0,
                alt: "float".into(),
                float,
                crop: [0.0; 4],
                fx: Default::default(),
                ole: None,
            };
            last.insert_object(0, picture, &Default::default()).unwrap();
        }
        inner.push(para_block(last));
        // Without a fill paragraph the cell starts with `inner`.
        let mut blocks = if self.fill > 0 { vec![lines_para(self.fill, self.fill_height, w, "A")] } else { Vec::new() };
        let mut at = vec![self.pre as u32, row as u32, 1, (blocks.len() + inner.len()) as u32 - 1];
        if self.nested {
            let mut nested = Table::new(1, 1, 200.0);
            nested.rows[0].cells[0].blocks = inner.clone();
            at = vec![self.pre as u32, row as u32, 1, blocks.len() as u32, 0, 0, inner.len() as u32 - 1];
            blocks.push(Arc::new(Block::Table(nested)));
            blocks.push(lines_para(1, 1.0, w, "Z"));
        } else {
            blocks.extend(inner);
        }
        t.rows[row].cells[1].blocks = blocks;
        d.insert_block(StoryRef::Body, &Path::top(self.pre), Block::Table(t)).unwrap();
        lay(&d)
            .pages
            .iter()
            .map(|p| {
                p.items
                    .iter()
                    .filter_map(|it| match it {
                        Placed::Lines { path, l0, l1, .. } if path.0 == at => Some(l1 - l0),
                        _ => None,
                    })
                    .sum()
            })
            .collect()
    }
}

/// A three-line paragraph at the top of a page with room for its first line only keeps that line
/// there (the next two go on, one to a page here).
#[test]
fn a_first_line_alone_at_the_top_of_a_page_stays_in_a_split_row() {
    assert_eq!(Case::new((3, 400.0)).per_page(), vec![0, 1, 1, 1]);
}

/// Two lines, room for one: one and one.
#[test]
fn a_two_line_paragraph_at_the_top_of_a_page_splits_one_and_one() {
    assert_eq!(Case::new((2, 400.0)).per_page(), vec![0, 1, 1]);
}

/// Three lines, room for two: two and one (not the whole paragraph on, which is no better at the
/// top of a page, and not one and two).
#[test]
fn a_three_line_paragraph_at_the_top_of_a_page_with_room_for_two_splits_two_and_one() {
    assert_eq!(Case::new((3, 300.0)).per_page(), vec![0, 2, 1]);
}

/// Guard (passes without the fix): four lines with room for three still keep two for the widow.
#[test]
fn four_lines_at_the_top_of_a_page_with_room_for_three_still_keep_two_for_the_widow() {
    assert_eq!(Case::new((4, 200.0)).per_page(), vec![0, 2, 2]);
}

/// Guard (passes without the fix): a paragraph below another one of its cell on the page keeps
/// the orphan rule, so its single line there moves on with the rest.
#[test]
fn a_paragraph_below_other_content_of_its_cell_still_moves_on_whole_instead_of_an_orphan() {
    let mut c = Case::new((3, 200.0));
    c.before = vec![(1, 300.0)];
    assert_eq!(c.per_page(), vec![0, 0, 3]);
}

/// Where only the later paragraphs of a cell continue, the one that moved to the top of a page
/// splits at its lines (two and one), the paragraph before it having stayed.
#[test]
fn a_later_paragraph_moved_to_the_top_of_a_page_splits_where_its_lines_fall() {
    let mut c = Case::new((3, 300.0));
    c.before = vec![(1, 100.0)];
    assert_eq!(c.per_page(), vec![0, 0, 2, 1]);
}

/// Guard (passes without the fix): four lines below a paragraph, room for three, keep two.
#[test]
fn four_lines_below_other_content_with_room_for_three_still_keep_two_for_the_widow() {
    let mut c = Case::new((4, 200.0));
    c.before = vec![(1, 40.0)];
    assert_eq!(c.per_page(), vec![0, 2, 2]);
}

/// Guard (passes without the fix): without widow control the paragraph always split where it fell.
#[test]
fn without_widow_control_the_lines_split_where_they_fall() {
    let mut c = Case::new((3, 400.0));
    c.widow = false;
    assert_eq!(c.per_page(), vec![0, 1, 1, 1]);
}

/// A paragraph of a nested table splits the same way.
#[test]
fn a_nested_paragraph_at_the_top_of_a_page_splits_where_its_lines_fall() {
    let mut c = Case::new((3, 300.0));
    c.nested = true;
    assert_eq!(c.per_page(), vec![0, 2, 1]);
    let mut c = Case::new((2, 400.0));
    c.nested = true;
    assert_eq!(c.per_page(), vec![0, 1, 1]);
}

/// A nested paragraph below another one of the nested cell moves on whole, then splits at the top
/// of the page.
#[test]
fn a_nested_paragraph_below_other_content_moves_on_then_splits() {
    let mut c = Case::new((3, 300.0));
    c.nested = true;
    c.before = vec![(1, 100.0)];
    assert_eq!(c.per_page(), vec![0, 0, 2, 1]);
}

/// Under a repeated header row the paragraph that starts the page splits at its lines too. The
/// lines per page are Word's; Word leaves the header off some of the pages, which is not covered.
#[test]
fn under_a_repeated_header_row_the_paragraph_starting_the_page_splits_where_its_lines_fall() {
    let mut c = Case::new((3, 300.0));
    c.header = true;
    c.fill = 44;
    assert_eq!(c.per_page(), vec![0, 2, 1]);
    let mut c = Case::new((3, 300.0));
    c.header = true;
    c.fill = 44;
    c.before = vec![(1, 100.0)];
    assert_eq!(c.per_page(), vec![0, 0, 2, 1]);
}

/// Guard (passes without the fix): under a header row, four lines with room for three keep two.
#[test]
fn under_a_repeated_header_row_four_lines_still_keep_two_for_the_widow() {
    let mut c = Case::new((4, 200.0));
    c.header = true;
    c.fill = 44;
    assert_eq!(c.per_page(), vec![0, 2, 2]);
}

/// A can't split row taller than a page splits where the pages end.
#[test]
fn a_cant_split_row_taller_than_a_page_splits_where_the_page_ends() {
    let mut c = Case::new((3, 400.0));
    c.cant_split = true;
    assert_eq!(c.per_page(), vec![0, 1, 1, 1]);
}

/// A can't split row taller than a page that starts below text goes to the next page first, then
/// splits there.
#[test]
fn a_tall_cant_split_row_below_text_starts_the_next_page_and_splits_there() {
    let mut c = Case::new((3, 400.0));
    c.cant_split = true;
    c.pre = 20;
    assert_eq!(c.per_page(), vec![0, 0, 1, 1, 1]);
}

/// Guard (passes without the fix): a can't split row that fits the next page moves there whole.
#[test]
fn a_cant_split_row_that_fits_the_next_page_moves_there_whole() {
    let mut c = Case::new((3, 16.0));
    c.cant_split = true;
    c.pre = 38;
    c.fill = 5;
    c.fill_height = 16.0;
    assert_eq!(c.per_page(), vec![0, 3]);
}

/// A picture floating above the paragraph that starts the cell doesn't make it any less the
/// first: only the text in the flow decides. (Floats are placed with the first piece of their
/// paragraph, so this is the table's first page.)
#[test]
fn a_floating_picture_above_the_first_paragraph_doesnt_keep_it_from_splitting() {
    let mut c = Case::new((3, 400.0));
    c.fill = 0;
    c.float_above = true;
    assert_eq!(c.per_page(), vec![1, 1, 1]);
}

/// Guard (passes without the fix): a can't split row that fits a page with room to spare stays
/// whole on it, whatever the border below it needs.
#[test]
fn a_cant_split_row_that_fits_a_page_is_left_whole() {
    let mut c = Case::new((1, 10.0));
    c.cant_split = true;
    assert_eq!(c.per_page(), vec![1, 0], "the row stays on page 1 (the paragraph after it is on page 2)");
}
