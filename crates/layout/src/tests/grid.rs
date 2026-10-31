//! Document grid (`w:docGrid`): line pitch and character pitch (#391). Assertions are relations
//! to the pitch, never font metrics (fonts differ between machines; CJK glyphs may be missing).

#[allow(unused_imports)]
use crate::*;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::section::{DocGrid, DocGridType};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

/// Mac Word window measurements: 11 pt fits one 14.35 pt row, 12/16 pt require two;
/// multiple spacing scales the pitch, not the already-rounded text height. atLeast=0
/// instead uses the natural single line and must not accumulate grid rounding.
#[test]
fn japanese_grid_multiple_and_zero_minimum_match_word_measurements() {
    for family in ["MS Gothic", "MS Mincho"] {
        let r = wordcraft_fonts::word::resolve(family, false, false);
        if wordcraft_fonts::word::substitute_cjk_leading(&r.face, r.substituted) == 0.0 {
            continue;
        }
        for (size, rows) in [(11.0, 1.0f32), (12.0, 2.0), (16.0, 2.0)] {
            for multiple in [0.5, 1.0, 1.5] {
                let mut d = with_grid("日本語\n日本語", DocGridType::LinesAndChars, -6030);
                d.styles.default_chr.font = Some(family.into());
                d.styles.default_chr.size = Some(size);
                d.last_section.doc_grid.as_mut().unwrap().line_pitch = 14.35;
                for b in &mut d.body {
                    if let Block::Para(p) = std::sync::Arc::make_mut(b) {
                        p.props.line_spacing = Some(LineSpacing::Multiple(multiple));
                    }
                }
                let expected = 14.35 * rows.max(multiple);
                assert!(lines(&lay(&d)).1.iter().all(|h| (h - expected).abs() < 0.01), "{family}, {size}, {multiple}");
                for b in &mut d.body {
                    if let Block::Para(p) = std::sync::Arc::make_mut(b) {
                        p.props.line_spacing = Some(LineSpacing::AtLeast(0.0));
                    }
                }
                assert!(lines(&lay(&d)).1.iter().all(|h| (h - size * 1.3).abs() < 0.01), "zero minimum: {family}, {size}");
            }
        }
    }
}

/// Mac Word gives this punctuation sequence two em less width with kerning enabled,
/// at both 12 and 24 pt, unchanged by splitting its characters into formatting runs.
#[test]
fn japanese_punctuation_kerning_obeys_threshold_and_run_boundaries() {
    let text = "「日本（東京）」、「案内（概要）」等。";
    for family in ["BIZ UDGothic", "BIZ UDMincho"] {
        let face = wordcraft_fonts::word::resolve(family, false, false);
        if face.substituted {
            continue;
        }
        for size in [9.0, 12.0, 24.0, 36.0] {
            let width = |kern, split, content: &str| {
                let mut d = Document::from_text(content);
                d.styles.default_chr.font = Some(family.into());
                d.styles.default_chr.size = Some(size);
                d.styles.default_chr.kern = Some(kern);
                if split {
                    let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!() };
                    for (i, c) in content.char_indices() {
                        // Visually identical black / near-black prevents run coalescing.
                        p.format(i, i + c.len_utf8(), &|r| {
                            r.color = Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::props::Rgb(i as u8 % 2, 0, 0)))
                        })
                        .unwrap();
                    }
                }
                let l = lay(&d);
                let para = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None }).unwrap();
                para.clusters.iter().map(|c| c.adv).sum::<f32>()
            };
            let off = width(0.0, false, text);
            let on = width(size, false, text);
            assert!((off - on - 2.0 * size).abs() < 0.01, "{family} {size}: {off} -> {on}");
            assert!((width(size + 0.5, false, text) - off).abs() < 0.01, "below threshold stays unkerned");
            assert!((width(size, true, text) - on).abs() < 0.01, "run boundaries preserve pairs");
            for spaced in ["） 」", "）　」", "）\t」", "）\n」", "日本語", "()[]"] {
                assert!((width(size, false, spaced) - width(0.0, false, spaced)).abs() < 0.01, "no pair in {spaced:?}");
            }
        }
    }
}

