//! Original packages for size-varying chart/SmartArt input-work limits (#3728).

use std::sync::Arc;

use wordcraft_doc::graphic::Graphic;
use wordcraft_doc::{Document, InlineObject};

use super::fixtures::zip;

const NS: &str = "http://schemas.openxmlformats.org/";

fn chart(points: usize) -> String {
    let pts: String = (0..points).map(|i| format!(r#"<c:pt idx="{i}"><c:v>not-a-number</c:v></c:pt>"#)).collect();
    format!(
        r#"<c:chartSpace xmlns:c="{NS}drawingml/2006/chart"><c:chart><c:plotArea><c:lineChart><c:ser><c:idx val="0"/><c:val><c:numLit><c:ptCount val="{points}"/>{pts}</c:numLit></c:val></c:ser></c:lineChart></c:plotArea></c:chart></c:chartSpace>"#
    )
}

fn frame(width: usize, diagram: bool) -> String {
    let payload = if diagram { r#"<dgm:relIds r:dm="data"/>"# } else { r#"<c:chart r:id="chart"/>"# };
    let kind = if diagram { "diagram" } else { "chart" };
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="{width}" cy="1524000"/><wp:docPr id="1" name="Original input-work probe"/><a:graphic><a:graphicData uri="{NS}drawingml/2006/{kind}">{payload}</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    )
}

fn package(frames: &str, chart: &str, drawing: &str, drawing_rels: &str) -> Vec<u8> {
    let doc = format!(
        r#"<w:document xmlns:w="{NS}wordprocessingml/2006/main" xmlns:r="{NS}officeDocument/2006/relationships" xmlns:wp="{NS}drawingml/2006/wordprocessingDrawing" xmlns:a="{NS}drawingml/2006/main" xmlns:c="{NS}drawingml/2006/chart" xmlns:dgm="{NS}drawingml/2006/diagram"><w:body>{frames}<w:p><w:r><w:t>Original preserved text</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"#
    );
    let root = format!(
        r#"<Relationships xmlns="{NS}package/2006/relationships"><Relationship Id="main" Type="{NS}officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let rels = format!(
        r#"<Relationships xmlns="{NS}package/2006/relationships"><Relationship Id="chart" Type="{NS}officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/><Relationship Id="data" Type="{NS}officeDocument/2006/relationships/diagramData" Target="diagrams/data1.xml"/><Relationship Id="drawing" Type="{NS}officeDocument/2006/relationships/diagramDrawing" Target="diagrams/drawing1.xml"/></Relationships>"#
    );
    let data = format!(
        r#"<dgm:dataModel xmlns:dgm="{NS}drawingml/2006/diagram" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram"><dsp:dataModelExt relId="drawing"/></dgm:dataModel>"#
    );
    zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/charts/chart1.xml", chart.as_bytes()),
        ("word/diagrams/data1.xml", data.as_bytes()),
        ("word/diagrams/drawing1.xml", drawing.as_bytes()),
        ("word/diagrams/_rels/drawing1.xml.rels", drawing_rels.as_bytes()),
    ])
}

fn graphics(doc: &Document) -> Vec<&Arc<Graphic>> {
    doc.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| &p.objects)
        .map(|o| match o {
            InlineObject::Graphic { graphic, .. } => graphic,
            other => panic!("unexpected object: {other:?}"),
        })
        .collect()
}

#[test]
fn graphic_input_work_bounds_size_variants_and_keeps_cached_sizes() {
    let chart = chart(5000);
    let mut frames: String = (0..80).map(|i| frame(2_540_000 + i * 12_700, false)).collect();
    frames.push_str(&frame(2_540_000, false));
    let original = package(&frames, &chart, "", "");
    let doc = wordcraft_docx::read(&original).unwrap();
    let gs = graphics(&doc);
    assert_eq!(gs.len(), 81);
    assert!(!gs[0].items.is_empty());
    // One 234 KB input part must not be rebuilt eighty times just because its output is small.
    assert!(gs[..80].iter().filter(|g| !g.items.is_empty()).count() < 10);
    assert!(gs[79].items.is_empty());
    assert!(Arc::ptr_eq(gs[0], gs[80]), "a cached size remains usable after exhaustion");
    for (i, g) in gs[..80].iter().enumerate() {
        assert_eq!(g.w, 200.0 + i as f32);
        assert_eq!(g.h, 120.0);
        assert!(g.source.is_some(), "unrendered charts still keep their source");
    }
    // Saving must keep every object and the complete original chart, including unread points.
    let saved = wordcraft_docx::write(&doc).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(saved.clone())).unwrap();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut archive.by_name("word/charts/chart1.xml").unwrap(), &mut bytes).unwrap();
    assert_eq!(bytes, chart.as_bytes());
    let reopened = wordcraft_docx::read(&saved).unwrap();
    assert_eq!(graphics(&reopened).len(), gs.len());
    assert_eq!(reopened.body.last().unwrap().as_para().unwrap().text, "Original preserved text");
}

