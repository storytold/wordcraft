//! Embedded objects whose part graph a file doesn't hold whole are reported, never saved with
//! dangling relationships (#1343). Packages are hand-written for these tests.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use wordcraft_docx::LossKind;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const CHART_XML: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart><c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"#;
const WORKBOOK: &str = "PK\u{3}\u{4} not really a workbook";
const COLORS_XML: &str = r#"<cs:colorStyle xmlns:cs="http://schemas.microsoft.com/office/drawing/2012/chartStyle" meth="cycle" id="10"/>"#;
const PACKAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/package";
const COLORS_REL: &str = "http://schemas.microsoft.com/office/2011/relationships/chartColorStyle";

fn zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        out.insert(f.name().to_string(), b);
    }
    out
}

/// A relationships part: (id, type, target) each.
fn rels(list: &[(String, &str, String)]) -> Vec<u8> {
    let mut s = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in list {
        let ext = if target.starts_with("http") { r#" TargetMode="External""# } else { "" };
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"{ext}/>"#));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

/// A package with one inline chart (`word/charts/chart1.xml`) whose relationships are
/// `chart_rels`, and the `extra` parts.
fn chart_package(chart_rels: &[(String, &str, String)], extra: Vec<(String, Vec<u8>)>) -> Vec<u8> {
    let body = r#"<w:p><w:r><w:t>Sales</w:t></w:r><w:r><w:drawing><wp:inline><wp:extent cx="2743200" cy="1371600"/><wp:docPr id="1" name="Chart 1"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId7"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let mut entries = vec![
        ("_rels/.rels".to_string(), ROOT_RELS.as_bytes().to_vec()),
        ("word/document.xml".to_string(), doc.into_bytes()),
        (
            "word/_rels/document.xml.rels".to_string(),
            rels(&[("rId7".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart", "charts/chart1.xml".into())]),
        ),
        ("word/charts/chart1.xml".to_string(), CHART_XML.as_bytes().to_vec()),
        ("word/charts/_rels/chart1.xml.rels".to_string(), rels(chart_rels)),
    ];
    entries.extend(extra);
    zip(&entries)
}

/// The chart's usual relationships: its workbook and its colour style.
fn usual_rels() -> Vec<(String, &'static str, String)> {
    vec![("rId1".into(), PACKAGE_REL, "../embeddings/book.xlsx".into()), ("rId2".into(), COLORS_REL, "colors1.xml".into())]
}

fn workbook() -> (String, Vec<u8>) {
    ("word/embeddings/book.xlsx".into(), WORKBOOK.as_bytes().to_vec())
}

fn colors() -> (String, Vec<u8>) {
    ("word/charts/colors1.xml".into(), COLORS_XML.as_bytes().to_vec())
}

/// Package part a relationship target of a part in `from_dir` names.
fn part_at(from_dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') { Vec::new() } else { from_dir.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Every internal relationship in the package targets a part the package holds.
fn assert_no_dangling(files: &BTreeMap<String, Vec<u8>>) {
    for (name, bytes) in files.iter().filter(|(n, _)| n.ends_with(".rels")) {
        // `dir/_rels/file.rels` relates parts from `dir`.
        let dir = name.rsplit_once("_rels/").map_or("", |(d, _)| d.trim_end_matches('/'));
        let xml = String::from_utf8_lossy(bytes);
        for r in xml.split("<Relationship ").skip(1) {
            if r.contains(r#"TargetMode="External""#) {
                continue;
            }
            let target = r.split("Target=\"").nth(1).and_then(|t| t.split('"').next()).unwrap();
            let part = part_at(dir, target);
            assert!(files.contains_key(&part), "{name} relates to {target} ({part}), which isn't in the package: {:?}", files.keys());
        }
    }
}

/// Every relationship id a written part's markup uses (`r:…` attributes) is in that part's
/// relationships.
fn assert_no_dangling_ids(files: &BTreeMap<String, Vec<u8>>) {
    for (name, bytes) in files.iter().filter(|(n, _)| n.ends_with(".xml") && !n.contains("_rels/") && n.as_str() != "[Content_Types].xml") {
        let xml = String::from_utf8_lossy(bytes);
        let (dir, file) = name.rsplit_once('/').unwrap_or(("", name));
        let rels_name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let rels = files.get(&rels_name).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        for attr in xml.split(" r:").skip(1) {
            let Some((_, value)) = attr.split_once("=\"") else { continue };
            let Some(id) = value.split('"').next() else { continue };
            assert!(rels.contains(&format!("Id=\"{id}\"")), "{name} uses r:id {id}, which {rels_name} doesn't have: {xml}\n{rels}");
        }
    }
}

/// (a) The chart is in the file but its workbook isn't: reading says the chart is incomplete;
/// saving reports it and writes no relationship to the missing workbook.
#[test]
fn missing_second_hop_part_is_reported_on_read_and_write() {
    let bytes = chart_package(&usual_rels(), vec![colors()]);
    let doc = wordcraft_docx::read(&bytes).unwrap();
    let incomplete = wordcraft_docx::incomplete_objects(&doc);
    assert!(
        incomplete
            .iter()
            .any(|l| l.part == "word/charts/chart1.xml" && l.kind == LossKind::MissingPart && l.reason.contains("word/embeddings/book.xlsx")),
        "the chart is marked incomplete: {incomplete:?}"
    );
    let (out, losses) = wordcraft_docx::write_with_report(&doc).unwrap();
    assert!(
        losses.iter().any(|l| l.kind == LossKind::MissingPart && l.reason.contains("word/embeddings/book.xlsx")),
        "saving reports the lost workbook: {losses:?}"
    );
    let files = unzip(&out);
    assert!(files.contains_key("word/charts/chart1.xml"), "the chart itself is still written");
    assert_no_dangling(&files);
    // The chart's optional link to its data (`c:externalData`) goes with the workbook.
    assert_no_dangling_ids(&files);
    assert!(!String::from_utf8_lossy(&files["word/charts/chart1.xml"]).contains("externalData"));
    // A Word XML Document (Flat OPC) reports the same.
    let (_, flat) = wordcraft_docx::write_flat_with_report(&doc).unwrap();
    assert!(flat.iter().any(|l| l.reason.contains("word/embeddings/book.xlsx")), "{flat:?}");
}

/// A part whose markup needs a relationship that's gone (not an optional link) can't be written
/// whole: the object is left out and reported, with no dangling id.
#[test]
fn part_needing_a_lost_relationship_leaves_the_object_out() {
    let chart = CHART_XML.replace("</c:chartSpace>", r#"<c:needed r:id="rId9"/></c:chartSpace>"#);
    let mut list = usual_rels();
    list.push(("rId9".into(), "x/needed", "needed.xml".into()));
    let body = chart_package(&list, vec![colors(), workbook()]);
    let mut files = unzip(&body);
    files.insert("word/charts/chart1.xml".into(), chart.into_bytes());
    let doc = wordcraft_docx::read(&zip(&files.into_iter().collect::<Vec<_>>())).unwrap();
    let (out, losses) = wordcraft_docx::write_with_report(&doc).unwrap();
    assert!(losses.iter().any(|l| l.kind == LossKind::LeftOut), "{losses:?}");
    let files = unzip(&out);
    assert_no_dangling(&files);
    assert_no_dangling_ids(&files);
    assert!(!String::from_utf8_lossy(&files["word/document.xml"]).contains("c:chart "), "the chart is left out");
}

/// Past the cap on notes, a last note says more weren't listed.
#[test]
fn notes_past_the_cap_end_with_a_truncation_note() {
    let mut list = usual_rels();
    for i in 0..300 {
        list.push((format!("rIdM{i}"), "x/missing", format!("missing{i}.xml")));
    }
    let doc = wordcraft_docx::read(&chart_package(&list, vec![colors(), workbook()])).unwrap();
    let incomplete = wordcraft_docx::incomplete_objects(&doc);
    assert_eq!(incomplete.len(), 256);
    assert_eq!(incomplete.last().map(|l| l.kind), Some(LossKind::Truncated), "{:?}", incomplete.last());
}

/// A part with an unreadable name (a tab in it) is still noted, under a placeholder name.
#[test]
fn unreadable_part_name_is_noted_with_a_placeholder() {
    let mut list = usual_rels();
    list.push(("rIdT".into(), "x/odd", "odd&#9;name.xml".into()));
    let doc = wordcraft_docx::read(&chart_package(&list, vec![colors(), workbook()])).unwrap();
    let incomplete = wordcraft_docx::incomplete_objects(&doc);
    assert!(incomplete.iter().any(|l| l.kind == LossKind::NotCarried && l.reason.contains("unreadable name")), "{incomplete:?}");

    // The object's own part, named so.
    let mut files = unzip(&chart_package(&usual_rels(), vec![colors(), workbook()]));
    files.insert(
        "word/_rels/document.xml.rels".into(),
        rels(&[("rId7".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart", "charts/c&#9;1.xml".into())]),
    );
    let doc = wordcraft_docx::read(&zip(&files.into_iter().collect::<Vec<_>>())).unwrap();
    let incomplete = wordcraft_docx::incomplete_objects(&doc);
    assert!(incomplete.iter().any(|l| l.kind == LossKind::NotCarried && l.reason.contains("unreadable name")), "{incomplete:?}");
}

/// A part the document lost after reading (its kept bytes are gone) is reported by the writer,
/// and its relationship left out rather than left dangling.
#[test]
fn part_lost_after_reading_is_reported_and_not_related() {
    let bytes = chart_package(&usual_rels(), vec![colors(), workbook()]);
    let mut doc = wordcraft_docx::read(&bytes).unwrap();
    assert!(doc.passthrough.remove("word/embeddings/book.xlsx").is_some());
    let (out, losses) = wordcraft_docx::write_with_report(&doc).unwrap();
    assert!(losses.iter().any(|l| l.kind == LossKind::MissingPart && l.reason.contains("word/embeddings/book.xlsx")), "{losses:?}");
    let files = unzip(&out);
    assert_no_dangling(&files);
    assert_no_dangling_ids(&files);
}

/// (b) Past the relationship cap of one part, the rest are reported, not silently dropped.
#[test]
fn relationship_cap_is_reported() {
    let mut list = usual_rels();
    for i in 0..1100 {
        list.push((format!("rIdX{i}"), "x/hyperlink", format!("http://example.com/{i}")));
    }
    let doc = wordcraft_docx::read(&chart_package(&list, vec![colors(), workbook()])).unwrap();
    let incomplete = wordcraft_docx::incomplete_objects(&doc);
    assert!(incomplete.iter().any(|l| l.part == "word/charts/chart1.xml" && l.kind == LossKind::RelationshipLimit), "{incomplete:?}");
    let (out, losses) = wordcraft_docx::write_with_report(&doc).unwrap();
    assert!(!losses.is_empty());
    assert_no_dangling(&unzip(&out));
}

/// (b) Past the part budget for embedded objects, the parts left behind are reported.
#[test]
fn part_budget_is_reported() {
    let mut list = usual_rels();
    let mut extra = vec![colors(), workbook()];
    for h in 0..5 {
        list.push((format!("rIdH{h}"), "x/hub", format!("hub{h}.xml")));
        let mut hub = Vec::new();
        for leaf in 0..1000 {
            hub.push((format!("rIdL{leaf}"), "x/leaf", format!("leaves/l{h}_{leaf}.bin")));
            extra.push((format!("word/charts/leaves/l{h}_{leaf}.bin"), vec![b'x']));
        }
        extra.push((format!("word/charts/hub{h}.xml"), b"<hub/>".to_vec()));
        extra.push((format!("word/charts/_rels/hub{h}.xml.rels"), rels(&hub)));
    }
    let doc = wordcraft_docx::read(&chart_package(&list, extra)).unwrap();
    let incomplete = wordcraft_docx::incomplete_objects(&doc);
    assert!(
        incomplete.iter().any(|l| l.kind == LossKind::PartLimit && l.reason.contains("limit of 4096 parts")),
        "over the part budget is reported: {:?}",
        incomplete.first()
    );
    let (out, losses) = wordcraft_docx::write_with_report(&doc).unwrap();
    assert!(!losses.is_empty());
    assert_no_dangling(&unzip(&out));
}

/// (c) A relationship cycle (chart → colours → chart) is complete: no loss, no endless walk.
#[test]
fn relationship_cycle_is_complete() {
    let back = rels(&[("rId1".into(), "x/back", "chart1.xml".into())]);
    let bytes = chart_package(&usual_rels(), vec![colors(), workbook(), ("word/charts/_rels/colors1.xml.rels".into(), back)]);
    let doc = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(wordcraft_docx::incomplete_objects(&doc), []);
    let (out, losses) = wordcraft_docx::write_with_report(&doc).unwrap();
    assert_eq!(losses, []);
    let files = unzip(&out);
    assert_no_dangling(&files);
    assert!(String::from_utf8_lossy(&files["word/charts/_rels/colors1.xml.rels"]).contains("chart1.xml"));
}

/// (d) A complete chart round-trips with nothing reported.
#[test]
fn complete_object_round_trips_without_loss() {
    let doc = wordcraft_docx::read(&chart_package(&usual_rels(), vec![colors(), workbook()])).unwrap();
    assert_eq!(wordcraft_docx::incomplete_objects(&doc), []);
    let (out, losses) = wordcraft_docx::write_with_report(&doc).unwrap();
    assert_eq!(losses, []);
    let files = unzip(&out);
    assert_eq!(files.get("word/embeddings/book.xlsx").map(Vec::as_slice), Some(WORKBOOK.as_bytes()));
    assert_no_dangling(&files);
    assert_no_dangling_ids(&files);
    assert!(String::from_utf8_lossy(&files["word/charts/chart1.xml"]).contains("externalData"), "a complete chart keeps its data link");
    // And again, from what was written.
    let again = wordcraft_docx::read(&out).unwrap();
    assert_eq!(wordcraft_docx::incomplete_objects(&again), []);
    assert_eq!(wordcraft_docx::write_with_report(&again).unwrap().1, []);
}
