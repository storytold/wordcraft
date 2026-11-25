use std::sync::Arc;

use craft_xml::Local;
use wordcraft_doc::Document;
use wordcraft_doc::chart::LegendPos;
use wordcraft_doc::graphic::{Embedded, EmbeddedRef};
use wordcraft_doc::props::Rgb;

use super::{
    ChartData, MANIFEST, PartEdit, SeriesData, column, draw_part, edit_kept, edit_part, edit_workbook, read_kept, read_part, sheet_name, sheet_ref,
};

const C: &str = r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

/// A series as Word writes one: name, categories and values by reference to Sheet1, with
/// cached values.
fn ser(idx: usize, name: &str, values: &[f64], colour: &str) -> String {
    let col = column(idx + 1);
    let pts: String = values.iter().enumerate().map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#)).collect();
    let cats: String = ["North", "South", "East"].iter().enumerate().map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#)).collect();
    format!(
        concat!(
            r#"<c:ser><c:idx val="{idx}"/><c:order val="{idx}"/>"#,
            r#"<c:tx><c:strRef><c:f>Sheet1!${col}$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{name}</c:v></c:pt></c:strCache></c:strRef></c:tx>"#,
            r#"<c:spPr><a:solidFill><a:srgbClr val="{colour}"/></a:solidFill></c:spPr><c:invertIfNegative val="0"/>"#,
            r#"<c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/>{cats}</c:strCache></c:strRef></c:cat>"#,
            r#"<c:val><c:numRef><c:f>Sheet1!${col}$2:${col}$4</c:f><c:numCache><c:formatCode>#,##0.0</c:formatCode><c:ptCount val="3"/>{pts}</c:numCache></c:numRef></c:val>"#,
            r#"<c:extLst><c:ext uri="{{C3380CC4-5D6E-409C-BE32-E72D297353CC}}"><c16:uniqueId xmlns:c16="http://schemas.microsoft.com/office/drawing/2014/chart" val="{{0000000{idx}-0000-0000-0000-000000000000}}"/></c:ext></c:extLst>"#,
            r#"</c:ser>"#
        ),
        idx = idx,
        col = col,
        name = name,
        colour = colour,
        cats = cats,
        pts = pts
    )
}

/// A clustered column chart as Word makes one: a styled title, two series, a legend at the
/// bottom, an embedded workbook.
fn word_chart() -> Vec<u8> {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<c:chartSpace {ns}><c:date1904 val="0"/><c:roundedCorners val="0"/><c:chart>"#,
            r#"<c:title><c:tx><c:rich><a:bodyPr rot="0"/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1400" b="1"/></a:pPr><a:r><a:rPr lang="en-US" sz="1400" b="1"><a:solidFill><a:srgbClr val="7030A0"/></a:solidFill></a:rPr><a:t>Sales</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title>"#,
            r#"<c:autoTitleDeleted val="0"/><c:plotArea><c:layout/>"#,
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>{s0}{s1}<c:gapWidth val="219"/><c:overlap val="-27"/><c:axId val="11"/><c:axId val="12"/></c:barChart>"#,
            r#"<c:catAx><c:axId val="11"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="12"/></c:catAx>"#,
            r##"<c:valAx><c:axId val="12"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/><c:numFmt formatCode="#,##0.0" sourceLinked="1"/><c:crossAx val="11"/></c:valAx>"##,
            r#"</c:plotArea><c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart>"#,
            r#"<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr/></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr>"#,
            r#"<c:externalData r:id="rId3"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"#
        ),
        ns = C,
        s0 = ser(0, "2025", &[1.5, 2.0, 3.25], "4472C4"),
        s1 = ser(1, "2026", &[4.0, 5.0, 6.0], "ED7D31"),
    )
    .into_bytes()
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = craft_zip::Writer::new();
    for (n, b) in entries {
        w.add(n, b, craft_zip::Compression::Deflated).unwrap();
    }
    w.finish().unwrap()
}

fn unzip(bytes: &[u8], name: &str) -> String {
    let mut a = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 100, max_total_bytes: 1 << 24 }).unwrap();
    String::from_utf8(a.read(name, 1 << 24).unwrap().unwrap()).unwrap()
}

