//! Right-to-left and complex-script OOXML: `w:bidi` paragraphs, `w:rtl` / `w:cs` runs, the
//! complex-script font, size, bold and italic, and the bidi language. Fixtures are hand-written
//! from ECMA-376 Part 1 §17.3.1.6 (bidi), §17.3.2 (run properties).
//! Table direction (#66) round-trips as `w:bidiVisual` (ECMA-376 §17.4.1) with logical cell order.

use std::io::{Read, Write};

use wordcraft_doc::props::{Align, CharProps, ParaProps};
use wordcraft_doc::{Block, Document, Paragraph, Table, para_block};

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A package whose body is `body`.
fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())])
}

fn paras(d: &Document) -> Vec<&Paragraph> {
    d.body.iter().filter_map(|b| b.as_para()).collect()
}

/// `word/document.xml` of a written package.
fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

/// Persian text with a ZWNJ (نیم‌فاصله), Persian digits, Latin and punctuation.
const FA: &str = "می‌خواهم نسخهٔ ۲٫۵ از WordCraft را (امروز) نصب کنم.";

fn persian_run() -> CharProps {
    CharProps {
        rtl: Some(true),
        font_cs: Some("B Nazanin".into()),
        size_cs: Some(14.0),
        bold_cs: Some(true),
        italic_cs: Some(false),
        lang_bidi: Some("fa-IR".into()),
        ..Default::default()
    }
}

#[test]
fn persian_paragraph_round_trips() {
    let mut rtl = Paragraph::with_text(FA, persian_run());
    rtl.props = ParaProps { bidi: Some(true), align: Some(Align::Justify), ..Default::default() };
    let ltr = Paragraph::with_text("English with فارسی inside", CharProps { cs: Some(true), ..Default::default() });
    let mut d = Document::new();
    d.body = vec![para_block(rtl.clone()), para_block(ltr.clone())];
    let bytes = wordcraft_docx::write(&d).unwrap();
    let r = wordcraft_docx::read(&bytes).unwrap();
    let ps = paras(&r);
    assert_eq!(ps[0].text, FA, "Persian text, ZWNJ and digits survive byte for byte");
    assert_eq!(ps[0].props.bidi, Some(true));
    assert_eq!(ps[0].props.align, Some(Align::Justify));
    assert_eq!(ps[0].runs[0].props, persian_run());
    assert_eq!(ps[1].text, ltr.text);
    assert_eq!(ps[1].props.bidi, None);
    assert_eq!(ps[1].runs[0].props.cs, Some(true));
}