/// A zero manual width clears fitting. It must preserve the same punctuation advances,
/// drawing positions and line breaks as ordinary text, including split and scaled runs.
#[test]
fn clearing_manual_width_preserves_japanese_punctuation_kerning() {
    use wordcraft_doc::props::FitText;
    let text = "「日本（東京）」、「案内（概要）」等。";
    for family in ["BIZ UDGothic", "BIZ UDMincho"] {
        if wordcraft_fonts::word::resolve(family, false, false).substituted {
            continue;
        }
        for size in [9.0, 12.0, 24.0, 36.0] {
            for scale in [50.0, 100.0, 150.0] {
                for width in [54.0, 132.0, 360.0] {
                    for split in [false, true] {
                        let make = |clear_fit, kern| {
                            let mut d = Document::from_text(text);
                            d.styles.default_chr.font = Some(family.into());
                            d.styles.default_chr.size = Some(size);
                            d.styles.default_chr.scale = Some(scale);
                            d.styles.default_chr.kern = Some(kern);
                            d.last_section.page_w = width + 40.0;
                            d.last_section.margin_left = 20.0;
                            d.last_section.margin_right = 20.0;
                            let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!() };
                            if clear_fit {
                                // An inherited fit is explicitly disabled by the run's zero.
                                d.styles.default_chr.fit_text = Some(FitText { width: 300.0, id: Some(1) });
                                p.runs[0].props.fit_text = Some(FitText { width: 0.0, id: Some(1) });
                            }
                            if split {
                                for (i, ch) in text.char_indices() {
                                    p.format(i, i + ch.len_utf8(), &|r| {
                                        r.color = Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::props::Rgb(i as u8 % 2, 0, 0)))
                                    })
                                    .unwrap();
                                }
                            }
                            let l = lay(&d);
                            l.pages[0]
                                .items
                                .iter()
                                .find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None })
                                .unwrap()
                        };
                        let natural = make(false, size);
                        let cleared = make(true, size);
                        let unkerned = make(false, 0.0);
                        let total = |p: &ParaLayout| p.clusters.iter().map(|c| c.adv).sum::<f32>();
                        assert!(total(&unkerned) - total(&natural) > size * scale / 100.0, "the fixture must exercise punctuation kerning");
                        assert_eq!(natural.clusters.len(), cleared.clusters.len());
                        for (a, b) in natural.clusters.iter().zip(&cleared.clusters) {
                            assert!(
                                (a.adv - b.adv).abs() < 0.001,
                                "{family}, {size} pt, {scale}%, width {width}, split {split}: {} != {}",
                                a.adv,
                                b.adv
                            );
                        }
                        assert_eq!(natural.glyphs.len(), cleared.glyphs.len());
                        for (a, b) in natural.glyphs.iter().zip(&cleared.glyphs) {
                            assert!((a.dx - b.dx).abs() < 0.001);
                        }
                        assert_eq!(
                            natural.lines.iter().map(|l| (l.c0, l.c1)).collect::<Vec<_>>(),
                            cleared.lines.iter().map(|l| (l.c0, l.c1)).collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }
}

/// Word's 12 pt synthetic rows keep punctuation kerning at both -1 and +1 pt spacing.
/// Each of this sequence's four pairs loses half the tracked em: 22/24/26 pt in total
/// at 12 pt with -1/0/+1 pt tracking. Other sizes/scales are synthetic consistency checks.
#[test]
fn japanese_punctuation_kerning_preserves_expanded_and_condensed_tracking() {
    let text = "「日本（東京）」、「案内（概要）」等。";
    for family in ["BIZ UDGothic", "BIZ UDMincho", "Hiragino Mincho ProN", "YuMincho"] {
        if wordcraft_fonts::word::resolve(family, false, false).substituted {
            continue;
        }
        for size in [9.0, 12.0, 24.0, 36.0] {
            for scale in [50.0, 100.0, 150.0] {
                for width in [54.0, 132.0, 360.0] {
                    for split in [false, true] {
                        let make = |spacing, kern| {
                            let mut d = Document::from_text(text);
                            d.styles.default_chr.font = Some(family.into());
                            d.styles.default_chr.size = Some(size);
                            d.styles.default_chr.scale = Some(scale);
                            d.styles.default_chr.spacing = Some(spacing);
                            d.styles.default_chr.kern = Some(kern);
                            d.last_section.page_w = width + 40.0;
                            d.last_section.margin_left = 20.0;
                            d.last_section.margin_right = 20.0;
                            if split {
                                let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!() };
                                for (i, ch) in text.char_indices() {
                                    p.format(i, i + ch.len_utf8(), &|r| {
                                        // Different colours retain independent runs.
                                        r.color = Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::props::Rgb(i as u8 % 2, 0, 0)))
                                    })
                                    .unwrap();
                                }
                            }
                            let l = lay(&d);
                            l.pages[0]
                                .items
                                .iter()
                                .find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None })
                                .unwrap()
                        };
                        let natural = make(0.0, size);
                        for spacing in [-1.0, 1.0] {
                            let tracked = make(spacing, size);
                            let unkerned = make(spacing, 0.0);
                            let total = |p: &ParaLayout| p.clusters.iter().map(|c| c.adv).sum::<f32>();
                            let expected_reduction = 2.0 * (size * scale / 100.0 + spacing);
                            assert!(
                                (total(&unkerned) - total(&tracked) - expected_reduction).abs() < 0.01,
                                "{family}, {size} pt, {scale}%, spacing {spacing}, width {width}, split {split}: punctuation bearing includes tracking"
                            );
                            assert_eq!(natural.clusters.len(), tracked.clusters.len());
                            // Four half-tracking bearings are removed from the tracked total.
                            let expected_delta = (text.chars().count() as f32 - 2.0) * spacing;
                            assert!((total(&tracked) - total(&natural) - expected_delta).abs() < 0.01);
                            assert_eq!(natural.glyphs.len(), tracked.glyphs.len());
                            for (a, b) in natural.glyphs.iter().zip(&tracked.glyphs) {
                                assert!((a.dx - b.dx).abs() < 0.001, "tracking leaves the glyph's offset inside its cluster unchanged");
                            }
                        }
                    }
                }
            }
        }
    }
}

