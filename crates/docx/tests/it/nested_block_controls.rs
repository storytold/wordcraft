//! A body-level content control nested directly in another body-level one stays body-level when
//! saved, even when it holds a single paragraph (ECMA-376 §17.5.2.29-30; [MS-DOCX] `w15`
//! repeating sections, which must wrap whole paragraphs or rows). All markup here is synthetic.

use std::io::Write;

use wordcraft_doc::{ContentControl, Document, StoryRef};
use wordcraft_docx::{read, write};

const NS: &str =
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml""#;
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

fn t(s: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#)
}
fn p(inner: &str) -> String {
    format!("<w:p>{inner}</w:p>")
}
fn sdt(pr: &str, content: &str) -> String {
    format!("<w:sdt><w:sdtPr>{pr}</w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>")
}

fn controls(d: &Document) -> Vec<ContentControl> {
    let mut out = Vec::new();
    wordcraft_doc::control::walk_controls(&d.body, &mut |c| out.push(c.clone()));
    out
}

/// How many `w:p` elements are open at byte `at` of `xml`.
fn open_paragraphs(xml: &str, at: usize) -> usize {
    let head = &xml[..at];
    let opened = head.matches("<w:p>").count() + head.matches("<w:p ").count();
    opened - head.matches("</w:p>").count()
}

/// Paragraph depth at the start of the `w:sdt` whose `w:sdtPr` holds `marker`.
fn depth_of_control(xml: &str, marker: &str) -> usize {
    let at = xml.find(marker).unwrap_or_else(|| panic!("{marker} not written:\n{xml}"));
    let start = xml[..at].rfind("<w:sdt>").expect("control start");
    open_paragraphs(xml, start)
}

fn round_trip(body: &str) -> (Document, Document, String) {
    let d = read(&package(body)).unwrap();
    let bytes = write(&d).unwrap();
    let back = read(&bytes).unwrap();
    (d, back, document_xml(&bytes))
}

