//! `wordcraft-cli convert` says on stderr what a saved Word package couldn't carry of an embedded
//! object, here a chart whose workbook the source file lacks (#1343).

use std::process::Command;

const RELS_CT: &str = "application/vnd.openxmlformats-package.relationships+xml";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn part(name: &str, ct: &str, xml: &str) -> String {
    format!(r#"<pkg:part pkg:name="{name}" pkg:contentType="{ct}"><pkg:xmlData>{xml}</pkg:xmlData></pkg:part>"#)
}

fn rels(id: &str, kind: &str, target: &str) -> String {
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="{id}" Type="{R}/{kind}" Target="{target}"/></Relationships>"#
    )
}

/// A Word XML Document (Flat OPC) with one chart that relates to a workbook it doesn't hold.
fn flat_with_incomplete_chart() -> String {
    let body = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="{R}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:body><w:p><w:r><w:drawing><wp:inline><wp:extent cx="2743200" cy="1371600"/><wp:docPr id="1" name="Chart 1"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId7"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p></w:body></w:document>"#
    );
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><?mso-application progid="Word.Document"?><pkg:package xmlns:pkg="http://schemas.microsoft.com/office/2006/xmlPackage">"#,
    );
    s.push_str(&part("/_rels/.rels", RELS_CT, &rels("rId1", "officeDocument", "word/document.xml")));
    s.push_str(&part("/word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml", &body));
    s.push_str(&part("/word/_rels/document.xml.rels", RELS_CT, &rels("rId7", "chart", "charts/chart1.xml")));
    s.push_str(&part(
        "/word/charts/chart1.xml",
        "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#,
    ));
    s.push_str(&part("/word/charts/_rels/chart1.xml.rels", RELS_CT, &rels("rId1", "package", "../embeddings/book.xlsx")));
    s.push_str("</pkg:package>");
    s
}

#[test]
fn convert_to_docx_prints_what_was_dropped() {
    let dir = std::env::temp_dir().join(format!("wordcraft-cli-dropped-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, out) = (dir.join("chart.xml"), dir.join("out.docx"));
    std::fs::write(&src, flat_with_incomplete_chart()).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).arg("convert").arg(&src).arg(&out).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert!(stderr.contains("book.xlsx") && stderr.contains("missingPart"), "stderr names the dropped workbook: {stderr}");
}
