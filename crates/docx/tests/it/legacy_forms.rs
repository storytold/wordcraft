//! Word forms keep their legacy form fields (`w:ffData`) and their rows' skipped grid columns
//! (`w:gridBefore`/`w:wBefore`, `w:gridAfter`/`w:wAfter`) through a round trip (#1174). All
//! markup here is synthetic, written from ECMA-376 §17.16.17 and §17.4.14-17.

use std::io::Write;

use wordcraft_doc::Block;
use wordcraft_docx::{read, write};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

fn package(body: &str) -> Vec<u8> {
    let doc = format!(r#"<w:document {NS}><w:body>{body}</w:body></w:document>"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn document_xml(docx: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut s).unwrap();
    s
}

fn resaved(body: &str) -> String {
    document_xml(&write(&read(&package(body)).unwrap()).unwrap())
}

fn t(s: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#)
}

fn checkbox(name: &str, checked: bool) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="{name}"/><w:enabled/><w:calcOnExit w:val="0"/><w:checkBox><w:sizeAuto/><w:default w:val="{}"/></w:checkBox></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMCHECKBOX </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        u8::from(checked)
    )
}

fn text_input(name: &str, text: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="{name}"/><w:enabled/><w:calcOnExit w:val="0"/><w:textInput><w:maxLength w:val="40"/></w:textInput></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMTEXT </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        t(text)
    )
}

/// The `w:ffData` of each field begin, in document order.
fn ff_data(xml: &str) -> Vec<&str> {
    xml.match_indices("<w:ffData>")
        .filter_map(|(at, _)| xml.get(at..).and_then(|s| s.find("</w:ffData>").and_then(|end| s.get(..end + 11))))
        .collect()
}

#[test]
fn form_fields_keep_their_ff_data() {
    let body = format!(
        "<w:p>{}{}{}{}</w:p><w:p>{}{}</w:p>",
        t("Yes "),
        checkbox("Yes1", true),
        t(" No "),
        checkbox("No1", false),
        t("Name: "),
        text_input("Name1", "Example Ltd")
    );
    let xml = resaved(&body);
    let ff = ff_data(&xml);
    assert_eq!(ff.len(), 3, "{xml}");
    assert!(
        ff[0].contains(r#"<w:name w:val="Yes1"/>"#) && ff[0].contains(r#"<w:checkBox><w:sizeAuto/><w:default w:val="1"/></w:checkBox>"#),
        "{}",
        ff[0]
    );
    assert!(ff[1].contains(r#"<w:name w:val="No1"/>"#) && ff[1].contains(r#"<w:default w:val="0"/>"#), "{}", ff[1]);
    assert!(ff[2].contains(r#"<w:name w:val="Name1"/>"#) && ff[2].contains(r#"<w:maxLength w:val="40"/>"#), "{}", ff[2]);
    // Each definition stays inside its own field's begin, ahead of its code.
    let begin = xml.find(r#"w:fldCharType="begin"><w:ffData><w:name w:val="Name1"/>"#).expect("ffData inside fldChar");
    assert!(xml.get(begin..).is_some_and(|s| s.find("FORMTEXT") < s.find("Example Ltd")), "{xml}");
}

#[test]
fn fields_without_ff_data_write_none() {
    let body = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let xml = resaved(body);
    assert!(xml.contains("PAGE") && !xml.contains("ffData"), "{xml}");
}

#[test]
fn rows_keep_their_skipped_grid_columns() {
    let tc = |w: u32, span: u32, s: &str| {
        let gs = if span > 1 { format!(r#"<w:gridSpan w:val="{span}"/>"#) } else { String::new() };
        format!(r#"<w:tc><w:tcPr><w:tcW w:w="{w}" w:type="dxa"/>{gs}</w:tcPr><w:p>{}</w:p></w:tc>"#, t(s))
    };
    let body = format!(
        concat!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="300"/><w:gridCol w:w="4200"/><w:gridCol w:w="4500"/></w:tblGrid>"#,
            "<w:tr>{}{}{}</w:tr>",
            r#"<w:tr><w:trPr><w:gridBefore w:val="1"/><w:wBefore w:w="300" w:type="dxa"/></w:trPr>{}</w:tr>"#,
            r#"<w:tr><w:trPr><w:gridAfter w:val="2"/><w:wAfter w:w="8700" w:type="dxa"/></w:trPr>{}</w:tr>"#,
            "</w:tbl><w:p/>"
        ),
        tc(300, 1, "a"),
        tc(4200, 1, "b"),
        tc(4500, 1, "c"),
        tc(8700, 2, "Title"),
        tc(300, 1, "x"),
    );
    let d = read(&package(&body)).unwrap();
    let Some(Block::Table(tb)) = d.body.first().map(|b| b.as_ref()) else { panic!("table") };
    let (r1, r2) = (&tb.rows[1].props, &tb.rows[2].props);
    assert_eq!((r1.grid_before, r1.w_before, r1.grid_after, r1.w_after), (1, Some(15.0), 0, None));
    assert_eq!((r2.grid_before, r2.w_before, r2.grid_after, r2.w_after), (0, None, 2, Some(435.0)));
    assert_eq!(tb.rows[0].props, Default::default());

    let xml = document_xml(&write(&d).unwrap());
    assert_eq!(xml.matches("<w:gridBefore ").count(), 1, "{xml}");
    assert!(xml.contains(r#"<w:gridBefore w:val="1"/><w:wBefore w:w="300" w:type="dxa"/>"#), "{xml}");
    assert!(xml.contains(r#"<w:gridAfter w:val="2"/><w:wAfter w:w="8700" w:type="dxa"/>"#), "{xml}");
    let again = read(&write(&d).unwrap()).unwrap();
    let Some(Block::Table(tb2)) = again.body.first().map(|b| b.as_ref()) else { panic!("table") };
    assert_eq!(tb2.rows.iter().map(|r| r.props.clone()).collect::<Vec<_>>(), tb.rows.iter().map(|r| r.props.clone()).collect::<Vec<_>>());
}