fn with_grid(text: &str, kind: DocGridType, char_space: i32) -> Document {
    let mut d = Document::from_text(text);
    for b in d.body.iter_mut() {
        if let Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
            p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
        }
    }
    d.last_section.doc_grid = Some(DocGrid { kind, line_pitch: 18.0, char_space });
    d
}

/// (first-page lines, every line height) of a layout.
fn lines(l: &DocLayout) -> (usize, Vec<f32>) {
    let mut first = 0;
    let mut hs = Vec::new();
    for (pi, p) in l.pages.iter().enumerate() {
        for it in &p.items {
            if let Placed::Lines { para, l0, l1, .. } = it {
                if pi == 0 {
                    first += l1 - l0;
                }
                hs.extend(para.lines[*l0..*l1].iter().map(|line| line.height));
            }
        }
    }
    (first, hs)
}

#[test]
fn fallback_font_metrics_do_not_double_japanese_grid_lines() {
    let mut d = with_grid("日本語の本文\n二行目\n", DocGridType::LinesAndChars, -6030);
    d.styles.default_chr.font = Some("ＭＳ ゴシック".into());
    d.styles.default_chr.size = Some(12.0);
    d.last_section.doc_grid.as_mut().unwrap().line_pitch = 14.35;
    for b in &mut d.body {
        if let Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
        }
    }
    let (_, heights) = lines(&lay(&d));
    assert_eq!(heights.len(), 3);
    assert!(heights.iter().all(|h| (h - 28.7).abs() < 0.01), "12 pt text and an empty mark take two 14.35 pt grid rows: {heights:?}");

    // A point size larger than the pitch still occupies the necessary extra row.
    d.styles.default_chr.size = Some(20.0);
    let (_, large) = lines(&lay(&d));
    assert!(large.iter().all(|h| (h - 28.7).abs() < 0.01), "20 pt text takes two grid rows: {large:?}");
}

#[test]
fn missing_ms_mincho_uses_two_rows_for_twelve_point_japanese_marks() {
    if !wordcraft_fonts::word::resolve("MS Mincho", false, false).substituted {
        return;
    }
    let mut d = with_grid("\n", DocGridType::LinesAndChars, -6030);
    d.last_section.doc_grid.as_mut().unwrap().line_pitch = 14.35;
    d.styles.default_chr.font = Some("Century".into());
    d.styles.default_chr.font_ea = Some("MS Mincho".into());
    d.styles.default_chr.size = Some(12.0);
    assert_eq!(lines(&lay(&d)).1, vec![28.7, 28.7]);

    d.styles.default_chr.size = Some(10.5);
    assert_eq!(lines(&lay(&d)).1, vec![14.35, 14.35]);

    // Latin text takes its own Century height even when the empty paragraph mark asks for
    // MS Mincho. The East Asian font controls the mark, not every Latin-only line.
    let mut latin = with_grid("ABC", DocGridType::LinesAndChars, -6030);
    latin.last_section.doc_grid.as_mut().unwrap().line_pitch = 14.35;
    latin.styles.default_chr.font = Some("Century".into());
    latin.styles.default_chr.font_ea = Some("MS Mincho".into());
    latin.styles.default_chr.size = Some(12.0);
    assert_eq!(lines(&lay(&latin)).1, vec![14.35]);
}

