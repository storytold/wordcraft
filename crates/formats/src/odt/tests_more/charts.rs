//! Charts (ODF 1.3 §10.4.3 embedded objects holding an `office:chart` sub-document, Part 3
//! §11): read into WordCraft's editable chart (type, categories, series, title, legend, data
//! labels) from the chart's own data table, so they're drawn and edited like charts WordCraft
//! makes. Charts WordCraft can't edit keep their replacement picture.

use std::sync::Arc;

use crate::model::Inline;
use crate::odt::parse;
use crate::odt::tests::{NS_T, paras, zip_of};
use wordcraft_doc::chart::{ChartSeries, ChartSpec, ChartType, LegendPos};

const NS_C: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;

/// LibreOffice's data table for categories Q1–Q4 and series North and South (South has a gap
/// in Q2, written `NaN`), with its labels in the first row and column.
const TABLE: &str = r#"<table:table table:name="local-table"><table:table-header-columns><table:table-column/></table:table-header-columns><table:table-columns><table:table-column table:number-columns-repeated="2"/></table:table-columns><table:table-header-rows><table:table-row><table:table-cell><text:p/></table:table-cell><table:table-cell office:value-type="string"><text:p>North</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>South</text:p></table:table-cell></table:table-row></table:table-header-rows><table:table-rows><table:table-row><table:table-cell office:value-type="string"><text:p>Q1</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="10"><text:p>10</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="12"><text:p>12</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell office:value-type="string"><text:p>Q2</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="14"><text:p>14</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="NaN"><text:p>NaN</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell office:value-type="string"><text:p>Q3</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="8"><text:p>8</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="11"><text:p>11</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell office:value-type="string"><text:p>Q4</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="16"><text:p>16</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="13"><text:p>13</text:p></table:table-cell></table:table-row></table:table-rows></table:table>"#;

/// The two series over [`TABLE`], each `class`, with style `ser` (and its data points).
fn series(class: &str) -> String {
    format!(
        r#"<chart:series chart:style-name="ser" chart:values-cell-range-address="local-table.$B$2:.$B$5" chart:label-cell-address="local-table.$B$1" chart:class="{class}"><chart:data-point chart:repeated="4"/></chart:series><chart:series chart:style-name="ser" chart:values-cell-range-address="local-table.$C$2:.$C$5" chart:label-cell-address="local-table.$C$1" chart:class="{class}"><chart:data-point chart:repeated="4"/></chart:series>"#
    )
}

/// A chart sub-document's `content.xml` as LibreOffice writes it: chart class `class`, plot
/// area chart properties `plot`, series style chart properties `ser`, and `inner` (title,
/// legend) before the plot area, holding `series`.
fn chart_xml(class: &str, plot: &str, ser: &str, inner: &str, series: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS_C}><office:automatic-styles><style:style style:name="plot" style:family="chart"><style:chart-properties {plot}/></style:style><style:style style:name="ser" style:family="chart"><style:chart-properties {ser}/></style:style></office:automatic-styles><office:body><office:chart><chart:chart svg:width="15cm" svg:height="7.5cm" chart:class="{class}">{inner}<chart:plot-area chart:style-name="plot" chart:data-source-has-labels="both"><chart:axis chart:dimension="x" chart:name="primary-x"><chart:categories table:cell-range-address="local-table.$A$2:.$A$5"/></chart:axis><chart:axis chart:dimension="y" chart:name="primary-y"/>{series}<chart:wall/><chart:floor/></chart:plot-area>{TABLE}</chart:chart></office:chart></office:body></office:document-content>"#
    )
}

