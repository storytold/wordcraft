//! Document grid (`w:docGrid`): line pitch and character pitch (#391). Assertions are relations
//! to the pitch, never font metrics (fonts differ between machines; CJK glyphs may be missing).

#[allow(unused_imports)]
use crate::*;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::section::{DocGrid, DocGridType};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
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
            p.props.line_spacing = Some(LineSpacing::AtLeast(0.0));
        }
    }
    let (_, heights) = lines(&lay(&d));
    assert_eq!(heights.len(), 3);
    assert!(heights.iter().all(|h| (h - 14.35).abs() < 0.01), "12 pt text and an empty mark take one 14.35 pt grid row: {heights:?}");

    // A point size larger than the pitch still occupies the necessary extra row.
    d.styles.default_chr.size = Some(20.0);
    let (_, large) = lines(&lay(&d));
    assert!(large.iter().all(|h| (h - 28.7).abs() < 0.01), "20 pt text takes two grid rows: {large:?}");
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
    let latin = text.find('a').unwrap();
    let mut expected_x = 0.0;
    for (k, c) in g.clusters.iter().enumerate() {
        let x = line.xs[k] - line.xs[0];
        assert!((x - expected_x).abs() < 0.01, "cluster {k} at {x}, expected {expected_x}");
        if c.start < latin {
            // Each shaped advance gets the grid delta. In the plain layout, the last
            // CJK glyph also gets automatic CJK/Latin spacing; the character grid
            // suppresses that extra quarter em at the boundary.
            let auto_space = if c.end == latin { size * 0.25 } else { 0.0 };
            assert!((c.adv - (p.clusters[k].adv + 2.5 - auto_space)).abs() < 0.01, "cluster {k}: grid={}, plain={}", c.adv, p.clusters[k].adv);
        } else {
            // Latin text keeps its own advances.
            assert!((c.adv - p.clusters[k].adv).abs() < 0.01, "latin cluster {k}");
        }
        expected_x += c.adv;
    }
}

/// A smaller character grid must not squeeze every run into the Normal-size cell. The
/// 12 pt body of a Japanese document with 10.5 pt Normal and charSpace=-6030 used to
/// advance only 9.03 pt per character, with full-size glyphs overlapping their neighbours.
#[test]
fn lines_and_chars_adds_spacing_to_each_runs_own_width() {
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
    let delta = -6030.0 / 4096.0;
    for (gc, pc) in g.clusters.iter().zip(&p.clusters) {
        assert!((gc.adv - (pc.adv + delta)).abs() < 0.001, "grid adds {delta} to this run's {} pt advance, got {}", pc.adv, gc.adv);
        for (gg, pg) in g.glyphs[gc.g0 as usize..gc.g1 as usize].iter().zip(&p.glyphs[pc.g0 as usize..pc.g1 as usize]) {
            assert!((gg.dx - pg.dx - delta / 2.0).abs() < 0.001, "glyph remains centred in the adjusted advance");
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
        assert!((st.rc.scale - 100.0).abs() > 0.1, "the drawn glyph width follows the fitted advance");
    }
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
