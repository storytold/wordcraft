use super::model::MarkerShape;
use super::*;
use wordcraft_doc::THEME_COLORS;

const NS: &str =
    r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

fn space(plot: &str) -> El {
    let x = format!(r#"<c:chartSpace {NS}><c:chart><c:plotArea>{plot}</c:plotArea></c:chart></c:chartSpace>"#);
    crate::xml::parse(x.as_bytes()).unwrap()
}

fn theme() -> Vec<Rgb> {
    THEME_COLORS.to_vec()
}

fn ser(i: usize, name: &str, color: Option<&str>, cats: &[&str], vals: &[&str]) -> String {
    let fill = color.map_or(String::new(), |c| format!(r#"<c:spPr><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></c:spPr>"#));
    let cat_pts: String = cats.iter().enumerate().map(|(k, c)| format!(r#"<c:pt idx="{k}"><c:v>{c}</c:v></c:pt>"#)).collect();
    let val_pts: String = vals.iter().enumerate().map(|(k, v)| format!(r#"<c:pt idx="{k}"><c:v>{v}</c:v></c:pt>"#)).collect();
    format!(
        r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/><c:tx><c:strRef><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{name}</c:v></c:pt></c:strCache></c:strRef></c:tx>{fill}<c:cat><c:strRef><c:strCache><c:ptCount val="{nc}"/>{cat_pts}</c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="{nv}"/>{val_pts}</c:numCache></c:numRef></c:val></c:ser>"#,
        nc = cats.len(),
        nv = vals.len()
    )
}

fn filled(items: &[GraphicItem]) -> Vec<(Rgb, f32, f32)> {
    items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Path { segs, fill: Some(c), .. } => {
                let ys: Vec<f32> = segs
                    .iter()
                    .filter_map(|s| match s {
                        PathSeg::Move(_, y) | PathSeg::Line(_, y) => Some(*y),
                        _ => None,
                    })
                    .collect();
                let lo = ys.iter().copied().fold(f32::INFINITY, f32::min);
                let hi = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                Some((*c, lo, hi))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn clustered_columns_draw_one_bar_per_point() {
    let plot = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/><c:majorGridlines/></c:valAx>"#,
        ser(0, "North", Some("FF0000"), &["Q1", "Q2", "Q3"], &["4", "8", "2"]),
        ser(1, "South", Some("00FF00"), &["Q1", "Q2", "Q3"], &["2", "6", "4"]),
    );
    let items = chart_items(&space(&plot), &theme(), 300.0, 200.0);
    let bars = filled(&items);
    assert_eq!(bars.len(), 6);
    let heights: Vec<f32> = bars.iter().map(|(_, lo, hi)| hi - lo).collect();
    // Scale 0..8: North Q2 is the full plot height, North Q1 half of it.
    assert!((heights[1] / heights[0] - 2.0).abs() < 1e-3, "{heights:?}");
    // South (second series) Q1 is 2 of 8, Q2 is 6 of 8.
    assert!((heights[3] / heights[1] - 0.25).abs() < 1e-3, "{heights:?}");
    assert!((heights[4] / heights[1] - 0.75).abs() < 1e-3, "{heights:?}");
    assert_eq!(bars[0].0, Rgb(0xFF, 0, 0));
    assert_eq!(bars[3].0, Rgb(0, 0xFF, 0));
    assert!(items.iter().any(|it| matches!(it, GraphicItem::Text { text, .. } if text == "Q2")));
}

#[test]
fn data_labels_show_the_values() {
    let texts = |items: &[GraphicItem]| -> Vec<String> {
        items.iter().filter_map(|it| if let GraphicItem::Text { text, .. } = it { Some(text.clone()) } else { None }).collect()
    };
    let labels = r#"<c:dLbls><c:showLegendKey val="0"/><c:showVal val="1"/></c:dLbls>"#;
    let plot = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{labels}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx>"#,
        ser(0, "North", None, &["Q1", "Q2"], &["4.25", "17"]),
    );
    let shown = texts(&chart_items(&space(&plot), &theme(), 300.0, 200.0));
    assert!(shown.contains(&"4.25".to_string()) && shown.contains(&"17".to_string()), "{shown:?}");
    let pie = format!(r#"<c:pieChart><c:varyColors val="1"/>{}{labels}</c:pieChart>"#, ser(0, "S", None, &["A", "B"], &["1", "3"]));
    let shown = texts(&chart_items(&space(&pie), &theme(), 200.0, 150.0));
    assert!(shown.contains(&"3".to_string()), "{shown:?}");
    // Without `c:dLbls`, no values are written next to the points.
    let plain = plot.replace(labels, "");
    assert!(!texts(&chart_items(&space(&plain), &theme(), 300.0, 200.0)).contains(&"4.25".to_string()));
}

#[test]
fn pie_draws_one_slice_per_point() {
    let plot =
        format!(r#"<c:pieChart><c:varyColors val="1"/>{}</c:pieChart>"#, ser(0, "Share", None, &["A", "B", "C", "D"], &["50", "25", "15", "10"]));
    let items = chart_items(&space(&plot), &theme(), 200.0, 150.0);
    let slices = filled(&items);
    assert_eq!(slices.len(), 4);
    assert_eq!(slices[0].0, theme()[4], "first slice is accent1");
}

#[test]
fn doughnut_slices_are_annuli() {
    let plot =
        format!(r#"<c:doughnutChart><c:varyColors val="1"/>{}<c:holeSize val="50"/></c:doughnutChart>"#, ser(0, "S", None, &["A", "B"], &["1", "3"]));
    let items = chart_items(&space(&plot), &theme(), 200.0, 150.0);
    assert_eq!(items.iter().filter(|it| matches!(it, GraphicItem::Path { fill: Some(_), .. })).count(), 2);
}

#[test]
fn nice_scale_rounds_to_round_steps() {
    assert_eq!(nice_scale(0.0, 22.3, true, None, None), Scale { lo: 0.0, hi: 25.0, step: 5.0 });
    assert_eq!(nice_scale(0.0, 100.0, true, None, None), Scale { lo: 0.0, hi: 120.0, step: 20.0 });
    assert_eq!(nice_scale(0.0, 22.3, true, Some(0.0), Some(30.0)).hi, 30.0);
    let flat = nice_scale(0.0, 0.0, true, None, None);
    assert!(flat.lo == 0.0 && (flat.hi - 1.2).abs() < 1e-9 && flat.step == 0.2, "{flat:?}");
}

#[test]
fn number_formats() {
    assert_eq!(format_value(1234567.0, "#,##0", 1.0), "1,234,567");
    assert_eq!(format_value(-1500.0, "#,##0", 500.0), "-1,500");
    assert_eq!(format_value(0.25, "0%", 0.05), "25%");
    assert_eq!(format_value(0.5, "General", 0.25), "0.50");
    assert_eq!(format_value(10.0, "\"€\"0.00", 5.0), "€10.00");
}

#[test]
fn hostile_values_never_panic() {
    let bodies = [
        format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="stacked"/><c:gapWidth val="-9999999999999999999999"/><c:overlap val="NaN"/>{}<c:axId val="1"/></c:barChart><c:valAx><c:axId val="1"/><c:scaling><c:max val="1e400"/><c:min val="-1e308"/></c:scaling></c:valAx>"#,
            r#"<c:ser><c:tx><c:v>x</c:v></c:tx><c:cat><c:strRef><c:strCache><c:ptCount val="99999999999"/><c:pt idx="18446744073709551615"><c:v>a</c:v></c:pt><c:pt idx="-3"><c:v>b</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:ptCount val="3"/><c:pt idx="0"><c:v>1e400</c:v></c:pt><c:pt idx="1"><c:v>NaN</c:v></c:pt><c:pt idx="2"><c:v>-1e308</c:v></c:pt><c:pt idx="99999"><c:v>5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:ser><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1e308</c:v></c:pt><c:pt idx="0"><c:v>1e308</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#
        ),
        r#"<c:scatterChart><c:ser><c:xVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>-1e308</c:v></c:pt><c:pt idx="1"><c:v>1e308</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:yVal></c:ser></c:scatterChart>"#.to_string(),
        r#"<c:pieChart><c:ser><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>-4</c:v></c:pt><c:pt idx="1"><c:v>1e308</c:v></c:pt></c:numLit></c:val><c:dPt><c:idx val="9999999999999"/></c:dPt></c:ser><c:firstSliceAng val="-9"/></c:pieChart>"#.to_string(),
    ];
    for body in &bodies {
        let s = space(body);
        for (w, h) in [(0.0, 10.0), (f32::NAN, 10.0), (-1.0, 1.0), (1e30, 1e30), (50.0, 40.0)] {
            let _ = chart_items(&s, &theme(), w, h);
        }
    }
    assert!(chart_items(&space(&bodies[0]), &theme(), f32::NAN, 10.0).is_empty());
    assert!(chart_items(&El::default(), &theme(), 100.0, 100.0).is_empty());
}

fn texts(items: &[GraphicItem]) -> Vec<&str> {
    items.iter().filter_map(|it| if let GraphicItem::Text { text, .. } = it { Some(text.as_str()) } else { None }).collect()
}

#[test]
fn unsupported_chart_types_draw_a_placeholder() {
    let ser = r#"<c:ser><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser>"#;
    let title = r#"<c:title><c:tx><c:rich><a:p><a:r><a:t>Skills</a:t></a:r></a:p></c:rich></c:tx></c:title>"#;
    let x = format!(r#"<c:chartSpace {NS}><c:chart>{title}<c:plotArea><c:bubbleChart>{ser}</c:bubbleChart></c:plotArea></c:chart></c:chartSpace>"#);
    let bubble = chart_items(&crate::xml::parse(x.as_bytes()).unwrap(), &theme(), 300.0, 200.0);
    assert_eq!(texts(&bubble), ["Skills", "Bubble chart (not supported yet)"]);
    let framed = bubble.iter().any(|it| matches!(it, GraphicItem::Shape { rect, stroke: Some(_), .. } if *rect == [0.0, 0.0, 300.0, 200.0]));
    assert!(framed, "a light border marks the chart's frame: {bubble:?}");

    let stock = chart_items(&space(&format!("<c:stockChart>{ser}{ser}{ser}</c:stockChart>")), &theme(), 300.0, 200.0);
    assert_eq!(texts(&stock), ["Stock chart (not supported yet)"]);
    let other = chart_items(&space(&format!("<c:fancyChart>{ser}</c:fancyChart>")), &theme(), 300.0, 200.0);
    assert_eq!(texts(&other), ["Chart (not supported yet)"]);
    // A supported type in the plot area is drawn, an unsupported one beside it is skipped.
    let combo = chart_items(&space(&format!("<c:stockChart>{ser}</c:stockChart><c:lineChart>{ser}</c:lineChart>")), &theme(), 300.0, 200.0);
    assert!(!texts(&combo).iter().any(|t| t.contains("not supported")), "{combo:?}");
}

fn radar_chart(style: &str, sers: &[String]) -> Vec<GraphicItem> {
    let plot = format!(r#"<c:radarChart><c:radarStyle val="{style}"/>{}</c:radarChart><c:valAx><c:majorGridlines/></c:valAx>"#, sers.concat());
    chart_items(&space(&plot), &theme(), 300.0, 220.0)
}

#[test]
fn radar_charts_draw_spokes_grid_and_series_polygons() {
    let cats = ["Speed", "Power", "Range", "Armor", "Luck"];
    let sers = [ser(0, "A", Some("FF0000"), &cats, &["3", "4", "5", "2", "1"]), ser(1, "B", Some("0000FF"), &cats, &["2", "2", "3", "5", "4"])];
    let std = radar_chart("standard", &sers);
    assert!(!texts(&std).iter().any(|t| t.contains("not supported")), "{std:?}");
    for c in cats {
        assert!(texts(&std).contains(&c), "category label {c}: {:?}", texts(&std));
    }
    let stroked = |items: &[GraphicItem], col: Rgb| {
        items.iter().filter(|it| matches!(it, GraphicItem::Path { stroke: Some(c), segs, .. } if *c == col && segs.len() == 6)).count()
    };
    assert_eq!(stroked(&std, Rgb(0xFF, 0, 0)), 1, "one closed pentagon per series: {std:?}");
    assert_eq!(stroked(&std, Rgb(0, 0, 0xFF)), 1);
    assert!(filled(&std).is_empty(), "standard radar is not filled");
    assert!(!std.iter().any(|it| matches!(it, GraphicItem::Shape { kind: ShapeKind::Ellipse, .. })));
    // 5 spokes (2 segs each) plus grid pentagons.
    let spokes = std.iter().filter(|it| matches!(it, GraphicItem::Path { stroke: Some(c), segs, .. } if *c == GRID && segs.len() == 2)).count();
    assert_eq!(spokes, 5);
    assert!(std.iter().any(|it| matches!(it, GraphicItem::Path { stroke: Some(c), segs, .. } if *c == GRID && segs.len() == 6)));

    let marked = radar_chart("marker", &sers);
    let dots = marked.iter().filter(|it| matches!(it, GraphicItem::Shape { kind: ShapeKind::Ellipse, .. })).count();
    assert_eq!(dots, 10, "a marker per point");

    let solid = radar_chart("filled", &sers);
    let f = filled(&solid);
    assert_eq!(f.len(), 2);
    assert!(f.iter().any(|(c, ..)| *c == Rgb(0xFF, 0, 0)) && f.iter().any(|(c, ..)| *c == Rgb(0, 0, 0xFF)));
}

#[test]
fn radar_charts_show_a_legend() {
    let cats = ["a", "b", "c"];
    let plot = format!("<c:radarChart>{}</c:radarChart>", ser(0, "Team", None, &cats, &["1", "2", "3"]));
    let items = with_legend(&plot, "r", 300.0, 200.0);
    assert!(texts(&items).contains(&"Team"), "{:?}", texts(&items));
}

#[test]
fn hostile_radar_charts_never_panic() {
    // No categories and no values.
    let empty = r#"<c:ser><c:cat><c:strLit><c:ptCount val="0"/></c:strLit></c:cat><c:val><c:numLit><c:ptCount val="0"/></c:numLit></c:val></c:ser>"#;
    let _ = chart_items(&space(&format!("<c:radarChart>{empty}</c:radarChart>")), &theme(), 300.0, 200.0);
    let none = chart_items(&space("<c:radarChart/>"), &theme(), 300.0, 200.0);
    assert!(none.is_empty(), "{none:?}");
    // One category: a degenerate polygon, nothing to crash on; two as well.
    for n in 1..=2 {
        let cats: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
        let refs: Vec<&str> = cats.iter().map(String::as_str).collect();
        let vals = vec!["5"; n];
        let items = radar_chart("filled", &[ser(0, "S", None, &refs, &vals)]);
        assert!(texts(&items).contains(&"c0"), "{items:?}");
    }
    // NaN, infinities, junk and a tiny frame.
    let junk = ser(0, "S", None, &["a", "b", "c"], &["NaN", "inf", "1e999"]);
    let _ = radar_chart("filled", &[junk]);
    let small =
        chart_items(&space(&format!("<c:radarChart>{}</c:radarChart>", ser(0, "S", None, &["a", "b", "c"], &["1", "2", "3"]))), &theme(), 20.0, 20.0);
    let _ = small;
    // Caps: thousands of categories and series stay bounded.
    let cats: Vec<String> = (0..5000).map(|i| format!("c{i}")).collect();
    let refs: Vec<&str> = cats.iter().map(String::as_str).collect();
    let vals = vec!["1"; 5000];
    let many: Vec<String> = (0..300).map(|i| ser(i, "S", None, &refs, &vals)).collect();
    let items = radar_chart("marker", &many);
    let polygons = items.iter().filter(|it| matches!(it, GraphicItem::Path { stroke: Some(c), .. } if *c != GRID)).count();
    assert!(polygons <= MAX_RADAR_SERIES, "{polygons}");
    let spokes = items.iter().filter(|it| matches!(it, GraphicItem::Path { stroke: Some(c), segs, .. } if *c == GRID && segs.len() == 2)).count();
    assert_eq!(spokes, MAX_SPOKES);
}

/// Chart items for a chart with a legend at `pos` (`r`, `b`…).
fn with_legend(plot: &str, pos: &str, w: f32, h: f32) -> Vec<GraphicItem> {
    let x = format!(
        r#"<c:chartSpace {NS}><c:chart><c:plotArea>{plot}</c:plotArea><c:legend><c:legendPos val="{pos}"/></c:legend></c:chart></c:chartSpace>"#
    );
    chart_items(&crate::xml::parse(x.as_bytes()).unwrap(), &theme(), w, h)
}

#[test]
fn series_are_capped_to_the_cell_budget() {
    // 256 series each declaring 10,000 points: only the 10 that fit the cell budget are kept.
    let sers: String = (0..256)
        .map(|i| format!(r#"<c:ser><c:idx val="{i}"/><c:val><c:numRef><c:numCache><c:ptCount val="10000"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#))
        .collect();
    let plot = format!(r#"<c:lineChart><c:grouping val="standard"/>{sers}<c:axId val="1"/></c:lineChart>"#);
    let items = chart_items(&space(&plot), &theme(), 300.0, 200.0);
    let strokes = items.iter().filter(|it| matches!(it, GraphicItem::Path { fill: None, stroke: Some(_), .. })).count();
    assert_eq!(strokes, MAX_CELLS / 10_000);
}

/// A combination's chart types share one cell budget: once the first type has spent it, the
/// next one's series are not parsed at all.
#[test]
fn combination_chart_types_share_one_cell_budget() {
    let sers: String = (0..20)
        .map(|i| format!(r#"<c:ser><c:idx val="{i}"/><c:val><c:numRef><c:numCache><c:ptCount val="10000"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#))
        .collect();
    let ty = |name: &str| crate::xml::parse(format!(r#"<c:{name} {NS}>{sers}</c:{name}>"#).as_bytes()).unwrap();
    let mut budget = MAX_CELLS;
    assert_eq!(read_series(&ty("barChart"), false, &theme(), &mut budget).len(), MAX_CELLS / 10_000);
    assert_eq!(budget, 0);
    assert!(read_series(&ty("lineChart"), false, &theme(), &mut budget).is_empty(), "nothing is left for the line");
}

#[test]
fn series_without_values_draw_nothing() {
    let plot = format!(
        r#"<c:areaChart><c:grouping val="standard"/>{}{}<c:axId val="1"/></c:areaChart>"#,
        ser(0, "A", Some("FF0000"), &["Q1", "Q2"], &["4", "8"]),
        ser(1, "Empty", Some("00FF00"), &["Q1", "Q2"], &[]),
    );
    let fills = filled(&chart_items(&space(&plot), &theme(), 300.0, 200.0));
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].0, Rgb(0xFF, 0, 0));
}

#[test]
fn value_scale_starts_bars_at_zero_for_negative_data() {
    // All-negative data pads below only: the axis ends at zero, as positive data's starts there.
    assert_eq!(nice_scale(-10.0, -2.0, true, None, None), Scale { lo: -12.0, hi: 0.0, step: 2.0 });
    assert_eq!(nice_scale(-1050.0, -1000.0, false, None, None).hi, -1000.0);
}

#[test]
fn narrow_line_scales_leave_zero_out_but_bars_keep_it() {
    assert!(narrow(1000.0, 1050.0) && narrow(-1050.0, -1000.0));
    assert!(!narrow(10.0, 100.0) && !narrow(0.0, 1.0));
    assert_eq!(nice_scale(1000.0, 1050.0, false, None, None).lo, 1000.0);
    assert_eq!(nice_scale(1000.0, 1050.0, true, None, None).lo, 0.0);
}

#[test]
fn horizontal_bar_labels_wrap_in_a_column_of_forty_percent() {
    let long = "A category label that is much too long for its column";
    let plot = format!(
        r#"<c:barChart><c:barDir val="bar"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx>"#,
        ser(0, "S", Some("FF0000"), &[long, "Short"], &["4", "8"]),
    );
    let items = chart_items(&space(&plot), &theme(), 300.0, 200.0);
    let labels: Vec<(&str, f32)> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Text { rect, text, align: TextAlign::Right, .. } => Some((text.as_str(), rect[2])),
            _ => None,
        })
        .collect();
    assert!(labels.iter().all(|(_, w)| *w <= 0.4 * 300.0), "{labels:?}");
    assert!(labels.iter().filter(|(t, _)| long.contains(t)).count() > 1, "wrapped: {labels:?}");
}

#[test]
fn stacked_lines_add_up_like_areas() {
    let stacked = format!(
        r#"<c:lineChart><c:grouping val="stacked"/>{}{}<c:axId val="1"/></c:lineChart>"#,
        ser(0, "A", Some("FF0000"), &["Q1", "Q2"], &["0", "10"]),
        ser(1, "B", Some("00FF00"), &["Q1", "Q2"], &["0", "5"]),
    );
    let single = format!(
        r#"<c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/></c:lineChart>"#,
        ser(0, "T", Some("0000FF"), &["Q1", "Q2"], &["0", "15"])
    );
    // Where the last point of the line drawn in `colour` is.
    let end_y = |items: &[GraphicItem], colour: Rgb| {
        items.iter().find_map(|it| match it {
            GraphicItem::Path { segs, stroke: Some(c), .. } if *c == colour => match segs.last() {
                Some(PathSeg::Line(_, y)) => Some(*y),
                _ => None,
            },
            _ => None,
        })
    };
    let top = end_y(&chart_items(&space(&stacked), &theme(), 300.0, 200.0), Rgb(0, 0xFF, 0)).expect("stacked line");
    let total = end_y(&chart_items(&space(&single), &theme(), 300.0, 200.0), Rgb(0, 0, 0xFF)).expect("single line");
    assert!((top - total).abs() < 1e-3, "stacked {top} vs {total}");
}

#[test]
fn category_numbers_show_in_their_format() {
    // Excel serial days: 45292 is 1 January 2024, 45323 is 1 February 2024.
    let cat = |code: &str| -> Vec<String> {
        let x = format!(
            r#"<c:cat {NS}><c:numRef><c:numCache><c:formatCode>{code}</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>45292</c:v></c:pt><c:pt idx="1"><c:v>45323</c:v></c:pt></c:numCache></c:numRef></c:cat>"#
        );
        labels(Some(&crate::xml::parse(x.as_bytes()).unwrap()), 10, false)
    };
    assert_eq!(cat("m/d/yyyy"), ["1/1/2024", "2/1/2024"]);
    assert_eq!(cat("d/m/yyyy"), ["1/1/2024", "1/2/2024"]);
    assert_eq!(cat("mmm-yy"), ["Jan-24", "Feb-24"]);
    assert_eq!(cat("yyyy"), ["2024", "2024"]);
    assert_eq!(cat("0.00"), ["45292.00", "45323.00"]);
    let ymd = |v: f64| excel_stamp(v, false).map(|t| (t.y, t.m, t.d));
    assert_eq!(ymd(61.0), Some((1900, 3, 1)));
    assert_eq!(ymd(45292.0), Some((2024, 1, 1)));
}

#[test]
fn category_format_code_length_is_capped() {
    let cat = |code: &str| -> Vec<String> {
        let x = format!(
            r#"<c:cat {NS}><c:numRef><c:numCache><c:formatCode>{code}</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numCache></c:numRef></c:cat>"#
        );
        labels(Some(&crate::xml::parse(x.as_bytes()).unwrap()), 10, false)
    };
    let ok = format!("{}0", "x".repeat(254));
    assert!(cat(&ok)[0].starts_with("xxx"));
    let long = format!("{}0", "x".repeat(300));
    assert_eq!(cat(&long), ["5"]);
}

#[test]
fn value_format_code_length_is_capped() {
    let val = |code: &str| -> Option<String> {
        let x = format!(
            r#"<c:val {NS}><c:numRef><c:numCache><c:formatCode>{code}</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numCache></c:numRef></c:val>"#
        );
        numbers(Some(&crate::xml::parse(x.as_bytes()).unwrap()), 10).1
    };
    let ok = format!("{}0", "x".repeat(254));
    assert_eq!(val(&ok), Some(ok.clone()));
    assert_eq!(val(&format!("{}0", "x".repeat(300))), None);
}

#[test]
fn axis_format_code_length_is_capped() {
    let fmt = |code: &str| -> Option<String> {
        let x = format!(r#"<c:valAx {NS}><c:numFmt formatCode="{code}" sourceLinked="0"/></c:valAx>"#);
        axis(Some(&crate::xml::parse(x.as_bytes()).unwrap()), &[]).fmt
    };
    let ok = format!("{}0", "x".repeat(254));
    assert_eq!(fmt(&ok), Some(ok.clone()));
    assert_eq!(fmt(&format!("{}0", "x".repeat(300))), None);
}

#[test]
fn civil_is_exact_beyond_chronos_range() {
    // Serial 1 is 1899-12-31, serial 60 Excel's phantom 1900-02-29 handled by `excel_stamp`.
    assert_eq!(civil(1), (1899, 12, 31));
    // Past `NaiveDate`'s ±262143 the 400-year cycles carry the year, exactly, both sides of
    // the epoch; values from the pre-chrono proleptic Gregorian formula.
    assert_eq!(civil(100_000_000), (275_690, 9, 11));
    assert_eq!(civil(-100_000_000), (-271_891, 4, 18));
    // The extremes must not overflow or panic (i128 intermediates).
    let _ = (civil(i64::MAX), civil(i64::MIN));
}

#[test]
fn number_formats_keep_literal_text_and_currency() {
    assert_eq!(format_value(1234.5, "#,##0.00 [$€-407]", 0.25), "1,234.50 €");
    assert_eq!(format_value(5.0, "[Red]0", 1.0), "5");
    assert_eq!(format_value(3.0, "\"Items: \"0", 1.0), "Items: 3");
    assert_eq!(format_value(3.0, "0\" units\"", 1.0), "3 units");
}

#[test]
fn side_legend_keeps_only_the_entries_that_fit() {
    let sers: String = (0..10).map(|i| ser(i, &format!("S{i}"), Some("FF0000"), &["Q1"], &["1"])).collect();
    let plot = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{sers}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx>"#
    );
    let items = with_legend(&plot, "r", 300.0, 60.0);
    let swatches = items.iter().filter(|it| matches!(it, GraphicItem::Shape { rect, .. } if rect[2] == 7.0)).count();
    assert!(swatches > 0 && swatches < 10, "{swatches} swatches");
    let bottom =
        items.iter().filter_map(|it| if let GraphicItem::Text { rect, .. } = it { Some(rect[1] + rect[3]) } else { None }).fold(0.0, f32::max);
    assert!(bottom <= 60.0, "legend text passes the chart's bottom: {bottom}");
}

/// The swatches of a legend: their (x, y).
fn swatches(items: &[GraphicItem]) -> Vec<(f32, f32)> {
    items
        .iter()
        .filter_map(|it| if let GraphicItem::Shape { rect, .. } = it { (rect[2] == SWATCH).then_some((rect[0], rect[1])) } else { None })
        .collect()
}

/// A column chart of one category with a series per name.
fn columns(names: &[&str]) -> String {
    let sers: String = names.iter().enumerate().map(|(i, n)| ser(i, n, Some("FF0000"), &["Q1"], &["1"])).collect();
    format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{sers}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx>"#
    )
}

#[test]
fn bottom_legend_wraps_into_rows_inside_the_chart() {
    let names: Vec<String> = (0..12).map(|i| format!("Region number {i}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let items = with_legend(&columns(&names), "b", 200.0, 200.0);
    let sw = swatches(&items);
    let rows: std::collections::BTreeSet<i32> = sw.iter().map(|(_, y)| *y as i32).collect();
    assert!(rows.len() > 1 && rows.len() <= MAX_LEGEND_ROWS, "rows {rows:?}");
    assert!(sw.iter().all(|(x, _)| *x >= 0.0 && *x <= 200.0), "{sw:?}");
    for it in &items {
        if let GraphicItem::Text { rect, text, .. } = it {
            assert!(rect[0] >= 0.0 && rect[0] + text_w(text, FONT) <= 200.0 + 1e-3, "{text:?} at {rect:?}");
        }
    }
}

#[test]
fn long_legend_names_are_cut_to_a_share_of_the_chart() {
    let long = "A very long series name that would push the legend past the left edge of the chart";
    let items = with_legend(&columns(&[long, "Short"]), "r", 200.0, 150.0);
    assert!(swatches(&items).iter().all(|(x, _)| *x >= 0.0), "{:?}", swatches(&items));
    let cut = items.iter().find_map(|it| match it {
        GraphicItem::Text { text, rect, .. } if text.ends_with('…') => Some((text.clone(), rect[2])),
        _ => None,
    });
    let (text, w) = cut.expect("the long name is cut");
    assert!(text_w(&text, FONT) <= 200.0 * LEGEND_SHARE && w <= 200.0 * LEGEND_SHARE + 2.0, "{text:?} {w}");
}

#[test]
fn scatter_dates_show_as_dates() {
    let pts = |vals: &[&str]| -> String { vals.iter().enumerate().map(|(k, v)| format!(r#"<c:pt idx="{k}"><c:v>{v}</c:v></c:pt>"#)).collect() };
    let plot = format!(
        r#"<c:scatterChart><c:ser><c:idx val="0"/><c:xVal><c:numRef><c:numCache><c:formatCode>m/d/yyyy</c:formatCode><c:ptCount val="3"/>{}</c:numCache></c:numRef></c:xVal><c:yVal><c:numRef><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/>{}</c:numCache></c:numRef></c:yVal></c:ser><c:axId val="1"/><c:axId val="2"/></c:scatterChart><c:valAx><c:axId val="1"/><c:axPos val="b"/></c:valAx><c:valAx><c:axId val="2"/><c:axPos val="l"/></c:valAx>"#,
        pts(&["45292", "45323", "45352"]),
        pts(&["1", "2", "3"])
    );
    let items = chart_items(&space(&plot), &theme(), 300.0, 200.0);
    let texts: Vec<&str> = items.iter().filter_map(|it| if let GraphicItem::Text { text, .. } = it { Some(text.as_str()) } else { None }).collect();
    assert!(texts.iter().any(|t| t.ends_with("/2024")), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("45")), "serials shown: {texts:?}");
}

#[test]
fn date1904_counts_from_1904() {
    assert_eq!(category_text(0.0, "m/d/yyyy", 1.0, true), "1/1/1904");
    assert_eq!(category_text(1462.0, "m/d/yyyy", 1.0, false), "1/1/1904");
    let x = format!(
        r#"<c:chartSpace {NS}><c:date1904 val="1"/><c:chart><c:plotArea><c:lineChart><c:ser><c:cat><c:numRef><c:numCache><c:formatCode>yyyy-mm-dd</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>366</c:v></c:pt></c:numCache></c:numRef></c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:lineChart><c:catAx/><c:valAx/></c:plotArea></c:chart></c:chartSpace>"#
    );
    let chart = Chart::parse(&crate::xml::parse(x.as_bytes()).unwrap(), &theme()).expect("chart");
    assert_eq!(chart.cats, ["1905-01-01"]);
}

#[test]
fn date_codes_show_times_weekdays_and_early_serials() {
    let t = |v: f64, code: &str| category_text(v, code, 1.0, false);
    // 45292 is Monday 1 January 2024; 13:05:09 into it.
    let at = 45292.0 + (13.0 * 3600.0 + 5.0 * 60.0 + 9.0) / 86_400.0;
    assert_eq!(t(at, "hh:mm:ss"), "13:05:09");
    assert_eq!(t(at, "h:mm AM/PM"), "1:05 PM");
    assert_eq!(t(at, "m:ss"), "5:09");
    assert_eq!(t(45292.0, "h:mm a/p"), "12:00 A");
    assert_eq!(t(at, "mm/dd"), "01/01");
    assert_eq!(t(45292.0, "dddd, mmmm d"), "Monday, January 1");
    assert_eq!(t(45292.0, "ddd d mmm"), "Mon 1 Jan");
    // Serials before March 1900, Excel's phantom 29 February included.
    assert_eq!(t(0.0, "m/d/yyyy"), "1/0/1900");
    assert_eq!(t(1.0, "m/d/yyyy"), "1/1/1900");
    assert_eq!(t(59.0, "m/d/yyyy"), "2/28/1900");
    assert_eq!(t(60.0, "m/d/yyyy"), "2/29/1900");
    assert_eq!(t(61.0, "m/d/yyyy dddd"), "3/1/1900 Thursday");
    // Outside the calendar: a number.
    assert_eq!(t(-1.0, "m/d/yyyy"), "-1");
}

#[test]
fn negative_sections_and_scientific_numbers() {
    assert_eq!(format_value(-5.0, "#,##0;(#,##0)", 1.0), "(5)");
    assert_eq!(format_value(5.0, "#,##0;(#,##0)", 1.0), "5");
    assert_eq!(format_value(-1500.0, "#,##0_);(#,##0)", 500.0), "(1,500)");
    assert_eq!(format_value(-2.0, "0\";\"0;-0", 1.0), "-2");
    assert_eq!(format_value(12345.0, "0.00E+00", 1.0), "1.23E+04");
    assert_eq!(format_value(0.00012, "0.0E+0", 0.0001), "1.2E-4");
    assert_eq!(format_value(0.0, "0.00E+00", 1.0), "0.00E+00");
    assert_eq!(format_value(-12345.0, "0.0E-00", 1.0), "-1.2E04");
}

/// A chart's title, legend and axis labels take the colour their own text properties state
/// (a dark chart style states light text); text that states none takes the chart's `c:txPr`
/// colour, and the default grey without one. All of it used to be the default grey, which
/// left the text of a dark chart invisible.
#[test]
fn chart_text_takes_the_colour_its_text_properties_state() {
    let fill = |c: &str| format!(r#"<a:solidFill><a:srgbClr val="{c}"/></a:solidFill>"#);
    let tx_pr = |c: &str| format!(r#"<c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr>{}</a:defRPr></a:pPr></a:p></c:txPr>"#, fill(c));
    let chart = |own: bool, chart_default: Option<&str>| {
        let rpr = if own { format!("<a:rPr>{}</a:rPr>", fill("FFFFFF")) } else { "<a:rPr/>".to_string() };
        let title = format!(r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r>{rpr}<a:t>Revenue</a:t></a:r></a:p></c:rich></c:tx></c:title>"#);
        let (cat, val, legend) = if own { (tx_pr("FF0000"), tx_pr("0000FF"), tx_pr("00FF00")) } else { Default::default() };
        let plot = format!(
            r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/>{cat}</c:catAx><c:valAx><c:axId val="2"/>{val}</c:valAx></c:plotArea>"#,
            ser(0, "North", None, &["Q1", "Q2"], &["4", "8"])
        );
        let x = format!(
            r#"<c:chartSpace {NS}><c:chart>{title}{plot}<c:legend><c:legendPos val="b"/>{legend}</c:legend></c:chart>{}</c:chartSpace>"#,
            chart_default.map(tx_pr).unwrap_or_default()
        );
        chart_items(&crate::xml::parse(x.as_bytes()).unwrap(), &theme(), 300.0, 200.0)
    };
    let colour = |items: &[GraphicItem], s: &str| {
        items.iter().find_map(|it| match it {
            GraphicItem::Text { text, color, .. } if text == s => Some(*color),
            _ => None,
        })
    };
    let own = chart(true, Some("123456"));
    assert_eq!(colour(&own, "Revenue"), Some(Rgb(0xFF, 0xFF, 0xFF)), "title");
    assert_eq!(colour(&own, "Q1"), Some(Rgb(0xFF, 0x00, 0x00)), "category axis");
    assert_eq!(colour(&own, "8"), Some(Rgb(0x00, 0x00, 0xFF)), "value axis");
    assert_eq!(colour(&own, "North"), Some(Rgb(0x00, 0xFF, 0x00)), "legend");
    let inherited = chart(false, Some("123456"));
    for s in ["Revenue", "Q1", "8", "North"] {
        assert_eq!(colour(&inherited, s), Some(Rgb(0x12, 0x34, 0x56)), "{s} takes the chart's colour");
    }
    let plain = chart(false, None);
    for s in ["Revenue", "Q1", "8", "North"] {
        assert_eq!(colour(&plain, s), Some(TEXT), "{s} keeps the default grey");
    }
}

/// The texts a chart draws, with their colour-independent facts.
fn chart_texts(items: &[GraphicItem]) -> Vec<(String, [f32; 4], bool)> {
    items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Text { text, rect, vertical, .. } => Some((text.clone(), *rect, *vertical)),
            _ => None,
        })
        .collect()
}

fn whole_chart(chart: &str) -> El {
    let x = format!(r#"<c:chartSpace {NS}><c:chart>{chart}</c:chart></c:chartSpace>"#);
    crate::xml::parse(x.as_bytes()).unwrap()
}

/// A combination chart: columns and a line in one plot area, on the same axes (Word's "Clustered
/// Column - Line"). Both are drawn, and the value axis takes in the line's values too. Only the
/// first chart type used to be drawn.
#[test]
fn a_combination_chart_draws_its_columns_and_its_line_on_the_shared_axes() {
    let cats = ["A", "B", "C"];
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
        ser(0, "Costs", Some("FF0000"), &cats, &["1", "2", "3"]),
        ser(1, "Benefits", Some("00FF00"), &cats, &["2", "2", "2"]),
        ser(2, "Total", Some("0000FF"), &cats, &["3", "7", "12"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    assert_eq!(filled(&items).len(), 6, "two column series of three");
    let blue_line = items.iter().any(|it| matches!(it, GraphicItem::Path { fill: None, stroke: Some(c), .. } if *c == Rgb(0, 0, 0xFF)));
    assert!(blue_line, "the line series is drawn");
    let ticks: Vec<f64> = chart_texts(&items).iter().filter_map(|(t, _, _)| t.parse().ok()).collect();
    assert!(ticks.iter().any(|t| *t >= 12.0), "the value axis reaches the line's 12: {ticks:?}");
}

/// A line chart with markers draws one per point in the series' marker colours; a series whose
/// marker symbol is `none` draws none.
#[test]
fn line_markers_are_drawn_unless_the_series_turns_them_off() {
    let marked = |symbol: &str| {
        let ser = format!(
            r#"<c:ser><c:idx val="1"/><c:order val="0"/><c:spPr><a:ln><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></a:ln></c:spPr><c:marker>{symbol}<c:size val="5"/><c:spPr><a:solidFill><a:srgbClr val="8EA9DB"/></a:solidFill></c:spPr></c:marker><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt><c:pt idx="2"><c:v>C</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>3</c:v></c:pt><c:pt idx="2"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#
        );
        let plot = format!(
            r#"<c:plotArea><c:lineChart><c:grouping val="standard"/>{ser}<c:marker val="1"/><c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#
        );
        let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
        items
            .iter()
            .filter(
                |it| matches!(it, GraphicItem::Path { fill: Some(c), .. } | GraphicItem::Shape { fill: Some(c), .. } if *c == Rgb(0x8E, 0xA9, 0xDB)),
            )
            .count()
    };
    assert_eq!(marked(""), 3, "an automatic marker on each point");
    assert_eq!(marked(r#"<c:symbol val="square"/>"#), 3);
    assert_eq!(marked(r#"<c:symbol val="none"/>"#), 0);
}

/// `c:symbol val="auto"` asks for the series' automatic marker, as if there were no symbol: the
/// first series a diamond, the third a triangle.
#[test]
fn an_auto_marker_symbol_takes_the_series_automatic_marker() {
    let shape = |idx: usize, symbol: &str| {
        let x = format!(
            r#"<c:lineChart {NS}><c:ser><c:idx val="{idx}"/><c:order val="{idx}"/><c:marker>{symbol}</c:marker><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:marker val="1"/></c:lineChart>"#
        );
        let ty = crate::xml::parse(x.as_bytes()).unwrap();
        read_series(&ty, false, &theme(), &mut { MAX_CELLS }).first().and_then(|s| s.marker.as_ref()).map(|m| m.shape)
    };
    assert_eq!(shape(0, r#"<c:symbol val="auto"/>"#), Some(MarkerShape::Diamond));
    assert_eq!(shape(2, r#"<c:symbol val="auto"/>"#), Some(MarkerShape::Triangle));
    assert_eq!(shape(2, ""), Some(MarkerShape::Triangle));
    assert_eq!(shape(2, r#"<c:symbol val="square"/>"#), Some(MarkerShape::Square));
}

/// Display units (`c:dispUnits`): the value axis labels count thousands, and the unit's name
/// stands turned beside the axis.
#[test]
fn display_units_divide_the_axis_labels_and_name_the_unit() {
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/><c:dispUnits><c:builtInUnit val="thousands"/><c:dispUnitsLbl/></c:dispUnits></c:valAx></c:plotArea>"#,
        ser(0, "S", None, &["A", "B"], &["4000", "8000"]),
    );
    let texts = chart_texts(&chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0));
    let ticks: Vec<&str> = texts.iter().map(|(t, _, _)| t.as_str()).collect();
    assert!(ticks.contains(&"8") && !ticks.contains(&"8000"), "labels in thousands: {ticks:?}");
    let unit = texts.iter().find(|(t, _, _)| t == "Thousands").expect("the unit's name");
    assert!(unit.2, "the unit's name is turned");
}

/// A data table (`c:dTable`) under the plot: the categories head its columns, and each series
/// has a row of its values in its number format. The categories aren't drawn twice.
#[test]
fn a_data_table_lists_each_series_values_under_the_plot() {
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:dTable><c:showHorzBorder val="1"/><c:showVertBorder val="1"/><c:showOutline val="1"/><c:showKeys val="1"/></c:dTable></c:plotArea>"#,
        ser(0, "North", None, &["Q1", "Q2"], &["41", "-13"]),
        ser(1, "South", None, &["Q1", "Q2"], &["27", "33"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let texts = chart_texts(&items);
    let count = |s: &str| texts.iter().filter(|(t, _, _)| t == s).count();
    // Values that are no axis tick, so each is drawn once.
    for s in ["North", "South", "41", "-13", "27", "33"] {
        assert_eq!(count(s), 1, "{s} in the table: {texts:?}");
    }
    assert_eq!(count("Q1"), 1, "the categories head the table, once");
    let y = |s: &str| texts.iter().find(|(t, _, _)| t == s).map(|(_, r, _)| r[1]).unwrap();
    assert!(y("Q1") < y("41") && y("41") < y("27"), "header row, then a row per series");
}

/// A legend placed by hand (`c:manualLayout`, edge mode) is drawn where the file puts it, over
/// the chart, rather than in its automatic place.
#[test]
fn a_legend_placed_by_hand_is_drawn_where_the_file_puts_it() {
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea><c:legend><c:legendPos val="b"/><c:layout><c:manualLayout><c:xMode val="edge"/><c:yMode val="edge"/><c:x val="0.1"/><c:y val="0.1"/><c:w val="0.4"/><c:h val="0.1"/></c:manualLayout></c:layout></c:legend>"#,
        ser(0, "Sales", None, &["A", "B"], &["1", "2"]),
    );
    let texts = chart_texts(&chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0));
    let (_, r, _) = texts.iter().find(|(t, _, _)| t == "Sales").expect("legend entry");
    assert!(r[1] >= 18.0 && r[1] + r[3] <= 42.0, "inside the box 20..40 pt down: {r:?}");
    assert!(r[0] >= 30.0 && r[0] + r[2] <= 150.0, "inside the box 30..150 pt across: {r:?}");
}

/// A column chart with negative values: the category axis crosses the value axis at zero
/// (`c:crosses` autoZero, the default), so its line is drawn there, in the colour and width
/// its `c:spPr` states; its labels sit next to it unless `c:tickLblPos` says `low`.
#[test]
fn the_category_axis_crosses_at_zero_in_its_own_line() {
    let chart = |tick: &str| {
        let plot = format!(
            r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/>{tick}<c:spPr><a:ln w="19050"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></c:spPr></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
            ser(0, "S", Some("FF0000"), &["A", "B"], &["-2", "4"]),
        );
        chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0)
    };
    let items = chart("");
    let texts = chart_texts(&items);
    let mid = |s: &str| texts.iter().find(|(t, _, _)| t == s).map(|(_, r, _)| r[1] + r[3] / 2.0).unwrap();
    let zero = mid("0");
    let axis: Vec<(f32, f32)> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Path { segs, stroke: Some(c), stroke_width, .. } if *c == Rgb::BLACK => match segs.first() {
                Some(PathSeg::Move(_, y)) => Some((*y, *stroke_width)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(axis.len(), 1, "one black axis line: {axis:?}");
    assert!((axis[0].0 - zero).abs() < 0.5, "the axis at zero ({zero}), not the bottom: {axis:?}");
    assert!((axis[0].1 - 1.5).abs() < 0.01, "its own 1.5 pt width");
    let label_top = |items: &[GraphicItem]| chart_texts(items).iter().find(|(t, _, _)| t == "A").map(|(_, r, _)| r[1]).unwrap();
    assert!((label_top(&items) - zero).abs() < 10.0, "labels next to the axis");
    let low = chart(r#"<c:tickLblPos val="low"/>"#);
    assert!(label_top(&low) > zero + 40.0, "low labels under the plot");
}

/// Bold comes from the text properties: a bold display-units label, a title that turns bold
/// off, axis labels as they are.
#[test]
fn chart_text_is_bold_as_its_text_properties_say() {
    let plot = format!(
        r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:pPr><a:defRPr b="0"/></a:pPr><a:r><a:t>Plain title</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/><c:dispUnits><c:builtInUnit val="thousands"/><c:dispUnitsLbl><c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr b="1"/></a:pPr></a:p></c:txPr></c:dispUnitsLbl></c:dispUnits></c:valAx></c:plotArea>"#,
        ser(0, "S", None, &["A", "B"], &["4000", "8000"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let bold = |s: &str| {
        items.iter().find_map(|it| match it {
            GraphicItem::Text { text, bold, .. } if text == s => Some(*bold),
            _ => None,
        })
    };
    assert_eq!(bold("Thousands"), Some(true));
    assert_eq!(bold("Plain title"), Some(false));
    assert_eq!(bold("A"), Some(false));
}

/// The chart area's gradient (`a:gradFill`) and the plot area's own fill are painted, the
/// plot's over the chart's.
#[test]
fn chart_and_plot_area_fills_are_painted() {
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:spPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></c:spPr></c:plotArea>"#,
        ser(0, "S", Some("FF0000"), &["A", "B"], &["1", "2"]),
    );
    let x = format!(
        r#"<c:chartSpace {NS}><c:chart>{plot}</c:chart><c:spPr><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="DDDDDD"/></a:gs><a:gs pos="100000"><a:srgbClr val="999999"/></a:gs></a:gsLst><a:lin ang="5400000" scaled="1"/></a:gradFill></c:spPr></c:chartSpace>"#
    );
    let items = chart_items(&crate::xml::parse(x.as_bytes()).unwrap(), &theme(), 300.0, 200.0);
    let fills: Vec<(&[f32; 4], &wordcraft_doc::fill::Fill)> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Fill { rect, fill } => Some((rect, fill)),
            _ => None,
        })
        .collect();
    assert_eq!(fills.len(), 2, "the chart's and the plot's: {fills:?}");
    assert!(matches!(fills[0].1, wordcraft_doc::fill::Fill::Gradient(_)), "the chart area's gradient first");
    assert_eq!(fills[0].0, &[0.0, 0.0, 300.0, 200.0]);
    assert!(matches!(fills[1].1, wordcraft_doc::fill::Fill::Solid { color: Rgb::WHITE, .. }), "then the white plot");
    assert!(fills[1].0[2] < 300.0 && fills[1].0[3] < 200.0, "inside the chart");
}

/// A series filled with a gradient (Word's chart styles shade their columns) paints each
/// column with that gradient rather than the gradient's first colour.
#[test]
fn a_gradient_series_fills_its_columns_with_the_gradient() {
    let grad = r#"<c:spPr><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FFCC99"/></a:gs><a:gs pos="100000"><a:srgbClr val="CC6600"/></a:gs></a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill></c:spPr>"#;
    let ser = ser(0, "S", None, &["A", "B"], &["1", "2"]).replacen("<c:cat>", &format!("{grad}<c:cat>"), 1);
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{ser}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let shaded = items.iter().filter(|it| matches!(it, GraphicItem::Fill { fill: wordcraft_doc::fill::Fill::Gradient(_), .. })).count();
    assert_eq!(shaded, 2, "both columns shaded");
    assert!(filled(&items).is_empty(), "no flat column under them");
}

/// The title's size comes from its text properties (`sz`), 14 pt without one.
#[test]
fn the_title_takes_its_stated_size() {
    let title = |rpr: &str| {
        let plot = format!(
            r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:pPr><a:defRPr {rpr}/></a:pPr><a:r><a:t>Sized</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:barDir val="col"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
            ser(0, "S", None, &["A"], &["1"]),
        );
        chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0).iter().find_map(|it| match it {
            GraphicItem::Text { text, size, .. } if text == "Sized" => Some(*size),
            _ => None,
        })
    };
    assert_eq!(title(r#"sz="1600""#), Some(16.0));
    assert_eq!(title(""), Some(TITLE_SIZE));
}

/// A 100% stacked column chart combined with a line: only the columns become shares of their
/// category's total; the line keeps its values and doesn't count in the totals.
#[test]
fn only_the_stacked_chart_type_becomes_percentages() {
    let cats = ["A", "B"];
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="percentStacked"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
        ser(0, "Left", None, &cats, &["1", "3"]),
        ser(1, "Right", None, &cats, &["3", "1"]),
        ser(2, "Trend", None, &cats, &["10", "20"]),
    );
    let chart = Chart::parse(&whole_chart(&plot), &theme()).expect("a chart");
    let values = |j: usize| chart.series[j].values.clone();
    assert_eq!(values(0), vec![Some(0.25), Some(0.75)], "column shares of the columns' totals");
    assert_eq!(values(1), vec![Some(0.75), Some(0.25)]);
    assert_eq!(values(2), vec![Some(10.0), Some(20.0)], "the line keeps its values");
}

/// A data table's name column stays inside the margin reserved for it: a series name wider
/// than that is cut short instead of running off the chart.
#[test]
fn a_long_data_table_name_stays_inside_the_chart() {
    let long = "A very long series name that is much wider than the chart allows for its table";
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:dTable><c:showKeys val="1"/></c:dTable></c:plotArea>"#,
        ser(0, long, None, &["Q1", "Q2"], &["41", "27"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let (name, rect, _) = chart_texts(&items).into_iter().find(|(t, _, _)| t.starts_with("A very")).expect("the series name");
    assert!(rect[0] >= 0.0, "the name starts inside the chart: {rect:?}");
    assert!(name.ends_with('…'), "the name is cut short: {name}");
}

/// A 100% stacked column chart combined with a line on the same axes: the line keeps its values,
/// so the value axis takes them in rather than stopping at 100%, and the line is drawn at them,
/// not pinned to the top of the plot.
#[test]
fn a_percent_stacked_combination_scales_its_axis_to_the_line() {
    let cats = ["A", "B"];
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="percentStacked"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
        ser(0, "Left", Some("FF0000"), &cats, &["1", "3"]),
        ser(1, "Right", Some("00FF00"), &cats, &["3", "1"]),
        ser(2, "Trend", Some("0000FF"), &cats, &["10", "20"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let line_ys: Vec<f32> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Path { segs, fill: None, stroke: Some(c), .. } if *c == Rgb(0, 0, 0xFF) => Some(segs),
            _ => None,
        })
        .flat_map(|segs| {
            segs.iter().filter_map(|s| match s {
                PathSeg::Move(_, y) | PathSeg::Line(_, y) => Some(*y),
                _ => None,
            })
        })
        .collect();
    assert_eq!(line_ys.len(), 2, "two line points: {line_ys:?}");
    assert!((line_ys[0] - line_ys[1]).abs() > 10.0, "10 and 20 at different heights, not both at the top: {line_ys:?}");
    let ticks: Vec<String> = chart_texts(&items).into_iter().map(|(t, _, _)| t).filter(|t| t.ends_with('%')).collect();
    assert!(
        ticks.iter().any(|t| t.trim_end_matches('%').parse::<f64>().is_ok_and(|v| v >= 2000.0)),
        "the axis reaches the line's 20 (2000%): {ticks:?}"
    );
}
