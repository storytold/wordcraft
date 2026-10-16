use super::*;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, ParaProps, TextDirection};
use wordcraft_doc::{Pos, Table};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

#[derive(Clone, Copy)]
enum SpacingLocation {
    Cell,
    Body,
    Header,
}

fn legacy_table_gap(mode: u32, allow: bool, location: SpacingLocation) -> f32 {
    let mut d = Document::new();
    d.settings.compat_mode = mode;
    d.settings.allow_space_of_same_style_in_table = allow;
    let chr = wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() };
    let mut a = wordcraft_doc::Paragraph::with_text("Alpha probe", chr.clone());
    a.props = ParaProps {
        space_before: Some(0.0),
        space_after: Some(12.0),
        line_spacing: Some(wordcraft_doc::props::LineSpacing::Exactly(24.0)),
        contextual_spacing: Some(true),
        ..Default::default()
    };
    let mut b = wordcraft_doc::Paragraph::with_text("Beta probe", chr);
    b.props = ParaProps { space_after: Some(0.0), ..a.props.clone() };
    let blocks = vec![wordcraft_doc::para_block(a), wordcraft_doc::para_block(b)];
    match location {
        SpacingLocation::Cell => {
            let mut table = Table::new(1, 1, 468.0);
            // Cell identity must not depend on whether table-style formatting exists.
            table.props.style = None;
            table.props.cell_margins = Some([0.0; 4]);
            table.rows[0].cells[0].blocks = blocks;
            d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(table))];
        }
        SpacingLocation::Body => d.body = blocks,
        SpacingLocation::Header => {
            let id = d.add_part(wordcraft_doc::PartKind::Header, blocks);
            d.last_section.headers.default = Some(id);
        }
    }
    let l = lay(&d);
    let items = if matches!(location, SpacingLocation::Header) { &l.pages[0].header } else { &l.pages[0].items };
    let baselines: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            Placed::Lines { para, y, .. } => Some(y + para.lines[0].baseline),
            _ => None,
        })
        .collect();
    assert_eq!(baselines.len(), 2);
    baselines[1] - baselines[0] - 24.0
}

#[test]
fn legacy_table_contextual_spacing_flag_keeps_after_space() {
    // Original Word for Mac 16.113.3 probe: after 12 pt, before 0 pt, both contextual.
    // Zero before-space keeps the compatibility rule independent of ordinary overlap.
    for mode in [11, 12, 14] {
        let gap = legacy_table_gap(mode, true, SpacingLocation::Cell);
        assert!((gap - 12.0).abs() < 0.01, "mode={mode}: gap={gap}, expected 12 pt");
    }
}

#[test]
fn legacy_table_contextual_spacing_flag_is_scoped_to_cells_and_legacy_mode() {
    for (mode, allow) in [(11, false), (12, false), (14, false), (15, false), (15, true)] {
        assert!(legacy_table_gap(mode, allow, SpacingLocation::Cell).abs() < 0.01);
    }
    for mode in [14, 15] {
        for location in [SpacingLocation::Body, SpacingLocation::Header] {
            assert!(legacy_table_gap(mode, true, location).abs() < 0.01);
        }
    }
}

fn contextual_gap(after: f32, before: f32, first: bool, second: bool, different_style: bool, header: bool) -> f32 {
    use wordcraft_doc::props::LineSpacing;
    let mut d = Document::new();
    for id in ["SpacingA", "SpacingB"] {
        d.styles.upsert(wordcraft_doc::Style { id: id.into(), name: id.into(), ..Default::default() });
    }
    let chr = wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() };
    let props = ParaProps {
        style: Some("SpacingA".into()),
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Exactly(24.0)),
        ..Default::default()
    };
    let mut a = wordcraft_doc::Paragraph::with_text("Alpha probe", chr.clone());
    a.props = ParaProps { space_after: Some(after), contextual_spacing: Some(first), ..props.clone() };
    let mut b = wordcraft_doc::Paragraph::with_text("Beta probe", chr);
    b.props = ParaProps {
        style: Some(if different_style { "SpacingB" } else { "SpacingA" }.into()),
        space_before: Some(before),
        contextual_spacing: Some(second),
        ..props
    };
    let paragraphs = vec![wordcraft_doc::para_block(a), wordcraft_doc::para_block(b)];
    if header {
        let id = d.add_part(wordcraft_doc::PartKind::Header, paragraphs);
        d.last_section.headers.default = Some(id);
    } else {
        d.body = paragraphs;
    }
    let l = lay(&d);
    let items = if header { &l.pages[0].header } else { &l.pages[0].items };
    let baselines: Vec<_> =
        items.iter().filter_map(|i| if let Placed::Lines { para, y, .. } = i { Some(y + para.lines[0].baseline) } else { None }).collect();
    assert_eq!(baselines.len(), 2);
    baselines[1] - baselines[0] - 24.0
}

// Original two-paragraph probes measured in Word for Mac 16.113.3, build 26092714.
// Exactly 24 pt spacing isolates the extra gap from font-dependent line height.
#[test]
fn contextual_after_removes_a_smaller_before_gap() {
    for header in [false, true] {
        let gap = contextual_gap(12.0, 8.0, true, false, false, header);
        assert!(gap.abs() < 0.01, "header={header}: gap={gap}, expected 0 pt");
    }
}

#[test]
fn contextual_after_keeps_only_the_residual_before_gap() {
    // ISO/IEC 29500-1 §17.3.1.9's worked example: max(10, 12) - 10 = 2 pt.
    for header in [false, true] {
        let gap = contextual_gap(10.0, 12.0, true, false, false, header);
        assert!((gap - 2.0).abs() < 0.01, "header={header}: gap={gap}, expected 2 pt");
    }
}

#[test]
fn contextual_spacing_preserves_each_side_and_style_identity() {
    for header in [false, true] {
        for (after, before, first, second, different, expected) in [
            (12.0, 8.0, false, true, false, 12.0),
            (12.0, 8.0, true, true, false, 0.0),
            (12.0, 0.0, true, true, true, 12.0),
            (12.0, 0.0, false, false, false, 12.0),
        ] {
            let gap = contextual_gap(after, before, first, second, different, header);
            assert!(
                (gap - expected).abs() < 0.01,
                "header={header}, first={first}, second={second}, different={different}: gap={gap}, expected {expected}"
            );
        }
    }
}

#[test]
fn contextual_spacing_removes_every_gap_in_a_four_paragraph_run() {
    use wordcraft_doc::props::LineSpacing;
    for table in [false, true] {
        let mut doc = Document::new();
        doc.styles.upsert(wordcraft_doc::Style { id: "SpacingA".into(), name: "SpacingA".into(), ..Default::default() });
        let paragraphs = ["One", "Two", "Three", "Four"]
            .into_iter()
            .map(|text| {
                let mut p = wordcraft_doc::Paragraph::with_text(
                    text,
                    wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() },
                );
                p.props = ParaProps {
                    style: Some("SpacingA".into()),
                    space_before: Some(6.0),
                    space_after: Some(6.0),
                    contextual_spacing: Some(true),
                    line_spacing: Some(LineSpacing::Exactly(24.0)),
                    ..Default::default()
                };
                wordcraft_doc::para_block(p)
            })
            .collect::<Vec<_>>();
        if table {
            let mut cell = Table::new(1, 1, 400.0);
            cell.rows[0].cells[0].blocks = paragraphs;
            doc.body = vec![Arc::new(Block::Table(cell))];
        } else {
            doc.body = paragraphs;
        }
        let layout = lay(&doc);
        let baselines = layout.pages[0]
            .items
            .iter()
            .filter_map(|item| match item {
                Placed::Lines { para, y, .. } => Some(y + para.lines[0].baseline),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(baselines.len(), 4);
        // Every same-style contextual boundary has zero extra space, including after paragraph two.
        for pair in baselines.windows(2) {
            assert!((pair[1] - pair[0] - 24.0).abs() < 0.01, "table={table}: {baselines:?}");
        }
    }
}

// Contextual and automatic list spacing can suppress the same after-space only once.
#[test]
fn contextual_automatic_lists_do_not_subtract_the_gap_twice() {
    use wordcraft_doc::props::{LineSpacing, NumRef};
    for table in [false, true] {
        let mut doc = Document::new();
        let num = doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        let paragraphs = ["First", "Second", "Third"]
            .into_iter()
            .map(|text| {
                let mut p = wordcraft_doc::Paragraph::with_text(text, Default::default());
                p.props = ParaProps {
                    contextual_spacing: Some(true),
                    space_before_auto: Some(true),
                    space_after_auto: Some(true),
                    numbering: Some(NumRef { num, level: 0 }),
                    line_spacing: Some(LineSpacing::Exactly(24.0)),
                    ..Default::default()
                };
                wordcraft_doc::para_block(p)
            })
            .collect::<Vec<_>>();
        if table {
            let mut cell = Table::new(1, 1, 400.0);
            cell.rows[0].cells[0].blocks = paragraphs;
            doc.body = vec![Arc::new(Block::Table(cell))];
        } else {
            doc.body = paragraphs;
        }
        let layout = lay(&doc);
        let baselines = layout.pages[0]
            .items
            .iter()
            .filter_map(|item| match item {
                Placed::Lines { para, y, .. } => Some(y + para.lines[0].baseline),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(baselines.len(), 3);
        for pair in baselines.windows(2) {
            assert!((pair[1] - pair[0] - 24.0).abs() < 0.01, "table={table}: {baselines:?}");
        }
    }
}

fn lines_of(l: &DocLayout) -> usize {
    l.pages.iter().flat_map(|p| p.items.iter()).map(|it| if let Placed::Lines { l0, l1, .. } = it { l1 - l0 } else { 0 }).sum()
}

#[test]
fn empty_doc_has_one_page() {
    let d = Document::new();
    let l = lay(&d);
    assert_eq!(l.pages.len(), 1);
    assert_eq!(lines_of(&l), 1);
    let c = l.caret(&Pos::body(0, 0)).unwrap();
    assert_eq!(c.page, 0);
    assert!((c.x - 72.0).abs() < 0.5, "{c:?}");
    assert!(c.top >= 72.0 && c.height > 8.0);
}

#[test]
fn hidden_marks_take_no_space() {
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Before", Default::default()))];
    for _ in 0..20 {
        let mut p = wordcraft_doc::Paragraph::new();
        p.mark.hidden = Some(true);
        d.body.push(wordcraft_doc::para_block(p));
    }
    d.body.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("After", Default::default())));
    let l = lay(&d);
    let y = |block: usize| {
        l.pages
            .iter()
            .flat_map(|p| &p.items)
            .find_map(|item| match item {
                Placed::Lines { path, y, .. } if *path == Path::top(block) => Some(*y),
                _ => None,
            })
            .unwrap()
    };
    let mut plain = Document::new();
    plain.body = vec![
        wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Before", Default::default())),
        wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("After", Default::default())),
    ];
    let plain_layout = lay(&plain);
    let plain_after = plain_layout.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { path, y, .. } if *path == Path::top(1) => Some(*y),
            _ => None,
        })
        .unwrap();
    assert!((y(21) - plain_after).abs() < 0.1, "hidden paragraphs moved the following text to {} vs {plain_after}", y(21));
    let shown = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden: true, ..Default::default() });
    let after = shown.caret(&Pos::body(21, 0)).unwrap();
    assert!(after.top > plain_after + 100.0, "showing hidden marks must restore their lines");
}

#[test]
fn long_paragraph_wraps_and_paginates() {
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(400);
    let d = Document::from_text(&text);
    let l = lay(&d);
    assert!(l.pages.len() >= 3, "pages {}", l.pages.len());
    // Every line fits the text width.
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { para, l0, l1, x, .. } = it {
                for li in *l0..*l1 {
                    let line = &para.lines[li];
                    let w = para.x_of(li, line.stop).unwrap() + x;
                    assert!(w <= 612.0 - 72.0 + 30.0, "line too wide: {w}");
                }
            }
        }
    }
}

#[test]
fn reading_layout_reflows_into_viewport_height_pages_without_changing_print_layout() {
    let d = Document::from_text(&"A moderately long sentence that should wrap across reading columns. ".repeat(250));
    let opts = LayoutOptions { view: ViewMode::Read, web_width: 230.0, read_height: 390.0, ..Default::default() };
    let read = layout(&d, &mut LayoutCache::new(), &opts);
    assert!(read.pages.len() > 5, "reading column count: {}", read.pages.len());
    assert!(read.pages.iter().all(|p| p.w == 230.0 && p.h == 390.0));
    assert!(read.pages.iter().all(|p| p.body.w < p.w && p.body.h < p.h));
    assert_eq!(
        lines_of(&read),
        lines_of(&layout(&d, &mut LayoutCache::new(), &LayoutOptions { view: ViewMode::Web, web_width: 230.0, ..Default::default() }))
    );
    let print = lay(&d);
    assert_eq!(print.pages[0].w, d.sections()[0].1.page_w);
    assert_eq!(print.pages[0].h, d.sections()[0].1.page_h);
    // Window height changes page breaks, not the text, and a malformed dimension stays bounded.
    let short = layout(&d, &mut LayoutCache::new(), &LayoutOptions { read_height: 260.0, ..opts.clone() });
    assert!(short.pages.len() > read.pages.len());
    let spaced = layout(&d, &mut LayoutCache::new(), &LayoutOptions { read_line_spacing: Some(2.0), ..opts.clone() });
    assert!(spaced.pages.len() > read.pages.len(), "reader-only line spacing repaginates text");
    let invalid = layout(&d, &mut LayoutCache::new(), &LayoutOptions { web_width: f32::NAN, read_height: f32::INFINITY, ..opts });
    assert!(invalid.pages.iter().all(|p| p.w.is_finite() && p.h.is_finite()));
}

#[test]
fn hit_and_caret_agree() {
    let d = Document::from_text("Hello world, this is WordCraft.\nSecond paragraph here.");
    let l = lay(&d);
    for off in [0, 5, 12, 31] {
        let pos = Pos::body(0, off);
        let c = l.caret(&pos).unwrap();
        let back = l.hit(c.page, c.x + 0.1, c.top + c.height / 2.0, StoryRef::Body).unwrap();
        assert_eq!(back, pos, "off {off}");
    }
    let c1 = l.caret(&Pos::body(1, 0)).unwrap();
    let (down, _) = l.vertical(&Pos::body(0, 0), None, 1, 0).unwrap();
    assert_eq!(down.path, Path::top(1));
    assert!(c1.top > l.caret(&Pos::body(0, 0)).unwrap().top);
    let (up, _) = l.vertical(&Pos::body(1, 3), None, -1, 0).unwrap();
    assert_eq!(up.path, Path::top(0));
}

#[test]
fn selection_bar_finds_the_line_beside_the_pointer() {
    // #856: the left margin beside a line is Word's selection bar.
    let text = format!("{}\nShort.", "The quick brown fox jumps over the lazy dog. ".repeat(8));
    let mut d = Document::from_text(&text);
    d.insert_block(StoryRef::Body, &Path::top(2), wordcraft_doc::Block::Table(Table::new(1, 1, 468.0))).unwrap();
    let l = lay(&d);
    let p = &l.pages[0];
    let c = l.caret(&Pos::body(0, 0)).unwrap();
    let first = l.selection_bar_at(0, 30.0, c.top + c.height / 2.0).unwrap();
    assert_eq!(first, Pos::body(0, 0));
    // The second line of the wrapped paragraph: its start, and its span is the whole line.
    let line2 = l.selection_bar_at(0, 30.0, c.top + c.height * 1.5).unwrap();
    assert_eq!(line2.path, Path::top(0));
    assert!(line2.off > 0);
    let (a, b, last) = l.line_span(&line2, 0).unwrap();
    assert_eq!(a, line2);
    assert!(b.off > a.off && !last);
    let (_, b, last) = l.line_span(&Pos::body(1, 0), 0).unwrap();
    assert_eq!(b.off, "Short.".len());
    assert!(last);
    // In the text column, the header area, off the page or beside a table: not the bar.
    assert_eq!(l.selection_bar_at(0, p.body.x + 5.0, c.top + 2.0), None);
    assert_eq!(l.selection_bar_at(0, 30.0, p.body.y - 10.0), None);
    assert_eq!(l.selection_bar_at(0, -5.0, c.top + 2.0), None);
    assert_eq!(l.selection_bar_at(0, f32::NAN, c.top), None);
    assert_eq!(l.selection_bar_at(9, 30.0, c.top), None);
    let cell = l.caret(&Pos { story: StoryRef::Body, path: Path(vec![2, 0, 0, 0]), off: 0 }).unwrap();
    assert_eq!(l.selection_bar_at(0, 30.0, cell.top + cell.height / 2.0), None);
    // Dragging takes the nearest top-level line at any height.
    assert_eq!(l.body_line_at(0, -100.0), Some(Pos::body(0, 0)));
}

#[test]
fn deletions_leave_the_final_text_layout() {
    // "Keep " + deleted "DELETEDTEXT " (5..17) + inserted "INSERTED".
    let mut d = Document::from_text("Keep DELETEDTEXT INSERTED");
    d.format_range(&Pos::body(0, 5), &Pos::body(0, 17), &|c| c.del = Some(0)).unwrap();
    d.format_range(&Pos::body(0, 17), &Pos::body(0, 25), &|c| c.ins = Some(0)).unwrap();
    let texts = |l: &DocLayout, markup: bool| -> (String, usize) {
        let items = display::page_display(&d, &l.pages[0], &display::DisplayOptions { markup, ..Default::default() });
        let text = items.iter().filter_map(|i| if let display::Draw::Glyphs { text, .. } = i { Some(text.as_str()) } else { None }).collect();
        (text, items.iter().filter(|i| matches!(i, display::Draw::Line { .. })).count())
    };
    // Markup: the deletion is laid out and struck through, the insertion underlined, and the
    // changed line has a bar in the margin.
    let full = lay(&d);
    assert_eq!(texts(&full, true), ("Keep DELETEDTEXT INSERTED".into(), 3));
    // Without markup a full layout still never prints the deletion as plain text.
    assert!(!texts(&full, false).0.contains("DELETED"));
    // The final layout gives the deletion no width.
    let fin = layout(&d, &mut LayoutCache::new(), &LayoutOptions { hide_deleted: true, ..Default::default() });
    assert_eq!(texts(&fin, false), ("Keep INSERTED".into(), 0));
    let at = |off| fin.caret(&Pos::body(0, off)).unwrap();
    assert!((at(5).x - at(17).x).abs() < 0.01, "{:?} {:?}", at(5), at(17));
    // Every offset maps to a caret, and clicks land outside the hidden text.
    for off in 0..=25 {
        let c = at(off);
        let back = fin.hit(c.page, c.x + 0.1, c.top + c.height / 2.0, StoryRef::Body).unwrap();
        assert!(back.off <= 5 || back.off >= 17, "off {off} hit {back:?}");
    }
}

#[test]
fn deleted_soft_hyphen_is_no_hyphenation_point_without_markup() {
    // "ab\u{ad}cdef" in a column where "ab-" fits but the whole word does not; only the soft
    // hyphen is track-deleted.
    let mut d = Document::from_text("ab\u{ad}cdef");
    d.format_range(&Pos::body(0, 2), &Pos::body(0, 4), &|c| c.del = Some(0)).unwrap();
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.indent_right = Some(468.0 - 26.0)).unwrap();
    let hyphens = |hide_deleted: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { hide_deleted, ..Default::default() });
        let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        para.lines.iter().filter(|l| l.hyphen.is_some()).count()
    };
    // With markup the deleted soft hyphen is still there, and the word breaks at it.
    assert_eq!(hyphens(false), 1);
    // Final text: the soft hyphen is gone, so is the hyphen.
    assert_eq!(hyphens(true), 0);
}

#[test]
fn deleted_text_adds_no_line_break_opportunity_without_markup() {
    // "xx ab\u{ad}cdef" in a column where "xx ab-" and "abcdef" fit but "xx abcdef" does not; only
    // the soft hyphen (5..7) is track-deleted.
    let mut d = Document::from_text("xx ab\u{ad}cdef");
    d.format_range(&Pos::body(0, 5), &Pos::body(0, 7), &|c| c.del = Some(0)).unwrap();
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.indent_right = Some(468.0 - 40.0)).unwrap();
    let line_starts = |hide_deleted: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { hide_deleted, ..Default::default() });
        let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        para.lines.iter().map(|l| para.clusters.get(l.c0).map_or(usize::MAX, |c| c.start)).collect::<Vec<_>>()
    };
    // With markup the word breaks at the (deleted) soft hyphen.
    assert_eq!(line_starts(false), vec![0, 7]);
    // Final text: "abcdef" is one word, so the line breaks at the space before it.
    assert_eq!(line_starts(true), vec![0, 3]);
}

#[test]
fn hidden_soft_hyphen_is_no_hyphenation_point_when_hidden_text_is_not_shown() {
    // As with deletions: "ab\u{ad}cdef" where only the soft hyphen is hidden text.
    let mut d = Document::from_text("ab\u{ad}cdef");
    d.format_range(&Pos::body(0, 2), &Pos::body(0, 4), &|c| c.hidden = Some(true)).unwrap();
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.indent_right = Some(468.0 - 26.0)).unwrap();
    let hyphens = |show_hidden: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden, ..Default::default() });
        let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        para.lines.iter().filter(|l| l.hyphen.is_some()).count()
    };
    // Hidden text shown: the soft hyphen is there, and the word breaks at it.
    assert_eq!(hyphens(true), 1);
    // Not shown (as printed): no soft hyphen, so no hyphen. Word breaks "abc" / "def" here.
    assert_eq!(hyphens(false), 0);
}

#[test]
fn hidden_text_adds_no_line_break_opportunity_when_not_shown() {
    // "xx ab" + hidden soft hyphen or space + "cdef", in a column where "xx ab-" and "abcdef" fit
    // but "xx abcdef" does not. Measured in Word (hidden text not printed): "xx" / "abcdef" both times.
    for hidden in ["\u{ad}", " "] {
        let mut d = Document::from_text(&format!("xx ab{hidden}cdef"));
        let end = 5 + hidden.len();
        d.format_range(&Pos::body(0, 5), &Pos::body(0, end), &|c| c.hidden = Some(true)).unwrap();
        d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.indent_right = Some(468.0 - 40.0)).unwrap();
        let line_starts = |show_hidden: bool| {
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden, ..Default::default() });
            let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
            para.lines.iter().map(|l| para.clusters.get(l.c0).map_or(usize::MAX, |c| c.start)).collect::<Vec<_>>()
        };
        // Shown, the word breaks at the hidden soft hyphen or space.
        assert_eq!(line_starts(true), vec![0, end], "{hidden:?}");
        // Not shown, "abcdef" is one word, so the line breaks at the space before it.
        assert_eq!(line_starts(false), vec![0, 3], "{hidden:?}");
    }
}

/// How much bigger the second document is in the linear-time tests. At 8x a linear layout costs
/// about 8x as much and a quadratic one about 64x, so a limit of 24x stays far from both.
const SCALE: usize = 8;

/// The CPU time `f` takes on this thread, in seconds. On Linux it is the scheduler's count of the
/// time the thread actually ran, so the time it waits while other work on a busy machine has the
/// CPU is left out; elsewhere, wall-clock time.
fn cpu_cost(f: &mut impl FnMut()) -> f64 {
    let ran = || {
        let s = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
        s.split_whitespace().next()?.parse::<u64>().ok()
    };
    let (t, before) = (std::time::Instant::now(), ran());
    f();
    match (before, ran()) {
        (Some(a), Some(b)) => b.saturating_sub(a) as f64 * 1e-9,
        _ => t.elapsed().as_secs_f64(),
    }
}

/// How much more `big` costs than `small`, for `big` an input `scale` times the size of `small`'s:
/// (small, big, big / small), ~`scale` when linear and ~`scale`² when quadratic. Each is the
/// cheapest of several runs after a warm-up, the runs alternating so a busy spell slows both sizes
/// alike; `small` is timed `scale` times over per run, so both timings are long against the clock's
/// resolution.
fn cost_ratio(scale: usize, mut small: impl FnMut(), mut big: impl FnMut()) -> (f64, f64, f64) {
    small();
    big();
    let mut small_scaled = || (0..scale).for_each(|_| small());
    let (mut t1, mut tk) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..5 {
        t1 = t1.min(cpu_cost(&mut small_scaled) / scale as f64);
        tk = tk.min(cpu_cost(&mut big));
    }
    (t1, tk, tk / t1.max(1e-9))
}

/// [`cost_ratio`] of laying out `doc(SCALE * n)` against `doc(n)`.
fn layout_time_ratio(doc: impl Fn(usize) -> Document, n: usize, opts: &LayoutOptions) -> (f64, f64, f64) {
    let (small, big) = (doc(n), doc(SCALE * n));
    cost_ratio(
        SCALE,
        || {
            layout(&small, &mut LayoutCache::new(), opts);
        },
        || {
            layout(&big, &mut LayoutCache::new(), opts);
        },
    )
}

#[test]
fn hidden_separators_with_automatic_hyphenation_lay_out_in_linear_time() {
    // "hyphenation " n times with every space hidden: one joined word of 11n letters reaches the
    // hyphenator, whose limit checks once rescanned the word for every candidate point.
    let doc = |n: usize| {
        let mut d = Document::from_text(&"hyphenation ".repeat(n));
        d.settings.auto_hyphenation = true;
        for k in 0..n {
            let at = 12 * k + 11;
            d.format_range(&Pos::body(0, at), &Pos::body(0, at + 1), &|c| c.hidden = Some(true)).unwrap();
        }
        d
    };
    let (t1, tk, ratio) = layout_time_ratio(doc, 800, &LayoutOptions::default());
    eprintln!("t(n) {t1:.4} s, t(8n) {tk:.4} s, ratio {ratio:.1}");
    assert!(ratio < 24.0, "8x the text took {ratio:.1}x the time ({t1:.4} s → {tk:.4} s): not linear");
}

#[test]
fn many_hidden_float_anchors_lay_out_in_linear_time() {
    // n square-wrapped shapes, each anchored in its own hidden run (bold alternating, so runs do not
    // merge), markup on: finding each anchor's formatting once rescanned the runs from the start.
    let doc = |n: usize| {
        let float = wordcraft_doc::para::Float {
            wrap: wordcraft_doc::para::Wrap::Square,
            h_rel: wordcraft_doc::para::Anchor::Column,
            v_rel: wordcraft_doc::para::Anchor::Paragraph,
            x: 0.0,
            y: 0.0,
            dist: 9.0,
            dist_top: 9.0,
            dist_bottom: 9.0,
            ..Default::default()
        };
        let shape = InlineObject::Shape {
            kind: wordcraft_doc::para::ShapeKind::Rectangle,
            w: 20.0,
            h: 20.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float,
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        let obj = wordcraft_doc::para::OBJ.to_string();
        let mut p = wordcraft_doc::Paragraph::with_text("Text ", Default::default());
        p.text.push_str(&obj.repeat(n));
        p.objects = vec![shape; n];
        p.runs = std::iter::once(wordcraft_doc::Run { len: 5, props: Default::default() })
            .chain((0..n).map(|i| wordcraft_doc::Run {
                len: obj.len(),
                props: wordcraft_doc::CharProps { hidden: Some(true), bold: Some(i % 2 == 0), ..Default::default() },
            }))
            .collect();
        let mut d = Document::from_text("");
        d.body = vec![wordcraft_doc::para_block(p)];
        d
    };
    // Correct first: hidden anchors place no shape, shown ones do.
    let shapes = |show_hidden: bool| {
        let l = layout(&doc(50), &mut LayoutCache::new(), &LayoutOptions { show_hidden, ..Default::default() });
        l.pages.iter().flat_map(|p| p.items.iter()).filter(|i| matches!(i, Placed::Shape { .. })).count()
    };
    assert_eq!((shapes(false), shapes(true)), (0, 50));
    let (t1, tk, ratio) = layout_time_ratio(doc, 10_000, &LayoutOptions::default());
    eprintln!("t(n) {t1:.4} s, t(8n) {tk:.4} s, ratio {ratio:.1}");
    assert!(ratio < 24.0, "8x the anchors took {ratio:.1}x the time ({t1:.4} s → {tk:.4} s): not linear");
}

#[test]
fn fragmented_hidden_text_lays_out_like_the_visible_text() {
    // Visible words with a hidden multi-byte char after every visible one, formatted in alternating
    // runs (bold on and off) so no two left-out runs are adjacent or merge. The plain paragraph has
    // the same visible runs without the hidden chars, so both lay out the same widths whatever font
    // the default font resolves to.
    let words = "Hyphenation wraps international words across narrow columns of text ".repeat(6);
    let mut d = Document::from_text("");
    d.settings.auto_hyphenation = true;
    let mut plain = Document::from_text("");
    plain.settings.auto_hyphenation = true;
    let mut text = String::new();
    let mut kept = Vec::new();
    for (i, c) in words.chars().enumerate() {
        kept.push(text.len());
        text.push(c);
        let at = text.len();
        text.push('é');
        let n = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
        let bold = wordcraft_doc::CharProps { bold: Some(i % 2 == 0), ..Default::default() };
        d.insert_text(&Pos::body(0, n), &c.to_string(), &bold).unwrap();
        let hidden = wordcraft_doc::CharProps { hidden: Some(true), bold: Some(i % 2 == 1), ..Default::default() };
        d.insert_text(&Pos::body(0, at), "é", &hidden).unwrap();
        let n = plain.para(StoryRef::Body, &Path::top(0)).unwrap().len();
        plain.insert_text(&Pos::body(0, n), &c.to_string(), &bold).unwrap();
    }
    assert_eq!(d.para(StoryRef::Body, &Path::top(0)).unwrap().text, text);
    assert_eq!(plain.para(StoryRef::Body, &Path::top(0)).unwrap().text, words);
    for doc in [&mut d, &mut plain] {
        doc.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.indent_right = Some(468.0 - 90.0)).unwrap();
    }
    // Line starts and hyphenation points, as paragraph offsets.
    let shape = |d: &Document| {
        let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
        let paras: Vec<_> = l
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .filter_map(|i| if let Placed::Lines { para, .. } = i { Some(para.clone()) } else { None })
            .collect();
        let para = &paras[0];
        let starts: Vec<usize> = para.lines.iter().map(|l| para.clusters.get(l.c0).map_or(usize::MAX, |c| c.start)).collect();
        let hyph: Vec<usize> = para.hyph_after.iter().map(|k| para.clusters[*k as usize].end).collect();
        (starts, hyph)
    };
    let (starts, hyph) = shape(&plain);
    assert!(starts.len() > 5 && !hyph.is_empty(), "{starts:?} {hyph:?}");
    // Mapped to the fragmented paragraph: a line starts after the previous visible char (with the
    // hidden one that follows it, which takes no room), a hyphen point is after its char.
    let want_starts: Vec<usize> = starts.iter().map(|s| if *s == 0 { 0 } else { kept[s - 1] + 1 }).collect();
    let want_hyph: Vec<usize> = hyph.iter().map(|e| kept[e - 1] + 1).collect();
    assert_eq!(shape(&d), (want_starts, want_hyph));
}

#[test]
fn automatic_hyphenation_sees_only_the_text_laid_out() {
    // Paragraph-text ends of the clusters after which a line may end with a hyphen.
    let hyph_ends = |d: &Document, opts: &LayoutOptions| {
        let l = layout(d, &mut LayoutCache::new(), opts);
        let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        para.hyph_after.iter().map(|k| para.clusters[*k as usize].end).collect::<Vec<_>>()
    };
    let doc = |text: &str| {
        let mut d = Document::from_text(text);
        d.settings.auto_hyphenation = true;
        d
    };
    // (text, hidden bytes, deleted bytes, the text as laid out without them).
    let cases = [
        // A hidden prefix: "table" must not break after its first letter as "acceptable" may.
        ("acceptable", 0..5, 0..0, "table"),
        // A hidden separator joins two words into one.
        ("foot ball", 4..5, 0..0, "football"),
        // Hidden text and a tracked deletion together.
        ("accXeptYable", 7..8, 3..4, "acceptable"),
    ];
    let hidden_opts = LayoutOptions { hide_deleted: true, ..Default::default() };
    for (text, hidden, deleted, visible) in cases {
        let mut d = doc(text);
        d.format_range(&Pos::body(0, hidden.start), &Pos::body(0, hidden.end), &|c| c.hidden = Some(true)).unwrap();
        if !deleted.is_empty() {
            d.format_range(&Pos::body(0, deleted.start), &Pos::body(0, deleted.end), &|c| c.del = Some(0)).unwrap();
        }
        // The same points as the visible text alone, at the matching paragraph offsets.
        let kept: Vec<usize> = (0..text.len()).filter(|i| !hidden.contains(i) && !deleted.contains(i)).collect();
        let want: Vec<usize> = hyph_ends(&doc(visible), &hidden_opts).iter().map(|e| kept[e - 1] + 1).collect();
        assert_eq!(hyph_ends(&d, &hidden_opts), want, "{text:?} as {visible:?}");
        // Hidden text shown and markup shown: the whole text is hyphenated as before.
        let all = LayoutOptions { show_hidden: true, ..Default::default() };
        assert_eq!(hyph_ends(&d, &all), hyph_ends(&doc(text), &all), "{text:?} shown");
    }
}

#[test]
fn page_break_char_starts_new_page() {
    let mut d = Document::from_text("one");
    d.insert_text(&Pos::body(0, 3), "\u{000C}", &Default::default()).unwrap();
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
}

#[test]
fn a_missing_roman_font_is_drawn_as_word_draws_it() {
    // Measured in Word: a font that isn't installed is drawn in Cambria when the font table calls
    // it roman, in Calibri otherwise.
    let face = |roman: bool| {
        let mut d = Document::from_text("Serif");
        if roman {
            d.settings.roman_fonts = vec!["No Such Brand Font".into()];
        }
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.runs[0].props.font = Some("No Such Brand Font".into());
        let l = lay(&d);
        let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!("no text") };
        let first = para.clusters.first().expect("a cluster");
        para.styles[first.style as usize].face
    };
    let cambria = wordcraft_fonts::word::resolve("Cambria", false, false).face;
    assert_eq!(face(true), cambria);
    assert_ne!(face(false), cambria);
}

#[test]
fn an_empty_section_mark_stays_on_a_full_page() {
    use wordcraft_doc::section::SectionProps;
    // Measured in Word: a page filled to the bottom, then an empty paragraph carrying a next-page
    // section break, makes no page for the mark; the next section starts on page 2. A plain empty
    // paragraph moves on as usual.
    let pages = |carries_section: bool| {
        let lines: Vec<String> = (1..=200).map(|i| format!("Line {i}")).collect();
        let d = Document::from_text(&lines.join("\n"));
        let one_page = lay(&d).pages[0].items.iter().filter(|i| matches!(i, Placed::Lines { .. })).count();
        let mut d = Document::from_text(&format!("{}\n\nNext", lines[..one_page].join("\n")));
        let mark = d.para_mut(StoryRef::Body, &Path::top(one_page)).unwrap();
        if carries_section {
            mark.section = Some(Box::new(SectionProps::default()));
        } else {
            d.para_mut(StoryRef::Body, &Path::top(one_page + 1)).unwrap().props.page_break_before = Some(true);
        }
        lay(&d).pages.len()
    };
    assert_eq!(pages(true), 2);
    assert_eq!(pages(false), 3);
}

#[test]
fn a_bookmark_after_a_page_break_takes_no_line() {
    // "Before", a page break, then a bookmark start (Word writes the next heading's TOC bookmark
    // there): the bookmark takes no room, so "After" starts at the top of page 2, as in Word.
    let top_of_after = |bookmark: bool| {
        let mut d = Document::from_text("Before\nAfter");
        d.insert_text(&Pos::body(0, 6), "\u{000C}", &Default::default()).unwrap();
        if bookmark {
            let mark = InlineObject::BookmarkStart { name: "_Toc1".into() };
            d.insert_object(&Pos::body(0, 7), mark, &Default::default()).unwrap();
        }
        let l = lay(&d);
        let page = l.pages.last().unwrap();
        let after = page.items.iter().find_map(|i| match i {
            Placed::Lines { path, y, .. } if path.0 == [1] => Some(*y),
            _ => None,
        });
        (l.pages.len(), after.map(|y| y - page.body.y))
    };
    assert_eq!(top_of_after(true), top_of_after(false));
}

#[test]
fn a_page_break_then_a_new_page_makes_one_page() {
    use wordcraft_doc::section::SectionProps;
    // "Before", a paragraph of only a page break, then "After" starting a page by itself: a page
    // break before, or a next-page section break carried by the break's own paragraph. Word's
    // output for a report: the break's mark stays on the old page, and "After" starts the next.
    let doc = |section_on_break: bool| {
        let mut d = Document::from_text("Before\n\u{000C}\nAfter");
        if section_on_break {
            d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().section = Some(Box::new(SectionProps::default()));
        } else {
            d.para_mut(StoryRef::Body, &Path::top(2)).unwrap().props.page_break_before = Some(true);
        }
        d
    };
    for section_on_break in [false, true] {
        let l = lay(&doc(section_on_break));
        let firsts: Vec<usize> = l.pages.iter().map(|p| p.first_block).collect();
        assert_eq!(l.pages.len(), 2, "section break on the page break: {section_on_break}; pages start at blocks {firsts:?}");
    }
}

#[test]
fn page_numbers_can_restart_at_zero() {
    // `w:pgNumType w:start="0"`: a cover page numbered 0, so the page after it reads 1.
    let mut d = Document::from_text("Cover\u{000C}Contents");
    d.last_section.page_num_start = Some(0);
    let numbers: Vec<u32> = lay(&d).pages.iter().map(|p| p.number).collect();
    assert_eq!(numbers, [0, 1]);
}

#[test]
fn center_and_right_alignment() {
    let mut d = Document::from_text("abc\nabc\nabc");
    d.format_paragraphs(&Pos::body(1, 0), &Pos::body(1, 0), &|p| p.align = Some(Align::Center)).unwrap();
    d.format_paragraphs(&Pos::body(2, 0), &Pos::body(2, 0), &|p| p.align = Some(Align::Right)).unwrap();
    let l = lay(&d);
    let x0 = l.caret(&Pos::body(0, 0)).unwrap().x;
    let x1 = l.caret(&Pos::body(1, 0)).unwrap().x;
    let x2 = l.caret(&Pos::body(2, 3)).unwrap().x;
    assert!((x0 - 72.0).abs() < 0.5);
    assert!(x1 > 250.0 && x1 < 320.0, "{x1}");
    assert!((x2 - 540.0).abs() < 1.0, "{x2}");
}

#[test]
fn tabs_land_on_default_stops() {
    let d = Document::from_text("a\tb");
    let l = lay(&d);
    let xb = l.caret(&Pos::body(0, 2)).unwrap().x;
    assert!((xb - (72.0 + 36.0)).abs() < 0.5, "{xb}");
}

/// Left of the margin Word's next default tab stop is the first multiple of the interval at or
/// after the position (its arithmetic truncates toward zero), so a tab already on a stop stays
/// put. Measured on Word: "a\t\tX" at -60 pt puts X at -36 pt (not 0), "\tX" starting at -72 pt
/// leaves X at -72, at -60 moves it to -36; right of the margin the stops are as usual.
/// Letterheads that line up a footer with tabs under a negative indent rely on it.
#[test]
fn tabs_left_of_the_margin_stay_on_a_default_stop() {
    let x_of = |text: &str, indent: f32, off: usize| {
        let mut d = Document::from_text(text);
        d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.indent_left = Some(indent)).unwrap();
        lay(&d).caret(&Pos::body(0, off)).unwrap().x - 72.0
    };
    assert!((x_of("a\t\tX", -60.0, 3) - -36.0).abs() < 0.5, "second tab from a stop left of the margin: {}", x_of("a\t\tX", -60.0, 3));
    assert!((x_of("\tX", -72.0, 1) - -72.0).abs() < 0.5, "leading tab on a stop: {}", x_of("\tX", -72.0, 1));
    assert!((x_of("\tX", -60.0, 1) - -36.0).abs() < 0.5, "leading tab between stops: {}", x_of("\tX", -60.0, 1));
    assert!((x_of("\t \tX", -60.0, 3) - 0.0).abs() < 0.5, "a space between the tabs: {}", x_of("\t \tX", -60.0, 3));
    assert!((x_of("a\t\tX", 0.0, 3) - 72.0).abs() < 0.5, "right of the margin: {}", x_of("a\t\tX", 0.0, 3));
}

#[test]
fn right_tab_aligns_text_end() {
    let mut d = Document::from_text("left\tright");
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| {
        p.tabs = Some(vec![wordcraft_doc::props::TabStop { pos: 400.0, align: wordcraft_doc::props::TabAlign::Right, leader: Default::default() }])
    })
    .unwrap();
    let l = lay(&d);
    let end = l.caret(&Pos::body(0, 10)).unwrap().x;
    assert!((end - 472.0).abs() < 1.0, "{end}");
}

#[test]
fn lists_get_labels_and_indent() {
    let mut d = Document::from_text("one\ntwo\nthree");
    let id = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(2, 5), &|p| p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 })).unwrap();
    let l = lay(&d);
    let labels: Vec<String> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| if let Placed::Lines { para, .. } = it { para.label.as_ref().map(|x| x.text.clone()) } else { None })
        .collect();
    assert_eq!(labels, vec!["1.", "2.", "3."]);
    let x = l.caret(&Pos::body(0, 0)).unwrap().x;
    assert!((x - 108.0).abs() < 0.5, "{x}");
}

#[test]
fn break_only_list_has_no_label() {
    use wordcraft_doc::ListKind;
    for kind in [ListKind::Numbered, ListKind::Bullet] {
        let mut d = Document::from_text("First\n\u{000C}\nLast\n\nAfter");
        let id = d.numbering.add_list(kind);
        d.format_paragraphs(&Pos::body(0, 0), &Pos::body(4, 5), &|p| {
            p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 });
        })
        .unwrap();
        let l = lay(&d);
        let labels: Vec<_> = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| {
                let Placed::Lines { path, para, .. } = it else { return None };
                Some((path.clone(), para.label.as_ref().map(|label| label.text.clone())))
            })
            .collect();
        let label = |n| Some(if kind == ListKind::Numbered { format!("{n}.") } else { "•".to_string() });
        // Word hides the break's label but counts it; a plain empty item still has a label.
        assert_eq!(
            labels,
            vec![(Path::top(0), label(1)), (Path::top(1), None), (Path::top(2), label(3)), (Path::top(3), label(4)), (Path::top(4), label(5)),],
            "{kind:?}"
        );
    }
}

#[test]
fn a_numbered_heading_opening_with_a_page_break_takes_its_number_along() {
    // "Before", then a numbered paragraph whose text opens with a page break: Word draws its
    // number on the next page with the text, not alone at the bottom of the first.
    let mut d = Document::from_text("Before\n\u{000C}Heading");
    let id = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    d.format_paragraphs(&Pos::body(1, 0), &Pos::body(1, 1), &|p| p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 })).unwrap();
    let l = lay(&d);
    let label_pages: Vec<usize> = l
        .pages
        .iter()
        .enumerate()
        .filter(|(_, pg)| {
            flat(display::page_display(&d, pg, &Default::default())).iter().any(|x| matches!(x, display::Draw::Glyphs { text, .. } if text == "1."))
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(label_pages, [1]);
}

#[test]
fn russian_letter_list_labels_draw_cyrillic() {
    // russianLower labelled items 1, 2, 3 instead of а), б), в).
    use wordcraft_doc::section::NumFormat;
    let mut d = Document::from_text("один\nдва\nтри");
    let id = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    for a in &mut d.numbering.abstracts {
        if let Some(l) = a.levels.first_mut() {
            l.format = NumFormat::RussianLower;
            l.text = "%1)".into();
        }
    }
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(2, 3), &|p| p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 })).unwrap();
    let l = lay(&d);
    let labels: Vec<(String, bool)> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None })
        .filter_map(|pl| {
            let lab = pl.label.as_ref()?;
            let drawn = lab.runs.iter().all(|r| {
                let face = &pl.styles[r.style as usize].face;
                let gids: Vec<u32> = lab.glyphs[r.glyphs.clone()].iter().map(|g| g.gid).collect();
                r.text.chars().all(|c| gids.contains(&face.glyph_for(c)) && face.glyph_for(c) != 0)
            });
            Some((lab.text.clone(), drawn))
        })
        .collect();
    assert_eq!(labels, [("а)".to_string(), true), ("б)".to_string(), true), ("в)".to_string(), true)]);
}

#[test]
fn chinese_counting_list_labels_and_their_font_runs() {
    // Issue #534: a "一、" list showed digits, drawn with glyph ids of another font.
    use wordcraft_doc::section::NumFormat;
    let mut d = Document::from_text("甲\n乙\n丙");
    let id = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    for a in &mut d.numbering.abstracts {
        if let Some(l) = a.levels.first_mut() {
            l.format = NumFormat::ChineseCounting;
            l.text = "%1、".into();
        }
    }
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(2, 3), &|p| p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 })).unwrap();
    let labels = |d: &Document| -> Vec<(String, Vec<(String, bool)>)> {
        let l = lay(d);
        l.pages[0]
            .items
            .iter()
            .filter_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None })
            .filter_map(|pl| {
                let lab = pl.label.clone()?;
                // Every run draws glyphs of its own face.
                let runs = lab
                    .runs
                    .iter()
                    .map(|r| {
                        let face = &pl.styles[r.style as usize].face;
                        let gids: Vec<u32> = lab.glyphs[r.glyphs.clone()].iter().map(|g| g.gid).collect();
                        (r.text.clone(), r.text.chars().all(|c| gids.contains(&face.glyph_for(c))))
                    })
                    .collect();
                Some((lab.text, runs))
            })
            .collect()
    };
    let got = labels(&d);
    assert_eq!(got.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["一、", "二、", "三、"]);
    // A decimal label next to a CJK comma mixes faces when the Latin face lacks the comma.
    for a in &mut d.numbering.abstracts {
        if let Some(l) = a.levels.first_mut() {
            l.format = NumFormat::Decimal;
        }
    }
    for (text, runs) in labels(&d).into_iter().chain(got) {
        assert_eq!(runs.iter().map(|(t, _)| t.as_str()).collect::<String>(), text);
        assert!(runs.iter().all(|(_, own)| *own), "{text}: {runs:?}");
    }
}

#[test]
fn symbol_font_bullet_without_the_font_draws_a_bullet() {
    // Word's default bullet: U+F0B7 in Symbol.
    let mut d = Document::from_text("one");
    let id = d.numbering.add_list(wordcraft_doc::ListKind::BulletChar('\u{F0B7}'));
    for a in &mut d.numbering.abstracts {
        if let Some(l) = a.levels.first_mut() {
            l.chr.font = Some("Symbol".into());
        }
    }
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 })).unwrap();
    let l = lay(&d);
    let label = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { para.label.clone() } else { None }).unwrap();
    // The label reads as the Unicode bullet (drawn from a real Symbol font where one is installed).
    assert_eq!(label.text, "\u{2022}");
    assert_eq!(label.glyphs.len(), 1);
}

#[test]
fn wingdings_text_draws_and_reads_as_its_unicode_symbols() {
    // Word stores symbol-font text both ways: plain codes ("J" = smiley) and private-use codes
    // (U+F0FC = check mark, what `w:sym` writes).
    let mut d = Document::from_text("Hi J\u{F0FC} ok");
    d.format_range(&Pos::body(0, 3), &Pos::body(0, 7), &|c| c.font = Some("Wingdings".into())).unwrap();
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    let text: String = items.iter().filter_map(|i| if let display::Draw::Glyphs { text, .. } = i { Some(text.as_str()) } else { None }).collect();
    assert_eq!(text, "Hi \u{263A}\u{2713} ok");
    // The stored text is unchanged.
    assert_eq!(d.plain_text(StoryRef::Body), "Hi J\u{F0FC} ok");
    // Without Wingdings, each symbol is drawn from a face that has its Unicode character (never
    // the substitute's "J" or a missing glyph).
    if wordcraft_fonts::word::resolve("Wingdings", false, false).substituted {
        for want in ['\u{263A}', '\u{2713}'] {
            let run = items.iter().find_map(|i| match i {
                display::Draw::Glyphs { face, glyphs, text, .. } if text.contains(want) => Some((*face, glyphs.clone())),
                _ => None,
            });
            let Some((face, glyphs)) = run else { panic!("no run for {want}") };
            if face.covers(want) {
                assert!(glyphs.iter().any(|g| g.0 == face.glyph_for(want)), "{want}");
            } else {
                // No installed face has it (a bare CI machine): never the letter J's glyph.
                assert!(!glyphs.iter().any(|g| g.0 != 0 && g.0 == face.glyph_for('J')), "{want}");
            }
        }
    }
}

#[test]
fn tables_lay_out_cells() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(2, 3, 468.0);
    t.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("cell text", Default::default()))];
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let pos = Pos { story: StoryRef::Body, path: Path(vec![1, 0, 1, 0]), off: 0 };
    let c = l.caret(&pos).unwrap();
    assert!(c.x > 72.0 + 150.0 && c.x < 72.0 + 170.0, "{c:?}");
    let after = l.caret(&Pos::body(2, 0)).unwrap();
    assert!(after.top > c.top + 20.0);
    let rules = l.pages[0].items.iter().filter(|i| matches!(i, Placed::Rule { .. })).count();
    assert!(rules >= 12, "rules {rules}");
    assert!(l.cell_at(0, c.x, c.top + 2.0).is_some());
}

/// A vertical merge is one cell: its box spans the rows, a click anywhere in it finds it, and the
/// continuation row's box is `covered` (no gridline, not a hit).
#[test]
fn a_vertical_merge_lays_out_as_one_cell() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(3, 2, 468.0);
    t.rows[0].cells[0].props.vmerge = wordcraft_doc::props::VMerge::Restart;
    t.rows[1].cells[0].props.vmerge = wordcraft_doc::props::VMerge::Continue;
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let cells: Vec<_> = l.pages[0]
        .items
        .iter()
        .filter_map(|i| if let Placed::Cell { rect, row, cell: 0, covered, .. } = i { Some((*row, *rect, *covered)) } else { None })
        .collect();
    let (r0, r1) = (cells[0].1, cells[1].1);
    assert!(!cells[0].2 && cells[1].2 && !cells[2].2, "{cells:?}");
    assert!((r0.bottom() - r1.bottom()).abs() < 0.01 && r0.h > r1.h * 1.9, "{r0:?} {r1:?}");
    // The lower half of the merge hits the restart cell (row 0), not the continuation row.
    let hit = l.cell_at(0, r1.x + 5.0, r1.y + r1.h / 2.0).unwrap();
    assert_eq!((hit.2, hit.3), (0, 0));
}

/// A 1×3 table after "before" whose middle cell holds `text` running `dir`.
fn turned_table(dir: TextDirection, text: &str, row: Option<f32>) -> Document {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(1, 3, 468.0);
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("plain", Default::default()))];
    t.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))];
    t.rows[0].cells[1].props.text_direction = dir;
    if let Some(h) = row {
        t.rows[0].props.height = Some(h);
        t.rows[0].props.height_rule = wordcraft_doc::props::HeightRule::Exact;
    }
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    d
}

fn turned_lines(l: &DocLayout) -> &Placed {
    l.pages[0].items.iter().find(|it| matches!(it, Placed::Lines { path, .. } if path.0 == [1, 0, 1, 0])).unwrap()
}

/// Table Layout › Text Direction (#226): turned cell text runs along the cell's height, the row
/// grows to fit it, and caret, clicks and drawing follow the turn.
#[test]
fn turned_cell_text_runs_down_the_cell() {
    let text = "Turned cell text";
    let d = turned_table(TextDirection::Down, text, None);
    let l = lay(&d);
    let it = turned_lines(&l);
    assert!(matches!(it, Placed::Lines { turn: TextDirection::Down, l0: 0, l1: 1, .. }), "one unwrapped line: {it:?}");
    let b = it.turned_bounds().unwrap();
    // Tall and narrow, against the cell's right edge (cell 2 spans x 228..384, 5.4pt margins).
    assert!(b.h > 60.0 && b.w < 20.0, "{b:?}");
    assert!((b.right() - (72.0 + 312.0 - 5.4)).abs() < 1.0 && b.x > 72.0 + 156.0, "{b:?}");
    // The row grew to hold the text: the next paragraph starts below it.
    let after = l.caret(&Pos::body(2, 0)).unwrap();
    assert!(after.top > b.bottom(), "{after:?} vs {b:?}");
    let cell = l.pages[0].items.iter().find_map(|i| if let Placed::Cell { rect, cell: 1, .. } = i { Some(*rect) } else { None }).unwrap();
    assert!(cell.h >= b.h, "{cell:?}");
    // The caret lies across the page and moves down as the text goes on.
    let pos = |off| Pos { story: StoryRef::Body, path: Path(vec![1, 0, 1, 0]), off };
    let c0 = l.caret(&pos(0)).unwrap();
    let c1 = l.caret(&pos(text.len())).unwrap();
    assert!(c0.width > 5.0 && c0.height == 0.0, "{c0:?}");
    assert!((c0.top - b.y).abs() < 1.0 && c1.top > c0.top + 50.0, "{c0:?} {c1:?}");
    assert!(c0.x >= b.x - 0.5 && c0.x + c0.width <= b.right() + 0.5, "{c0:?} {b:?}");
    // A click on the turned text lands in it, by how far down the click is.
    let hit = l.hit(0, b.x + b.w / 2.0, b.y + b.h * 0.6, StoryRef::Body).unwrap();
    assert_eq!(hit.path, Path(vec![1, 0, 1, 0]));
    assert!(hit.off > 3 && hit.off < text.len(), "{hit:?}");
    // Selection highlights are turned too: tall, inside the text's area.
    let sel = l.selection_rects(&d, &pos(0), &pos(text.len()), 0);
    assert!(sel.iter().all(|(_, r)| r.h > r.w && r.x >= b.x - 1.0 && r.right() <= b.right() + 1.0), "{sel:?}");
    // Drawn in a turned frame.
    let draws = display::page_display(&d, &l.pages[0], &Default::default());
    let turned = draws.iter().find_map(|x| if let display::Draw::Turned { turn, items, .. } = x { Some((*turn, items)) } else { None });
    let (turn, items) = turned.expect("turned draw");
    assert_eq!(turn, TextDirection::Down);
    assert!(items.iter().any(|x| matches!(x, display::Draw::Glyphs { text, .. } if text.contains("Turned"))), "{items:?}");
}

#[test]
fn turned_up_cell_text_reads_bottom_to_top() {
    let text = "Bottom to top";
    let d = turned_table(TextDirection::Up, text, None);
    let l = lay(&d);
    let b = turned_lines(&l).turned_bounds().unwrap();
    // Against the cell's left edge; the text starts at the bottom and climbs.
    assert!((b.x - (72.0 + 156.0 + 5.4)).abs() < 1.0, "{b:?}");
    let pos = |off| Pos { story: StoryRef::Body, path: Path(vec![1, 0, 1, 0]), off };
    let (c0, c1) = (l.caret(&pos(0)).unwrap(), l.caret(&pos(text.len())).unwrap());
    assert!((c0.top - b.bottom()).abs() < 1.0 && c1.top < c0.top - 50.0, "{c0:?} {c1:?} {b:?}");
    let hit = l.hit(0, b.x + b.w / 2.0, b.bottom() - 2.0, StoryRef::Body).unwrap();
    assert_eq!((hit.path, hit.off), (Path(vec![1, 0, 1, 0]), 0));
}

#[test]
fn turned_text_wraps_in_an_exact_row() {
    let d = turned_table(TextDirection::Down, &"word ".repeat(30), Some(72.0));
    let l = lay(&d);
    let it = turned_lines(&l);
    let lines = if let Placed::Lines { l0, l1, .. } = it { l1 - l0 } else { 0 };
    assert!(lines > 2, "the text wraps at the row height");
    let b = it.turned_bounds().unwrap();
    assert!(b.h <= 72.0 && b.w > 30.0, "{b:?}");
    let after = l.caret(&Pos::body(2, 0)).unwrap();
    assert!(after.top < b.y + 72.0 + 30.0, "the row keeps its exact height: {after:?}");
}

#[test]
fn hostile_turned_cells_never_panic() {
    // Empty, huge and nested turned cells, in exact rows too small for anything.
    for dir in [TextDirection::Down, TextDirection::Up] {
        for row in [None, Some(0.5), Some(1e9)] {
            let mut d = turned_table(dir, "", row);
            let l = lay(&d);
            let _ = l.caret(&Pos { story: StoryRef::Body, path: Path(vec![1, 0, 1, 0]), off: 0 });
            let mut inner = Table::new(1, 1, 20.0);
            inner.rows[0].cells[0].props.text_direction = dir;
            if let Some(Block::Table(t)) = d.body.get_mut(1).map(Arc::make_mut) {
                t.rows[0].cells[1].blocks.insert(0, Arc::new(Block::Table(inner)));
            }
            let l = lay(&d);
            for p in &l.pages {
                let _ = display::page_display(&d, p, &Default::default());
            }
        }
    }
    let l = lay(&turned_table(TextDirection::Down, &"long ".repeat(5000), None));
    assert!(!l.pages.is_empty());
}

fn rules_on_page0(l: &DocLayout) -> Vec<f32> {
    l.pages[0].items.iter().filter_map(|i| if let Placed::Rule { border, .. } = i { Some(border.width) } else { None }).collect()
}

#[test]
fn table_style_borders_survive_empty_tbl_borders() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(2, 2, 468.0); // defaults to the TableGrid style
    t.props.borders = Some(Borders::default()); // what an empty <w:tblBorders/> parses to
    for r in &mut t.rows {
        for c in &mut r.cells {
            c.props.borders = Some(Borders::default());
        }
    }
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    assert!(rules_on_page0(&l).len() >= 8, "rules {:?}", rules_on_page0(&l));
}

/// Thick borders meet in closed corners: a horizontal edge starts at the outer side of the
/// vertical edge it meets, not at its centre line.
#[test]
fn table_border_corners_close() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(2, 2, 468.0);
    let thick = Some(Border::single(3.0));
    for r in &mut t.rows {
        for c in &mut r.cells {
            c.props.borders = Some(Borders { top: thick, left: thick, bottom: thick, right: thick, ..Borders::default() });
        }
    }
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let (mut h_left, mut v_left) = (f32::MAX, f32::MAX);
    for it in &l.pages[0].items {
        if let Placed::Rule { x0, y0, x1, y1, .. } = it {
            if y0 == y1 {
                h_left = h_left.min(x0.min(*x1));
            } else if x0 == x1 {
                v_left = v_left.min(*x0);
            }
        }
    }
    assert!((h_left - (v_left - 1.5)).abs() < 0.01, "horizontal starts at {h_left}, vertical centre line at {v_left}");
}

#[test]
fn table_own_border_overrides_only_that_side() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(2, 2, 468.0);
    t.props.borders = Some(Borders { top: Some(Border::single(2.0)), ..Default::default() });
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let w = rules_on_page0(&l);
    assert!(w.contains(&2.0), "own top missing: {w:?}");
    assert!(w.iter().filter(|x| (**x - 0.5).abs() < 1e-6).count() >= 6, "style sides missing: {w:?}");
}

#[test]
fn table_nil_border_hides_style_side() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(1, 1, 468.0); // 1×1: no inside edges
    t.props.borders = Some(Borders::all(Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 }));
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    assert!(rules_on_page0(&lay(&d)).is_empty());
}

#[test]
fn table_without_style_stays_borderless() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(2, 2, 468.0);
    t.props.style = None;
    t.props.borders = Some(Borders::default());
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    assert!(rules_on_page0(&lay(&d)).is_empty());
}

#[test]
fn headers_and_page_fields() {
    let mut d = Document::from_text(&"para\n".repeat(120));
    let mut hp = wordcraft_doc::Paragraph::new();
    hp.insert_object(
        0,
        InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &Default::default(),
    )
    .unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Footer, vec![wordcraft_doc::para_block(hp)]);
    d.last_section.footers.default = Some(id);
    let l = lay(&d);
    assert!(l.pages.len() >= 2);
    for (i, p) in l.pages.iter().enumerate() {
        assert_eq!(p.footer_story, Some(id));
        let label = p.footer.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clusters.len()) } else { None });
        assert!(label.is_some(), "page {i}");
    }
    let c = l.caret_on(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 }, 1).unwrap();
    assert_eq!(c.page, 1);
    assert!(c.top > 700.0);
}

#[test]
fn tall_first_page_header_pushes_body_down() {
    let mut d = Document::from_text(&"para\n".repeat(120));
    let tall: Vec<_> = (0..12).map(|_| wordcraft_doc::para_block(wordcraft_doc::Paragraph::new())).collect();
    let id = d.add_part(wordcraft_doc::PartKind::Header, tall);
    d.last_section.headers.first = Some(id);
    d.last_section.title_page = true;
    let l = lay(&d);
    let first = l.caret(&Pos::body(0, 0)).unwrap();
    assert!(first.top > d.last_section.margin_top + 50.0, "{first:?}");
    // The first paragraph on page 2 (which has no header) sits higher than page 1's first line,
    // whatever the installed fonts do to pagination.
    let later = (0..120).filter_map(|i| l.caret(&Pos::body(i, 0))).find(|c| c.page == 1).unwrap();
    assert!(later.top < first.top, "{later:?}");
}

#[test]
fn cache_reuses_unchanged_paragraphs() {
    let d = Document::from_text(&"some text here\n".repeat(50));
    let mut c = LayoutCache::new();
    layout(&d, &mut c, &LayoutOptions::default());
    let misses = c.misses;
    let mut d2 = d.clone();
    d2.insert_text(&Pos::body(10, 0), "x", &Default::default()).unwrap();
    layout(&d2, &mut c, &LayoutOptions::default());
    assert_eq!(c.misses, misses + 1);
}

/// Autofit measures every cell at an unbounded width; the cache keeps what it measured, not
/// those layouts, so a long table holds one layout per cell paragraph. Laying the document out
/// again after an edit elsewhere measures nothing again.
#[test]
fn cache_keeps_one_layout_per_table_cell() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(4, 3, 468.0);
    for (r, row) in t.rows.iter_mut().enumerate() {
        for (k, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("row {r} cell {k}"), Default::default()))];
        }
    }
    assert!(!t.props.fixed);
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let mut c = LayoutCache::new();
    layout(&d, &mut c, &LayoutOptions::default());
    assert_eq!(c.len(), 2 + 12);
    let misses = c.misses;
    d.insert_text(&Pos::body(0, 0), "x", &Default::default()).unwrap();
    layout(&d, &mut c, &LayoutOptions::default());
    assert_eq!(c.misses, misses + 1);
}

#[test]
fn selection_rects_cover_range() {
    let d = Document::from_text("Hello world\nSecond line");
    let l = lay(&d);
    let r = l.selection_rects(&d, &Pos::body(0, 6), &Pos::body(1, 6), 0);
    assert_eq!(r.len(), 2);
    assert!(r[0].1.w > 20.0);
}

#[test]
fn column_segments_take_the_same_x_range_on_every_line() {
    let d = Document::from_text("abcdef\nabcdef\nab");
    let l = lay(&d);
    let left = l.caret(&Pos::body(0, 2)).unwrap().x;
    let right = l.caret(&Pos::body(0, 4)).unwrap().x;
    let segs = l.column_segments(&d, &Pos::body(2, 2), &Pos::body(0, 2), right, left, 0, 100);
    let offs: Vec<(u32, usize, usize)> = segs.iter().map(|(a, b)| (a.path.0[0], a.off, b.off)).collect();
    assert_eq!(offs, vec![(0, 2, 4), (1, 2, 4), (2, 2, 2)]);
    // Capped, and junk x gives nothing.
    assert_eq!(l.column_segments(&d, &Pos::body(0, 2), &Pos::body(2, 2), left, right, 0, 2).len(), 2);
    assert!(l.column_segments(&d, &Pos::body(0, 2), &Pos::body(2, 2), f32::NAN, right, 0, 100).is_empty());
}

#[test]
fn display_has_glyphs_and_marks() {
    let mut d = Document::from_text("Hello\tworld");
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| c.underline = Some(wordcraft_doc::props::Underline::Single)).unwrap();
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &display::DisplayOptions { marks: true, ..Default::default() });
    assert!(items.iter().any(|i| matches!(i, display::Draw::Glyphs { .. })));
    assert!(items.iter().any(|i| matches!(i, display::Draw::Line { .. })));
    assert!(items.iter().any(|i| matches!(i, display::Draw::Mark { ch: '¶', .. })));
    assert!(items.iter().any(|i| matches!(i, display::Draw::Mark { ch: '→', .. })));
}

#[test]
fn a_tracked_paragraph_mark_shows_in_its_authors_colour() {
    let mut d = Document::from_text("Split\nhere");
    d.revisions.push(wordcraft_doc::Revision {
        kind: wordcraft_doc::RevisionKind::Insert,
        author: "Ana".into(),
        date: String::new(),
        move_name: None,
    });
    d.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().mark.ins = Some(0);
    let l = lay(&d);
    let marks = |markup: bool| -> Vec<Option<wordcraft_doc::props::Rgb>> {
        display::page_display(&d, &l.pages[0], &display::DisplayOptions { marks: true, markup, ..Default::default() })
            .into_iter()
            .filter_map(|i| if let display::Draw::Mark { ch: '¶', color, .. } = i { Some(color) } else { None })
            .collect()
    };
    assert_eq!(marks(true), [Some(display::revision_color(0)), None]);
    assert_eq!(marks(false), [None, None]);
}

#[test]
fn section_breaks_show_with_formatting_marks() {
    use wordcraft_doc::section::{SectionProps, SectionStart};
    // Two sections: "One" ends the first; the second starts as `start`.
    let doc = |start: SectionStart| {
        let mut d = Document::from_text("One\nTwo");
        d.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().section = Some(Box::new(SectionProps::default()));
        d.last_section.start = start;
        d
    };
    let labels = |d: &Document, marks: bool| -> (Vec<String>, usize) {
        let l = lay(d);
        let items = display::page_display(d, &l.pages[0], &display::DisplayOptions { marks, ..Default::default() });
        let texts = items.iter().filter_map(|i| if let display::Draw::MarkText { text, .. } = i { Some(text.clone()) } else { None }).collect();
        let pilcrows = items.iter().filter(|i| matches!(i, display::Draw::Mark { ch: '¶', .. })).count();
        (texts, pilcrows)
    };
    let d = doc(SectionStart::Continuous);
    // The break replaces the first paragraph's ¶; the last paragraph keeps its own.
    assert_eq!(labels(&d, true), (vec!["Section Break (Continuous)".to_string()], 1));
    assert_eq!(labels(&d, false), (vec![], 0));
    assert_eq!(labels(&doc(SectionStart::NextPage), true).0, ["Section Break (Next Page)"]);
    assert_eq!(labels(&doc(SectionStart::EvenPage), true).0, ["Section Break (Even Page)"]);
    assert_eq!(labels(&doc(SectionStart::OddPage), true).0, ["Section Break (Odd Page)"]);
    // A document with one section has no break.
    assert_eq!(labels(&Document::from_text("One\nTwo"), true), (vec![], 2));
}

#[test]
fn space_between_paragraphs_is_the_larger_of_after_and_before() {
    // (space after the first, space before the second, contextual spacing on both) -> gap
    let gap = |after: f32, before: f32, contextual: bool| -> f32 {
        let mut d = Document::from_text("One\nTwo");
        for (i, (a, b)) in [(after, 0.0), (0.0, before)].into_iter().enumerate() {
            let p = d.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(i)).unwrap();
            (p.props.space_after, p.props.space_before, p.props.contextual_spacing) = (Some(a), Some(b), Some(contextual));
        }
        let l = lay(&d);
        let ys: Vec<(f32, f32)> =
            l.pages[0].items.iter().filter_map(|i| if let Placed::Lines { y, para, .. } = i { Some((*y, para.height)) } else { None }).collect();
        ys[1].0 - (ys[0].0 + ys[0].1)
    };
    // Word's output for docxide-pdf's clean_energy_regulator_guidelines: 6pt after and 6pt before
    // leave 6pt, not 12pt.
    assert!((gap(6.0, 6.0, false) - 6.0).abs() < 0.01);
    assert!((gap(6.0, 12.0, false) - 12.0).abs() < 0.01);
    assert!((gap(12.0, 6.0, false) - 12.0).abs() < 0.01);
    assert!((gap(0.0, 6.0, false) - 6.0).abs() < 0.01);
    // Contextual spacing drops both between paragraphs of one style.
    assert!(gap(6.0, 12.0, true).abs() < 0.01);
}

#[test]
fn space_between_paragraphs_is_their_sum_when_html_auto_spacing_is_off() {
    // Measured in Word (compatibility modes 14 and 15): with `w:doNotUseHTMLParagraphAutoSpacing`,
    // 12 pt after and 12 pt before leave 24 pt; without it, 12 pt.
    let gaps = |sum: bool| -> (f32, f32) {
        let mut d = Document::from_text("One\nTwo");
        d.settings.sum_paragraph_spacing = sum;
        for i in 0..2 {
            let p = d.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
            (p.props.space_after, p.props.space_before) = (Some(12.0), Some(12.0));
        }
        let mut t = Table::new(1, 1, 468.0);
        t.rows[0].cells[0].blocks = d.body.clone();
        d.insert_block(StoryRef::Body, &Path::top(2), wordcraft_doc::Block::Table(t)).unwrap();
        let l = lay(&d);
        let tops = |cell: bool| -> Vec<(f32, f32)> {
            l.pages[0]
                .items
                .iter()
                .filter_map(|it| match it {
                    Placed::Lines { path, y, para, .. } if (path.0.len() == 4) == cell => Some((*y, para.height)),
                    _ => None,
                })
                .collect()
        };
        let gap = |ys: Vec<(f32, f32)>| ys[1].0 - (ys[0].0 + ys[0].1);
        (gap(tops(false)), gap(tops(true)))
    };
    let near = |(a, b): (f32, f32), want: f32| (a - want).abs() < 0.01 && (b - want).abs() < 0.01;
    assert!(near(gaps(true), 24.0), "{:?}", gaps(true));
    assert!(near(gaps(false), 12.0), "{:?}", gaps(false));
}

#[test]
fn space_between_paragraphs_in_a_cell_follows_the_body_rule() {
    // Three paragraphs in a cell, each 6pt before and 6pt after -> the gaps between them.
    let gaps = |contextual: bool| -> Vec<f32> {
        let mut d = Document::from_text("before\nafter");
        let mut t = Table::new(1, 1, 468.0);
        t.rows[0].cells[0].blocks = (0..3)
            .map(|k| {
                let mut p = wordcraft_doc::Paragraph::with_text(&format!("p{k}"), Default::default());
                (p.props.space_after, p.props.space_before, p.props.contextual_spacing) = (Some(6.0), Some(6.0), Some(contextual));
                wordcraft_doc::para_block(p)
            })
            .collect();
        d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
        let l = lay(&d);
        let mut ys: Vec<(u32, f32, f32)> = l.pages[0]
            .items
            .iter()
            .filter_map(|it| match it {
                Placed::Lines { path, y, para, .. } if path.0.len() == 4 => Some((path.0[3], *y, para.height)),
                _ => None,
            })
            .collect();
        ys.sort_by_key(|v| v.0);
        ys.windows(2).map(|w| w[1].1 - (w[0].1 + w[0].2)).collect()
    };
    let near = |got: Vec<f32>, want: [f32; 2]| got.len() == 2 && got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.01);
    // The larger of after and before, not their sum.
    assert!(near(gaps(false), [6.0, 6.0]), "{:?}", gaps(false));
    // Contextual spacing drops the space between every pair, not just the first.
    assert!(near(gaps(true), [0.0, 0.0]), "{:?}", gaps(true));
}

#[test]
fn sections_land_on_the_side_their_break_and_numbering_ask_for() {
    use wordcraft_doc::section::{SectionProps, SectionStart};
    // "One" ends the first section on page 1; the second section starts as `start`.
    let pages = |start: SectionStart, restart: Option<u32>, even_odd: bool| -> Vec<(usize, u32)> {
        let mut d = Document::from_text("One\nTwo");
        d.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().section = Some(Box::new(SectionProps::default()));
        d.last_section.start = start;
        d.last_section.page_num_start = restart;
        d.settings.even_odd_headers = even_odd;
        lay(&d).pages.iter().map(|p| (p.section, p.number)).collect()
    };
    // An odd-page break after page 1 leaves page 2 blank; the blank page ends the first section.
    assert_eq!(pages(SectionStart::OddPage, None, false), [(0, 1), (0, 2), (1, 3)]);
    assert_eq!(pages(SectionStart::EvenPage, None, false), [(0, 1), (1, 2)]);
    // Restarting at 1 needs no blank page unless both sides print; at 2 it takes the next number.
    assert_eq!(pages(SectionStart::OddPage, Some(1), false), [(0, 1), (1, 1)]);
    assert_eq!(pages(SectionStart::OddPage, Some(2), false), [(0, 1), (1, 3)]);
    assert_eq!(pages(SectionStart::OddPage, Some(1), true), [(0, 1), (0, 2), (1, 1)]);
    // With different odd and even headers, a next-page break restarting at 1 starts on an odd
    // sheet (Word's output for docxide-pdf's bush_fires_act_comparison).
    assert_eq!(pages(SectionStart::NextPage, Some(1), true), [(0, 1), (0, 2), (1, 1)]);
    assert_eq!(pages(SectionStart::NextPage, Some(2), true), [(0, 1), (1, 2)]);
    assert_eq!(pages(SectionStart::NextPage, Some(1), false), [(0, 1), (1, 1)]);
    assert_eq!(pages(SectionStart::NextPage, None, true), [(0, 1), (1, 2)]);
}

#[test]
fn paragraph_marks_sit_past_text_running_the_other_way() {
    // A left-to-right paragraph ending in Hebrew, and a right-to-left one ending in Latin: the
    // logically last letters are at the wrong edge, but the ¶ still goes past every glyph.
    let mut d = Document::from_text("Hello \u{5e9}\u{5dc}\u{5d5}\u{5dd}\n\u{5e9}\u{5dc}\u{5d5}\u{5dd} abc");
    let at = Pos::body(1, 0);
    d.format_paragraphs(&at, &at, &|p: &mut ParaProps| p.bidi = Some(true)).unwrap();
    let l = lay(&d);
    // Leftmost and rightmost glyph edges (page x) of each paragraph's single line.
    let extents: Vec<(f32, f32)> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| if let Placed::Lines { para, x, .. } = it { Some((para, *x)) } else { None })
        .map(|(pl, x)| {
            let ln = &pl.lines[0];
            let edges = (ln.c0..ln.c1).filter_map(|k| Some((ln.cl_left(k)?, ln.cl_right(k)?)));
            edges.fold((f32::MAX, f32::MIN), |(a, b), (l, r)| (a.min(x + l), b.max(x + r)))
        })
        .collect();
    let marks: Vec<(f32, f32)> = display::page_display(&d, &l.pages[0], &display::DisplayOptions { marks: true, ..Default::default() })
        .into_iter()
        .filter_map(|i| if let display::Draw::Mark { ch: '¶', x, size, .. } = i { Some((x, size)) } else { None })
        .collect();
    assert_eq!((extents.len(), marks.len()), (2, 2));
    let ((ltr_x, _), (rtl_x, size)) = (marks[0], marks[1]);
    assert!(ltr_x >= extents[0].1, "LTR ¶ at {ltr_x} should be right of all glyphs {:?}", extents[0]);
    assert!(rtl_x + size * 0.5 <= extents[1].0, "RTL ¶ at {rtl_x} should be left of all glyphs {:?}", extents[1]);
}

fn border_lines(d: &Document) -> Vec<(f32, f32, f32, f32)> {
    border_lines_in(d, &lay(d))
}

fn border_lines_in(d: &Document, l: &DocLayout) -> Vec<(f32, f32, f32, f32)> {
    display::page_display(d, &l.pages[0], &display::DisplayOptions::default())
        .into_iter()
        .filter_map(|i| if let display::Draw::Line { x0, y0, x1, y1, .. } = i { Some((x0, y0, x1, y1)) } else { None })
        .collect()
}

#[test]
fn character_border_draws_one_box_around_run() {
    use wordcraft_doc::props::Border;
    let mut d = Document::from_text("Hello world");
    assert!(border_lines(&d).is_empty());
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| c.border = Some(Border::single(0.5))).unwrap();
    // One layout for both the draw list and the expected geometry: the background font scan can change metrics between layouts.
    let l = lay(&d);
    let lines = border_lines_in(&d, &l);
    assert_eq!(lines.len(), 4, "{lines:?}");
    let Some(Placed::Lines { para, x, y, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
    let (gx0, gx1) = (x + para.x_of(0, 0).unwrap(), x + para.x_of(0, 5).unwrap());
    let (top, bottom) = (*y, y + para.lines[0].height);
    let horiz: Vec<_> = lines.iter().filter(|l| l.1 == l.3).collect();
    let vert: Vec<_> = lines.iter().filter(|l| l.0 == l.2).collect();
    assert_eq!(horiz.len(), 2);
    assert_eq!(vert.len(), 2);
    assert!(horiz.iter().any(|h| (h.1 - top).abs() < 0.01) && horiz.iter().any(|h| (h.1 - bottom).abs() < 0.01), "{lines:?}");
    assert!(horiz.iter().all(|h| (h.0 - gx0).abs() < 0.01 && (h.2 - gx1).abs() < 0.01), "{lines:?} {gx0} {gx1}");
}

#[test]
fn equal_adjacent_borders_share_a_box() {
    use wordcraft_doc::props::Border;
    let mut d = Document::from_text("ab");
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 2), &|c| c.border = Some(Border::single(0.5))).unwrap();
    d.format_range(&Pos::body(0, 1), &Pos::body(0, 2), &|c| c.bold = Some(true)).unwrap();
    assert_eq!(border_lines(&d).len(), 4);
    d.format_range(&Pos::body(0, 1), &Pos::body(0, 2), &|c| c.border = Some(Border::single(1.0))).unwrap();
    assert_eq!(border_lines(&d).len(), 8);
}

#[test]
fn wrapped_border_boxes_each_line() {
    use wordcraft_doc::props::Border;
    let text = "boxed ".repeat(20);
    let mut d = Document::from_text(text.trim_end());
    let n = d.para_at(&Pos::body(0, 0)).unwrap().len();
    d.format_range(&Pos::body(0, 0), &Pos::body(0, n), &|c| c.border = Some(Border::single(0.5))).unwrap();
    assert_eq!(lines_of(&lay(&d)), 2);
    assert_eq!(border_lines(&d).len(), 8);
}

#[test]
fn runs_differing_only_in_link_or_decoration_keep_their_own_style() {
    let mut d = Document::from_text("Alpha Beta Gamma Delta");
    let set = |d: &mut Document, a: usize, b: usize, f: &dyn Fn(&mut wordcraft_doc::props::CharProps)| {
        d.format_range(&Pos::body(0, a), &Pos::body(0, b), f).unwrap();
    };
    set(&mut d, 0, 5, &|c| c.link = Some("https://a.example/".into()));
    set(&mut d, 6, 10, &|c| c.link = Some("https://b.example/".into()));
    set(&mut d, 11, 16, &|c| c.strike = Some(true));
    set(&mut d, 17, 22, &|c| c.double_strike = Some(true));
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    let link_of = |word: &str| {
        items.iter().find_map(|i| if let display::Draw::Glyphs { text, link, .. } = i { text.contains(word).then(|| link.clone()) } else { None })
    };
    assert_eq!(link_of("Alpha"), Some(Some("https://a.example/".into())));
    assert_eq!(link_of("Beta"), Some(Some("https://b.example/".into())));
    let strokes: Vec<_> = items.iter().filter_map(|i| if let display::Draw::Line { stroke, .. } = i { Some(*stroke) } else { None }).collect();
    assert!(strokes.contains(&display::Stroke::Solid) && strokes.contains(&display::Stroke::Double), "{strokes:?}");
    // Underline colour is per run too.
    let mut d = Document::from_text("Red Blue");
    let red = wordcraft_doc::props::Rgb(0xFF, 0, 0);
    let blue = wordcraft_doc::props::Rgb(0, 0, 0xFF);
    for (a, b, c) in [(0, 3, red), (4, 8, blue)] {
        d.format_range(&Pos::body(0, a), &Pos::body(0, b), &|p| {
            p.underline = Some(wordcraft_doc::props::Underline::Single);
            p.underline_color = Some(c);
        })
        .unwrap();
    }
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    let colors: Vec<_> = items.iter().filter_map(|i| if let display::Draw::Line { color, .. } = i { Some(*color) } else { None }).collect();
    assert!(colors.contains(&red) && colors.contains(&blue), "{colors:?}");
}

#[test]
fn web_view_is_one_page() {
    let d = Document::from_text(&"text ".repeat(3000));
    let mut c = LayoutCache::new();
    let l = layout(&d, &mut c, &LayoutOptions { view: ViewMode::Web, web_width: 800.0, ..Default::default() });
    assert_eq!(l.pages.len(), 1);
    assert!(l.pages[0].h > 800.0);
}

#[test]
fn hostile_props_do_not_panic() {
    let mut d = Document::from_text("x\ny");
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(1, 0), &|p| {
        *p = ParaProps { indent_left: Some(1e9), indent_right: Some(1e9), indent_first: Some(-1e9), ..Default::default() }
    })
    .unwrap();
    d.last_section.page_w = 1.0;
    d.last_section.margin_left = 500.0;
    let l = lay(&d);
    assert!(!l.pages.is_empty());
}

#[test]
fn layout_is_fast() {
    let d =
        Document::from_text(&"Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore.\n".repeat(3000));
    let mut c = LayoutCache::new();
    let t0 = now_ms();
    let l = layout(&d, &mut c, &LayoutOptions::default());
    let cold = now_ms() - t0;
    let mut d2 = d.clone();
    d2.insert_text(&Pos::body(1500, 0), "x", &Default::default()).unwrap();
    let t1 = now_ms();
    let _ = layout(&d2, &mut c, &LayoutOptions::default());
    let warm = now_ms() - t1;
    eprintln!("3000 paragraphs, {} pages: cold {cold:.1} ms, warm {warm:.1} ms", l.pages.len());
    assert!(warm < cold);
}

#[test]
fn empty_center_tab_then_right_tab() {
    let mut d = Document::from_text("left\t\tright");
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.style = Some("Header".into())).unwrap();
    let l = lay(&d);
    let end = l.caret(&Pos::body(0, 11)).unwrap().x;
    assert!((end - (72.0 + 468.0)).abs() < 1.0, "{end}");
}

#[test]
fn footnotes_sit_at_page_bottom() {
    let mut d = Document::from_text(&"Body text line.\n".repeat(30));
    let mut note = wordcraft_doc::Paragraph::with_text("The note text.", Default::default()).styled("FootnoteText");
    note.props.space_after = Some(0.0);
    let id = d.add_part(wordcraft_doc::PartKind::Footnote, vec![wordcraft_doc::para_block(note)]);
    d.insert_object(
        &Pos::body(3, 4),
        InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: String::new() },
        &Default::default(),
    )
    .unwrap();
    let eid = d.add_part(
        wordcraft_doc::PartKind::Endnote,
        vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("An endnote.", Default::default()))],
    );
    d.insert_object(
        &Pos::body(5, 2),
        InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Endnote, id: eid, custom: String::new() },
        &Default::default(),
    )
    .unwrap();
    let l = lay(&d);
    let c = l.caret_on(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 }, 0).unwrap();
    assert_eq!(c.page, 0);
    assert!(c.top > 650.0 && c.top < 720.0, "{c:?}");
    let body_bottom = l.pages[0]
        .items
        .iter()
        .filter_map(|it| if let Placed::Lines { story: StoryRef::Body, y, para, l0, l1, .. } = it { item_bottom(*y, para, *l0, *l1) } else { None })
        .fold(0.0f32, f32::max);
    assert!(body_bottom < c.top, "{body_bottom} {}", c.top);
    assert!(l.caret_on(&Pos { story: StoryRef::Part(eid), path: Path::top(0), off: 0 }, 0).is_some());
}

#[test]
fn deleted_note_anchors_take_no_number_and_print_no_note() {
    use wordcraft_doc::para::NoteKind;
    let mut d = Document::from_text("Alpha Beta Gamma");
    let mut ids = Vec::new();
    // Footnotes after "Alpha" and "Beta", an endnote after "Gamma" (inserted back to front).
    for (off, kind, part) in [
        (16, NoteKind::Endnote, wordcraft_doc::PartKind::Endnote),
        (10, NoteKind::Footnote, wordcraft_doc::PartKind::Footnote),
        (5, NoteKind::Footnote, wordcraft_doc::PartKind::Footnote),
    ] {
        let id = d.add_part(part, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("A note.", Default::default()))]);
        d.insert_object(&Pos::body(0, off), InlineObject::NoteRef { kind, id, custom: String::new() }, &Default::default()).unwrap();
        ids.push(id);
    }
    let [end_id, second, first] = ids[..] else { panic!() };
    // Track-delete the first footnote's anchor and the endnote's anchor.
    let obj = wordcraft_doc::para::OBJ.len_utf8();
    d.format_range(&Pos::body(0, 5), &Pos::body(0, 5 + obj), &|c| c.del = Some(0)).unwrap();
    let end = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
    d.format_range(&Pos::body(0, end - obj), &Pos::body(0, end), &|c| c.del = Some(0)).unwrap();
    // The glyphs of each note mark in the body, and whether the endnote's text is placed.
    let marks = |hide_deleted: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { hide_deleted, ..Default::default() });
        let items: Vec<&Placed> = l.pages.iter().flat_map(|p| p.items.iter()).collect();
        let mut marks = HashMap::new();
        for it in &items {
            if let Placed::Lines { story: StoryRef::Body, para, .. } = it {
                for (ci, id) in &para.notes {
                    let c = &para.clusters[*ci];
                    marks.insert(*id, para.glyphs[c.g0 as usize..c.g1 as usize].iter().map(|g| g.gid).collect::<Vec<_>>());
                }
            }
        }
        let endnote = items.iter().any(|i| matches!(i, Placed::Lines { story: StoryRef::Part(p), .. } if *p == end_id));
        (marks, endnote)
    };
    // Markup: footnotes 1 and 2, and the endnote prints.
    let (full, endnote) = marks(false);
    assert!(full[&first] != full[&second] && endnote, "{full:?}");
    // Final text: the surviving footnote is number 1, and the deleted endnote anchor prints no note.
    let (fin, endnote) = marks(true);
    assert_eq!(fin.get(&second), full.get(&first), "the surviving footnote keeps number 2");
    assert!(!endnote, "the endnote of a deleted anchor is printed");
}

#[test]
fn hidden_note_anchors_keep_their_number_and_their_note() {
    // Measured in Word (hidden text not printed): a hidden footnote or endnote reference mark still
    // takes its number, so the visible footnote after it is 2, and both notes are still printed.
    use wordcraft_doc::para::NoteKind;
    let mut d = Document::from_text("Alpha Beta Gamma");
    let mut ids = Vec::new();
    for (off, kind, part) in [
        (16, NoteKind::Endnote, wordcraft_doc::PartKind::Endnote),
        (10, NoteKind::Footnote, wordcraft_doc::PartKind::Footnote),
        (5, NoteKind::Footnote, wordcraft_doc::PartKind::Footnote),
    ] {
        let id = d.add_part(part, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("A note.", Default::default()))]);
        d.insert_object(&Pos::body(0, off), InlineObject::NoteRef { kind, id, custom: String::new() }, &Default::default()).unwrap();
        ids.push(id);
    }
    let [end_id, second, first] = ids[..] else { panic!() };
    // Hide the first footnote's anchor and the endnote's anchor.
    let obj = wordcraft_doc::para::OBJ.len_utf8();
    d.format_range(&Pos::body(0, 5), &Pos::body(0, 5 + obj), &|c| c.hidden = Some(true)).unwrap();
    let end = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
    d.format_range(&Pos::body(0, end - obj), &Pos::body(0, end), &|c| c.hidden = Some(true)).unwrap();
    // The glyphs of each visible note mark in the body, and which notes' text is placed.
    let marks = |show_hidden: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden, ..Default::default() });
        let items: Vec<&Placed> = l.pages.iter().flat_map(|p| p.items.iter()).collect();
        let mut marks = HashMap::new();
        for it in &items {
            if let Placed::Lines { story: StoryRef::Body, para, .. } = it {
                for (ci, id) in &para.notes {
                    let c = &para.clusters[*ci];
                    marks.insert(*id, para.glyphs[c.g0 as usize..c.g1 as usize].iter().map(|g| g.gid).collect::<Vec<_>>());
                }
            }
        }
        let printed = |id: u32| items.iter().any(|i| matches!(i, Placed::Lines { story: StoryRef::Part(p), .. } if *p == id));
        (marks, [first, second, end_id].map(printed))
    };
    let (shown, printed) = marks(true);
    assert!(shown[&first] != shown[&second] && printed == [true; 3], "{shown:?} {printed:?}");
    // Not shown: the visible footnote is still number 2, and every note is printed.
    let (fin, printed) = marks(false);
    assert_eq!(fin.get(&second), shown.get(&second), "the visible footnote is renumbered");
    assert_eq!(printed, [true; 3], "a note whose reference is hidden is not printed");
}

#[test]
fn text_wraps_around_square_float() {
    let mut d = Document::from_text(&"Words flow around the picture here. ".repeat(30));
    let float = wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::Square,
        h_rel: wordcraft_doc::para::Anchor::Column,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        x: 0.0,
        y: 0.0,
        dist: 9.0,
        ..Default::default()
    };
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        w: 144.0,
        h: 100.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let l = lay(&d);
    let Placed::Lines { para, x, .. } = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })).unwrap() else { panic!() };
    let first = &para.lines[0];
    assert!(first.left >= 144.0, "first line starts beside the float: {}", first.left);
    let later = para.lines.iter().find(|ln| ln.top > 120.0).unwrap();
    assert!(later.left < 1.0, "lines below the float use the full width: {}", later.left);
    assert!(x + first.left > 72.0 + 144.0);
    assert!(l.pages[0].items.iter().any(|i| matches!(i, Placed::Shape { .. })));
}

#[test]
fn freeform_arrow_end_draws_at_the_last_point() {
    use std::sync::Arc;
    use wordcraft_doc::connector::ShapeExtra;
    use wordcraft_doc::freeform::{FreePath, Freeform};
    use wordcraft_doc::graphic::PathSeg;
    use wordcraft_doc::para::{Float, ShapeKind};
    use wordcraft_doc::props::Rgb;

    let shape = InlineObject::Shape {
        kind: ShapeKind::Freeform,
        w: 60.0,
        h: 40.0,
        fill: None,
        stroke: Some(Rgb::BLACK),
        stroke_width: 1.0,
        float: Float::default(),
        story: None,
        freeform: Some(Arc::new(Freeform {
            w: 60.0,
            h: 40.0,
            paths: vec![FreePath { pts: vec![[0.0, 40.0], [60.0, 0.0]], closed: false }],
            ..Default::default()
        })),
        effects: Default::default(),
        extra: ShapeExtra { arrow_end: true, ..Default::default() },
    };
    let mut doc = Document::from_text("Scan");
    doc.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();

    let draws = flat(display::page_display(&doc, &lay(&doc).pages[0], &Default::default()));
    let endpoint = draws.iter().find_map(|draw| match draw {
        display::Draw::Ink { pts, color: Rgb::BLACK, .. } => pts.last().copied(),
        _ => None,
    });
    let arrow_tip = draws.iter().find_map(|draw| match draw {
        display::Draw::Path { fill: Some(Rgb::BLACK), segs, .. } if segs.len() == 4 => match segs.first() {
            Some(PathSeg::Move(x, y)) => Some((*x, *y)),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(arrow_tip, endpoint, "the arrowhead tip meets the custom path's final point");
}

#[test]
fn hidden_drawings_are_neither_drawn_nor_take_room() {
    // A hidden floating shape (VML `visibility:hidden`, `wp:docPr/@hidden`): no wrap area, nothing
    // drawn, as in Word for Windows. A hidden inline one takes no room in its line.
    let mut d = Document::from_text(&"Words flow around the picture here. ".repeat(30));
    let float = wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::Square,
        h_rel: wordcraft_doc::para::Anchor::Column,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        hidden: true,
        ..Default::default()
    };
    let inline = wordcraft_doc::para::Float { hidden: true, ..Default::default() };
    d.insert_object(&Pos::body(0, 0), picture(200.0, 200.0, inline), &Default::default()).unwrap();
    d.insert_object(&Pos::body(0, 0), picture(144.0, 100.0, float), &Default::default()).unwrap();
    let l = lay(&d);
    let Placed::Lines { para, .. } = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })).unwrap() else { panic!() };
    let first = &para.lines[0];
    assert!(first.left < 1.0, "the first line uses the full width: {}", first.left);
    assert!(first.baseline - first.top < 20.0, "the hidden inline shape doesn't raise the line: {first:?}");
    assert!(!l.pages[0].items.iter().any(|i| matches!(i, Placed::Shape { .. } | Placed::Object { .. })));
    let draws = flat(display::page_display(&d, &l.pages[0], &Default::default()));
    assert!(!draws.iter().any(|x| matches!(x, display::Draw::Shape { .. })), "nothing drawn");
    // Showing hidden text doesn't show hidden drawings.
    let shown = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden: true, ..Default::default() });
    let Placed::Lines { para, .. } = shown.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })).unwrap() else { panic!() };
    assert!(para.lines[0].left < 1.0 && !shown.pages[0].items.iter().any(|i| matches!(i, Placed::Shape { .. } | Placed::Object { .. })));

    // A group draws its members but a hidden one.
    let mut d = Document::from_text("");
    let member = |hidden: bool, x: f32| wordcraft_doc::para::GroupChild {
        x,
        y: 0.0,
        obj: picture(50.0, 50.0, wordcraft_doc::para::Float { hidden, ..Default::default() }),
    };
    let group = InlineObject::Group {
        w: 100.0,
        h: 50.0,
        float: Default::default(),
        ch_w: 100.0,
        ch_h: 50.0,
        children: vec![member(true, 0.0), member(false, 50.0)],
        canvas: None,
    };
    d.insert_object(&Pos::body(0, 0), group, &Default::default()).unwrap();
    let l = lay(&d);
    let draws = flat(display::page_display(&d, &l.pages[0], &Default::default()));
    assert_eq!(draws.iter().filter(|x| matches!(x, display::Draw::Shape { .. })).count(), 1, "{draws:?}");
}

#[test]
fn deleted_float_leaves_no_wrap_area_without_markup() {
    // A square-wrapped shape anchored at the start of the paragraph, its anchor track-deleted.
    let text = "Words flow around the picture here. ".repeat(30);
    let mut d = Document::from_text(&text);
    let float = wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::Square,
        h_rel: wordcraft_doc::para::Anchor::Column,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        x: 0.0,
        y: 0.0,
        dist: 9.0,
        dist_top: 9.0,
        dist_bottom: 9.0,
        ..Default::default()
    };
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        w: 144.0,
        h: 100.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let obj = wordcraft_doc::para::OBJ.len_utf8();
    d.format_range(&Pos::body(0, 0), &Pos::body(0, obj), &|c| c.del = Some(0)).unwrap();
    // Where the first line starts, and whether the shape is placed.
    let first = |hide_deleted: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { hide_deleted, ..Default::default() });
        let p = &l.pages[0];
        let Some(Placed::Lines { para, .. }) = p.items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        (para.lines[0].left, p.items.iter().any(|i| matches!(i, Placed::Shape { .. })))
    };
    // With markup the deleted shape is shown and the text flows beside it.
    let (left, shown) = first(false);
    assert!(left >= 144.0 && shown, "{left} {shown}");
    // Final text: no shape, and no gap where it was.
    assert_eq!(first(true), (0.0, false));
}

#[test]
fn hidden_float_leaves_no_wrap_area_when_hidden_text_is_not_shown() {
    // A square-wrapped shape whose anchor is hidden text, through a character style (so only the
    // resolved formatting says so). Measured in Word: not printed, and the text runs full width.
    let text = "Words flow around the picture here. ".repeat(30);
    let mut d = Document::from_text(&text);
    d.styles.upsert(wordcraft_doc::Style {
        id: "Secret".into(),
        name: "Secret".into(),
        kind: wordcraft_doc::StyleKind::Character,
        chr: wordcraft_doc::CharProps { hidden: Some(true), ..Default::default() },
        ..Default::default()
    });
    let float = wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::Square,
        h_rel: wordcraft_doc::para::Anchor::Column,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        x: 0.0,
        y: 0.0,
        dist: 9.0,
        dist_top: 9.0,
        dist_bottom: 9.0,
        ..Default::default()
    };
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        w: 144.0,
        h: 100.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let obj = wordcraft_doc::para::OBJ.len_utf8();
    d.format_range(&Pos::body(0, 0), &Pos::body(0, obj), &|c| c.style = Some("Secret".into())).unwrap();
    // Where the first line starts, and whether the shape is placed.
    let first = |show_hidden: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden, ..Default::default() });
        let p = &l.pages[0];
        let Some(Placed::Lines { para, .. }) = p.items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        (para.lines[0].left, p.items.iter().any(|i| matches!(i, Placed::Shape { .. })))
    };
    // Hidden text shown: the shape is placed and the text flows beside it.
    let (left, shown) = first(true);
    assert!(left >= 144.0 && shown, "{left} {shown}");
    // Not shown: no shape, and no gap where it was.
    assert_eq!(first(false), (0.0, false));
}

/// A page-number start or a line-number start near `u32::MAX` (accepted from files and commands)
/// overflowed when the next page or line was counted: a panic with overflow checks on.
#[test]
fn extreme_page_and_line_number_starts_do_not_overflow() {
    let mut d = Document::from_text("one\ntwo\nthree");
    for b in d.body.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.page_break_before = Some(true);
        }
    }
    d.last_section.page_num_start = Some(u32::MAX);
    d.last_section.line_numbers = Some(wordcraft_doc::section::LineNumbering { start: u32::MAX, ..Default::default() });
    let l = lay(&d);
    assert!(l.pages.len() >= 3, "{} pages", l.pages.len());
}

#[test]
fn line_numbers_borders_text_boxes() {
    let mut d = Document::from_text("one\ntwo\nthree");
    d.last_section.line_numbers = Some(Default::default());
    d.last_section.page_borders = Some(wordcraft_doc::props::Borders::box_(wordcraft_doc::props::Border::single(1.0)));
    let id = d.add_part(
        wordcraft_doc::PartKind::TextBox,
        vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("inside the box", Default::default()))],
    );
    let tb = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 144.0,
        h: 72.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(2, 5), tb, &Default::default()).unwrap();
    let l = lay(&d);
    let p = &l.pages[0];
    let numbers = p.items.iter().filter(|i| matches!(i, Placed::Lines { story: StoryRef::Part(u32::MAX), .. })).count();
    assert_eq!(numbers, 3);
    assert!(p.items.iter().filter(|i| matches!(i, Placed::Rule { .. })).count() >= 4);
    assert!(l.caret(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 }).is_some());
}

#[test]
fn tall_rows_split_across_pages() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(2, 2, 468.0);
    let long = "Row text that keeps going and going so the cell grows taller than a page. ".repeat(120);
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&long, Default::default()))];
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t.clone())).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 2, "pages {}", l.pages.len());
    // The cell's lines start on page 1 (row split, not moved) and continue on page 2.
    let cell_lines = |p: &Page| p.items.iter().filter(|i| matches!(i, Placed::Lines { path, .. } if path.0 == vec![1, 0, 0, 0])).count();
    assert!(cell_lines(&l.pages[0]) > 0);
    assert!(cell_lines(&l.pages[1]) > 0);
    // Nothing runs past the bottom margin.
    for p in &l.pages[..l.pages.len() - 1] {
        for it in &p.items {
            if let Placed::Lines { y, para, l0, l1, .. } = it {
                let b = item_bottom(*y, para, *l0, *l1).unwrap_or(0.0);
                assert!(b <= 792.0 - 72.0 + 0.5, "line bottom {b}");
            }
        }
    }
    // Every line of the long paragraph is placed exactly once.
    let total: usize = l
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|i| if let Placed::Lines { path, l0, l1, .. } = i { (path.0 == vec![1, 0, 0, 0]).then_some(l1 - l0) } else { None })
        .sum();
    let n = l
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .find_map(|i| if let Placed::Lines { path, para, .. } = i { (path.0 == vec![1, 0, 0, 0]).then_some(para.lines.len()) } else { None })
        .unwrap_or(0);
    assert_eq!(total, n);
    // "Can't split" rows move whole instead.
    let mut d2 = Document::from_text("before\nafter");
    t.rows[0].props.cant_split = true;
    d2.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l2 = lay(&d2);
    assert!(l2.pages.len() >= 2);
}

/// A table row that starts `push` points lower on a page: one cell with a 4pt bottom margin
/// holding `paras` (text, space after) with no space before. Returns the layout and each cell
/// paragraph's lines per page.
fn row_at_page_bottom(push: f32, paras: &[(&str, f32)], widow: bool) -> (DocLayout, Vec<Vec<usize>>) {
    let mut d = Document::from_text("Filler");
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) {
        p.props.space_before = Some(push);
        p.props.space_after = Some(0.0);
    }
    let mut t = Table::new(1, 1, 200.0);
    t.props.cell_margins = Some([0.0, 5.4, 4.0, 5.4]);
    t.rows[0].cells[0].blocks = paras
        .iter()
        .map(|(text, after)| {
            let mut p = wordcraft_doc::Paragraph::with_text(text, Default::default());
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(*after);
            p.props.widow_control = Some(widow);
            wordcraft_doc::para_block(p)
        })
        .collect();
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let per_page = (0..paras.len())
        .map(|i| {
            l.pages
                .iter()
                .map(|pg| {
                    pg.items
                        .iter()
                        .filter_map(|it| match it {
                            Placed::Lines { path, l0, l1, .. } if path.0 == vec![1, 0, 0, i as u32] => Some(l1 - l0),
                            _ => None,
                        })
                        .sum()
                })
                .collect()
        })
        .collect();
    (l, per_page)
}

/// The bottom of the last line of cell paragraph `i` when the row starts at the top of the
/// page's body (no push).
fn cell_line_bottoms(paras: &[(&str, f32)], i: usize) -> Vec<f32> {
    let (l, _) = row_at_page_bottom(0.0, paras, true);
    let mut out = Vec::new();
    for it in &l.pages[0].items {
        if let Placed::Lines { path, para, l0, l1, y, .. } = it
            && path.0 == vec![1, 0, 0, i as u32]
        {
            for k in *l0..*l1 {
                out.push(item_bottom(*y, para, *l0, k + 1).unwrap_or(0.0));
            }
        }
    }
    out
}

/// Word splits a row whose text fits above the page bottom but whose bottom cell margin does
/// not: the last line moves on, and widow control takes the line before it along (an eight-line
/// cell paragraph breaks 6 + 2). It used to move the whole row to the next page.
#[test]
fn a_split_row_keeps_its_bottom_margin_and_widow_control() {
    let text = "Dotted leader lines to fill in by hand later on. ".repeat(14);
    let paras = [(text.as_str(), 0.0)];
    let bottoms = cell_line_bottoms(&paras, 0);
    let n = bottoms.len();
    assert!(n >= 6, "{n} lines");
    // Put the last line 2pt above the page bottom (720): the 4pt margin under it doesn't fit.
    let push = 720.0 - 2.0 - bottoms[n - 1];
    let (_, lines) = row_at_page_bottom(push, &paras, true);
    assert_eq!(lines[0], vec![n - 2, 2], "widow control: two lines move on");
    let (_, lines) = row_at_page_bottom(push, &paras, false);
    assert_eq!(lines[0], vec![n - 1, 1], "without widow control only the last line moves on");
    // With the margin room to spare, the whole row stays.
    let (_, lines) = row_at_page_bottom(push - 3.0, &paras, true);
    assert_eq!(lines[0], vec![n], "the row fits with its margin");
    // Only the first line fits: an orphan, so the paragraph moves on whole.
    let push = 720.0 - 4.0 - 1.0 - bottoms[0];
    let (l, lines) = row_at_page_bottom(push, &paras, true);
    assert_eq!(lines[0], vec![0, n], "orphan control moves the paragraph");
    assert!(l.pages[0].items.iter().all(|it| !matches!(it, Placed::Cell { .. })), "and the row with it");
}

/// The cell's last paragraph needs its space after on the page too: a row ending in an empty
/// paragraph with 8pt after, whose text fits but whose space after doesn't, splits before it
/// (Word keeps the "Remarks:" line on the page) instead of moving whole.
#[test]
fn a_split_row_keeps_the_last_paragraphs_space_after() {
    let paras = [("Remarks:", 0.0), ("", 8.0)];
    let bottom = cell_line_bottoms(&paras, 1).last().copied().unwrap_or(0.0);
    // The empty paragraph's line ends 6pt above the page bottom (720): 8pt after + 4pt margin
    // don't fit.
    let (_, lines) = row_at_page_bottom(720.0 - 6.0 - bottom, &paras, true);
    assert_eq!(lines, vec![vec![1, 0], vec![0, 1]], "split between the paragraphs");
}

/// Each cell of a split row goes on at the top of the continuation: widow control moves two
/// lines of one cell's paragraph on, a neighbour of one-line paragraphs only its last line, and
/// both start level on the next page (the neighbour used to start a line lower). The
/// continuation is as tall as the cell needing the most room.
#[test]
fn split_row_cells_each_continue_at_the_top() {
    let text = "Dotted leader lines to fill in by hand later on. ".repeat(14);
    let row = |push: f32, singles: usize| {
        let mut d = Document::from_text("Filler");
        if let Some(wordcraft_doc::Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) {
            p.props.space_before = Some(push);
            p.props.space_after = Some(0.0);
        }
        let mut t = Table::new(1, 2, 400.0);
        t.props.cell_margins = Some([0.0, 5.4, 4.0, 5.4]);
        let para = |s: &str| {
            let mut p = wordcraft_doc::Paragraph::with_text(s, Default::default());
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
            p.props.widow_control = Some(true);
            wordcraft_doc::para_block(p)
        };
        t.rows[0].cells[0].blocks = vec![para(&text)];
        t.rows[0].cells[1].blocks = (0..singles).map(|i| para(&format!("Item {i}"))).collect();
        d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
        lay(&d)
    };
    // The long paragraph's line bottoms with the row at the top of the page; the neighbour gets
    // as many one-line paragraphs.
    let l = row(0.0, 1);
    let Some((y, para, n)) = l.pages[0].items.iter().find_map(|it| match it {
        Placed::Lines { path, y, para, l1, .. } if path.0 == vec![1, 0, 0, 0] => Some((*y, para.clone(), *l1)),
        _ => None,
    }) else {
        panic!("no cell text")
    };
    assert!(n >= 6, "{n} lines");
    let bottoms: Vec<f32> = (1..=n).map(|k| item_bottom(y, &para, 0, k).unwrap()).collect();
    let line = bottoms[1] - bottoms[0];
    // The last lines end 2pt above the page bottom (720): the 4pt margin under them doesn't fit.
    let l = row(720.0 - 2.0 - bottoms[n - 1], n);
    assert_eq!(l.pages.len(), 2);
    let tops: Vec<(usize, u32, f32)> = l.pages[1]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { path, l1, l0, y, .. } if path.0.len() == 4 => Some((l1 - l0, path.0[2], *y)),
            _ => None,
        })
        .collect();
    assert_eq!(tops.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>(), vec![(2, 0), (1, 1)], "{tops:?}");
    assert!((tops[0].2 - tops[1].2).abs() < 0.01, "both cells go on at the top: {tops:?}");
    let cells: Vec<Rect> = l.pages[1].items.iter().filter_map(|it| if let Placed::Cell { rect, .. } = it { Some(*rect) } else { None }).collect();
    assert_eq!(cells.len(), 2);
    for c in &cells {
        assert!((c.y - (tops[0].2 - 2.0)).abs() < 0.01, "{cells:?} {tops:?}");
        // Two lines, the 2pt kept of the top margin and the 4pt bottom margin (and border band).
        assert!(c.h >= 2.0 * line + 6.0 - 0.05 && c.h <= 2.0 * line + 7.0, "{} vs {}", c.h, 2.0 * line + 6.0);
    }
}

/// Word never splits a row of an exact height: one the page bottom cuts through, its text
/// included, moves to the next page whole. An at-least row also moves when the remaining
/// room is less than its minimum height.
#[test]
fn exact_rows_move_on_whole() {
    let text = "Dotted leader lines to fill in by hand later on. ".repeat(14);
    let lay_row = |rule: wordcraft_doc::props::HeightRule| {
        let mut d = Document::from_text("Filler");
        if let Some(wordcraft_doc::Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) {
            p.props.space_before = Some(560.0);
            p.props.space_after = Some(0.0);
        }
        let mut t = Table::new(1, 1, 200.0);
        t.props.cell_margins = Some([0.0, 5.4, 4.0, 5.4]);
        t.rows[0].props.height = Some(120.0);
        t.rows[0].props.height_rule = rule;
        let mut p = wordcraft_doc::Paragraph::with_text(&text, Default::default());
        p.props.space_after = Some(0.0);
        t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(p)];
        d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
        let l = lay(&d);
        l.pages
            .iter()
            .map(|pg| {
                pg.items
                    .iter()
                    .filter_map(|it| if let Placed::Lines { path, l0, l1, .. } = it { (path.0.len() == 4).then_some(l1 - l0) } else { None })
                    .sum()
            })
            .collect::<Vec<usize>>()
    };
    let exact = lay_row(wordcraft_doc::props::HeightRule::Exact);
    assert_eq!(exact.len(), 2);
    assert_eq!(exact[0], 0, "the exact row moves on whole: {exact:?}");
    let at_least = lay_row(wordcraft_doc::props::HeightRule::AtLeast);
    assert_eq!(at_least[0], 0, "less than the minimum height remains: {at_least:?}");
    assert!(at_least.get(1).is_some_and(|n| *n > 0), "the at-least row moves: {at_least:?}");
}

/// A nested table's cell the page bottom cuts through goes on at the top of the next page with
/// what is left of it, and the paragraph after the nested table below it, while its neighbour's
/// text runs on too. The cell's shift used to come from that paragraph (below the page bottom),
/// which squashed the nested cell to 2pt on the next page.
#[test]
fn a_nested_cell_cut_by_the_page_bottom_keeps_its_rest() {
    let mut d = Document::from_text("Filler");
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) {
        p.props.space_before = Some(400.0);
        p.props.space_after = Some(0.0);
    }
    let mut inner = Table::new(1, 1, 150.0);
    inner.rows[0].props.height = Some(300.0);
    inner.rows[0].props.height_rule = wordcraft_doc::props::HeightRule::Exact;
    let mut t = Table::new(1, 2, 400.0);
    let text = "Dotted leader lines to fill in by hand later on. ".repeat(40);
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
    t.rows[0].cells[1].blocks =
        vec![std::sync::Arc::new(Block::Table(inner)), wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("After", Default::default()))];
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 2);
    let nested = |pg: usize| {
        l.pages[pg].items.iter().find_map(|it| if let Placed::Cell { rect, table, .. } = it { (table.0.len() > 1).then_some(*rect) } else { None })
    };
    let (Some(a), Some(b)) = (nested(0), nested(1)) else { panic!("the nested cell is on both pages") };
    assert!((a.bottom() - 720.0).abs() < 0.5, "cut at the page bottom: {a:?}");
    assert!(a.h + b.h >= 300.0 - 0.5 && a.h + b.h <= 300.0 + 2.5, "the nested cell keeps its height: {a:?} + {b:?}");
    let after = l.pages[1].items.iter().find_map(|it| match it {
        Placed::Lines { path, y, .. } if path.0 == vec![1, 0, 1, 1] => Some(*y),
        _ => None,
    });
    assert!(after.is_some_and(|y| y >= b.bottom() - 0.01), "the paragraph after it follows it: {after:?} {b:?}");
    // The neighbour's text goes on at the top too, every line once.
    let lines: usize = l
        .pages
        .iter()
        .flat_map(|pg| &pg.items)
        .map(|it| if let Placed::Lines { path, l0, l1, .. } = it { if path.0 == vec![1, 0, 0, 0] { l1 - l0 } else { 0 } } else { 0 })
        .sum();
    let total = l.pages[0].items.iter().find_map(|it| match it {
        Placed::Lines { path, para, .. } if path.0 == vec![1, 0, 0, 0] => Some(para.lines.len()),
        _ => None,
    });
    assert_eq!(Some(lines), total);
}

/// A row of a minimum ("at least") height taller than its text that splits across pages keeps
/// its height over them: the continuation holds the rest of the row, not just the rest of the
/// text (no public Word reference crosses a page with such a row; this keeps the earlier
/// behaviour).
#[test]
fn a_split_at_least_row_keeps_its_height() {
    let mut d = Document::from_text("Filler");
    let mut t = Table::new(1, 1, 400.0);
    t.rows[0].props.height = Some(1200.0);
    t.rows[0].props.height_rule = wordcraft_doc::props::HeightRule::AtLeast;
    let text = "Dotted leader lines to fill in by hand later on. ".repeat(80);
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let parts: Vec<f32> = l
        .pages
        .iter()
        .flat_map(|pg| &pg.items)
        .filter_map(|it| if let Placed::Cell { rect, table, .. } = it { (table.0.len() == 1).then_some(rect.h) } else { None })
        .collect();
    assert_eq!(parts.len(), 2, "{parts:?}");
    // 1200pt and the 0.5pt top and bottom border bands.
    let sum: f32 = parts.iter().sum();
    assert!((sum - 1201.0).abs() < 0.05, "{parts:?}");
}

/// A cut that is NaN or infinite, or a row of a NaN height, splits nothing (`clamp` used to panic
/// on a NaN cut).
#[test]
fn split_row_rejects_a_hostile_cut() {
    use crate::table::{CellPart, RowLayout, split_row};
    let d = Document::from_text("Line");
    let l = lay(&d);
    let Some(Placed::Lines { story, path, para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })).cloned() else {
        panic!("no line")
    };
    let line = |y: f32| Placed::Lines { story, path: path.clone(), para: para.clone(), l0: 0, l1: 1, x: 0.0, y, turn: TextDirection::Horizontal };
    let row = |height: f32, bottom: f32| RowLayout {
        height,
        items: vec![line(0.0), line(150.0)],
        bottom,
        cells: vec![CellPart { items: 0..2, end: Some(170.0) }],
    };
    for cut in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0, 200.0] {
        assert!(split_row(&row(200.0, 4.0), cut).is_none(), "{cut}");
    }
    assert!(split_row(&row(f32::NAN, 4.0), 100.0).is_none());
    assert!(split_row(&row(f32::INFINITY, 4.0), f32::INFINITY).is_none());
    // A NaN or infinite bottom margin doesn't panic either.
    for bottom in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let _ = split_row(&row(200.0, bottom), 100.0);
        let _ = split_row(&row(f32::INFINITY, bottom), 100.0);
    }
    // Cell ranges past the items are ignored.
    let mut r = row(200.0, 4.0);
    r.cells.push(CellPart { items: 5..9, end: None });
    r.cells.push(CellPart { items: std::ops::Range { start: 2, end: 1 }, end: None });
    assert!(split_row(&r, 100.0).is_some());
}

#[test]
fn repeated_header_is_followed_by_the_next_row_even_when_it_overflows() {
    // A one-column table at the top of a letter page (body 648 pt), then an empty paragraph. Returns
    // the table rows on each page, in order.
    let rows_per_page = |t: Table| -> Vec<Vec<usize>> {
        let mut d = Document::new();
        d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(t)).unwrap();
        let l = lay(&d);
        l.pages
            .iter()
            .map(|p| p.items.iter().filter_map(|i| if let Placed::Cell { row, cell: 0, .. } = i { Some(*row) } else { None }).collect())
            .collect()
    };
    // A header row of `header` points and body rows of `rows` points (at least), one line of text each.
    let table = |header: f32, rows: &[f32]| {
        let mut t = Table::new(rows.len() + 1, 1, 468.0);
        t.rows[0].props.header = true;
        for (row, h) in t.rows.iter_mut().zip(std::iter::once(&header).chain(rows)) {
            row.props.height = Some(*h);
            row.props.height_rule = wordcraft_doc::props::HeightRule::AtLeast;
        }
        t
    };
    // Header and row fit together: one row per page, under the header.
    assert_eq!(rows_per_page(table(100.0, &[400.0; 3])), vec![vec![0, 1], vec![0, 2], vec![0, 3]]);
    // A 300 pt header and 400 pt rows never fit together. As in Word, the first row moves to a new
    // page, goes under the repeated header and runs into the bottom margin; so does every later row.
    assert_eq!(rows_per_page(table(300.0, &[400.0; 4])), vec![vec![0], vec![0, 1], vec![0, 2], vec![0, 3], vec![0, 4], vec![]]);
    // A header that almost fills the page still repeats, with one row under it on each page.
    assert_eq!(rows_per_page(table(640.0, &[100.0; 3])), vec![vec![0], vec![0, 1], vec![0, 2], vec![0, 3], vec![]]);
    // A 20-row table under a 640 pt header: one page per row, never a runaway.
    assert_eq!(rows_per_page(table(640.0, &[12.0; 19])).len(), 21);
    // A row of text that is taller than the room under the header still splits across pages.
    let mut t = table(100.0, &[12.0]);
    let long = "Row text that keeps going and going so the cell grows taller than a page. ".repeat(120);
    t.rows[1].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&long, Default::default()))];
    let pages = rows_per_page(t);
    assert!(pages.len() >= 3, "{pages:?}");
    assert!(pages.iter().all(|p| p.first() == Some(&0) && p.contains(&1)), "{pages:?}");
    // An ordinary header still repeats on every page the table reaches.
    let mut t = Table::new(80, 2, 468.0);
    t.rows[0].props.header = true;
    let pages = rows_per_page(t);
    assert!(pages.len() >= 2, "{pages:?}");
    assert!(pages.iter().all(|p| p.first() == Some(&0)), "{pages:?}");
}

#[test]
fn repeated_header_moves_with_the_first_row_that_does_not_fit() {
    // A paragraph that leaves `room` points of the letter page's 648 pt body, then a table with a
    // 20 pt header row and 40 pt one-line rows. Returns the table rows on each page.
    let rows_per_page = |room: f32| -> Vec<Vec<usize>> {
        let mut d = Document::from_text("Text above the table");
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.space_after = Some(648.0 - room - 14.0);
        let mut t = Table::new(3, 1, 468.0);
        t.rows[0].props.header = true;
        for (row, h) in t.rows.iter_mut().zip([20.0, 40.0, 40.0]) {
            row.props.height = Some(h);
            row.props.height_rule = wordcraft_doc::props::HeightRule::AtLeast;
        }
        d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
        lay(&d)
            .pages
            .iter()
            .map(|p| p.items.iter().filter_map(|i| if let Placed::Cell { row, cell: 0, .. } = i { Some(*row) } else { None }).collect())
            .collect()
    };
    // Header and first row fit: the table starts on the first page.
    assert_eq!(rows_per_page(80.0)[0], vec![0, 1]);
    // Only the header fits: it goes to the next page with the first row instead of staying alone.
    assert_eq!(rows_per_page(40.0), vec![vec![], vec![0, 1, 2]]);
}

/// A drop-cap paragraph in a font larger than 1000 pt used to panic in layout (`clamp` with min > max).
#[test]
fn drop_cap_with_a_huge_font_lays_out() {
    for size in [999.0, 1000.0, 1001.0, 1638.0] {
        let cp = wordcraft_doc::CharProps { size: Some(size), ..Default::default() };
        let mut p = wordcraft_doc::Paragraph::with_text("Hello drop cap world", cp);
        p.props.drop_cap = Some(3);
        let mut d = Document::from_text("x");
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = lay(&d);
        let Some(pl) = l.pages[0].items.iter().find_map(|i| if let Placed::Lines { para, .. } = i { Some(para.clone()) } else { None }) else {
            panic!("no lines at size {size}")
        };
        let (nc, lines, _) = pl.drop_cap.unwrap_or((0, 0, 0.0));
        assert_eq!((nc, lines), (1, 3), "size {size}");
    }
}

#[test]
fn drop_cap_indents_its_lines() {
    let mut p =
        wordcraft_doc::Paragraph::with_text(&"Every line of this paragraph wraps around a large first letter. ".repeat(6), Default::default());
    p.props.drop_cap = Some(3);
    let mut d = Document::from_text("x");
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = lay(&d);
    let Some(pl) = l.pages[0].items.iter().find_map(|i| if let Placed::Lines { para, .. } = i { Some(para.clone()) } else { None }) else {
        panic!("no lines")
    };
    let (nc, lines, w) = pl.drop_cap.unwrap_or((0, 0, 0.0));
    assert_eq!((nc, lines), (1, 3));
    assert!(w > 20.0, "drop cap width {w}");
    assert!(pl.lines.len() > 4);
    for (k, line) in pl.lines.iter().enumerate() {
        let x = line.xs.first().copied().unwrap_or(0.0);
        if k < 3 {
            assert!(x >= w - 0.5, "line {k} at {x}");
        } else {
            assert!(x < 1.0, "line {k} at {x}");
        }
    }
    // The letter's glyphs are lowered toward the third line's baseline.
    let g = pl.glyphs.first().map(|g| g.dy).unwrap_or(0.0);
    assert!(g < -20.0, "dy {g}");
}

#[test]
fn auto_hyphenation_breaks_long_words() {
    let text = "Internationalization considerations notwithstanding, administrators systematically reconsidered extraordinarily uncharacteristic responsibilities. ".repeat(4);
    let mut d = Document::from_text(&text);
    let off = lay(&d);
    let hyphens = |l: &DocLayout| {
        l.pages[0]
            .items
            .iter()
            .filter_map(|i| if let Placed::Lines { para, .. } = i { Some(para.lines.iter().filter(|x| x.hyphen.is_some()).count()) } else { None })
            .sum::<usize>()
    };
    assert_eq!(hyphens(&off), 0);
    d.settings.auto_hyphenation = true;
    let on = lay(&d);
    assert!(hyphens(&on) > 0, "no hyphenated lines");
    // Hyphenated lines stay inside the margins.
    for it in &on.pages[0].items {
        if let Placed::Lines { para, .. } = it {
            for l in para.lines.iter().filter(|l| l.hyphen.is_some()) {
                assert!(l.xs.last().copied().unwrap_or(0.0) <= l.right + 0.5, "{} > {}", l.xs.last().copied().unwrap_or(0.0), l.right);
            }
        }
    }
    // Soft hyphens break even without auto hyphenation, and show a hyphen.
    let soft = format!("{}extra\u{ad}ordinary", "word ".repeat(13));
    let d2 = Document::from_text(&soft);
    let l2 = lay(&d2);
    let _ = hyphens(&l2);
}

/// Hyphenation Options (#407): a consecutive-hyphen limit of 1 never lets two lines in a row end
/// with a hyphen, and words in capitals stay whole when the document says so.
#[test]
fn hyphenation_options_limit_hyphens_and_spare_capitals() {
    let text = "Internationalization considerations notwithstanding, administrators systematically reconsidered extraordinarily uncharacteristic responsibilities. ".repeat(6);
    let lines = |d: &Document| -> Vec<bool> {
        lay(d).pages[0]
            .items
            .iter()
            .filter_map(|i| {
                if let Placed::Lines { para, .. } = i { Some(para.lines.iter().map(|x| x.hyphen.is_some()).collect::<Vec<_>>()) } else { None }
            })
            .flatten()
            .collect()
    };
    let mut d = Document::from_text(&text);
    d.settings.auto_hyphenation = true;
    d.settings.consecutive_hyphen_limit = 1;
    let hy = lines(&d);
    assert!(hy.windows(2).all(|w| !(w[0] && w[1])), "two hyphenated lines in a row: {hy:?}");
    let mut caps = Document::from_text(&text.to_uppercase());
    caps.settings.auto_hyphenation = true;
    caps.settings.hyphenate_caps = false;
    assert!(lines(&caps).iter().all(|h| !h), "a word in capitals was hyphenated");
}

/// A table of `rows` x 2 with text in every cell, after a "before" paragraph.
fn styled_table_doc(style: Option<&str>, rows: usize) -> Document {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(rows, 2, 468.0);
    t.props.style = style.map(str::to_string);
    for (r, row) in t.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("R{r} C{c}"), Default::default()))];
        }
    }
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    d
}

fn row_height(l: &DocLayout, row: usize) -> f32 {
    l.pages[0].items.iter().find_map(|i| if let Placed::Cell { rect, row: r, .. } = i { (*r == row).then_some(rect.h) } else { None }).unwrap()
}

/// The laid-out paragraph in cell (row, col) of the table at body index 1.
fn cell_para(l: &DocLayout, row: u32, col: u32) -> Arc<ParaLayout> {
    l.pages[0]
        .items
        .iter()
        .find_map(|i| if let Placed::Lines { path, para, .. } = i { (path.0 == vec![1, row, col, 0]).then(|| para.clone()) } else { None })
        .unwrap()
}

/// Text in a table cell takes its table style's paragraph and run formatting over the document
/// defaults (ECMA-376 §17.7.2): Table Grid's single spacing and no space after, not the defaults'
/// 8 pt after and 1.15 lines.
#[test]
fn table_style_paragraph_props_beat_document_defaults() {
    let d = styled_table_doc(Some("TableGrid"), 3);
    let plain = styled_table_doc(None, 3);
    let (lg, lp) = (lay(&d), lay(&plain));
    let p = cell_para(&lg, 1, 0);
    assert_eq!((p.rp.space_after, p.rp.line_spacing), (0.0, wordcraft_doc::props::LineSpacing::Multiple(1.0)));
    let q = cell_para(&lp, 1, 0);
    assert_eq!((q.rp.space_after, q.rp.line_spacing), (8.0, wordcraft_doc::props::LineSpacing::Multiple(1.15)));
    let (g, n) = (row_height(&lg, 1), row_height(&lp, 1));
    assert!(g + 8.0 < n, "Table Grid row {g}, unstyled row {n}");
    assert!(g < 18.0, "one 12 pt line, single spaced: {g}");
}

/// The paragraph's own style (Normal included) and direct formatting beat the table style; the
/// table style only fills in what they leave unset.
#[test]
fn paragraph_style_and_direct_formatting_beat_table_style() {
    let mut d = styled_table_doc(Some("TableGrid"), 3);
    d.styles.upsert(wordcraft_doc::Style {
        id: "Spaced".into(),
        name: "Spaced".into(),
        based_on: Some("Normal".into()),
        para: ParaProps { space_after: Some(12.0), ..Default::default() },
        ..Default::default()
    });
    d.format_paragraphs(
        &Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 },
        &Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 },
        &|p| p.style = Some("Spaced".into()),
    )
    .unwrap();
    d.format_paragraphs(
        &Pos { story: StoryRef::Body, path: Path(vec![1, 1, 0, 0]), off: 0 },
        &Pos { story: StoryRef::Body, path: Path(vec![1, 1, 0, 0]), off: 0 },
        &|p| p.space_after = Some(3.0),
    )
    .unwrap();
    let l = lay(&d);
    let styled = cell_para(&l, 0, 0);
    assert_eq!(styled.rp.space_after, 12.0, "paragraph style");
    assert_eq!(styled.rp.line_spacing, wordcraft_doc::props::LineSpacing::Multiple(1.0), "unset by the style: from the table style");
    assert_eq!(cell_para(&l, 1, 0).rp.space_after, 3.0, "direct formatting");
    assert_eq!(cell_para(&l, 2, 0).rp.space_after, 0.0, "table style");
    // Normal setting its own spacing wins too, even where it equals nothing in the defaults.
    if let Some(n) = d.styles.get_mut("Normal") {
        n.para.space_after = Some(10.0);
    }
    assert_eq!(cell_para(&lay(&d), 2, 0).rp.space_after, 10.0, "Normal");
}

/// A table style based on another keeps the base style's borders and cell text formatting, and
/// adds its own run formatting and conditional formats.
#[test]
fn derived_table_style_merges_its_base() {
    use wordcraft_doc::styles::TableStyleParts;
    let red = wordcraft_doc::Rgb(0xC0, 0, 0);
    let mut d = styled_table_doc(Some("RedGrid"), 3);
    d.styles.upsert(wordcraft_doc::Style {
        id: "RedGrid".into(),
        name: "Red Grid".into(),
        kind: wordcraft_doc::StyleKind::Table,
        based_on: Some("TableGrid".into()),
        chr: CharProps { bold: Some(true), color: Some(wordcraft_doc::TextColor::Rgb(red)), ..Default::default() },
        table: Some(TableStyleParts { header_chr: CharProps { italic: Some(true), ..Default::default() }, ..Default::default() }),
        ..Default::default()
    });
    let l = lay(&d);
    let rules = l.pages[0].items.iter().filter(|i| matches!(i, Placed::Rule { .. })).count();
    assert!(rules >= 18, "Table Grid's borders: {rules}");
    let body = cell_para(&l, 1, 1);
    let rc = &body.styles[0].rc;
    assert!(rc.bold && !rc.italic, "{rc:?}");
    assert_eq!(rc.color, wordcraft_doc::TextColor::Rgb(red));
    assert_eq!(body.rp.space_after, 0.0, "Table Grid's paragraph formatting");
    let head = &cell_para(&l, 0, 0).styles[0].rc;
    assert!(head.bold && head.italic, "{head:?}");
    // Direct formatting still wins over the table style.
    let mut d2 = d.clone();
    let path = Path(vec![1, 2, 0, 0]);
    d2.format_range(&Pos { story: StoryRef::Body, path: path.clone(), off: 0 }, &Pos { story: StoryRef::Body, path, off: 5 }, &|c| {
        c.bold = Some(false)
    })
    .unwrap();
    assert!(!cell_para(&lay(&d2), 2, 0).styles[0].rc.bold);
}

fn text_box(d: &mut Document, at: usize, text: &str, w: f32, h: f32, float: wordcraft_doc::para::Float) -> u32 {
    text_box_at(d, &Pos::body(0, at), text, w, h, float)
}

fn text_box_at(d: &mut Document, pos: &Pos, text: &str, w: f32, h: f32, float: wordcraft_doc::para::Float) -> u32 {
    let id =
        d.add_part(wordcraft_doc::PartKind::TextBox, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))]);
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float,
        story: Some(id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(pos, shape, &Default::default()).unwrap();
    id
}

fn box_rect(l: &DocLayout, id: u32) -> wordcraft_geom::Rect {
    l.pages[0]
        .items
        .iter()
        .find_map(|i| if let Placed::Object { rect, text_box, .. } = i { (*text_box == Some(id)).then_some(*rect) } else { None })
        .unwrap()
}

#[test]
fn clicks_inside_a_text_box_hit_its_story() {
    let mut d = Document::from_text("Before after");
    let id = text_box(&mut d, 7, "Inside the box", 144.0, 72.0, Default::default());
    let l = lay(&d);
    let r = box_rect(&l, id);
    assert!((r.w - 144.0).abs() < 0.5 && (r.h - 72.0).abs() < 0.5, "{r:?}");
    let box_story = StoryRef::Part(id);
    // Anywhere inside the box, even below its text, is the box.
    for (x, y) in [(r.x + 20.0, r.y + 10.0), (r.x + r.w - 4.0, r.y + r.h - 4.0)] {
        assert_eq!(l.story_at(0, x, y), Some(box_story), "({x}, {y}) in {r:?}");
        assert_eq!(l.hit(0, x, y, box_story).map(|p| p.story), Some(box_story));
    }
    // The body text beside it is the body; so is the margin just right of the box, although the
    // box's own lines are within reach there.
    let c = l.caret(&Pos::body(0, 2)).unwrap();
    assert_eq!(l.story_at(0, c.x, c.top + c.height / 2.0), Some(StoryRef::Body));
    assert_ne!(l.story_at(0, r.right() + 8.0, r.y + 10.0), Some(box_story));
    // The caret in the box sits inside it.
    let bc = l.caret(&Pos { story: box_story, path: Path::top(0), off: 0 }).unwrap();
    assert!(r.contains(wordcraft_geom::Point::new(bc.x, bc.top)), "{bc:?} outside {r:?}");
}

#[test]
fn body_text_wins_over_a_text_box_behind_it() {
    let mut d = Document::from_text("Hi");
    let float = wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::BehindText,
        h_rel: wordcraft_doc::para::Anchor::Column,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        x: 0.0,
        y: 0.0,
        dist: 0.0,
        ..Default::default()
    };
    let id = text_box(&mut d, 0, "Behind", 200.0, 150.0, float);
    let l = lay(&d);
    let r = box_rect(&l, id);
    let c = l.caret(&Pos::body(0, 0)).unwrap();
    assert!(r.contains(wordcraft_geom::Point::new(c.x + 2.0, c.top + 2.0)), "box covers the text: {r:?} {c:?}");
    assert_eq!(l.story_at(0, c.x + 2.0, c.top + c.height / 2.0), Some(StoryRef::Body));
    assert_eq!(l.story_at(0, r.right() - 10.0, r.bottom() - 10.0), Some(StoryRef::Part(id)));
}

#[test]
fn presses_grab_pictures_anywhere_and_text_boxes_by_their_border() {
    let mut d = Document::from_text("Words before the objects.");
    let id = text_box(&mut d, 0, "Inside", 144.0, 72.0, Default::default());
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        w: 60.0,
        h: 40.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let l = lay(&d);
    let b = box_rect(&l, id);
    // The text box: its border grabs it, inside is its text.
    let edge = l.object_at(0, b.x + 1.0, b.y + b.h / 2.0, 4.0).unwrap();
    assert_eq!(edge.text_box, Some(id));
    assert_eq!(l.object(&edge.pos(), 0).map(|o| o.rect), Some(b));
    assert_eq!(l.text_box(id, 5).map(|o| o.rect), Some(b), "a bad hint still finds it");
    assert!(l.object_at(0, b.x + b.w / 2.0, b.y + b.h / 2.0, 4.0).is_none());
    // The shape: anywhere on it.
    let s = l.find_object(0, |o| o.text_box.is_none()).unwrap();
    let hit = l.object_at(0, s.rect.x + s.rect.w / 2.0, s.rect.y + s.rect.h / 2.0, 4.0).unwrap();
    assert_eq!((hit.off, hit.text_box, hit.floating()), (0, None, false));
    // Plain text: nothing.
    let c = l.caret(&Pos::body(0, 10)).unwrap();
    assert!(l.object_at(0, c.x, c.top + 2.0, 4.0).is_none());
}

/// (lines placed, lines laid out) of a text box's story, and its area.
fn box_lines(l: &DocLayout, id: u32) -> (usize, usize, wordcraft_geom::Rect) {
    let mut shown = 0;
    let mut total = 0;
    for it in &l.pages[0].items {
        if let Placed::Lines { story: StoryRef::Part(p), para, l0, l1, y, .. } = it
            && *p == id
        {
            shown += l1 - l0;
            total += para.lines.len();
            let first = &para.lines[*l0];
            let last = &para.lines[l1 - 1];
            let bottom = y + last.top + last.height - first.top;
            let r = box_rect(l, id);
            assert!(bottom <= r.bottom() + 0.5 || shown == 1, "a placed line overflows: {bottom} > {}", r.bottom());
        }
    }
    (shown, total, box_rect(l, id))
}

#[test]
fn text_box_hides_text_that_does_not_fit() {
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(12);
    let mut d = Document::from_text("Body");
    let id = text_box(&mut d, 0, &long, 144.0, 72.0, Default::default());
    let (shown, total, r) = box_lines(&lay(&d), id);
    assert!(shown >= 2 && shown < total, "{shown} of {total} lines shown in {r:?}");
    // Nothing below the box belongs to it.
    let l = lay(&d);
    assert_ne!(l.story_at(0, r.x + 20.0, r.bottom() + 6.0), Some(StoryRef::Part(id)));
    // A taller box shows more; a tiny one still shows its first line.
    let mut d2 = Document::from_text("Body");
    let id2 = text_box(&mut d2, 0, &long, 144.0, 1500.0, Default::default());
    let (shown2, total2, _) = box_lines(&lay(&d2), id2);
    assert_eq!(shown2, total2);
    let mut d3 = Document::from_text("Body");
    let id3 = text_box(&mut d3, 0, &long, 144.0, 18.0, Default::default());
    assert_eq!(box_lines(&lay(&d3), id3).0, 1);
}

#[test]
fn square_wrap_flows_text_on_both_sides() {
    let float = |x: f32| wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::Square,
        h_rel: wordcraft_doc::para::Anchor::Column,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        x,
        y: 0.0,
        dist: 9.0,
        ..Default::default()
    };
    let text = "Words flow on both sides of the box in the middle here. ".repeat(20);
    let mut d = Document::from_text(&text);
    let id = text_box(&mut d, 0, "Middle", 144.0, 100.0, float(160.0));
    let l = lay(&d);
    let r = box_rect(&l, id);
    let Placed::Lines { para, x, y, .. } = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { story: StoryRef::Body, .. })).unwrap() else {
        panic!()
    };
    let beside: Vec<&para::Line> = para.lines.iter().filter(|ln| ln.beside).collect();
    assert!(beside.len() >= 4, "{} rows have text on the far side", beside.len());
    for (k, ln) in para.lines.iter().enumerate().filter(|(_, ln)| ln.beside) {
        let prev = &para.lines[k - 1];
        assert_eq!((prev.top, prev.height), (ln.top, ln.height), "one row");
        assert!(x + ink_end(para, prev) <= r.x + 0.01, "left part stops before the box");
        assert!(x + ln.xs[0] >= r.right(), "right part starts after it");
        assert_eq!(prev.stop, ln.start, "text runs left part, then right part");
    }
    // Below the box: full-width rows again.
    let below = para.lines.iter().find(|ln| y + ln.top > r.bottom() + 10.0).unwrap();
    assert!(!below.beside && below.right - below.left > 400.0);
    // The right-hand part is hittable and has a caret.
    let ln = beside[0];
    let pos = l.hit(0, x + ln.xs[1], y + ln.top + 2.0, StoryRef::Body).unwrap();
    assert!(pos.off >= ln.start && pos.off <= ln.stop, "{pos:?}");
    assert!(l.caret(&pos).unwrap().x >= r.right());
    // A gap too narrow for text stays empty.
    let mut d2 = Document::from_text(&text);
    let id2 = text_box(&mut d2, 0, "Edge", 144.0, 100.0, float(468.0 - 144.0 - 20.0));
    let l2 = lay(&d2);
    let r2 = box_rect(&l2, id2);
    let Placed::Lines { para, x, .. } = l2.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { story: StoryRef::Body, .. })).unwrap() else {
        panic!()
    };
    assert!(para.lines.iter().all(|ln| !ln.beside && ink_end(para, ln) <= (r2.x - x).max(ln.right) + 0.01));
}

/// #1376: a word too long for the strip beside a square-wrapped object moves on whole (to the
/// row's next strip, or the next row) instead of being split across the strip; words that fit
/// still use it, and a word wider than the whole line still breaks where it is.
#[test]
fn a_word_too_wide_for_the_strip_beside_a_float_moves_on_whole() {
    use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
    // A `w` x 150pt square at `x` in the 468pt column, leaving a 32pt strip on its right. Each
    // line: (top, left, beside the object's far side, ends inside a word, where its text ends).
    let lines = |text: &str, x: f32, w: f32| {
        let mut d = Document::from_text(&text.repeat(6));
        let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x, ..Default::default() };
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w,
            h: 150.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float,
            story: None,
            effects: Default::default(),
            freeform: None,
            extra: Default::default(),
        };
        d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
        let l = lay(&d);
        let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { story: StoryRef::Body, .. })) else {
            panic!()
        };
        let split = |ln: &para::Line| ln.end == para::LineEnd::Wrap && ln.c1 > ln.c0 && !para.clusters[ln.c1 - 1].break_after;
        para.lines.iter().map(|ln| (ln.top, ln.left, ln.beside, split(ln), ink_end(para, ln))).collect::<Vec<_>>()
    };
    let long = "La presente procedura descrive le modalità della documentazione sanitaria del reparto e delle responsabilità degli operatori. ";
    // The strip right of the object, after the text on its left.
    let right = lines(long, 468.0 - 150.0 - 32.0, 150.0);
    assert!(right.iter().all(|l| !l.3 && l.4 <= 468.01), "{right:?}");
    let short = lines("It is so, as it is to be, if we go on. ", 468.0 - 150.0 - 32.0, 150.0);
    assert!(short.iter().any(|l| l.2) && short.iter().all(|l| !l.3), "{short:?}");
    // The strip is the row's only span: long words go below the object.
    let only = lines(long, 0.0, 468.0 - 32.0);
    assert!(only.iter().all(|l| !l.3 && (l.0 >= 150.0 - 0.01 || l.1 >= 468.0 - 32.0 - 0.01)), "{only:?}");
    assert!(only.iter().any(|l| l.0 >= 150.0 - 0.01 && l.1 < 1.0), "{only:?}");
    // A word wider than the whole line breaks where it is, beside the object.
    let huge = lines(&format!("{} ", "x".repeat(120)), 468.0 - 150.0 - 32.0, 150.0);
    assert!(huge[0].0 < 1.0 && huge.iter().any(|l| l.3), "{huge:?}");
    // The strip left of the object: a skipped strip leaves the row's other span its first line.
    let left = lines(long, 32.0, 150.0);
    assert!(left.iter().all(|l| !l.3) && left.iter().any(|l| l.0 < 150.0 && l.1 >= 182.0 - 0.01 && !l.2), "{left:?}");
    // Rows go down the page, and a line beside the object shares its row with the line before it.
    for ls in [&right, &short, &only, &huge, &left] {
        assert!(ls.windows(2).all(|w| if w[1].2 { w[1].0 == w[0].0 } else { w[1].0 > w[0].0 }), "{ls:?}");
    }
}

/// Where a line's text ends (its last non-space cluster's right edge; trailing spaces may hang
/// past the margin), relative to the column.
fn ink_end(pl: &ParaLayout, ln: &para::Line) -> f32 {
    (ln.c0..ln.c1)
        .filter_map(|k| {
            let c = pl.clusters.get(k)?;
            (c.kind != para::ClKind::Space).then_some(ln.xs.get(k - ln.c0)? + c.adv)
        })
        .fold(0.0, f32::max)
}

fn footnote(d: &mut Document, pos: &Pos, text: &str) -> u32 {
    let id =
        d.add_part(wordcraft_doc::PartKind::Footnote, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))]);
    d.insert_object(pos, InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: String::new() }, &Default::default())
        .unwrap();
    id
}

fn page_float(x: f32, y: f32) -> wordcraft_doc::para::Float {
    wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::InFrontOfText,
        h_rel: wordcraft_doc::para::Anchor::Page,
        v_rel: wordcraft_doc::para::Anchor::Page,
        x,
        y,
        dist: 0.0,
        ..Default::default()
    }
}

#[test]
fn text_boxes_show_in_headers_footers_cells_and_notes() {
    let mut d = Document::from_text(&"Body paragraph.\n".repeat(4));
    // Header: an inline box and a box placed on the page.
    let hid =
        d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Head", Default::default()))]);
    d.last_section.headers.default = Some(hid);
    let e = d.end_of(StoryRef::Part(hid));
    let inline = text_box_at(&mut d, &e, "Inline in header", 100.0, 30.0, Default::default());
    let e = d.end_of(StoryRef::Part(hid));
    let placed = text_box_at(&mut d, &e, "On the page", 100.0, 40.0, page_float(400.0, 20.0));
    // Footer: a box placed on the page (the footer is laid out twice to find it).
    let fid =
        d.add_part(wordcraft_doc::PartKind::Footer, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Foot", Default::default()))]);
    d.last_section.footers.default = Some(fid);
    let e = d.end_of(StoryRef::Part(fid));
    let in_footer = text_box_at(&mut d, &e, "Footer box", 100.0, 30.0, page_float(300.0, 740.0));
    // A table cell and a footnote.
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(wordcraft_doc::Table::new(1, 1, 300.0))).unwrap();
    let cell = Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 };
    let in_cell = text_box_at(&mut d, &cell, "In a cell", 120.0, 30.0, Default::default());
    let note = footnote(&mut d, &Pos::body(0, 4), "Note: ");
    let e = d.end_of(StoryRef::Part(note));
    let in_note = text_box_at(&mut d, &e, "In a note", 120.0, 30.0, Default::default());
    let l = lay(&d);
    let p = &l.pages[0];
    let shows =
        |id: u32| p.header.iter().chain(&p.footer).chain(&p.items).any(|i| matches!(i, Placed::Lines { story: StoryRef::Part(s), .. } if *s == id));
    for (name, id) in
        [("inline header box", inline), ("page header box", placed), ("footer box", in_footer), ("cell box", in_cell), ("note box", in_note)]
    {
        assert!(shows(id), "{name}: no text");
        assert!(l.caret(&d.start_of(StoryRef::Part(id))).is_some(), "{name}: no caret");
    }
    // Page-anchored boxes land where they say, header and footer alike; their shapes are drawn.
    assert_eq!(l.text_box(placed, 0).map(|o| (o.rect.x, o.rect.y)), Some((400.0, 20.0)));
    assert_eq!(l.text_box(in_footer, 0).map(|o| (o.rect.x, o.rect.y)), Some((300.0, 740.0)));
    assert!(p.header.iter().any(|i| matches!(i, Placed::Shape { rect, .. } if rect.x == 400.0)));
    // Header boxes are found for clicks while editing the header.
    let r = l.text_box(inline, 0).unwrap().rect;
    assert_eq!(l.header_footer_text_box_at(0, r.x + 5.0, r.y + 5.0), Some(inline));
}

#[test]
fn footnotes_inside_text_boxes_are_numbered_and_placed() {
    let mut d = Document::from_text(&"Body text line.\n".repeat(30));
    let id = text_box_at(&mut d, &Pos::body(2, 0), "Box text", 144.0, 40.0, Default::default());
    let e = d.end_of(StoryRef::Part(id));
    let in_box = footnote(&mut d, &e, "Note from the box.");
    let after = footnote(&mut d, &Pos::body(5, 4), "Note from the body.");
    let l = lay(&d);
    // Both notes are at the bottom of page 1, the box's first (reading order).
    let a = l.caret(&d.start_of(StoryRef::Part(in_box))).expect("the box's footnote is placed");
    let b = l.caret(&d.start_of(StoryRef::Part(after))).unwrap();
    assert_eq!((a.page, b.page), (0, 0));
    assert!(a.top > 600.0 && a.top < b.top, "{a:?} {b:?}");
    let nums = note_numbers(&d, false);
    assert_eq!((nums.get(&in_box), nums.get(&after)), (Some(&1), Some(&2)));
}

/// A document whose text box `ids[k]` holds `fan` shapes showing `ids[k + 1]` (the last one shows
/// itself, `fan` times): crafted, since the app never builds these.
fn box_fan_out(levels: usize, fan: usize) -> Document {
    let (w, h) = (40.0, 20.0);
    let mut d = Document::from_text("Body");
    let ids: Vec<u32> = (0..levels)
        .map(|k| {
            d.add_part(
                wordcraft_doc::PartKind::TextBox,
                vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("Level {k}"), Default::default()))],
            )
        })
        .collect();
    let shape = |story| InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    for (k, id) in ids.iter().enumerate() {
        let next = *ids.get(k + 1).unwrap_or(id);
        for _ in 0..fan {
            d.insert_object(&Pos { story: StoryRef::Part(*id), path: Path::top(0), off: 0 }, shape(next), &Default::default()).unwrap();
        }
    }
    d.insert_object(&Pos::body(0, 0), shape(ids[0]), &Default::default()).unwrap();
    d
}

#[test]
fn self_showing_and_fanned_out_text_boxes_stay_bounded() {
    // A box whose 30 shapes all show itself: laid out once.
    assert_eq!(lay(&box_fan_out(1, 30)).text_boxes, 1);
    // Chains of boxes each showing the next 30 times (30^4 expansions unbounded).
    let d = box_fan_out(5, 30);
    let l = lay(&d);
    assert!(l.text_boxes <= 1 + wordcraft_doc::BoxBudget::MAX_NESTED, "{} boxes laid out", l.text_boxes);
    // Footnote numbering's walk is bounded the same way.
    let mut visits = 0usize;
    d.objects_in_reading_order(&mut |_| visits += 1);
    assert!(visits < 5_000, "{visits} visits");
}

#[test]
fn many_text_boxes_and_a_header_box_all_show() {
    // Outermost boxes aren't budgeted: a long document's boxes, and its header's on every page,
    // all get their text.
    let mut d = Document::from_text("Paragraph with a box.");
    for _ in 1..400 {
        let end = d.end_of(StoryRef::Body);
        d.split_paragraph(&end).unwrap();
    }
    let ids: Vec<u32> = (0..400).map(|k| text_box_at(&mut d, &Pos::body(k, 0), &format!("Box {k}"), 100.0, 30.0, Default::default())).collect();
    let hid =
        d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Head", Default::default()))]);
    d.last_section.headers.default = Some(hid);
    let e = d.end_of(StoryRef::Part(hid));
    let in_header = text_box_at(&mut d, &e, "Header box", 100.0, 30.0, Default::default());
    let l = lay(&d);
    assert!(l.pages.len() >= 10, "{} pages", l.pages.len());
    for id in ids {
        assert!(l.caret(&d.start_of(StoryRef::Part(id))).is_some(), "box {id} has no text");
    }
    for (i, p) in l.pages.iter().enumerate() {
        let shown = p.header.iter().any(|it| matches!(it, Placed::Lines { story: StoryRef::Part(s), .. } if *s == in_header));
        assert!(shown, "page {i}: the header box has no text");
    }
}

#[test]
fn a_text_box_inside_a_text_box_still_shows_its_text() {
    let mut d = Document::from_text("Body");
    let outer = text_box(&mut d, 0, "Outer ", 300.0, 200.0, Default::default());
    let e = d.end_of(StoryRef::Part(outer));
    let inner = text_box_at(&mut d, &e, "Inner", 120.0, 40.0, Default::default());
    let l = lay(&d);
    assert_eq!(l.text_boxes, 2);
    assert!(l.caret(&d.start_of(StoryRef::Part(inner))).is_some(), "the inner box's text is laid out");
}

fn picture(w: f32, h: f32, float: wordcraft_doc::para::Float) -> InlineObject {
    InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

#[test]
fn line_spacing_does_not_scale_pictures() {
    use wordcraft_doc::props::LineSpacing;
    let mut d = Document::from_text("");
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.props.line_spacing = Some(LineSpacing::Multiple(1.15));
        p.insert_object(0, picture(144.0, 144.0, Default::default()), &Default::default()).unwrap();
    }
    let l = lay(&d);
    let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
    let line = &para.lines[0];
    let desc = para.styles[0].descent;
    // Word: the picture plus the font's descent, not (picture + descent) x 1.15.
    assert!((line.height - (144.0 + desc)).abs() < 0.01, "line {} for a 144pt picture (descent {desc})", line.height);
    assert!((line.baseline - line.top - 144.0).abs() < 0.01, "the picture stands on the baseline");
}

#[test]
fn multiple_line_spacing_adds_its_leading_below_the_text() {
    use wordcraft_doc::props::LineSpacing;
    // (spacing, expected baseline below the line top as a share of the ascent)
    for (spacing, share) in [(LineSpacing::Multiple(1.15), 1.0), (LineSpacing::Multiple(2.0), 1.0), (LineSpacing::Multiple(0.8), 0.8)] {
        let mut d = Document::from_text(&"Several words wrap onto more lines. ".repeat(30));
        if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
            p.props.line_spacing = Some(spacing);
        }
        let l = lay(&d);
        let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        let asc = para.styles[0].ascent;
        assert!(para.lines.len() > 1);
        for line in &para.lines {
            // Word's 1.15-spaced Times New Roman 12pt sits one ascent below the line top; the
            // 0.15 extra goes below the descent (docxide-pdf's hyphenation fixtures).
            assert!(
                (line.baseline - line.top - asc * share).abs() < 0.01,
                "{spacing:?}: baseline {} below the top, ascent {asc}",
                line.baseline - line.top
            );
        }
    }
    // Exact spacing still keeps the descent below the baseline.
    let mut d = Document::from_text("Exact");
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.props.line_spacing = Some(LineSpacing::Exactly(30.0));
    }
    let l = lay(&d);
    let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
    let line = &para.lines[0];
    assert!((line.top + line.height - line.baseline - para.styles[0].descent).abs() < 0.01);
}

#[test]
fn list_label_does_not_inflate_an_inline_picture_floor() {
    use wordcraft_doc::props::{LineSpacing, NumRef};
    for spacing in [LineSpacing::Multiple(1.25), LineSpacing::AtLeast(12.0)] {
        let mut d = Document::from_text("");
        let id = d.numbering.add_list(wordcraft_doc::ListKind::Bullet);
        d.numbering.abstracts[0].levels[0].chr.size = Some(24.0);
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.props.line_spacing = Some(spacing);
        p.props.numbering = Some(NumRef { num: id, level: 0 });
        p.insert_object(0, picture(144.0, 144.0, Default::default()), &Default::default()).unwrap();
        let l = lay(&d);
        let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        let line = &para.lines[0];
        let desc = para.styles[para.clusters[0].style as usize].descent;
        assert!((line.height - (144.0 + desc)).abs() < 0.01, "{spacing:?}: {}", line.height);
        assert!((line.baseline - line.top - 144.0).abs() < 0.01);
    }
}

#[test]
fn body_starts_right_below_a_tall_header() {
    let mut d = Document::from_text("Body");
    let mut hp = wordcraft_doc::Paragraph::with_text("", Default::default());
    hp.insert_object(0, picture(100.0, 80.0, Default::default()), &Default::default()).unwrap();
    hp.props.space_after = Some(0.0);
    let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
    d.last_section.headers.default = Some(id);
    let l = lay(&d);
    let Some(Placed::Lines { para, y, .. }) = l.pages[0].header.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
    let header_bottom = y + para.height;
    assert!(header_bottom > d.last_section.margin_top, "the header reaches past the top margin");
    // No gap: the body's first paragraph starts where the header ends (Word).
    let body = l.pages[0].items.iter().find_map(|i| if let Placed::Lines { y, story: StoryRef::Body, .. } = i { Some(*y) } else { None }).unwrap();
    assert!((body - header_bottom).abs() < 0.01, "body at {body}, header ends at {header_bottom}");
}

#[test]
fn inline_picture_keeps_room_for_its_effects() {
    let mut d = Document::from_text("");
    let shadow = wordcraft_doc::para::Float { effect: [6.0, 12.0, 18.0, 27.0], ..Default::default() };
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.insert_object(0, picture(100.0, 50.0, shadow.clone()), &Default::default()).unwrap();
    }
    let l = lay(&d);
    let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
    let c = para.clusters.iter().find(|c| matches!(c.kind, para::ClKind::Object(_))).unwrap();
    assert_eq!((c.adv, c.obj_h), (124.0, 89.0), "picture plus effect extents");
    // Drawn inside that room.
    let obj = picture(100.0, 50.0, shadow);
    let r = display::inline_rect(Some(&obj), 10.0, 200.0, c.adv, c.obj_h);
    assert_eq!((r.x, r.y, r.w, r.h), (16.0, 123.0, 100.0, 50.0));
}

/// Tops of the body text lines drawn on page one, in order.
fn line_tops(l: &DocLayout) -> Vec<f32> {
    l.pages[0].items.iter().filter_map(|i| if let Placed::Lines { y, story: StoryRef::Body, .. } = i { Some(*y) } else { None }).collect()
}

#[test]
fn at_least_rows_add_cell_margins_and_border_bands() {
    use wordcraft_doc::props::{Border, BorderStyle, Borders, HeightRule};
    let mut d = Document::from_text("");
    let line = Some(Border { style: BorderStyle::Single, width: 0.5, color: None, space: 0.0 });
    let mut t = Table::new(2, 1, 200.0);
    t.props.borders = Some(Borders { top: line, left: line, bottom: line, right: line, between: line, inside_v: line });
    t.props.cell_margins = Some([5.0, 5.4, 5.0, 5.4]);
    for (r, text) in t.rows.iter_mut().zip(["one", "two"]) {
        r.props.height = Some(30.0);
        r.props.height_rule = HeightRule::AtLeast;
        r.cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))];
    }
    d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t))];
    let tops = line_tops(&lay(&d));
    // Word: a 30pt at-least row with 5pt margins and 0.5pt borders steps 40.5pt, and the text
    // starts below the top border and the top margin.
    assert!((tops[1] - tops[0] - 40.5).abs() < 0.01, "row pitch {}", tops[1] - tops[0]);
    assert!((tops[0] - (72.0 + 0.5 + 5.0)).abs() < 0.01, "first cell text at {}", tops[0]);
}

/// Word gives every cell in a row the row's largest top and bottom margins: a cell without its
/// own lines up with a neighbour that has them, and the row is as tall as if all had them.
#[test]
fn a_row_shares_its_largest_top_and_bottom_margins() {
    let mut d = Document::from_text("");
    let mut t = Table::new(3, 2, 300.0);
    t.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    t.rows[0].cells[0].props.margins = Some([6.0, 5.4, 3.0, 5.4]);
    t.rows[0].cells[1].props.margins = Some([2.0, 5.4, 7.0, 5.4]);
    for (ri, row) in t.rows.iter_mut().enumerate() {
        for (c, text) in row.cells.iter_mut().zip(["one", "two"]) {
            c.blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("{text}{ri}"), Default::default()))];
        }
    }
    d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t))];
    let tops = line_tops(&lay(&d));
    assert_eq!(tops.len(), 6, "{tops:?}");
    assert!((tops[0] - tops[1]).abs() < 0.01, "both cells start 6pt down: {tops:?}");
    assert!((tops[0] - (72.0 + 0.5 + 6.0)).abs() < 0.01, "below the 0.5pt top border: {tops:?}");
    // The rows below have no margins: the second is one line tall, and the first is a line plus
    // 6pt above and 7pt below it.
    let line = tops[4] - tops[2];
    assert!((tops[2] - tops[0] - (line + 7.0)).abs() < 0.01, "{tops:?}");
}

/// An exact row height gains only the bottom cell margin (Word: a 35pt exact row with 4pt top
/// and bottom margins steps 39pt; the text starts 4pt below its top).
#[test]
fn exact_rows_add_only_the_bottom_margin() {
    use wordcraft_doc::props::HeightRule;
    let mut d = Document::from_text("");
    let mut t = Table::new(2, 1, 200.0);
    t.props.cell_margins = Some([4.0, 5.4, 4.0, 5.4]);
    for (r, text) in t.rows.iter_mut().zip(["one", "two"]) {
        r.props.height = Some(35.0);
        r.props.height_rule = HeightRule::Exact;
        r.cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))];
    }
    d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t))];
    let tops = line_tops(&lay(&d));
    assert!((tops[1] - tops[0] - 39.0).abs() < 0.01, "row pitch {}", tops[1] - tops[0]);
    assert!((tops[0] - (72.0 + 0.5 + 4.0)).abs() < 0.01, "first cell text at {}", tops[0]);
}

/// `cell_margins` is what the layout uses: a cell's own margins, else the table's, else Word's;
/// each cell's text starts its left margin inside the cell.
#[test]
fn cell_margins_are_where_cell_text_starts() {
    let mut d = Document::from_text("");
    let mut t = Table::new(1, 2, 300.0);
    t.props.cell_margins = Some([0.0, 9.0, 0.0, 3.0]);
    t.rows[0].cells[1].props.margins = Some([0.0, 20.0, 0.0, 4.0]);
    for (c, text) in t.rows[0].cells.iter_mut().zip(["one", "two"]) {
        c.blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))];
    }
    assert_eq!(cell_margins(&d, &t, 0, 0), [0.0, 9.0, 0.0, 3.0]);
    assert_eq!(cell_margins(&d, &t, 0, 1), [0.0, 20.0, 0.0, 4.0]);
    assert_eq!(cell_margins(&d, &t, 5, 9), [0.0, 9.0, 0.0, 3.0], "no such cell: the table's");
    assert_eq!(cell_margins(&d, &Table::new(1, 1, 100.0), 0, 0), [0.0, 5.4, 0.0, 5.4], "Word's");
    d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t.clone()))];
    let l = lay(&d);
    let items = &l.pages[0].items;
    for c in 0..2 {
        let boxed = items.iter().find_map(|it| match it {
            Placed::Cell { rect, cell, .. } if *cell == c => Some(*rect),
            _ => None,
        });
        let text_x = items.iter().find_map(|it| match it {
            Placed::Lines { path, x, .. } if path.0.get(2) == Some(&(c as u32)) => Some(*x),
            _ => None,
        });
        let (boxed, text_x) = (boxed.unwrap(), text_x.unwrap());
        assert!((text_x - boxed.x - cell_margins(&d, &t, 0, c)[1]).abs() < 0.01, "cell {c}: text at {text_x}, cell at {}", boxed.x);
    }
}

/// Line count and, per line, how far the text (trailing spaces excluded) reaches.
fn justified_lines(text: &str, compat_mode: u32, align: Align) -> (usize, Vec<f32>) {
    let mut d = Document::from_text(text);
    d.settings.compat_mode = compat_mode;
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.align = Some(align)).unwrap();
    let l = lay(&d);
    let mut ends = Vec::new();
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { para, l0, l1, x, .. } = it {
                for li in *l0..*l1 {
                    let line = &para.lines[li];
                    let content = text[line.start..line.stop].trim_end().len();
                    ends.push(para.x_of(li, line.start + content).unwrap() + x);
                }
            }
        }
    }
    (lines_of(&l), ends)
}

#[test]
fn justified_lines_shrink_spaces_in_word_2013_mode() {
    let text = "We tie the spend to a pipe and to the books, so it is in an ad hoc view of all of it. ".repeat(40);
    let (modern, ends) = justified_lines(&text, 15, Align::Justify);
    let (legacy, legacy_ends) = justified_lines(&text, 14, Align::Justify);
    assert!(modern < legacy, "mode 15 fits more per line: {modern} vs {legacy} lines");
    // Shrinking only ever pulls a line back inside the margin.
    for e in ends.iter().chain(&legacy_ends) {
        assert!(*e <= 540.0 + 0.05, "line ends past the right margin: {e}");
    }
    // Justified lines still reach the margin; the last line is the only short one.
    for e in &ends[..ends.len() - 1] {
        assert!((*e - 540.0).abs() < 0.05, "justified line ends at {e}");
    }
}

#[test]
fn space_shrinking_is_only_for_justified_text() {
    let text = "We tie the spend to a pipe and to the books, so it is in an ad hoc view of all of it. ".repeat(40);
    for align in [Align::Left, Align::Center, Align::Right] {
        assert_eq!(justified_lines(&text, 15, align).0, justified_lines(&text, 14, align).0, "{align:?}");
    }
}

#[test]
fn lines_with_tabs_never_shrink() {
    let text = "Item\tWe tie the spend to a pipe and to the books, so it is in an ad hoc view of all of it. ".repeat(30);
    let (modern, _) = justified_lines(&text, 15, Align::Justify);
    let (legacy, _) = justified_lines(&text, 14, Align::Justify);
    assert!(modern <= legacy);
    let (_, ends) = justified_lines(&text, 15, Align::Justify);
    for e in &ends {
        assert!(*e <= 540.0 + 0.05, "{e}");
    }
}

pub(super) fn rect_shape(w: f32, h: f32, float: wordcraft_doc::para::Float) -> InlineObject {
    InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

pub(super) fn shapes(items: &[Placed]) -> Vec<Rect> {
    items.iter().filter_map(|i| if let Placed::Shape { rect, .. } = i { Some(*rect) } else { None }).collect()
}

/// Top of the first body line drawn on a page.
fn first_line_top(items: &[Placed]) -> f32 {
    items.iter().find_map(|i| if let Placed::Lines { y, story: StoryRef::Body, .. } = i { Some(*y) } else { None }).unwrap()
}

#[test]
fn floats_align_within_their_reference_area() {
    use wordcraft_doc::para::{Anchor, Float, FloatAlign, Wrap};
    let mut d = Document::from_text("Some text beside the pictures.");
    let centred = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Margin, h_align: Some(FloatAlign::Center), x: 999.0, ..Default::default() };
    let right = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, h_align: Some(FloatAlign::End), ..Default::default() };
    let in_left_margin = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::LeftMargin, x: 10.0, ..Default::default() };
    let bottom = Float { wrap: Wrap::InFrontOfText, v_rel: Anchor::BottomMargin, v_align: Some(FloatAlign::Start), ..Default::default() };
    for f in [centred, right, in_left_margin, bottom] {
        d.insert_object(&Pos::body(0, 0), rect_shape(100.0, 50.0, f), &Default::default()).unwrap();
    }
    let l = lay(&d);
    // Objects are inserted at the start, so they come out in reverse order.
    let r = shapes(&l.pages[0].items);
    assert_eq!(r.len(), 4, "{r:?}");
    assert!((r[3].x - (72.0 + (468.0 - 100.0) / 2.0)).abs() < 0.01, "centred in the margins: {:?}", r[3]);
    assert!((r[2].right() - 612.0).abs() < 0.01, "right edge of the page: {:?}", r[2]);
    assert!((r[1].x - 10.0).abs() < 0.01, "measured from the page's left edge: {:?}", r[1]);
    assert!((r[0].y - (792.0 - 72.0)).abs() < 0.01, "top of the bottom margin: {:?}", r[0]);
}

#[test]
fn top_and_bottom_float_pushes_its_own_first_line_down() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    let mut d = Document::from_text(&"Text that starts below the picture. ".repeat(10));
    let f = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, dist_bottom: 6.0, ..Default::default() };
    d.insert_object(&Pos::body(0, 0), rect_shape(144.0, 100.0, f), &Default::default()).unwrap();
    let l = lay(&d);
    let r = shapes(&l.pages[0].items)[0];
    let top = first_line_top(&l.pages[0].items);
    assert!(top >= r.bottom() + 6.0 - 0.01, "first line {top} drawn under the picture ending at {}", r.bottom());
    // The caret agrees with what is drawn.
    let c = l.caret(&Pos::body(0, 4)).unwrap();
    assert!((c.top - top).abs() < 2.0, "{c:?} vs {top}");
}

#[test]
fn float_follows_its_paragraph_to_the_next_page() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    let mut d = Document::from_text(&format!("First page.\n{}", "Text beside the picture on page two. ".repeat(20)));
    let f = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, dist: 9.0, ..Default::default() };
    d.insert_object(&Pos::body(1, 0), rect_shape(144.0, 100.0, f), &Default::default()).unwrap();
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(1).map(std::sync::Arc::make_mut) {
        p.props.page_break_before = Some(true);
    }
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    assert!(shapes(&l.pages[0].items).is_empty());
    let r = shapes(&l.pages[1].items)[0];
    assert!((r.y - 72.0).abs() < 0.5, "at the top of page two: {r:?}");
    let Placed::Lines { para, .. } = l.pages[1].items.iter().find(|i| matches!(i, Placed::Lines { .. })).unwrap() else { panic!() };
    assert!(para.lines[0].left >= 144.0, "text wraps beside it: {}", para.lines[0].left);
}

/// A floating object belongs to its paragraph, not to the line holding its anchor: Word draws
/// it on the page the paragraph starts on. A Word cover page is one paragraph holding a page
/// break and then the page-anchored text box with the title; the box stays on the cover.
#[test]
fn a_float_anchored_after_a_page_break_stays_on_its_paragraph_s_first_page() {
    let mut d = Document::from_text(&format!("{}\nAfter the cover", wordcraft_doc::para::PAGE_BREAK));
    d.insert_object(&Pos::body(0, 1), rect_shape(200.0, 100.0, page_float(100.0, 300.0)), &Default::default()).unwrap();
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    let on_cover = shapes(&l.pages[0].items);
    assert_eq!(on_cover.len(), 1, "the shape is drawn on the cover: {on_cover:?}");
    assert!((on_cover[0].y - 300.0).abs() < 0.5, "{:?}", on_cover[0]);
    assert!(shapes(&l.pages[1].items).is_empty(), "and not again on page two");
}

#[test]
fn floats_in_headers_and_table_cells_are_drawn() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    let mut d = Document::from_text("Body text.");
    let logo = Float { wrap: Wrap::BehindText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 400.0, y: 20.0, ..Default::default() };
    let mut hp = wordcraft_doc::Paragraph::with_text("Header", Default::default());
    hp.insert_object(0, rect_shape(120.0, 60.0, logo), &Default::default()).unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
    d.last_section.headers.default = Some(id);
    let mut t = Table::new(1, 2, 468.0);
    let mut cp = wordcraft_doc::Paragraph::with_text("cell", Default::default());
    let in_cell = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 5.0, ..Default::default() };
    cp.insert_object(0, rect_shape(40.0, 20.0, in_cell), &Default::default()).unwrap();
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(cp)];
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let h = shapes(&l.pages[0].header);
    assert_eq!(h.len(), 1, "header float drawn");
    assert!((h[0].x - 400.0).abs() < 0.01 && (h[0].y - 20.0).abs() < 0.01, "page coordinates: {:?}", h[0]);
    assert!(matches!(l.pages[0].header.first(), Some(Placed::Shape { .. })), "behind the header text");
    let c = shapes(&l.pages[0].items);
    assert_eq!(c.len(), 1, "cell float drawn");
    assert!(c[0].x > 72.0 && c[0].x < 72.0 + 234.0, "inside the first cell: {:?}", c[0]);
}

#[test]
fn wrap_area_uses_each_distance() {
    use wordcraft_doc::para::{Float, Wrap};
    let f = Float { wrap: Wrap::Square, dist: 9.0, dist_top: 0.0, dist_bottom: 4.0, ..Default::default() };
    let (r, tb, ..) = wrap_area(Rect::new(10.0, 20.0, 100.0, 50.0), &f, None).unwrap();
    assert_eq!((r.x, r.y, r.w, r.h, tb), (1.0, 20.0, 118.0, 54.0, false));
    let hostile = Float { wrap: Wrap::TopAndBottom, dist: f32::NAN, dist_top: -5.0, dist_bottom: f32::INFINITY, ..Default::default() };
    let (r, tb, ..) = wrap_area(Rect::new(0.0, 0.0, 10.0, 10.0), &hostile, None).unwrap();
    assert!(r.w.is_finite() && r.h.is_finite() && tb, "{r:?}");
    assert!(wrap_area(Rect::new(0.0, 0.0, 10.0, 10.0), &Float { wrap: Wrap::BehindText, ..Default::default() }, None).is_none());
}

#[test]
fn float_moved_to_a_new_page_sits_at_its_paragraph_top() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    let mut d = Document::from_text(&format!("First page.\n{}", "Text beside the picture on page two. ".repeat(20)));
    let f = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, dist: 9.0, ..Default::default() };
    d.insert_object(&Pos::body(1, 0), rect_shape(144.0, 100.0, f), &Default::default()).unwrap();
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(1).map(std::sync::Arc::make_mut) {
        p.props.page_break_before = Some(true);
        p.props.space_before = Some(24.0);
    }
    let l = lay(&d);
    let r = shapes(&l.pages[1].items)[0];
    let top = first_line_top(&l.pages[1].items);
    // The picture sits at the paragraph's start, above its space before, as in Word (`plan/word`
    // R22); the text starts below the space before.
    let body_top = d.last_section.margin_top;
    assert!((r.y - body_top).abs() < 0.5 && top >= body_top, "picture at {} but its paragraph starts at {body_top}", r.y);
}

#[test]
fn page_relative_header_float_wraps_where_it_is_drawn() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    let body_top = |wrap| {
        let mut d = Document::from_text("Body text.");
        let logo = Float { wrap, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 0.0, ..Default::default() };
        let mut hp = wordcraft_doc::Paragraph::with_text("Header", Default::default());
        hp.insert_object(0, rect_shape(400.0, 20.0, logo), &Default::default()).unwrap();
        let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
        d.last_section.headers.default = Some(id);
        first_line_top(&lay(&d).pages[0].items)
    };
    // The logo sits at the top edge of the page, above the header text, so wrapping text around
    // it must not make the header taller (and push the body down) compared with no wrapping.
    let (wrapped, in_front) = (body_top(Wrap::TopAndBottom), body_top(Wrap::InFrontOfText));
    assert!((wrapped - in_front).abs() < 0.5, "body starts at {wrapped} with wrapping, {in_front} without");
}

/// Issue #149: a tall text box floating behind the text in the header (placed relative to the
/// page, no wrapping) neither makes the header taller nor pushes the body down: the body starts
/// at the top margin and paginates exactly as without the box.
#[test]
fn floating_header_text_box_leaves_the_body_at_the_top_margin() {
    use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
    let body: Vec<String> = (1..=40).map(|i| format!("Body paragraph {i}: synthetic public test content.")).collect();
    let make = |float: Option<Float>| {
        let mut d = Document::from_text(&body.join("\n"));
        // The header starts 18pt from the top, so its two lines end well above the 72pt top
        // margin whatever font the default text gets (the line height differs between the
        // installed fonts and the bundled fallback: a header at 36pt overflows the margin there).
        d.last_section.header = 18.0;
        // A label, then the paragraph the box is anchored to (empty in the control).
        let tight = ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
        let mut label = wordcraft_doc::Paragraph::with_text("Header label", Default::default());
        label.props = tight.clone();
        let mut anchor = wordcraft_doc::Paragraph { props: tight, ..Default::default() };
        if let Some(float) = float {
            let text = wordcraft_doc::Paragraph::with_text("WATERMARK", Default::default());
            let story = d.add_part(wordcraft_doc::PartKind::TextBox, vec![wordcraft_doc::para_block(text)]);
            let tb = InlineObject::Shape {
                kind: ShapeKind::TextBox,
                w: 405.0,
                h: 491.4,
                fill: None,
                stroke: None,
                stroke_width: 0.75,
                float,
                story: Some(story),
                freeform: None,
                effects: Default::default(),
                extra: Default::default(),
            };
            anchor.insert_object(0, tb, &Default::default()).unwrap();
        }
        let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(label), wordcraft_doc::para_block(anchor)]);
        d.last_section.headers.default = Some(id);
        let l = lay(&d);
        (l.pages.len(), first_line_top(&l.pages[0].items))
    };
    let (control_pages, control_top) = make(None);
    let behind = Float { wrap: Wrap::BehindText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 0.0, y: 80.0, ..Default::default() };
    let (pages, top) = make(Some(behind));
    assert!(control_pages > 1, "the control needs more than one page: {control_pages}");
    assert!((control_top - 72.0).abs() < 0.5, "control body starts at the top margin: {control_top}");
    assert!((top - control_top).abs() < 0.5, "body starts at {top} with the floating box, {control_top} without");
    assert_eq!(pages, control_pages);
    // The same box inline does take room in the header, which is what the reader used to make of it.
    let (inline_pages, inline_top) = make(Some(Float::default()));
    assert!(inline_top > 400.0 && inline_pages > control_pages, "inline box: body at {inline_top}, {inline_pages} pages");
}

/// List labels drawn, in page order (each paragraph's first line).
fn labels(l: &DocLayout) -> Vec<String> {
    l.pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|i| if let Placed::Lines { para, l0: 0, .. } = i { para.label.as_ref().map(|lb| lb.text.clone()) } else { None })
        .collect()
}

fn numbered(d: &mut Document, texts: &[&str], props: ParaProps) -> Vec<wordcraft_doc::Paragraph> {
    let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    texts
        .iter()
        .map(|t| {
            let mut p = wordcraft_doc::Paragraph::with_text(t, Default::default());
            p.props = ParaProps { numbering: Some(wordcraft_doc::props::NumRef { num, level: 0 }), ..props.clone() };
            p
        })
        .collect()
}

#[test]
fn list_items_are_counted_once_when_laid_out_again() {
    // Contextual spacing drops the space before, so each item's top differs from the first guess.
    let mut d = Document::new();
    let props = ParaProps { space_before: Some(6.0), contextual_spacing: Some(true), ..Default::default() };
    let mut items = numbered(&mut d, &["one", "two", "three"], props);
    // A page break before moves the last item after its first layout.
    items[2].props.page_break_before = Some(true);
    d.body = items.into_iter().map(wordcraft_doc::para_block).collect();
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    assert_eq!(labels(&l), ["1.", "2.", "3."]);
}

#[test]
fn list_items_beside_a_float_in_a_cell_are_counted_once() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    let mut d = Document::from_text("Body.");
    let mut items = numbered(&mut d, &["first item", "second item", "third item"], ParaProps::default());
    // The float makes the cell's paragraphs wrap, so they are laid out a second time.
    let f = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, ..Default::default() };
    items[0].insert_object(0, rect_shape(40.0, 60.0, f), &Default::default()).unwrap();
    let mut t = Table::new(1, 1, 468.0);
    t.rows[0].cells[0].blocks = items.into_iter().map(wordcraft_doc::para_block).collect();
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    assert_eq!(labels(&lay(&d)), ["1.", "2.", "3."]);
}

#[test]
fn compatibility_mode_decides_where_a_table_s_edge_sits() {
    use wordcraft_doc::props::{Border, BorderStyle, Borders};
    let cell_text_x = |mode: u32| {
        let mut d = Document::from_text("");
        d.settings.compat_mode = mode;
        let line = Some(Border { style: BorderStyle::Single, width: 0.5, color: None, space: 0.0 });
        let mut t = Table::new(1, 1, 200.0);
        t.props.borders = Some(Borders { top: line, left: line, bottom: line, right: line, between: line, inside_v: line });
        t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("cell", Default::default()))];
        d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t))];
        let l = lay(&d);
        l.pages[0].items.iter().find_map(|i| if let Placed::Lines { x, .. } = i { Some(*x) } else { None }).unwrap()
    };
    // Word 2013+: the border at the margin (moved in by half its width), the text a cell margin
    // inside it. Earlier modes: the text at the margin, the border a cell margin outside it.
    assert!((cell_text_x(15) - (72.0 + 0.25 + 5.4)).abs() < 0.01, "{}", cell_text_x(15));
    assert!((cell_text_x(12) - 72.0).abs() < 0.01, "{}", cell_text_x(12));
}

#[test]
fn floating_tables_take_no_room_and_text_wraps_beside_them() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::TableFloat;
    let mut d = Document::from_text("Body text beside the narrow table.");
    let cell = |t: &mut Table, text: &str| {
        t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))]
    };
    let mut wide = Table::new(1, 1, 468.0);
    cell(&mut wide, "Wide");
    wide.props.float = Some(TableFloat { h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, overlap: false, ..Default::default() });
    let mut narrow = Table::new(1, 1, 200.0);
    cell(&mut narrow, "Narrow");
    narrow.props.float =
        Some(TableFloat { h_rel: Anchor::Column, v_rel: Anchor::Paragraph, dist: [9.0, 0.0, 9.0, 0.0], overlap: false, ..Default::default() });
    d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(wide)).unwrap();
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(narrow)).unwrap();
    let l = lay(&d);
    let lines: Vec<(f32, f32, f32)> = l.pages[0]
        .items
        .iter()
        .filter_map(|i| if let Placed::Lines { x, y, para, .. } = i { Some((*x, *y, para.lines[0].left)) } else { None })
        .collect();
    let [(wx, wy, _), (nx, ny, _), (bx, by, bleft)] = lines.as_slice() else { panic!("{lines:?}") };
    // The wide table stands where the text is; the narrow one may not overlap it, so it goes
    // right below, keeping its 9pt from the text's left edge.
    assert!(*wy >= 72.0 && *wy < 80.0, "wide at {wy}");
    assert!(*ny > *wy + 10.0, "narrow at {ny}, below the wide table at {wy}");
    assert!((nx - wx - 9.0).abs() < 0.01, "narrow text at {nx}, wide text at {wx}");
    // The paragraph doesn't wait below them: it runs beside the narrow table, level with it.
    assert!((by - ny).abs() < 2.0, "body at {by}, narrow table text at {ny}");
    assert!(bx + bleft > nx + 200.0, "body text at {}, right of the narrow table", bx + bleft);
}

#[test]
fn a_floating_table_ignores_a_paragraphs_empty_lines() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::TableFloat;

    let mut doc = Document::from_text(&[""; 9].join("\n"));
    doc.body.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Visible\n\n\n", Default::default())));
    doc.body.extend((0..14).map(|_| wordcraft_doc::para_block(wordcraft_doc::Paragraph::new())));
    let mut table = Table::new(12, 1, 300.0);
    for (i, row) in table.rows.iter_mut().enumerate() {
        row.cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("Approval {i}"), Default::default()))];
    }
    table.props.float = Some(TableFloat { h_rel: Anchor::Margin, v_rel: Anchor::Margin, x: 100.0, y: 50.0, ..Default::default() });
    let at = doc.body.len();
    doc.insert_block(StoryRef::Body, &Path::top(at), wordcraft_doc::Block::Table(table)).unwrap();

    let layout = lay(&doc);
    let table_pages: Vec<usize> = layout
        .pages
        .iter()
        .enumerate()
        .flat_map(|(page, p)| {
            p.items.iter().filter_map(move |item| match item {
                Placed::Lines { story: StoryRef::Body, path, .. } if path.0.len() > 1 => Some(page),
                _ => None,
            })
        })
        .collect();
    assert!(!table_pages.is_empty() && table_pages.iter().all(|page| *page == 0), "table rows on pages {table_pages:?}");
}

#[test]
fn a_floating_table_taller_than_a_page_runs_across_pages() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::TableFloat;
    let mut d = Document::from_text("After the table.");
    let mut t = Table::new(80, 1, 300.0);
    for (i, r) in t.rows.iter_mut().enumerate() {
        r.cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("Row {i}"), Default::default()))];
    }
    t.props.float = Some(TableFloat { h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, x: -30.0, overlap: false, ..Default::default() });
    d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 2, "{} page(s)", l.pages.len());
    // Every row is drawn on a page, none past a page's bottom margin, and all from the table's
    // own left edge.
    let mut rows = 0;
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { x, y, story: StoryRef::Body, path, .. } = it
                && path.0.len() > 1
            {
                rows += 1;
                assert!(*y < 792.0 - 72.0, "row drawn at {y}, below the bottom margin");
                assert!(*x < 72.0, "row text at {x}, not from the table's edge 30pt left of the margin");
            }
        }
    }
    assert_eq!(rows, 80);
}

/// #534: a table placed on the page (`w:tblpPr w:vertAnchor="page"`) whose anchor comes after a
/// page full of text goes to the next page with it, like Word, instead of covering that text.
#[test]
fn a_page_anchored_floating_table_after_a_full_page_moves_to_the_next() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::TableFloat;
    let text: Vec<String> = (0..24).map(|i| format!("Paragraph {i} before the table.")).collect();
    let mut d = Document::from_text(&(text.join("\n") + "\nAfter the table."));
    let mut t = Table::new(15, 1, 300.0);
    for (i, r) in t.rows.iter_mut().enumerate() {
        r.cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("Row {i}"), Default::default()))];
    }
    t.props.float = Some(TableFloat { h_rel: Anchor::Page, v_rel: Anchor::Page, x: 70.0, y: 71.0, ..Default::default() });
    d.insert_block(StoryRef::Body, &Path::top(24), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let page_of = |want: &dyn Fn(&Path) -> bool| -> Vec<usize> {
        l.pages
            .iter()
            .enumerate()
            .flat_map(|(i, p)| p.items.iter().map(move |it| (i, it)))
            .filter_map(|(i, it)| if let Placed::Lines { story: StoryRef::Body, path, .. } = it { want(path).then_some(i) } else { None })
            .collect()
    };
    // The text before the table fills page 1; the table and the text after it are on page 2.
    assert_eq!(page_of(&|p| p.0 == [23]), vec![0]);
    let rows = page_of(&|p| p.0.first() == Some(&24) && p.0.len() > 1);
    assert!(rows.len() == 15 && rows.iter().all(|pg| *pg == 1), "table rows on pages {rows:?}");
    assert_eq!(page_of(&|p| p.0 == [25]), vec![1]);
}

/// #534: a paragraph that starts with a page break right where the page is full keeps the break
/// on that page: no blank page between the two tables around it.
#[test]
fn a_page_break_at_the_bottom_of_a_full_page_leaves_no_blank_page() {
    use wordcraft_doc::props::LineSpacing;
    let props = ParaProps { line_spacing: Some(LineSpacing::Exactly(24.0)), space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
    let para = |text: &str| {
        let mut p = wordcraft_doc::Paragraph::with_text(text, Default::default());
        p.props = props.clone();
        wordcraft_doc::para_block(p)
    };
    let mut d = Document::from_text("");
    // 27 lines of 24pt fill the 648pt of text on a Letter page.
    d.body = (0..27).map(|i| para(&format!("Line {i}"))).collect();
    d.body.push(para("\u{000C}Next page"));
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2, "pages: {}", l.pages.len());
    let last_on_2 = l.pages[1].items.iter().any(|it| matches!(it, Placed::Lines { path, .. } if path.0 == [27]));
    assert!(last_on_2, "the text after the break is on page 2");
}

/// #534: indents in characters (`w:firstLineChars`) are measured in the paragraph mark's font
/// size and win over the indent in points.
#[test]
fn character_indents_use_the_paragraph_mark_size() {
    let mut p = wordcraft_doc::Paragraph::with_text(&"字".repeat(80), Default::default());
    p.props.indent_first = Some(5.0);
    p.props.indent_first_chars = Some(2.0);
    p.props.indent_left_chars = Some(1.0);
    p.mark.size = Some(20.0);
    let mut d = Document::from_text("");
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = lay(&d);
    let para = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    assert!((para.lines[0].left - 60.0).abs() < 0.01, "first line at {}", para.lines[0].left);
    assert!((para.lines[1].left - 20.0).abs() < 0.01, "second line at {}", para.lines[1].left);
}

#[test]
fn no_line_break_right_after_a_slash() {
    // Word keeps "and/or" and web addresses whole on a line when they fit.
    let text = "word and/or https://example.org/keepers/log/winter ".repeat(40);
    let d = Document::from_text(&text);
    let l = lay(&d);
    let mut lines = 0;
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { para, l0, l1, .. } = it {
                for line in &para.lines[*l0..*l1] {
                    lines += 1;
                    assert!(!text[..line.start].ends_with('/'), "line starts after a slash: {:?}", &text[line.start..line.stop]);
                }
            }
        }
    }
    assert!(lines > 10, "{lines} lines");
}

#[test]
fn unequal_formats_lay_out_in_linear_time() {
    // Runs whose formats never compare equal (NaN spacing from a hostile file) each get a style;
    // finding a run's style must not compare it against every earlier one.
    let doc = |n: usize| {
        let mut p = wordcraft_doc::Paragraph::with_text("", Default::default());
        p.text = "ab ".repeat(n);
        p.runs = (0..n)
            .map(|i| wordcraft_doc::Run {
                len: 3,
                props: wordcraft_doc::CharProps { spacing: Some(f32::NAN), bold: Some(i % 2 == 0), ..Default::default() },
            })
            .collect();
        let mut d = Document::from_text("");
        d.body = vec![wordcraft_doc::para_block(p)];
        d
    };
    let (t1, tk, ratio) = layout_time_ratio(doc, 5_000, &LayoutOptions::default());
    eprintln!("t(n) {t1:.4} s, t(8n) {tk:.4} s, ratio {ratio:.1}");
    assert!(ratio < 24.0, "8x the runs took {ratio:.1}x the time ({t1:.4} s → {tk:.4} s): not linear");
}

#[test]
fn formats_past_65536_in_a_paragraph_keep_their_own_style() {
    // More distinct formats than a 16-bit index counts (the paragraph mark takes a style too): every
    // run past that limit still draws with its own link, deletion and size, not an earlier run's.
    let n = usize::from(u16::MAX) + 3;
    let props = |i: usize| wordcraft_doc::CharProps {
        link: Some(format!("#r{i}")),
        del: (i % 2 == 1).then_some(0),
        size: Some(if i.is_multiple_of(3) { 9.0 } else { 11.0 }),
        ..Default::default()
    };
    let mut p = wordcraft_doc::Paragraph::with_text("", Default::default());
    p.text = "ab ".repeat(n);
    p.runs = (0..n).map(|i| wordcraft_doc::Run { len: 3, props: props(i) }).collect();
    let mut d = Document::from_text("");
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = lay(&d);
    let Some(Placed::Lines { para, .. }) = l.pages.iter().flat_map(|pg| pg.items.iter()).find(|i| matches!(i, Placed::Lines { .. })) else {
        panic!("no lines")
    };
    assert!(para.styles.len() > n, "{} styles for {n} formats", para.styles.len());
    let mut last = 0;
    for c in &para.clusters {
        let i = c.start / 3;
        let rc = &para.styles[c.style as usize].rc;
        let want = props(i);
        assert_eq!((&rc.link, rc.del, rc.size), (&want.link, want.del, want.size.unwrap()), "run {i}");
        last = last.max(i);
    }
    assert_eq!(last, n - 1);
}

#[test]
fn hidden_float_in_a_table_cell_takes_no_room() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    // A square-wrapped shape anchored in hidden text inside a table cell, beside wrapping text.
    let mut d = Document::from_text("Body text.");
    let mut t = Table::new(1, 2, 468.0);
    let mut cp = wordcraft_doc::Paragraph::with_text(&"Cell words wrap around it. ".repeat(8), Default::default());
    let f = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, dist: 9.0, ..Default::default() };
    let hidden = wordcraft_doc::CharProps { hidden: Some(true), ..Default::default() };
    cp.insert_object(0, rect_shape(60.0, 40.0, f), &hidden).unwrap();
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(cp)];
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    // (shapes placed, the cell text's widest first-line indent)
    let run = |show_hidden: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden, ..Default::default() });
        let items = &l.pages[0].items;
        let left = items
            .iter()
            .filter_map(|i| if let Placed::Lines { para, .. } = i { para.lines.first().map(|l| l.left) } else { None })
            .fold(0.0f32, f32::max);
        (shapes(items).len(), left)
    };
    let (n, left) = run(true);
    assert!(n == 1 && left >= 60.0, "shown: placed and wrapped around ({n}, {left})");
    assert_eq!(run(false), (0, 0.0), "hidden: not placed, no wrap area");
}

#[test]
fn objects_left_out_through_the_table_style_formatting() {
    // The table style's character formatting sits under the runs' own, as the cell is laid out.
    let mut p = wordcraft_doc::Paragraph::with_text("ab", Default::default());
    p.insert_object(1, rect_shape(10.0, 10.0, Default::default()), &Default::default()).unwrap();
    let d = Document::from_text("");
    let hidden = wordcraft_doc::CharProps { hidden: Some(true), ..Default::default() };
    assert!(!left_out_objects(&d, &p, None, false, false)(0));
    assert!(left_out_objects(&d, &p, Some(&hidden), false, false)(0));
    assert!(!left_out_objects(&d, &p, Some(&hidden), true, false)(0), "hidden text shown");
}

#[test]
fn hidden_float_in_a_text_box_is_not_placed() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    // A shape anchored in hidden text inside a text box: no drawing, no hit area.
    let mut d = Document::from_text("Body text.");
    let id = text_box(&mut d, 0, "Box words.", 200.0, 120.0, Default::default());
    let f = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, ..Default::default() };
    let hidden = wordcraft_doc::CharProps { hidden: Some(true), ..Default::default() };
    let at = Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 };
    d.insert_object(&at, rect_shape(30.0, 20.0, f), &hidden).unwrap();
    let count = |show_hidden: bool| {
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden, ..Default::default() });
        let items = &l.pages[0].items;
        let areas = items.iter().filter(|i| matches!(i, Placed::Object { story: StoryRef::Part(_), .. })).count();
        (shapes(items).len(), areas)
    };
    let (shown, shown_areas) = count(true);
    let (hidden_n, hidden_areas) = count(false);
    assert_eq!((shown - hidden_n, shown_areas - hidden_areas), (1, 1), "shown {shown}/{shown_areas}, hidden {hidden_n}/{hidden_areas}");
}

#[test]
fn inline_graphic_reserves_its_height() {
    let mut d = Document::from_text("Chart\nAfter");
    let obj = InlineObject::Graphic { w: 216.0, h: 144.0, alt: String::new(), float: Default::default(), graphic: Arc::new(Default::default()) };
    d.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
    let l = lay(&d);
    let tops: Vec<f32> = l.pages[0].items.iter().filter_map(|i| if let Placed::Lines { y, .. } = i { Some(*y) } else { None }).collect();
    assert_eq!(tops.len(), 2);
    assert!(tops[1] - tops[0] >= 144.0, "the next paragraph starts below the chart: {tops:?}");
}

#[test]
fn graphic_items_draw_inside_the_object() {
    use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg, TextAlign};
    use wordcraft_doc::props::Rgb;
    let items = vec![
        GraphicItem::Shape {
            rect: [10.0, 5.0, 20.0, 20.0],
            kind: wordcraft_doc::para::ShapeKind::Rectangle,
            fill: Some(Rgb::BLACK),
            stroke: None,
            stroke_width: 0.0,
        },
        GraphicItem::Path {
            segs: vec![PathSeg::Move(1.0, 2.0), PathSeg::Line(30.0, 40.0), PathSeg::Close],
            fill: None,
            stroke: Some(Rgb::BLACK),
            stroke_width: 0.0,
        },
        GraphicItem::Text {
            rect: [0.0, 100.0, 216.0, 20.0],
            text: "Q1".into(),
            size: 10.0,
            color: Rgb::BLACK,
            bold: false,
            align: TextAlign::Center,
            font: None,
        },
    ];
    let mut d = Document::from_text("Chart");
    let graphic = Graphic { kind: GraphicKind::Chart, items, ..Default::default() };
    let obj = InlineObject::Graphic { w: 216.0, h: 144.0, alt: String::new(), float: Default::default(), graphic: Arc::new(graphic) };
    d.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
    let l = lay(&d);
    // The chart's draws sit inside its figure.
    let draws = flat(display::page_display(&d, &l.pages[0], &Default::default()));
    let shape = draws.iter().find_map(|x| if let display::Draw::Shape { rect, .. } = x { Some(*rect) } else { None }).expect("a shape");
    let first = draws.iter().find_map(|x| if let display::Draw::Path { segs, .. } = x { segs.first().copied() } else { None }).expect("a path");
    let PathSeg::Move(mx, my) = first else { panic!("{first:?}") };
    // Path points are page coordinates, moved with the object like the shape's rectangle.
    assert_eq!((mx - shape.x, my - shape.y), (1.0 - 10.0, 2.0 - 5.0));
    assert!(draws.iter().any(|x| matches!(x, display::Draw::Glyphs { text, .. } if text == "Q1")));
}

#[test]
fn graphic_text_in_a_degenerate_rectangle_is_not_drawn() {
    use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, TextAlign};
    use wordcraft_doc::props::Rgb;
    let text = |rect: [f32; 4], size: f32| GraphicItem::Text {
        rect,
        text: "Q1".into(),
        size,
        color: Rgb::BLACK,
        bold: false,
        align: TextAlign::Center,
        font: None,
    };
    let items = vec![text([f32::NAN, 0.0, 10.0, 10.0], 10.0), text([0.0, 0.0, f32::INFINITY, 10.0], 10.0), text([0.0, 0.0, 10.0, 10.0], f32::NAN)];
    let graphic = Graphic { kind: GraphicKind::Diagram, items, ..Default::default() };
    let obj = InlineObject::Graphic { w: 216.0, h: 144.0, alt: String::new(), float: Default::default(), graphic: Arc::new(graphic) };
    let mut d = Document::from_text("Chart");
    d.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
    let draws = flat(display::page_display(&d, &lay(&d).pages[0], &Default::default()));
    assert!(!draws.iter().any(|x| matches!(x, display::Draw::Glyphs { text, .. } if text == "Q1")));
}

/// The draws with figures opened up.
fn flat(draws: Vec<display::Draw>) -> Vec<display::Draw> {
    draws.into_iter().flat_map(|x| if let display::Draw::Figure { draws, .. } = x { draws } else { vec![x] }).collect()
}

#[test]
fn inline_chart_is_a_figure_with_its_alt_text() {
    use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg};
    use wordcraft_doc::props::Rgb;
    let items = vec![GraphicItem::Path {
        segs: vec![PathSeg::Move(1.0, 1.0), PathSeg::Line(2.0, 2.0)],
        fill: None,
        stroke: Some(Rgb::BLACK),
        stroke_width: 1.0,
    }];
    let graphic = Graphic { kind: GraphicKind::Chart, items, ..Default::default() };
    let obj = InlineObject::Graphic { w: 216.0, h: 144.0, alt: "Sales by quarter".into(), float: Default::default(), graphic: Arc::new(graphic) };
    let mut d = Document::from_text("Chart");
    d.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
    let draws = display::page_display(&d, &lay(&d).pages[0], &Default::default());
    let figure = draws.iter().find_map(|x| if let display::Draw::Figure { alt, draws, .. } = x { Some((alt.clone(), draws.len())) } else { None });
    assert_eq!(figure, Some(("Sales by quarter".to_string(), 1)));
}

#[test]
fn graphic_paths_drop_segments_that_cannot_be_drawn() {
    use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg};
    use wordcraft_doc::props::Rgb;
    // The line before the first move, the move with a NaN, the infinite line and the line after the
    // close are all dropped.
    let segs = vec![
        PathSeg::Line(5.0, 5.0),
        PathSeg::Move(f32::NAN, 1.0),
        PathSeg::Move(1.0, 2.0),
        PathSeg::Line(3.0, 4.0),
        PathSeg::Line(f32::INFINITY, 0.0),
        PathSeg::Close,
        PathSeg::Line(9.0, 9.0),
    ];
    let items = vec![GraphicItem::Path { segs, fill: None, stroke: Some(Rgb::BLACK), stroke_width: 1.0 }];
    let graphic = Graphic { kind: GraphicKind::Chart, items, ..Default::default() };
    let obj = InlineObject::Graphic { w: 216.0, h: 144.0, alt: String::new(), float: Default::default(), graphic: Arc::new(graphic) };
    let mut d = Document::from_text("Chart");
    d.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
    let draws = flat(display::page_display(&d, &lay(&d).pages[0], &Default::default()));
    let segs: Vec<PathSeg> =
        draws.iter().find_map(|x| if let display::Draw::Path { segs, .. } = x { Some(segs.clone()) } else { None }).expect("a path");
    assert!(matches!(segs.as_slice(), [PathSeg::Move(x, y), PathSeg::Line(..), PathSeg::Close] if x.is_finite() && y.is_finite()), "{segs:?}");
}

#[test]
fn graphic_items_scale_with_their_frame() {
    use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind};
    use wordcraft_doc::props::Rgb;
    // A chart drawn for 1000 x 500 pt in a 500 x 250 pt frame: its items and strokes scale by half.
    let items = vec![GraphicItem::Shape {
        rect: [0.0, 0.0, 1000.0, 500.0],
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        fill: None,
        stroke: Some(Rgb::BLACK),
        stroke_width: 4.0,
    }];
    let graphic = Graphic { kind: GraphicKind::Chart, items, w: 1000.0, h: 500.0, source: None, chart: None, smart_art: None };
    let obj = InlineObject::Graphic { w: 500.0, h: 250.0, alt: String::new(), float: Default::default(), graphic: Arc::new(graphic) };
    let mut d = Document::from_text("Chart");
    d.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
    let draws = flat(display::page_display(&d, &lay(&d).pages[0], &Default::default()));
    let (r, stroke) = draws
        .iter()
        .find_map(|x| if let display::Draw::Shape { rect, stroke_width, .. } = x { Some((*rect, *stroke_width)) } else { None })
        .expect("a shape");
    assert!((r.w - 500.0).abs() < 1e-3 && (r.h - 250.0).abs() < 1e-3, "{r:?}");
    assert!((stroke - 2.0).abs() < 1e-5, "stroke {stroke}");
}

#[test]
fn inline_picture_wider_than_a_text_box_is_fitted_to_it() {
    // Text boxes don't clip what runs past their sides, so a 400 x 100 pt picture in a 144 pt box
    // is fitted to the box's 129.6 pt inside its margins instead of spilling over the page.
    let mut d = Document::from_text("Before");
    let mut p = wordcraft_doc::Paragraph::with_text("", Default::default());
    p.insert_object(0, picture(400.0, 100.0, Default::default()), &Default::default()).unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::TextBox, vec![wordcraft_doc::para_block(p)]);
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 144.0,
        h: 72.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: Some(id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let l = lay(&d);
    let draws = flat(display::page_display(&d, &l.pages[0], &Default::default()));
    let r = draws
        .iter()
        .find_map(|x| match x {
            display::Draw::Shape { rect, kind: wordcraft_doc::para::ShapeKind::Rectangle, .. } => Some(*rect),
            _ => None,
        })
        .expect("the picture");
    assert!((r.w - 129.6).abs() < 0.1 && (r.h - 32.4).abs() < 0.1, "{r:?}");
}

#[test]
fn inline_picture_wider_than_the_text_keeps_its_size() {
    // A 1000 x 100 pt picture in 468 pt of text: Word draws it at its size, past the margin, and the
    // page edge clips it; the line is as tall as the picture (docxide-pdf's case179).
    let mut d = Document::from_text("");
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.insert_object(0, picture(1000.0, 100.0, Default::default()), &Default::default()).unwrap();
    }
    let l = lay(&d);
    let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
    let line = &para.lines[0];
    assert!((line.baseline - line.top - 100.0).abs() < 0.01, "the picture keeps its height: {line:?}");
    let draws = flat(display::page_display(&d, &l.pages[0], &Default::default()));
    let r = draws.iter().find_map(|x| if let display::Draw::Shape { rect, .. } = x { Some(*rect) } else { None }).expect("the picture");
    assert!((r.w - 1000.0).abs() < 0.01 && (r.h - 100.0).abs() < 0.01, "{r:?}");
}

#[test]
fn a_picture_too_wide_after_a_content_control_start_stays_on_the_first_line() {
    // The picture opening a picture content control (a logo in a text box a little too
    // narrow for it): the control's start marker before it takes no room, so the picture is
    // still the line's first item and stays there instead of leaving an empty line above it.
    let mut d = Document::from_text("");
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.insert_object(0, picture(500.0, 100.0, Default::default()), &Default::default()).unwrap();
        let start = InlineObject::ControlStart { control: Box::default() };
        p.insert_object(0, start, &Default::default()).unwrap();
    }
    let l = lay(&d);
    let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
    assert_eq!(para.lines.len(), 1, "{:?}", para.lines);
    let line = &para.lines[0];
    assert!((line.baseline - line.top - 100.0).abs() < 0.01, "the picture is on the first line: {line:?}");
}

/// The paragraph at `path` on page 0: its first line's x, its y and its line count.
fn para_at(l: &DocLayout, path: &[u32]) -> Vec<(f32, f32, usize)> {
    l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { x, y, para, path: p, .. } if p.0 == path => Some((*x + para.lines.first().map_or(0.0, |l| l.left), *y, para.lines.len())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_floating_table_in_a_cell_takes_no_room_and_the_cell_holds_it() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::TableFloat;
    let text = |s: &str| wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(s, Default::default()));
    let mut inner = Table::new(3, 1, 200.0);
    for (i, r) in inner.rows.iter_mut().enumerate() {
        r.cells[0].blocks = vec![text(&format!("Inner {i}"))];
    }
    inner.props.float = Some(TableFloat { h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, dist: [9.0, 0.0, 9.0, 0.0], ..Default::default() });
    let mut outer = Table::new(1, 1, 468.0);
    outer.rows[0].cells[0].blocks = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(inner)), text("Beside")];
    let mut d = Document::from_text("Below");
    d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(outer)).unwrap();
    let l = lay(&d);
    let (&[(ix, iy, _)], &[(_, last_y, _)], &[(bx, by, _)], &[(_, below_y, _)]) =
        (&para_at(&l, &[0, 0, 0, 0, 0, 0, 0])[..], &para_at(&l, &[0, 0, 0, 0, 2, 0, 0])[..], &para_at(&l, &[0, 0, 0, 1])[..], &para_at(&l, &[1])[..])
    else {
        panic!("lines missing")
    };
    // The paragraph after the floating table runs beside it, level with its top.
    assert!((by - iy).abs() < 2.0, "beside at {by}, inner table at {iy}");
    assert!(bx > ix + 200.0, "beside text at {bx}, inner table text at {ix}");
    // The cell grows to hold the whole floating table.
    assert!(below_y > last_y + 10.0, "text below the outer table at {below_y}, the inner table's last row at {last_y}");
}

#[test]
fn autofit_sizes_columns_by_their_content() {
    let text = |s: &str| wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(s, Default::default()));
    // No width anywhere: the columns come from their content.
    let mut t = Table::new(1, 2, 468.0);
    t.rows[0].cells[0].blocks = vec![text("Short")];
    t.rows[0].cells[1].blocks = vec![text(&"A long cell that needs most of the width. ".repeat(4))];
    t.rows[0].cells.iter_mut().for_each(|c| c.props.width = None);
    let mut d = Document::from_text("After");
    d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let (&[(sx, _, _)], &[(lx, _, _)]) = (&para_at(&l, &[0, 0, 0, 0])[..], &para_at(&l, &[0, 0, 1, 0])[..]) else { panic!("lines missing") };
    assert!(lx - sx < 80.0, "the short column is {}pt wide", lx - sx);
    // A grid column narrower than its longest word grows; the others give up the difference.
    let mut t = Table::new(1, 3, 468.0);
    t.grid = vec![400.0, 20.0, 48.0];
    t.rows[0].cells[1].blocks = vec![text("Unbreakable")];
    let mut d = Document::from_text("After");
    d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let [(_, _, n)] = para_at(&l, &[0, 0, 1, 0])[..] else { panic!("line missing") };
    assert_eq!(n, 1, "the word is broken over {n} lines");
}

#[test]
fn autofit_handles_spanning_cells_long_words_and_empty_tables() {
    let text = |s: &str| wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(s, Default::default()));
    let table_x = |t: Table, path: &[u32]| {
        let mut d = Document::from_text("After");
        d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(t)).unwrap();
        para_at(&lay(&d), path)
    };
    let automatic = |rows: usize, cols: usize| {
        let mut t = Table::new(rows, cols, 468.0);
        t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).for_each(|c| c.props.width = None);
        t
    };
    // A title cell spanning both columns needs its width shared between them: it stays on one line.
    let mut t = automatic(2, 2);
    t.rows[0].cells.truncate(1);
    t.rows[0].cells[0].props.span = 2;
    t.rows[0].cells[0].blocks = vec![text("A title that spans both columns")];
    t.rows[1].cells[0].blocks = vec![text("A")];
    t.rows[1].cells[1].blocks = vec![text("B")];
    let [(_, _, n)] = table_x(t, &[0, 0, 0, 0])[..] else { panic!("title missing") };
    assert_eq!(n, 1, "the title wraps over {n} lines");
    // An unbreakable string longer than the page doesn't widen the table past it.
    let mut t = automatic(1, 2);
    t.rows[0].cells[0].blocks = vec![text(&"x".repeat(300))];
    t.rows[0].cells[1].blocks = vec![text("Second")];
    let [(x, _, _)] = table_x(t, &[0, 0, 1, 0])[..] else { panic!("second cell missing") };
    assert!(x < 72.0 + 468.0, "the second column starts at {x}, past the margin");
    // A table of empty cells keeps its grid.
    let [(x, _, _)] = table_x(automatic(1, 2), &[0, 0, 1, 0])[..] else { panic!("second cell missing") };
    assert!(x > 72.0 + 200.0, "the second empty column starts at {x}");
    // Cells asking for a share of the table's width keep their grid too.
    let mut t = automatic(1, 2);
    for (c, s) in t.rows[0].cells.iter_mut().zip(["A", "B"]) {
        c.props.width_pct = Some(50.0);
        c.blocks = vec![text(s)];
    }
    let [(x, _, _)] = table_x(t, &[0, 0, 1, 0])[..] else { panic!("second cell missing") };
    assert!(x > 72.0 + 200.0, "the second 50% column starts at {x}");
}

#[test]
fn an_inherited_footer_sizes_its_percentage_table_by_its_own_section() {
    use wordcraft_doc::section::SectionProps;
    // Measured in Word: a 100%-wide footer table defined by a portrait section keeps that width on
    // a landscape page that inherits the footer, centred there; a landscape section naming the
    // same footer itself gets the landscape width.
    let table_width = |landscape_names_it: bool| {
        let mut d = Document::from_text("Portrait\nLandscape");
        let mut t = Table::new(1, 2, 100.0);
        t.props.width_pct = Some(100.0);
        let footer = d.add_part(wordcraft_doc::PartKind::Footer, vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t))]);
        let mut portrait = SectionProps::default();
        portrait.footers.default = Some(footer);
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(portrait.clone()));
        let mut landscape = portrait.clone();
        (landscape.page_w, landscape.page_h) = (portrait.page_h, portrait.page_w);
        if !landscape_names_it {
            landscape.footers.default = None;
        }
        d.last_section = landscape;
        let l = lay(&d);
        let cells: Vec<Rect> = l.pages[1].footer.iter().filter_map(|i| if let Placed::Cell { rect, .. } = i { Some(*rect) } else { None }).collect();
        let (x0, x1) = cells.iter().fold((f32::MAX, f32::MIN), |(a, b), r| (a.min(r.x), b.max(r.right())));
        (x1 - x0, portrait.text_width(), d.last_section.text_width())
    };
    let (inherited, portrait_w, _) = table_width(false);
    assert!((inherited - portrait_w).abs() < 1.0, "inherited: {inherited} wide, portrait text {portrait_w}");
    let (own, _, landscape_w) = table_width(true);
    assert!((own - landscape_w).abs() < 1.0, "own footer: {own} wide, landscape text {landscape_w}");
}

#[test]
fn a_row_with_less_room_than_its_minimum_height_moves_on_whole() {
    use wordcraft_doc::props::HeightRule;
    // Measured in Word: with 49 pt left on the page, a row of eight lines splits, but the same row
    // with an at-least height of 63.6 pt goes to the next page whole.
    let first_row_line_page = |min: Option<f32>| {
        let lines: Vec<String> = (1..=200).map(|i| format!("Filler {i}")).collect();
        let one_page = lay(&Document::from_text(&lines.join("\n"))).pages[0].items.iter().filter(|i| matches!(i, Placed::Lines { .. })).count();
        let mut t = Table::new(1, 1, 400.0);
        t.rows[0].props.height = min;
        t.rows[0].props.height_rule = HeightRule::AtLeast;
        t.rows[0].cells[0].blocks =
            (1..=8).map(|k| wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("Row line {k}"), Default::default()))).collect();
        // Fill the page, leaving two filler paragraphs' room (less than 63.6 pt) for the row.
        let mut d = Document::from_text(&lines[..one_page - 2].join("\n"));
        let end = d.body.len();
        d.insert_block(StoryRef::Body, &Path::top(end), wordcraft_doc::Block::Table(t)).unwrap();
        let l = lay(&d);
        l.pages.iter().position(|p| p.items.iter().any(|i| matches!(i, Placed::Lines { path, .. } if path.0.first() == Some(&(end as u32)))))
    };
    assert_eq!(first_row_line_page(None), Some(0), "a row without a minimum height splits");
    assert_eq!(first_row_line_page(Some(63.6)), Some(1), "a row with a minimum height that doesn't fit moves on");
}

#[test]
fn text_overflowing_an_exact_row_is_clipped_to_it() {
    use wordcraft_doc::props::HeightRule;
    // An exactly 12 pt row holding three lines: Word shows what fits and cuts the rest at the row's
    // edge, rather than drawing it over the rows below.
    let mut t = Table::new(2, 1, 200.0);
    t.rows[0].props.height = Some(12.0);
    t.rows[0].props.height_rule = HeightRule::Exact;
    t.rows[0].cells[0].blocks =
        ["one", "two", "three"].iter().map(|s| wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(s, Default::default()))).collect();
    t.rows[1].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("below", Default::default()))];
    let mut d = Document::from_text("After");
    d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(t)).unwrap();
    let draws = display::page_display(&d, &lay(&d).pages[0], &Default::default());
    let clipped = |want: &str| {
        draws.iter().any(|x| match x {
            display::Draw::Clip { items, .. } => {
                flat(items.clone()).iter().any(|i| matches!(i, display::Draw::Glyphs { text, .. } if text.contains(want)))
            }
            _ => false,
        })
    };
    assert!(clipped("three"), "the overflowing row's text is drawn inside a clip");
    assert!(!clipped("below"), "a row that fits isn't clipped");
}

#[test]
fn a_percentage_width_past_the_text_column_is_kept() {
    // `w:tblW w:type="pct" w:w="6564"` (131.28%) with the left margin as a negative indent: a
    // cover-page footer table that spans the page edge to edge.
    let mut t = Table::new(1, 2, 234.0);
    t.props.width_pct = Some(131.28);
    t.props.indent = Some(-72.0);
    for (c, s) in t.rows[0].cells.iter_mut().zip(["Left", "Right"]) {
        c.blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(s, Default::default()))];
    }
    let mut d = Document::from_text("After");
    d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(t)).unwrap();
    let [(x, _, _)] = para_at(&lay(&d), &[0, 0, 1, 0])[..] else { panic!("second cell missing") };
    // Two equal columns of 614.4 pt from x = 0: the second starts past the middle of the page.
    assert!(x > 307.0, "the second column starts at {x}");
}

#[test]
fn a_floating_table_in_the_footer_leaves_the_footer_where_it_is() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::TableFloat;
    let footer_y = |float: bool| {
        let mut d = Document::from_text("Body");
        let mut t = Table::new(1, 1, 100.0);
        t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Float", Default::default()))];
        if float {
            t.props.float = Some(TableFloat { h_rel: Anchor::Page, v_rel: Anchor::Page, y: 720.0, ..Default::default() });
        }
        let blocks = vec![
            wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Footer", Default::default())),
            std::sync::Arc::new(wordcraft_doc::Block::Table(t)),
        ];
        let id = d.add_part(wordcraft_doc::PartKind::Footer, blocks);
        d.last_section.footers.default = Some(id);
        let l = lay(&d);
        l.pages[0].footer.iter().find_map(|it| if let Placed::Lines { y, path, .. } = it { (path.0 == [0]).then_some(*y) } else { None }).unwrap()
    };
    // A floating table in the footer doesn't push the footer's text up.
    assert!(footer_y(true) >= footer_y(false) - 0.01, "footer text at {} with the float, {} without", footer_y(true), footer_y(false));
}

#[test]
fn a_split_row_moves_shading_below_the_cut_with_it() {
    use crate::table::{CellPart, RowLayout, split_row};
    let line_row = |y: f32| {
        let d = Document::from_text("Line");
        let l = lay(&d);
        let Some(Placed::Lines { story, path, para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })).cloned() else {
            panic!("no line")
        };
        Placed::Lines { story, path, para, l0: 0, l1: 1, x: 0.0, y, turn: TextDirection::Horizontal }
    };
    let fill = |y: f32, h: f32| Placed::Fill { rect: wordcraft_geom::Rect::new(0.0, y, 100.0, h), color: wordcraft_doc::props::Rgb(200, 0, 0) };
    // The row's own shading (from its top) and a nested cell's shading starting below the cut.
    let row = RowLayout {
        height: 200.0,
        items: vec![line_row(0.0), line_row(150.0), fill(0.0, 200.0), fill(150.0, 50.0)],
        bottom: 0.0,
        cells: vec![CellPart { items: 0..4, end: None }],
    };
    let (a, b) = split_row(&row, 100.0).expect("splits");
    let fills = |r: &RowLayout| {
        r.items.iter().filter_map(|i| if let Placed::Fill { rect, .. } = i { Some((rect.y, rect.h)) } else { None }).collect::<Vec<_>>()
    };
    assert_eq!(fills(&a), vec![(0.0, 100.0)], "only the row's shading above the cut stays");
    let moved = fills(&b);
    assert_eq!(moved.first().map(|f| f.0), Some(0.0), "the row's shading starts the continuation: {moved:?}");
    let nested = moved.get(1).copied().unwrap_or_default();
    assert!(nested.0 > 0.0 && (nested.1 - 50.0).abs() < 0.01, "the nested shading keeps its place and size: {moved:?}");
}

#[test]
fn kinsoku_keeps_a_manual_line_break_before_a_closing_bracket() {
    // #269: kinsoku forbids a line starting with 」, but a manual line break (Shift+Enter) right
    // before one still ends the line there.
    let text = "日本語\n」テスト";
    let mut d = Document::from_text("");
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))];
    let l = lay(&d);
    let mut lines = Vec::new();
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { para, l0, l1, .. } = it {
                lines.extend(para.lines[*l0..*l1].iter().map(|line| (line.start, line.end)));
            }
        }
    }
    let bracket = text.find('」').unwrap();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0].1, LineEnd::LineBreak, "{lines:?}");
    assert_eq!(lines[1].0, bracket, "the second line starts at the bracket: {lines:?}");
}

/// #332: a rotated floating shape keeps text clear of its rotated bounds (it reaches further at
/// the side), its frame stays put (rotation is about the centre) and it is drawn turned; hit
/// testing follows the turned shape, not its frame.
#[test]
fn rotated_float_wraps_around_its_rotated_bounds() {
    let text = "Words flow around the turned shape here. ".repeat(30);
    let place = |rot: f32| {
        let mut d = Document::from_text(&text);
        let float = wordcraft_doc::para::Float {
            wrap: wordcraft_doc::para::Wrap::Square,
            h_rel: wordcraft_doc::para::Anchor::Column,
            v_rel: wordcraft_doc::para::Anchor::Paragraph,
            x: 0.0,
            y: 40.0,
            rot,
            ..Default::default()
        };
        let shape = InlineObject::Shape {
            kind: wordcraft_doc::para::ShapeKind::Rectangle,
            w: 144.0,
            h: 40.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float,
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let p = &l.pages[0];
        let Some(Placed::Lines { para, .. }) = p.items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { panic!() };
        // The widest indent of a line beside the shape.
        let left = para.lines.iter().map(|l| l.left).fold(0.0f32, f32::max);
        let hit = l.object(&Pos::body(0, 0), 0).unwrap();
        let drawn = crate::display::page_display(&d, p, &Default::default()).iter().any(|d| matches!(d, crate::display::Draw::Rotated { .. }));
        (left, hit, drawn, l)
    };
    let (flat, frame, flat_drawn, _) = place(0.0);
    let (turned, hit, drawn, l) = place(30.0);
    // 144 × 40 turned 30° is 144.7 wide: 0.36 pt further out on each side, the centre kept.
    let bounds = hit.bounds();
    assert!((bounds.w - (144.0 * 30f32.to_radians().cos() + 40.0 * 0.5)).abs() < 0.01, "{bounds:?}");
    assert_eq!(hit.rect, frame.rect);
    assert!(turned > flat && turned >= bounds.right() - hit.origin.x, "{flat} → {turned} ({bounds:?})");
    assert!(drawn && !flat_drawn);
    // Its frame's top-right corner turns away from where it was: no longer on the shape.
    let r = hit.rect;
    assert!(l.object_at(0, r.right() - 2.0, r.y + 2.0, 0.0).is_none());
    assert!(l.object_at(0, r.x + r.w / 2.0, r.y + r.h / 2.0, 0.0).is_some());
}

/// #332: a group turns as a whole: each member's centre is carried round the group's centre and
/// the member drawn with the group's turn on top of its own; hit testing follows the turned
/// group. A shadow that doesn't rotate with its shape (`rotWithShape="0"`, Word's presets) keeps
/// its page direction inside the turn; one that does turns with the shape.
#[test]
fn rotated_group_turns_its_members_and_their_shadows() {
    use wordcraft_doc::effects::{Shadow, ShapeEffects};
    use wordcraft_doc::para::{Anchor, Float, GroupChild, ShapeKind, Wrap};
    let member = |rot_with_shape: bool, rot: f32| InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 60.0,
        h: 40.0,
        fill: Some(wordcraft_doc::props::Rgb(1, 2, 3)),
        stroke: None,
        stroke_width: 0.0,
        float: Float { rot, ..Default::default() },
        story: None,
        freeform: None,
        effects: ShapeEffects { shadow: Some(Shadow { angle: 0.0, rot_with_shape, ..Default::default() }), ..Default::default() },
        extra: Default::default(),
    };
    let float =
        Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 100.0, y: 100.0, rot: 90.0, ..Default::default() };
    let group = InlineObject::Group {
        w: 120.0,
        h: 40.0,
        float,
        ch_w: 120.0,
        ch_h: 40.0,
        children: vec![GroupChild { x: 0.0, y: 0.0, obj: member(false, 0.0) }, GroupChild { x: 60.0, y: 0.0, obj: member(true, 10.0) }],
        canvas: None,
    };
    let mut d = Document::from_text("Text under the group.");
    d.insert_object(&Pos::body(0, 0), group, &Default::default()).unwrap();
    let l = lay(&d);
    let p = &l.pages[0];
    let hit = l.object(&Pos::body(0, 0), 0).unwrap();
    let g = hit.rect;
    let (cx, cy) = (g.x + g.w / 2.0, g.y + g.h / 2.0);
    assert_eq!(hit.spin.deg, 90.0);
    // Turned a quarter clockwise, the left member is now above the centre, the right one below.
    let shapes: Vec<(Rect, Spin)> =
        p.items.iter().filter_map(|i| if let Placed::Shape { rect, spin, .. } = i { Some((*rect, *spin)) } else { None }).collect();
    assert_eq!(shapes.len(), 2, "{shapes:?}");
    let centre = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01;
    assert!(near(centre(shapes[0].0), (cx, cy - 30.0)), "{shapes:?} about ({cx}, {cy})");
    assert!(near(centre(shapes[1].0), (cx, cy + 30.0)), "{shapes:?}");
    assert_eq!((shapes[0].1.deg, shapes[1].1.deg), (90.0, 100.0));
    // Drawn turned: the page-fixed shadow still falls to the right on the page (270° in the
    // quarter-turned frame); the other turns with its shape (still 0° in its own frame).
    let shadows: Vec<f32> = crate::display::page_display(&d, p, &Default::default())
        .iter()
        .filter_map(|d| match d {
            crate::display::Draw::Rotated { items, .. } => items.iter().find_map(|i| match i {
                crate::display::Draw::Shape { effects, .. } => effects.shadow.map(|s| s.angle),
                _ => None,
            }),
            _ => None,
        })
        .collect();
    assert_eq!(shadows.len(), 2, "{shadows:?}");
    assert!((shadows[0] - 270.0).abs() < 0.01 && shadows[1].abs() < 0.01, "{shadows:?}");
    // Grabbed on the turned group: its frame's left end is empty now, above the centre isn't.
    assert!(l.object_at(0, g.x + 5.0, cy, 0.0).is_none());
    assert!(l.object_at(0, cx, cy - 50.0, 0.0).is_some());
}

/// The pieces of footnote `id` as (page, first line, end line, top).
fn note_pieces(l: &DocLayout, id: u32) -> Vec<(usize, usize, usize, f32)> {
    let mut v = Vec::new();
    for (pi, p) in l.pages.iter().enumerate() {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Part(s), l0, l1, y, .. } = it
                && *s == id
            {
                v.push((pi, *l0, *l1, *y));
            }
        }
    }
    v
}

/// Lengths of the horizontal rules on page `pi` below `y`.
fn rules_below(l: &DocLayout, pi: usize, y: f32) -> Vec<f32> {
    l.pages[pi]
        .items
        .iter()
        .filter_map(|it| if let Placed::Rule { x0, y0, x1, y1, .. } = it { (*y0 == *y1 && *y0 > y).then_some(x1 - x0) } else { None })
        .collect()
}

#[test]
fn long_footnotes_continue_on_the_next_page() {
    let mut d = Document::from_text(&"Body text line.\n".repeat(16));
    let long = footnote(&mut d, &Pos::body(4, 4), &"A long footnote that runs on and on. ".repeat(100));
    let own = footnote(&mut d, &Pos::body(14, 4), "The second page's own note.");
    let l = lay(&d);
    let refs = l.caret(&Pos::body(4, 4)).unwrap();
    assert_eq!(refs.page, 0, "the reference stays where it was");
    let pieces = note_pieces(&l, long);
    assert!(pieces.len() >= 2, "{pieces:?}");
    let (p0, p1) = (&pieces[0], &pieces[1]);
    assert_eq!((p0.0, p1.0), (0, 1), "split over pages 1 and 2: {pieces:?}");
    assert_eq!(p0.1, 0);
    assert_eq!(p0.2, p1.1, "continues at the next line");
    // Every line of the note is placed once.
    let lines = pieces.iter().map(|p| p.2 - p.1).sum::<usize>();
    let para = l.pages[0]
        .items
        .iter()
        .find_map(|it| if let Placed::Lines { story: StoryRef::Part(s), para, .. } = it { (*s == long).then(|| para.lines.len()) } else { None })
        .unwrap();
    assert_eq!(lines, para);
    // Page 1: the note fills the space under the text, which ends above it.
    let body_bottom = |pi: usize| {
        l.pages[pi]
            .items
            .iter()
            .filter_map(
                |it| if let Placed::Lines { story: StoryRef::Body, y, para, l0, l1, .. } = it { item_bottom(*y, para, *l0, *l1) } else { None },
            )
            .fold(0.0f32, f32::max)
    };
    assert!(body_bottom(0) <= p0.3, "{} {}", body_bottom(0), p0.3);
    assert_eq!(rules_below(&l, 0, body_bottom(0)), vec![144.0], "a normal separator on page 1");
    // Page 2: the continuation separator spans the text width, the rest of the long note comes
    // first and the page's own note follows it.
    assert!(body_bottom(1) <= p1.3, "{} {}", body_bottom(1), p1.3);
    assert_eq!(rules_below(&l, 1, body_bottom(1)), vec![468.0], "continuation separator");
    let own_ref = l.caret(&Pos::body(14, 4)).unwrap();
    let own_note = l.caret(&d.start_of(StoryRef::Part(own))).unwrap();
    assert_eq!((own_ref.page, own_note.page), (1, 1));
    assert!(own_note.top > p1.3, "own note after the continued one: {own_note:?} {p1:?}");
}

#[test]
fn short_footnotes_stay_whole() {
    let mut d = Document::from_text(&"Body text line.\n".repeat(60));
    let id = footnote(&mut d, &Pos::body(10, 4), "A short note, two lines long. ".repeat(4).as_str());
    let l = lay(&d);
    let pieces = note_pieces(&l, id);
    assert_eq!(pieces.len(), 1, "{pieces:?}");
    assert_eq!(pieces[0].0, l.caret(&Pos::body(10, 4)).unwrap().page);
}

#[test]
fn footnote_longer_than_pages_ends() {
    // A note several pages long after a one-line body: it continues on pages of its own.
    let mut d = Document::from_text("Body");
    let id = footnote(&mut d, &Pos::body(0, 4), &"Words in a very long note. ".repeat(1500));
    let l = lay(&d);
    let pieces = note_pieces(&l, id);
    assert!(pieces.len() >= 3 && l.pages.len() == pieces.len(), "{} pages, {pieces:?}", l.pages.len());
    assert!(pieces.windows(2).all(|w| w[0].2 == w[1].1 && w[1].0 == w[0].0 + 1), "{pieces:?}");
}

/// A comment by `author` saying `text` on body paragraph `para`, bytes `a..b`; its id.
fn comment_on(d: &mut Document, para: usize, a: usize, b: usize, author: &str, text: &str, parent: Option<u32>) -> u32 {
    let part = d.add_part(wordcraft_doc::PartKind::Comment, vec![wordcraft_doc::para_block(Paragraph::with_text(text, CharProps::default()))]);
    let id = d.comments.keys().next_back().map_or(0, |k| k + 1);
    d.comments.insert(id, wordcraft_doc::Comment { author: author.into(), part, parent, ..Default::default() });
    if parent.is_none() {
        let p = CharProps::default();
        d.insert_object(&Pos::body(para, b), InlineObject::CommentEnd { id }, &p).unwrap();
        d.insert_object(&Pos::body(para, a), InlineObject::CommentStart { id }, &p).unwrap();
    }
    id
}

fn balloon_layout(d: &Document) -> DocLayout {
    layout(d, &mut LayoutCache::new(), &LayoutOptions { comment_balloons: true, ..Default::default() })
}

/// The comment text lines placed on page `pi` (story, top-left).
fn comment_lines(l: &DocLayout, pi: usize) -> Vec<(StoryRef, f32, f32)> {
    l.pages[pi]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: s @ StoryRef::Part(_), x, y, .. } => Some((*s, *x, *y)),
            _ => None,
        })
        .collect()
}

/// Print Markup (#589): each page grows by the markup area, and the comment, its author and its
/// reply are laid out there beside the line they comment on.
#[test]
fn comment_balloons_widen_pages_beside_their_anchors() {
    let mut d = Document::from_text("Intro line.\nThe quarterly figures look strong.\nClosing line.");
    let id = comment_on(&mut d, 1, 4, 13, "Ada Reviewer", "Check the totals", None);
    comment_on(&mut d, 0, 0, 0, "Ben Editor", "Totals checked", Some(id));
    let plain = lay(&d);
    let l = balloon_layout(&d);
    assert_eq!(l.pages.len(), plain.pages.len());
    let (w, pw) = (l.pages[0].w, plain.pages[0].w);
    assert_eq!(l.pages[0].markup, balloons::MARKUP_AREA);
    assert!((w - pw - balloons::MARKUP_AREA).abs() < 0.01, "{w} vs {pw}");
    assert_eq!(plain.pages[0].markup, 0.0);
    // Author + text for the comment, then for the reply: four lines, all in the markup area.
    let lines = comment_lines(&l, 0);
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines.iter().all(|(_, x, _)| *x > pw && *x < w), "{lines:?}");
    assert!(lines.windows(2).all(|p| p[0].2 < p[1].2), "stacked top to bottom: {lines:?}");
    // The card sits by the commented line.
    let anchor = l.caret(&Pos::body(1, 5)).unwrap();
    assert!((lines[0].2 - anchor.top).abs() < 30.0, "{lines:?} vs {anchor:?}");
    // The body is laid out exactly as without balloons.
    assert_eq!(lines_of(&plain), lines_of(&l) - 4);
    // The leader starts at the anchor.
    assert!(l.pages[0].items.iter().any(|it| matches!(it, Placed::Rule { x0, x1, .. } if (*x0 - anchor.x).abs() < 0.5 && (*x1 - pw).abs() < 0.5)));
}

#[test]
fn no_balloons_without_anchored_comments() {
    let d = Document::from_text("No comments here.");
    let l = balloon_layout(&d);
    assert_eq!(l.pages[0].markup, 0.0);
    assert_eq!(l.pages[0].w, lay(&d).pages[0].w);
    // A web view never gets a markup area.
    let mut d = Document::from_text("Hello there.");
    comment_on(&mut d, 0, 0, 5, "A", "x", None);
    let opts = LayoutOptions { view: ViewMode::Web, web_width: 500.0, comment_balloons: true, ..Default::default() };
    let l = layout(&d, &mut LayoutCache::new(), &opts);
    assert_eq!(l.pages[0].markup, 0.0);
}

/// Hostile comments: parent cycles, a missing text part, and far more comments than fit beside
/// one page. Nothing panics or hangs, and the cards stay on the page.
#[test]
fn comment_balloons_survive_hostile_comments() {
    let mut d = Document::from_text(&"Commented words here. ".repeat(20));
    let mut ids = Vec::new();
    for i in 0..120 {
        ids.push(comment_on(&mut d, 0, 0, 9, &format!("Author {i}"), &"A long remark that wraps over lines. ".repeat(4), None));
    }
    // A reply cycle and a comment whose text part is gone.
    let a = comment_on(&mut d, 0, 0, 0, "R", "one", Some(ids[0]));
    let b = comment_on(&mut d, 0, 0, 0, "R", "two", Some(a));
    d.comments.get_mut(&a).unwrap().parent = Some(b);
    d.comments.get_mut(&ids[1]).unwrap().parent = Some(ids[1]);
    d.comments.get_mut(&ids[2]).unwrap().part = 9_999_999;
    let l = balloon_layout(&d);
    let page = &l.pages[0];
    for (_, _, y) in comment_lines(&l, 0) {
        assert!(y.is_finite() && y >= 0.0 && y <= page.h, "{y}");
    }
    for it in &page.items {
        if let Placed::Fill { rect, .. } = it {
            assert!(rect.y >= 0.0 && rect.bottom() <= page.h + 0.5, "{rect:?}");
        }
    }
}

/// The first paragraph's lines on page one: (top, bottom, left, right), relative to its column.
fn line_spans(l: &DocLayout) -> Vec<(f32, f32, f32, f32)> {
    let Some(Placed::Lines { para, .. }) = l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { .. })) else { return Vec::new() };
    para.lines.iter().map(|ln| (ln.top, ln.top + ln.height, ln.left, ln.right)).collect()
}

/// A shape floating at the column's top-left with `wrap`, beside a long paragraph.
fn wrapped_shape_doc(kind: wordcraft_doc::para::ShapeKind, w: f32, h: f32, float: wordcraft_doc::para::Float) -> Document {
    let mut d = Document::from_text(&"Words flow around the shape here. ".repeat(40));
    let float =
        wordcraft_doc::para::Float { h_rel: wordcraft_doc::para::Anchor::Column, v_rel: wordcraft_doc::para::Anchor::Paragraph, dist: 9.0, ..float };
    let shape = InlineObject::Shape {
        kind,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    d
}

#[test]
fn tight_wrap_follows_an_ellipse() {
    use wordcraft_doc::para::{Float, ShapeKind, Wrap};
    let spans = |wrap| line_spans(&lay(&wrapped_shape_doc(ShapeKind::Ellipse, 160.0, 160.0, Float { wrap, ..Default::default() })));
    let (square, tight, through) = (spans(Wrap::Square), spans(Wrap::Tight), spans(Wrap::Through));
    // Square: every line beside the shape starts past its frame and distance from text.
    assert!(square.iter().filter(|l| l.0 < 160.0).all(|l| l.2 >= 169.0 - 0.01), "{square:?}");
    // Tight: no line runs into the ellipse (a 160pt circle at 80, 80) or its distance from text.
    // Where a line from `top` to `bottom` meets the circle plus that distance: (left, right) edges.
    let contour = |top: f32, bottom: f32| {
        let y = if bottom < 80.0 {
            80.0 - bottom
        } else if top > 80.0 {
            top - 80.0
        } else {
            0.0
        };
        let half = (80.0f32 * 80.0 - y.min(80.0).powi(2)).sqrt();
        (80.0 - half - 9.0, 80.0 + half + 9.0)
    };
    for &(top, bottom, left, right) in tight.iter().filter(|l| l.0 < 160.0) {
        let (a, b) = contour(top, bottom);
        assert!(right <= a + 1.5 || left >= b - 1.5, "line {top}..{bottom} at {left}..{right} runs into {a}..{b}: {tight:?}");
    }
    // Lines near the ellipse's top and bottom start at the curve, well inside its frame; beside
    // the middle they don't. Measured against each line's own contour edge, so the check holds
    // whatever line height the test machine's fonts give.
    let line = |pred: &dyn Fn(f32) -> bool| *tight.iter().find(|l| pred(l.0) && l.2 > 60.0).unwrap();
    for (what, l) in [("first line", line(&|top| top < 1.0)), ("near the bottom", line(&|top| top > 140.0 && top < 160.0))] {
        let edge = contour(l.0, l.1).1;
        assert!(l.2 <= edge + 3.0 && l.2 < 169.0 - 10.0, "{what} hugs the curve at {edge}: {l:?} in {tight:?}");
    }
    assert!(line(&|top| top > 65.0 && top < 80.0).2 >= 160.0, "beside the widest part: {tight:?}");
    // A convex shape has no gaps for Through to use: it wraps as Tight.
    assert_eq!(through, tight);
}

#[test]
fn contour_wrap_turns_with_the_object() {
    use wordcraft_doc::para::{Float, ShapeKind, Wrap};
    use wordcraft_doc::wrap::WrapPolygon;
    // A wrap polygon covering the left half of a 200 × 100 frame.
    let poly = WrapPolygon::new(true, [(0, 0), (10_800, 0), (10_800, 21_600), (0, 21_600)]).map(Arc::new);
    // Where text goes on the first row: (left, right) of each line there.
    let first_row = |wrap: Wrap, rot: f32, flip_h: bool| {
        let f = Float { wrap, rot, flip_h, wrap_polygon: poly.clone(), ..Default::default() };
        let l = line_spans(&lay(&wrapped_shape_doc(ShapeKind::Rectangle, 200.0, 100.0, f)));
        l.iter().filter(|s| s.0 < 1.0).map(|s| (s.2.round(), s.3.round())).collect::<Vec<_>>()
    };
    // Unturned, text comes up to the polygon (half the frame) plus the distance from text.
    assert_eq!(first_row(Wrap::Tight, 0.0, false)[0].0, 109.0);
    // Turned half a turn or mirrored, the polygon covers the frame's right half: text goes to
    // its left and past the frame.
    for (rot, flip) in [(180.0, false), (0.0, true)] {
        let row = first_row(Wrap::Tight, rot, flip);
        assert_eq!((row[0].0, row[0].1, row[1].0), (0.0, 91.0, 209.0), "{rot} {flip}");
    }
    // Square wrapping ignores the polygon.
    assert_eq!(first_row(Wrap::Square, 0.0, false)[0].0, 209.0);
}

/// Enclosed characters (#297, an `EQ \o\ac` field): the shape is drawn as a vector around the text,
/// whatever the font has; "shrink text" draws the text smaller inside a one-em shape, "enlarge
/// symbol" a bigger shape around text at its own size. Plain `\o` overstrikes its arguments.
#[test]
fn enclosed_characters_draw_a_shape_around_the_text() {
    use wordcraft_doc::eq::{EncloseShape, EncloseStyle, Enclosure};
    let draw = |shape: EncloseShape, style: EncloseStyle| {
        let mut d = Document::from_text("a  b");
        let e = Enclosure { shape, style, text: "A".into() };
        d.insert_object(
            &Pos::body(0, 2),
            InlineObject::Field { instr: e.instr(), result: "A".into(), locked: false, dirty: false, code: false, form: String::new() },
            &Default::default(),
        )
        .unwrap();
        let l = lay(&d);
        let items = display::page_display(&d, &l.pages[0], &Default::default());
        let shape = items
            .iter()
            .find_map(|i| if let display::Draw::Shape { rect, kind, stroke, .. } = i { Some((*rect, *kind, stroke.is_some())) } else { None });
        // Each glyph's character, size and x.
        let mut chars = Vec::new();
        for i in &items {
            if let display::Draw::Glyphs { text, size, glyphs, ranges, .. } = i {
                for (g, r) in glyphs.iter().zip(ranges) {
                    chars.push((text.get(r.clone()).unwrap_or("").to_string(), *size, g.1));
                }
            }
        }
        let at = |c: &str| chars.iter().find(|x| x.0 == c).map(|x| (x.1, x.2)).unwrap_or_else(|| panic!("{c}: {items:?}"));
        (shape.expect("a shape"), at("A"), at("b"))
    };
    let ((rect, kind, stroked), (size, gx), (base, bx)) = draw(EncloseShape::Circle, EncloseStyle::Shrink);
    assert_eq!(kind, wordcraft_doc::para::ShapeKind::Ellipse);
    assert!(stroked);
    assert!(size < base * 0.8, "the text shrinks: {size} vs {base}");
    assert!((rect.h - base * 1.05).abs() < 0.1 && (rect.w - rect.h).abs() < 0.1, "{rect:?}");
    assert!(gx > rect.x && gx < rect.x + rect.w, "the text is inside: {gx} {rect:?}");
    assert!(bx >= rect.x + rect.w, "the next text follows the shape: {bx} {rect:?}");
    let ((rect, kind, _), (size, _), (base, _)) = draw(EncloseShape::Diamond, EncloseStyle::Enlarge);
    assert_eq!(kind, wordcraft_doc::para::ShapeKind::Diamond);
    assert!((size - base).abs() < 0.01 && rect.h > base * 1.5, "the shape grows: {rect:?}");

    // A general overstrike: both arguments at the same place, no shape.
    let mut d = Document::from_text("x");
    d.insert_object(
        &Pos::body(0, 1),
        InlineObject::Field { instr: "eq \\o(=,/)".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &Default::default(),
    )
    .unwrap();
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &Default::default());
    assert!(!items.iter().any(|i| matches!(i, display::Draw::Shape { .. })));
    assert!(items.iter().any(|i| matches!(i, display::Draw::Glyphs { text, .. } if text.contains("=/"))), "{items:?}");
}

#[test]
fn ruby_text_sits_over_its_base_and_the_line_grows_to_fit() {
    use wordcraft_doc::para::RubyAlign;
    let plain = Document::from_text("Plain words here");
    let mut d = Document::from_text("Plain  here");
    let ruby = InlineObject::Ruby {
        base: "words".into(),
        ruby: "a much longer reading".into(),
        align: RubyAlign::Center,
        size: 6.0,
        raise: 12.0,
        base_size: 12.0,
        lang: String::new(),
        props: Default::default(),
    };
    d.insert_object(&Pos::body(0, 6), ruby, &Default::default()).unwrap();
    let height = |l: &DocLayout| {
        l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { para.lines.first().map(|x| x.height) } else { None }).unwrap()
    };
    let (lp, lr) = (lay(&plain), lay(&d));
    assert!(height(&lr) > height(&lp) + 3.0, "ruby line {} vs plain {}", height(&lr), height(&lp));
    let items = display::page_display(&d, &lr.pages[0], &Default::default());
    let glyphs_of = |want: &str| {
        items.iter().find_map(|i| match i {
            display::Draw::Glyphs { text, glyphs, size, .. } if text.contains(want) => Some((*size, glyphs.clone())),
            _ => None,
        })
    };
    let (_, line) = glyphs_of("Plain words").expect("base text drawn");
    let (size, over) = glyphs_of("a much longer reading").expect("ruby text drawn");
    let w = line[6]; // "Plain " then the base's first letter
    // Smaller, 12 pt above the base text's baseline, and (being wider) centred over it: it starts
    // at the base's slot, the base letters move in.
    assert_eq!(size, 6.0);
    assert!((w.2 - over[0].2 - 12.0).abs() < 0.01, "{w:?} {over:?}");
    let slot = lr.caret(&Pos::body(0, 6)).unwrap().x;
    assert!((over[0].1 - slot).abs() < 0.5 && w.1 > slot + 1.0, "{slot} {w:?} {over:?}");
    // The plain text reads the base, and the caret steps over the ruby as one character.
    assert_eq!(d.body[0].as_para().unwrap().plain_text(), "Plain words here");
    let (a, b) = (lr.caret(&Pos::body(0, 6)).unwrap(), lr.caret(&Pos::body(0, 9)).unwrap());
    assert!(b.x - a.x > 30.0, "{a:?} {b:?}");
}

/// #354: `wrapText`. Left: text only to the left of a centred object (Square or Tight); Right:
/// only to its right; Largest: only on its wider side.
#[test]
fn wrap_text_keeps_text_on_one_side() {
    use wordcraft_doc::para::{Float, ShapeKind, Wrap, WrapText};
    // A 100pt shape at `x` in the 468pt column; the lines beside it.
    let beside = |kind, wrap, x: f32, side| {
        let f = Float { wrap, wrap_text: side, x, ..Default::default() };
        line_spans(&lay(&wrapped_shape_doc(kind, 100.0, 100.0, f))).into_iter().filter(|l| l.0 < 100.0).collect::<Vec<_>>()
    };
    for (kind, wrap) in [(ShapeKind::Rectangle, Wrap::Square), (ShapeKind::Ellipse, Wrap::Tight)] {
        let both = beside(kind, wrap, 184.0, WrapText::BothSides);
        assert!(both.iter().any(|l| l.2 < 1.0) && both.iter().any(|l| l.2 > 200.0), "{both:?}");
        let left = beside(kind, wrap, 184.0, WrapText::Left);
        // Nothing right of the object (Tight: lines come up to the outline, short of its middle).
        let mid = if wrap == Wrap::Square { 175.0 } else { 234.0 };
        assert!(!left.is_empty() && left.iter().all(|l| l.2 < 1.0 && l.3 <= mid + 0.01), "{wrap:?}: {left:?}");
        let right = beside(kind, wrap, 184.0, WrapText::Right);
        let mid = if wrap == Wrap::Square { 293.0 } else { 234.0 };
        assert!(!right.is_empty() && right.iter().all(|l| l.2 >= mid - 0.01), "{wrap:?}: {right:?}");
    }
    // Nearer the left edge the right side is wider, nearer the right edge the left.
    let near_left = beside(ShapeKind::Rectangle, Wrap::Square, 100.0, WrapText::Largest);
    assert!(!near_left.is_empty() && near_left.iter().all(|l| l.2 >= 209.0 - 0.01), "{near_left:?}");
    let near_right = beside(ShapeKind::Rectangle, Wrap::Square, 268.0, WrapText::Largest);
    assert!(!near_right.is_empty() && near_right.iter().all(|l| l.2 < 1.0 && l.3 <= 259.0 + 0.01), "{near_right:?}");
}

#[test]
fn document_note_separators_and_continuation_notice() {
    let mut d = Document::from_text(&"Body text line.\n".repeat(16));
    let long = footnote(&mut d, &Pos::body(4, 4), &"A long footnote that runs on and on. ".repeat(100));
    let sep = |cont: bool| {
        let mut p = wordcraft_doc::Paragraph::new();
        p.props.space_after = Some(0.0);
        p.insert_object(0, InlineObject::note_separator(cont), &Default::default()).unwrap();
        Some(vec![wordcraft_doc::para_block(p)])
    };
    let mut notice = wordcraft_doc::Paragraph::with_text("(continued)", Default::default());
    notice.props.space_after = Some(0.0);
    d.footnote_separators = wordcraft_doc::NoteSeparators {
        separator: sep(false),
        continuation_separator: sep(true),
        continuation_notice: Some(vec![wordcraft_doc::para_block(notice)]),
    };
    let l = lay(&d);
    let pieces = note_pieces(&l, long);
    assert!(pieces.len() >= 2 && (pieces[0].0, pieces[1].0) == (0, 1), "{pieces:?}");
    // The document's separators are paragraphs: their lines are drawn across the separator's
    // place, short on page 1 and across the text on page 2.
    let decor = |pi: usize| -> Vec<(f32, f32)> {
        l.pages[pi]
            .items
            .iter()
            .filter_map(|it| match it {
                Placed::Lines { story, y, para, l0, l1, .. } if *story == DECOR_STORY => Some((*y, item_bottom(*y, para, *l0, *l1).unwrap())),
                _ => None,
            })
            .collect()
    };
    let (d0, d1) = (decor(0), decor(1));
    assert_eq!((d0.len(), d1.len()), (2, 1), "separator and notice on page 1, continuation separator on page 2");
    assert_eq!(rules_below(&l, 0, 0.0), vec![144.0]);
    assert_eq!(rules_below(&l, 1, 0.0), vec![468.0]);
    // The notice sits right under the part of the note on page 1, inside the page.
    let note_bottom = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), y, para, l0, l1, .. } if *s == long => item_bottom(*y, para, *l0, *l1),
            _ => None,
        })
        .fold(0.0f32, f32::max);
    let (ny, nb) = d0[1];
    assert!((ny - note_bottom).abs() < 0.01, "notice at {ny}, note ends at {note_bottom}");
    assert!(nb <= 720.01, "notice inside the page: {nb}");
    // The separators and notice aren't text anyone clicks into.
    assert_ne!(l.story_at(0, 80.0, ny + 2.0), Some(DECOR_STORY));
}

/// #344 with #332: a Drawing Canvas turns as a whole, like a group: its background and outline,
/// then its members clipped to its frame, all drawn inside its turn (the clip in the unturned
/// frame, so the clip turns too); floating or inline. Hit testing follows the turned canvas.
#[test]
fn rotated_canvas_draws_its_clipped_members_inside_the_turn() {
    use crate::display::Draw;
    use wordcraft_doc::para::{Anchor, CanvasStyle, Float, GroupChild, ShapeKind, Wrap};
    let member = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 60.0,
        h: 40.0,
        fill: Some(wordcraft_doc::props::Rgb(1, 2, 3)),
        stroke: None,
        stroke_width: 0.0,
        float: Float { rot: 10.0, ..Default::default() },
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let canvas = |float: Float| InlineObject::Group {
        w: 200.0,
        h: 100.0,
        float,
        ch_w: 200.0,
        ch_h: 100.0,
        children: vec![GroupChild { x: 20.0, y: 30.0, obj: member.clone() }],
        canvas: Some(CanvasStyle { fill: Some(wordcraft_doc::props::Rgb(0xF0, 0xF4, 0xFF)), stroke: None, stroke_width: 0.0 }),
    };
    // Turned draws: (turn, background rect, clip rect, member rect and its own turn).
    let turned = |draws: &[Draw]| -> (Vec<f32>, Vec<Rect>, Vec<(Rect, Vec<(Rect, f32)>)>) {
        let (mut turns, mut bgs, mut clips) = (Vec::new(), Vec::new(), Vec::new());
        for d in draws {
            let Draw::Rotated { spin, items, .. } = d else { continue };
            turns.push(spin.deg);
            for i in items {
                match i {
                    Draw::Shape { rect, fill: Some(c), .. } if c.0 == 0xF0 => bgs.push(*rect),
                    Draw::Clip { rect, items } => {
                        let members = items
                            .iter()
                            .map(|m| match m {
                                Draw::Rotated { spin, items, .. } => match items.first() {
                                    Some(Draw::Shape { rect, .. }) => (*rect, spin.deg),
                                    o => panic!("{o:?}"),
                                },
                                o => panic!("{o:?}"),
                            })
                            .collect();
                        clips.push((*rect, members));
                    }
                    _ => {}
                }
            }
        }
        (turns, bgs, clips)
    };
    for floating in [true, false] {
        let float = if floating {
            Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 100.0, y: 100.0, rot: 90.0, ..Default::default() }
        } else {
            Float { rot: 90.0, ..Default::default() }
        };
        let mut d = Document::from_text("Text by the canvas.");
        d.insert_object(&Pos::body(0, 0), canvas(float), &Default::default()).unwrap();
        let l = lay(&d);
        let p = &l.pages[0];
        let hit = l.object(&Pos::body(0, 0), 0).unwrap();
        assert_eq!(hit.spin.deg, 90.0);
        let f = hit.rect;
        let (turns, bgs, clips) = turned(&crate::display::page_display(&d, p, &Default::default()));
        assert!(turns.contains(&90.0), "floating {floating}: {turns:?}");
        // Background and clip are the canvas's unturned frame; the member sits in it, unturned
        // but for its own 10°.
        assert_eq!(bgs.len(), 1, "floating {floating}");
        assert!((bgs[0].x - f.x).abs() < 0.01 && (bgs[0].w - 200.0).abs() < 0.01, "{bgs:?} vs {f:?}");
        assert_eq!(clips.len(), 1, "floating {floating}");
        let (clip, members) = &clips[0];
        assert!((clip.x - bgs[0].x).abs() < 0.01 && (clip.y - bgs[0].y).abs() < 0.01 && (clip.h - 100.0).abs() < 0.01, "{clip:?}");
        assert_eq!(members.len(), 1, "{members:?}");
        let (m, own) = members[0];
        assert!((m.x - (clip.x + 20.0)).abs() < 0.01 && (m.y - (clip.y + 30.0)).abs() < 0.01, "{m:?} in {clip:?}");
        assert_eq!(own, 10.0);
        if floating {
            // Turned a quarter, the canvas's frame's left end is empty; above its centre isn't.
            let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
            assert!(l.object_at(0, f.x + 5.0, cy, 0.0).is_none());
            assert!(l.object_at(0, cx, cy - 80.0, 0.0).is_some());
        }
    }
}

#[test]
fn mirror_indents_swap_sides_on_even_pages() {
    // #512: with mirrored indents the left indent is the inside one, on the right of an even page.
    let mut d = Document::from_text("odd page\neven page");
    let last = Pos::body(1, 0);
    d.format_paragraphs(&Pos::body(0, 0), &last, &|p| {
        p.indent_left = Some(72.0);
        p.mirror_indents = Some(true);
    })
    .unwrap();
    d.format_paragraphs(&last, &last, &|p| p.page_break_before = Some(true)).unwrap();
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    let odd = l.caret(&Pos::body(0, 0)).unwrap();
    let even = l.caret(&last).unwrap();
    assert_eq!(even.page, 1);
    assert!((odd.x - 144.0).abs() < 0.5, "odd page: inside indent on the left, {odd:?}");
    assert!((even.x - 72.0).abs() < 0.5, "even page: the left side has the outside indent (0), {even:?}");
}

/// Ten short paragraphs in a two-column section (`widths`: unequal columns), then one paragraph
/// in a one-column section starting `start`.
fn two_column_section(start: wordcraft_doc::section::SectionStart, widths: Vec<(f32, f32)>) -> Document {
    use wordcraft_doc::section::SectionProps;
    let text: Vec<String> = (0..10).map(|i| format!("Short paragraph {i}")).chain(["After the break".to_string()]).collect();
    let mut d = Document::from_text(&text.join("\n"));
    let mut s = SectionProps::default();
    s.columns.count = 2;
    s.columns.widths = widths;
    d.para_mut(StoryRef::Body, &Path::top(9)).unwrap().section = Some(Box::new(s));
    d.last_section.start = start;
    d
}

/// Each top-level paragraph's first piece: (page, x, top, bottom).
fn para_boxes(l: &DocLayout) -> Vec<(usize, f32, f32, f32)> {
    let mut v: Vec<(usize, usize, f32, f32, f32)> = Vec::new();
    for (pi, p) in l.pages.iter().enumerate() {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } = it
                && let [b] = path.0[..]
                && !v.iter().any(|e| e.0 == b as usize)
            {
                v.push((b as usize, pi, *x, *y, item_bottom(*y, para, *l0, *l1).unwrap()));
            }
        }
    }
    v.sort_by_key(|e| e.0);
    v.into_iter().map(|(_, pi, x, t, b)| (pi, x, t, b)).collect()
}

/// Bottoms of the two columns (paragraphs 0–9 by their x) and the top of paragraph 10.
fn column_bottoms(l: &DocLayout) -> (usize, f32, usize, f32, (usize, f32)) {
    let b = para_boxes(l);
    let x0 = b[0].1;
    let (left, right): (Vec<_>, Vec<_>) = b[..10].iter().partition(|e| (e.1 - x0).abs() < 1.0);
    let bottom = |v: &[&(usize, f32, f32, f32)]| v.iter().map(|e| e.3).fold(0.0f32, f32::max);
    (left.len(), bottom(&left), right.len(), bottom(&right), (b[10].0, b[10].2))
}

#[test]
fn columns_before_a_continuous_break_are_balanced() {
    use wordcraft_doc::section::SectionStart;
    for widths in [vec![], vec![(150.0, 36.0), (282.0, 0.0)]] {
        let l = lay(&two_column_section(SectionStart::Continuous, widths.clone()));
        let (nl, bl, nr, br, (page, after)) = column_bottoms(&l);
        // Filled left to right, level: five paragraphs each (the left one may hold one more).
        assert!(nl == 5 || nl == 6, "{widths:?}: {nl} left, {nr} right");
        assert!(bl >= br - 0.5 && bl - br < 20.0, "{widths:?}: columns end at {bl} and {br}");
        // The next section starts right below, on the same page.
        assert_eq!(page, 0);
        assert!(after >= bl - 0.5 && after < bl + 30.0, "{widths:?}: next section at {after}, columns end at {bl}");
    }
}

#[test]
fn columns_before_a_page_break_or_a_column_break_are_not_balanced() {
    use wordcraft_doc::section::SectionStart;
    // Next-page break: the ten paragraphs fill the first column; the next section is on page 2.
    let l = lay(&two_column_section(SectionStart::NextPage, vec![]));
    let (nl, _, nr, _, (page, _)) = column_bottoms(&l);
    assert_eq!((nl, nr, page), (10, 0, 1));
    // An explicit column break after paragraph 1 decides where the first column ends.
    let mut d = two_column_section(SectionStart::Continuous, vec![]);
    let p = d.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    let end = p.text.len();
    p.insert_text(end, &wordcraft_doc::para::COLUMN_BREAK.to_string(), &Default::default()).unwrap();
    let l = lay(&d);
    let (nl, bl, nr, br, (page, after)) = column_bottoms(&l);
    assert_eq!((nl, nr, page), (2, 8, 0));
    // The next section starts below the taller column.
    assert!(br > bl + 50.0 && after >= br - 0.5, "columns end at {bl} and {br}, next section at {after}");
}

/// The glyphs of each note reference mark in the body, by note id.
fn note_marks(l: &DocLayout) -> HashMap<u32, Vec<u32>> {
    let mut marks = HashMap::new();
    for it in l.pages.iter().flat_map(|p| p.items.iter()) {
        if let Placed::Lines { story: StoryRef::Body, para, .. } = it {
            for (ci, id) in &para.notes {
                let c = &para.clusters[*ci];
                marks.insert(*id, para.glyphs[c.g0 as usize..c.g1 as usize].iter().map(|g| g.gid).collect::<Vec<_>>());
            }
        }
    }
    marks
}

#[test]
fn footnotes_restart_on_each_page_and_section() {
    use wordcraft_doc::section::{NoteProps, NoteRestart};
    let mut d = Document::from_text(&"Body text line.\n".repeat(120));
    let a = footnote(&mut d, &Pos::body(2, 4), "First note.");
    let b = footnote(&mut d, &Pos::body(4, 4), "Second note.");
    let c = footnote(&mut d, &Pos::body(110, 4), "A later page's note.");
    let l = lay(&d);
    let page = |l: &DocLayout, id: u32| l.caret(&d.start_of(StoryRef::Part(id))).unwrap().page;
    assert!(page(&l, c) > page(&l, a), "the third note is on a later page");
    let marks = note_marks(&l);
    assert_ne!(marks[&c], marks[&a], "numbered on: 3 is not 1");
    // Restart each page: the later page's note is 1 again; the first page counts 1, 2.
    d.settings.footnote_pr = NoteProps { num_restart: Some(NoteRestart::EachPage), ..Default::default() };
    let marks = note_marks(&lay(&d));
    assert_eq!(marks[&c], marks[&a]);
    assert_ne!(marks[&b], marks[&a]);
    // Restart each section, starting at 5, in a second section that starts at paragraph 50.
    d.settings.footnote_pr = NoteProps::default();
    d.para_mut(StoryRef::Body, &Path::top(49)).unwrap().section = Some(Box::new(SectionProps::default()));
    d.last_section.footnote_pr = NoteProps { num_restart: Some(NoteRestart::EachSect), num_start: Some(5), ..Default::default() };
    let nums = note_numbers(&d, false);
    assert_eq!((nums[&a], nums[&b], nums[&c]), (1, 2, 5));
}

#[test]
fn endnotes_at_section_end_and_footnotes_below_text() {
    use wordcraft_doc::section::{NotePos, NoteProps};
    let mut d = Document::from_text("Alpha\nBeta\nGamma\nDelta");
    let eid = d.add_part(
        wordcraft_doc::PartKind::Endnote,
        vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("An endnote.", Default::default()))],
    );
    d.insert_object(
        &Pos::body(0, 5),
        InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Endnote, id: eid, custom: String::new() },
        &Default::default(),
    )
    .unwrap();
    // Two sections, the second on a new page.
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().section = Some(Box::new(SectionProps::default()));
    let end_page = |d: &Document| {
        let l = lay(d);
        (l.caret(&d.start_of(StoryRef::Part(eid))).unwrap().page, l.caret(&Pos::body(2, 0)).unwrap().page)
    };
    assert_eq!(end_page(&d), (1, 1), "end of document: after the second section's text");
    d.settings.endnote_pr = NoteProps { pos: Some(NotePos::SectEnd), ..Default::default() };
    assert_eq!(end_page(&d), (0, 1), "end of section: on the first section's page");
    // A footnote below the text sits right under the last line, above where it'd be at the bottom.
    let fid = footnote(&mut d, &Pos::body(0, 2), "A footnote.");
    let top = |d: &Document| lay(d).caret(&d.start_of(StoryRef::Part(fid))).unwrap().top;
    let bottom = top(&d);
    d.settings.footnote_pr = NoteProps { pos: Some(NotePos::BeneathText), ..Default::default() };
    let below = top(&d);
    let text = lay(&d).caret(&Pos::body(1, 0)).unwrap();
    assert!(below > text.top + text.height && below < bottom - 100.0, "{below} {bottom} {text:?}");
}

/// Each run is hyphenated and proofed in its own language (`w:lang`, resolved through the
/// styles): Polish patterns and dictionary for Polish runs, English ones for English runs.
#[test]
fn hyphenation_and_proofing_follow_each_runs_language() {
    // A tiny synthetic Polish dictionary (real ones are installed on the system, not bundled).
    wordcraft_proof::pl::install_dictionary("SET UTF-8\nSFX B Y 1\nSFX B y ego y\n", "2\nkonstytucyjny/B\npłynie\n").unwrap();
    let text = "Konstytucyjnego żeka typography beautifull";
    let mut d = Document::from_text(text);
    d.settings.auto_hyphenation = true;
    let eng = text.find("typography").unwrap();
    d.format_range(&Pos::body(0, 0), &Pos::body(0, eng), &|c| c.lang = Some("pl-PL".into())).unwrap();
    d.format_range(&Pos::body(0, eng), &Pos::body(0, text.len()), &|c| c.lang = Some("en-US".into())).unwrap();
    let mut c = LayoutCache::new();
    let l = layout(&d, &mut c, &LayoutOptions { proofing: true, ..LayoutOptions::default() });
    let para = l.pages[0].items.iter().find_map(|i| if let Placed::Lines { para, .. } = i { Some(para.clone()) } else { None }).expect("a paragraph");
    // Byte offsets after which a hyphen may go.
    let ends: Vec<usize> = para.hyph_after.iter().filter_map(|&k| para.clusters.get(k as usize).map(|c| c.end)).collect();
    for prefix in ["Kon", "Konsty", "Konstytu", "Konstytucyj", "Konstytucyjne"] {
        assert!(ends.contains(&prefix.len()), "Polish kon-sty-tu-cyj-ne-go: no break after {prefix:?} in {ends:?}");
    }
    for prefix in ["ty", "typog", "typogra"] {
        assert!(ends.contains(&(eng + prefix.len())), "English ty-pog-ra-phy: no break after {prefix:?} in {ends:?}");
    }
    // Spelling: the Polish misspelling and the English one; Polish words aren't English errors.
    let flagged: Vec<&str> = para.issues.iter().filter(|i| !i.2).map(|i| &text[i.0..i.1]).collect();
    assert_eq!(flagged, ["żeka", "beautifull"]);
    // The layout and the Review commands report the same issues.
    let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    let issues = para::paragraph_issues(p, &para::proof_runs(&d, p, None), &Default::default(), true);
    assert_eq!(issues.iter().map(|i| (i.start, i.end, i.kind == wordcraft_proof::IssueKind::Grammar)).collect::<Vec<_>>(), para.issues);
    assert_eq!(issues[0].lang, wordcraft_proof::ProofLang::Pl);
    assert_eq!(issues[1].lang, wordcraft_proof::ProofLang::En);
}

/// Text in a language WordCraft can't proof gets no spelling marks, and "Do not check spelling
/// or grammar" text none either; the document default language applies to text without its own.
#[test]
fn unproofed_languages_and_no_proof_runs_are_not_marked() {
    // A tiny synthetic Polish dictionary (real ones are installed on the system, not bundled).
    wordcraft_proof::pl::install_dictionary("SET UTF-8\nSFX B Y 1\nSFX B y ego y\n", "2\nkonstytucyjny/B\npłynie\n").unwrap();
    let opts = LayoutOptions { proofing: true, ..LayoutOptions::default() };
    let issues = |d: &Document| -> usize {
        let l = layout(d, &mut LayoutCache::new(), &opts);
        l.pages[0].items.iter().map(|i| if let Placed::Lines { para, .. } = i { para.issues.len() } else { 0 }).sum()
    };
    let text = "Nous écrivons beaucoup aujourd'hui.";
    let mut d = Document::from_text(text);
    assert!(issues(&d) > 0, "French words are English misspellings in an English document");
    d.styles.default_chr.lang = Some("fr-FR".into());
    assert_eq!(issues(&d), 0, "no French dictionary: no marks");
    d.styles.default_chr.lang = Some("pl-PL".into());
    let mut pl = Document::from_text("Płynie żeka.");
    pl.styles.default_chr.lang = Some("pl-PL".into());
    assert_eq!(issues(&pl), 1);
    pl.format_range(&Pos::body(0, 0), &Pos::body(0, "Płynie żeka.".len()), &|c| c.no_proof = Some(true)).unwrap();
    assert_eq!(issues(&pl), 0);
}

/// #451: page totals in the body show the document's page count (the stale cached result is
/// ignored), on every page, like the same fields in the footer.
#[test]
fn body_page_totals_count_every_page() {
    let mut d = Document::from_text("Body-1\nBody-2\nBody-3");
    let field =
        |instr: &str| InlineObject::Field { instr: instr.into(), result: "9".into(), locked: false, dirty: false, code: false, form: String::new() };
    for i in 0..3 {
        let p = d.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        p.props.page_break_before = Some(i > 0);
        let n = p.len();
        p.insert_object(n, field("NUMPAGES"), &Default::default()).unwrap();
        let n = p.len();
        p.insert_object(n, field("SECTIONPAGES"), &Default::default()).unwrap();
    }
    let mut fp = wordcraft_doc::Paragraph::new();
    fp.insert_object(0, field("NUMPAGES"), &Default::default()).unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Footer, vec![wordcraft_doc::para_block(fp)]);
    d.last_section.footers.default = Some(id);
    let l = lay(&d);
    assert_eq!(l.pages.len(), 3);
    let shown = |items: &[Placed]| -> Vec<String> {
        items
            .iter()
            .flat_map(|it| if let Placed::Lines { para, .. } = it { para.shown.iter().map(|s| s.1.clone()).collect() } else { vec![] })
            .collect()
    };
    for (i, p) in l.pages.iter().enumerate() {
        assert_eq!(shown(&p.items), ["3", "3"], "body of page {}", i + 1);
        assert_eq!(shown(&p.footer), ["3"], "footer of page {}", i + 1);
    }
    // One page: one pass, total 1.
    let mut d1 = Document::from_text("Only");
    let p = d1.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.insert_object(4, field("NUMPAGES"), &Default::default()).unwrap();
    assert_eq!(shown(&lay(&d1).pages[0].items), ["1"]);
}

/// #450: a footnote referenced in a table cell is placed at the bottom of its page, like one in
/// a body paragraph.
#[test]
fn table_cell_footnotes_are_placed() {
    let mut d = Document::from_text("After the table.");
    d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(wordcraft_doc::Table::new(1, 1, 300.0))).unwrap();
    let cell = Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, 0]), off: 0 };
    d.para_mut(StoryRef::Body, &cell.path).unwrap().insert_text(0, "Table reference", &Default::default()).unwrap();
    let id = footnote(&mut d, &Pos { off: "Table reference".len(), ..cell }, "Table note payload.");
    let l = lay(&d);
    assert_eq!(l.pages.len(), 1);
    let c = l.caret(&d.start_of(StoryRef::Part(id))).expect("the cell's footnote is placed");
    assert_eq!(c.page, 0);
    let body = l.caret(&Pos::body(1, 0)).unwrap();
    assert!(c.top > body.top + 100.0, "at the page bottom: {c:?} {body:?}");
}

#[test]
fn east_asian_text_takes_the_asian_font_and_latin_text_the_latin_font() {
    // #505: one run with a Latin font and an Asian font (w:eastAsia) of its own sets each script in
    // its font. Prefer an installed CJK font other than the one fallback would pick anyway.
    let resolve = |f: &str| wordcraft_fonts::word::resolve(f, false, false);
    let fallback = wordcraft_fonts::FontDb::global().fallback_for('你', 0).map(|f| f.family.clone());
    let mut fonts: Vec<String> = wordcraft_fonts::FontDb::global()
        .east_asian_families()
        .into_iter()
        .filter(|f| {
            let r = resolve(f);
            !r.substituted && r.face.covers('你') && r.face.covers('好')
        })
        .collect();
    fonts.sort_by_key(|f| Some(f) == fallback.as_ref());
    let Some(asian) = fonts.first().cloned() else {
        eprintln!("skipped: no Chinese font installed");
        return;
    };
    let latin = wordcraft_doc::styles::BODY_FONT;
    let text = "Hello 你好 World";
    let mut d = Document::from_text(text);
    d.format_range(&Pos::body(0, 0), &Pos::body(0, text.len()), &|c| {
        c.font = Some(latin.into());
        c.font_ea = Some(asian.clone());
    })
    .unwrap();
    let l = lay(&d);
    let para =
        l.pages.iter().flat_map(|p| p.items.iter()).find_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None }).unwrap();
    let (latin_face, asian_face) = (resolve(latin).face.id(), resolve(&asian).face.id());
    for c in &para.clusters {
        let Some(ch) = text.get(c.start..c.end).and_then(|s| s.chars().next()).filter(|ch| !ch.is_whitespace()) else { continue };
        let face = para.styles[c.style as usize].face.id();
        let want = if ch.is_ascii() { latin_face } else { asian_face };
        assert_eq!(face, want, "{ch:?} in {}", if ch.is_ascii() { latin } else { &asian });
    }
}

/// A page total in the footer ("Page X of Y") is filled in after pagination, so it doesn't make
/// the body lay out a second time; one in the body still does.
#[test]
fn footer_page_total_needs_one_body_pass() {
    let field =
        || InlineObject::Field { instr: "NUMPAGES".into(), result: "9".into(), locked: false, dirty: false, code: false, form: String::new() };
    let mut d = Document::from_text("Body-1\nBody-2\nBody-3");
    for i in 1..3 {
        d.para_mut(StoryRef::Body, &Path::top(i)).unwrap().props.page_break_before = Some(true);
    }
    let mut fp = wordcraft_doc::Paragraph::new();
    fp.insert_object(0, field(), &Default::default()).unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Footer, vec![wordcraft_doc::para_block(fp)]);
    d.last_section.footers.default = Some(id);
    let first = |d: &Document| {
        let totals = Totals::first_guess(d.sections().len());
        layout_pass(d, &mut LayoutCache::new(), &LayoutOptions::default(), &HashMap::new(), &totals, true)
    };
    assert!(matches!(first(&d), Pass::Done(l) if l.pages.len() == 3), "the footer's total asked for another pass");
    // The same field in the body is only known once every page is laid out.
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    let n = p.len();
    p.insert_object(n, field(), &Default::default()).unwrap();
    assert!(matches!(first(&d), Pass::Again(t) if t.pages == 3));
}

/// Bulgarian text (`w:lang` `bg-BG`) is hyphenated by the Bulgarian syllable rules; the same
/// Cyrillic text in an English run is not hyphenated (the English data has nothing to say about it).
#[test]
fn bulgarian_text_is_hyphenated_by_its_own_rules() {
    let text = "Сричкопренасянето в професионалните документи задължително следва правописните правила, разработени от специалистите. ".repeat(6);
    let hyphenated = |d: &Document| -> Vec<(String, String)> {
        let mut out = Vec::new();
        for it in &lay(d).pages[0].items {
            if let Placed::Lines { para, .. } = it {
                for l in para.lines.iter().filter(|x| x.hyphen.is_some()) {
                    let end = l.stop;
                    let before = text.get(..end).unwrap_or_default();
                    let word_start = before.rfind(' ').map_or(0, |i| i + 1);
                    let after = text.get(end..).unwrap_or_default();
                    let word_end = after.find(|c: char| !c.is_alphabetic()).unwrap_or(after.len());
                    out.push((text[word_start..end].to_string(), after[..word_end].to_string()));
                }
            }
        }
        out
    };
    let mut d = Document::from_text(&text);
    d.settings.auto_hyphenation = true;
    assert!(hyphenated(&d).is_empty(), "Cyrillic in an English run: {:?}", hyphenated(&d));
    d.format_range(&Pos::body(0, 0), &Pos::body(0, text.len()), &|c| c.lang = Some("bg-BG".into())).unwrap();
    let splits = hyphenated(&d);
    assert!(!splits.is_empty(), "no hyphenated lines");
    let lim = wordcraft_proof::hyphen::Limits::default();
    for (head, tail) in &splits {
        let word = format!("{head}{tail}");
        let at = head.chars().count();
        assert!(wordcraft_proof::hyphen::hyphen_points_in(&word, &lim, wordcraft_proof::ProofLang::Bg).contains(&at), "{head}-{tail}");
    }
}

/// Spaces and tabs never make a line taller in Word, whatever their size (measured in Word for
/// Windows: 9 pt text with 11–20 pt tabs, spaces and ideographic spaces kept the 9 pt line; a
/// line of only big spaces took the paragraph mark's height; a big letter or no-break space did
/// grow it). Tables of contents with big tab runs were a third taller than Word's.
#[test]
fn spaces_and_tabs_do_not_make_a_line_taller() {
    use wordcraft_doc::props::CharProps;
    let small = CharProps { size: Some(9.0), ..Default::default() };
    let big = CharProps { size: Some(20.0), ..Default::default() };
    let pitch = |middle: &str, middle_props: &CharProps| {
        let mut d = Document::new();
        d.body = (0..3)
            .map(|i| {
                let mut p = wordcraft_doc::Paragraph::with_text(&format!("L{i}A"), small.clone());
                let n = p.len();
                p.insert_text(n, middle, middle_props).unwrap();
                let n = p.len();
                p.insert_text(n, "B", &small).unwrap();
                p.props = ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
                p.mark = small.clone();
                wordcraft_doc::para_block(p)
            })
            .collect();
        let l = lay(&d);
        l.caret(&Pos::body(1, 0)).unwrap().top - l.caret(&Pos::body(0, 0)).unwrap().top
    };
    let plain = pitch(" ", &small);
    for (what, middle) in [("a tab", "\t"), ("a space", " "), ("two spaces", "  "), ("an ideographic space", "\u{3000}")] {
        assert!((pitch(middle, &big) - plain).abs() < 0.01, "{what} at 20 pt: {} vs {plain}", pitch(middle, &big));
    }
    assert!(pitch("x", &big) > plain + 5.0, "a 20 pt letter does make it taller");
    assert!(pitch("\u{A0}", &big) > plain + 5.0, "so does a no-break space");
}

/// A list label taller than its text makes the first line taller above the baseline only: Word
/// keeps the text's descent below it (measured in Word for Windows, `plan/word` R18: a Symbol
/// bullet's taller ascent counts, a Courier New label's deeper descent doesn't). NIH's bulleted
/// items each came out a fifth of a point taller, enough to push a paragraph's last lines over
/// the page bottom.
#[test]
fn a_list_label_adds_ascent_but_not_descent() {
    let first_line = |label_size: Option<f32>| {
        let mut d = Document::new();
        let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        for a in &mut d.numbering.abstracts {
            for l in &mut a.levels {
                l.chr.size = label_size;
            }
        }
        let ten = wordcraft_doc::props::CharProps { size: Some(10.0), ..Default::default() };
        let mut p = wordcraft_doc::Paragraph::with_text("Item", ten.clone());
        p.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
        p.mark = ten;
        p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = lay(&d);
        l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { para.lines.first().cloned() } else { None }).unwrap()
    };
    let plain = first_line(None);
    let big = first_line(Some(30.0));
    let below = |l: &crate::para::Line| l.top + l.height - l.baseline;
    assert!(big.baseline - big.top > plain.baseline - plain.top + 5.0, "the big label raises the baseline");
    assert!((below(&big) - below(&plain)).abs() < 0.01, "the text's descent: {} vs {}", below(&big), below(&plain));
}

/// A cell's hidden end-of-cell mark (`w:hideMark`), measured in Word for Windows (`plan/word`
/// R20, 9 cases): its empty last paragraph takes no room, spacing and all, so a row of such empty
/// cells is only its borders; one ordinary empty cell keeps the row a line tall, and a paragraph
/// with text is never hidden. NIH's Table A showed a blank row Word doesn't.
#[test]
fn a_hidden_cell_mark_takes_no_room() {
    let row3_top = |hide: [bool; 2], second: Option<&str>| {
        let mut d = Document::from_text("Above");
        let mut t = Table::new(3, 2, 300.0);
        for (r, text) in [(0, "R1"), (2, "R3")] {
            for c in 0..2 {
                t.rows[r].cells[c].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))];
            }
        }
        for (cell, hide) in t.rows[1].cells.iter_mut().zip(hide) {
            cell.props.hide_mark = hide;
            let mut blocks = Vec::new();
            if let Some(text) = second {
                blocks.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default())));
            }
            let mut mark = wordcraft_doc::Paragraph::with_text("", Default::default());
            mark.props.space_before = Some(10.0);
            blocks.push(wordcraft_doc::para_block(mark));
            cell.blocks = blocks;
        }
        d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
        let l = lay(&d);
        let at = |r: u32| l.caret(&Pos { story: StoryRef::Body, path: Path(vec![1, r, 0, 0]), off: 0 }).unwrap().top;
        // Rows 1 and 2 together.
        at(2) - at(0)
    };
    // Row 1 is one line of text; row 2's height is what's left.
    let line = {
        let both = row3_top([false, false], Some("Tx"));
        let mut d = Document::from_text("Above");
        let mut t = Table::new(2, 1, 300.0);
        t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("R1", Default::default()))];
        t.rows[1].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("R3", Default::default()))];
        d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
        let l = lay(&d);
        let at = |r: u32| l.caret(&Pos { story: StoryRef::Body, path: Path(vec![1, r, 0, 0]), off: 0 }).unwrap().top;
        assert!(both > at(1) - at(0));
        at(1) - at(0)
    };
    let row2 = |hide: [bool; 2], second: Option<&str>| row3_top(hide, second) - line;
    let hidden = row2([true, true], None);
    assert!(hidden < 1.5, "a row of hidden marks is only its borders: {hidden} vs a line of {line}");
    let mixed = row2([true, false], None);
    let plain = row2([false, false], None);
    assert!(plain > 5.0 && (mixed - plain).abs() < 0.01, "an ordinary empty cell keeps the row: {mixed} vs {plain}");
    let text = row2([true, true], Some("Tx"));
    assert!((text - line).abs() < 0.5, "text stays, the empty mark after it goes: {text} vs {line}");
}

/// A text frame in a header, footer or cell (NIH's footers put the page number in one, centred,
/// with the footer text wrapping around it) is placed apart: it takes no room in the flow, sits
/// where its alignment says, and the text beside it keeps clear of it. WordCraft laid it out as
/// an ordinary paragraph, a line of its own above the footer text.
#[test]
fn a_text_frame_in_a_cell_floats_beside_the_text() {
    use wordcraft_doc::props::TextFrame;
    let mut d = Document::from_text("Above");
    let mut t = Table::new(1, 1, 300.0);
    let mut framed = wordcraft_doc::Paragraph::with_text("7", Default::default());
    framed.props.frame = Some(TextFrame {
        x_align: Some("right".into()),
        h_anchor: "text".into(),
        v_anchor: "text".into(),
        wrap: "around".into(),
        ..Default::default()
    });
    let text = wordcraft_doc::Paragraph::with_text("Cell text", Default::default());
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(framed), wordcraft_doc::para_block(text)];
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let l = lay(&d);
    let at = |i: u32| l.caret(&Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, i]), off: 0 }).unwrap();
    let (frame, text) = (at(0), at(1));
    assert!((frame.top - text.top).abs() < 0.5, "the frame takes no room: {} vs {}", frame.top, text.top);
    assert!(frame.x > text.x + 200.0, "right-aligned in the cell: {} vs {}", frame.x, text.x);
}

/// Where a list paragraph's text starts when its style has indents of its own (measured in Word,
/// `plan/word` R16): the list level's indents go on top of the most derived style that gives the
/// list, so that style's own indent (and those of styles based on it) win, its base styles' don't
/// and the paragraph's own list wins over every style; a paragraph that switches the list off
/// (`num: 0`) drops the styles' indents. NIH's Heading 3 (its own 0.9" on Heading 2's list) sat at
/// the list's 0.6", and its "sample text" paragraphs with the list off at 0.9" instead of the margin.
#[test]
fn list_indents_and_style_indents_combine_like_word() {
    use wordcraft_doc::props::NumRef;
    use wordcraft_doc::styles::{Style, StyleKind};
    let text_x = |style: &str, own: ParaProps| {
        let mut d = Document::new();
        let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        let style_of = |id: &str, based: Option<&str>, para: ParaProps| Style {
            id: id.into(),
            name: id.into(),
            kind: StyleKind::Paragraph,
            based_on: based.map(Into::into),
            para,
            ..Default::default()
        };
        let list = ParaProps { numbering: Some(NumRef { num, level: 0 }), ..Default::default() };
        let ind = ParaProps { indent_left: Some(64.8), ..Default::default() };
        // List and indent in one style; list in the base, indent in the derived one; the reverse.
        d.styles.upsert(style_of("Both", None, ParaProps { indent_left: Some(64.8), ..list.clone() }));
        d.styles.upsert(style_of("ListBase", None, list.clone()));
        d.styles.upsert(style_of("IndentOnList", Some("ListBase"), ind.clone()));
        d.styles.upsert(style_of("IndentBase", None, ind));
        d.styles.upsert(style_of("ListOnIndent", Some("IndentBase"), list.clone()));
        let own =
            ParaProps { style: Some(style.into()), numbering: own.numbering.map(|n| NumRef { num: if n.num == 0 { 0 } else { num }, ..n }), ..own };
        let mut p = wordcraft_doc::Paragraph::with_text("Text", Default::default());
        p.props = own;
        d.body = vec![wordcraft_doc::para_block(p)];
        lay(&d).caret(&Pos::body(0, 0)).unwrap().x
    };
    let listed = || ParaProps { numbering: Some(NumRef { num: 1, level: 0 }), ..Default::default() };
    let off = || ParaProps { numbering: Some(NumRef { num: 0, level: 0 }), ..Default::default() };
    let cases = [
        ("the style's own indent beats its list's", text_x("Both", ParaProps::default()), 72.0 + 64.8),
        ("a style based on the list's style wins too", text_x("IndentOnList", ParaProps::default()), 72.0 + 64.8),
        ("the list's style beats its base style", text_x("ListOnIndent", ParaProps::default()), 108.0),
        ("the paragraph's own list beats every style", text_x("Both", listed()), 108.0),
        ("list off: the style's indent goes too", text_x("Both", off()), 72.0),
        ("list off drops an indent from any style", text_x("IndentBase", off()), 72.0),
        ("list off keeps the paragraph's own indent", text_x("Both", ParaProps { indent_left: Some(10.0), ..off() }), 82.0),
    ];
    for (what, got, want) in cases {
        assert!((got - want).abs() < 0.5, "{what}: {got} vs {want}");
    }
}

/// Keep with next at a page bottom, measured in Word for Windows (`plan/word` R19, 20 cases): a
/// paragraph that would fit on the page by itself moves over with the next one, but one that
/// doesn't fit anyway breaks as usual, its last lines going over with the next paragraph (with
/// Keep lines together it moves whole); a chain of kept paragraphs moves together. NIH's
/// five-line kept paragraph left a third of a page empty.
#[test]
fn keep_with_next_moves_a_paragraph_only_when_it_would_fit() {
    // Page of each of the kept paragraph's 5 lines and of the first paragraph after it.
    let pages = |fillers: usize, kept: ParaProps, next: ParaProps| {
        let mut d = Document::new();
        let plain = ParaProps {
            space_before: Some(0.0),
            space_after: Some(0.0),
            line_spacing: Some(wordcraft_doc::props::LineSpacing::Multiple(1.0)),
            ..Default::default()
        };
        let para = |text: &str, p: &ParaProps| {
            let mut x = wordcraft_doc::Paragraph::with_text(text, Default::default());
            x.props = ParaProps { keep_next: p.keep_next, keep_lines: p.keep_lines, ..plain.clone() };
            wordcraft_doc::para_block(x)
        };
        d.body = (0..fillers).map(|i| para(&format!("F{i}"), &plain)).collect();
        d.body.push(para("Ka\nKb\nKc\nKd\nKe", &kept));
        d.body.push(para("NX", &next));
        d.body.push(para("NY", &plain));
        let l = lay(&d);
        let k = fillers;
        [0, 3, 6, 9, 12]
            .map(|off| l.caret(&Pos::body(k, off)).unwrap().page)
            .into_iter()
            .chain([l.caret(&Pos::body(k + 1, 0)).unwrap().page])
            .collect::<Vec<_>>()
    };
    let none = ParaProps::default();
    let kn = ParaProps { keep_next: Some(true), ..Default::default() };
    let kl = ParaProps { keep_lines: Some(true), ..kn.clone() };
    // Find where the kept paragraph just fits: its last line on page 1, the next one's on page 2.
    let fit = (20..80).find(|n| pages(*n, none.clone(), none.clone()) == [0, 0, 0, 0, 0, 1]).expect("a filler count that fills the page");
    assert_eq!(pages(fit, kn.clone(), none.clone()), [1, 1, 1, 1, 1, 1], "fits by itself: moves with the next");
    let split = pages(fit + 1, none.clone(), none.clone());
    assert!(split[0] == 0 && split[4] == 1, "one more line splits it: {split:?}");
    assert_eq!(pages(fit + 1, kn.clone(), none.clone()), split, "doesn't fit anyway: breaks as usual");
    assert_eq!(pages(fit + 1, kl.clone(), none.clone()), [1, 1, 1, 1, 1, 1], "with keep lines together it moves whole");
    assert_eq!(pages(fit - 1, kn.clone(), kn.clone()), [1, 1, 1, 1, 1, 1], "a kept chain moves together");
}

/// A kept heading goes with the paragraph after it when that paragraph cannot start on its
/// page (#966): with widow/orphan control on, one line of it at the bottom of the page moves
/// over with the rest, so only two lines below the heading hold it there. Word and LibreOffice
/// both move the heading; WordCraft left it alone. With the control off the single line stays,
/// and the heading keeps it company.
#[test]
fn keep_with_next_carries_the_heading_when_widow_control_moves_the_next_paragraph() {
    // Page of the heading, then of the next paragraph's lines at "Bb", "Bc" and "Be".
    let pages = |fillers: usize, widow: bool| -> [usize; 4] {
        let mut d = Document::new();
        let plain = ParaProps {
            space_before: Some(0.0),
            space_after: Some(0.0),
            line_spacing: Some(wordcraft_doc::props::LineSpacing::Multiple(1.0)),
            widow_control: Some(widow),
            ..Default::default()
        };
        let para = |text: &str, p: &ParaProps| {
            let mut x = wordcraft_doc::Paragraph::with_text(text, Default::default());
            x.props = p.clone();
            wordcraft_doc::para_block(x)
        };
        d.body = (0..fillers).map(|i| para(&format!("F{i}"), &plain)).collect();
        let mut heading = wordcraft_doc::Paragraph::with_text("HEADING", Default::default());
        heading.props = ParaProps { keep_next: Some(true), ..plain.clone() };
        d.body.push(wordcraft_doc::para_block(heading));
        d.body.push(para("Bb\nBc\nBd\nBe\nBf", &plain));
        let l = lay(&d);
        [
            l.caret(&Pos::body(fillers, 0)).unwrap().page,
            l.caret(&Pos::body(fillers + 1, 0)).unwrap().page,
            l.caret(&Pos::body(fillers + 1, 3)).unwrap().page,
            l.caret(&Pos::body(fillers + 1, 9)).unwrap().page,
        ]
    };
    // Find the page edge that leaves the heading room for exactly one line of the paragraph
    // after it: with the control off it starts there and the rest goes over.
    let fit = (20..100).find(|n| pages(*n, false) == [0, 0, 1, 1]).expect("a filler count that leaves room for one line after the heading");
    assert_eq!(pages(fit, false), [0, 0, 1, 1], "without the control one line stays: {fit}");
    // With widow/orphan control the paragraph moves over whole, and the heading goes with it.
    assert_eq!(pages(fit, true), [1, 1, 1, 1], "the heading follows the paragraph: {fit}");
    // One line more of room holds both of its first lines below the heading, under either
    // setting.
    assert_eq!(pages(fit - 1, true), [0, 0, 0, 1], "two lines below the heading hold it");
    assert_eq!(pages(fit - 1, false), [0, 0, 0, 1]);
}

/// The chain case of #966: a kept heading before a kept paragraph before a long one all move
/// over when the last one cannot start on the page, and all stay when it can.
#[test]
fn keep_with_next_chain_carries_over_when_the_last_paragraph_cannot_start() {
    let pages = |fillers: usize, widow: bool| -> Vec<usize> {
        let mut d = Document::new();
        let plain = ParaProps {
            space_before: Some(0.0),
            space_after: Some(0.0),
            line_spacing: Some(wordcraft_doc::props::LineSpacing::Multiple(1.0)),
            widow_control: Some(widow),
            ..Default::default()
        };
        let para = |text: &str, p: &ParaProps| {
            let mut x = wordcraft_doc::Paragraph::with_text(text, Default::default());
            x.props = p.clone();
            wordcraft_doc::para_block(x)
        };
        d.body = (0..fillers).map(|i| para(&format!("F{i}"), &plain)).collect();
        let kept = ParaProps { keep_next: Some(true), ..plain.clone() };
        d.body.push(para("HEADING", &kept));
        d.body.push(para("MID", &kept));
        d.body.push(para("Bb\nBc\nBd\nBe\nBf", &plain));
        let l = lay(&d);
        // Pages of the heading, the kept middle, and the long paragraph's first two lines.
        vec![
            l.caret(&Pos::body(fillers, 0)).unwrap().page,
            l.caret(&Pos::body(fillers + 1, 0)).unwrap().page,
            l.caret(&Pos::body(fillers + 2, 0)).unwrap().page,
            l.caret(&Pos::body(fillers + 2, 3)).unwrap().page,
        ]
    };
    // The page edge that leaves the heading and the kept middle room for one line of the long
    // paragraph after them.
    let fit = (20..100).find(|n| pages(*n, false) == [0, 0, 0, 1]).expect("a filler count that leaves room for one line after the chain");
    // With widow/orphan control the long paragraph cannot start there, so the whole chain
    // moves over.
    assert_eq!(pages(fit, true), [1, 1, 1, 1], "the chain follows the paragraph: {fit}");
    assert_eq!(pages(fit, false), [0, 0, 0, 1], "without the control one line stays: {fit}");
}

/// Keep lines together on the paragraph after a kept heading: when the whole paragraph does
/// not fit below the heading, both move over. Word does not leave a kept heading holding part
/// of a paragraph that keep lines together will not split (#966).
#[test]
fn keep_with_next_carries_the_heading_when_keep_lines_moves_the_next_paragraph() {
    let pages = |fillers: usize, kept_lines: bool| -> Vec<usize> {
        let mut d = Document::new();
        let plain = ParaProps {
            space_before: Some(0.0),
            space_after: Some(0.0),
            line_spacing: Some(wordcraft_doc::props::LineSpacing::Multiple(1.0)),
            widow_control: Some(false),
            ..Default::default()
        };
        let para = |text: &str, p: &ParaProps| {
            let mut x = wordcraft_doc::Paragraph::with_text(text, Default::default());
            x.props = p.clone();
            wordcraft_doc::para_block(x)
        };
        d.body = (0..fillers).map(|i| para(&format!("F{i}"), &plain)).collect();
        let mut heading = wordcraft_doc::Paragraph::with_text("HEADING", Default::default());
        heading.props = ParaProps { keep_next: Some(true), ..plain.clone() };
        d.body.push(wordcraft_doc::para_block(heading));
        let body = ParaProps { keep_lines: Some(kept_lines), ..plain.clone() };
        d.body.push(para("Bb\nBc\nBd", &body));
        let l = lay(&d);
        // Pages of the heading and of the paragraph's first two lines.
        vec![
            l.caret(&Pos::body(fillers, 0)).unwrap().page,
            l.caret(&Pos::body(fillers + 1, 0)).unwrap().page,
            l.caret(&Pos::body(fillers + 1, 3)).unwrap().page,
        ]
    };
    // The page edge that leaves the heading room for exactly one line of the three-line
    // paragraph after it.
    let fit = (20..100).find(|n| pages(*n, false) == [0, 0, 1]).expect("a filler count that leaves room for one line after the heading");
    // With keep lines together the paragraph moves over whole, and the heading goes with it.
    assert_eq!(pages(fit, true), [1, 1, 1], "the heading follows the paragraph: {fit}");
    assert_eq!(pages(fit, false), [0, 0, 1], "without it the paragraph splits: {fit}");
}

/// An empty paragraph that only carries a section break before a continuous section takes no
/// room, measured in Word for Windows (`plan/word` R21, 10 cases): the text after it follows right
/// on, and columns balanced above it don't count it (two one-line paragraphs in two columns sit
/// side by side). A section break on a paragraph with text keeps its line, and so does any break
/// before a section on a new page. The SARP form's fields drifted a line lower at each of its
/// many continuous breaks.
#[test]
fn an_empty_continuous_section_mark_takes_no_room() {
    use wordcraft_doc::section::{SectionProps, SectionStart};
    // "Pa", the section's last paragraph (`mark`), "Pb", "Pc" in the next section of `cols`
    // columns, then "Pd" after a second break.
    let tops = |mark: &str, cols: u32, start: SectionStart| {
        let mut d = Document::from_text(&format!("Pa\n{mark}\nPb\nPc\n\nPd"));
        d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().section =
            Some(Box::new(SectionProps { start: SectionStart::Continuous, ..Default::default() }));
        let mut s = SectionProps { start, ..Default::default() };
        s.columns.count = cols;
        d.para_mut(StoryRef::Body, &Path::top(4)).unwrap().section = Some(Box::new(s));
        d.last_section.start = SectionStart::Continuous;
        let l = lay(&d);
        [0, 2, 3, 5].map(|i| {
            let c = l.caret(&Pos::body(i, 0)).unwrap();
            (c.page, (c.top * 10.0).round() / 10.0, c.x)
        })
    };
    let one = tops("", 1, SectionStart::Continuous);
    let line = one[2].1 - one[1].1;
    assert!((one[1].1 - one[0].1 - line).abs() < 0.15, "Pb right under Pa: {one:?}");
    assert!((one[3].1 - one[2].1 - line).abs() < 0.15, "Pd right under Pc: {one:?}");
    let text = tops("Mark", 1, SectionStart::Continuous);
    assert!((text[1].1 - text[0].1 - 2.0 * line).abs() < 0.15, "a mark with text keeps its line: {text:?}");
    let two = tops("", 2, SectionStart::Continuous);
    assert_eq!(two[1].1, two[2].1, "balanced: Pb and Pc side by side: {two:?}");
    assert!(two[2].2 > two[1].2 + 100.0, "Pc in the second column: {two:?}");
    assert!((two[3].1 - two[1].1 - line).abs() < 0.15, "Pd a line below them: {two:?}");
    let new_page = tops("", 1, SectionStart::NextPage);
    assert_eq!(new_page[1].0, 1, "a next-page section starts a page: {new_page:?}");
}

/// A floating object positioned relative to its paragraph is measured from the paragraph's start
/// above its space before, measured in Word for Windows (`plan/word` R22, 10 cases: 0, 10 and 20
/// pt before, offsets 0 and −7.2 pt; the previous paragraph's space after still counts). The SARP
/// form's field boxes sat a space-before too low, over their labels.
#[test]
fn paragraph_relative_floats_start_above_the_space_before() {
    let shape_top = |before: f32, prev_after: f32, y: f32| {
        let float = wordcraft_doc::para::Float {
            wrap: wordcraft_doc::para::Wrap::InFrontOfText,
            h_rel: wordcraft_doc::para::Anchor::Column,
            v_rel: wordcraft_doc::para::Anchor::Paragraph,
            y,
            ..Default::default()
        };
        let shape = InlineObject::Shape {
            kind: wordcraft_doc::para::ShapeKind::Rectangle,
            w: 20.0,
            h: 20.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float,
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        let plain = ParaProps { space_before: Some(0.0), space_after: Some(prev_after), ..Default::default() };
        let mut first = wordcraft_doc::Paragraph::with_text("First", Default::default());
        first.props = plain;
        let mut p = wordcraft_doc::Paragraph::with_text("Text ", Default::default());
        p.text.push(wordcraft_doc::para::OBJ);
        p.objects = vec![shape];
        p.runs = vec![wordcraft_doc::Run { len: p.text.len(), props: Default::default() }];
        p.props = ParaProps { space_before: Some(before), space_after: Some(0.0), ..Default::default() };
        let mut d = Document::from_text("");
        d.body = vec![wordcraft_doc::para_block(first), wordcraft_doc::para_block(p)];
        let l = lay(&d);
        let first_bottom = {
            let c = l.caret(&Pos::body(0, 0)).unwrap();
            c.top + c.height
        };
        let top = l.pages[0].items.iter().find_map(|i| if let Placed::Shape { rect, .. } = i { Some(rect.y) } else { None }).unwrap();
        (top, first_bottom)
    };
    for before in [0.0, 10.0, 20.0] {
        for y in [0.0, -7.2] {
            let (top, first_bottom) = shape_top(before, 0.0, y);
            assert!((top - (first_bottom + y)).abs() < 1.0, "{before} pt before, offset {y}: {top} vs {}", first_bottom + y);
        }
    }
    let (top, first_bottom) = shape_top(0.0, 10.0, 0.0);
    assert!((top - (first_bottom + 10.0)).abs() < 1.0, "below the previous paragraph's space after: {top} vs {first_bottom}");
}

/// Unequal columns (a narrow label column and a wide text column, as in NIH's SARP form): text
/// after a column break is broken for the column it lands in. WordCraft broke it for the narrow
/// column it started in, so a paragraph took a dozen lines a word or two wide.
#[test]
fn text_after_a_column_break_takes_the_new_column_s_width() {
    let mut d = Document::new();
    d.last_section.columns = wordcraft_doc::section::Columns { count: 2, space: 36.0, separator: false, widths: vec![(67.0, 52.0), (348.0, 0.0)] };
    let brk = wordcraft_doc::para::COLUMN_BREAK;
    let text = format!("Label{brk}Provide a description of the planned recruitment methods including use of contact lists and databases.");
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
    let l = lay(&d);
    let lines: Vec<(f32, usize)> =
        l.pages[0].items.iter().filter_map(|it| if let Placed::Lines { x, l0, l1, .. } = it { Some((*x, l1 - l0)) } else { None }).collect();
    let (wide_x, wide_lines) = lines.last().copied().unwrap();
    assert!(wide_x > 72.0 + 67.0, "the text is in the second column: {lines:?}");
    assert!(wide_lines <= 3, "broken for the 348 pt column, not the 67 pt one: {lines:?}");
}

/// Space before at the top of a page, measured in Word for Windows (32 cases): a paragraph that
/// ends with a page break keeps its mark with the break (nothing of it, not even its space after,
/// goes to the new page) and the next paragraph gets no space before there; a paragraph that starts with a page break starts on
/// the new page with its space before; a page break before keeps the space before in compatibility
/// modes up to 14 (less the previous paragraph's space after) and drops it in mode 15.
#[test]
fn space_before_at_the_top_of_a_page_like_word() {
    let para = |text: &str, before: f32, pbb: bool| {
        let mut p = wordcraft_doc::Paragraph::with_text(text, Default::default());
        p.props = ParaProps { space_before: Some(before), space_after: Some(0.0), page_break_before: Some(pbb), ..Default::default() };
        wordcraft_doc::para_block(p)
    };
    let probe_top = |blocks: Vec<std::sync::Arc<wordcraft_doc::Block>>, mode: u32| {
        let mut d = Document::new();
        d.settings.compat_mode = mode;
        let n = blocks.len();
        d.body = blocks;
        let l = lay(&d);
        let c = l.caret(&Pos::body(n - 1, 0)).unwrap();
        (c.page, c.top)
    };
    let plain_top = probe_top(vec![para("first", 0.0, false), para("PROBE", 0.0, true)], 15).1;
    let brk = wordcraft_doc::para::PAGE_BREAK;
    // A paragraph ending with a page break, then one with 24 pt before: no space, in both modes.
    for mode in [12, 15] {
        let (page, top) = probe_top(vec![para(&format!("first{brk}"), 0.0, false), para("PROBE", 24.0, false)], mode);
        assert_eq!(page, 1, "mode {mode}");
        assert!((top - plain_top).abs() < 0.05, "mode {mode}: the mark stays with the break and the space goes: {top} vs {plain_top}");
    }
    // Its space after stays behind too (modes 12 to 15), also when it holds only the break: the next
    // paragraph, one with a page break before (no blank page) or a table starts at the top.
    let after = |text: &str| {
        let mut p = wordcraft_doc::Paragraph::with_text(text, Default::default());
        p.props = ParaProps { space_before: Some(0.0), space_after: Some(12.4), ..Default::default() };
        wordcraft_doc::para_block(p)
    };
    let table_top = |blocks: Vec<std::sync::Arc<wordcraft_doc::Block>>| {
        let mut d = Document::new();
        d.body = blocks;
        d.body.push(std::sync::Arc::new(wordcraft_doc::Block::Table(Table::new(1, 1, 468.0))));
        let l = lay(&d);
        let page = l.pages.len() - 1;
        let y = l.pages[page].items.iter().find_map(|i| if let Placed::Cell { rect, .. } = i { Some(rect.y) } else { None });
        (page, y)
    };
    let (_, alone) = table_top(vec![]);
    for text in [brk.to_string(), format!("first{brk}")] {
        for mode in [12, 14, 15] {
            for (before, pbb) in [(0.0, false), (24.0, false), (0.0, true)] {
                let (page, top) = probe_top(vec![after(&text), para("PROBE", before, pbb)], mode);
                assert_eq!(page, 1, "mode {mode} {text:?} page break before {pbb}");
                assert!((top - plain_top).abs() < 0.05, "mode {mode} {text:?} before {before}: {top} vs {plain_top}");
            }
        }
        assert_eq!(table_top(vec![after(&text)]), (1, alone), "{text:?}: a table after the break");
    }
    // A paragraph starting with a page break: its space before on the new page.
    let mut d = Document::new();
    d.body = vec![para("first", 0.0, false), para(&format!("{brk}PROBE"), 24.0, false)];
    let l = lay(&d);
    let c = l.caret(&Pos::body(1, brk.len_utf8())).unwrap();
    assert_eq!(c.page, 1);
    assert!((c.top - plain_top - 24.0).abs() < 0.05, "{} vs {}", c.top, plain_top + 24.0);
    // Page break before: kept up to mode 14, dropped in 15.
    let (_, kept) = probe_top(vec![para("first", 0.0, false), para("PROBE", 24.0, true)], 14);
    let (_, dropped) = probe_top(vec![para("first", 0.0, false), para("PROBE", 24.0, true)], 15);
    assert!((kept - plain_top - 24.0).abs() < 0.05 && (dropped - plain_top).abs() < 0.05, "{kept} {dropped} {plain_top}");
    // After a page break before (mode 14) the previous paragraph's space after comes off: 24 - 10.
    let mut first = wordcraft_doc::Paragraph::with_text("first", Default::default());
    first.props = ParaProps { space_before: Some(0.0), space_after: Some(10.0), ..Default::default() };
    let (_, top) = probe_top(vec![wordcraft_doc::para_block(first.clone()), para("PROBE", 24.0, true)], 14);
    assert!((top - plain_top - 14.0).abs() < 0.05, "{top} vs {}", plain_top + 14.0);
    // More after than before: nothing left.
    first.props.space_after = Some(30.0);
    let (_, top) = probe_top(vec![wordcraft_doc::para_block(first), para("PROBE", 24.0, true)], 14);
    assert!((top - plain_top).abs() < 0.05, "{top} vs {plain_top}");
    // The caret can still reach the end of a paragraph whose mark stayed with its page break.
    let mut d = Document::new();
    d.body = vec![para(&format!("first{brk}"), 0.0, false), para("next", 0.0, false)];
    let l = lay(&d);
    assert!(l.caret(&Pos::body(0, "first".len() + brk.len_utf8())).is_some(), "caret after the break");
}

/// Word kerns a run only when Kerning for fonts (`w:kern`) is set and the font is at least that
/// size; WordCraft kerned everything, so its lines held more text than Word's (measured in Word for
/// Windows with Calibri at 14 pt: no `w:kern`, `w:kern="0"` and a threshold above the size are
/// all unkerned).
#[test]
fn kerning_follows_kerning_for_fonts_like_word() {
    use wordcraft_doc::props::CharProps;
    let width = |kern: Option<f32>| {
        let mut d = Document::new();
        let chr = CharProps { size: Some(14.0), kern, ..Default::default() };
        d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("AVAVAV To Wo Ty Yo AWAY", chr))];
        let l = lay(&d);
        let end = d.para_at(&Pos::body(0, 0)).map_or(0, |p| p.len());
        l.caret(&Pos::body(0, end)).unwrap().x - l.caret(&Pos::body(0, 0)).unwrap().x
    };
    let plain = width(Some(0.0));
    assert!(width(Some(1.0)) < plain - 1.0, "kerned from 1 pt: {} vs {plain}", width(Some(1.0)));
    assert!((width(Some(14.0)) - width(Some(1.0))).abs() < 0.01, "a threshold equal to the size kerns");
    assert!((width(Some(15.0)) - plain).abs() < 0.01, "a threshold above the size doesn't kern");
    // A new document kerns from 1 pt, like Word's Normal template.
    assert!((width(None) - width(Some(1.0))).abs() < 0.01, "new documents kern");
    let mut d = Document::new();
    d.styles.default_chr.kern = None;
    assert!(d.styles.resolve_char(None, &Default::default()).kern == 0.0, "no Kerning for fonts: not kerned");
}

/// Word's automatic (HTML-style) paragraph spacing, measured in Word for Windows: 14 pt at any
/// size; none before the first paragraph of the document or a cell, none after the last one of a
/// cell; next to another paragraph's space the larger of the two counts.
#[test]
fn automatic_paragraph_spacing_like_word() {
    let auto = |before: bool, after: bool| ParaProps {
        space_before: Some(5.0),
        space_after: Some(5.0),
        space_before_auto: before.then_some(true),
        space_after_auto: after.then_some(true),
        ..Default::default()
    };
    let plain = ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
    let tops = |props: [ParaProps; 3]| {
        let mut d = Document::new();
        d.body = ["FIRST", "SECOND", "THIRD"]
            .iter()
            .zip(props)
            .map(|(t, p)| {
                let mut para = wordcraft_doc::Paragraph::with_text(t, Default::default());
                para.props = p;
                wordcraft_doc::para_block(para)
            })
            .collect();
        let l = lay(&d);
        [0, 1, 2].map(|i| l.caret(&Pos::body(i, 0)).unwrap().top)
    };
    let base = tops([plain.clone(), plain.clone(), plain.clone()]);
    let line = base[1] - base[0];
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    // Space after: 14 pt.
    let t = tops([plain.clone(), auto(false, true), plain.clone()]);
    assert!(near(t[2] - t[1], line + 14.0), "{t:?}");
    // Space before: 14 pt, but none for the document's first paragraph.
    let t = tops([auto(true, false), auto(true, false), plain.clone()]);
    assert!(near(t[0], base[0]), "no automatic space at the top of the document");
    assert!(near(t[1] - t[0], line + 14.0), "{t:?}");
    // Two automatic spaces meet: 14 pt, not 28; an automatic one beside 10 pt: 14.
    let t = tops([plain.clone(), auto(true, true), auto(true, true)]);
    assert!(near(t[2] - t[1], line + 14.0), "{t:?}");
    let ten = ParaProps { space_before: Some(10.0), ..plain.clone() };
    let t = tops([plain.clone(), auto(false, true), ten]);
    assert!(near(t[2] - t[1], line + 14.0), "{t:?}");
    // Explicitly off: the measured spacing again.
    let off = ParaProps { space_after_auto: Some(false), ..auto(false, false) };
    let t = tops([plain.clone(), off, plain.clone()]);
    assert!(near(t[2] - t[1], line + 5.0), "{t:?}");
}

/// In a table cell automatic spacing drops at the cell's top and bottom (Word for Windows).
#[test]
fn automatic_paragraph_spacing_drops_at_cell_edges() {
    let cell_tops = |auto: bool| {
        let mut d = Document::from_text("Above");
        let mut t = Table::new(1, 1, 300.0);
        t.rows[0].cells[0].blocks = ["SECOND", "THIRD"]
            .iter()
            .map(|s| {
                let mut p = wordcraft_doc::Paragraph::with_text(s, Default::default());
                p.props = ParaProps {
                    space_before: Some(0.0),
                    space_after: Some(0.0),
                    space_before_auto: auto.then_some(true),
                    space_after_auto: auto.then_some(true),
                    ..Default::default()
                };
                wordcraft_doc::para_block(p)
            })
            .collect();
        d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
        d.insert_block(StoryRef::Body, &Path::top(2), wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::with_text("Below", Default::default())))
            .unwrap();
        let l = lay(&d);
        let in_cell = |i: u32| l.caret(&Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, i]), off: 0 }).unwrap().top;
        (in_cell(0), in_cell(1), l.caret(&Pos::body(2, 0)).unwrap().top)
    };
    let (a0, a1, a_below) = cell_tops(true);
    let (p0, p1, p_below) = cell_tops(false);
    assert!((a0 - p0).abs() < 0.05, "no automatic space at the top of the cell");
    assert!((a1 - a0 - (p1 - p0) - 14.0).abs() < 0.05, "14 pt between the cell's paragraphs");
    assert!((a_below - a1 - (p_below - p1)).abs() < 0.05, "none after the cell's last paragraph");
}

/// Automatic spacing inside a list, measured in Word for Windows (`plan/word` R17, 16 cases): none
/// between two list items, of one list or two, at any level, whichever item asks for it; the first
/// item keeps its automatic space before and the last its space after, a paragraph with its list
/// switched off in between keeps both, and measured spacing stays. NIH's bulleted IND reports
/// (each item automatic before and after) were 14 pt apart instead of touching.
#[test]
fn automatic_spacing_drops_between_list_items() {
    use wordcraft_doc::props::NumRef;
    let auto = ParaProps {
        space_before: Some(5.0),
        space_after: Some(5.0),
        space_before_auto: Some(true),
        space_after_auto: Some(true),
        ..Default::default()
    };
    let plain = ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
    let tops = |items: [ParaProps; 3], in_cell: bool| {
        let mut d = Document::new();
        let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        let other = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        let fix = |p: ParaProps| ParaProps {
            numbering: p.numbering.map(|n| NumRef {
                num: match n.num {
                    0 => 0,
                    1 => num,
                    _ => other,
                },
                ..n
            }),
            ..p
        };
        let paras: Vec<_> = std::iter::once(plain.clone())
            .chain(items.map(fix))
            .chain(std::iter::once(plain.clone()))
            .enumerate()
            .map(|(i, p)| {
                let mut para = wordcraft_doc::Paragraph::with_text(&format!("P{i}"), Default::default());
                para.props = p;
                wordcraft_doc::para_block(para)
            })
            .collect();
        if in_cell {
            let mut t = Table::new(1, 1, 300.0);
            t.rows[0].cells[0].blocks = paras;
            d.body = vec![wordcraft_doc::Block::Table(t).into()];
        } else {
            d.body = paras;
        }
        let at = |i: u32| if in_cell { Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, i]), off: 0 } } else { Pos::body(i as usize, 0) };
        let l = lay(&d);
        [0, 1, 2, 3, 4].map(|i| l.caret(&at(i)).unwrap().top)
    };
    let item = |num: u32, level: u8, p: &ParaProps| ParaProps { numbering: Some(NumRef { num, level }), ..p.clone() };
    for in_cell in [false, true] {
        let base = tops([plain.clone(), plain.clone(), plain.clone()], in_cell);
        let line = base[1] - base[0];
        let gaps = |t: [f32; 5]| [1, 2, 3, 4].map(|i| ((t[i] - t[i - 1] - line) * 10.0).round() / 10.0);
        let one = item(1, 0, &auto);
        let cases = [
            ("one list", [one.clone(), one.clone(), one.clone()], [14.0, 0.0, 0.0, 14.0]),
            ("two lists", [one.clone(), item(2, 0, &auto), item(2, 0, &auto)], [14.0, 0.0, 0.0, 14.0]),
            ("two levels", [one.clone(), item(1, 1, &auto), one.clone()], [14.0, 0.0, 0.0, 14.0]),
            ("a plain item between", [one.clone(), item(1, 0, &plain), one.clone()], [14.0, 0.0, 0.0, 14.0]),
            ("list off between", [one.clone(), item(0, 0, &auto), one.clone()], [14.0, 14.0, 14.0, 14.0]),
            ("no list", [auto.clone(), auto.clone(), auto.clone()], [14.0, 14.0, 14.0, 14.0]),
        ];
        for (what, items, want) in cases {
            assert_eq!(gaps(tops(items, in_cell)), want, "{what} (in a cell: {in_cell})");
        }
        let fourteen = ParaProps { space_before: Some(14.0), ..plain.clone() };
        assert_eq!(
            gaps(tops([item(1, 0, &fourteen), item(1, 0, &fourteen), item(1, 0, &fourteen)], in_cell)),
            [14.0, 14.0, 14.0, 0.0],
            "measured spacing stays"
        );
    }
}

/// Vietnamese is written in single syllables: automatic hyphenation leaves its words whole. (The
/// English patterns happen not to split these accented words today; the Vietnamese guard in
/// `hyphenation_points` keeps it so whatever the patterns become.)
#[test]
fn automatic_hyphenation_leaves_vietnamese_whole() {
    let text =
        "Những người khuyến khích chuyện trường thường nghiêng nghiêng nhưng thành phố Nguyễn Trường Thượng xuyên suốt chương trình. ".repeat(8);
    let mut d = Document::from_text(&text);
    d.settings.auto_hyphenation = true;
    let hyphens: usize = lay(&d).pages[0]
        .items
        .iter()
        .filter_map(|i| if let Placed::Lines { para, .. } = i { Some(para.lines.iter().filter(|x| x.hyphen.is_some()).count()) } else { None })
        .sum();
    assert_eq!(hyphens, 0);
}

/// #1252: highlight and character-shading fills are marked as text backgrounds (a dark page dims
/// them), and so is the text drawn on them; a paragraph's shading is an ordinary fill.
#[test]
fn highlight_fill_is_marked_as_a_text_background() {
    use wordcraft_doc::props::Highlight;
    let cp = wordcraft_doc::CharProps { highlight: Some(Highlight::Yellow), ..Default::default() };
    let mut p = wordcraft_doc::Paragraph::with_text("Marked", cp);
    p.props.shading = Some(wordcraft_doc::Rgb(0xDD, 0xEE, 0xFF));
    let plain = wordcraft_doc::Paragraph::with_text("Plain", Default::default());
    let mut d = Document::from_text("x");
    d.body = vec![wordcraft_doc::para_block(p), wordcraft_doc::para_block(plain)];
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    let fills: Vec<(wordcraft_doc::Rgb, bool)> =
        items.iter().filter_map(|i| if let display::Draw::Fill { color, text_bg, .. } = i { Some((*color, *text_bg)) } else { None }).collect();
    assert!(fills.contains(&(wordcraft_doc::Rgb(255, 255, 0), true)), "highlight is a text background: {fills:?}");
    assert!(fills.contains(&(wordcraft_doc::Rgb(0xDD, 0xEE, 0xFF), false)), "paragraph shading is not: {fills:?}");
    let yellow = Some(wordcraft_doc::Rgb(255, 255, 0));
    let glyphs: Vec<(String, Option<wordcraft_doc::Rgb>)> = items
        .iter()
        .filter_map(|i| if let display::Draw::Glyphs { text, text_bg, .. } = i { Some((text.clone(), *text_bg)) } else { None })
        .collect();
    assert!(glyphs.iter().any(|(t, bg)| t.contains("Marked") && *bg == yellow), "{glyphs:?}");
    assert!(glyphs.iter().any(|(t, bg)| t.contains("Plain") && bg.is_none()), "{glyphs:?}");
}

/// #1252 review: an underline and strikethrough on a highlighted run carry the highlight, like
/// its glyphs; other lines (here a paragraph border) don't.
#[test]
fn underline_on_a_highlight_carries_the_text_background() {
    use wordcraft_doc::props::{Highlight, Underline};
    let cp =
        wordcraft_doc::CharProps { highlight: Some(Highlight::Teal), underline: Some(Underline::Single), strike: Some(true), ..Default::default() };
    let mut d = Document::from_text("x");
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Marked", cp))];
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    let lines: Vec<Option<wordcraft_doc::Rgb>> =
        items.iter().filter_map(|i| if let display::Draw::Line { text_bg, .. } = i { Some(*text_bg) } else { None }).collect();
    assert_eq!(lines, vec![Some(wordcraft_doc::Rgb(0, 128, 128)); 2], "underline and strikethrough");
}

/// #1352 helpers: a body paragraph with a PAGE field after each piece of `pieces` (a piece may
/// hold a page break), after `before` paragraphs that each start a new page.
fn page_field_doc(before: usize, pieces: &[&str]) -> Document {
    let mut d = Document::new();
    let mut body = Vec::new();
    for i in 0..before {
        let mut q = wordcraft_doc::Paragraph::with_text(&format!("Filler {i}"), Default::default());
        q.props.page_break_before = Some(i > 0);
        body.push(wordcraft_doc::para_block(q));
    }
    let mut p = wordcraft_doc::Paragraph::new();
    for piece in pieces {
        let n = p.len();
        p.insert_text(n, piece, &Default::default()).unwrap();
        let n = p.len();
        p.insert_object(
            n,
            InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
            &Default::default(),
        )
        .unwrap();
    }
    p.props.page_break_before = Some(before > 0);
    body.push(wordcraft_doc::para_block(p));
    d.body = body;
    d
}

/// #1352: the field results drawn on each page (only the lines placed there), and the text
/// ranges of body paragraph `block` placed on each page, in order.
fn shown_and_ranges(l: &DocLayout, block: u32) -> (Vec<Vec<String>>, Vec<std::ops::Range<usize>>) {
    let mut shown = Vec::new();
    let mut ranges = Vec::new();
    for page in &l.pages {
        let mut on = Vec::new();
        for it in &page.items {
            let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } = it else { continue };
            if path.0 != [block] {
                continue;
            }
            let (Some(f), Some(e)) = (para.lines.get(*l0), l1.checked_sub(1).and_then(|k| para.lines.get(k))) else { continue };
            on.extend(para.shown.iter().filter(|s| s.0 >= f.c0 && s.0 < e.c1).map(|s| s.1.clone()));
            ranges.push(f.start..e.stop);
        }
        shown.push(on);
    }
    (shown, ranges)
}

/// The placed pieces of a paragraph cover its text once, in order.
fn assert_contiguous(ranges: &[std::ops::Range<usize>], len: usize) {
    let mut at = 0;
    for r in ranges {
        assert_eq!(r.start, at, "piece {r:?} doesn't follow on from {at}: {ranges:?}");
        at = r.end;
    }
    assert_eq!(at, len, "the text up to {len} is placed: {ranges:?}");
}

/// #1352: a PAGE field after a page break inside its paragraph shows the page it is on.
#[test]
fn page_field_after_intra_paragraph_break_shows_its_page() {
    let brk = wordcraft_doc::para::PAGE_BREAK;
    let d = page_field_doc(0, &["A", &format!("{brk}B")]);
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    let (shown, _) = shown_and_ranges(&l, 0);
    assert_eq!(shown, [vec!["1".to_string()], vec!["2".to_string()]]);
}

/// #1352: a paragraph running over onto the next page shows that page's number in its tail.
#[test]
fn page_field_in_a_split_paragraph_tail_shows_its_page() {
    let long = "words that fill the page ".repeat(400);
    let d = page_field_doc(0, &["Head", &long]);
    let l = lay(&d);
    assert!(l.pages.len() >= 2, "{} pages", l.pages.len());
    let (shown, ranges) = shown_and_ranges(&l, 0);
    assert_eq!(shown.first().unwrap(), &["1"]);
    let last = l.pages.len().to_string();
    assert_eq!(shown.last().unwrap(), &[last]);
    assert_contiguous(&ranges, d.para(StoryRef::Body, &Path::top(0)).map(|p| p.len()).unwrap_or(0));
}

/// #1352: from page 9 to page 10 every field gets a digit wider; the lines on page 10 are broken
/// again for the wider results, and the paragraph's text is still placed exactly once.
#[test]
fn page_fields_from_page_nine_to_ten_rebreak_the_rest() {
    let pieces: Vec<String> = (0..900).map(|i| format!(" word{i}")).collect();
    let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
    let d = page_field_doc(8, &pieces);
    let l = lay(&d);
    assert!(l.pages.len() >= 10, "{} pages", l.pages.len());
    let (shown, ranges) = shown_and_ranges(&l, 8);
    let nine = shown.get(8).unwrap();
    let ten = shown.get(9).unwrap();
    assert!(!nine.is_empty() && nine.iter().all(|s| s == "9"), "page 9: {nine:?}");
    assert!(!ten.is_empty() && ten.iter().all(|s| s == "10"), "page 10: {ten:?}");
    assert_eq!(shown.iter().map(Vec::len).sum::<usize>(), 900, "every field drawn once");
    assert_contiguous(&ranges, d.para(StoryRef::Body, &Path::top(8)).map(|p| p.len()).unwrap_or(0));
    // No line overflows its column with the wider results: each ends with the space after a field
    // (which may hang), so the field before it ends within the line's right edge.
    for (pi, page) in l.pages.iter().enumerate() {
        for it in &page.items {
            let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } = it else { continue };
            if path.0 != [8] {
                continue;
            }
            for line in para.lines.get(*l0..*l1).unwrap_or(&[]) {
                let ink = line.xs.len().checked_sub(2).and_then(|k| line.xs.get(k)).copied().unwrap_or(0.0);
                assert!(ink <= line.right + 0.5, "page {}: a line runs to {ink} past its right edge {}", pi + 1, line.right);
            }
        }
    }
}

/// #1352: a footnote's PAGE field, referenced from the part of a paragraph on its second page,
/// shows the page the footnote is on (the paragraph's own page map doesn't leak into it).
#[test]
fn footnote_page_field_from_a_split_paragraph_shows_its_page() {
    let long = "words that fill the page ".repeat(200);
    let mut d = page_field_doc(0, &["Head", &long, "Tail"]);
    let end = d.end_of(StoryRef::Body);
    // A custom mark, so the note marks aren't mistaken for page numbers.
    let id = d.add_part(
        wordcraft_doc::PartKind::Footnote,
        vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Note on page ", Default::default()))],
    );
    d.insert_object(&end, InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: "*".into() }, &Default::default())
        .unwrap();
    let note = d.para_mut(StoryRef::Part(id), &Path::top(0)).unwrap();
    let n = note.len();
    note.insert_object(
        n,
        InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &Default::default(),
    )
    .unwrap();
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2, "the paragraph runs onto a second page");
    // The PAGE results drawn in `story`, by page (not the "*" note marks).
    let fields_in = |story: StoryRef| -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (pi, page) in l.pages.iter().enumerate() {
            for it in page.items.iter().chain(page.footer.iter()) {
                let Placed::Lines { story: s, para, l0, l1, .. } = it else { continue };
                let (Some(f), Some(e)) = (para.lines.get(*l0), l1.checked_sub(1).and_then(|k| para.lines.get(k))) else { continue };
                if *s != story {
                    continue;
                }
                out.extend(para.shown.iter().filter(|x| x.0 >= f.c0 && x.0 < e.c1 && x.1 != "*").map(|x| (pi, x.1.clone())));
            }
        }
        out
    };
    let body = fields_in(StoryRef::Body);
    assert_eq!(body.first(), Some(&(0, "1".to_string())), "{body:?}");
    assert!(body.iter().all(|(pi, t)| *t == (pi + 1).to_string()), "{body:?}");
    assert_eq!(body.last().map(|x| x.0), Some(1), "{body:?}");
    let in_note = fields_in(StoryRef::Part(id));
    assert_eq!(in_note, [(1, "2".to_string())], "the note is on page 2 and says so");
}

/// The automatic equation numbers drawn in each body paragraph (from the layout of its last
/// piece, the one its last lines are drawn with), in order.
fn equation_numbers(l: &DocLayout) -> Vec<String> {
    let mut last: Vec<(u32, Arc<ParaLayout>)> = Vec::new();
    for it in l.pages.iter().flat_map(|p| p.items.iter()) {
        if let Placed::Lines { story: StoryRef::Body, path, para, .. } = it
            && let [b] = path.0[..]
        {
            match last.iter_mut().find(|e| e.0 == b) {
                Some(e) => e.1 = para.clone(),
                None => last.push((b, para.clone())),
            }
        }
    }
    last.sort_by_key(|e| e.0);
    let mut out = Vec::new();
    for (_, pl) in last {
        for (_, m) in &pl.maths {
            let all: String =
                m.items.iter().filter_map(|it| if let crate::math::MItem::Glyphs { text, .. } = it { Some(text.as_str()) } else { None }).collect();
            if let Some(at) = all.rfind('(') {
                out.push(all[at..].to_string());
            }
        }
    }
    out
}

/// Laying a paragraph out again (placed again after a page break before, or the rest of it again
/// for a later page's PAGE fields) doesn't count its numbered equations twice: they read (1), (2), (3).
#[test]
fn relaid_paragraphs_keep_their_equation_numbers() {
    let eq = |s: &str| InlineObject::Equation {
        linear: s.into(),
        display: true,
        math: wordcraft_doc::math::Math { nodes: wordcraft_doc::math::parse_linear(s), ..Default::default() },
    };
    let mut d = Document::from_text("Before\n\nTail");
    // Paragraph 1: a numbered equation on a new page (laid out again there).
    let p1 = d.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    p1.insert_object(0, eq("a=b#"), &Default::default()).unwrap();
    p1.props.page_break_before = Some(true);
    // Paragraph 2: long, with a PAGE field, running onto the next page (the rest laid out again
    // there), then a numbered equation.
    let p2 = d.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    let long = "words that fill the page ".repeat(400);
    p2.insert_text(0, &long, &Default::default()).unwrap();
    let n = p2.len();
    p2.insert_object(
        n,
        InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &Default::default(),
    )
    .unwrap();
    let n = p2.len();
    p2.insert_object(n, eq("c=d#"), &Default::default()).unwrap();
    // Paragraph 3: one more.
    d.body.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::new()));
    let p3 = d.para_mut(StoryRef::Body, &Path::top(3)).unwrap();
    p3.insert_object(0, eq("e=f#"), &Default::default()).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 3, "{} pages", l.pages.len());
    assert_eq!(equation_numbers(&l), ["(1)", "(2)", "(3)"]);
}

/// #1116: a document of a top paragraph with `before` points of space before, filler lines, a
/// paragraph of several lines (block `t`) with 6 pt after and, when `note`, a footnote on its last line.
fn space_after_doc(before: f32, note: bool) -> (Document, usize, Option<u32>) {
    let text = format!("Top.\n{}{}\nAfter.", "Body text line.\n".repeat(14), "Words in a paragraph that wraps. ".repeat(14));
    let mut d = Document::from_text(&text);
    let t = 15;
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.props.space_before = Some(before);
    }
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(t).map(std::sync::Arc::make_mut) {
        p.props.space_after = Some(6.0);
        p.props.widow_control = Some(true);
    }
    let len = d.para(StoryRef::Body, &Path(vec![t as u32])).map(|p| p.text.len()).unwrap();
    let id = note.then(|| footnote(&mut d, &Pos::body(t, len), "The note."));
    (d, t, id)
}

/// The pieces of body paragraph `t` as (page, l0, l1, bottom), and its line count.
fn para_pieces(l: &DocLayout, t: usize) -> (Vec<(usize, usize, usize, f32)>, usize) {
    let mut v = Vec::new();
    let mut n = 0;
    for (pi, p) in l.pages.iter().enumerate() {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, y, .. } = it
                && path.0 == [t as u32]
            {
                n = para.lines.len();
                v.push((pi, *l0, *l1, item_bottom(*y, para, *l0, *l1).unwrap()));
            }
        }
    }
    (v, n)
}

#[test]
fn space_after_stays_clear_of_the_footnote_separator() {
    // #1116, measured in Word: on a page with footnotes a paragraph ending there keeps its space
    // after above the separator, or its last lines (two, with widow control) go to the next page
    // with the note.
    let mut moved = 0;
    for before in 0..=300 {
        let (d, t, id) = space_after_doc(before as f32, true);
        let l = lay(&d);
        let (pieces, n) = para_pieces(&l, t);
        assert!(n >= 4, "a paragraph of several lines: {n}");
        let &(page, _, l1, bottom) = pieces.last().unwrap();
        assert_eq!(l1, n);
        let note = note_pieces(&l, id.unwrap());
        assert_eq!(note.first().map(|p| p.0), Some(page), "before {before}: the note is on its reference's page");
        // The separator's area (default separator: NOTE_SEP high, its rule half way down) sits
        // right above the note.
        let sep_top = note[0].3 - NOTE_SEP;
        assert!(sep_top - bottom >= 6.0 - 0.01, "before {before}: last line ends at {bottom}, the separator starts at {sep_top}");
        if pieces.len() > 1 {
            assert_eq!(pieces.len(), 2, "{pieces:?}");
            assert!(pieces[0].2 - pieces[0].1 >= 2 && pieces[1].2 - pieces[1].1 >= 2, "widow/orphan control: {pieces:?}");
            moved += 1;
        }
    }
    assert!(moved > 0, "the sweep crosses the page bottom");
}

#[test]
fn space_after_may_overflow_the_bottom_margin_without_notes() {
    // Without footnotes nothing changes: the last line only has to fit, its space after may run
    // past the bottom margin (Word).
    let mut tight = 0;
    for before in 0..=300 {
        let (d, t, _) = space_after_doc(before as f32, false);
        let l = lay(&d);
        let bottom_margin = d.last_section.page_h - d.last_section.margin_bottom;
        let (pieces, _) = para_pieces(&l, t);
        let &(_, _, _, bottom) = pieces.last().unwrap();
        assert!(bottom <= bottom_margin + 0.01, "before {before}: {bottom}");
        if bottom > bottom_margin - 6.0 {
            tight += 1;
        }
    }
    assert!(tight > 0, "some step leaves less than the space after below the last line");
}

/// #1116: the sweep of `space_after_stays_clear_of_the_footnote_separator` with paragraphs `t`
/// and `t + 1` changed by `tweak`: the last line of `t` stays `need` points clear of the separator.
fn space_after_clear_with(tweak: impl Fn(&mut Document, usize), need: f32) {
    for before in 0..=300 {
        let (mut d, t, id) = space_after_doc(before as f32, true);
        tweak(&mut d, t);
        let l = lay(&d);
        let (pieces, _) = para_pieces(&l, t);
        let &(page, _, _, bottom) = pieces.last().unwrap();
        let note = note_pieces(&l, id.unwrap());
        assert_eq!(note.first().map(|p| p.0), Some(page), "before {before}: the note is on its reference's page");
        let sep_top = note[0].3 - NOTE_SEP;
        assert!(sep_top - bottom >= need - 0.01, "before {before}: last line ends at {bottom}, the separator starts at {sep_top}");
    }
}

fn para_props(d: &mut Document, i: usize) -> &mut ParaProps {
    match d.body.get_mut(i).map(std::sync::Arc::make_mut) {
        Some(wordcraft_doc::Block::Para(p)) => &mut p.props,
        _ => panic!("no paragraph {i}"),
    }
}

#[test]
fn space_after_stays_clear_when_only_the_next_paragraph_has_contextual_spacing() {
    // Contextual spacing on the next paragraph alone drops its space before, not this one's space
    // after (as the spacing between them): the space after still stays clear of the separator.
    space_after_clear_with(|d, t| para_props(d, t + 1).contextual_spacing = Some(true), 6.0);
}

#[test]
fn space_after_stays_clear_before_a_numbered_paragraph_without_a_label() {
    // Automatic spacing drops only between two list items with labels: before a paragraph whose
    // list doesn't exist (no label) a list item's automatic space after stays clear.
    use wordcraft_doc::props::NumRef;
    space_after_clear_with(
        |d, t| {
            let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
            let p = para_props(d, t);
            p.numbering = Some(NumRef { num, level: 0 });
            p.space_after_auto = Some(true);
            para_props(d, t + 1).numbering = Some(NumRef { num: 9999, level: 0 });
        },
        wordcraft_doc::resolve::AUTO_SPACING,
    );
}

#[test]
fn cell_widths_override_grid() {
    for fixed in [false, true] {
        let mut t = Table::new(2, 3, 450.0);
        t.props.fixed = fixed;
        t.grid = vec![40.0, 175.0, 235.0];
        for row in &mut t.rows {
            for (cell, width) in row.cells.iter_mut().zip([50.0, 60.0, 340.0]) {
                cell.props.width = Some(width);
                cell.blocks = vec![wordcraft_doc::para_block(Paragraph::with_text("A", Default::default()))];
            }
        }
        let mut d = Document::from_text("After");
        d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(t)).unwrap();
        let l = lay(&d);
        let [(x0, _, _)] = para_at(&l, &[0, 0, 0, 0])[..] else { panic!("cell missing") };
        let [(x1, _, _)] = para_at(&l, &[0, 0, 1, 0])[..] else { panic!("cell missing") };
        let [(x2, _, _)] = para_at(&l, &[0, 0, 2, 0])[..] else { panic!("cell missing") };
        assert!((x1 - x0 - 50.0).abs() < 0.01);
        assert!((x2 - x0 - 110.0).abs() < 0.01);
    }
}

#[test]
fn ambiguous_widths_keep_grid() {
    let mut t = Table::new(2, 3, 450.0);
    t.grid = vec![40.0, 175.0, 235.0];
    for row in &mut t.rows {
        for (cell, width) in row.cells.iter_mut().zip([50.0, 60.0, 340.0]) {
            cell.props.width = Some(width);
            cell.blocks = vec![wordcraft_doc::para_block(Paragraph::with_text("A", Default::default()))];
        }
    }
    let mut conflicting = t.clone();
    conflicting.rows[1].cells[0].props.width = Some(60.0);
    let mut percentage = t.clone();
    percentage.rows[0].cells[0].props.width_pct = Some(10.0);
    let mut invalid = t.clone();
    invalid.rows[0].cells[0].props.width = Some(f32::NAN);
    let mut different_total = t.clone();
    different_total.rows.iter_mut().for_each(|r| r.cells[0].props.width = Some(60.0));
    let mut spanned = t;
    spanned.rows[0].cells.remove(1);
    spanned.rows[0].cells[0].props.span = 2;
    for table in [conflicting, percentage, invalid, different_total, spanned] {
        let mut d = Document::from_text("After");
        d.insert_block(StoryRef::Body, &Path::top(0), wordcraft_doc::Block::Table(table)).unwrap();
        let l = lay(&d);
        let [(x0, _, _)] = para_at(&l, &[0, 1, 0, 0])[..] else { panic!("cell missing") };
        let [(x1, _, _)] = para_at(&l, &[0, 1, 1, 0])[..] else { panic!("cell missing") };
        assert!((x1 - x0 - 40.0).abs() < 0.01);
    }
}

/// #1374 helpers: a page-anchored, square-wrapped rectangle `w`×`h` at (`x`, `y`) on the page.
fn page_shape(x: f32, y: f32, w: f32, h: f32) -> InlineObject {
    use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, x, y, dist: 9.0, ..Default::default() };
    InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

/// #1374: a two-column (`cols`) document: paragraph 0 holds `shape` (if any), paragraph 1 is
/// `words` numbered words, long enough to run on from where it starts.
fn column_wrap_doc(cols: u32, shape: Option<InlineObject>, words: usize) -> Document {
    let text: String = (0..words).map(|i| format!("word{i} ")).collect();
    let mut d = Document::from_text(&format!("Anchor\n{}", text.trim_end()));
    d.last_section.columns.count = cols;
    if let Some(s) = shape {
        d.insert_object(&Pos::body(0, 0), s, &Default::default()).unwrap();
    }
    d
}

/// #1374: every line of body paragraph `block` drawn on page `page` whose x is `col_x` (one column):
/// its page box (left, top, right, bottom).
fn line_boxes(l: &DocLayout, page: usize, block: u32, col_x: f32) -> Vec<(f32, f32, f32, f32)> {
    let mut v = Vec::new();
    for it in &l.pages[page].items {
        let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } = it else { continue };
        if path.0 != [block] || (x - col_x).abs() > 1.0 {
            continue;
        }
        let first = &para.lines[*l0];
        for line in &para.lines[*l0..*l1] {
            let top = y + (line.top - first.top);
            v.push((x + line.left, top, x + line.right, top + line.height));
        }
    }
    v
}

/// #1374: no line in `lines` runs through `r` (grown by the shape's 9 pt wrap distance at the sides).
fn assert_clear_of(lines: &[(f32, f32, f32, f32)], r: Rect, what: &str) {
    let mut beside = 0;
    for &(l, t, rt, b) in lines {
        if b <= r.y + 0.01 || t >= r.bottom() - 0.01 {
            continue;
        }
        beside += 1;
        assert!(l >= r.right() + 8.5 || rt <= r.x - 8.5, "{what}: line {l}..{rt} at {t}..{b} runs through the shape {r:?}");
    }
    assert!(beside > 0, "{what}: some lines are beside the shape: {lines:?}");
}

/// #1374: a paragraph that runs on from column 1 into column 2 wraps around a shape in column 2.
#[test]
fn paragraph_continued_into_next_column_wraps_around_a_shape_there() {
    let r = Rect::new(340.0, 300.0, 100.0, 100.0);
    let d = column_wrap_doc(2, Some(page_shape(r.x, r.y, r.w, r.h)), 450);
    let l = lay(&d);
    let (_, ranges) = shown_and_ranges(&l, 1);
    assert_contiguous(&ranges, d.para(StoryRef::Body, &Path::top(1)).map(|p| p.len()).unwrap_or(0));
    let col2 = l.pages[0].body.x + d.last_section.column_boxes()[1].0;
    assert!(ranges.len() >= 2, "the paragraph runs into column 2: {ranges:?}");
    assert_clear_of(&line_boxes(&l, 0, 1, col2), r, "column 2");
}

/// #1374: a shape far to the left of the next column (in the page margin, beyond a column's width
/// from it) doesn't make the paragraph going into that column be laid out again.
#[test]
fn paragraph_continued_into_next_column_ignores_shapes_well_clear_of_it() {
    let d = column_wrap_doc(2, Some(page_shape(0.0, 300.0, 20.0, 100.0)), 450);
    WRAP_RELAYS.with(|c| c.set(0));
    let l = lay(&d);
    assert_eq!(WRAP_RELAYS.with(|c| c.get()), 0, "no wrap relay");
    let (_, ranges) = shown_and_ranges(&l, 1);
    assert_contiguous(&ranges, d.para(StoryRef::Body, &Path::top(1)).map(|p| p.len()).unwrap_or(0));
}

/// #1374: lines continued onto a new page are broken for that page: a shape on the page before
/// (reaching below its body) doesn't push them aside there.
#[test]
fn paragraph_continued_onto_next_page_is_broken_for_that_page() {
    // On page 1 from 640 down past the page bottom, at the left of the text.
    let d = column_wrap_doc(1, Some(page_shape(72.0, 640.0, 150.0, 150.0)), 900);
    let l = lay(&d);
    assert!(l.pages.len() >= 2, "{} pages", l.pages.len());
    let (_, ranges) = shown_and_ranges(&l, 1);
    assert_contiguous(&ranges, d.para(StoryRef::Body, &Path::top(1)).map(|p| p.len()).unwrap_or(0));
    // Page 1's lines beside the shape keep clear of it; page 2 has nothing to wrap around.
    let page1 = line_boxes(&l, 0, 1, 72.0);
    assert_clear_of(&page1, Rect::new(72.0, 640.0, 150.0, 150.0), "page 1");
    let page2 = line_boxes(&l, 1, 1, 72.0);
    assert!(!page2.is_empty());
    for (left, top, ..) in page2 {
        assert!(left < 72.5, "page 2: the line at {top} starts at {left}, beside a shape that isn't on this page");
    }
}

/// #1374: with nothing to wrap around, the lines going into the next column are those broken in
/// the first: the paragraph isn't laid out again (no wrap relay), and both pieces share one layout.
#[test]
fn paragraph_continued_into_next_column_without_shapes_is_not_laid_out_again() {
    let d = column_wrap_doc(2, None, 450);
    WRAP_RELAYS.with(|c| c.set(0));
    let l = lay(&d);
    assert_eq!(WRAP_RELAYS.with(|c| c.get()), 0, "no wrap relay");
    let pieces: Vec<&Arc<ParaLayout>> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, para, .. } if path.0 == [1] => Some(para),
            _ => None,
        })
        .collect();
    assert_eq!(pieces.len(), 2, "one piece per column");
    assert!(Arc::ptr_eq(pieces[0], pieces[1]), "the second column reuses the first column's layout");
}

/// #1374: the paragraph's own floating object stays where it was placed (at its start, in column 1)
/// when its lines in column 2 are broken again around a shape there.
#[test]
fn paragraph_own_float_is_not_replaced_when_its_rest_wraps_in_the_next_column() {
    use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
    let r = Rect::new(340.0, 300.0, 100.0, 100.0);
    let mut d = column_wrap_doc(2, Some(page_shape(r.x, r.y, r.w, r.h)), 450);
    let own = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, dist: 9.0, ..Default::default() };
    let own = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 80.0,
        h: 60.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: own,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(1, 0), own, &Default::default()).unwrap();
    let l = lay(&d);
    let owns: Vec<(usize, Rect)> = l
        .pages
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| p.items.iter().map(move |it| (pi, it)))
        .filter_map(|(pi, it)| match it {
            Placed::Shape { rect, .. } if (rect.w - 80.0).abs() < 0.5 => Some((pi, *rect)),
            _ => None,
        })
        .collect();
    let col2 = l.pages[0].body.x + d.last_section.column_boxes()[1].0;
    let boxes = para_boxes(&l);
    assert_eq!(owns.len(), 1, "the own float is drawn once: {owns:?}");
    let (pi, own_rect) = owns[0];
    assert_eq!(pi, 0);
    assert!((own_rect.x - 72.0).abs() < 0.5, "in column 1: {own_rect:?}");
    assert!((own_rect.y - boxes[1].2).abs() < 20.0, "at the paragraph's top {}: {own_rect:?}", boxes[1].2);
    let (_, ranges) = shown_and_ranges(&l, 1);
    assert_contiguous(&ranges, d.para(StoryRef::Body, &Path::top(1)).map(|p| p.len()).unwrap_or(0));
    assert_clear_of(&line_boxes(&l, 0, 1, col2), r, "column 2");
}

/// "Straße" laid out with `f` applied to all of it: its first line's layout.
fn strasse_with(f: &dyn Fn(&mut wordcraft_doc::props::CharProps)) -> ParaLayout {
    let mut d = Document::from_text("Straße");
    d.format_range(&Pos::body(0, 0), &Pos::body(0, "Straße".len()), f).unwrap();
    let l = lay(&d);
    l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some((**para).clone()) } else { None }).expect("a laid-out line")
}

/// Text clusters of the first line: (start, end, glyph ids, left x, advance).
fn text_clusters(pl: &ParaLayout) -> Vec<(usize, usize, Vec<u32>, f32, f32)> {
    let line = &pl.lines[0];
    (line.c0..line.c1)
        .filter(|&k| pl.clusters[k].kind == para::ClKind::Text)
        .map(|k| {
            let c = &pl.clusters[k];
            let gids = pl.glyphs[c.g0 as usize..c.g1 as usize].iter().map(|g| g.gid).collect();
            (c.start, c.end, gids, line.cl_left(k).unwrap(), c.adv)
        })
        .collect()
}

/// All Caps and Small Caps draw ß as SS, as Word does, and the ß stays one character for the
/// caret, selection and hit testing (#1581).
#[test]
fn caps_draw_sharp_s_as_two_capitals_in_one_cluster() {
    let width = |pl: &ParaLayout| text_clusters(pl).iter().map(|c| c.4).sum::<f32>();
    let plain_strase = {
        let d = Document::from_text("STRASE");
        let l = lay(&d);
        let pl = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some((**para).clone()) } else { None }).unwrap();
        width(&pl)
    };
    let caps: &dyn Fn(&mut wordcraft_doc::props::CharProps) = &|c| c.caps = Some(true);
    let small: &dyn Fn(&mut wordcraft_doc::props::CharProps) = &|c| c.small_caps = Some(true);
    for (name, f) in [("All Caps", caps), ("Small Caps", small)] {
        let pl = strasse_with(f);
        let cl = text_clusters(&pl);
        let glyphs: usize = cl.iter().map(|c| c.2.len()).sum();
        assert_eq!(glyphs, 7, "{name}: S T R A S S E is seven glyphs: {cl:?}");
        assert_eq!(cl.len(), 6, "{name}: six source characters, six clusters: {cl:?}");
        let ss = cl.iter().find(|c| c.0 == 4).expect("a cluster for ß");
        assert_eq!(ss.1, 6, "{name}: the ß cluster covers exactly the ß");
        // Both glyphs drawn for ß are the S that starts the word.
        let s_gid = cl.first().and_then(|c| c.2.first().copied());
        assert_eq!(ss.2.len(), 2, "{name}: ß draws two glyphs");
        assert!(ss.2.iter().all(|g| Some(*g) == s_gid), "{name}: both glyphs are S");
        if name == "All Caps" {
            assert!(width(&pl) > plain_strase + 1.0, "All Caps: STRASSE is wider than STRASE: {} vs {plain_strase}", width(&pl));
        }
        // Caret stops: one before each source character, and the end; none inside ß.
        assert_eq!(pl.line_offsets(0), vec![0, 1, 2, 3, 4, 6, 7], "{name}");
        // A click on the first S lands before the ß, on the second S after it.
        assert_eq!(pl.off_at_x(0, ss.3 + ss.4 * 0.25), 4, "{name}: click on the first S");
        assert_eq!(pl.off_at_x(0, ss.3 + ss.4 * 0.75), 6, "{name}: click on the second S");
        if name == "Small Caps" {
            // The ß is a lowercase letter: drawn at the small-caps size, like the "t", not
            // at the size of the capital S that starts the word.
            let style = |off: usize| pl.clusters.iter().find(|c| c.start == off).map(|c| c.style);
            assert_eq!(style(4), style(1), "Small Caps: ß is scaled like t");
            assert_ne!(style(4), style(0), "Small Caps: ß is not the size of the first S");
        }
    }
    // Without caps, ß is one glyph of its own.
    let pl = strasse_with(&|c| c.bold = Some(false));
    let cl = text_clusters(&pl);
    assert_eq!(cl.len(), 6);
    assert_eq!(cl.iter().map(|c| c.2.len()).sum::<usize>(), 6, "{cl:?}");
}

/// A caps run in a symbol font draws each code as its symbol: ß never becomes two glyphs (#1581).
#[test]
fn caps_in_a_symbol_font_never_expand_characters() {
    let pl = strasse_with(&|c| {
        c.caps = Some(true);
        c.font = Some("Wingdings".into());
    });
    let cl = text_clusters(&pl);
    assert_eq!(cl.len(), 6, "{cl:?}");
    let ss = cl.iter().find(|c| c.0 == 4).expect("a cluster for the ß code");
    assert_eq!((ss.1, ss.2.len()), (6, 1), "the ß code is one symbol: {cl:?}");
}

/// #1320: a section's vertical alignment (`w:vAlign`, ECMA-376 §17.6.23). Justified shares the
/// room left on each page out equally between the page's blocks, so the first line stays at the
/// top margin and the last one sits on the bottom margin (Word for Mac 16.113.4): a partly filled
/// last page too, and a page ending in a manual page break, where the paragraph holding the break
/// is the page's last block. A page with one block keeps it at the top.
#[test]
fn section_vertical_alignment_top_center_justify_bottom() {
    use wordcraft_doc::section::PageVAlign;
    let para = |text: &str| {
        let mut p = wordcraft_doc::Paragraph::with_text(text, Default::default());
        p.props = ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
        wordcraft_doc::para_block(p)
    };
    let laid = |texts: &[&str], va: PageVAlign| {
        let mut d = Document::new();
        d.body = texts.iter().map(|t| para(t)).collect();
        d.last_section.valign = va;
        lay(&d)
    };
    let three = ["One", "Two", "Three"];
    let top = laid(&three, PageVAlign::Top);
    let body = top.pages[0].body;
    let (body_top, body_bottom) = (body.y, body.y + body.h);
    let t = para_boxes(&top);
    let h: f32 = t[2].3 - t[0].2;
    assert!((t[0].2 - body_top).abs() < 0.5, "top: at the top margin {t:?} {body:?}");
    let c = para_boxes(&laid(&three, PageVAlign::Center));
    let room = body_bottom - body_top - h;
    assert!((c[0].2 - (body_top + room / 2.0)).abs() < 0.5, "center: {c:?}");
    assert!((c[1].2 - c[0].3).abs() < 0.05 && (c[2].2 - c[1].3).abs() < 0.05, "center keeps the blocks together: {c:?}");
    let b = para_boxes(&laid(&three, PageVAlign::Bottom));
    assert!((b[2].3 - body_bottom).abs() < 0.5, "bottom: {b:?} {body:?}");
    // Justified: first at the top, last on the bottom margin, equal gaps.
    let j = para_boxes(&laid(&three, PageVAlign::Justify));
    assert!((j[0].2 - body_top).abs() < 0.5, "justify: first line at the top margin {j:?}");
    assert!((j[2].3 - body_bottom).abs() < 0.5, "justify: last line on the bottom margin {j:?} {body:?}");
    let (g1, g2) = (j[1].2 - j[0].3, j[2].2 - j[1].3);
    assert!(g1 > 100.0 && (g1 - g2).abs() < 0.05, "justify: equal gaps {g1} {g2}");
    // One block on the page stays at the top.
    let one = para_boxes(&laid(&["Only"], PageVAlign::Justify));
    assert!((one[0].2 - body_top).abs() < 0.5, "a single block stays at the top: {one:?}");
    // A manual page break: its paragraph is page 1's last block; "Four" alone on page 2 stays up.
    let brk = wordcraft_doc::para::PAGE_BREAK.to_string();
    let p = para_boxes(&laid(&["One", "Two", "Three", &brk, "Four"], PageVAlign::Justify));
    assert_eq!(p.iter().map(|e| e.0).collect::<Vec<_>>(), vec![0, 0, 0, 0, 1], "{p:?}");
    assert!((p[0].2 - body_top).abs() < 0.5, "{p:?}");
    assert!((p[3].3 - body_bottom).abs() < 0.5, "the break's paragraph sits on the bottom margin: {p:?}");
    let gaps: Vec<f32> = (1..4).map(|i| p[i].2 - p[i - 1].3).collect();
    assert!(gaps.iter().all(|g| (g - gaps[0]).abs() < 0.05 && *g > 50.0), "{gaps:?}");
    assert!((p[4].2 - body_top).abs() < 0.5, "Four keeps to the top of page 2: {p:?}");
}

/// #1320 review: a section's last page takes that section's vertical alignment, not the next
/// section's (Justified then Top: spread; Top then Justified: stays at the top).
#[test]
fn a_sections_last_page_keeps_its_own_vertical_alignment() {
    use wordcraft_doc::section::{PageVAlign, SectionProps};
    let para = |text: &str| {
        let mut p = wordcraft_doc::Paragraph::with_text(text, Default::default());
        p.props = ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
        wordcraft_doc::para_block(p)
    };
    let laid = |first: PageVAlign, second: PageVAlign| {
        let mut d = Document::new();
        d.body = ["One", "Two", "Three", "Next section"].iter().map(|t| para(t)).collect();
        d.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section = Some(Box::new(SectionProps { valign: first, ..Default::default() }));
        d.last_section.valign = second;
        lay(&d)
    };
    let l = laid(PageVAlign::Justify, PageVAlign::Top);
    let body = l.pages[0].body;
    let b = para_boxes(&l);
    assert_eq!(b.iter().map(|e| e.0).collect::<Vec<_>>(), vec![0, 0, 0, 1]);
    assert!((b[0].2 - body.y).abs() < 0.5 && (b[2].3 - (body.y + body.h)).abs() < 0.5, "section 1 justified: {b:?} {body:?}");
    assert!((b[3].2 - body.y).abs() < 0.5, "section 2 at the top: {b:?}");
    let b = para_boxes(&laid(PageVAlign::Top, PageVAlign::Justify));
    assert!((b[1].2 - b[0].3).abs() < 0.05 && (b[2].2 - b[1].3).abs() < 0.05, "section 1 stays at the top: {b:?}");
}

/// Word for Mac 16.113.4 (user-checked, #1320): a justified page keeps its last paragraph's space
/// after inside the text area, so "Three" with 72 pt after sits 72 pt above the bottom margin.
#[test]
fn justified_page_keeps_the_last_paragraphs_space_after() {
    let mut d = Document::new();
    d.body = ["One", "Two", "Three"]
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut p = wordcraft_doc::Paragraph::with_text(t, Default::default());
            p.props = ParaProps { space_before: Some(0.0), space_after: Some(if i == 2 { 72.0 } else { 0.0 }), ..Default::default() };
            wordcraft_doc::para_block(p)
        })
        .collect();
    d.last_section.valign = wordcraft_doc::section::PageVAlign::Justify;
    let l = lay(&d);
    let body = l.pages[0].body;
    let b = para_boxes(&l);
    assert!((b[0].2 - body.y).abs() < 0.5, "{b:?}");
    assert!((body.y + body.h - b[2].3 - 72.0).abs() < 0.5, "72 pt left under Three: {b:?} {body:?}");
    assert!((b[1].2 - b[0].3 - (b[2].2 - b[1].3)).abs() < 0.05, "equal gaps: {b:?}");
}

/// #1320: a justified page with several newspaper columns stays at the top (not spread).
#[test]
fn justified_multi_column_pages_stay_at_the_top() {
    let laid = |va| {
        let mut d = two_column_section(wordcraft_doc::section::SectionStart::NextPage, Vec::new());
        d.para_mut(StoryRef::Body, &Path::top(9)).unwrap().section.as_mut().unwrap().valign = va;
        para_boxes(&lay(&d))
    };
    use wordcraft_doc::section::PageVAlign;
    assert_eq!(laid(PageVAlign::Justify), laid(PageVAlign::Top));
    // The document's last section in two columns.
    let last = |va| {
        let mut d = Document::from_text("One\nTwo\nThree\nFour");
        d.last_section.columns.count = 2;
        d.last_section.valign = va;
        para_boxes(&lay(&d))
    };
    assert_eq!(last(PageVAlign::Justify), last(PageVAlign::Top));
}

#[test]
fn row_borders_stay_on_row() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(2, 2, 200.0);
    t.props.style = None;
    t.rows[0].props.exceptions =
        Some(Box::new(wordcraft_doc::props::TableProps { borders: Some(Borders::all(Border::single(1.0))), ..Default::default() }));
    t.rows[0].cells[0].props.borders = Some(Borders { top: Some(Border::single(2.0)), ..Default::default() });
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    let rules = rules_on_page0(&lay(&d));
    assert_eq!(rules.len(), 8);
    assert_eq!(rules.iter().filter(|w| **w == 2.0).count(), 1);
    assert_eq!(rules.iter().filter(|w| **w == 1.0).count(), 7);
}

#[test]
fn row_nil_overrides_style() {
    let mut d = Document::from_text("before\nafter");
    let mut t = Table::new(1, 1, 200.0);
    t.rows[0].props.exceptions = Some(Box::new(wordcraft_doc::props::TableProps {
        borders: Some(Borders::all(Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 })),
        ..Default::default()
    }));
    t.rows[0].cells[0].props.borders = Some(Borders { right: Some(Border::single(2.0)), ..Default::default() });
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
    assert_eq!(rules_on_page0(&lay(&d)), vec![2.0]);
}

/// One paragraph of `words` words in a section with these column `widths` (Word's Left and
/// Right presets are a narrow and a wide column), on a landscape page so three columns fit.
fn unequal_columns_doc(widths: Vec<(f32, f32)>, text: &str) -> Document {
    let mut d = Document::new();
    d.last_section.page_w = 792.0;
    d.last_section.columns = wordcraft_doc::section::Columns { count: widths.len() as u32, space: 24.0, separator: false, widths };
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))];
    d
}

fn words(n: usize) -> String {
    (0..n).map(|i| ["lorem", "ipsum", "dolor", "sit", "amet"][i % 5]).collect::<Vec<_>>().join(" ")
}

/// The pieces of the paragraph on the first page: (x, lines, widest line), a line's width
/// measured to its last cluster that isn't a space (a trailing space may run past the edge).
fn pieces(l: &DocLayout) -> Vec<(f32, usize, f32)> {
    l.pages[0]
        .items
        .iter()
        .filter_map(|it| {
            let Placed::Lines { story: StoryRef::Body, para, l0, l1, x, .. } = it else { return None };
            let widest = para.lines[*l0..*l1]
                .iter()
                .filter_map(|line| {
                    let k = (line.c0..line.c1).rev().find(|k| !matches!(para.clusters[*k].kind, para::ClKind::Space | para::ClKind::Marker))?;
                    Some(line.xs[k - line.c0] + para.clusters[k].adv)
                })
                .fold(0.0f32, f32::max);
            Some((*x, l1 - l0, widest))
        })
        .collect()
}

/// Lays out `n` words in two columns `first_w` and `second_w` wide and checks the paragraph starts
/// in the first and runs into the second, each piece's lines as wide as its column at most;
/// returns the second piece's widest line.
fn runs_into_the_second_column(first_w: f32, second_w: f32, n: usize) -> f32 {
    let l = lay(&unequal_columns_doc(vec![(first_w, 24.0), (second_w, 0.0)], &words(n)));
    let p = pieces(&l);
    let [(x0, lines0, w0), (x1, lines1, w1), ..] = p[..] else { panic!("two pieces on the first page: {p:?}") };
    // Not moved whole for an orphan: the paragraph starts in the first column.
    assert!(lines0 >= 2 && (x0 - 72.0).abs() < 0.5, "first piece in the first column: {p:?}");
    assert!((x1 - (96.0 + first_w)).abs() < 0.5 && lines1 >= 2, "second piece in the second column: {p:?}");
    assert!(w0 <= first_w + 0.5, "the first column's lines fit it: {p:?}");
    assert!(w1 <= second_w + 0.5, "the second column's lines fit it: {p:?}");
    w1
}

/// Word's Left preset (#2382): a paragraph running on from a narrow column into a wide one is
/// broken for the wide one. WordCraft kept the narrow column's lines down the wide column.
#[test]
fn a_paragraph_running_into_a_wider_column_takes_its_width() {
    let w = runs_into_the_second_column(100.0, 524.0, 300);
    assert!(w > 400.0, "broken for the 524 pt column, not the 100 pt one: widest line {w}");
}

/// Word's Right preset (#2382): a paragraph running on from a wide column into a narrow one is
/// broken for the narrow one. WordCraft kept the wide lines, running over the column into the margin.
#[test]
fn a_paragraph_running_into_a_narrower_column_takes_its_width() {
    runs_into_the_second_column(524.0, 100.0, 1100);
}

/// A paragraph running from a narrow column into a wide one, then on after a column break into a
/// third, narrower than the second: the text after the break is broken for the third column only,
/// not for the second's width it ran into before (#2382, with #934's column break).
#[test]
fn a_column_break_after_running_into_a_wider_column_takes_the_third_column_s_width() {
    let brk = wordcraft_doc::para::COLUMN_BREAK;
    let text = format!("{}{brk}{}", words(200), words(120));
    WIDTH_RELAYS.with(|c| c.set(0));
    let l = lay(&unequal_columns_doc(vec![(100.0, 24.0), (300.0, 24.0), (200.0, 0.0)], &text));
    // Broken again once, for the second column: the break lays it out afresh for the third.
    assert_eq!(WIDTH_RELAYS.with(|c| c.get()), 1, "one relay for the second column");
    let p = pieces(&l);
    let [(x0, lines0, _), (x1, lines1, w1), (x2, lines2, w2)] = p[..] else { panic!("three pieces on the first page: {p:?}") };
    assert!(lines0 >= 2 && (x0 - 72.0).abs() < 0.5, "starts in the first column: {p:?}");
    assert!((x1 - 196.0).abs() < 0.5 && lines1 >= 2 && w1 > 200.0 && w1 <= 300.5, "runs into the 300 pt column: {p:?}");
    assert!((x2 - 520.0).abs() < 0.5 && lines2 >= 2 && w2 <= 200.5, "after the break, broken for the 200 pt column: {p:?}");
}

/// Equal columns: a paragraph running from one into the next is not broken again (#2382).
#[test]
fn a_paragraph_running_into_a_column_of_the_same_width_is_not_broken_again() {
    WIDTH_RELAYS.with(|c| c.set(0));
    let l = lay(&unequal_columns_doc(vec![(312.0, 24.0), (312.0, 0.0)], &words(800)));
    assert!(pieces(&l).len() >= 2, "runs into the second column");
    assert_eq!(WIDTH_RELAYS.with(|c| c.get()), 0);
}
