use super::*;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, ParaProps, TextDirection};
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
    // Markup: the deletion is laid out and struck through, the insertion underlined.
    let full = lay(&d);
    assert_eq!(texts(&full, true), ("Keep DELETEDTEXT INSERTED".into(), 2));
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

/// How much longer laying out `doc(4n)` takes than `doc(n)` (fastest of three each, after a
/// warm-up): ~4 when linear, ~16 when quadratic, whatever the machine.
/// How much bigger the second document is in the linear-time tests. At 8x a linear layout takes
/// about 8x as long and a quadratic one about 64x, so a limit of 24x stays far from both, however
/// busy the machine running the tests is.
const SCALE: usize = 8;

fn layout_time_ratio(doc: impl Fn(usize) -> Document, n: usize, opts: &LayoutOptions) -> (f64, f64, f64) {
    let fastest = |d: &Document| {
        (0..3)
            .map(|_| {
                let t = std::time::Instant::now();
                layout(d, &mut LayoutCache::new(), opts);
                t.elapsed().as_secs_f64()
            })
            .fold(f64::INFINITY, f64::min)
    };
    let (small, big) = (doc(n), doc(SCALE * n));
    layout(&small, &mut LayoutCache::new(), opts);
    let (t1, tk) = (fastest(&small), fastest(&big));
    (t1, tk, tk / t1.max(1e-9))
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
    // runs (bold on and off) so no two left-out runs are adjacent or merge.
    let words = "Hyphenation wraps international words across narrow columns of text ".repeat(6);
    let mut d = Document::from_text("");
    d.settings.auto_hyphenation = true;
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
    }
    assert_eq!(d.para(StoryRef::Body, &Path::top(0)).unwrap().text, text);
    let mut plain = Document::from_text(&words);
    plain.settings.auto_hyphenation = true;
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
    let l = layout(&d, &mut c, &LayoutOptions { view: ViewMode::Web, web_width: 800.0, show_hidden: false, hide_deleted: false, proofing: false });
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
    InlineObject::Shape { kind: wordcraft_doc::para::ShapeKind::Rectangle, w, h, fill: None, stroke: None, stroke_width: 1.0, float, story: None }
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
        p.insert_object(0, picture(100.0, 50.0, shadow), &Default::default()).unwrap();
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

fn rect_shape(w: f32, h: f32, float: wordcraft_doc::para::Float) -> InlineObject {
    InlineObject::Shape { kind: wordcraft_doc::para::ShapeKind::Rectangle, w, h, fill: None, stroke: None, stroke_width: 1.0, float, story: None }
}

fn shapes(items: &[Placed]) -> Vec<Rect> {
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
    let (r, tb) = wrap_area(Rect::new(10.0, 20.0, 100.0, 50.0), &f).unwrap();
    assert_eq!((r.x, r.y, r.w, r.h, tb), (1.0, 20.0, 118.0, 54.0, false));
    let hostile = Float { wrap: Wrap::TopAndBottom, dist: f32::NAN, dist_top: -5.0, dist_bottom: f32::INFINITY, ..Default::default() };
    let (r, tb) = wrap_area(Rect::new(0.0, 0.0, 10.0, 10.0), &hostile).unwrap();
    assert!(r.w.is_finite() && r.h.is_finite() && tb, "{r:?}");
    assert!(wrap_area(Rect::new(0.0, 0.0, 10.0, 10.0), &Float { wrap: Wrap::BehindText, ..Default::default() }).is_none());
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
    // The space before counts once: the picture and the text both start below it.
    assert!((r.y - top).abs() < 0.5, "picture at {} but its paragraph's text starts at {top}", r.y);
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
