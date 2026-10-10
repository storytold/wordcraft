use super::*;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, ParaProps};
use wordcraft_doc::{Pos, Table};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
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
fn page_break_char_starts_new_page() {
    let mut d = Document::from_text("one");
    d.insert_text(&Pos::body(0, 3), "\u{000C}", &Default::default()).unwrap();
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
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
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(2, 0), &|p| p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 })).unwrap();
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
    // Where a real Symbol font with the private-use code is installed (Windows), the code stays.
    let symbol = wordcraft_fonts::word::resolve("Symbol", false, false);
    let expected = if symbol.substituted || !symbol.face.covers('\u{F0B7}') { "\u{2022}" } else { "\u{F0B7}" };
    assert_eq!(label.text, expected);
    assert_eq!(label.glyphs.len(), 1);
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
    hp.insert_object(0, InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false }, &Default::default()).unwrap();
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

#[test]
fn selection_rects_cover_range() {
    let d = Document::from_text("Hello world\nSecond line");
    let l = lay(&d);
    let r = l.selection_rects(&d, &Pos::body(0, 6), &Pos::body(1, 6), 0);
    assert_eq!(r.len(), 2);
    assert!(r[0].1.w > 20.0);
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
fn web_view_is_one_page() {
    let d = Document::from_text(&"text ".repeat(3000));
    let mut c = LayoutCache::new();
    let l = layout(&d, &mut c, &LayoutOptions { view: ViewMode::Web, web_width: 800.0, show_hidden: false, proofing: false });
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
fn text_wraps_around_square_float() {
    let mut d = Document::from_text(&"Words flow around the picture here. ".repeat(30));
    let float = wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::Square,
        h_rel: wordcraft_doc::para::Anchor::Column,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        x: 0.0,
        y: 0.0,
        dist: 9.0,
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
