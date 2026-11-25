//! A chart made in Word (a chart part with its own look and an embedded workbook) opens drawn,
//! and Chart Design edits it in place: its data, title, legend and data labels change, its look
//! and its workbook come along through a save, and Undo takes an edit back. The DOCX is original
//! and synthetic, shaped like Word's.

use serde_json::json;
use wordcraft_doc::graphic::GraphicItem;
use wordcraft_doc::para::InlineObject;
use wordcraft_engine::{Session, io};

const CT_CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = craft_zip::Writer::new();
    for (n, b) in entries {
        w.add(n, b, craft_zip::Compression::Deflated).unwrap();
    }
    w.finish().unwrap()
}

fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut a = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 1000, max_total_bytes: 1 << 26 }).unwrap();
    (0..a.len())
        .filter_map(|i| {
            let name = a.entry_name(i).unwrap()?;
            let data = a.read_index(i, 1 << 24).unwrap()?;
            Some((name, data))
        })
        .collect()
}

fn workbook() -> Vec<u8> {
    zip(&[
        ("_rels/.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#),
        ("xl/workbook.xml", br#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#),
        ("xl/_rels/workbook.xml.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#),
        ("xl/worksheets/sheet1.xml", br#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:B3"/><sheetData><row r="1"><c r="B1" t="inlineStr"><is><t>Units</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Mon</t></is></c><c r="B2"><v>3</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>Tue</t></is></c><c r="B3"><v>5</v></c></row></sheetData></worksheet>"#),
    ])
}

/// A line chart as Word makes one, with a dashed purple series (beyond WordCraft's own chart
/// model, so it is kept as read), a title and a legend.
fn chart() -> String {
    concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
        r#"<c:roundedCorners val="0"/><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1600"/><a:t>Units sold</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title><c:autoTitleDeleted val="0"/>"#,
        r#"<c:plotArea><c:layout/><c:lineChart><c:grouping val="standard"/><c:varyColors val="0"/>"#,
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Units</c:v></c:pt></c:strCache></c:strRef></c:tx>"#,
        r#"<c:spPr><a:ln w="38100"><a:solidFill><a:srgbClr val="7030A0"/></a:solidFill><a:prstDash val="dash"/></a:ln></c:spPr><c:marker><c:symbol val="none"/></c:marker>"#,
        r#"<c:cat><c:strRef><c:f>Sheet1!$A$2:$A$3</c:f><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>Mon</c:v></c:pt><c:pt idx="1"><c:v>Tue</c:v></c:pt></c:strCache></c:strRef></c:cat>"#,
        r#"<c:val><c:numRef><c:f>Sheet1!$B$2:$B$3</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>3</c:v></c:pt><c:pt idx="1"><c:v>5</c:v></c:pt></c:numCache></c:numRef></c:val><c:smooth val="0"/></c:ser>"#,
        r#"<c:marker val="1"/><c:axId val="1"/><c:axId val="2"/></c:lineChart>"#,
        r#"<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="2"/></c:catAx>"#,
        r#"<c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/><c:crossAx val="1"/></c:valAx></c:plotArea>"#,
        r#"<c:legend><c:legendPos val="r"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/></c:chart>"#,
        r#"<c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"#
    )
    .to_string()
}

fn docx() -> Vec<u8> {
    let document = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
        r#"<w:body><w:p><w:r><w:t xml:space="preserve">Sales </w:t></w:r><w:r><w:drawing><wp:inline><wp:extent cx="3810000" cy="2286000"/><wp:docPr id="1" name="Chart 1"/>"#,
        r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rId5"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let types = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
            r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#,
            r#"<Default Extension="xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/>"#,
            r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
            r#"<Override PartName="/word/charts/chart1.xml" ContentType="{ct}"/></Types>"#
        ),
        ct = CT_CHART
    );
    let chart = chart();
    let wb = workbook();
    zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/></Relationships>"#),
        ("word/charts/chart1.xml", chart.as_bytes()),
        ("word/charts/_rels/chart1.xml.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="../embeddings/Microsoft_Excel_Worksheet.xlsx"/></Relationships>"#),
        ("word/embeddings/Microsoft_Excel_Worksheet.xlsx", &wb),
    ])
}

