//! A chart in an ODT file (ODF 1.3 §10.4.3, an embedded `office:chart` object) opens as a chart
//! WordCraft draws and edits: Chart Design's commands change it like a chart made here. The ODT
//! is original and synthetic, shaped like LibreOffice's.

use std::io::Write;

use serde_json::json;
use wordcraft_doc::chart::ChartType;
use wordcraft_doc::para::InlineObject;
use wordcraft_engine::{Session, io};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;

fn odt() -> Vec<u8> {
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text><text:p>Sales <draw:frame svg:width="4in" svg:height="2.5in"><draw:object xlink:href="./Object 1"/></draw:frame></text:p></office:text></office:body></office:document-content>"#
    );
    // A well-formed chart part, its prefixes declared as LibreOffice writes them.
    let chart = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:chart><chart:chart chart:class="chart:line"><chart:title><text:p>Visits</text:p></chart:title><chart:plot-area><chart:axis chart:dimension="x"><chart:categories table:cell-range-address="local-table.$A$2:.$A$4"/></chart:axis><chart:series chart:values-cell-range-address="local-table.$B$2:.$B$4" chart:label-cell-address="local-table.$B$1"/></chart:plot-area><table:table table:name="local-table"><table:table-header-rows><table:table-row><table:table-cell/><table:table-cell office:value-type="string"><text:p>Site</text:p></table:table-cell></table:table-row></table:table-header-rows><table:table-rows><table:table-row><table:table-cell office:value-type="string"><text:p>Mon</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="3"/></table:table-row><table:table-row><table:table-cell office:value-type="string"><text:p>Tue</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="5"/></table:table-row><table:table-row><table:table-cell office:value-type="string"><text:p>Wed</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="4"/></table:table-row></table:table-rows></table:table></chart:chart></office:chart></office:body></office:document-content>"#;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str()), ("Object 1/content.xml", chart)]
    {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// The document's charts: their type, title, and whether they're drawn.
fn charts(s: &Session) -> Vec<(ChartType, Option<String>, bool)> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Graphic { graphic, .. } => graphic.chart.as_ref().map(|c| (c.kind, c.title.clone(), !graphic.items.is_empty())),
            _ => None,
        })
        .collect()
}

#[test]
fn an_odt_chart_opens_drawn_and_editable() {
    let mut s = Session::new(io::open_bytes("chart.odt", &odt()).unwrap());
    assert_eq!(charts(&s), [(ChartType::Line, Some("Visits".to_string()), true)]);
    s.run("select.objects", &json!({"index": 0})).unwrap();
    s.run("chart.type", &json!({"type": "column"})).unwrap();
    s.run("chart.title", &json!({"text": "Weekly visits"})).unwrap();
    assert_eq!(charts(&s), [(ChartType::Column, Some("Weekly visits".to_string()), true)]);
}

#[test]
fn a_chart_in_a_group_opens_drawn() {
    // The chart is a member of a drawing group (`draw:g`): it is drawn too.
    let odt = odt();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&odt)).unwrap();
    let chart = std::io::read_to_string(z.by_name("Object 1/content.xml").unwrap()).unwrap();
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text><text:p>Group <draw:g text:anchor-type="as-char"><draw:frame svg:x="0in" svg:y="0in" svg:width="3in" svg:height="2in"><draw:object xlink:href="./Object 1"/></draw:frame><draw:rect svg:x="3in" svg:y="0in" svg:width="1in" svg:height="1in"/></draw:g></text:p></office:text></office:body></office:document-content>"#
    );
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in
        [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str()), ("Object 1/content.xml", chart.as_str())]
    {
        w.start_file(name, o).unwrap();
        w.write_all(data.as_bytes()).unwrap();
    }
    let doc = io::open_bytes("group.odt", &w.finish().unwrap().into_inner()).unwrap();
    let members: Vec<bool> = doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::Group { children, .. } = o { Some(children) } else { None })
        .flatten()
        .filter_map(|c| match &c.obj {
            InlineObject::Graphic { graphic, .. } if graphic.chart.is_some() => Some(!graphic.items.is_empty()),
            _ => None,
        })
        .collect();
    assert_eq!(members, [true]);
}
