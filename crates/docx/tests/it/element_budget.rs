//! The package-wide XML element budget: a file of many parts each inside the per-part
//! cap reads bounded, where every part used to build its own tree and the trees the
//! reader keeps accumulated.

use crate::fixtures;
use wordcraft_docx::read;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

#[test]
fn a_file_of_element_budget_parts_still_reads() {
    // Nine chart parts of 2M elements each: 18M over the package's 16M budget. The later
    // parts are skipped with a warning instead of built, and the file still reads. Before
    // the budget, every part's whole tree was parsed and the chart trees stayed cached for
    // the whole read (~230 MB per 2M-element part, measured), so a file like this cost one
    // tree per part and scales linearly with the part count.
    let parts = 9usize;
    let chunk = "<a/>".repeat(2_000_000);
    let chart_xml = format!(r#"<c:chart {W_NS}>{chunk}</c:chart>"#);
    let mut document = format!(
        r#"<w:document {W_NS} xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><w:body>"#
    );
    for i in 0..parts {
        document.push_str(&format!(
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="635000" cy="635000"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rId{i}"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
        ));
    }
    document.push_str("</w:body></w:document>");
    let mut rels = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for i in 0..parts {
        rels.push_str(&format!(
            r#"<Relationship Id="rId{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart{i}.xml"/>"#
        ));
    }
    rels.push_str("</Relationships>");
    let mut entries: Vec<(&str, Vec<u8>)> = vec![("word/document.xml", document.into_bytes()), ("word/_rels/document.xml.rels", rels.into_bytes())];
    for i in 0..parts {
        entries.push((Box::leak(format!("word/charts/chart{i}.xml").into_boxed_str()), chart_xml.clone().into_bytes()));
    }
    let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    let z = fixtures::zip(&refs);
    let doc = read(&z).expect("a file over the element budget still reads, the extra parts skipped");
    assert_eq!(doc.body.len(), parts, "the document's own paragraphs read");
}
