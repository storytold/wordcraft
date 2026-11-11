use super::*;
use crate::flow::WIDTH_RELAYS;

/// Unequal columns (#2382): a paragraph that fills a narrow column and overflows into a wide one
/// (without a column break) breaks its remaining lines for the column they land in. Word 2024's
/// Left and Right preset column layouts (a narrow and a wide column) showed it: the overflowing
/// text ran down the wide column a word or two wide, and the two next to each other (till it
/// reached the page edge).
#[test]
fn text_overflowing_into_an_unequal_column_takes_its_width() {
    // Like Word's "Left" preset: a 120 pt column, then a 280 pt one.
    let text = "Cras vitae aliquam leo. Integer quam ex, fringilla non ante quis, pharetra iaculis tortor. Vestibulum vel consequat augue, a dignissim massa. Vestibulum condimentum maximus urna, ac luctus quam consectetur et. Quisque convallis leo sem, id malesuada sem finibus elementum. Nam id nulla ac neque scelerisque volutpat sit amet sed ipsum. Donec hendrerit lacus ut felis porta, sed ultricies sapien tristique. Nullam eget dignissim lacus. Pellentesque habitant morbi tristique senectus et netus et malesuada fames ac turpis egestas. In quis enim eu nunc dignissim accumsan et bibendum turpis. Vivamus volutpat, libero id scelerisque commodo, nulla massa tempus velit, quis ultricies ex eros sed sapien. Proin dapibus tortor varius porttitor tincidunt. Curabitur ullamcorper magna sed lectus luctus, vel bibendum mauris semper.";
    let items = |widths: Vec<(f32, f32)>| {
        let mut d = Document::from_text(text);
        d.last_section.columns = wordcraft_doc::section::Columns { count: 2, space: 36.0, separator: false, widths };
        WIDTH_RELAYS.with(|c| c.set(0));
        let l = lay(&d);
        let mut covered = 0;
        for page in &l.pages {
            for item in &page.items {
                if let Placed::Lines { path, para, l0, l1, .. } = item
                    && path.0 == [0]
                {
                    for line in &para.lines[*l0..*l1] {
                        assert_eq!(line.start, covered, "the continuation neither omits nor repeats text");
                        covered = line.stop;
                    }
                }
            }
        }
        assert_eq!(covered, text.len(), "the complete input paragraph is placed");
        let lines: Vec<(f32, f32)> = l.pages[0]
            .items
            .iter()
            .filter_map(|it| match it {
                Placed::Lines { path, para, l0, l1, x, .. } if path.0 == [0] => {
                    // The caret end includes trailing spaces that may hang beyond the margin.
                    // Compare visible text, as the other unequal-column controls do.
                    let right = (*l0..*l1).filter_map(|li| para.lines.get(li)).map(|line| x + ink_end(para, line)).fold(f32::MIN, f32::max);
                    Some((*x, right))
                }
                _ => None,
            })
            .collect();
        (lines, WIDTH_RELAYS.with(|c| c.get()))
    };
    let (lines, relays) = items(vec![(120.0, 36.0), (280.0, 0.0)]);
    let (narrow, wide) = (lines.first().copied().unwrap(), lines.last().copied().unwrap());
    assert!(wide.0 > narrow.0, "the second column is right of the first: {lines:?}");
    assert!(narrow.1 <= narrow.0 + 120.0 + 0.5, "the first column's lines stay in it: {lines:?}");
    assert!(wide.1 > wide.0 + 160.0, "the overflowing lines are broken for the 280 pt column, not the 120 pt one: {lines:?}");
    assert!(relays >= 1, "a width relay happened");
    // Equal columns are unaffected (nothing is laid out twice).
    let (_, relays) = items(vec![]);
    assert_eq!(relays, 0, "no width relay for equal columns");
}

/// A long paragraph in Word 2024's "Left" preset columns (#2382): a 120 pt column, a 36 pt gap,
/// then a 280 pt one; `edit` shapes the paragraph (alignment, shading…). The page is short so the
/// narrow column holds two lines and the overflow resumes inside the wide column's first line
/// (the tail cut, not a line boundary).
fn unequal_column_doc(edit: impl Fn(&mut ParaProps)) -> Document {
    let text = "Cras vitae aliquam leo. Integer quam ex, fringilla non ante quis, pharetra iaculis tortor. Vestibulum vel consequat augue, a dignissim massa. Vestibulum condimentum maximus urna, ac luctus quam consectetur et. Quisque convallis leo sem, id malesuada sem finibus elementum. Nam id nulla ac neque scelerisque volutpat sit amet sed ipsum. Donec hendrerit lacus ut felis porta, sed ultricies sapien tristique. Nullam eget dignissim lacus. Pellentesque habitant morbi tristique senectus et netus et malesuada fames ac turpis egestas. In quis enim eu nunc dignissim accumsan et bibendum turpis. Vivamus volutpat, libero id scelerisque commodo, nulla massa tempus velit, quis ultricies ex eros sed sapien. Proin dapibus tortor varius porttitor tincidunt. Curabitur ullamcorper magna sed lectus luctus, vel bibendum mauris semper.";
    let mut d = Document::from_text(text);
    d.last_section.page_h = 180.0;
    d.last_section.columns = wordcraft_doc::section::Columns { count: 2, space: 36.0, separator: false, widths: vec![(120.0, 36.0), (280.0, 0.0)] };
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &edit).unwrap();
    d
}