#[test]
fn japanese_symbols_follow_the_requested_east_asian_face() {
    let mut d = with_grid("◎※○", DocGridType::LinesAndChars, 0);
    d.styles.default_chr.font = Some("Century".into());
    d.styles.default_chr.font_ea = Some("MS Mincho".into());
    let expected = wordcraft_fonts::word::resolve("MS Mincho", false, false).face;
    let l = lay(&d);
    let para = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None }).unwrap();
    assert_eq!(para.clusters.len(), 3);
    for cluster in &para.clusters {
        assert_eq!(para.styles[cluster.style as usize].face.id(), expected.id());
    }
}

#[test]
fn missing_ms_gothic_keeps_fixed_width_latin_when_japanese_uses_a_narrower_ink_face() {
    let requested = "ＭＳ ゴシック";
    let Some(asian) = wordcraft_fonts::word::east_asian_substitute(requested, false, false) else {
        eprintln!("skipped: requires missing MS Gothic and installed Hiragino");
        return;
    };
    let primary = wordcraft_fonts::word::resolve(requested, false, false);
    if !primary.face.named("BIZ UDGothic") {
        eprintln!("skipped: requires fixed-width BIZ UDGothic substitute");
        return;
    }
    let text = "東京都教育委員会事務局 カーニング 0123456789-ABC 9876543210";
    for explicit_asian in [false, true] {
        let mut d = with_grid(text, DocGridType::LinesAndChars, -6030);
        d.styles.default_chr.font = Some(requested.into());
        d.styles.default_chr.size = Some(12.0);
        d.styles.default_chr.spacing = Some(0.35);
        d.styles.default_chr.kern = Some(1.0);
        if explicit_asian {
            d.styles.default_chr.font_ea = Some(requested.into());
        }
        d.last_section.page_w = 260.0;
        d.last_section.margin_left = 20.0;
        d.last_section.margin_right = 20.0;
        let actual = lay(&d);
        let para = actual.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None }).unwrap();
        for cl in &para.clusters {
            let Some(ch) = text.get(cl.start..cl.end).and_then(|s| s.chars().next()).filter(|c| !c.is_whitespace()) else { continue };
            let style = &para.styles[cl.style as usize];
            assert_eq!(style.face.id(), if ch.is_ascii() { primary.face.id() } else { asian.face.id() }, "{ch}");
        }

        // Whole-face substitution would change the width of ASCII letters/digits, including
        // phone-like sequences. The script split retains both those widths and the CJK grid.
        d.styles.default_chr.font = Some(primary.face.family.clone());
        d.styles.default_chr.font_ea = None;
        let baseline = lay(&d);
        let old = baseline.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None }).unwrap();
        assert_eq!(para.clusters.len(), old.clusters.len());
        for (new, old) in para.clusters.iter().zip(&old.clusters) {
            assert!((new.adv - old.adv).abs() < 0.01, "cluster {} advance {} != {}", new.start, new.adv, old.adv);
        }
        assert_eq!(para.lines.len(), old.lines.len(), "the narrower ink must not change line wrapping");
        assert_eq!(para.lines.iter().map(|l| (l.c0, l.c1)).collect::<Vec<_>>(), old.lines.iter().map(|l| (l.c0, l.c1)).collect::<Vec<_>>());
        for (new, old) in para.lines.iter().zip(&old.lines) {
            assert!(new.baseline >= old.baseline, "missing CJK leading preserves room above the ink");
            assert!(new.height >= old.height, "missing CJK leading may require an extra grid row");
        }
        assert_eq!(actual.pages.len(), baseline.pages.len());
    }
}

#[test]
fn positioned_text_reserves_its_raise_or_drop_on_the_line_grid() {
    let mut d = with_grid("Raised", DocGridType::Lines, 0);
    d.last_section.doc_grid.as_mut().unwrap().line_pitch = 14.35;
    d.styles.default_chr.size = Some(12.0);
    if let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) {
        p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
        p.runs[0].props.position = Some(6.0);
    }
    assert_eq!(lines(&lay(&d)).1, vec![28.7]);
    if let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) {
        p.runs[0].props.position = Some(-6.0);
    }
    assert_eq!(lines(&lay(&d)).1, vec![28.7]);
}

