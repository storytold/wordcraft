//! A paragraph that starts a cell's part of a page, in a row split across pages, is split where
//! its lines fall, as in Word (#6823): moving it on whole would gain nothing, and keeping
//! widow/orphan control left the cell with nothing on the page, so the row was placed whole and
//! ran off the bottom. A paragraph below other content of its cell keeps the rule.
use crate::*;
use wordcraft_doc::props::{LineSpacing, ParaProps};
use wordcraft_doc::{Table, para_block};

fn exact(height: f32, widow: bool) -> ParaProps {
    ParaProps { line_spacing: Some(LineSpacing::Exactly(height)), space_before: Some(0.0), space_after: Some(0.0), widow_control: Some(widow), ..Default::default() }
}

fn lines_para(lines: usize, height: f32, widow: bool) -> Arc<Block> {
    let text: Vec<String> = (1..=lines).map(|i| format!("L{i}")).collect();
    let mut p = Paragraph::with_text(&text.join(&wordcraft_doc::para::LINE_BREAK.to_string()), Default::default());
    p.props = exact(height, widow);
    para_block(p)
}

/// A one-row table at the top of the document whose second cell holds `before` (lines and line
/// height of each paragraph), then a paragraph of `lines` lines `height` points tall: the lines
/// of that last paragraph on each page. The first paragraph fills the first page but for a few
/// points, so the later ones start the second page.
fn lines_per_page(before: &[(usize, f32)], (lines, height): (usize, f32), widow: bool) -> Vec<usize> {
    crate::test_fonts();
    let mut d = Document::from_text("After");
    let fill = ((d.last_section.text_height() - 8.0) / 14.0).floor() as usize;
    let mut t = Table::new(1, 2, 400.0);
    t.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    let mut blocks = vec![lines_para(fill, 14.0, widow)];
    blocks.extend(before.iter().map(|&(n, h)| lines_para(n, h, widow)));
    let at = blocks.len() as u32;
    blocks.push(lines_para(lines, height, widow));
    t.rows[0].cells[1].blocks = blocks;
    d.insert_block(StoryRef::Body, &Path::top(0), Block::Table(t)).unwrap();
    let l = lay(&d);
    l.pages
        .iter()
        .map(|p| {
            p.items
                .iter()
                .filter_map(|it| match it {
                    Placed::Lines { path, l0, l1, .. } if path.0 == [0, 0, 1, at] => Some(l1 - l0),
                    _ => None,
                })
                .sum()
        })
        .collect()
}

/// Word for Windows 16.0, compatibility mode 15, widow control on: a three-line paragraph at the
/// top of a page with room for its first line only keeps that line there (the next two go on).
#[test]
fn a_first_line_alone_at_the_top_of_a_page_stays_in_a_split_row() {
    assert_eq!(lines_per_page(&[], (3, 400.0), true), vec![0, 1, 1, 1]);
}

/// Two lines, room for one: one and one.
#[test]
fn a_two_line_paragraph_at_the_top_of_a_page_splits_one_and_one() {
    assert_eq!(lines_per_page(&[], (2, 400.0), true), vec![0, 1, 1]);
}

/// Three lines, room for two: two and one (not the whole paragraph on, which is no better at the
/// top of a page, and not one and two).
#[test]
fn a_three_line_paragraph_at_the_top_of_a_page_with_room_for_two_splits_two_and_one() {
    assert_eq!(lines_per_page(&[], (3, 300.0), true), vec![0, 2, 1]);
}

/// Guard (passes without the fix): four lines with room for three still take the widow along.
#[test]
fn four_lines_at_the_top_of_a_page_with_room_for_three_still_keep_two_for_the_widow() {
    assert_eq!(lines_per_page(&[], (4, 200.0), true), vec![0, 2, 2]);
}

/// Guard (passes without the fix): a paragraph below another one of its cell on the page keeps
/// the orphan rule, so its single line there moves on with the rest (Word: the same).
#[test]
fn a_paragraph_below_other_content_of_its_cell_still_moves_on_whole_instead_of_an_orphan() {
    assert_eq!(lines_per_page(&[(1, 300.0)], (3, 200.0), true), vec![0, 0, 3]);
}

/// Guard (passes without the fix): without widow control the paragraph always split where it fell.
#[test]
fn without_widow_control_the_lines_split_where_they_fall() {
    assert_eq!(lines_per_page(&[], (3, 400.0), false), vec![0, 1, 1, 1]);
}
