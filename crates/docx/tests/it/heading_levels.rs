//! #4696: Word's built-in heading styles have their outline level even when the file's
//! `styles.xml` doesn't write `w:outlineLvl` (some producers leave it out). Word for Windows,
//! opening such a file, reports `OutlineLevel` 1 and 2 for "heading 1" and "heading 2" (whatever
//! their style ids) and body text for a custom style.
use std::io::Write;
use wordcraft_doc::Paragraph;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn package(styles: &str, body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#);
    let styles = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}">{styles}</w:styles>"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in
        [("_rels/.rels", root), ("word/document.xml", doc.as_str()), ("word/_rels/document.xml.rels", rels), ("word/styles.xml", styles.as_str())]
    {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(body.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn para(style: &str, text: &str) -> String {
    format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

fn levels(d: &wordcraft_doc::Document) -> Vec<Option<u8>> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p: &Paragraph| d.styles.resolve_para(&p.props).outline_level).collect()
}

#[test]
fn built_in_heading_styles_have_their_outline_level_without_outline_lvl() {
    let styles = r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Titolo2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Heading9"><w:name w:val="Heading 9"/></w:style>
        <w:style w:type="paragraph" w:styleId="Below"><w:name w:val="Below heading 2"/><w:basedOn w:val="Titolo2"/></w:style>
        <w:style w:type="paragraph" w:customStyle="1" w:styleId="MyHead"><w:name w:val="My heading 3"/><w:basedOn w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Heading10"><w:name w:val="heading 10"/></w:style>
        <w:style w:type="paragraph" w:customStyle="on" w:styleId="Fake4"><w:name w:val="heading 4"/></w:style>
        <w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:pPr><w:outlineLvl w:val="5"/></w:pPr></w:style>"#;
    let body = ["Heading1", "Titolo2", "Heading9", "Below", "MyHead", "Heading10", "Fake4", "Heading3", "Normal"]
        .iter()
        .map(|s| para(s, "x"))
        .collect::<String>();
    let d = wordcraft_docx::read(&package(styles, &body)).unwrap();
    // Heading 1, 2 and 9 by name; a style based on a heading inherits its level; a custom style,
    // "heading 10", a custom style named "heading 4" (`w:customStyle="on"`) and Normal have none; a
    // level the file gives is kept.
    let want = vec![Some(0), Some(1), Some(8), Some(1), None, None, None, Some(5), None];
    assert_eq!(levels(&d), want);
    // The level survives a save and reopen.
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(levels(&again), want);
}
