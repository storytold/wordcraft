use std::io::Write;
use std::sync::Arc;

#[test]
fn hyperlink_screentips_share_long_hints_across_differently_formatted_runs() {
    let tip = "Original long hint. ".repeat(512);
    let runs: String = (0..32).map(|i| format!("<w:r><w:rPr><w:b w:val=\"{}\"/></w:rPr><w:t>word </w:t></w:r>", i % 2)).collect();
    let xml = format!(
        r##"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:hyperlink w:anchor="target" w:tooltip="{tip}">{runs}</w:hyperlink></w:p></w:body></w:document>"##
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(xml.as_bytes()).unwrap();
    let doc = wordcraft_docx::read(&zip.finish().unwrap().into_inner()).unwrap();
    let p = doc.body[0].as_para().unwrap();
    assert_eq!(p.runs.len(), 32);
    let first = p.runs[0].props.link_tooltip.as_ref().unwrap();
    assert_eq!(first.as_ref(), tip);
    for run in &p.runs {
        assert!(Arc::ptr_eq(first, run.props.link_tooltip.as_ref().unwrap()));
    }
}