/// The workbook Word embeds for `word_chart`: shared strings, a styled sheet and a table over
/// A1:C4.
fn workbook() -> Vec<u8> {
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let wb = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let wb_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;
    let ss = r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="5" uniqueCount="5"><si><t>2025</t></si><si><t>2026</t></si><si><t>North</t></si><si><t>South</t></si><si><t>East</t></si></sst>"#;
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><dimension ref="A1:C4"/><sheetData>"#,
        r#"<row r="1"><c r="B1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row>"#,
        r#"<row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2" s="1"><v>1.5</v></c><c r="C2" s="1"><v>4</v></c></row>"#,
        r#"<row r="3"><c r="A3" t="s"><v>3</v></c><c r="B3" s="1"><v>2</v></c><c r="C3" s="1"><v>5</v></c></row>"#,
        r#"<row r="4"><c r="A4" t="s"><v>4</v></c><c r="B4" s="1"><v>3.25</v></c><c r="C4" s="1"><v>6</v></c></row>"#,
        r#"<row r="9"><c r="E9"><v>42</v></c></row>"#,
        r#"</sheetData><tableParts count="1"><tablePart r:id="rId1"/></tableParts></worksheet>"#
    );
    let sheet_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/></Relationships>"#;
    let table = r#"<?xml version="1.0"?><table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Table1" displayName="Table1" ref="A1:C4"><autoFilter ref="A1:C4"/><tableColumns count="3"><tableColumn id="1" name=" "/><tableColumn id="2" name="2025"/><tableColumn id="3" name="2026"/></tableColumns></table>"#;
    zip(&[
        ("_rels/.rels", rels.as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
        ("xl/sharedStrings.xml", ss.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/tables/table1.xml", table.as_bytes()),
    ])
}

fn data(cats: &[&str], series: &[(&str, &[f64])]) -> ChartData {
    ChartData {
        categories: cats.iter().map(|c| c.to_string()).collect(),
        series: series.iter().map(|(n, v)| SeriesData { name: n.to_string(), values: v.iter().map(|x| Some(*x)).collect() }).collect(),
    }
}

fn new_data(d: &ChartData) -> PartEdit {
    PartEdit::Data { categories: Some(d.categories.clone()), series: Some(d.series.clone()) }
}

#[test]
fn a_word_chart_reads_as_its_data_title_and_legend() {
    let c = read_part(&word_chart()).unwrap();
    assert_eq!(c.data, data(&["North", "South", "East"], &[("2025", &[1.5, 2.0, 3.25]), ("2026", &[4.0, 5.0, 6.0])]));
    assert_eq!(c.title.as_deref(), Some("Sales"));
    assert_eq!(c.legend, Some(LegendPos::Bottom));
    assert_eq!(c.sheet.as_deref(), Some("Sheet1"));
    assert_eq!(c.workbook.as_deref(), Some("rId3"));
    assert!(!c.data_labels);
}

#[test]
fn new_data_replaces_the_caches_and_keeps_the_look() {
    let want = data(&["Q1", "Q2", "Q3", "Q4"], &[("Plan", &[1.0, 2.0, 3.0, 4.0]), ("Actual", &[2.0, 3.0, 4.0, 5.5])]);
    let out = edit_part(&word_chart(), &new_data(&want)).unwrap();
    assert_eq!(read_part(&out).unwrap().data, want);
    let xml = String::from_utf8(out).unwrap();
    for kept in ["4472C4", "ED7D31", "7030A0", "#,##0.0", r#"<c:gapWidth val="219"/>"#, "rId3", "C3380CC4"] {
        assert!(xml.contains(kept), "{kept} is kept");
    }
    assert!(xml.contains("Sheet1!$A$2:$A$5") && xml.contains("Sheet1!$C$2:$C$5"), "references cover the new rows");
}

#[test]
fn added_series_take_the_next_accent_and_removed_ones_go() {
    let three = data(&["A", "B"], &[("x", &[1.0, 2.0]), ("y", &[3.0, 4.0]), ("z", &[5.0, 6.0])]);
    let out = edit_part(&word_chart(), &new_data(&three)).unwrap();
    let root = craft_xml::parse(&out).unwrap();
    let sers: Vec<_> = root.find_all(Local("ser")).collect();
    assert_eq!(sers.len(), 3);
    let idx: Vec<_> = sers.iter().map(|s| s.child(Local("idx")).unwrap().attr("val").unwrap().to_string()).collect();
    assert!(idx.iter().all(|i| idx.iter().filter(|j| *j == i).count() == 1), "unique c:idx: {idx:?}");
    assert!(sers[2].child(Local("spPr")).is_none() && sers[2].child(Local("extLst")).is_none(), "the copy has no colour or id of its own");
    assert_eq!(read_part(&out).unwrap().data, three);

    let one = data(&["A"], &[("only", &[9.0])]);
    let out = edit_part(&word_chart(), &new_data(&one)).unwrap();
    assert_eq!(read_part(&out).unwrap().data, one);
    assert!(String::from_utf8(out).unwrap().contains("4472C4"), "the first series keeps its colour");

    assert!(edit_part(&word_chart(), &new_data(&data(&["A"], &[]))).is_err(), "a chart keeps one series");
}

#[test]
fn removing_a_combo_charts_only_line_series_drops_its_group_and_axes() {
    let xml = String::from_utf8(word_chart()).unwrap().replace(
        r#"<c:catAx>"#,
        &format!(
            r#"<c:lineChart><c:grouping val="standard"/>{}<c:axId val="21"/><c:axId val="22"/></c:lineChart><c:catAx>"#,
            ser(2, "Trend", &[1.0, 1.0, 1.0], "000000").replace("<c:invertIfNegative val=\"0\"/>", "")
        ),
    );
    let xml = xml.replace("</c:valAx>", r#"</c:valAx><c:valAx><c:axId val="22"/><c:crossAx val="21"/></c:valAx><c:catAx><c:axId val="21"/><c:delete val="1"/><c:crossAx val="22"/></c:catAx>"#);
    assert_eq!(read_part(xml.as_bytes()).unwrap().data.series.len(), 3);
    let out = String::from_utf8(edit_part(xml.as_bytes(), &new_data(&data(&["a"], &[("x", &[1.0]), ("y", &[2.0])]))).unwrap()).unwrap();
    assert!(!out.contains("lineChart"), "the empty group is gone");
    assert!(!out.contains(r#"<c:axId val="22"/>"#) && !out.contains(r#"<c:axId val="21"/>"#), "with its axes");
    assert!(out.contains(r#"<c:axId val="12"/>"#), "the bar chart's axes stay");
}

#[test]
fn title_legend_and_labels_change_in_place() {
    let out = edit_part(&word_chart(), &PartEdit::Title(Some("Revenue".into()))).unwrap();
    let c = read_part(&out).unwrap();
    assert_eq!(c.title.as_deref(), Some("Revenue"));
    assert!(String::from_utf8_lossy(&out).contains("7030A0"), "the title keeps its colour");

    let out = edit_part(&word_chart(), &PartEdit::Title(None)).unwrap();
    assert_eq!(read_part(&out).unwrap().title, None);
    let back = edit_part(&out, &PartEdit::Title(Some("Back".into()))).unwrap();
    assert_eq!(read_part(&back).unwrap().title.as_deref(), Some("Back"));

    let out = edit_part(&word_chart(), &PartEdit::Legend(Some(LegendPos::Right))).unwrap();
    assert_eq!(read_part(&out).unwrap().legend, Some(LegendPos::Right));
    let out = edit_part(&out, &PartEdit::Legend(None)).unwrap();
    assert_eq!(read_part(&out).unwrap().legend, None);
    let out = edit_part(&out, &PartEdit::Legend(Some(LegendPos::Top))).unwrap();
    assert_eq!(read_part(&out).unwrap().legend, Some(LegendPos::Top));

    let out = edit_part(&word_chart(), &PartEdit::DataLabels(true)).unwrap();
    assert!(read_part(&out).unwrap().data_labels);
    let out = edit_part(&out, &PartEdit::DataLabels(false)).unwrap();
    assert!(!read_part(&out).unwrap().data_labels);
}

#[test]
fn edits_change_the_drawing() {
    let theme = [Rgb(0, 0, 0), Rgb(255, 255, 255)];
    let before = draw_part(&word_chart(), &theme, 300.0, 200.0);
    assert!(!before.is_empty(), "a Word chart draws");
    let out = edit_part(&word_chart(), &new_data(&data(&["A", "B"], &[("x", &[10.0, 1.0])]))).unwrap();
    let after = draw_part(&out, &theme, 300.0, 200.0);
    assert!(!after.is_empty());
    assert_ne!(before, after);
}

#[test]
fn the_workbook_gets_the_new_cells_and_keeps_the_rest() {
    let want = data(&["Q1", "Q2"], &[("Plan", &[7.0, 8.0])]);
    let out = edit_workbook(&workbook(), Some("Sheet1"), &want, 4, 3).unwrap();
    let sheet = unzip(&out, "xl/worksheets/sheet1.xml");
    let ws = craft_xml::parse(sheet.as_bytes()).unwrap();
    let cell = |r: &str| ws.find_all(Local("c")).find(|c| c.attr("r") == Some(r)).cloned();
    assert_eq!(cell("A2").unwrap().find(Local("t")).unwrap().text(), "Q1");
    assert_eq!(cell("B1").unwrap().find(Local("t")).unwrap().text(), "Plan");
    assert_eq!(cell("B3").unwrap().child(Local("v")).unwrap().text(), "8");
    assert_eq!(cell("B3").unwrap().attr("s"), Some("1"), "a cell keeps its style");
    assert!(cell("A4").is_none() && cell("C2").is_none_or(|c| c.child(Local("v")).is_none()), "old cells outside the data are cleared");
    assert_eq!(cell("E9").unwrap().child(Local("v")).unwrap().text(), "42", "cells away from the data stay");
    let table = unzip(&out, "xl/tables/table1.xml");
    assert!(table.contains(r#"ref="A1:B3""#) && table.contains(r#"count="2""#), "{table}");
    assert!(table.contains(r#"name="Plan""#));
    assert_eq!(unzip(&out, "xl/sharedStrings.xml"), unzip(&workbook(), "xl/sharedStrings.xml"), "other parts are copied");
}

#[test]
fn table_columns_take_unique_names() {
    let want = data(&["a"], &[("Same", &[1.0]), ("Same", &[2.0]), ("", &[3.0])]);
    let table = unzip(&edit_workbook(&workbook(), Some("Sheet1"), &want, 4, 3).unwrap(), "xl/tables/table1.xml");
    for name in [r#"name="Same""#, r#"name="Same2""#, r#"name="Column4""#] {
        assert!(table.contains(name), "{name} in {table}");
    }
}

/// A document holding `word_chart` as a kept object, its workbook beside it.
fn kept_doc() -> (Document, Embedded) {
    let mut doc = Document::default();
    doc.passthrough.insert("word/charts/chart1.xml".into(), Arc::new(word_chart()));
    doc.passthrough.insert("word/embeddings/Microsoft_Excel_Worksheet.xlsx".into(), Arc::new(workbook()));
    let manifest = concat!(
        "P\tword/charts/chart1.xml\tapplication/vnd.openxmlformats-officedocument.drawingml.chart+xml\n",
        "P\tword/embeddings/Microsoft_Excel_Worksheet.xlsx\tapplication/vnd.openxmlformats-officedocument.spreadsheetml.sheet\n",
        "R\tword/charts/chart1.xml\trId3\thttp://schemas.openxmlformats.org/officeDocument/2006/relationships/package\tword/embeddings/Microsoft_Excel_Worksheet.xlsx\tI\n",
    );
    doc.passthrough.insert(MANIFEST.into(), Arc::new(manifest.as_bytes().to_vec()));
    let src = Embedded {
        xml: "<a:graphic/>".into(),
        refs: vec![EmbeddedRef {
            id: "rId9".into(),
            kind: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart".into(),
            target: "word/charts/chart1.xml".into(),
            external: false,
        }],
    };
    (doc, src)
}

#[test]
fn a_kept_chart_is_edited_into_new_parts() {
    let (mut doc, src) = kept_doc();
    let before = doc.passthrough.clone();
    let want = data(&["Q1"], &[("Plan", &[7.0])]);
    let done = edit_kept(&mut doc, &src, &new_data(&want)).unwrap();
    assert_eq!(done.workbook, "updated");
    let new_chart = &done.source.refs[0].target;
    assert_ne!(new_chart, "word/charts/chart1.xml");
    assert_eq!(read_kept(&doc, &done.source).unwrap().data, want);
    for (k, v) in &before {
        if k != MANIFEST {
            assert_eq!(doc.passthrough.get(k), Some(v), "{k} is kept for the chart's other copies and undo");
        }
    }
    let manifest = String::from_utf8(doc.passthrough[MANIFEST].to_vec()).unwrap();
    let wb_line = manifest.lines().find(|l| l.starts_with(&format!("R\t{new_chart}\trId3"))).unwrap();
    let wb = wb_line.split('\t').nth(4).unwrap();
    assert_ne!(wb, "word/embeddings/Microsoft_Excel_Worksheet.xlsx", "the workbook is copied too");
    assert!(unzip(&doc.passthrough[wb], "xl/worksheets/sheet1.xml").contains("Plan"));

    // A second edit replaces the suffix rather than growing it.
    let again = edit_kept(&mut doc, &done.source, &PartEdit::Title(Some("T".into()))).unwrap();
    assert!(again.source.refs[0].target.matches("-e").count() == 1, "{}", again.source.refs[0].target);
    assert_eq!(again.workbook, "none");
}

#[test]
fn a_linked_workbook_is_left_alone() {
    let (mut doc, src) = kept_doc();
    let manifest = "P\tword/charts/chart1.xml\tapplication/vnd.openxmlformats-officedocument.drawingml.chart+xml\nR\tword/charts/chart1.xml\trId3\thttp://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject\tfile:///C:/data.xlsx\tE\n";
    doc.passthrough.insert(MANIFEST.into(), Arc::new(manifest.as_bytes().to_vec()));
    let done = edit_kept(&mut doc, &src, &new_data(&data(&["a"], &[("b", &[1.0])]))).unwrap();
    assert_eq!(done.workbook, "linked");
}

#[test]
fn an_excel_97_workbook_refuses_the_edit_and_changes_nothing() {
    let (mut doc, src) = kept_doc();
    doc.passthrough.insert("word/embeddings/Microsoft_Excel_Worksheet.xlsx".into(), Arc::new(b"\xD0\xCF\x11\xE0 not a zip".to_vec()));
    let manifest = doc.passthrough[MANIFEST].clone();
    let manifest = String::from_utf8_lossy(&manifest).replace("spreadsheetml.sheet", "ms-excel");
    doc.passthrough.insert(MANIFEST.into(), Arc::new(manifest.into_bytes()));
    let before = doc.passthrough.clone();
    assert!(edit_kept(&mut doc, &src, &new_data(&data(&["a"], &[("b", &[1.0])]))).is_err());
    assert_eq!(doc.passthrough, before);
}

#[test]
fn hostile_parts_are_refused_without_panicking() {
    for bytes in [
        &b""[..],
        b"<x/>",
        b"<c:chartSpace xmlns:c='u'/>",
        b"\xFF\xFE garbage",
        b"<c:chartSpace xmlns:c='u'><c:chart><c:plotArea><c:barChart/></c:plotArea></c:chart></c:chartSpace>",
    ] {
        assert!(read_part(bytes).is_none());
        for e in [new_data(&data(&["a"], &[("b", &[1.0])])), PartEdit::Title(Some("t".into())), PartEdit::Legend(None), PartEdit::DataLabels(true)] {
            let _ = edit_part(bytes, &e);
        }
        assert!(draw_part(bytes, &[], 100.0, 100.0).is_empty() || !bytes.is_empty());
    }
    assert!(edit_workbook(b"not a zip", None, &data(&["a"], &[("b", &[1.0])]), 1, 1).is_err());
    let huge = data(&["a"; 10], &[("b", &[f64::NAN, f64::INFINITY, 1e308])]);
    let out = edit_part(&word_chart(), &new_data(&huge)).unwrap();
    assert_eq!(read_part(&out).unwrap().data.series[0].values[..3], [None, None, Some(1e308)]);
}

#[test]
fn sheet_names_and_columns() {
    assert_eq!(column(0), "A");
    assert_eq!(column(25), "Z");
    assert_eq!(column(26), "AA");
    assert_eq!(column(701), "ZZ");
    assert_eq!(sheet_ref("Sheet1"), "Sheet1");
    assert_eq!(sheet_name(&sheet_ref("Bob's data")), "Bob's data");
}