#[test]
fn ruby_reserves_base_descent_when_it_nearly_fills_a_grid_row() {
    use wordcraft_doc::para::RubyAlign;
    let mut d = with_grid("base", DocGridType::Lines, 0);
    d.last_section.doc_grid.as_mut().unwrap().line_pitch = 14.35;
    d.styles.default_chr.size = Some(12.0);
    if let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) {
        p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
    }
    let ruby = wordcraft_doc::InlineObject::Ruby {
        base: "base".into(),
        ruby: "reading".into(),
        align: RubyAlign::Center,
        size: 5.0,
        raise: 8.0,
        base_size: 12.0,
        lang: String::new(),
        props: Default::default(),
    };
    d.insert_object(&wordcraft_doc::Pos::body(0, 0), ruby, &Default::default()).unwrap();
    let l = lay(&d);
    let para = l.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { para, .. } => Some(para),
            _ => None,
        })
        .unwrap();
    let cluster = &para.clusters[0];
    assert!(cluster.obj_h + cluster.obj_d < 14.35, "ruby ascent alone should fit one row");
    assert!(cluster.obj_h + para.styles[cluster.style as usize].descent > 14.35, "base descent requires a second row");
    assert_eq!(lines(&l).1, vec![28.7]);
}

#[test]
fn an_inline_drawing_below_the_font_ascent_still_reserves_grid_rows() {
    let mut d = with_grid("A", DocGridType::Default, 0);
    d.styles.default_chr.font = Some("Source Sans 3".into());
    d.styles.default_chr.font_ea = Some("Source Sans 3".into());
    d.styles.default_chr.size = Some(12.0);
    let initial = lay(&d);
    let para = initial.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { para, .. } => Some(para),
            _ => None,
        })
        .unwrap();
    let style = &para.styles[para.clusters[0].style as usize];
    let height = style.ascent - 0.1;
    let pitch = (12.0 + height + style.descent) / 2.0;
    assert!(height > 0.0 && pitch > 12.0 && height + style.descent > pitch);
    d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: pitch, char_space: 0 });
    let drawing = wordcraft_doc::InlineObject::Graphic {
        w: 12.0,
        h: height,
        alt: String::new(),
        float: Default::default(),
        graphic: std::sync::Arc::new(Default::default()),
    };
    d.insert_object(&wordcraft_doc::Pos::body(0, 1), drawing, &Default::default()).unwrap();
    assert!((lines(&lay(&d)).1[0] - pitch * 2.0).abs() < 0.01, "the drawing plus descent needs two grid rows");
}

#[test]
fn line_grid_snaps_line_heights_and_fixes_lines_per_page() {
    let text = vec!["Grid line"; 80].join("\n");
    let d = with_grid(&text, DocGridType::Lines, 0);
    let (first, hs) = lines(&lay(&d));
    for h in &hs {
        let cells = h / 18.0;
        assert!((cells - cells.round()).abs() < 1e-3 && cells >= 1.0, "line height {h} is not a multiple of the pitch");
    }
    // Text at the default size fits one pitch: 648 pt of text height holds 36 lines.
    let h = hs[0];
    assert_eq!(first, (648.0 / h + 1e-3).floor() as usize, "heights {h}");
    // Turning the paragraphs' snapToGrid off leaves their natural line heights.
    let mut off = d.clone();
    for b in off.body.iter_mut() {
        if let Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.snap_to_grid = Some(false);
        }
    }
    let plain = with_grid(&text, DocGridType::Default, 0);
    assert_eq!(lines(&lay(&off)).1[0], lines(&lay(&plain)).1[0]);
}

#[test]
fn character_grid_places_east_asian_text_on_the_pitch() {
    let text = "日本語の文書abc";
    // The default font size plus 2.5 pt: a pitch unlike any font's advance.
    let lines = wordcraft_doc::SectionProps { doc_grid: Some(DocGrid { kind: DocGridType::Lines, ..Default::default() }), ..Default::default() };
    let size = para::Grid::of(&Document::from_text(""), &lines).font_size;
    let cs = DocGrid::char_space_for(size + 2.5, size);
    let d = with_grid(text, DocGridType::LinesAndChars, cs);
    let pitch = d.last_section.grid_char_pitch(size).unwrap();
    assert!((pitch - (size + 2.5)).abs() < 0.01);
    let l = lay(&d);
    let plain = lay(&with_grid(text, DocGridType::Lines, cs));
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    let (g, p) = (first(&l), first(&plain));
    let line = &g.lines[0];
    let mut expected_x = 0.0;
    for (k, c) in g.clusters.iter().enumerate() {
        let x = line.xs[k] - line.xs[0];
        assert!((x - expected_x).abs() < 0.01, "cluster {k} at {x}, expected {expected_x}");
        // Every shaped advance gets the grid delta, including Latin text. The automatic
        // CJK/Latin boundary gap survives a `linesAndChars` grid in both layouts.
        assert!((c.adv - p.clusters[k].adv - 2.5).abs() < 0.01, "cluster {k}: grid={}, plain={}", c.adv, p.clusters[k].adv);
        expected_x += c.adv;
    }
}