fn repeating_section() -> String {
    let field = sdt(r#"<w:alias w:val="note"/><w:id w:val="3"/><w:text/>"#, &t("note 1"));
    let item = sdt(r#"<w:alias w:val="item"/><w:id w:val="2"/><w15:repeatingSectionItem/>"#, &p(&format!("{field}{}", t("   "))));
    let section = sdt(r#"<w:alias w:val="notes"/><w:id w:val="1"/><w15:repeatingSection/>"#, &item);
    format!("{}{section}{}", p(&t("Notes")), p(&t("End")))
}

#[test]
fn a_repeating_section_around_one_paragraph_stays_around_the_paragraph() {
    let (_, _, xml) = round_trip(&repeating_section());
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="notes"/>"#), 0, "section written inside a paragraph:\n{xml}");
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="item"/>"#), 0, "item written inside a paragraph:\n{xml}");
    // The text control stays inside the paragraph, and the item holds that paragraph.
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="note"/>"#), 1);
    assert!(xml.contains("<w15:repeatingSectionItem/></w:sdtPr><w:sdtContent><w:p"), "{xml}");
}

#[test]
fn a_repeating_section_round_trips_its_controls_and_text() {
    let (d, back, _) = round_trip(&repeating_section());
    assert_eq!(controls(&back), controls(&d));
    assert_eq!(back.plain_text(StoryRef::Body), d.plain_text(StoryRef::Body));
    assert_eq!(back.body.len(), d.body.len());
}

#[test]
fn plain_nested_block_controls_around_one_paragraph_stay_block_level() {
    let inner = sdt(r#"<w:alias w:val="inner"/><w:id w:val="11"/>"#, &p(&t("Only paragraph")));
    let outer = sdt(r#"<w:alias w:val="outer"/><w:id w:val="10"/>"#, &inner);
    let (d, back, xml) = round_trip(&format!("{outer}{}", p(&t("After"))));
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="outer"/>"#), 0, "{xml}");
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="inner"/>"#), 0, "{xml}");
    assert_eq!(controls(&back), controls(&d));
}

#[test]
fn nested_block_controls_in_a_table_cell_stay_block_level() {
    let inner = sdt(r#"<w:alias w:val="cellInner"/><w:id w:val="21"/>"#, &p(&t("In cell")));
    let outer = sdt(r#"<w:alias w:val="cellOuter"/><w:id w:val="20"/>"#, &inner);
    let table = format!(r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc>{outer}</w:tc></w:tr></w:tbl>"#);
    let (_, _, xml) = round_trip(&format!("{table}{}", p(&t("After"))));
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="cellOuter"/>"#), 0, "{xml}");
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="cellInner"/>"#), 0, "{xml}");
}

#[test]
fn a_run_level_control_stays_inside_its_paragraph() {
    let field = sdt(r#"<w:alias w:val="inline"/><w:id w:val="30"/>"#, &t("value"));
    let (_, _, xml) = round_trip(&p(&format!("{}{field}", t("Name: "))));
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="inline"/>"#), 1, "{xml}");
}

/// A bound repeating section as Word writes one at the end of a form, optionally followed by
/// the body-level `_GoBack` bookmark Word leaves after the last control.
fn bound_section_body(bound: bool, bookmark: bool) -> String {
    let section_binding = if bound {
        r#"<w15:dataBinding w:prefixMappings="" w:xpath="/a[1]/n[1]" w:storeItemID="{00000000-0000-0000-0000-0000000000AA}"/>"#
    } else {
        ""
    };
    let trailing = if bookmark { r#"<w:bookmarkStart w:id="0" w:name="_GoBack"/><w:bookmarkEnd w:id="0"/>"# } else { "" };
    let bind = |x: &str| format!(r#"<w:dataBinding w:prefixMappings="" w:xpath="{x}" w:storeItemID="{{00000000-0000-0000-0000-0000000000AA}}"/>"#);
    let field = format!(
        r#"<w:sdt><w:sdtPr><w:alias w:val="note"/><w:placeholder><w:docPart w:val="DefaultPlaceholder_1"/></w:placeholder>{}<w:text/></w:sdtPr><w:sdtEndPr/><w:sdtContent>{}</w:sdtContent></w:sdt>"#,
        if bound { bind("/a[1]/n[1]") } else { String::new() },
        t("note 1")
    );
    let item = format!(
        r#"<w:sdt><w:sdtPr><w:alias w:val="item"/><w:placeholder><w:docPart w:val="DefaultPlaceholder_2"/></w:placeholder><w15:repeatingSectionItem/></w:sdtPr><w:sdtEndPr/><w:sdtContent>{}</w:sdtContent></w:sdt>"#,
        p(&format!("{field}{}", t("   ")))
    );
    let section = format!(
        r#"<w:sdt><w:sdtPr><w:alias w:val="notes"/>{section_binding}<w15:repeatingSection/></w:sdtPr><w:sdtContent>{item}</w:sdtContent></w:sdt>"#
    );
    format!(r#"{}{section}{trailing}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>"#, p(&t("Notes")))
}

fn assert_section_block_level(body: &str) {
    let (d, back, xml) = round_trip(body);
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="notes"/>"#), 0, "section written inside a paragraph:\n{xml}");
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="item"/>"#), 0, "item written inside a paragraph:\n{xml}");
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="note"/>"#), 1, "{xml}");
    assert_eq!(controls(&back), controls(&d));
    assert_eq!(back.plain_text(StoryRef::Body), d.plain_text(StoryRef::Body));
}

#[test]
fn a_bound_repeating_section_followed_by_a_body_bookmark_stays_block_level() {
    assert_section_block_level(&bound_section_body(true, true));
}

#[test]
fn a_bound_repeating_section_at_the_end_of_the_body_stays_block_level() {
    assert_section_block_level(&bound_section_body(true, false));
}

#[test]
fn a_repeating_section_followed_by_a_body_bookmark_stays_block_level() {
    assert_section_block_level(&bound_section_body(false, true));
}

#[test]
fn a_block_control_followed_by_a_body_bookmark_stays_around_its_paragraph() {
    let inner = sdt(r#"<w:alias w:val="inner"/><w:id w:val="41"/>"#, &p(&t("Last paragraph")));
    let outer = sdt(r#"<w:alias w:val="outer"/><w:id w:val="40"/>"#, &inner);
    let body = format!(r#"{}{outer}<w:bookmarkStart w:id="0" w:name="_GoBack"/><w:bookmarkEnd w:id="0"/>"#, p(&t("First")));
    let (d, back, xml) = round_trip(&body);
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="outer"/>"#), 0, "{xml}");
    assert_eq!(depth_of_control(&xml, r#"<w:alias w:val="inner"/>"#), 0, "{xml}");
    assert_eq!(controls(&back), controls(&d));
    // The bookmark is kept.
    assert!(xml.contains(r#"w:name="_GoBack""#), "{xml}");
}