/// The body lines on page 1 as (x, ink end), one entry per line.
fn page_lines(l: &DocLayout) -> Vec<(f32, f32)> {
    l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { path, para, l0, l1, x, .. } if path.0 == [0] => {
                Some((*l0..*l1).filter_map(|li| para.lines.get(li)).map(|line| (*x, *x + ink_end(para, line))).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// Unequal columns (#2382): the tail that opens the wide column keeps the paragraph's alignment,
/// as Word lays the rest of the paragraph out there. An end-aligned paragraph's overflow reaches
/// the wide column's right edge; before, the tail was anchored at the left indent and its line
/// ended short of the edge.
#[test]
fn an_overflowing_tail_keeps_the_paragraphs_alignment() {
    let d = unequal_column_doc(|p| p.align = Some(Align::Right));
    let lines = page_lines(&lay(&d));
    let first_x = lines.first().copied().unwrap().0;
    assert!(lines.iter().any(|(x, _)| *x > first_x + 0.5), "the paragraph overflows into the second column: {lines:?}");
    for (x, right) in &lines {
        let width = if (*x - first_x).abs() < 0.5 { 120.0 } else { 280.0 };
        assert!((*right - (*x + width)).abs() < 0.75, "an end-aligned line ends at its column's right edge: {lines:?}");
    }
}

/// A wrapped continuation is a full justified line; only the actual final line keeps natural
/// word spacing. Reusing a shortened tail made the first line stop short of its column edge.
#[test]
fn a_justified_overflowing_line_fills_the_destination_span() {
    let d = unequal_column_doc(|p| p.align = Some(Align::Justify));
    let l = lay(&d);
    let col2 = l.pages[0].body.x + d.last_section.column_boxes()[1].0;
    let (para, line) = l.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { para, l0, x, .. } if (*x - col2).abs() < 0.5 => Some((para, &para.lines[*l0])),
            _ => None,
        })
        .unwrap();
    assert_eq!(line.end, para::LineEnd::Wrap);
    assert!((ink_end(para, line) - 280.0).abs() < 0.75, "the full justified continuation fills its span");
}

/// Unequal columns (#2382): a shaded paragraph's overflow shades the wide column it lands in, not
/// the width of the column it started in (borders follow the same width).
#[test]
fn an_overflowing_paragraph_shades_the_column_it_lands_in() {
    let colour = wordcraft_doc::Rgb(0xDD, 0xEE, 0xFF);
    let d = unequal_column_doc(|p| p.shading = Some(colour));
    let l = lay(&d);
    let fills: Vec<_> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Fill { rect, color } if *color == colour => Some(*rect),
            _ => None,
        })
        .collect();
    assert!(fills.iter().any(|r| (r.w - 280.0).abs() < 0.75), "a piece in the 280 pt column shades it: {fills:?}");
    assert!(fills.iter().any(|r| (r.w - 120.0).abs() < 0.75), "the first piece shades the 120 pt column: {fills:?}");
}

/// Unequal columns (#2382): a paragraph's top border is drawn once, on its first piece. The piece
/// that opens the wide column is a continuation, even though the overflow cuts a line mid-way
/// (before, it held line 0 of the relaid layout and push_para drew the border again there, as
/// display draws the list label again).
#[test]
fn an_overflowing_paragraph_draws_its_top_border_once() {
    let d = unequal_column_doc(|p| {
        p.borders = Some(wordcraft_doc::props::Borders { top: Some(Border::single(1.0)), ..Default::default() });
    });
    let l = lay(&d);
    let horizontal: Vec<_> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Rule { x0, y0, x1, y1, .. } if (y0 - y1).abs() < 0.01 => Some((*x0, *x1)),
            _ => None,
        })
        .collect();
    assert_eq!(horizontal.len(), 1, "one top border, on the paragraph's first piece: {horizontal:?}");
}