/// What the document holding chart object `chart` (with a replacement picture) reads as: the
/// paragraph's charts, and whether a picture was read instead.
fn read(chart: &str) -> (Vec<crate::model::Chart>, bool) {
    let frame = r#"<draw:frame draw:name="Object1" text:anchor-type="as-char" svg:width="5.9055in" svg:height="2.9528in"><draw:object xlink:href="./Object 1" xlink:type="simple"/><draw:image xlink:href="./ObjectReplacements/Object 1"/></draw:frame>"#;
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:body><office:text><text:p>Before {frame}</text:p></office:text></office:body></office:document-content>"#
    );
    let img = image::RgbaImage::from_fn(4, 2, |_, _| image::Rgba([0, 80, 160, 255]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let flow =
        parse(&zip_of(&[("content.xml", content.as_bytes()), ("Object 1/content.xml", chart.as_bytes()), ("ObjectReplacements/Object 1", &png)]))
            .unwrap();
    let inlines: Vec<Inline> = paras(&flow).into_iter().flat_map(|p| p.inlines.clone()).collect();
    let charts = inlines.iter().filter_map(|i| if let Inline::Chart(c) = i { Some(c.clone()) } else { None }).collect();
    (charts, inlines.iter().any(|i| matches!(i, Inline::Image(_))))
}

fn spec(chart: &str) -> Option<ChartSpec> {
    let (charts, _) = read(chart);
    charts.first().map(|c| (*c.spec).clone())
}

fn north_south() -> Vec<ChartSeries> {
    vec![
        ChartSeries { name: "North".into(), values: vec![Some(10.0), Some(14.0), Some(8.0), Some(16.0)] },
        ChartSeries { name: "South".into(), values: vec![Some(12.0), None, Some(11.0), Some(13.0)] },
    ]
}

#[test]
fn a_column_chart_reads_its_data_title_and_legend() {
    let inner = r#"<chart:title><text:p>Sales by quarter</text:p></chart:title><chart:legend chart:legend-position="end"/>"#;
    let xml = chart_xml("chart:bar", r#"chart:vertical="false""#, r#"chart:data-label-number="none""#, inner, &series("chart:bar"));
    let (charts, picture) = read(&xml);
    assert!(!picture, "the chart replaces its preview picture");
    let c = charts.first().expect("a chart");
    assert_eq!(
        *c.spec,
        ChartSpec {
            kind: ChartType::Column,
            categories: vec!["Q1".into(), "Q2".into(), "Q3".into(), "Q4".into()],
            series: north_south(),
            title: Some("Sales by quarter".into()),
            legend: Some(LegendPos::Right),
            data_labels: false,
        }
    );
    // The frame's size, in points.
    assert_eq!(((c.w * 10.0).round(), (c.h * 10.0).round()), (4252.0, 2126.0));
}

#[test]
fn bar_stacking_legend_and_labels_come_from_the_styles() {
    let cases = [
        (r#"chart:vertical="true""#, ChartType::Bar),
        (r#"chart:vertical="true" chart:stacked="true""#, ChartType::BarStacked),
        (r#"chart:stacked="true""#, ChartType::ColumnStacked),
    ];
    for (plot, kind) in cases {
        let xml = chart_xml(
            "chart:bar",
            plot,
            r#"chart:data-label-number="value""#,
            r#"<chart:legend chart:legend-position="bottom"/>"#,
            &series("chart:bar"),
        );
        let s = spec(&xml).expect(plot);
        assert_eq!((s.kind, s.legend, s.data_labels, s.title.clone()), (kind, Some(LegendPos::Bottom), true, None), "{plot}");
        assert_eq!(s.series, north_south());
    }
    // No legend element: no legend. Legend corners and sides.
    for (pos, want) in [
        ("", None),
        ("start", Some(LegendPos::Left)),
        ("top", Some(LegendPos::Top)),
        ("top-end", Some(LegendPos::Top)),
        ("bottom-start", Some(LegendPos::Bottom)),
    ] {
        let legend = if pos.is_empty() { String::new() } else { format!(r#"<chart:legend chart:legend-position="{pos}"/>"#) };
        let s = spec(&chart_xml("chart:bar", "", "", &legend, &series("chart:bar"))).unwrap();
        assert_eq!(s.legend, want, "{pos}");
    }
}

#[test]
fn line_area_pie_doughnut_and_scatter_classes() {
    for (class, ser_class, kind) in [
        ("chart:line", "chart:line", ChartType::Line),
        ("chart:area", "chart:area", ChartType::Area),
        ("chart:circle", "chart:circle", ChartType::Pie),
        // LibreOffice writes a doughnut's series as `chart:circle` in a `chart:ring` chart.
        ("chart:ring", "chart:circle", ChartType::Doughnut),
    ] {
        let s = spec(&chart_xml(class, "", "", "", &series(ser_class))).expect(class);
        assert_eq!(s.kind, kind, "{class}");
        assert_eq!(s.series, north_south());
    }
    // A scatter chart's x values are its first series' domain: they're its categories.
    let scatter = r#"<chart:series chart:values-cell-range-address="local-table.$C$2:.$C$5" chart:label-cell-address="local-table.$C$1" chart:class="chart:scatter"><chart:domain table:cell-range-address="local-table.$B$2:.$B$5"/></chart:series>"#;
    let s = spec(&chart_xml("chart:scatter", "", "", "", scatter)).unwrap();
    assert_eq!(s.kind, ChartType::Scatter);
    assert_eq!(s.categories, vec!["10", "14", "8", "16"]);
    assert_eq!(s.series, vec![ChartSeries { name: "South".into(), values: vec![Some(12.0), None, Some(11.0), Some(13.0)] }]);
}

#[test]
fn charts_wordcraft_cannot_edit_keep_their_picture() {
    // Percent-stacked, a radar chart, a chart mixing columns and a line, 3D: the picture.
    let mixed = format!(
        "{}{}",
        series("chart:bar").split("</chart:series>").next().unwrap_or_default().to_string() + "</chart:series>",
        r#"<chart:series chart:values-cell-range-address="local-table.$C$2:.$C$5" chart:class="chart:line"/>"#
    );
    for xml in [
        chart_xml("chart:bar", r#"chart:stacked="true" chart:percentage="true""#, "", "", &series("chart:bar")),
        chart_xml("chart:radar", "", "", "", &series("chart:radar")),
        chart_xml("chart:bar", "", "", "", &mixed),
        chart_xml("chart:bar", r#"chart:three-dimensional="true""#, "", "", &series("chart:bar")),
        // Not a chart at all, or broken.
        "<office:document-content><office:body><office:text/></office:body></office:document-content>".to_string(),
        "<office:document-content><office:body><office:chart><chart:chart chart:class=\"chart:bar\">".to_string(),
    ] {
        let (charts, picture) = read(&xml);
        assert!(charts.is_empty() && picture, "{xml}");
    }
}

#[test]
fn hostile_chart_tables_are_bounded() {
    // Huge repeat counts, references far outside the table and into reversed or broken ranges:
    // never a panic or a huge allocation, and the series stay within the chart's limits.
    let table = r#"<table:table><table:table-rows><table:table-row table:number-rows-repeated="2000000000"><table:table-cell table:number-columns-repeated="2000000000" office:value-type="float" office:value="1e308"/></table:table-row></table:table-rows></table:table>"#;
    let series = r#"<chart:series chart:values-cell-range-address="t.$A$1:.$A$99999999"/><chart:series chart:values-cell-range-address="t.$ZZZZZZZZ$1:.$A$1"/><chart:series chart:values-cell-range-address="t.$B$9:.$B$1"/><chart:series chart:values-cell-range-address=":::"/><chart:series chart:label-cell-address="t.$A$0"/>"#;
    let xml = format!(
        r#"<office:document-content><office:body><office:chart><chart:chart chart:class="chart:line"><chart:plot-area><chart:axis chart:dimension="x"><chart:categories table:cell-range-address="t.A1:XFD1048576"/></chart:axis>{series}</chart:plot-area>{table}</chart:chart></office:chart></office:body></office:document-content>"#
    );
    let s = spec(&xml).expect("a chart");
    assert!(s.series.len() <= wordcraft_doc::chart::MAX_SERIES);
    assert!(s.series.iter().all(|x| x.values.len() <= wordcraft_doc::chart::MAX_POINTS));
    assert!(s.categories.len() <= wordcraft_doc::chart::MAX_POINTS);
}

#[test]
fn an_imported_chart_is_an_editable_chart_object() {
    let xml = chart_xml("chart:line", "", "", "", &series("chart:line"));
    let frame = r#"<draw:frame svg:width="4in" svg:height="2in"><draw:object xlink:href="./Object 1"/></draw:frame>"#;
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:body><office:text><text:p>{frame}</text:p></office:text></office:body></office:document-content>"#
    );
    let doc = crate::odt::import(&zip_of(&[("content.xml", content.as_bytes()), ("Object 1/content.xml", xml.as_bytes())])).unwrap();
    let graphics: Vec<Arc<wordcraft_doc::graphic::Graphic>> = doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects_with_offsets())
        .filter_map(
            |(_, o)| if let wordcraft_doc::para::InlineObject::Graphic { graphic, w, h, .. } = o { Some((graphic.clone(), *w, *h)) } else { None },
        )
        .map(|(g, w, h)| {
            assert_eq!((w, h), (288.0, 144.0));
            g
        })
        .collect();
    let g = graphics.first().expect("a chart object");
    assert_eq!(g.kind, wordcraft_doc::graphic::GraphicKind::Chart);
    assert_eq!(g.chart.as_ref().map(|c| c.kind), Some(ChartType::Line));
}

/// A document holding `spec` as a chart `w` × `h` points, after some text.
fn doc_with(spec: ChartSpec, float: bool) -> wordcraft_doc::Document {
    let flow = crate::model::Flow {
        blocks: vec![crate::model::FBlock::Para(crate::model::Para {
            inlines: vec![
                Inline::Text("See ".into(), Default::default()),
                Inline::Chart(crate::model::Chart {
                    spec: Arc::new(spec),
                    w: 300.0,
                    h: 180.0,
                    alt: "Sales chart".into(),
                    float: float.then(|| {
                        Box::new(wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::Square, x: 20.0, y: 10.0, ..Default::default() })
                    }),
                    source: None,
                }),
            ],
            ..Default::default()
        })],
        ..Default::default()
    };
    crate::model::to_doc(&flow)
}

/// The charts the saved document reads back as.
fn reread(bytes: &[u8]) -> Vec<crate::model::Chart> {
    paras(&parse(bytes).unwrap())
        .into_iter()
        .flat_map(|p| p.inlines.clone())
        .filter_map(|i| if let Inline::Chart(c) = i { Some(c) } else { None })
        .collect()
}

#[test]
fn every_chart_type_is_saved_as_a_chart_object_and_reads_back() {
    for kind in ChartType::ALL {
        let mut spec = ChartSpec::sample(kind);
        spec.title = Some("Q & A <totals>".into());
        spec.data_labels = kind == ChartType::Bar;
        spec.legend = Some(LegendPos::Left);
        if let Some(s) = spec.series.first_mut()
            && let Some(v) = s.values.get_mut(1)
        {
            *v = None;
        }
        let bytes = crate::odt::export(&doc_with(spec.clone(), false)).unwrap();
        let back = reread(&bytes);
        assert_eq!(back.len(), 1, "{kind:?}");
        assert_eq!(*back[0].spec, spec.clone().sanitized(), "{kind:?}");
        assert_eq!((back[0].w, back[0].h, back[0].alt.as_str()), (300.0, 180.0, "Sales chart"), "{kind:?}");
        // The package lists the chart object.
        let z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert!(z.file_names().any(|n| n == "Object 1/content.xml"), "{kind:?}");
        let manifest = crate::odt::tests::entry(&bytes, "META-INF/manifest.xml");
        assert!(
            manifest
                .contains(r#"manifest:full-path="Object 1/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.chart""#)
        );
    }
}

#[test]
fn a_floating_chart_keeps_its_place_and_wrap() {
    let bytes = crate::odt::export(&doc_with(ChartSpec::sample(ChartType::Column), true)).unwrap();
    let back = reread(&bytes);
    let f = back.first().and_then(|c| c.float.as_deref()).expect("a floating chart");
    assert_eq!((f.wrap, f.x.round(), f.y.round()), (wordcraft_doc::para::Wrap::Square, 20.0, 10.0));
}

#[test]
fn a_flat_document_holds_its_chart_inline() {
    let flat = crate::odt::export_flat(&doc_with(ChartSpec::sample(ChartType::Pie), false)).unwrap();
    let text = String::from_utf8(flat.clone()).unwrap();
    assert!(text.contains("<draw:object><office:document"), "{text}");
    let back: Vec<crate::model::Chart> = crate::odt::import_flat(&flat)
        .unwrap()
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.clone())
        .filter_map(|o| match o {
            wordcraft_doc::para::InlineObject::Graphic { graphic, w, h, alt, .. } => {
                graphic.chart.clone().map(|spec| crate::model::Chart { spec, w, h, alt, float: None, source: None })
            }
            _ => None,
        })
        .collect();
    assert_eq!(back.first().map(|c| c.spec.kind), Some(ChartType::Pie));
}

#[test]
fn an_unchanged_chart_is_written_back_as_it_was_and_an_edited_one_anew() {
    // A chart keeps the look WordCraft doesn't model (its fonts, lines and colours) only if its
    // object is written back whole; once edited, it's written from WordCraft's chart.
    let original = chart_xml(
        "chart:bar",
        r#"chart:vertical="false""#,
        "",
        r#"<chart:title><text:p>Sales</text:p></chart:title>"#,
        &series("chart:bar"),
    )
    .replace(
        "</office:automatic-styles>",
        r#"<style:style style:name="look" style:family="chart"><style:text-properties fo:font-size="18pt"/></style:style></office:automatic-styles>"#,
    );
    // With its replacement picture (what a reader that can't draw charts shows).
    let frame = r#"<draw:frame draw:name="Object1" text:anchor-type="as-char" svg:width="5.9055in" svg:height="2.9528in"><draw:object xlink:href="./Object 1" xlink:type="simple"/><draw:image xlink:href="./ObjectReplacements/Object 1"/></draw:frame>"#;
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:body><office:text><text:p>Before {frame}</text:p></office:text></office:body></office:document-content>"#
    );
    let picture: &[u8] = b"VCLMTF the chart as drawn";
    let doc = crate::odt::import(&zip_of(&[
        ("content.xml", content.as_bytes()),
        ("Object 1/content.xml", original.as_bytes()),
        ("ObjectReplacements/Object 1", picture),
    ]))
    .unwrap();
    let replacement_of = |bytes: &[u8]| {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        let name = (0..z.len()).filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string())).find(|n| n.starts_with("ObjectReplacements/"))?;
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut z.by_name(&name).unwrap(), &mut out).unwrap();
        Some(out)
    };
    let saved = crate::odt::export(&doc).unwrap();
    assert_eq!(replacement_of(&saved).as_deref(), Some(picture));
    assert!(crate::odt::tests::entry(&saved, "content.xml").contains(r#"<draw:image xlink:href="./ObjectReplacements/"#));
    let chart_of = |bytes: &[u8]| {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        let name = (0..z.len())
            .filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string()))
            .find(|n| n.ends_with("/content.xml") && n.starts_with("Object"))
            .unwrap();
        let mut out = String::new();
        std::io::Read::read_to_string(&mut z.by_name(&name).unwrap(), &mut out).unwrap();
        out
    };
    assert_eq!(chart_of(&crate::odt::export(&doc).unwrap()), original);
    // Edited: a new title.
    let mut edited = doc.clone();
    for b in edited.body.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(b) {
            for o in &mut p.objects {
                if let wordcraft_doc::para::InlineObject::Graphic { graphic, .. } = o {
                    let g = Arc::make_mut(graphic);
                    let mut spec = (**g.chart.as_ref().unwrap()).clone();
                    spec.title = Some("Revenue".into());
                    g.chart = Some(Arc::new(spec));
                }
            }
        }
    }
    let saved = crate::odt::export(&edited).unwrap();
    let out = chart_of(&saved);
    assert!(out.contains("Revenue") && !out.contains(r#"fo:font-size="18pt""#), "{out}");
    // The old picture would show the old chart.
    assert_eq!(replacement_of(&saved), None);
}
