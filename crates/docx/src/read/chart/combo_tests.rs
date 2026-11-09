//! Combo charts (several chart-type groups in one plot area, a secondary value axis) and stock
//! charts (hi-low lines, up/down bars): ECMA-376 Part 1 §21.2.2.145, §21.2.2.198, §21.2.2.218.

use super::*;
use wordcraft_doc::THEME_COLORS;

const NS: &str =
    r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

const W: f32 = 400.0;
const H: f32 = 300.0;

fn draw(plot: &str, legend: bool) -> Vec<GraphicItem> {
    let legend = if legend { r#"<c:legend><c:legendPos val="b"/></c:legend>"# } else { "" };
    let x = format!(r#"<c:chartSpace {NS}><c:chart><c:plotArea>{plot}</c:plotArea>{legend}</c:chart></c:chartSpace>"#);
    let space = crate::xml::parse(x.as_bytes()).unwrap();
    chart_items(&space, &THEME_COLORS, W, H)
}

/// A series with string categories, values and an optional `c:spPr` body.
fn ser(idx: usize, name: &str, cats: &[&str], vals: &[f64], sppr: &str) -> String {
    let cat_pts: String = cats.iter().enumerate().map(|(k, c)| format!(r#"<c:pt idx="{k}"><c:v>{c}</c:v></c:pt>"#)).collect();
    let val_pts: String = vals.iter().enumerate().map(|(k, v)| format!(r#"<c:pt idx="{k}"><c:v>{v}</c:v></c:pt>"#)).collect();
    let sppr = if sppr.is_empty() { String::new() } else { format!("<c:spPr>{sppr}</c:spPr>") };
    format!(
        r#"<c:ser><c:idx val="{idx}"/><c:order val="{idx}"/><c:tx><c:strRef><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{name}</c:v></c:pt></c:strCache></c:strRef></c:tx>{sppr}<c:cat><c:strRef><c:strCache><c:ptCount val="{nc}"/>{cat_pts}</c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="{nv}"/>{val_pts}</c:numCache></c:numRef></c:val></c:ser>"#,
        nc = cats.len(),
        nv = vals.len()
    )
}

fn solid(hex: &str) -> String {
    format!(r#"<a:solidFill><a:srgbClr val="{hex}"/></a:solidFill>"#)
}

fn no_line() -> &'static str {
    "<a:ln><a:noFill/></a:ln>"
}

fn bar_group(series: &str, ax: (u32, u32)) -> String {
    format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{series}<c:gapWidth val="150"/><c:axId val="{}"/><c:axId val="{}"/></c:barChart>"#,
        ax.0, ax.1
    )
}

fn line_group(series: &str, ax: (u32, u32)) -> String {
    format!(r#"<c:lineChart><c:grouping val="standard"/>{series}<c:marker val="1"/><c:axId val="{}"/><c:axId val="{}"/></c:lineChart>"#, ax.0, ax.1)
}

fn area_group(series: &str, ax: (u32, u32)) -> String {
    format!(r#"<c:areaChart><c:grouping val="standard"/>{series}<c:axId val="{}"/><c:axId val="{}"/></c:areaChart>"#, ax.0, ax.1)
}

fn cat_ax(id: u32, cross: u32) -> String {
    format!(
        r#"<c:catAx><c:axId val="{id}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="{cross}"/><c:crossBetween val="between"/></c:catAx>"#
    )
}

/// A value axis; `right` makes it a secondary axis on the right that crosses at the maximum.
fn val_ax(id: u32, cross: u32, right: bool, max: Option<f64>) -> String {
    let scaling = match max {
        Some(m) => format!(r#"<c:scaling><c:orientation val="minMax"/><c:max val="{m}"/><c:min val="0"/></c:scaling>"#),
        None => r#"<c:scaling><c:orientation val="minMax"/></c:scaling>"#.to_string(),
    };
    let (pos, crosses) = if right { ("r", "max") } else { ("l", "autoZero") };
    format!(
        r#"<c:valAx><c:axId val="{id}"/>{scaling}<c:delete val="0"/><c:axPos val="{pos}"/><c:majorGridlines/><c:numFmt formatCode="General" sourceLinked="1"/><c:crossAx val="{cross}"/><c:crosses val="{crosses}"/><c:crossBetween val="between"/></c:valAx>"#
    )
}

/// Filled paths: (colour, min x, min y, max x, max y, index in the item list).
fn fills(items: &[GraphicItem]) -> Vec<(Rgb, f32, f32, f32, f32, usize)> {
    items
        .iter()
        .enumerate()
        .filter_map(|(k, it)| match it {
            GraphicItem::Path { segs, fill: Some(c), .. } => {
                let p = points(segs);
                let xs = p.iter().map(|q| q.0);
                let ys = p.iter().map(|q| q.1);
                Some((
                    *c,
                    xs.clone().fold(f32::INFINITY, f32::min),
                    ys.clone().fold(f32::INFINITY, f32::min),
                    xs.fold(f32::NEG_INFINITY, f32::max),
                    ys.fold(f32::NEG_INFINITY, f32::max),
                    k,
                ))
            }
            _ => None,
        })
        .collect()
}

/// Stroked, unfilled paths: (colour, their points, index in the item list).
fn strokes(items: &[GraphicItem]) -> Vec<(Rgb, Vec<(f32, f32)>, usize)> {
    items
        .iter()
        .enumerate()
        .filter_map(|(k, it)| match it {
            GraphicItem::Path { segs, fill: None, stroke: Some(c), .. } => Some((*c, points(segs), k)),
            _ => None,
        })
        .collect()
}

fn points(segs: &[PathSeg]) -> Vec<(f32, f32)> {
    segs.iter()
        .filter_map(|s| match s {
            PathSeg::Move(x, y) | PathSeg::Line(x, y) => Some((*x, *y)),
            _ => None,
        })
        .collect()
}

fn texts(items: &[GraphicItem]) -> Vec<(String, f32)> {
    items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Text { text, rect, .. } => Some((text.clone(), rect[0])),
            _ => None,
        })
        .collect()
}

const CATS: [&str; 3] = ["Q1", "Q2", "Q3"];
const BAR: &str = "4472C4";
const LINE: &str = "ED7D31";

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

#[test]
fn combo_draws_the_line_group_on_the_bar_groups_scale() {
    let plot = format!(
        "{}{}{}{}",
        bar_group(&ser(0, "Sales", &CATS, &[10.0, 20.0, 30.0], &solid(BAR)), (1, 2)),
        line_group(&ser(1, "Average", &CATS, &[15.0, 25.0, 5.0], &format!("<a:ln w=\"28575\">{}</a:ln>", solid(LINE))), (1, 2)),
        cat_ax(1, 2),
        val_ax(2, 1, false, None)
    );
    let items = draw(&plot, false);
    let bars: Vec<_> = fills(&items).into_iter().filter(|f| f.0 == Rgb(0x44, 0x72, 0xC4)).collect();
    assert_eq!(bars.len(), 3, "three columns");
    let line: Vec<_> = strokes(&items).into_iter().filter(|s| s.0 == Rgb(0xED, 0x7D, 0x31)).collect();
    assert_eq!(line.len(), 1, "the line series of the combo is drawn: {:?}", strokes(&items));
    let pts = &line[0].1;
    assert_eq!(pts.len(), 3);
    // Same category places: each line point at its column's centre.
    for (i, b) in bars.iter().enumerate() {
        assert!(near(pts[i].0, (b.1 + b.3) / 2.0), "point {i} x {} vs column centre {}", pts[i].0, (b.1 + b.3) / 2.0);
    }
    // Same value scale: the baseline is the columns' bottom, the 20 column's top gives the slope.
    let base = bars[1].4;
    let per = (base - bars[1].2) / 20.0;
    for (i, v) in [15.0f32, 25.0, 5.0].iter().enumerate() {
        assert!(near(pts[i].1, base - per * v), "point {i} y {} vs {}", pts[i].1, base - per * v);
    }
}

#[test]
fn combo_layers_areas_then_bars_then_lines_whatever_the_xml_order() {
    let plot = format!(
        "{}{}{}{}{}",
        line_group(&ser(2, "Line", &CATS, &[5.0, 6.0, 7.0], &format!("<a:ln>{}</a:ln>", solid(LINE))), (1, 2)),
        bar_group(&ser(1, "Bars", &CATS, &[3.0, 4.0, 5.0], &solid(BAR)), (1, 2)),
        area_group(&ser(0, "Area", &CATS, &[1.0, 2.0, 3.0], &solid("A5A5A5")), (1, 2)),
        cat_ax(1, 2),
        val_ax(2, 1, false, None)
    );
    let items = draw(&plot, false);
    let f = fills(&items);
    let area = f.iter().filter(|x| x.0 == Rgb(0xA5, 0xA5, 0xA5)).map(|x| x.5).max().expect("area drawn");
    let bars: Vec<usize> = f.iter().filter(|x| x.0 == Rgb(0x44, 0x72, 0xC4)).map(|x| x.5).collect();
    assert_eq!(bars.len(), 3, "bars drawn");
    let line = strokes(&items).into_iter().find(|s| s.0 == Rgb(0xED, 0x7D, 0x31)).expect("line drawn").2;
    assert!(area < *bars.iter().min().unwrap(), "area behind the bars");
    assert!(line > *bars.iter().max().unwrap(), "line in front of the bars");
}

#[test]
fn combo_legend_lists_every_group_and_colours_series_by_their_index() {
    let plot = format!(
        "{}{}{}{}",
        bar_group(&ser(0, "Sales", &CATS, &[10.0, 20.0, 30.0], ""), (1, 2)),
        line_group(&ser(1, "Average", &CATS, &[15.0, 25.0, 5.0], ""), (1, 2)),
        cat_ax(1, 2),
        val_ax(2, 1, false, None)
    );
    let items = draw(&plot, true);
    let t: Vec<String> = texts(&items).into_iter().map(|t| t.0).collect();
    assert!(t.iter().any(|s| s == "Sales") && t.iter().any(|s| s == "Average"), "legend {t:?}");
    // The default colours: what a plain two-series column chart gives its first and second series.
    let reference = draw(
        &format!(
            "{}{}{}",
            bar_group(&format!("{}{}", ser(0, "A", &CATS, &[1.0, 2.0, 3.0], ""), ser(1, "B", &CATS, &[1.0, 2.0, 3.0], "")), (1, 2)),
            cat_ax(1, 2),
            val_ax(2, 1, false, None)
        ),
        false,
    );
    let rf = fills(&reference);
    let first = rf.iter().find(|f| f.1 > 0.0 && f.3 - f.1 < W / 2.0).expect("a column").0;
    let second = rf.iter().map(|f| f.0).find(|c| *c != first).expect("a second series colour");
    let bar = fills(&items).into_iter().find(|f| f.0 == first).map(|f| f.0);
    assert_eq!(bar, Some(first), "the bar series (idx 0) keeps accent 1");
    assert!(strokes(&items).iter().any(|s| s.0 == second && s.1.len() == 3), "the line series (idx 1) gets accent 2");
}

#[test]
fn combo_line_on_a_secondary_axis_gets_its_own_scale_and_right_hand_labels() {
    let plot = format!(
        "{}{}{}{}{}{}",
        bar_group(&ser(0, "Units", &CATS, &[100.0, 200.0, 300.0], &solid(BAR)), (1, 2)),
        line_group(&ser(1, "Margin", &CATS, &[0.1, 0.2, 0.3], &format!("<a:ln>{}</a:ln>", solid(LINE))), (3, 4)),
        cat_ax(1, 2),
        val_ax(2, 1, false, Some(400.0)),
        // The secondary category axis Word writes alongside (deleted).
        r#"<c:catAx><c:axId val="3"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="1"/><c:axPos val="b"/><c:crossAx val="4"/></c:catAx>"#,
        val_ax(4, 3, true, Some(0.4))
    );
    let items = draw(&plot, false);
    let bars: Vec<_> = fills(&items).into_iter().filter(|f| f.0 == Rgb(0x44, 0x72, 0xC4)).collect();
    assert_eq!(bars.len(), 3);
    let line = strokes(&items).into_iter().find(|s| s.0 == Rgb(0xED, 0x7D, 0x31)).expect("line drawn").1;
    // 0.1 / 0.4 of the secondary axis is as high as 100 / 400 of the primary one.
    for (i, (p, b)) in line.iter().zip(&bars).enumerate() {
        assert!(near(p.1, b.2), "point {i} y {} vs column top {}", p.1, b.2);
    }
    let right_edge = bars.iter().map(|b| b.3).fold(0.0f32, f32::max);
    let t = texts(&items);
    assert!(t.iter().any(|(s, x)| s == "0.4" && *x > right_edge), "secondary labels right of the plot: {t:?}");
    assert!(t.iter().any(|(s, x)| s == "400" && *x < bars[0].1), "primary labels stay on the left: {t:?}");
}

fn stock(series: &str, extra: &str) -> String {
    format!("<c:stockChart>{series}{extra}<c:axId val=\"1\"/><c:axId val=\"2\"/></c:stockChart>{}{}", cat_ax(1, 2), val_ax(2, 1, false, Some(40.0)))
}

#[test]
fn stock_chart_draws_hi_low_lines_not_a_placeholder() {
    let series = format!(
        "{}{}{}",
        ser(0, "High", &CATS, &[30.0, 35.0, 25.0], no_line()),
        ser(1, "Low", &CATS, &[10.0, 20.0, 5.0], no_line()),
        ser(2, "Close", &CATS, &[20.0, 22.0, 15.0], no_line())
    );
    let items = draw(&stock(&series, &format!("<c:hiLowLines><c:spPr><a:ln w=\"9525\">{}</a:ln></c:spPr></c:hiLowLines>", solid("000000"))), false);
    assert!(!texts(&items).iter().any(|t| t.0.contains("not supported")), "no placeholder");
    let hl: Vec<_> = strokes(&items).into_iter().filter(|s| s.0 == Rgb::BLACK).collect();
    assert_eq!(hl.len(), 3, "one hi-low line per category: {hl:?}");
    // Vertical, ordered left to right, lengths in proportion to high - low (20, 15, 20).
    let len: Vec<f32> = hl
        .iter()
        .map(|s| {
            assert_eq!(s.1.len(), 2);
            assert!(near(s.1[0].0, s.1[1].0), "vertical: {:?}", s.1);
            (s.1[0].1 - s.1[1].1).abs()
        })
        .collect();
    assert!(hl[0].1[0].0 < hl[1].1[0].0 && hl[1].1[0].0 < hl[2].1[0].0);
    let per = len[0] / 20.0;
    assert!(near(len[1], per * 15.0) && near(len[2], per * 20.0), "{len:?}");
    // The lowest end of the 3rd line is 5, the top of the 2nd is 35: 30 apart on the shared scale.
    let low3 = hl[2].1.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
    let high2 = hl[1].1.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    assert!(near(low3 - high2, per * 30.0), "{low3} {high2}");
    // The series lines have `a:ln/a:noFill`: nothing else stroked in the series colours.
    let accents: Vec<Rgb> = THEME_COLORS.to_vec();
    assert!(strokes(&items).iter().all(|s| s.0 == Rgb::BLACK || !accents.contains(&s.0) || s.1.len() <= 2), "no series lines");
}

#[test]
fn stock_chart_up_down_bars_span_open_to_close() {
    let series = format!(
        "{}{}{}{}",
        ser(0, "Open", &CATS[..2], &[10.0, 30.0], no_line()),
        ser(1, "High", &CATS[..2], &[25.0, 35.0], no_line()),
        ser(2, "Low", &CATS[..2], &[5.0, 12.0], no_line()),
        ser(3, "Close", &CATS[..2], &[20.0, 15.0], no_line())
    );
    let extra = format!(
        r#"<c:hiLowLines/><c:upDownBars><c:gapWidth val="150"/><c:upBars><c:spPr>{}</c:spPr></c:upBars><c:downBars><c:spPr>{}</c:spPr></c:downBars></c:upDownBars>"#,
        solid("00FF00"),
        solid("FF0000")
    );
    let items = draw(&stock(&series, &extra), false);
    let f = fills(&items);
    let up: Vec<_> = f.iter().filter(|x| x.0 == Rgb(0, 0xFF, 0)).collect();
    let down: Vec<_> = f.iter().filter(|x| x.0 == Rgb(0xFF, 0, 0)).collect();
    assert_eq!((up.len(), down.len()), (1, 1), "an up bar for Q1 (10 -> 20), a down bar for Q2 (30 -> 15)");
    let (u, d) = (up[0], down[0]);
    // Heights 10 and 15 on the same scale.
    assert!(near((u.4 - u.2) / (d.4 - d.2), 10.0 / 15.0), "heights {} {}", u.4 - u.2, d.4 - d.2);
    // gapWidth 150: a bar is band / 2.5 wide, centred on its category.
    let band = (d.1 + d.3) / 2.0 - (u.1 + u.3) / 2.0;
    assert!(band > 0.0);
    assert!(near((u.3 - u.1) / band, 0.4) && near((d.3 - d.1) / band, 0.4), "widths {} {} band {band}", u.3 - u.1, d.3 - d.1);
    // The bars are on top of the hi-low lines.
    let hl =
        strokes(&items).into_iter().filter(|s| s.1.len() == 2 && near(s.1[0].0, s.1[1].0) && near(s.1[0].0, (u.1 + u.3) / 2.0)).map(|s| s.2).max();
    assert!(hl.is_some_and(|k| k < u.5), "Q1 hi-low line drawn before (under) its up bar");
}

#[test]
fn deleted_secondary_axis_keeps_its_scale_but_draws_no_labels() {
    let deleted = val_ax(4, 3, true, Some(0.4)).replace(r#"<c:delete val="0"/>"#, r#"<c:delete val="1"/>"#);
    let plot = format!(
        "{}{}{}{}{}",
        bar_group(&ser(0, "Units", &CATS, &[100.0, 200.0, 300.0], &solid(BAR)), (1, 2)),
        line_group(&ser(1, "Margin", &CATS, &[0.1, 0.2, 0.3], &format!("<a:ln>{}</a:ln>", solid(LINE))), (3, 4)),
        cat_ax(1, 2),
        val_ax(2, 1, false, Some(400.0)),
        deleted
    );
    let items = draw(&plot, false);
    let bars: Vec<_> = fills(&items).into_iter().filter(|f| f.0 == Rgb(0x44, 0x72, 0xC4)).collect();
    let line = strokes(&items).into_iter().find(|s| s.0 == Rgb(0xED, 0x7D, 0x31)).expect("line drawn").1;
    for (p, b) in line.iter().zip(&bars) {
        assert!(near(p.1, b.2), "secondary scale kept: {} vs {}", p.1, b.2);
    }
    let t = texts(&items);
    assert!(!t.iter().any(|(s, _)| s == "0.4"), "no secondary labels: {t:?}");
}

#[test]
fn stock_chart_without_hi_low_lines_draws_its_series_lines() {
    let series = format!(
        "{}{}{}",
        ser(0, "High", &CATS, &[30.0, 35.0, 25.0], ""),
        ser(1, "Low", &CATS, &[10.0, 20.0, 5.0], ""),
        ser(2, "Close", &CATS, &[20.0, 22.0, 15.0], "")
    );
    let items = draw(&stock(&series, ""), false);
    assert!(!texts(&items).iter().any(|t| t.0.contains("not supported")));
    let three: Vec<_> = strokes(&items).into_iter().filter(|s| s.1.len() == 3).collect();
    assert_eq!(three.len(), 3, "a line per series");
    assert!(!strokes(&items).iter().any(|s| s.0 == TEXT && s.1.len() == 2 && near(s.1[0].0, s.1[1].0)), "no hi-low lines");
}

#[test]
fn a_combo_skips_an_unsupported_group_and_caps_series_over_all_groups() {
    let plot = format!(
        "{}<c:bubbleChart>{}</c:bubbleChart>{}{}{}",
        bar_group(&ser(0, "Sales", &CATS, &[1.0, 2.0, 3.0], &solid(BAR)), (1, 2)),
        ser(1, "Bubbles", &CATS, &[1.0, 2.0, 3.0], ""),
        line_group(&ser(2, "Trend", &CATS, &[2.0, 2.0, 2.0], &format!("<a:ln>{}</a:ln>", solid(LINE))), (1, 2)),
        cat_ax(1, 2),
        val_ax(2, 1, false, None)
    );
    let items = draw(&plot, true);
    let t: Vec<String> = texts(&items).into_iter().map(|t| t.0).collect();
    assert!(!t.iter().any(|s| s.contains("not supported") || s == "Bubbles"), "{t:?}");
    assert!(t.iter().any(|s| s == "Sales") && t.iter().any(|s| s == "Trend"), "{t:?}");

    // MAX_SERIES counts the series of every group together.
    let many = |k: usize| (0..k).map(|i| ser(i, "S", &CATS[..1], &[1.0], "")).collect::<String>();
    let x = format!(
        r#"<c:chartSpace {NS}><c:chart><c:plotArea>{}{}{}{}</c:plotArea></c:chart></c:chartSpace>"#,
        bar_group(&many(MAX_SERIES - 1), (1, 2)),
        line_group(&many(10), (1, 2)),
        cat_ax(1, 2),
        val_ax(2, 1, false, None)
    );
    let chart = Chart::parse(&crate::xml::parse(x.as_bytes()).unwrap(), &THEME_COLORS).expect("chart");
    assert_eq!(chart.groups.iter().map(|g| g.series.len()).sum::<usize>(), MAX_SERIES);
    assert_eq!(chart.groups.len(), 2);
}

// ---- PR #5972 review ----

/// Text items with their rects (x, y, w, h).
fn text_rects(items: &[GraphicItem]) -> Vec<(String, [f32; 4])> {
    items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Text { text, rect, .. } => Some((text.clone(), *rect)),
            _ => None,
        })
        .collect()
}

fn gridlines(items: &[GraphicItem]) -> usize {
    strokes(items).iter().filter(|s| s.0 == GRID).count()
}

#[test]
fn hostile_axis_ids_do_not_hang() {
    // Each chart type names 200,000 axis ids that match nothing, and 100,000 value axes come
    // before the real one: a nested scan is ~10^10 comparisons per group.
    let junk_ids: String = (0..200_000).map(|i| format!(r#"<c:axId val="{}"/>"#, 1_000_000 + i)).collect();
    let junk_axes: String = (0..100_000).map(|i| format!(r#"<c:valAx><c:axId val="{}"/></c:valAx>"#, 5_000_000 + i)).collect();
    let bar = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{junk_ids}</c:barChart>"#,
        ser(0, "Sales", &CATS, &[10.0, 20.0, 30.0], &solid(BAR))
    );
    let line = format!(
        r#"<c:lineChart><c:grouping val="standard"/>{}{junk_ids}</c:lineChart>"#,
        ser(1, "Average", &CATS, &[15.0, 25.0, 5.0], &format!("<a:ln>{}</a:ln>", solid(LINE)))
    );
    let plot = format!("{bar}{line}{}{junk_axes}{}", cat_ax(1, 2), val_ax(2, 1, false, None));
    let start = std::time::Instant::now();
    let items = draw(&plot, false);
    let took = start.elapsed();
    assert!(took < std::time::Duration::from_secs(5), "took {took:?}");
    assert_eq!(fills(&items).iter().filter(|f| f.0 == Rgb(0x44, 0x72, 0xC4)).count(), 3, "three columns");
    assert!(strokes(&items).iter().any(|s| s.0 == Rgb(0xED, 0x7D, 0x31) && s.1.len() == 3), "the line is drawn");
}

#[test]
fn dangling_axis_ids_fall_back_to_the_primary_axis() {
    let plot = format!(
        "{}{}{}{}",
        bar_group(&ser(0, "Units", &CATS, &[100.0, 200.0, 300.0], &solid(BAR)), (1, 2)),
        line_group(&ser(1, "Margin", &CATS, &[100.0, 200.0, 300.0], &format!("<a:ln>{}</a:ln>", solid(LINE))), (7, 8)),
        cat_ax(1, 2),
        val_ax(2, 1, false, Some(400.0))
    );
    let items = draw(&plot, false);
    let bars: Vec<_> = fills(&items).into_iter().filter(|f| f.0 == Rgb(0x44, 0x72, 0xC4)).collect();
    let line = strokes(&items).into_iter().find(|s| s.0 == Rgb(0xED, 0x7D, 0x31)).expect("line drawn").1;
    for (i, (p, b)) in line.iter().zip(&bars).enumerate() {
        assert!(near(p.1, b.2), "point {i} on the primary scale: {} vs column top {}", p.1, b.2);
    }
    let right_edge = bars.iter().map(|b| b.3).fold(0.0f32, f32::max);
    assert!(!texts(&items).iter().any(|(_, x)| *x > right_edge), "no right-hand labels");
}

#[test]
fn duplicate_axis_ids_draw_the_series_once() {
    let line = format!(
        r#"<c:lineChart><c:grouping val="standard"/>{}<c:axId val="3"/><c:axId val="3"/><c:axId val="4"/><c:axId val="4"/></c:lineChart>"#,
        ser(1, "Margin", &CATS, &[0.1, 0.2, 0.3], &format!("<a:ln>{}</a:ln>", solid(LINE)))
    );
    let plot = format!(
        "{}{line}{}{}{}",
        bar_group(&ser(0, "Units", &CATS, &[100.0, 200.0, 300.0], &solid(BAR)), (1, 2)),
        cat_ax(1, 2),
        val_ax(2, 1, false, Some(400.0)),
        val_ax(4, 3, true, Some(0.4))
    );
    let items = draw(&plot, false);
    assert_eq!(strokes(&items).iter().filter(|s| s.0 == Rgb(0xED, 0x7D, 0x31) && s.1.len() == 3).count(), 1);
}

#[test]
fn horizontal_bar_combo_labels_its_secondary_axis_along_the_top() {
    let across = |series: &str, ax: (u32, u32)| {
        format!(
            r#"<c:barChart><c:barDir val="bar"/><c:grouping val="clustered"/>{series}<c:gapWidth val="150"/><c:axId val="{}"/><c:axId val="{}"/></c:barChart>"#,
            ax.0, ax.1
        )
    };
    let val = |id: u32, cross: u32, pos: &str, max: f64, crosses: &str| {
        format!(
            r#"<c:valAx><c:axId val="{id}"/><c:scaling><c:orientation val="minMax"/><c:max val="{max}"/><c:min val="0"/></c:scaling><c:delete val="0"/><c:axPos val="{pos}"/><c:numFmt formatCode="General" sourceLinked="1"/><c:crossAx val="{cross}"/><c:crosses val="{crosses}"/><c:crossBetween val="between"/></c:valAx>"#
        )
    };
    let cat = |id: u32, cross: u32, deleted: u8| {
        format!(
            r#"<c:catAx><c:axId val="{id}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="{deleted}"/><c:axPos val="l"/><c:crossAx val="{cross}"/></c:catAx>"#
        )
    };
    let plot = format!(
        "{}{}{}{}{}{}",
        across(&ser(0, "Units", &CATS, &[100.0, 200.0, 300.0], &solid(BAR)), (1, 2)),
        across(&ser(1, "Margin", &CATS, &[0.1, 0.2, 0.3], &solid(LINE)), (3, 4)),
        cat(1, 2, 0),
        val(2, 1, "b", 400.0, "autoZero"),
        cat(3, 4, 1),
        val(4, 3, "t", 0.4, "max")
    );
    let items = draw(&plot, false);
    let bars: Vec<_> = fills(&items).into_iter().filter(|f| f.0 == Rgb(0x44, 0x72, 0xC4) || f.0 == Rgb(0xED, 0x7D, 0x31)).collect();
    assert_eq!(bars.len(), 6, "both groups' bars");
    let top = bars.iter().map(|b| b.2).fold(f32::INFINITY, f32::min);
    let bottom = bars.iter().map(|b| b.4).fold(f32::NEG_INFINITY, f32::max);
    let t = text_rects(&items);
    let sec = t.iter().find(|(s, _)| s == "0.4").unwrap_or_else(|| panic!("secondary labels drawn: {t:?}")).1;
    let pri = t.iter().find(|(s, _)| s == "400").unwrap_or_else(|| panic!("primary labels drawn: {t:?}")).1;
    assert!(sec[1] + sec[3] <= top + 0.05, "0.4 above the bars: {sec:?} vs top {top}");
    assert!(pri[1] >= bottom - 0.05, "400 below the bars: {pri:?} vs bottom {bottom}");
    // Both maxima sit at the plot's right end.
    let (sc, pc) = (sec[0] + sec[2] / 2.0, pri[0] + pri[2] / 2.0);
    assert!((sc - pc).abs() <= 1.0, "0.4 centred on the right end like 400: {sc} vs {pc}");
}

#[test]
fn secondary_axis_draws_no_gridlines() {
    let bar = bar_group(&ser(0, "Units", &CATS, &[100.0, 200.0, 300.0], &solid(BAR)), (1, 2));
    let with = format!(
        "{bar}{}{}{}{}{}",
        line_group(&ser(1, "Margin", &CATS, &[0.1, 0.2, 0.3], &format!("<a:ln>{}</a:ln>", solid(LINE))), (3, 4)),
        cat_ax(1, 2),
        val_ax(2, 1, false, Some(400.0)),
        r#"<c:catAx><c:axId val="3"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="1"/><c:axPos val="b"/><c:crossAx val="4"/></c:catAx>"#,
        val_ax(4, 3, true, Some(0.4))
    );
    let without = format!("{bar}{}{}", cat_ax(1, 2), val_ax(2, 1, false, Some(400.0)));
    let (a, b) = (gridlines(&draw(&with, false)), gridlines(&draw(&without, false)));
    assert!(b > 0, "the primary axis draws gridlines");
    assert_eq!(a, b, "the secondary axis adds none");
}

#[test]
fn combo_legend_follows_c_order_across_groups() {
    let sales = ser(0, "Sales", &CATS, &[10.0, 20.0, 30.0], "").replace(r#"<c:order val="0"/>"#, r#"<c:order val="1"/>"#);
    let average = ser(1, "Average", &CATS, &[15.0, 25.0, 5.0], "").replace(r#"<c:order val="1"/>"#, r#"<c:order val="0"/>"#);
    let plot = format!("{}{}{}{}", bar_group(&sales, (1, 2)), line_group(&average, (1, 2)), cat_ax(1, 2), val_ax(2, 1, false, None));
    let t = texts(&draw(&plot, true));
    let x = |name: &str| t.iter().find(|(s, _)| s == name).map(|(_, x)| *x).unwrap_or_else(|| panic!("{name} in the legend: {t:?}"));
    assert!(x("Average") < x("Sales"), "order 0 first: {t:?}");
}
