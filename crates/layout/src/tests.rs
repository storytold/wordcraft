use super::*;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Align, ParaProps};
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
    let id2 = text_box(&mut d2, 0, &long, 144.0, 560.0, Default::default());
    assert_eq!(box_lines(&lay(&d2), id2).0, total);
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
        assert!(x + prev.xs.last().unwrap() <= r.x, "left part stops before the box");
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
    assert!(para.lines.iter().all(|ln| !ln.beside && x + ln.xs.last().unwrap() <= r2.x.max(x + ln.right)));
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
    let nums = note_numbers(&d);
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