/// Unequal columns (#2382): a paragraph's floating object is placed once, at the paragraph's
/// start, when its overflow opens the wide column. The wide column can hold text from before the
/// overflow on its first line, so the resumed piece is not the paragraph's first piece and places
/// no floating objects again.
#[test]
fn a_float_in_an_overflowing_paragraph_is_placed_once() {
    let mut d = unequal_column_doc(|_| {});
    let id = text_box(
        &mut d,
        40,
        "Box",
        72.0,
        36.0,
        wordcraft_doc::para::Float {
            wrap: wordcraft_doc::para::Wrap::Square,
            h_rel: wordcraft_doc::para::Anchor::Column,
            v_rel: wordcraft_doc::para::Anchor::Paragraph,
            x: 0.0,
            y: 0.0,
            dist: 0.0,
            ..Default::default()
        },
    );
    let l = lay(&d);
    let placed = l.pages.iter().flat_map(|p| p.items.iter()).filter(|i| matches!(i, Placed::Object { text_box: Some(t), .. } if *t == id)).count();
    assert_eq!(placed, 1, "the floating box is placed once");
    // The box wraps the text in the narrow column it is anchored in only: every line laid out
    // for the wide column starts at the paragraph's left edge there (before, the line after the
    // tail kept clear of the box across the column it is not in).
    let starts: Vec<(f32, f32)> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, .. } if path.0 == [0] => {
                Some((*l0..*l1).filter_map(|li| para.lines.get(li)).map(|ln| (*x, *x + ln.xs.first().copied().unwrap_or(0.0))).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect();
    let first_x = starts.first().copied().unwrap().0;
    assert!(starts.iter().any(|(x, _)| *x > first_x + 0.5), "the paragraph overflows into the wide column: {starts:?}");
    for (x, start) in starts.iter().filter(|(x, _)| *x > first_x + 0.5) {
        assert!((start - x).abs() < 0.5, "every line in the wide column starts at its left edge, clear of the narrow column's box: {starts:?}");
    }
}

/// Unequal columns (#2382): the lines that run on into the wide column are broken again around
/// that column's wrap areas, the page-anchored table in the footer (its areas come from the
/// footer story, not from the page's floating objects) included: without, they ran through it.
#[test]
fn text_overflowing_into_another_column_keeps_clear_of_a_footer_table() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::{HeightRule, TableFloat};
    let text: String = (0..200).map(|i| format!("word{i} ")).collect();
    let mut d = Document::from_text(&format!("Anchor\n{}", text.trim_end()));
    d.last_section.columns.count = 2;
    d.last_section.columns.widths = vec![(120.0, 36.0), (280.0, 0.0)];
    let mut table = Table::new(1, 1, 468.0);
    table.props.float = Some(TableFloat { x: 0.0, y: 100.0, h_rel: Anchor::Page, v_rel: Anchor::Page, ..Default::default() });
    table.rows[0].props.height = Some(40.0);
    table.rows[0].props.height_rule = HeightRule::AtLeast;
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("", Default::default()))];
    let id = d.add_part(wordcraft_doc::PartKind::Footer, vec![std::sync::Arc::new(wordcraft_doc::Block::Table(table))]);
    d.last_section.footers.default = Some(id);
    let l = lay(&d);
    let r = l.pages[0]
        .footer
        .iter()
        .find_map(|it| match it {
            Placed::Cell { table, rect, .. } if *table == Path::top(0) => Some(*rect),
            _ => None,
        })
        .expect("the footer table is drawn");
    let col2 = l.pages[0].body.x + d.last_section.column_boxes()[1].0;
    let lines = line_boxes(&l, 0, 1, col2);
    assert!(lines.iter().any(|(_, top, _, _)| *top > r.bottom()), "the paragraph runs past the table: {lines:?}");
    for (left, top, right, bottom) in lines {
        let clear = bottom <= r.y + 0.01 || top >= r.bottom() - 0.01 || left >= r.right() - 0.01 || right <= r.x + 0.01;
        assert!(clear, "a line runs through the footer table {r:?}: {left}..{right} at {top}..{bottom}");
    }
}

#[test]
fn continuation_rebreaks_the_remaining_text_at_the_destination_width() {
    for align in [Align::Left, Align::Center, Align::Right, Align::Justify] {
        let d = unequal_column_doc(|p| p.align = Some(align));
        let l = lay(&d);
        let col2 = l.pages[0].body.x + d.last_section.column_boxes()[1].0;
        let (para, line) = l.pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Lines { para, l0, x, .. } if (*x - col2).abs() < 0.5 => Some((para, &para.lines[*l0])),
                _ => None,
            })
            .unwrap();
        let text = &d.para(StoryRef::Body, &Path::top(0)).unwrap().text;
        let mut suffix = Document::from_text(&text[line.start..]);
        suffix.last_section.page_w = 424.0; // 280 pt between the 72 pt margins.
        suffix.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.align = Some(align)).unwrap();
        let expected = lay(&suffix);
        let end = expected.pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Lines { para, l0, .. } => Some(para.lines[*l0].stop),
                _ => None,
            })
            .unwrap();
        assert_eq!(line.stop - line.start, end, "{align:?}: continuation packs a full line from its resume point");
        assert!(line.c1 <= para.clusters.len());
    }
}