#[test]
fn lines_and_chars_grid_delta_covers_latin_spaces_and_cjk() {
    let text = "A        B　国1";
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    for family in ["BIZ UDMincho", "Arial", "Hiragino Mincho ProN"] {
        for size in [9.0, 12.0, 18.0] {
            for delta in [-1.5, -1.0, 2.0] {
                for balance in [false, true] {
                    let mut doc = with_grid(text, DocGridType::LinesAndChars, (delta * 4096.0) as i32);
                    doc.styles.default_chr.font = Some(family.into());
                    doc.styles.default_chr.font_ea = Some(family.into());
                    doc.styles.default_chr.size = Some(size);
                    doc.settings.balance_single_byte_double_byte_width = balance;
                    let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
                    p.props.auto_space_de = Some(false);
                    p.props.auto_space_dn = Some(false);
                    let gridded = first(&lay(&doc));
                    doc.last_section.doc_grid.as_mut().unwrap().kind = DocGridType::Lines;
                    let plain = first(&lay(&doc));
                    assert_eq!(gridded.clusters.len(), plain.clusters.len());
                    for (g, p) in gridded.clusters.iter().zip(&plain.clusters) {
                        let ch = text[g.start..g.end].chars().next().unwrap();
                        let factor = if balance && !para::is_east_asian(ch) { 0.5 } else { 1.0 };
                        assert!(
                            (g.adv - p.adv - delta * factor).abs() < 0.01,
                            "{family}, {size}pt, delta {delta}, balance {balance}, {ch:?}: {} vs {}",
                            g.adv,
                            p.adv
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn lines_and_chars_keeps_automatic_asian_boundary_spacing() {
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    for kind in [DocGridType::LinesAndChars, DocGridType::SnapToChars] {
        for (de, dn, boundary) in [(true, false, 0), (false, true, 1)] {
            let mut doc = with_grid("A国1", kind, -4096);
            doc.styles.default_chr.size = Some(12.0);
            let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
            p.props.auto_space_de = Some(false);
            p.props.auto_space_dn = Some(false);
            let without = first(&lay(&doc));
            let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
            p.props.auto_space_de = Some(de);
            p.props.auto_space_dn = Some(dn);
            let with = first(&lay(&doc));
            for k in 0..3 {
                let expected = if kind == DocGridType::LinesAndChars && k == boundary { 3.0 } else { 0.0 };
                assert!((with.clusters[k].adv - without.clusters[k].adv - expected).abs() < 0.01, "{kind:?}, DE {de}, DN {dn}, cluster {k}");
            }
        }
    }
}

/// Expanded run tracking and the document's character delta both apply. A prior
/// paragraph-wide exception hid the grid delta instead of honoring width balancing.
#[test]
fn positive_run_tracking_keeps_the_character_grid_delta() {
    let mut d = with_grid("電気工事士", DocGridType::LinesAndChars, -6030);
    d.styles.default_chr.size = Some(10.5);
    if let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) {
        for run in &mut p.runs {
            run.props.size = Some(12.0);
            run.props.spacing = Some(1.0);
        }
    }
    let gridded = lay(&d);
    d.last_section.doc_grid.as_mut().unwrap().kind = DocGridType::Lines;
    let plain = lay(&d);
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    let (g, p) = (first(&gridded), first(&plain));
    assert_eq!(g.clusters.len(), p.clusters.len());
    for (gc, pc) in g.clusters.iter().zip(&p.clusters) {
        let delta = -6030.0 / 4096.0;
        assert!((gc.adv - pc.adv - delta).abs() < 0.001, "tracked advance and grid delta both apply");
        for (gg, pg) in g.glyphs[gc.g0 as usize..gc.g1 as usize].iter().zip(&p.glyphs[pc.g0 as usize..pc.g1 as usize]) {
            assert!((gg.dx - pg.dx).abs() < 0.001, "additive grid spacing leaves each glyph at its original offset");
        }
    }
}

#[test]
fn width_balancing_doubles_only_fullwidth_tracking_before_applying_the_grid() {
    let text = "国Ａカ１A1　ｶﾞキ\u{3099}";
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    for family in ["Hiragino Mincho ProN", "Yu Mincho", "Noto Sans JP"] {
        for size in [9.0, 10.5, 12.0, 24.0] {
            for spacing in [-1.0, 0.0, 0.8, 1.1, 1.5] {
                for delta in [-1.5, 0.0, 2.0] {
                    let mut doc = with_grid(text, DocGridType::LinesAndChars, (delta * 4096.0) as i32);
                    doc.styles.default_chr.font = Some(family.into());
                    doc.styles.default_chr.font_ea = Some(family.into());
                    doc.styles.default_chr.size = Some(size);
                    doc.styles.default_chr.spacing = Some(spacing);
                    let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
                    p.props.snap_to_grid = Some(false);
                    p.props.auto_space_de = Some(false);
                    p.props.auto_space_dn = Some(false);
                    let unbalanced = first(&lay(&doc));
                    doc.settings.balance_single_byte_double_byte_width = true;
                    let balanced = first(&lay(&doc));
                    assert_eq!(balanced.clusters.len(), unbalanced.clusters.len());
                    for (a, b) in balanced.clusters.iter().zip(&unbalanced.clusters) {
                        let ch = text[a.start..a.end].chars().next().unwrap();
                        let grid_delta = if para::is_east_asian(ch) { 0.0 } else { -delta / 2.0 };
                        let extra = if ch.is_ascii() || ('\u{ff61}'..='\u{ffdc}').contains(&ch) { 0.0 } else { spacing };
                        assert!((a.adv - b.adv - extra - grid_delta).abs() < 0.01, "{family}, {size}, tracking {spacing}, delta {delta}, {ch}");
                    }
                    let combining = balanced.clusters.iter().find(|c| text.get(c.start..c.end) == Some("キ\u{3099}")).unwrap();
                    let natural = unbalanced.clusters.iter().find(|c| c.start == combining.start).unwrap();
                    assert!((combining.adv - natural.adv - spacing).abs() < 0.01, "a combining mark adds no second tracking unit");
                }
            }
        }
    }
}

#[test]
fn width_balancing_leaves_narrow_ambiguous_symbols_at_their_original_advances() {
    let mut doc = with_grid("※○◎", DocGridType::Lines, 0);
    doc.styles.default_chr.font = Some("Source Sans 3".into());
    doc.styles.default_chr.font_ea = Some("Source Sans 3".into());
    doc.styles.default_chr.size = Some(12.0);
    doc.styles.default_chr.spacing = Some(1.0);
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    let off = first(&lay(&doc));
    doc.settings.balance_single_byte_double_byte_width = true;
    let on = first(&lay(&doc));
    let mut narrow = 0;
    for (a, b) in on.clusters.iter().zip(&off.clusters) {
        let style = &off.styles[b.style as usize];
        let em = style.size * style.rc.scale / 100.0;
        if (b.adv - style.rc.spacing - em).abs() >= em * 0.01 {
            narrow += 1;
            assert!((a.adv - b.adv).abs() < 0.01, "ambiguous characters drawn narrower than an em do not get fullwidth tracking");
        }
    }
    assert!(narrow > 0, "the Latin font fixture must include an actually narrow symbol");
}

#[test]
fn width_balancing_setting_invalidates_cached_paragraph_advances() {
    let mut doc = with_grid("国語", DocGridType::LinesAndChars, -6030);
    doc.styles.default_chr.spacing = Some(1.0);
    let mut cache = LayoutCache::new();
    let plain = layout(&doc, &mut cache, &LayoutOptions::default());
    doc.settings.balance_single_byte_double_byte_width = true;
    let balanced = layout(&doc, &mut cache, &LayoutOptions::default());
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    let (plain, balanced) = (first(&plain), first(&balanced));
    for (p, b) in plain.clusters.iter().zip(&balanced.clusters) {
        assert!((b.adv - p.adv - 1.0).abs() < 0.01, "the changed compatibility setting must force reshaping");
    }
}

#[test]
fn character_grid_uses_run_snapping_independently_of_paragraph_line_snapping() {
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    for family in ["Hiragino Mincho ProN", "Yu Mincho", "Noto Sans JP"] {
        for size in [9.0, 10.5, 12.0, 24.0] {
            for delta in [-1.5, 2.0] {
                let mut doc = with_grid("国語ABC", DocGridType::LinesAndChars, (delta * 4096.0) as i32);
                doc.styles.default_chr.font = Some(family.into());
                doc.styles.default_chr.font_ea = Some(family.into());
                doc.styles.default_chr.size = Some(size);
                doc.styles.default_chr.snap_to_grid = Some(true);
                let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
                p.props.auto_space_de = Some(false);
                p.props.auto_space_dn = Some(false);
                let grid = first(&lay(&doc));
                doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.snap_to_grid = Some(false);
                let paragraph_off = first(&lay(&doc));
                assert_eq!(grid.clusters.iter().map(|c| c.adv).collect::<Vec<_>>(), paragraph_off.clusters.iter().map(|c| c.adv).collect::<Vec<_>>());
                doc.styles.default_chr.snap_to_grid = Some(false);
                let run_off = first(&lay(&doc));
                for (g, off) in grid.clusters.iter().zip(&run_off.clusters) {
                    let difference = delta;
                    assert!((g.adv - off.adv - difference).abs() < 0.01, "{family}, {size}, delta {delta}, cluster {}", g.start);
                }
                let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
                p.format(0, "国".len(), &|c| c.snap_to_grid = Some(true)).unwrap();
                let mixed = first(&lay(&doc));
                assert!((mixed.clusters[0].adv - grid.clusters[0].adv).abs() < 0.01, "direct formatting enables only its run");
                assert!((mixed.clusters[1].adv - run_off.clusters[1].adv).abs() < 0.01, "the next run retains inherited opt-out");
            }
        }
    }
}

#[test]
fn manual_run_width_groups_contiguous_runs_and_overrides_grid_and_spacing() {
    use wordcraft_doc::para::Run;
    use wordcraft_doc::props::FitText;
    let mut d = with_grid("第二種電気工事士免状交付申請についてx免状交付", DocGridType::LinesAndChars, -6030);
    let fit = |width| CharProps { size: Some(16.0), spacing: Some(20.0), fit_text: Some(FitText { width, id: Some(-1234) }), ..Default::default() };
    if let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) {
        let mut bold = fit(321.0);
        bold.bold = Some(true);
        p.runs = vec![
            Run { len: "第二種電気工事士".len(), props: fit(321.0) },
            Run { len: "免状交付申請について".len(), props: bold },
            Run { len: 1, props: CharProps::default() },
            Run { len: "免状交付".len(), props: fit(42.0) },
        ];
    }
    let l = lay(&d);
    let p = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None }).unwrap();
    let width = |start, end| p.clusters.iter().filter(|c| c.start >= start && c.end <= end).map(|c| c.adv).sum::<f32>();
    assert!((width(0, 54) - 321.0).abs() < 0.01, "two styled runs share one 321 pt width");
    assert!((width(55, 67) - 42.0).abs() < 0.01, "the same ID after unformatted text starts another group");
    let plain = p.clusters.iter().find(|c| c.start == 54).unwrap();
    assert_eq!(p.styles[plain.style as usize].rc.scale, 100.0, "fitting does not mutate a shared style outside the group");
    for c in p.clusters.iter().filter(|c| c.start != 54) {
        let st = &p.styles[c.style as usize];
        assert!(st.rc.scale.is_finite() && st.rc.scale > 0.0);
    }
}

#[test]
fn manual_width_expands_between_characters_but_condenses_glyphs() {
    use wordcraft_doc::props::FitText;
    let layout = |fit: Option<f32>| {
        let mut d = with_grid("申請者", DocGridType::Lines, 0);
        if let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) {
            p.runs[0].props.size = Some(12.0);
            p.runs[0].props.fit_text = fit.map(|width| FitText { width, id: Some(1) });
        }
        let l = lay(&d);
        let p = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None }).unwrap();
        p.clusters.iter().map(|c| (c.adv, p.styles[c.style as usize].rc.scale)).collect::<Vec<_>>()
    };
    let natural = layout(None);
    let width: f32 = natural.iter().map(|c| c.0).sum();
    let expanded = layout(Some(width + 12.0));
    assert_eq!(expanded.len(), 3);
    for i in 0..3 {
        let extra = if i < 2 { 6.0 } else { 0.0 };
        assert!((expanded[i].0 - natural[i].0 - extra).abs() < 0.01);
        assert_eq!(expanded[i].1, natural[i].1);
    }
    let condensed = layout(Some(width * 0.75));
    assert!((condensed.iter().map(|c| c.0).sum::<f32>() - width * 0.75).abs() < 0.01);
    assert!(condensed.iter().all(|c| (c.1 - 75.0).abs() < 0.01));
}

#[test]
fn manual_width_outside_the_horizontal_scale_budget_keeps_natural_text() {
    use wordcraft_doc::props::FitText;
    for (size, width) in [(1.0, 1584.0), (100.0, 0.1)] {
        let mut d = with_grid(".", DocGridType::Lines, 0);
        if let Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) {
            p.runs[0].props.size = Some(size);
            p.runs[0].props.fit_text = Some(FitText { width, id: Some(1) });
        }
        let l = lay(&d);
        let p = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para) } else { None }).unwrap();
        assert!(p.clusters.iter().all(|c| c.adv.is_finite() && c.adv >= 0.0));
        assert!(p.styles.iter().all(|s| s.rc.scale == 100.0), "an extreme fit is skipped rather than sent to the renderer");
    }
}
