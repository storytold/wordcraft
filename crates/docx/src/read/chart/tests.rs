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
    let x = format!(r#"<c:chartSpace {NS}><c:chart>{title}<c:plotArea><c:surfaceChart>{ser}</c:surfaceChart></c:plotArea></c:chart></c:chartSpace>"#);
    let surface = chart_items(&crate::xml::parse(x.as_bytes()).unwrap(), &theme(), 300.0, 200.0);
    assert_eq!(texts(&surface), ["Skills", "Surface chart (not supported yet)"]);
    let framed = surface.iter().any(|it| matches!(it, GraphicItem::Shape { rect, stroke: Some(_), .. } if *rect == [0.0, 0.0, 300.0, 200.0]));
    assert!(framed, "a light border marks the chart's frame: {surface:?}");

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
        axis(Some(&crate::xml::parse(x.as_bytes()).unwrap())).fmt
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

// Bubble charts (#5941). The sizes are Word's, measured in Word for Microsoft 365 from charts of
// known data: the largest bubble's diameter is `L * 3s / (3s + 1000)`, with `s` the
// `c:bubbleScale` and `L` the short side of the chart less 10 pt (no title or legend). Other
// bubbles scale from the largest size of the chart: by area (`sqrt`) or, with
// `c:sizeRepresents="w"`, by width.

/// A bubble series: `c:xVal`, `c:yVal` and, when given, `c:bubbleSize`, as literals.
fn bser(i: usize, color: &str, xs: &[&str], ys: &[&str], sizes: Option<&[&str]>) -> String {
    let lit = |tag: &str, vals: &[&str]| {
        let pts: String = vals.iter().enumerate().map(|(k, v)| format!(r#"<c:pt idx="{k}"><c:v>{v}</c:v></c:pt>"#)).collect();
        format!(r#"<c:{tag}><c:numLit><c:ptCount val="{}"/>{pts}</c:numLit></c:{tag}>"#, vals.len())
    };
    let sizes = sizes.map_or(String::new(), |s| lit("bubbleSize", s));
    format!(
        r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/><c:spPr><a:solidFill><a:srgbClr val="{color}"/></a:solidFill></c:spPr>{}{}{sizes}</c:ser>"#,
        lit("xVal", xs),
        lit("yVal", ys)
    )
}

/// A bubble chart of `w` x `h` points, both axes fixed at 0..4, with `extra` after the series.
fn bubble_chart(sers: &[String], extra: &str, w: f32, h: f32) -> Vec<GraphicItem> {
    let ax = |id: u32, pos: &str| {
        format!(
            r#"<c:valAx><c:axId val="{id}"/><c:scaling><c:orientation val="minMax"/><c:max val="4"/><c:min val="0"/></c:scaling><c:delete val="0"/><c:axPos val="{pos}"/></c:valAx>"#
        )
    };
    let plot = format!(
        r#"<c:bubbleChart><c:varyColors val="0"/>{}{extra}<c:axId val="1"/><c:axId val="2"/></c:bubbleChart>{}{}"#,
        sers.concat(),
        ax(1, "b"),
        ax(2, "l")
    );
    chart_items(&space(&plot), &theme(), w, h)
}

/// Circles filled with `col`: (centre x, centre y, diameter), in drawing order.
fn bubbles(items: &[GraphicItem], col: Rgb) -> Vec<(f32, f32, f32)> {
    items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Shape { rect: [x, y, w, h], kind: ShapeKind::Ellipse, fill: Some(c), .. } if *c == col => {
                assert!((w - h).abs() < 1e-3, "a bubble is round: {w} x {h}");
                Some((x + w / 2.0, y + h / 2.0, *w))
            }
            _ => None,
        })
        .collect()
}

/// Word's largest bubble diameter for a chart of `w` x `h` with no title or legend.
fn word_d_max(w: f32, h: f32, scale: f32) -> f32 {
    (w.min(h) - 10.0) * 3.0 * scale / (3.0 * scale + 1000.0)
}

/// Bubbles in order of their centre's x.
fn by_x(mut b: Vec<(f32, f32, f32)>) -> Vec<(f32, f32, f32)> {
    b.sort_by(|p, q| p.0.total_cmp(&q.0));
    b
}

const RED: Rgb = Rgb(0xFF, 0, 0);
const BLUE: Rgb = Rgb(0, 0, 0xFF);

/// Three red bubbles at x = 1, 2, 3 (y = 2) with sizes 1, 4 and 16.
fn three() -> Vec<String> {
    vec![bser(0, "FF0000", &["1", "2", "3"], &["2", "2", "2"], Some(&["1", "4", "16"]))]
}

/// The diameter of the largest red bubble, if any.
fn largest(items: &[GraphicItem]) -> Option<f32> {
    bubbles(items, RED).iter().map(|b| b.2).reduce(f32::max)
}

#[test]
fn bubble_charts_draw_a_circle_per_point_sized_by_area() {
    let items = bubble_chart(&three(), "", 432.0, 252.0);
    assert!(!texts(&items).iter().any(|t| t.contains("not supported")), "{:?}", texts(&items));
    let b = by_x(bubbles(&items, RED));
    assert_eq!(b.len(), 3, "a bubble per point: {items:?}");
    // Word, 432 x 252, scale 100: 13.96, 27.92 and 55.84 pt across.
    let d_max = word_d_max(432.0, 252.0, 100.0);
    assert!((b[2].2 - d_max).abs() < 0.1, "largest bubble {} vs Word's {d_max}", b[2].2);
    assert!((b[1].2 - d_max / 2.0).abs() < 0.1 && (b[0].2 - d_max / 4.0).abs() < 0.1, "area grows with size: {b:?}");
    // Centres on the scatter positions: one height, evenly spaced along x.
    assert!(b.iter().all(|p| (p.1 - b[0].1).abs() < 1e-3), "{b:?}");
    assert!(((b[1].0 - b[0].0) - (b[2].0 - b[1].0)).abs() < 1e-3 && b[1].0 > b[0].0, "{b:?}");
}

#[test]
fn bubble_size_is_relative_to_the_largest_of_the_chart() {
    let small = bser(0, "FF0000", &["1", "2"], &["1", "1"], Some(&["1", "4"]));
    let big = bser(1, "0000FF", &["1", "2"], &["3", "3"], Some(&["16", "64"]));
    let items = bubble_chart(&[small, big], "", 432.0, 252.0);
    let d_max = word_d_max(432.0, 252.0, 100.0);
    let r = by_x(bubbles(&items, RED));
    let b = by_x(bubbles(&items, BLUE));
    assert_eq!((r.len(), b.len()), (2, 2), "{items:?}");
    assert!((b[1].2 - d_max).abs() < 0.1, "{b:?}");
    assert!((r[0].2 - d_max / 8.0).abs() < 0.1, "size 1 of 64 across both series: {r:?}");
    // The same bubbles at ten times the sizes.
    let tens = bser(0, "FF0000", &["1", "2", "3"], &["2", "2", "2"], Some(&["10", "40", "160"]));
    let t = by_x(bubbles(&bubble_chart(&[tens], "", 432.0, 252.0), RED));
    assert!((t[0].2 - d_max / 4.0).abs() < 0.1 && (t[2].2 - d_max).abs() < 0.1, "{t:?}");
}

#[test]
fn bubble_scale_and_size_represents_follow_word() {
    let at = |extra: &str| largest(&bubble_chart(&three(), extra, 432.0, 252.0));
    // Word: 31.56, 90.75 and 114.63 pt. The scale isn't linear.
    for scale in [50.0, 200.0, 300.0] {
        let d = at(&format!(r#"<c:bubbleScale val="{scale}"/>"#)).unwrap_or_default();
        assert!((d - word_d_max(432.0, 252.0, scale)).abs() < 0.1, "scale {scale}: {d}");
    }
    // ISO/IEC 29500-4:2016 also allows a percent.
    let pct = at(r#"<c:bubbleScale val="150%"/>"#).unwrap_or_default();
    assert!((pct - word_d_max(432.0, 252.0, 150.0)).abs() < 0.1, "150%: {pct}");
    // Out of range clamps to 0..300, and 0 draws no bubbles.
    let huge = at(r#"<c:bubbleScale val="99999"/>"#).unwrap_or_default();
    assert!((huge - word_d_max(432.0, 252.0, 300.0)).abs() < 0.1, "{huge}");
    assert_eq!(at(r#"<c:bubbleScale val="0"/>"#), None);

    let w = by_x(bubbles(&bubble_chart(&three(), r#"<c:sizeRepresents val="w"/>"#, 432.0, 252.0), RED));
    let d_max = word_d_max(432.0, 252.0, 100.0);
    assert_eq!(w.len(), 3, "{w:?}");
    assert!(
        (w[2].2 - d_max).abs() < 0.1 && (w[1].2 - d_max / 4.0).abs() < 0.1 && (w[0].2 - d_max / 16.0).abs() < 0.1,
        "width grows with size: {w:?}"
    );
}

#[test]
fn bubble_size_follows_the_charts_short_side() {
    // Word: 43.84 pt (600 x 200), 55.38 (250 x 450), 66.92 (300 x 300), 113.08 (700 x 500).
    for (w, h) in [(600.0, 200.0), (250.0, 450.0), (300.0, 300.0), (700.0, 500.0)] {
        let d = largest(&bubble_chart(&three(), "", w, h)).unwrap_or_default();
        assert!((d - word_d_max(w, h, 100.0)).abs() < 0.1, "{w} x {h}: {d}");
    }
}

#[test]
fn zero_and_negative_bubbles() {
    let s = || vec![bser(0, "FF0000", &["1", "2", "3"], &["2", "2", "2"], Some(&["0", "-4", "16"]))];
    let ellipses = |items: &[GraphicItem]| -> Vec<f32> {
        items
            .iter()
            .filter_map(|it| match it {
                GraphicItem::Shape { rect: [_, _, w, _], kind: ShapeKind::Ellipse, .. } => Some(*w),
                _ => None,
            })
            .collect()
    };
    let off = ellipses(&bubble_chart(&s(), r#"<c:showNegBubbles val="0"/>"#, 432.0, 252.0));
    assert_eq!(off.len(), 1, "size 0 and negative sizes aren't drawn: {off:?}");
    // When shown, a negative bubble is drawn at its size's magnitude. An empty
    // `c:showNegBubbles` means true (CT_Boolean's default).
    let d_max = word_d_max(432.0, 252.0, 100.0);
    for extra in [r#"<c:showNegBubbles val="1"/>"#, "<c:showNegBubbles/>"] {
        let all = ellipses(&bubble_chart(&s(), extra, 432.0, 252.0));
        assert_eq!(all.len(), 2, "{extra}: {all:?}");
        assert!(all.iter().any(|d| (d - d_max / 2.0).abs() < 0.1), "{extra}: {all:?}");
    }
}

/// Every bubble in drawing order: (centre x, fill, outline colour, outline width).
fn bubble_paint(items: &[GraphicItem]) -> Vec<(f32, Option<Rgb>, Option<Rgb>, f32)> {
    let mut b: Vec<_> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Shape { rect: [x, _, w, _], kind: ShapeKind::Ellipse, fill, stroke, stroke_width } => {
                Some((x + w / 2.0, *fill, *stroke, *stroke_width))
            }
            _ => None,
        })
        .collect();
    b.sort_by(|p, q| p.0.total_cmp(&q.0));
    b
}

/// The series' `c:spPr` with a 1.5 pt `a:ln` of `rgb` after its fill.
fn with_series_line(ser: &str, rgb: &str) -> String {
    ser.replacen(
        "</a:solidFill></c:spPr>",
        &format!(r#"</a:solidFill><a:ln w="19050"><a:solidFill><a:srgbClr val="{rgb}"/></a:solidFill></a:ln></c:spPr>"#),
        1,
    )
}

/// A `c:dPt` for point `idx` with a line: of `rgb`, or `a:noFill` when `None`.
fn dpt_line(idx: usize, rgb: Option<&str>) -> String {
    let ln = rgb.map_or("<a:noFill/>".to_string(), |c| format!(r#"<a:solidFill><a:srgbClr val="{c}"/></a:solidFill>"#));
    format!(r#"<c:dPt><c:idx val="{idx}"/><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln w="38100">{ln}</a:ln></c:spPr></c:dPt>"#)
}

#[test]
fn negative_bubbles_are_white_with_an_outline() {
    // Word (measured): a shown negative bubble is filled white, whatever the series' colour, and
    // outlined in the point's line colour, else the series', else black. Office always inverts
    // negative bubbles (MS-OI29500 §21.2.2.86), whatever `c:invertIfNegative` says.
    let ser = bser(0, "FF0000", &["1", "2", "3"], &["2", "2", "2"], Some(&["4", "-4", "16"]));
    let shown = r#"<c:showNegBubbles val="1"/>"#;
    let plain = bubble_paint(&bubble_chart(std::slice::from_ref(&ser), shown, 432.0, 252.0));
    assert_eq!(plain.len(), 3, "{plain:?}");
    assert_eq!((plain[1].1, plain[1].2), (Some(Rgb::WHITE), Some(Rgb::BLACK)), "the negative bubble: {plain:?}");
    assert!((plain[1].3 - 0.25).abs() < 1e-3, "{plain:?}");
    assert!(plain[0].2.is_none() && plain[2].2.is_none(), "positive bubbles with no line have no outline: {plain:?}");
    assert_eq!((plain[0].1, plain[2].1), (Some(RED), Some(RED)));

    // A series line outlines every bubble, the negative one included.
    let red_line = bubble_paint(&bubble_chart(&[with_series_line(&ser, "CC2222")], shown, 432.0, 252.0));
    let line = Some(Rgb(0xCC, 0x22, 0x22));
    assert!(red_line.iter().all(|b| b.2 == line && (b.3 - 0.25).abs() < 1e-3), "{red_line:?}");
    assert_eq!(red_line[1].1, Some(Rgb::WHITE));

    // A point's own line wins, drawn 0.25 pt whatever its width (3 pt here); its `a:noFill`
    // turns the series line off for that point.
    let points = format!("{}{}<c:xVal>", dpt_line(0, None), dpt_line(1, Some("202060")));
    let own = with_series_line(&ser, "CC2222").replacen("<c:xVal>", &points, 1);
    let own = bubble_paint(&bubble_chart(&[own], shown, 432.0, 252.0));
    assert_eq!(own[0].2, None, "point 0 turns its line off: {own:?}");
    assert_eq!((own[1].1, own[1].2), (Some(Rgb::WHITE), Some(Rgb(0x20, 0x20, 0x60))), "{own:?}");
    assert!((own[1].3 - 0.25).abs() < 1e-3, "{own:?}");
    assert_eq!(own[2].2, line, "point 2 keeps the series line: {own:?}");
}

#[test]
fn bubble_points_take_their_own_colour() {
    let dpt = r#"<c:dPt><c:idx val="1"/><c:spPr><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></c:spPr></c:dPt><c:xVal>"#;
    let s = three().concat().replacen("<c:xVal>", dpt, 1);
    let items = bubble_chart(&[s], "", 432.0, 252.0);
    assert_eq!((bubbles(&items, RED).len(), bubbles(&items, BLUE).len()), (2, 1), "{items:?}");
}

#[test]
fn bubble_data_labels_show_the_values() {
    let s = bser(0, "FF0000", &["1", "2", "3"], &["1.5", "2.5", "3.5"], Some(&["1", "4", "16"]));
    let items = bubble_chart(&[s], r#"<c:dLbls><c:showVal val="1"/></c:dLbls>"#, 432.0, 252.0);
    for v in ["1.5", "2.5", "3.5"] {
        assert!(texts(&items).contains(&v), "label {v}: {:?}", texts(&items));
    }
}

#[test]
fn hostile_bubble_sizes_never_panic() {
    let cases: [&[&str]; 4] =
        [&["NaN", "inf", "-inf", "1"], &["1e308", "1e308", "1", "0"], &["-1e308", "x", "", "5"], &["1e-308", "4.9e-324", "0", "-0"]];
    let extras = ["", r#"<c:bubbleScale val="300"/><c:showNegBubbles val="1"/>"#, r#"<c:bubbleScale val="-5"/><c:sizeRepresents val="w"/>"#];
    for sizes in cases {
        let s = bser(0, "FF0000", &["1", "2", "3", "1e308"], &["2", "2", "NaN", "2"], Some(sizes));
        for extra in extras {
            for (w, h) in [(432.0, 252.0), (5.0, 5.0), (1e6, 1e6)] {
                for it in bubble_chart(std::slice::from_ref(&s), extra, w, h) {
                    if let GraphicItem::Shape { rect, .. } = it {
                        assert!(rect.iter().all(|v| v.is_finite()), "{sizes:?} {extra}: {rect:?}");
                        let inside = rect[0] >= -0.01 && rect[1] >= -0.01 && rect[0] + rect[2] <= w + 0.01 && rect[1] + rect[3] <= h + 0.01;
                        assert!(inside, "{sizes:?} {extra}: {rect:?} outside the {w} x {h} chart");
                    }
                }
            }
        }
    }
    // No `c:bubbleSize` at all, and a series with no points.
    let bare = bser(0, "FF0000", &["1", "2"], &["2", "3"], None);
    let _ = bubble_chart(&[bare, bser(1, "0000FF", &[], &[], Some(&[]))], "", 432.0, 252.0);
}

// Bubbles stay in the plot area (#5948 review). Measured in Word: automatic axes are widened so
// every bubble fits (x 1..3 with sizes 1, 4, 16 at 432 x 252 gets x 0..3.5, y 2..4 gets 0..5;
// the largest bubble at y = 10 gets y -2..12), and a bubble that crosses a fixed axis end is
// clipped to the plot area. A point outside a fixed axis range isn't drawn.

/// A bubble chart of `w` x `h` points with automatic axes (no `c:min` / `c:max`).
fn auto_bubble_chart(sers: &[String], extra: &str, w: f32, h: f32) -> Vec<GraphicItem> {
    let ax = |id: u32, pos: &str| {
        format!(
            r#"<c:valAx><c:axId val="{id}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="{pos}"/></c:valAx>"#
        )
    };
    let plot = format!(
        r#"<c:bubbleChart><c:varyColors val="0"/>{}{extra}<c:axId val="1"/><c:axId val="2"/></c:bubbleChart>{}{}"#,
        sers.concat(),
        ax(1, "b"),
        ax(2, "l")
    );
    chart_items(&space(&plot), &theme(), w, h)
}

/// The plot area's box [left, top, right, bottom], from the axis lines (x axis along the bottom,
/// y axis down the left).
fn plot_box(items: &[GraphicItem]) -> [f32; 4] {
    let lines: Vec<(f32, f32, f32, f32)> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Path { segs, stroke: Some(c), .. } if *c == AXIS => match segs.as_slice() {
                [PathSeg::Move(x0, y0), PathSeg::Line(x1, y1)] => Some((*x0, *y0, *x1, *y1)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let x_axis = lines.iter().find(|l| l.1 == l.3).expect("an x axis line");
    let y_axis = lines.iter().find(|l| l.0 == l.2).expect("a y axis line");
    [x_axis.0, y_axis.1.min(y_axis.3), x_axis.2, x_axis.1]
}

/// The box [left, top, right, bottom] of each thing painted in `col`: whole bubbles (ellipses)
/// and clipped ones (paths).
fn painted(items: &[GraphicItem], col: Rgb) -> Vec<[f32; 4]> {
    items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Shape { rect: [x, y, w, h], kind: ShapeKind::Ellipse, fill: Some(c), .. } if *c == col => Some([*x, *y, x + w, y + h]),
            GraphicItem::Path { segs, fill: Some(c), .. } if *c == col => {
                let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
                for s in segs {
                    let pts = match *s {
                        PathSeg::Move(x, y) | PathSeg::Line(x, y) => vec![(x, y)],
                        PathSeg::Cubic(a, b, c, d, e, f) => vec![(a, b), (c, d), (e, f)],
                        PathSeg::Close => vec![],
                    };
                    for (x, y) in pts {
                        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                    }
                }
                Some(b)
            }
            _ => None,
        })
        .collect()
}

/// Asserts every red and blue bubble lies inside the plot area and the `w` x `h` chart; returns
/// how many there are.
fn assert_bubbles_inside(items: &[GraphicItem], w: f32, h: f32, what: &str) -> usize {
    const EPS: f32 = 0.01;
    let p = plot_box(items);
    let all: Vec<[f32; 4]> = painted(items, RED).into_iter().chain(painted(items, BLUE)).collect();
    for b in &all {
        assert!(b[0] >= -EPS && b[1] >= -EPS && b[2] <= w + EPS && b[3] <= h + EPS, "{what}: bubble {b:?} outside the {w} x {h} chart");
        assert!(
            b[0] >= p[0] - EPS && b[1] >= p[1] - EPS && b[2] <= p[2] + EPS && b[3] <= p[3] + EPS,
            "{what}: bubble {b:?} outside the plot area {p:?}"
        );
    }
    all.len()
}

/// `ser` without its `c:xVal`: x is then 1, 2, 3...
fn without_x(ser: &str) -> String {
    let start = ser.find("<c:xVal>").expect("bser writes c:xVal");
    let end = ser.find("</c:xVal>").expect("bser writes c:xVal") + "</c:xVal>".len();
    format!("{}{}", &ser[..start], &ser[end..])
}

#[test]
fn bubbles_stay_inside_the_plot_area_on_automatic_axes() {
    let red = |xs: &[&str], ys: &[&str], sizes: &[&str]| bser(0, "FF0000", xs, ys, Some(sizes));
    let cases: [(&str, Vec<String>, usize); 7] = [
        ("default x", vec![without_x(&red(&[], &["2", "3", "4"], &["1", "4", "16"]))], 3),
        ("big at min x", vec![red(&["1", "2", "3"], &["2", "3", "4"], &["16", "4", "1"])], 3),
        ("big at max y", vec![red(&["1", "2", "3"], &["1", "2", "10"], &["1", "4", "16"])], 3),
        ("flat", vec![red(&["1", "2", "3"], &["2", "2", "2"], &["1", "4", "16"])], 3),
        ("negative", vec![red(&["-5", "0", "7"], &["-3", "2", "8"], &["2", "9", "4"])], 3),
        ("single", vec![red(&["3"], &["7"], &["5"])], 1),
        (
            "two series",
            vec![red(&["1", "2", "3"], &["2", "3", "4"], &["1", "4", "16"]), bser(1, "0000FF", &["1.5", "2.5"], &["5", "1"], Some(&["64", "2"]))],
            5,
        ),
    ];
    for (w, h) in [(432.0, 252.0), (600.0, 200.0), (250.0, 450.0), (150.0, 120.0)] {
        for extra in ["", r#"<c:bubbleScale val="200"/>"#, r#"<c:bubbleScale val="300"/>"#] {
            for (name, sers, n) in &cases {
                let what = format!("{name} {extra} {w} x {h}");
                let items = auto_bubble_chart(sers, extra, w, h);
                assert_eq!(assert_bubbles_inside(&items, w, h, &what), *n, "{what}: every bubble drawn");
                // Automatic axes leave room, so the bubbles are whole, not cut.
                assert_eq!(bubbles(&items, RED).len() + bubbles(&items, BLUE).len(), *n, "{what}: cut");
            }
        }
    }
}

#[test]
fn bubbles_are_clipped_to_the_plot_area_on_fixed_axes() {
    // Axes fixed at 0..4: bubbles centred on the corners are cut at the plot's edges, as Word cuts
    // them.
    for (extra, scale) in [("", 100.0), (r#"<c:bubbleScale val="300"/>"#, 300.0)] {
        let s = bser(0, "FF0000", &["0", "4", "2"], &["0", "4", "2"], Some(&["16", "16", "1"]));
        let items = bubble_chart(&[s], extra, 432.0, 252.0);
        assert_eq!(assert_bubbles_inside(&items, 432.0, 252.0, extra), 3, "{extra}");
        let p = plot_box(&items);
        let r = word_d_max(432.0, 252.0, scale) / 2.0;
        let cut = painted(&items, RED);
        // The bottom-left bubble keeps its upper-right quarter, the top-right its lower-left.
        let near = |a: f32, b: f32| (a - b).abs() < 0.1;
        assert!(
            cut.iter().any(|b| near(b[0], p[0]) && near(b[3], p[3]) && near(b[2], p[0] + r) && near(b[1], p[3] - r)),
            "{extra}: {cut:?} in {p:?}"
        );
        assert!(
            cut.iter().any(|b| near(b[2], p[2]) && near(b[1], p[1]) && near(b[0], p[2] - r) && near(b[3], p[1] + r)),
            "{extra}: {cut:?} in {p:?}"
        );
        // The middle one is whole.
        assert_eq!(bubbles(&items, RED).len(), 1, "{extra}");
    }
    // A point outside the fixed range isn't drawn, even when its bubble would reach in.
    let out = bser(0, "FF0000", &["5", "2"], &["2", "2"], Some(&["16", "1"]));
    let items = bubble_chart(&[out], "", 432.0, 252.0);
    assert_eq!(painted(&items, RED).len(), 1, "{items:?}");
}

#[test]
fn points_with_no_x_or_y_dont_size_the_others() {
    // Word: a size-64 point with no y (or no x) isn't drawn, and the other bubbles keep the sizes
    // they have without it (13.96, 27.92 and 55.84 pt at 432 x 252).
    let base = bser(0, "FF0000", &["1", "2", "3", "3.5"], &["2", "2", "2", "3"], Some(&["1", "4", "16", "64"]));
    let no_y = base.replacen(r#"<c:pt idx="3"><c:v>3</c:v></c:pt>"#, "", 1);
    let no_x = base.replacen(r#"<c:pt idx="3"><c:v>3.5</c:v></c:pt>"#, "", 1);
    let d_max = word_d_max(432.0, 252.0, 100.0);
    for (what, s) in [("no y", no_y), ("no x", no_x)] {
        assert_ne!(s, base, "{what}: the point was removed");
        let b = by_x(bubbles(&bubble_chart(&[s], "", 432.0, 252.0), RED));
        assert_eq!(b.len(), 3, "{what}: {b:?}");
        assert!((b[2].2 - d_max).abs() < 0.1 && (b[0].2 - d_max / 4.0).abs() < 0.1, "{what}: {b:?}");
    }
}

#[test]
fn bubble_overdraw_is_capped() {
    // 10,000 full-size bubbles at scale 300: what's drawn covers at most BUBBLE_COVER plot areas.
    let n = 10_000;
    let xs: Vec<String> = (0..n).map(|i| (i % 100).to_string()).collect();
    let ys: Vec<String> = (0..n).map(|i| (i / 100).to_string()).collect();
    let (xs, ys): (Vec<&str>, Vec<&str>) = (xs.iter().map(String::as_str).collect(), ys.iter().map(String::as_str).collect());
    let s = bser(0, "FF0000", &xs, &ys, Some(&vec!["1"; n]));
    let items = auto_bubble_chart(&[s], r#"<c:bubbleScale val="300"/>"#, 432.0, 252.0);
    let p = plot_box(&items);
    let drawn = assert_bubbles_inside(&items, 432.0, 252.0, "dense");
    let d = word_d_max(432.0, 252.0, 300.0);
    let cover = drawn as f32 * std::f32::consts::FRAC_PI_4 * d * d / ((p[2] - p[0]) * (p[3] - p[1]));
    assert!(drawn > 0 && drawn < n, "{drawn} of {n} drawn");
    assert!(cover <= BUBBLE_COVER + 1.0, "covers {cover} plot areas");
}

// An automatic series colour follows the series' `c:idx`, not its place in the file (#5963), as
// measured in Word: `idx` 3 then 0 are accent4 then accent1, a lone `idx` 2 is accent3, and
// `c:order` changes nothing.

/// The fill colours of the bars of a column chart of `sers`, and of its legend swatches.
fn column_colours(sers: &[String]) -> (Vec<Rgb>, Vec<Rgb>) {
    let plot = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx>"#,
        sers.concat()
    );
    let items = with_legend(&plot, "r", 300.0, 200.0);
    let bars = filled(&items).iter().map(|b| b.0).collect();
    let keys = items
        .iter()
        .filter_map(|it| if let GraphicItem::Shape { rect, fill, .. } = it { (rect[2] == SWATCH).then_some(*fill).flatten() } else { None })
        .collect();
    (bars, keys)
}

/// `ser` with its `c:idx` / `c:order` replaced by `idx_order` (an empty string drops them).
fn renumbered(ser: &str, idx_order: &str) -> String {
    let start = ser.find("<c:idx").expect("ser writes c:idx");
    let end = ser.find("<c:tx>").expect("ser writes c:tx");
    format!("{}{idx_order}{}", &ser[..start], &ser[end..])
}

#[test]
fn automatic_series_colours_follow_c_idx() {
    let accent = |k: usize| theme()[4 + k];
    let a = ser(0, "A", None, &["Q1"], &["1"]);
    let b = ser(1, "B", None, &["Q1"], &["2"]);
    // idx 3 first, idx 0 second: accent4, accent1, in the bars and the legend.
    let sers = [renumbered(&a, r#"<c:idx val="3"/><c:order val="0"/>"#), renumbered(&b, r#"<c:idx val="0"/><c:order val="1"/>"#)];
    assert_eq!(column_colours(&sers), (vec![accent(3), accent(0)], vec![accent(3), accent(0)]));
    // A lone series with idx 2 is accent3.
    let lone = [renumbered(&a, r#"<c:idx val="2"/><c:order val="0"/>"#)];
    assert_eq!(column_colours(&lone).0, [accent(2)]);
    // c:order doesn't matter: idx 0 and 1 stay accent1 and accent2 with their orders swapped.
    let swapped = [renumbered(&a, r#"<c:idx val="0"/><c:order val="1"/>"#), renumbered(&b, r#"<c:idx val="1"/><c:order val="0"/>"#)];
    assert_eq!(column_colours(&swapped).0, [accent(0), accent(1)]);
    // A series' own colour still wins.
    let own = [renumbered(&ser(0, "A", Some("FF0000"), &["Q1"], &["1"]), r#"<c:idx val="3"/><c:order val="0"/>"#)];
    assert_eq!(column_colours(&own).0, [Rgb(0xFF, 0, 0)]);
}

#[test]
fn series_without_a_usable_c_idx_take_their_place() {
    let accent = |k: usize| theme()[4 + k];
    let a = ser(0, "A", None, &["Q1"], &["1"]);
    let b = ser(1, "B", None, &["Q1"], &["2"]);
    // No c:idx at all: the place in the file.
    assert_eq!(column_colours(&[renumbered(&a, ""), renumbered(&b, "")]).0, [accent(0), accent(1)]);
    // Junk, negative and out-of-range values: no panic, the place in the file.
    for junk in ["x", "-1", "1e3", "99999999999999999999999"] {
        let sers = [renumbered(&a, &format!(r#"<c:idx val="{junk}"/>"#)), renumbered(&b, &format!(r#"<c:idx val="{junk}"/>"#))];
        assert_eq!(column_colours(&sers).0, [accent(0), accent(1)], "{junk:?}");
    }
    // The largest unsignedInt `c:idx` allows draws, without a panic.
    let big = [renumbered(&a, &format!(r#"<c:idx val="{}"/>"#, u32::MAX))];
    assert_eq!(column_colours(&big).0.len(), 1);
}
