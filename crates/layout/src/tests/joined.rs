//! A paragraph going on from the line of one whose mark is hidden (#3986): what changes when it
//! moves off that line or is placed apart from it.

use super::lay;
use crate::{Arc, DocLayout, Placed, StoryRef, hit};
use wordcraft_doc::para::{Anchor, Float, Wrap};
use wordcraft_doc::props::{Border, Borders, LineSpacing, NumRef, TextFrame};
use wordcraft_doc::{Block, Document, Path, Pos, Table};

/// A table around the document's paragraphs (the cell paths are `[0, 0, 0, i]`).
fn in_cell(d: &mut Document) {
    let mut table = Table::new(1, 1, 400.0);
    table.rows[0].cells[0].blocks = std::mem::take(&mut d.body);
    d.body = vec![Arc::new(Block::Table(table))];
}

/// The top of the lines of the body block or cell paragraph at `path`.
fn top_of(l: &DocLayout, path: &Path) -> f32 {
    l.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path: p, y, .. } if p == path => Some(*y),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no lines at {path:?}"))
}

#[test]
fn a_joined_paragraph_moved_to_the_next_page_gets_its_top_border() {
    // Sharing the line it takes no room for a top border; moved whole to the next page it is
    // laid out on its own and draws one above its first line.
    let omega = "Omega ".repeat(60);
    let boxed = Some(Borders::all(Border { space: 1.0, ..Border::single(1.5) }));
    for filler in 20..80 {
        let mut text: Vec<String> = (0..filler).map(|i| format!("Filler {i}")).collect();
        text.push("Alpha".into());
        text.push(omega.clone());
        let mut d = Document::from_text(&text.join("\n"));
        d.para_mut(StoryRef::Body, &Path::top(filler)).unwrap().mark.hidden = Some(true);
        let o = d.para_mut(StoryRef::Body, &Path::top(filler + 1)).unwrap();
        (o.props.keep_lines, o.props.borders) = (Some(true), boxed);
        let l = lay(&d);
        let o = l.caret(&Pos::body(filler + 1, 0)).unwrap();
        if l.caret(&Pos::body(filler, 0)).unwrap().page != 0 || o.page != 1 {
            continue;
        }
        let tops = l.pages[1].items.iter().filter(|i| matches!(i, Placed::Rule { y0, y1, border, .. } if y0 == y1 && border.width == 1.5));
        let above = tops.filter(|i| matches!(i, Placed::Rule { y0, .. } if *y0 < o.top)).count();
        assert_eq!(above, 1, "the moved paragraph draws its top border above its first line");
        return;
    }
    panic!("no filler moved the joined paragraph to the next page");
}

#[test]
fn a_joined_first_line_below_the_shared_line_does_not_cut_the_line_above() {
    // "Omega"'s first word does not fit after "Alpha", so its first line goes below; its indent
    // (inside "Alpha"'s text) must not cut "Alpha"'s hit extent.
    let mut d = Document::from_text(&format!("Alpha\n{} tail", "W".repeat(48)));
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().mark.hidden = Some(true);
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.indent_left = Some(20.0);
    let l = lay(&d);
    let lines = hit::page_lines(&l.pages[0], StoryRef::Body);
    let alpha = lines.iter().find(|h| h.path == &Path::top(0)).unwrap();
    let omega = lines.iter().find(|h| h.path == &Path::top(1) && h.li == 0).unwrap();
    assert!(omega.top > alpha.top + 1.0, "setup: the first word moved below the shared line");
    let text_end = l.caret(&Pos::body(0, 5)).unwrap().x;
    assert!(alpha.right >= text_end - 0.5, "Alpha's line reaches {} but its text ends at {text_end}", alpha.right);
}

#[test]
fn a_joined_paragraphs_drawings_are_relative_to_the_shared_line() {
    // A drawing anchored to the paragraph, at its top: the paragraph's top is the shared line's,
    // not the end of the paragraph before.
    let mut d = Document::from_text("Alpha\nOmega\nAfter");
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().mark.hidden = Some(true);
    let float =
        Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 200.0, y: 0.0, dist: 0.0, ..Default::default() };
    super::text_box_at(&mut d, &Pos::body(1, 0), "Box", 100.0, 40.0, float);
    in_cell(&mut d);
    let l = lay(&d);
    let alpha = top_of(&l, &Path(vec![0, 0, 0, 0]));
    let boxed = l.pages[0].items.iter().find_map(|it| match it {
        Placed::Lines { story: StoryRef::Part(_), y, .. } => Some(*y),
        _ => None,
    });
    let boxed = boxed.expect("the box's text is laid out");
    assert!(boxed < alpha + 12.0, "the box's text at {boxed} is below the shared line at {alpha}");
}