#[test]
fn bidi_markup_is_written_in_schema_order() {
    let mut p =
        Paragraph::with_text(FA, CharProps { bold: Some(true), italic: Some(true), size: Some(12.0), font: Some("Arial".into()), ..persian_run() });
    p.props.bidi = Some(true);
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains("<w:bidi/>"), "paragraph direction: {xml}");
    assert!(xml.contains(r#"w:cs="B Nazanin""#), "complex-script font");
    assert!(xml.contains(r#"<w:szCs w:val="28"/>"#), "complex-script size in half points");
    assert!(xml.contains(r#"w:bidi="fa-IR""#), "bidi language");
    // CT_RPr is a sequence: b, bCs, i, iCs … sz, szCs … rtl, cs, lang.
    let pos = |t: &str| xml.find(t).unwrap_or_else(|| panic!("{t} missing: {xml}"));
    assert!(pos("<w:rFonts") < pos("<w:b/>"));
    assert!(pos("<w:b/>") < pos("<w:bCs/>") && pos("<w:bCs/>") < pos("<w:i/>"));
    assert!(pos("<w:i/>") < pos(r#"<w:iCs w:val="0"/>"#));
    assert!(pos(r#"<w:sz w:val="24"/>"#) < pos("<w:szCs"));
    assert!(pos("<w:rtl/>") < pos("<w:lang"));
}

#[test]
fn reads_word_style_bidi_markup() {
    // What a right-to-left paragraph looks like in a Word document: w:bidi, logical
    // alignment and indents (left = start), runs with w:rtl and the complex-script values.
    let body = r#"
<w:p>
  <w:pPr><w:bidi/><w:ind w:left="720" w:right="0"/><w:jc w:val="left"/></w:pPr>
  <w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:cs="B Lotus"/><w:b/><w:bCs/><w:sz w:val="24"/><w:szCs w:val="32"/><w:rtl/><w:lang w:val="en-US" w:bidi="fa-IR"/></w:rPr><w:t>سلام دنیا</w:t></w:r>
  <w:r><w:rPr><w:rtl/></w:rPr><w:t xml:space="preserve"> </w:t></w:r>
  <w:r><w:t>Word 2025</w:t></w:r>
</w:p>
<w:p><w:pPr><w:bidi w:val="0"/></w:pPr><w:r><w:rPr><w:cs/><w:iCs/></w:rPr><w:t>x</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let ps = paras(&d);
    assert_eq!(ps[0].text, "سلام دنیا Word 2025");
    assert_eq!(ps[0].props.bidi, Some(true));
    assert_eq!(ps[0].props.indent_left, Some(36.0), "w:left is the start indent");
    assert_eq!(ps[0].props.align, Some(Align::Left), "jc=left is the start edge");
    let fa = &ps[0].runs[0].props;
    assert_eq!(fa.font.as_deref(), Some("Times New Roman"));
    assert_eq!(fa.font_cs.as_deref(), Some("B Lotus"));
    assert_eq!((fa.size, fa.size_cs), (Some(12.0), Some(16.0)));
    // b + bCs alike is stored once: an unset complex-script bold is the same as `bold`.
    assert_eq!((fa.bold, fa.bold_cs, fa.rtl), (Some(true), None, Some(true)));
    assert_eq!((fa.lang.as_deref(), fa.lang_bidi.as_deref()), (Some("en-US"), Some("fa-IR")));
    assert_eq!(ps[1].props.bidi, Some(false));
    let x = &ps[1].runs[0].props;
    assert_eq!((x.cs, x.italic_cs, x.italic), (Some(true), Some(true), None));
    // The resolved complex-script formatting is what Persian characters are drawn with.
    let rc = d.styles.resolve_char(None, fa);
    let cs = rc.complex();
    assert_eq!((cs.font.as_str(), cs.size, cs.bold), ("B Lotus", 16.0, true));
    assert!(rc.uses_complex('س') && rc.uses_complex('W'), "every character of an rtl run is complex script");
}

#[test]
fn kashida_modes_read_distinctly_and_round_trip() {
    // Word's three kashida justification modes stay distinct; plain justification is untouched.
    let body = r#"
<w:p><w:pPr><w:bidi/><w:jc w:val="lowKashida"/></w:pPr><w:r><w:t>أ</w:t></w:r></w:p>
<w:p><w:pPr><w:jc w:val="mediumKashida"/></w:pPr><w:r><w:t>b</w:t></w:r></w:p>
<w:p><w:pPr><w:jc w:val="highKashida"/></w:pPr><w:r><w:t>c</w:t></w:r></w:p>
<w:p><w:pPr><w:jc w:val="both"/></w:pPr><w:r><w:t>d</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let ps = paras(&d);
    assert_eq!(ps.len(), 4);
    assert_eq!((ps[0].props.align, ps[0].props.kashida), (Some(Align::Justify), Some(wordcraft_doc::props::Kashida::Low)));
    assert_eq!((ps[1].props.align, ps[1].props.kashida), (Some(Align::Justify), Some(wordcraft_doc::props::Kashida::Medium)));
    assert_eq!((ps[2].props.align, ps[2].props.kashida), (Some(Align::Justify), Some(wordcraft_doc::props::Kashida::High)));
    assert_eq!((ps[3].props.align, ps[3].props.kashida), (Some(Align::Justify), None));
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    for mode in ["lowKashida", "mediumKashida", "highKashida"] {
        assert!(xml.contains(&format!(r#"w:val="{mode}""#)), "mode kept: {mode}");
    }
    let r = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(paras(&r).iter().map(|p| p.props.kashida).collect::<Vec<_>>(), ps.iter().map(|p| p.props.kashida).collect::<Vec<_>>());
}

#[test]
fn hostile_bidi_markup_is_bounded() {
    let long = "x".repeat(10_000);
    let body = format!(
        r#"<w:p><w:pPr><w:bidi w:val="banana"/></w:pPr><w:r><w:rPr><w:rFonts w:cs="{long}"/><w:szCs w:val="-5"/><w:szCs w:val="99999999"/><w:lang w:bidi="{long}"/><w:rtl w:val="maybe"/></w:rPr><w:t>ok</w:t></w:r></w:p>"#
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let c = &paras(&d)[0].runs[0].props;
    assert_eq!(c.font_cs, None, "absurd font names are dropped");
    assert_eq!(c.lang_bidi, None);
    assert!(c.size_cs.is_none_or(|s| (1.0..=1638.0).contains(&s)));
}

/// A two-column table with Arabic and English cells, right to left or not.
fn two_col_table(rtl: bool) -> Table {
    let mut t = Table::new(1, 2, 200.0);
    t.props.rtl = rtl;
    let mut ar = Paragraph::with_text("العمود الأول", CharProps { rtl: Some(true), ..Default::default() });
    ar.props.bidi = Some(true);
    t.rows[0].cells[0].blocks = vec![para_block(ar)];
    t.rows[0].cells[1].blocks = vec![para_block(Paragraph::with_text("second column", Default::default()))];
    t
}

fn tables(d: &Document) -> Vec<&Table> {
    d.body.iter().filter_map(|b| b.as_table()).collect()
}

#[test]
fn rtl_table_direction_round_trips_with_logical_cell_order() {
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("before", Default::default())), Block::Table(two_col_table(true)).into()];
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains("<w:bidiVisual/>"), "table direction is written: {xml}");
    let r = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    let ts = tables(&r);
    assert_eq!(ts.len(), 1);
    assert!(ts[0].props.rtl, "bidiVisual survives the round trip");
    let texts: Vec<String> =
        ts[0].rows[0].cells.iter().map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect()).collect();
    assert_eq!(texts, ["العمود الأول", "second column"], "logical cell order is never reversed");
    assert_eq!(ts[0].rows[0].cells[0].blocks[0].as_para().unwrap().props.bidi, Some(true), "cell text direction is independent");
    assert_eq!(ts[0].rows[0].cells[1].blocks[0].as_para().unwrap().props.bidi, None);
}

#[test]
fn ltr_tables_write_no_bidi_visual() {
    let mut d = Document::new();
    d.body = vec![Block::Table(two_col_table(false)).into()];
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(!xml.contains("bidiVisual"), "no direction element for LTR tables: {xml}");
    let r = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert!(!tables(&r)[0].props.rtl);
}

#[test]
fn bidi_visual_markup_is_read_in_schema_order() {
    // What a right-to-left table looks like in a Word document: `w:bidiVisual` between the
    // floating placement and the width (ECMA-376 §17.4.1), as an on/off element.
    let body = r#"
<w:tbl>
  <w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblpPr w:horzAnchor="margin" w:vertAnchor="text" w:tblpX="10" w:tblpY="10"/><w:tblOverlap w:val="never"/><w:bidiVisual/><w:tblW w:w="4000" w:type="dxa"/><w:jc w:val="right"/></w:tblPr>
  <w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>
  <w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:pPr><w:bidi/></w:pPr><w:r><w:t>أ</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
<w:tbl>
  <w:tblPr><w:tblW w:w="0" w:type="auto"/><w:bidiVisual w:val="0"/></w:tblPr>
  <w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>
  <w:tr><w:tc><w:tcPr><w:tcW w:w="0" w:type="auto"/></w:tcPr><w:p><w:r><w:t>x</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let ts = tables(&d);
    assert_eq!(ts.len(), 2);
    assert!(ts[0].props.rtl, "bare bidiVisual means right to left");
    assert_eq!(ts[0].props.align, Some(Align::Right));
    assert!(ts[0].props.float.is_some() && !ts[0].props.float.unwrap().overlap, "placement around it still parses");
    assert_eq!(ts[0].rows[0].cells[0].blocks[0].as_para().unwrap().text, "أ");
    assert!(!ts[1].props.rtl, "val=0 means left to right");
    // Our writer keeps schema order: style, placement, bidiVisual, width.
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    let pos = |t: &str| xml.find(t).unwrap_or_else(|| panic!("{t} missing: {xml}"));
    assert!(pos("<w:tblStyle") < pos("<w:bidiVisual/>") && pos("<w:bidiVisual/>") < pos("<w:tblW"));
}

#[test]
fn rtl_hyperlinks_and_bookmarks_round_trip() {
    // Links, bookmarks and their anchors survive in RTL paragraphs with logical text.
    let body = r#"
<w:p><w:pPr><w:bidi/></w:pPr>
<w:r><w:rPr><w:rtl/></w:rPr><w:t>انظر</w:t></w:r>
<w:bookmarkStart w:id="7" w:name="مقطع"/><w:r><w:rPr><w:rtl/></w:rPr><w:t xml:space="preserve"> </w:t></w:r>
<w:hyperlink r:id="rId9" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:r><w:rPr><w:rtl/></w:rPr><w:t>الرابط</w:t></w:r></w:hyperlink>
<w:bookmarkEnd w:id="7"/></w:p>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.test/" TargetMode="External"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let bytes =
        zip(&[("_rels/.rels", rels.as_bytes()), ("word/document.xml", doc.as_bytes()), ("word/_rels/document.xml.rels", doc_rels.as_bytes())]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    let ps = paras(&d);
    // Bookmark markers live in the text as placeholders; the words stay logical.
    assert!(ps[0].text.contains("انظر") && ps[0].text.contains("الرابط"), "{:?}", ps[0].text);
    let links: Vec<_> = ps[0].runs.iter().filter_map(|r| r.props.link.clone()).collect();
    assert_eq!(links, ["https://example.test/"]);
    assert!(d.bookmarks().iter().any(|(n, _)| n == "مقطع"), "bookmark kept");
    let r = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert!(paras(&r)[0].text.contains("انظر") && paras(&r)[0].text.contains("الرابط"));
    assert_eq!(paras(&r)[0].props.bidi, Some(true));
    assert!(r.bookmarks().iter().any(|(n, _)| n == "مقطع"));
}