/// The first chart's drawing.
fn drawn(s: &Session) -> Vec<GraphicItem> {
    let mut out = None;
    s.doc.objects_in_reading_order(&mut |o| {
        if out.is_none()
            && let InlineObject::Graphic { graphic, .. } = o
        {
            out = Some(graphic.items.clone());
        }
    });
    out.unwrap()
}

fn select(s: &mut Session) {
    s.run("select.objects", &json!({"index": 0})).unwrap();
}

#[test]
fn a_word_chart_is_edited_in_place_and_saved_with_its_look() {
    let mut s = Session::new(io::open_bytes("sales.docx", &docx()).unwrap());
    select(&mut s);
    let view = wordcraft_engine::cmd::chart::selected_chart(&s).expect("Chart Design shows the chart");
    assert!(wordcraft_engine::cmd::chart::selected_chart_is_kept(&s), "the chart keeps its own part");
    assert_eq!(view.categories, ["Mon", "Tue"]);
    assert_eq!(view.title.as_deref(), Some("Units sold"));
    let before = drawn(&s);

    let out = s
        .run("chart.editData", &json!({"data": {"categories": ["Mon", "Tue", "Wed"], "series": [{"name": "Units", "values": [3, 5, 9]}, {"name": "Returns", "values": [1, 0, 2]}]}}))
        .unwrap();
    assert_eq!(out["workbook"], "updated");
    s.run("chart.title", &json!({"text": "Units this week"})).unwrap();
    s.run("chart.legend", &json!({"pos": "bottom"})).unwrap();
    s.run("chart.dataLabels", &json!({"on": true})).unwrap();
    assert_ne!(drawn(&s), before, "the chart is drawn again");
    assert!(s.run("chart.type", &json!({"type": "column"})).is_err(), "its type needs WordCraft's model");

    let saved = io::save_bytes("sales.docx", &s.doc).unwrap();
    let parts = entries(&saved);
    let chart_xml = parts
        .iter()
        .find(|(n, _)| n.starts_with("word/charts/") && !n.contains("_rels"))
        .map(|(_, b)| String::from_utf8_lossy(b).to_string())
        .unwrap();
    for kept in ["7030A0", r#"<a:prstDash val="dash"/>"#, r#"sz="1600""#] {
        assert!(chart_xml.contains(kept), "{kept} is kept through the save");
    }
    let (_, wb) = parts.iter().find(|(n, _)| n.ends_with(".xlsx")).expect("the workbook is saved");
    let sheet = entries(wb).into_iter().find(|(n, _)| n.ends_with("sheet1.xml")).map(|(_, b)| String::from_utf8(b).unwrap()).unwrap();
    assert!(sheet.contains("Returns") && sheet.contains("Wed"), "the workbook holds the new data");

    let mut again = Session::new(io::open_bytes("sales.docx", &saved).unwrap());
    select(&mut again);
    let view = wordcraft_engine::cmd::chart::selected_chart(&again).unwrap();
    assert_eq!(view.categories, ["Mon", "Tue", "Wed"]);
    assert_eq!(view.series.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Units", "Returns"]);
    assert_eq!(view.series[1].values, [Some(1.0), Some(0.0), Some(2.0)]);
    assert_eq!(view.title.as_deref(), Some("Units this week"));
    assert_eq!(view.legend, Some(wordcraft_doc::chart::LegendPos::Bottom));
    assert!(view.data_labels);
}

#[test]
fn undo_takes_a_word_chart_edit_back() {
    let mut s = Session::new(io::open_bytes("sales.docx", &docx()).unwrap());
    select(&mut s);
    let before = drawn(&s);
    s.run("chart.editData", &json!({"data": {"series": [{"name": "Units", "values": [30, 1]}]}})).unwrap();
    assert_ne!(drawn(&s), before);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(drawn(&s), before);
    select(&mut s);
    assert_eq!(wordcraft_engine::cmd::chart::selected_chart(&s).unwrap().series[0].values, [Some(3.0), Some(5.0)]);
}