#[test]
fn empty_diagram_builds_spend_input_work_too() {
    let drawing = format!(
        r#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram"><dsp:spTree>{}</dsp:spTree></dsp:drawing>"#,
        "<dsp:sp/>".repeat(20_000)
    );
    let mut frames = frame(2_540_000, false);
    for i in 0..20 {
        frames.push_str(&frame(2_540_000 + i * 12_700, true));
    }
    frames.push_str(&frame(3_810_000, false));
    frames.push_str(&frame(2_540_000, false));
    let doc = wordcraft_docx::read(&package(&frames, &chart(1), &drawing, "")).unwrap();
    let gs = graphics(&doc);
    assert_eq!(gs.len(), 23);
    assert!(!gs[0].items.is_empty());
    assert!(gs[1..21].iter().all(|g| g.items.is_empty() && g.source.is_some()));
    assert!(gs[21].items.is_empty(), "empty diagram output must not allow unlimited input work");
    assert!(Arc::ptr_eq(gs[0], gs[22]));
}

#[test]
fn ordinary_chart_sizes_still_render_independently() {
    let frames: String = (0..40).map(|i| frame(2_540_000 + i * 12_700, false)).collect();
    let doc = wordcraft_docx::read(&package(&frames, &chart(1), "", "")).unwrap();
    let gs = graphics(&doc);
    assert_eq!(gs.len(), 40);
    assert!(gs.iter().all(|g| !g.items.is_empty()));
    assert!(!Arc::ptr_eq(gs[0], gs[1]));
    assert_ne!(gs[0].items, gs[1].items, "size-dependent chart layout must not be scaled from one size");
}

#[test]
fn diagram_relationship_input_also_bounds_repeated_empty_builds() {
    let drawing = r#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram"><dsp:spTree/></dsp:drawing>"#;
    let entries: String = (0..5000)
        .map(|i| format!(r#"<Relationship Id="image{i}" Type="{NS}officeDocument/2006/relationships/image" Target="https://example.invalid/{i}" TargetMode="External"/>"#))
        .collect();
    let rels = format!(r#"<Relationships xmlns="{NS}package/2006/relationships">{entries}</Relationships>"#);
    let mut frames = frame(2_540_000, false);
    for i in 0..10 {
        frames.push_str(&frame(2_540_000 + i * 12_700, true));
    }
    frames.push_str(&frame(3_810_000, false));
    let doc = wordcraft_docx::read(&package(&frames, &chart(1), drawing, &rels)).unwrap();
    let gs = graphics(&doc);
    assert!(!gs[0].items.is_empty());
    assert!(gs[1..11].iter().all(|g| g.items.is_empty()));
    assert!(gs[11].items.is_empty(), "relationship parsing counts even when the drawing is tiny");
}

#[test]
fn a_single_chart_may_finish_the_remaining_work_budget() {
    let chart = chart(50_000);
    assert!(chart.len() > 2_000_000);
    let frames = frame(2_540_000, false) + &frame(3_810_000, false) + &frame(2_540_000, false);
    let doc = wordcraft_docx::read(&package(&frames, &chart, "", "")).unwrap();
    let gs = graphics(&doc);
    assert!(!gs[0].items.is_empty(), "the last permitted build is still rendered");
    assert!(gs[1].items.is_empty());
    assert!(gs[1].source.is_some());
    assert!(Arc::ptr_eq(gs[0], gs[2]));
}
