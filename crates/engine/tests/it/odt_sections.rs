//! #1379: an ODT file with several page styles and a column section converts to DOCX with one
//! `w:sectPr` per section. The ODT is original and synthetic, built from ODF 1.3: paragraphs
//! whose automatic styles name a master page (`style:master-page-name`), one restarting page
//! numbering (`style:page-number`), and a two-column `text:section`.

use wordcraft_engine::io;

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#;

fn odt() -> Vec<u8> {
    let styles = format!(
        r#"<office:document-styles {NS}><office:automatic-styles>
        <style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" style:print-orientation="portrait" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
        <style:page-layout style:name="pW"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" style:print-orientation="landscape" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
        </office:automatic-styles><office:master-styles>
        <style:master-page style:name="Upright" style:page-layout-name="pU"/>
        <style:master-page style:name="Wide" style:page-layout-name="pW"/>
        <style:master-page style:name="Back" style:page-layout-name="pU"/>
        </office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS}><office:automatic-styles>
        <style:style style:name="PWide" style:family="paragraph" style:master-page-name="Wide"><style:paragraph-properties fo:break-before="page" style:page-number="1"/></style:style>
        <style:style style:name="PBack" style:family="paragraph" style:master-page-name="Back"><style:paragraph-properties fo:break-before="page" style:page-number="1"/></style:style>
        <style:style style:name="Cols" style:family="section"><style:section-properties><style:columns fo:column-count="2" fo:column-gap="1.25cm"><style:column-sep style:width="0.02cm"/></style:columns></style:section-properties></style:style>
        </office:automatic-styles><office:body><office:text>
        <text:p>Annual summary</text:p><text:p>Opening text of the first part.</text:p>
        <text:p text:style-name="PWide">Volumes table</text:p>
        <table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell></table:table-row></table:table>
        <text:section text:name="S" text:style-name="Cols"><text:p>Column text one.</text:p><text:p>Column text two.</text:p></text:section>
        <text:p text:style-name="PBack">Closing words</text:p><text:p>Last line.</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = craft_zip::Writer::new();
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", &content), ("styles.xml", &styles)] {
        z.add(name, data.as_bytes(), craft_zip::Compression::Stored).unwrap();
    }
    z.finish().unwrap()
}

fn document_xml(docx: &[u8]) -> String {
    let mut zip = craft_zip::Archive::open(docx, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 }).unwrap();

    String::from_utf8(zip.read("word/document.xml", 16 << 20).unwrap().unwrap()).unwrap()
}

#[test]
fn odt_sections_convert_to_one_docx_section_each() {
    let doc = io::open_bytes("sections.odt", &odt()).unwrap();
    assert_eq!(doc.sections().len(), 4);
    let xml = document_xml(&io::save_bytes("sections.docx", &doc).unwrap());
    let secs: Vec<&str> = xml.split("<w:sectPr").skip(1).map(|s| s.split("</w:sectPr>").next().unwrap_or("")).collect();
    assert_eq!(secs.len(), 4, "{xml}");
    let orient: Vec<bool> = secs.iter().map(|s| s.contains(r#"w:orient="landscape""#)).collect();
    assert_eq!(orient, [false, true, true, false]);
    // A4 both ways.
    assert!(secs[0].contains(r#"w:w="11906""#) && secs[0].contains(r#"w:h="16838""#), "{}", secs[0]);
    assert!(secs[1].contains(r#"w:w="16838""#) && secs[1].contains(r#"w:h="11906""#), "{}", secs[1]);
    // Section 3: continuous, two columns with a line between; the others one column.
    assert!(secs[2].contains(r#"<w:type w:val="continuous""#), "{}", secs[2]);
    assert!(secs[2].contains(r#"w:num="2""#) && secs[2].contains(r#"w:sep="1""#), "{}", secs[2]);
    for s in [secs[0], secs[1], secs[3]] {
        assert!(!s.contains(r#"w:num="2""#) && !s.contains("continuous"), "{s}");
    }
    // Numbering restarts at 1 in sections 2 and 4 only.
    let restarts: Vec<bool> = secs.iter().map(|s| s.contains(r#"w:start="1""#)).collect();
    assert_eq!(restarts, [false, true, false, true]);
}