fn assert_body_clear_of_box(l: &DocLayout, id: u32) {
    let mut beside = 0;
    let objects = l.pages.iter().flat_map(|p| &p.items).filter(|item| matches!(item, Placed::Object { text_box: Some(t), .. } if *t == id)).count();
    for (page, pg) in l.pages.iter().enumerate() {
        let Some(object) = l.text_box(id, page).filter(|o| o.page == page) else { continue };
        let r = object.rect;
        for item in &pg.items {
            let Placed::Lines { story: StoryRef::Body, para, l0, l1, x, y, .. } = item else { continue };
            let first = &para.lines[*l0];
            for line in &para.lines[*l0..*l1] {
                let top = y + line.top - first.top;
                let left = x + line.xs.first().copied().unwrap_or(line.left);
                let right = x + ink_end(para, line);
                if top + line.height > r.y + 0.01 && top < r.bottom() - 0.01 && *x < r.right() && *x + 280.0 > r.x {
                    beside += 1;
                    assert!(left >= r.right() - 0.01 || right <= r.x + 0.01, "body text {left}..{right} at {top} overlaps floating box {r:?}");
                }
            }
        }
    }
    assert_eq!(objects, 1, "the floating box is placed once");
    assert!(beside > 0, "some body lines share the floating box's row");
}

#[test]
fn a_continuation_uses_the_available_edges_beside_a_destination_float() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    for width in [30.0, 60.0, 90.0] {
        for align in [Align::Left, Align::Center, Align::Right, Align::Justify] {
            let mut d = unequal_column_doc(|p| p.align = Some(align));
            let id = text_box(
                &mut d,
                0,
                "Box",
                width,
                16.0,
                Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 228.0, y: 72.0, dist: 0.0, ..Default::default() },
            );
            assert_body_clear_of_box(&lay(&d), id);
        }
    }
}

#[test]
fn a_wider_continuation_wraps_around_a_newly_due_float() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    let mut d = unequal_column_doc(|_| {});
    let id = text_box(
        &mut d,
        100,
        "Box",
        60.0,
        16.0,
        Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 0.0, y: 0.0, dist: 0.0, ..Default::default() },
    );
    assert_body_clear_of_box(&lay(&d), id);
}

#[test]
fn bidi_continuations_rebreak_without_losing_or_repeating_text() {
    for rtl in [false, true] {
        let text = "שלום עולם Latin words שלום נוספים ".repeat(40);
        let template = unequal_column_doc(|_| {});
        let mut d = Document::from_text(&text);
        d.last_section = template.last_section;
        d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.bidi = Some(rtl)).unwrap();
        let l = lay(&d);
        let col2 = l.pages[0].body.x + d.last_section.column_boxes()[1].0;
        let line = l.pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Lines { para, l0, x, .. } if (*x - col2).abs() < 0.5 => Some(&para.lines[*l0]),
                _ => None,
            })
            .unwrap();
        let mut suffix = Document::from_text(&text[line.start..]);
        suffix.last_section.page_w = 424.0;
        suffix.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.bidi = Some(rtl)).unwrap();
        let expected = lay(&suffix);
        let end = expected.pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Lines { para, l0, .. } => Some(para.lines[*l0].stop),
                _ => None,
            })
            .unwrap();
        assert_eq!(line.stop - line.start, end, "bidi continuation uses the destination width");
        let mut covered = 0;
        for item in l.pages.iter().flat_map(|p| &p.items) {
            if let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } = item
                && path.0 == [0]
            {
                for line in &para.lines[*l0..*l1] {
                    assert_eq!(line.start, covered);
                    covered = line.stop;
                }
            }
        }
        assert_eq!(covered, text.len());
    }
}

#[test]
fn a_full_width_destination_float_keeps_its_vertical_clearance() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    let mut d = unequal_column_doc(|_| {});
    let id = text_box(
        &mut d,
        0,
        "Box",
        280.0,
        16.0,
        Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 228.0, y: 72.0, dist: 0.0, ..Default::default() },
    );
    let l = lay(&d);
    let r = l.text_box(id, 0).unwrap().rect;
    let lines = line_boxes(&l, 0, 0, 228.0);
    assert!(!lines.is_empty(), "the continuation fits below the float");
    for (_, top, _, _) in lines {
        assert!(top >= r.bottom() - 0.01, "a full-width float moves the continuation below it: {top} vs {r:?}");
    }
    let next = line_boxes(&l, 1, 0, 72.0);
    assert!(!next.is_empty());
    assert!((next[0].1 - 72.0).abs() < 0.01, "the next page does not retain the old float's clearance");
}
