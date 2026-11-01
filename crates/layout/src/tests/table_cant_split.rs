//! #4736: an oversized cantSplit row moves once, then retains its text across pages.
use super::*;

/// Every source line is placed once, wholly inside its page's body rather than below it.
fn assert_complete_row(layout: &DocLayout, block: u32, row: u32, count: usize) -> Vec<usize> {
    let mut seen = std::collections::BTreeSet::new();
    let mut pages = Vec::new();
    let mut total = 0;
    for (page_index, page) in layout.pages.iter().enumerate() {
        for item in &page.items {
            let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, y, .. } = item else { continue };
            if path.0.len() != 4 || path.0[..3] != [block, row, 0] {
                continue;
            }
            assert!(seen.insert(path.0[3]), "a short source paragraph is drawn only once");
            total += l1 - l0;
            assert!(*y >= page.body.y - 0.02, "paragraph starts above the body: {y}");
            let bottom = item_bottom(*y, para, *l0, *l1).unwrap();
            assert!(bottom <= page.body.bottom() + 0.02, "paragraph {} runs below page {}: {bottom} > {}", path.0[3], page_index, page.body.bottom());
            if pages.last() != Some(&page_index) {
                pages.push(page_index);
            }
        }
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), (0..count as u32).collect::<Vec<_>>());
    assert_eq!(total, count, "no source line is lost or repeated");
    pages
}

/// Word for Mac 16.114.2 starts the oversized row on a fresh page, then continues it.
#[test]
fn oversized_cant_split_row_moves_once_then_flows() {
    for minimum in [false, true] {
        let (layout, rows) = at_least_rows(200.0, 1, |table| {
            table.rows[0].props.cant_split = true;
            table.rows[0].cells[0].blocks = lines_in_cell(80, "Synthetic");
            if minimum {
                table.rows[0].props.height = Some(12.0);
                table.rows[0].props.height_rule = HeightRule::AtLeast;
            }
        });
        assert_eq!(rows[0], Vec::<usize>::new(), "cantSplit still moves off the partly filled page");
        let pages = assert_complete_row(&layout, 1, 0, 80);
        assert_eq!(pages.first(), Some(&1));
        assert!(pages.len() >= 2, "a row taller than a page must continue");
    }
}

/// At the top of a fresh page there is no benefit in advancing again before splitting.
#[test]
fn oversized_cant_split_row_at_page_top_flows_without_a_blank_page() {
    let (layout, _) = at_least_rows(648.0, 1, |table| {
        table.rows[0].props.cant_split = true;
        table.rows[0].cells[0].blocks = lines_in_cell(80, "Synthetic");
    });
    let pages = assert_complete_row(&layout, 0, 0, 80);
    assert_eq!(pages.first(), Some(&0));
    assert!(pages.len() >= 2);
}

/// The exception does not split a cantSplit row that fits on a fresh page.
#[test]
fn fitting_cant_split_row_still_moves_whole() {
    let (layout, rows) = at_least_rows(200.0, 1, |table| {
        table.rows[0].props.cant_split = true;
        table.rows[0].cells[0].blocks = lines_in_cell(20, "Synthetic");
    });
    assert_eq!(rows, vec![vec![], vec![0]]);
    assert_eq!(assert_complete_row(&layout, 1, 0, 20), vec![1]);
}

/// Exact heights retain their explicit clipping contract, even with cantSplit present.
#[test]
fn exact_height_cant_split_row_is_not_split_to_reveal_overflow() {
    let (layout, rows) = at_least_rows(648.0, 1, |table| {
        let row = &mut table.rows[0];
        row.props.cant_split = true;
        row.props.height = Some(240.0);
        row.props.height_rule = HeightRule::Exact;
        row.cells[0].blocks = lines_in_cell(80, "Synthetic");
    });
    assert_eq!(rows, vec![vec![0]]);
    assert!(layout.pages[0].items.iter().any(|item| matches!(item, Placed::Cell { table, row: 0, clip: true, .. } if table.0 == [0])));
}

/// A section top can lie below earlier physical-page content; it is not a fresh page.
fn continuous_section_table(count: usize, header: bool) -> DocLayout {
    let (mut doc, at) = at_least_rows_filler(200.0);
    assert_eq!(at, 1);
    let mut section = doc.last_section.clone();
    section.start = SectionStart::Continuous;
    doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(section));
    doc.last_section.start = SectionStart::Continuous;
    let mut table = Table::new(if header { 2 } else { 1 }, 1, 468.0);
    let body = usize::from(header);
    if header {
        table.rows[0].props.header = true;
        table.rows[0].props.height = Some(64.0);
        table.rows[0].props.height_rule = HeightRule::AtLeast;
    }
    table.rows[body].props.cant_split = true;
    table.rows[body].cells[0].blocks = lines_in_cell(count, "Synthetic");
    set_at_least_row_spacing(&mut table);
    doc.insert_block(StoryRef::Body, &Path::top(at), Block::Table(table)).unwrap();
    let layout = lay(&doc);
    assert_pagination_fixture_font(&layout);
    layout
}

/// Both fitting and oversized rows leave a mid-page section before the split exception applies.
#[test]
fn cant_split_at_a_mid_page_section_top_moves_to_a_fresh_page() {
    for count in [20, 80] {
        let layout = continuous_section_table(count, false);
        let pages = assert_complete_row(&layout, 1, 0, count);
        assert_eq!(pages.first(), Some(&1), "a section boundary must not enable splitting on page 1");
        if count == 20 {
            assert_eq!(pages, vec![1], "a row that fits on the next page stays whole");
        }
    }
}

