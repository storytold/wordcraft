//! Right-to-left and complex-script OOXML: `w:bidi` paragraphs, `w:rtl` / `w:cs` runs, the
//! complex-script font, size, bold and italic, and the bidi language. Fixtures are hand-written
//! from ECMA-376 Part 1 §17.3.1.6 (bidi), §17.3.2 (run properties).

use std::io::{Read, Write};

use wordcraft_doc::props::{Align, CharProps, ParaProps};
use wordcraft_doc::{Document, Paragraph, para_block};

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