#[test]
fn a_text_frame_after_a_hidden_mark_is_not_joined() {
    // A framed paragraph is placed apart, so it shares no line with the one before: its number is
    // drawn (it is counted either way). The frame is its own, or given by the table's style.
    let frame =
        TextFrame { x_align: Some("right".into()), h_anchor: "text".into(), v_anchor: "text".into(), wrap: "around".into(), ..Default::default() };
    for from_style in [false, true] {
        let mut d = Document::from_text("Alpha\n7");
        let id = Arc::make_mut(&mut d.numbering).add_list(wordcraft_doc::ListKind::Numbered);
        d.format_paragraphs(&Pos::body(0, 0), &Pos::body(1, 1), &|p| p.numbering = Some(NumRef { num: id, level: 0 })).unwrap();
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().mark.hidden = Some(true);
        // From the table's style, the frame is given to "Alpha" too: zero dimensions cancel it there.
        let (at, own) = if from_style { (0, TextFrame { w: Some(0.0), h: Some(0.0), ..frame.clone() }) } else { (1, frame.clone()) };
        d.para_mut(StoryRef::Body, &Path::top(at)).unwrap().props.frame = Some(own);
        in_cell(&mut d);
        if from_style {
            Arc::make_mut(&mut d.styles).get_mut("TableGrid").expect("Table Grid").para.frame = Some(frame.clone());
            if let Some(Block::Table(t)) = d.body.first_mut().map(Arc::make_mut) {
                t.props.style = Some("TableGrid".into());
            }
        }
        let l = lay(&d);
        let label = l.pages[0].items.iter().find_map(|it| match it {
            Placed::Lines { path, para, .. } if path.0 == [0, 0, 0, 1] => Some(para.label.is_some()),
            _ => None,
        });
        assert_eq!(label, Some(true), "the framed paragraph keeps its number (frame from the table style: {from_style})");
    }
}

#[test]
fn a_joined_paragraphs_later_lines_start_below_a_taller_shared_line() {
    // A heading whose mark is hidden, then body text going on from it: the text's first line
    // shares the heading's line; its later lines start below that line and the heading's bottom
    // border, not inside it (Word), whatever the text's line spacing.
    let spacings = [None, Some(LineSpacing::Exactly(12.0)), Some(LineSpacing::AtLeast(12.0)), Some(LineSpacing::Multiple(2.0))];
    // A large word that fits the first line of the text laid out alone but wraps off the
    // narrower shared line onto the second (`big`), so the two first lines differ in height.
    let cases = [false, true].into_iter().flat_map(|c| spacings.into_iter().flat_map(move |s| [(c, s, false, None), (c, s, true, None)]));
    let big = [60, 70, 80].into_iter().flat_map(|at| [(false, None, false, Some(at)), (true, None, false, Some(at))]);
    for (cell, spacing, bordered, large) in cases.chain(big) {
        let case = format!("cell {cell}, {spacing:?}, border {bordered}, large word at {large:?}");
        let mut d = Document::from_text(&format!("Heading\n{}", "body text ".repeat(40)));
        d.format_range(&Pos::body(0, 0), &Pos::body(0, 7), &|c| c.size = Some(40.0)).unwrap();
        if let Some(at) = large {
            d.format_range(&Pos::body(1, at), &Pos::body(1, at + 4), &|c| c.size = Some(30.0)).unwrap();
        }
        let heading = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        heading.mark.hidden = Some(true);
        if bordered {
            heading.props.borders = Some(Borders { bottom: Some(Border { space: 4.0, ..Border::single(1.5) }), ..Default::default() });
        }
        d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.line_spacing = spacing;
        if cell {
            in_cell(&mut d);
        }
        let l = lay(&d);
        let lines = hit::page_lines(&l.pages[0], StoryRef::Body);
        let heading = lines.iter().find(|h| h.path.0.last() == Some(&0)).unwrap();
        // The shared line's bottom, with the heading's bottom border under it.
        let rule = l.pages[0].items.iter().filter_map(|it| match it {
            Placed::Rule { y0, y1, border, .. } if y0 == y1 && border.width == 1.5 => Some(*y0),
            _ => None,
        });
        let floor = rule.fold(heading.bottom, f32::max);
        assert!(!bordered || floor > heading.bottom, "setup: the border is drawn below the shared line ({case})");
        let body: Vec<_> = lines.iter().filter(|h| h.path.0.last() == Some(&1)).collect();
        assert!(body.len() > 2, "setup: the body text runs to several lines ({case})");
        assert!(body[0].top < heading.bottom - 1.0, "the first line at {} shares the heading's line ({case})", body[0].top);
        for line in &body[1..] {
            assert!(line.top >= floor - 0.01, "line {} at {} starts inside the shared line, ending {floor} ({case})", line.li, line.top);
        }
    }
}