/// Capture freshness before placing one or more repeated headers above the oversized row.
#[test]
fn oversized_cant_split_row_starts_below_headers_on_the_first_fresh_page() {
    for headers in [1, 2] {
        let (layout, rows) = at_least_rows(648.0, headers + 1, |table| {
            for row in table.rows.iter_mut().take(headers) {
                row.props.header = true;
                row.props.height = Some(64.0);
                row.props.height_rule = HeightRule::AtLeast;
            }
            table.rows[headers].props.cant_split = true;
            table.rows[headers].cells[0].blocks = lines_in_cell(80, "Synthetic");
        });
        let pages = assert_complete_row(&layout, 0, headers as u32, 80);
        assert_eq!(pages.first(), Some(&0), "do not leave an initial header-only page");
        for page in &pages {
            assert_eq!(rows[*page], (0..=headers).collect::<Vec<_>>(), "repeat every header with body content");
        }
    }
}

/// Headers at a mid-page continuous section move with their oversized first body row.
#[test]
fn continuous_section_does_not_leave_repeated_headers_stranded() {
    let layout = continuous_section_table(80, true);
    let rows = at_least_rows_pages(&layout);
    assert!(rows[0].is_empty(), "the header must move with its body: {rows:?}");
    let pages = assert_complete_row(&layout, 1, 1, 80);
    assert_eq!(pages.first(), Some(&1));
    assert!(pages.iter().all(|page| rows[*page] == [0, 1]));
}

/// A fresh page's usable space includes the repeated headers; fitting rows stay whole.
#[test]
fn fitting_cant_split_row_stays_whole_below_repeated_headers() {
    for room in [648.0, 200.0] {
        for headers in [1, 2] {
            let (layout, rows) = at_least_rows(room, headers + 1, |table| {
                for row in table.rows.iter_mut().take(headers) {
                    row.props.header = true;
                    row.props.height = Some(64.0);
                    row.props.height_rule = HeightRule::AtLeast;
                }
                table.rows[headers].props.cant_split = true;
                table.rows[headers].cells[0].blocks = lines_in_cell(20, "Synthetic");
            });
            let expected = usize::from(room < 648.0);
            assert_eq!(assert_complete_row(&layout, expected as u32, headers as u32, 20), vec![expected]);
            assert_eq!(rows[expected], (0..=headers).collect::<Vec<_>>());
            if expected > 0 {
                assert!(rows[0].is_empty(), "headers move with a fitting cantSplit row");
            }
        }
    }
}

/// Repeating headers can make a row oversized even if the same row fits on a blank page.
#[test]
fn repeated_headers_count_toward_the_fresh_page_budget() {
    let (plain, _) = at_least_rows(648.0, 1, |table| {
        table.rows[0].props.cant_split = true;
        table.rows[0].cells[0].blocks = lines_in_cell(40, "Synthetic");
    });
    assert_eq!(assert_complete_row(&plain, 0, 0, 40), vec![0]);
    let (with_headers, rows) = at_least_rows(648.0, 2, |table| {
        table.rows[0].props.header = true;
        table.rows[0].props.height = Some(128.0);
        table.rows[0].props.height_rule = HeightRule::AtLeast;
        table.rows[1].props.cant_split = true;
        table.rows[1].cells[0].blocks = lines_in_cell(40, "Synthetic");
    });
    let pages = assert_complete_row(&with_headers, 0, 1, 40);
    assert_eq!(pages, vec![0, 1], "do not emit a header-only page when no page can hold the row whole");
    assert!(pages.iter().all(|page| rows[*page] == [0, 1]));
}

/// The office corpus's long row follows another table and must retain all lines below headers.
#[test]
fn office_corpus_row_after_another_table_keeps_every_line_inside_the_page() {
    let mut doc = pagination_fixture_document("SYNTHETIC OFFICE CONTROL");
    let mut preceding = Table::new(30, 1, 468.0);
    for row in &mut preceding.rows {
        row.props.cant_split = true;
        row.props.height = Some(44.0);
        row.props.height_rule = HeightRule::AtLeast;
        row.cells[0].blocks = lines_in_cell(1, "Synthetic preceding row");
    }
    set_at_least_row_spacing(&mut preceding);
    doc.insert_block(StoryRef::Body, &Path::top(1), Block::Table(preceding)).unwrap();
    let mut title = wordcraft_doc::Paragraph::with_text("SYNTHETIC LONG ROW", Default::default());
    title.props.keep_next = Some(true);
    doc.insert_block(StoryRef::Body, &Path::top(2), Block::Para(title)).unwrap();
    let mut long = Table::new(2, 1, 468.0);
    long.rows[0].props.header = true;
    long.rows[0].props.height = Some(28.0);
    long.rows[0].props.height_rule = HeightRule::AtLeast;
    long.rows[0].cells[0].blocks = lines_in_cell(1, "Synthetic repeated header");
    long.rows[1].props.cant_split = true;
    long.rows[1].cells[0].blocks = lines_in_cell(100, "Synthetic long row");
    set_at_least_row_spacing(&mut long);
    doc.insert_block(StoryRef::Body, &Path::top(3), Block::Table(long)).unwrap();
    doc.insert_block(StoryRef::Body, &Path::top(4), Block::Para(wordcraft_doc::Paragraph::with_text("SYNTHETIC END", Default::default()))).unwrap();
    let layout = lay(&doc);
    assert_pagination_fixture_font(&layout);
    let pages = assert_complete_row(&layout, 3, 1, 100);
    assert!(pages.len() >= 2, "an oversized row must continue after the preceding table");
    for page in pages {
        assert!(
            layout.pages[page].items.iter().any(|item| { matches!(item, Placed::Cell { table, row: 0, .. } if table.0 == [3]) }),
            "the header follows every body fragment"
        );
    }
}
