//! Hand-written minimal WordprocessingML fixtures (written for these tests from the spec).

use std::io::Write;

use wordcraft_doc::para::{Anchor, FloatAlign, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{Align, Border, BorderStyle, Rgb, TextColor, VMerge};
use wordcraft_doc::{Block, Document, InlineObject, Paragraph, RevisionKind};

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

pub fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn rels(list: &[(&str, &str, &str)]) -> String {
    let mut s = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in list {
        let ext = if target.starts_with("http") { r#" TargetMode="External""# } else { "" };
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"{ext}/>"#
        ));
    }
    s.push_str("</Relationships>");
    s
}

/// A package with `body` as the document body and extra (path, content) parts.
fn docx(body: &str, doc_rels: &[(&str, &str, &str)], extra: &[(&str, &str)]) -> Vec<u8> {
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let r = rels(doc_rels);
    let mut entries: Vec<(&str, &[u8])> =
        vec![("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes()), ("word/_rels/document.xml.rels", r.as_bytes())];
    for (p, c) in extra {
        entries.push((p, c.as_bytes()));
    }
    zip(&entries)
}

fn read_body(body: &str) -> Document {
    wordcraft_docx::read(&docx(body, &[], &[])).unwrap()
}

#[test]
fn paragraph_default_presence_survives_repeated_saves() {
    // Omission leaves paragraph defaults application-defined (ISO 29500 pPrDefault).
    // An explicitly empty container must therefore remain distinct from omission.
    for (defaults, present, after) in [
        ("", false, None),
        ("<w:docDefaults/>", false, None),
        ("<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault></w:docDefaults>", false, None),
        ("<w:docDefaults><w:pPrDefault/></w:docDefaults>", true, None),
        ("<w:docDefaults><w:pPrDefault><w:pPr/></w:pPrDefault></w:docDefaults>", true, None),
        ("<w:docDefaults><w:pPrDefault><w:pPr><w:spacing w:after=\"160\"/></w:pPr></w:pPrDefault></w:docDefaults>", true, Some(8.0)),
    ] {
        let styles = format!(
            r#"<w:styles {W_NS}>{defaults}<w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style></w:styles>"#
        );
        let mut d = wordcraft_docx::read(&docx(
            "<w:p><w:r><w:t>First</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p>",
            &[("rId1", "styles", "styles.xml")],
            &[("word/styles.xml", &styles)],
        ))
        .unwrap();
        for _ in 0..2 {
            let bytes = wordcraft_docx::write(&d).unwrap();
            let xml = text(&unzip(&bytes), "word/styles.xml");
            assert_eq!(xml.contains("<w:pPrDefault>"), present, "{defaults}");
            d = wordcraft_docx::read(&bytes).unwrap();
            assert_eq!(d.styles.default_para.space_after, after);
            assert_eq!(paras(&d).iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
        }
    }
}

#[test]
fn omitted_paragraph_defaults_can_be_edited() {
    let styles =
        format!(r#"<w:styles {W_NS}><w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style></w:styles>"#);
    let mut d = wordcraft_docx::read(&docx("<w:p/>", &[("rId1", "styles", "styles.xml")], &[("word/styles.xml", &styles)])).unwrap();
    assert!(!d.styles.default_para_present);
    d.styles.default_para.space_after = Some(12.0);
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert!(again.styles.default_para_present);
    assert_eq!(again.styles.default_para.space_after, Some(12.0));
    let new = Document::new();
    assert_eq!(wordcraft_docx::read(&wordcraft_docx::write(&new).unwrap()).unwrap().styles.default_para, new.styles.default_para);
}

fn paras(d: &Document) -> Vec<&Paragraph> {
    d.body.iter().filter_map(|b| b.as_para()).collect()
}

#[test]
fn single_entry_toc_keeps_formatted_result_and_nested_page_field() {
    let d = read_body(
        r#"
<w:p><w:pPr><w:pStyle w:val="TOCHeading"/></w:pPr><w:r><w:t>Contents</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="TOC1"/></w:pPr>
 <w:r><w:fldChar w:fldCharType="begin" w:fldLock="true"/></w:r>
 <w:r><w:instrText> TOC \o "1-1" \h </w:instrText></w:r>
 <w:r><w:fldChar w:fldCharType="separate"/></w:r>
 <w:hyperlink w:anchor="_Toc1"><w:r><w:rPr><w:b/></w:rPr><w:t>Intro</w:t></w:r>
 <w:r><w:tab/></w:r><w:fldSimple w:instr=" PAGEREF _Toc1 \h "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:hyperlink>
 <w:r><w:fldChar w:fldCharType="end"/></w:r>
</w:p>"#,
    );
    let p = paras(&d);
    assert_eq!(p[0].plain_text(), "Contents");
    assert_eq!(
        p[0].objects,
        vec![InlineObject::Field {
            instr: r#"TOC \o "1-1" \h"#.into(),
            result: String::new(),
            locked: true,
            dirty: false,
            code: false,
            form: String::new()
        }]
    );
    assert_eq!(p[1].plain_text(), "Intro\t1");
    assert_eq!(p[1].props_of_char(0).bold, Some(true));
    assert_eq!(p[1].props_of_char(0).link.as_deref(), Some("#_Toc1"));
    assert_eq!(
        p[1].objects,
        vec![InlineObject::Field {
            instr: r#"PAGEREF _Toc1 \h"#.into(),
            result: "1".into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new()
        }]
    );
}

#[test]
fn toc_complex_field_spanning_paragraphs() {
    let body = r#"
<w:p><w:pPr><w:pStyle w:val="TOC1"/></w:pPr>
 <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>
 <w:hyperlink w:anchor="_Toc1"><w:r><w:t>Intro</w:t></w:r><w:r><w:tab/></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGEREF _Toc1 \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:hyperlink>
</w:p>
<w:p><w:hyperlink w:anchor="_Toc2"><w:r><w:t>Body</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>2</w:t></w:r></w:hyperlink></w:p>
<w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:p><w:bookmarkStart w:id="0" w:name="_Toc1"/><w:r><w:t>Intro</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>
<w:p><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>7</w:t></w:r></w:fldSimple></w:p>"#;
    let d = read_body(body);
    let p = paras(&d);
    assert_eq!(p.len(), 5);
    assert_eq!(p[0].text, "\u{FFFC}Intro\t\u{FFFC}");
    assert_eq!(p[0].objects[0], InlineObject::FieldStart { instr: r#"TOC \o "1-3" \h \z \u"#.into(), locked: false, dirty: false });
    assert_eq!(
        p[0].objects[1],
        InlineObject::Field {
            instr: r#"PAGEREF _Toc1 \h"#.into(),
            result: "1".into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new()
        }
    );
    assert_eq!(p[0].props_of_char(3).link.as_deref(), Some("#_Toc1"));
    assert_eq!(p[0].props.style.as_deref(), Some("TOC1"));
    assert_eq!(p[1].text, "Body\t2");
    assert_eq!(p[1].props_of_char(0).link.as_deref(), Some("#_Toc2"));
    assert_eq!(p[2].objects, vec![InlineObject::FieldEnd]);
    assert_eq!(p[3].objects, vec![InlineObject::BookmarkStart { name: "_Toc1".into() }, InlineObject::BookmarkEnd { name: "_Toc1".into() }]);
    assert_eq!(p[4].plain_text(), "Page 7");
    assert_eq!(
        p[4].objects[0],
        InlineObject::Field { instr: "PAGE".into(), result: "7".into(), locked: false, dirty: false, code: false, form: String::new() }
    );
    // And it writes back out and reads the same.
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(again.body, d.body);
}

#[test]
fn hyperlink_runs_and_wrappers() {
    let body = r#"
<w:p>
 <w:hyperlink r:id="rId5" w:history="1"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>site</w:t></w:r></w:hyperlink>
 <w:hyperlink r:id="rId5" w:anchor="frag"><w:r><w:t>X</w:t></w:r></w:hyperlink>
 <w:smartTag><w:r><w:t>A</w:t></w:r></w:smartTag><w:customXml><w:r><w:t>B</w:t></w:r></w:customXml>
 <w:sdt><w:sdtPr/><w:sdtContent><w:r><w:t>C</w:t></w:r></w:sdtContent></w:sdt>
 <w:r><w:sym w:font="Wingdings" w:char="F04A"/><w:noBreakHyphen/><w:softHyphen/><w:cr/><w:br w:type="page"/><w:br w:type="column"/><w:t xml:space="preserve"> e&amp;</w:t></w:r>
 <w:proofErr w:type="spellStart"/><w:r><w:rPr><w:b w:val="false"/><w:i w:val="0"/><w:caps w:val="off"/><w:strike w:val="true"/><w:color w:val="auto"/></w:rPr><w:t>T</w:t></w:r>
</w:p>
<w:sdt><w:sdtContent><w:p><w:r><w:t>in block sdt</w:t></w:r></w:p></w:sdtContent></w:sdt>"#;
    let bytes = docx(body, &[("rId5", "hyperlink", "https://example.org/")], &[]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    let p = paras(&d);
    // The content control around "C" is kept (zero-width markers in the text).
    assert_eq!(p[0].plain_text(), "siteXABC\u{F04A}\u{2011}\u{AD}\n\u{C}\u{E} e&T");
    assert!(matches!(p[0].objects.first(), Some(InlineObject::ControlStart { .. })));
    assert_eq!(p[0].props_of_char(0).link.as_deref(), Some("https://example.org/"));
    assert_eq!(p[0].props_of_char(0).style.as_deref(), Some("Hyperlink"));
    assert_eq!(p[0].props_of_char(4).link.as_deref(), Some("https://example.org/#frag"));
    let sym_off = p[0].text.find('\u{F04A}').unwrap();
    assert_eq!(p[0].props_of_char(sym_off).font.as_deref(), Some("Wingdings"));
    let t = p[0].props_of_char(p[0].text.len() - 1);
    assert_eq!((t.bold, t.italic, t.caps, t.strike, t.color), (Some(false), Some(false), Some(false), Some(true), Some(TextColor::Auto)));
    assert_eq!(p[1].plain_text(), "in block sdt");
}

#[test]
fn symbol_font_text_reads_as_unicode_and_round_trips_unchanged() {
    // Wingdings stored both ways Word writes it: `w:sym` (private-use code) and a plain code in a
    // run whose font is Wingdings; plus Symbol and Webdings.
    let body = r#"
<w:p>
 <w:r><w:t xml:space="preserve">Done </w:t></w:r>
 <w:r><w:sym w:font="Wingdings" w:char="F0FC"/></w:r>
 <w:r><w:rPr><w:rFonts w:ascii="Wingdings" w:hAnsi="Wingdings"/></w:rPr><w:t>J</w:t></w:r>
 <w:r><w:sym w:font="Symbol" w:char="F061"/><w:sym w:font="Webdings" w:char="F061"/></w:r>
</w:p>"#;
    let d = wordcraft_docx::read(&docx(body, &[], &[])).unwrap();
    let p = paras(&d);
    // Stored as Word stores it…
    assert_eq!(p[0].plain_text(), "Done \u{F0FC}J\u{F061}\u{F061}");
    // …read (copied, exported as text) as the symbols shown.
    assert_eq!(p[0].shown_text(), "Done \u{2713}\u{263A}\u{3B1}\u{2714}");
    assert_eq!(d.shown_text(wordcraft_doc::StoryRef::Body), "Done \u{2713}\u{263A}\u{3B1}\u{2714}");
    // A round trip keeps the original characters and fonts.
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    let q = paras(&again);
    assert_eq!(q[0].plain_text(), p[0].plain_text());
    assert_eq!(q[0].shown_text(), p[0].shown_text());
}

#[test]
fn pcl6_symbol_roundtrip() {
    let body = r#"<w:p><w:r><w:sym w:font="Symbol (PCL6)" w:char="F0B3"/></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body, &[], &[])).unwrap();
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    for doc in [&d, &again] {
        let p = paras(doc)[0];
        assert_eq!(p.text, "\u{F0B3}");
        assert_eq!(p.props_of_char(0).font.as_deref(), Some("Symbol (PCL6)"));
        assert_eq!(p.shown_text(), "≥");
    }
}

#[test]
fn alternate_content_choice_and_fallback() {
    let body = r#"
<w:p><w:r><mc:AlternateContent>
  <mc:Choice Requires="wps"><w:drawing><wp:anchor behindDoc="0" distL="12700"><wp:positionH relativeFrom="page"><wp:posOffset>127000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="914400" cy="457200"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="1" name="Text Box 1"/>
    <a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:prstGeom prst="rect"/></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>boxed</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic>
  </wp:anchor></w:drawing></mc:Choice>
  <mc:Fallback><w:pict><v:shape style="width:72pt;height:36pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>old</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback>
</mc:AlternateContent></w:r></w:p>
<w:p><mc:AlternateContent><mc:Choice Requires="w99"><w:r><w:t>future</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>fallback text</w:t></w:r></mc:Fallback></mc:AlternateContent></w:p>"#;
    let d = read_body(body);
    let p = paras(&d);
    match &p[0].objects[0] {
        InlineObject::Shape { kind, w, h, float, story, .. } => {
            assert_eq!((*kind, *w, *h), (ShapeKind::TextBox, 72.0, 36.0));
            assert_eq!(float.wrap, Wrap::Square);
            assert_eq!(float.x, 10.0);
            assert_eq!(float.dist, 1.0);
            let s = d.parts.get(&story.unwrap()).unwrap();
            assert_eq!(s.blocks[0].as_para().unwrap().text, "boxed");
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(p[1].text, "fallback text");
}

#[test]
fn table_look_bitmask_and_legacy_merges() {
    let body = r#"
<w:tbl>
 <w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="5000" w:type="pct"/><w:jc w:val="center"/><w:tblLook w:val="04A0"/></w:tblPr>
 <w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>
 <w:tr><w:tc><w:tcPr><w:hMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>wide</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:hMerge/></w:tcPr><w:p/></w:tc><w:tc><w:tcPr><w:vMerge w:val="restart"/><w:shd w:val="clear" w:fill="FF0000"/></w:tcPr><w:p/></w:tc></w:tr>
 <w:tr><w:trPr><w:trHeight w:val="400"/></w:trPr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p/></w:tc><w:sdt><w:sdtContent><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:sdtContent></w:sdt></w:tr>
</w:tbl>
<w:tbl><w:tblPr><w:tblLook w:firstRow="0" w:lastRow="1" w:firstColumn="1" w:lastColumn="0" w:noHBand="1" w:noVBand="0"/></w:tblPr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>
<w:p/>"#;
    let d = read_body(body);
    let Block::Table(t) = &*d.body[0] else { panic!() };
    assert_eq!(t.props.width_pct, Some(100.0));
    assert_eq!(t.props.align, Some(Align::Center));
    let l = t.props.look;
    assert!(l.header_row && l.first_column && l.banded_rows && !l.banded_columns && !l.total_row && !l.last_column);
    assert_eq!(t.rows[0].cells.len(), 2);
    assert_eq!(t.rows[0].cells[0].span(), 2);
    assert_eq!(t.rows[0].cells[1].props.vmerge, VMerge::Restart);
    assert_eq!(t.rows[1].cells[1].props.vmerge, VMerge::Continue);
    assert_eq!(t.rows[1].props.height, Some(20.0));
    assert_eq!(t.grid, vec![100.0, 100.0, 100.0]);
    let Block::Table(t2) = &*d.body[1] else { panic!() };
    let l = t2.props.look;
    assert!(!l.header_row && l.total_row && l.first_column && !l.last_column && !l.banded_rows && l.banded_columns);
}

#[test]
fn styles_numbering_settings_notes_comments_from_parts() {
    let styles = format!(
        r#"<w:styles {W_NS}>
 <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>
 <w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:qFormat/></w:style>
 <w:style w:type="paragraph" w:styleId="1"><w:name w:val="heading 1"/><w:basedOn w:val="a"/><w:next w:val="a"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi"/><w:b/><w:sz w:val="32"/></w:rPr></w:style>
 <w:style w:type="paragraph" w:customStyle="1" w:styleId="Fancy"><w:name w:val="Fancy"/><w:basedOn w:val="Fancy"/></w:style>
 <w:style w:type="table" w:styleId="Grid"><w:name w:val="Grid"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4"/></w:tblBorders></w:tblPr><w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr><w:tcPr><w:shd w:val="clear" w:fill="4472C4"/></w:tcPr></w:tblStylePr></w:style>
</w:styles>"#
    );
    let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="T"><a:themeElements><a:clrScheme name="T"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:accent1><a:srgbClr val="4472C4"/></a:accent1></a:clrScheme><a:fontScheme name="T"><a:majorFont><a:latin typeface="Major Face"/></a:majorFont><a:minorFont><a:latin typeface="Minor Face"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;
    let numbering = format!(
        r#"<w:numbering {W_NS}><w:abstractNum w:abstractNumId="7"><w:name w:val="L"/><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="upperLetter"/><w:lvlText w:val="%1)"/><w:lvlJc w:val="right"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/><w:suff w:val="space"/><w:lvlText w:val="o"/><w:rPr><w:rFonts w:ascii="Courier New"/></w:rPr></w:lvl></w:abstractNum><w:num w:numId="2"><w:abstractNumId w:val="7"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="5"/></w:lvlOverride></w:num></w:numbering>"#
    );
    let settings = format!(
        r#"<w:settings {W_NS}><w:trackRevisions/><w:defaultTabStop w:val="708"/><w:evenAndOddHeaders w:val="1"/><w:footnotePr><w:pos w:val="sideways"/><w:numFmt w:val="lowerRoman"/><w:numStart w:val="-99999999999"/><w:numRestart w:val="eachPage"/></w:footnotePr></w:settings>"#
    );
    let footnotes = format!(
        r#"<w:footnotes {W_NS}><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> Note text</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    let comments = format!(
        r#"<w:comments {W_NS}><w:comment w:id="5" w:author="Zed" w:initials="Z" w:date="2026-01-01T00:00:00Z"><w:p><w:r><w:annotationRef/></w:r><w:r><w:t>Look</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let header = format!(r#"<w:hdr {W_NS}><w:p><w:r><w:t>H</w:t></w:r></w:p></w:hdr>"#);
    let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Doc Title</dc:title><cp:revision>3</cp:revision></cp:coreProperties>"#;
    let body = r#"
<w:p><w:pPr><w:pStyle w:val="1"/><w:numPr><w:ilvl w:val="1"/><w:numId w:val="2"/></w:numPr></w:pPr><w:commentRangeStart w:id="5"/><w:r><w:rPr><w:rFonts w:asciiTheme="majorHAnsi"/></w:rPr><w:t>Heading</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r><w:r><w:commentReference w:id="5"/></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="a"/></w:pPr><w:ins w:id="1" w:author="I" w:date="2026-01-01T00:00:00Z"><w:r><w:t>new</w:t></w:r></w:ins><w:del w:id="2" w:author="I"><w:r><w:delText>old</w:delText></w:r></w:del></w:p>
<w:sectPr><w:headerReference w:type="default" r:id="rId9"/><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/><w:cols w:space="708"/><w:titlePg w:val="0"/></w:sectPr>"#;
    let doc_rels = [
        ("rId1", "styles", "styles.xml"),
        ("rId2", "theme", "theme/theme1.xml"),
        ("rId3", "numbering", "numbering.xml"),
        ("rId4", "settings", "/word/settings.xml"),
        ("rId5", "footnotes", "footnotes.xml"),
        ("rId6", "comments", "comments.xml"),
        ("rId9", "header", "header1.xml"),
    ];
    let root_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#;
    let doc = format!(r#"<w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let r = rels(&doc_rels);
    let bytes = zip(&[
        ("_rels/.rels", root_rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", r.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
        ("word/theme/theme1.xml", theme.as_bytes()),
        ("word/numbering.xml", numbering.as_bytes()),
        ("word/settings.xml", settings.as_bytes()),
        ("word/footnotes.xml", footnotes.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
        ("word/header1.xml", header.as_bytes()),
        ("docProps/core.xml", core.as_bytes()),
    ]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    // Styles: localized default id renamed to Normal; display names mapped.
    let h = d.styles.get("1").unwrap();
    assert_eq!(h.name, "Heading 1");
    assert_eq!(h.based_on.as_deref(), Some("Normal"));
    assert!(h.builtin && h.quick);
    assert_eq!(h.chr.font.as_deref(), Some("Major Face"));
    assert!(d.styles.get("Normal").is_some());
    assert_eq!(d.styles.get("Fancy").unwrap().based_on, None, "self-reference dropped");
    assert!(!d.styles.get("Fancy").unwrap().builtin);
    assert_eq!(d.styles.default_chr.font.as_deref(), Some("Minor Face"));
    assert_eq!(d.styles.default_chr.size, Some(11.0));
    let g = d.styles.get("Grid").unwrap().table.as_ref().unwrap();
    assert_eq!(g.header_fill, Some(wordcraft_doc::Rgb(0x44, 0x72, 0xC4)));
    assert_eq!(g.header_chr.bold, Some(true));
    assert_eq!(d.settings.theme_colors[4], wordcraft_doc::Rgb(0x44, 0x72, 0xC4));
    assert_eq!(d.settings.major_font, "Major Face");
    // Numbering.
    let lv = d.numbering.level(2, 0).unwrap();
    assert_eq!((lv.start, lv.text.as_str(), lv.indent, lv.hanging, lv.align), (3, "%1)", 36.0, 18.0, Align::Right));
    let lv1 = d.numbering.level(2, 1).unwrap();
    assert_eq!(lv1.chr.font.as_deref(), Some("Courier New"));
    assert_eq!(lv1.suffix, wordcraft_doc::numbering::LevelSuffix::Space);
    assert_eq!(d.numbering.num(2).unwrap().start_overrides, vec![(0, 5)]);
    // Settings.
    assert!(d.settings.track_changes && d.settings.even_odd_headers);
    assert_eq!(d.settings.default_tab, 35.4);
    assert_eq!(d.settings.footnote_format, wordcraft_doc::section::NumFormat::LowerRoman);
    // A hostile start is clamped, an unknown position left out.
    let fp = d.settings.footnote_pr;
    assert_eq!((fp.pos, fp.num_start, fp.num_restart), (None, Some(1), Some(wordcraft_doc::section::NoteRestart::EachPage)));
    // Body.
    let p = paras(&d);
    assert_eq!(p[0].props.style.as_deref(), Some("1"));
    assert_eq!(p[0].props.numbering, Some(wordcraft_doc::props::NumRef { num: 2, level: 1 }));
    assert_eq!(p[0].props_of_char(4).font.as_deref(), Some("Major Face"));
    match &p[0].objects[..] {
        [InlineObject::CommentStart { id: 5 }, InlineObject::NoteRef { kind: NoteKind::Footnote, id, .. }, InlineObject::CommentEnd { id: 5 }] => {
            let note = d.parts.get(id).unwrap().blocks[0].as_para().unwrap();
            assert_eq!(note.text, format!("{} Note text", wordcraft_doc::para::OBJ));
            assert_eq!(note.objects, vec![InlineObject::NoteRef { kind: NoteKind::Footnote, id: *id, custom: String::new() }]);
        }
        o => panic!("{o:?}"),
    }
    let c = d.comments.get(&5).unwrap();
    assert_eq!((c.author.as_str(), c.initials.as_str()), ("Zed", "Z"));
    assert_eq!(d.parts.get(&c.part).unwrap().blocks[0].as_para().unwrap().text, "Look");
    assert_eq!(p[1].props.style.as_deref(), Some("Normal"));
    assert_eq!(p[1].text, "newold");
    assert_eq!(d.revisions.len(), 2);
    assert_eq!(p[1].props_of_char(0).ins, Some(0));
    assert_eq!(p[1].props_of_char(3).del, Some(1));
    // Section + header.
    let s = &d.last_section;
    assert!(s.landscape);
    assert_eq!((s.page_w, s.page_h), (841.9, 595.3));
    assert!(!s.title_page);
    let hid = s.headers.default.unwrap();
    assert_eq!(d.parts.get(&hid).unwrap().blocks[0].as_para().unwrap().text, "H");
    assert_eq!(d.core.title, "Doc Title");
    assert_eq!(d.core.revision, 3);
}

/// A table style's own pPr/rPr, its `wholeTable` region (which covers every cell) and its table
/// shading and cell margins; a derived style keeps only what it sets itself.
#[test]
fn table_style_whole_table_shading_and_margins() {
    let styles = format!(
        r#"<w:styles {W_NS}>
 <w:style w:type="table" w:styleId="Base"><w:name w:val="Base"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:tblPr><w:shd w:val="clear" w:color="auto" w:fill="EEEEEE"/><w:tblCellMar><w:top w:w="20" w:type="dxa"/><w:left w:w="288" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="288" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>
 <w:style w:type="table" w:styleId="Whole"><w:name w:val="Whole"/><w:basedOn w:val="Base"/><w:rPr><w:b/></w:rPr><w:tblStylePr w:type="wholeTable"><w:pPr><w:jc w:val="center"/></w:pPr><w:rPr><w:color w:val="1F4E79"/></w:rPr><w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="DDEBF7"/></w:tcPr></w:tblStylePr><w:tblStylePr w:type="bogus"/><w:tblStylePr/></w:style>
</w:styles>"#
    );
    let bytes = docx("<w:p/>", &[("rId1", "styles", "styles.xml")], &[("word/styles.xml", &styles)]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    let base = d.styles.get("Base").unwrap();
    let parts = base.table.as_ref().unwrap();
    assert_eq!(parts.fill, Some(wordcraft_doc::Rgb(0xEE, 0xEE, 0xEE)));
    assert_eq!(parts.cell_margins, Some([1.0, 14.4, 0.0, 14.4]));
    let whole = d.styles.get("Whole").unwrap();
    assert_eq!(whole.para.align, Some(Align::Center));
    assert_eq!(whole.chr.bold, Some(true));
    assert_eq!(whole.chr.color, Some(TextColor::Rgb(wordcraft_doc::Rgb(0x1F, 0x4E, 0x79))));
    assert_eq!(whole.table.as_ref().unwrap().fill, Some(wordcraft_doc::Rgb(0xDD, 0xEB, 0xF7)));
    assert_eq!(whole.table.as_ref().unwrap().cell_margins, None);
    let merged = d.styles.table_style("Whole").unwrap();
    assert_eq!(merged.para.space_after, Some(0.0), "from Base");
    assert_eq!(merged.para.align, Some(Align::Center));
    assert_eq!(merged.parts.fill, Some(wordcraft_doc::Rgb(0xDD, 0xEB, 0xF7)));
    assert_eq!(merged.parts.cell_margins, Some([1.0, 14.4, 0.0, 14.4]), "from Base");
}

fn gallery_ids(styles: &str) -> Vec<String> {
    let bytes = docx("<w:p/>", &[("rId1", "styles", "styles.xml")], &[("word/styles.xml", styles)]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    d.styles.gallery().iter().map(|s| s.id.clone()).collect()
}

/// LibreOffice writes no `w:qFormat`: the Styles gallery still shows Word's usual quick styles.
#[test]
fn styles_without_qformat_get_the_default_gallery() {
    let styles = format!(
        r#"<w:styles {W_NS}>
 <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
 <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style>
 <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/></w:style>
 <w:style w:type="character" w:styleId="Emphasis"><w:name w:val="Emphasis"/></w:style>
</w:styles>"#
    );
    let ids = gallery_ids(&styles);
    for id in ["Normal", "NoSpacing", "Heading1", "Heading2", "Title", "Subtitle"] {
        assert!(ids.iter().any(|i| i == id), "{id} in {ids:?}");
    }
}

/// A custom style called "Title" is not the built-in one: it stays out of the gallery, and the
/// built-in Title is not added over its name.
#[test]
fn custom_style_named_like_a_quick_builtin_is_not_quick() {
    let styles = format!(
        r#"<w:styles {W_NS}>
 <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
 <w:style w:type="paragraph" w:customStyle="1" w:styleId="MyTitle"><w:name w:val="Title"/></w:style>
</w:styles>"#
    );
    let ids = gallery_ids(&styles);
    assert!(ids.iter().any(|i| i == "Normal"), "{ids:?}");
    assert!(!ids.iter().any(|i| i == "MyTitle" || i == "Title"), "{ids:?}");
}

/// A file that uses `w:qFormat` (Word's own) keeps exactly the gallery it says.
#[test]
fn styles_with_qformat_keep_their_gallery() {
    let styles = format!(
        r#"<w:styles {W_NS}>
 <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
 <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style>
</w:styles>"#
    );
    assert_eq!(gallery_ids(&styles), vec!["Normal".to_string()]);
}

/// Word starts every note with a `w:footnoteRef` / `w:endnoteRef` mark: the note's own number.
/// It reads as a reference to the note itself, so the note text shows its number.
#[test]
fn note_marks_reference_their_own_note() {
    let note = |tag: &str, mark: &str, style: &str, id: u32, text: &str| {
        format!(
            r#"<w:{tag} w:id="{id}"><w:p><w:pPr><w:pStyle w:val="{style}Text"/></w:pPr><w:r><w:rPr><w:rStyle w:val="{style}Reference"/></w:rPr><w:{mark}/></w:r><w:r><w:t xml:space="preserve"> {text}</w:t></w:r></w:p></w:{tag}>"#
        )
    };
    let sep = |tag: &str| {
        format!(
            r#"<w:{tag} w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:{tag}><w:{tag} w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:{tag}>"#
        )
    };
    // The second footnote has a stray endnote mark, which is not its number.
    let footnotes = format!(
        r#"<w:footnotes {W_NS}>{}{}<w:footnote w:id="2"><w:p><w:r><w:endnoteRef/><w:t>Stray</w:t></w:r></w:p></w:footnote>{}</w:footnotes>"#,
        sep("footnote"),
        note("footnote", "footnoteRef", "Footnote", 1, "First."),
        note("footnote", "footnoteRef", "Footnote", 3, "Third."),
    );
    let endnotes = format!(r#"<w:endnotes {W_NS}>{}{}</w:endnotes>"#, sep("endnote"), note("endnote", "endnoteRef", "Endnote", 1, "End."));
    // A mark outside a note (here in the body) refers to nothing.
    let body = r#"<w:p><w:r><w:footnoteRef/><w:t>A</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r><w:r><w:footnoteReference w:id="2"/></w:r><w:r><w:footnoteReference w:id="3"/></w:r><w:r><w:endnoteReference w:id="1"/></w:r></w:p>"#;
    let bytes = docx(
        body,
        &[("rId1", "footnotes", "footnotes.xml"), ("rId2", "endnotes", "endnotes.xml")],
        &[("word/footnotes.xml", &footnotes), ("word/endnotes.xml", &endnotes)],
    );
    let d = wordcraft_docx::read(&bytes).unwrap();
    let p = paras(&d)[0];
    assert_eq!(p.text.chars().filter(|c| *c == wordcraft_doc::para::OBJ).count(), 4, "{:?}", p.objects);
    let refs: Vec<(NoteKind, u32)> = p
        .objects
        .iter()
        .map(|o| match o {
            InlineObject::NoteRef { kind, id, .. } => (*kind, *id),
            o => panic!("{o:?}"),
        })
        .collect();
    let first_para = |id: u32| d.parts.get(&id).unwrap().blocks[0].as_para().unwrap();
    for (kind, id) in [refs[0], refs[2], refs[3]] {
        let n = first_para(id);
        assert_eq!(n.objects, vec![InlineObject::NoteRef { kind, id, custom: String::new() }], "note {id}");
        assert!(n.text.starts_with(wordcraft_doc::para::OBJ), "{:?}", n.text);
        let style = if kind == NoteKind::Footnote { "FootnoteReference" } else { "EndnoteReference" };
        assert_eq!(n.props_of_char(0).style.as_deref(), Some(style));
    }
    assert_eq!(first_para(refs[0].1).plain_text(), " First.");
    assert_eq!(first_para(refs[3].1).plain_text(), " End.");
    let stray = first_para(refs[1].1);
    assert!(stray.objects.is_empty(), "{:?}", stray.objects);
    assert_eq!(stray.text, "Stray");
}

#[test]
fn empty_tbl_borders_elements_leave_sides_unset() {
    // Style TS: single borders on all six sides. Document: table styled TS with an empty
    // paired <w:tblBorders></w:tblBorders> and empty <w:tcBorders></w:tcBorders> per cell.
    // (Hand-written from ECMA-376 §17.4.39/§17.4.43; do not use files produced by Word.)
    let styles = format!(
        r#"<w:styles {W_NS}><w:style w:type="table" w:styleId="TS"><w:name w:val="TS"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="2" w:color="000000"/><w:left w:val="single" w:sz="2" w:color="000000"/><w:bottom w:val="single" w:sz="2" w:color="000000"/><w:right w:val="single" w:sz="2" w:color="000000"/><w:insideH w:val="single" w:sz="2" w:color="000000"/><w:insideV w:val="single" w:sz="2" w:color="000000"/></w:tblBorders></w:tblPr></w:style></w:styles>"#
    );
    let body = r#"<w:tbl><w:tblPr><w:tblStyle w:val="TS"/><w:tblBorders></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcBorders></w:tcBorders></w:tcPr><w:p/></w:tc><w:tc><w:tcPr><w:tcBorders></w:tcBorders></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#;
    let d = wordcraft_docx::read(&docx(body, &[("rId1", "styles", "styles.xml")], &[("word/styles.xml", &styles)])).unwrap();
    let t = d.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.props.borders, Some(wordcraft_doc::props::Borders::default()));
    assert_eq!(t.rows[0].cells[0].props.borders, Some(wordcraft_doc::props::Borders::default()));
    let parts = d.styles.get("TS").unwrap().table.clone().unwrap();
    let b = parts.borders.unwrap();
    assert!(b.top.is_some() && b.between.is_some() && b.inside_v.is_some());
}

#[test]
fn vml_image_and_inline_drawing() {
    let png: &[u8] = b"\x89PNG\r\n\x1a\nnot really a png";
    let body = r#"
<w:p><w:r><w:pict><v:shape style="width:1in;height:36pt" alt="vml pic"><v:imagedata r:id="rIdImg" o:title="t"/></v:shape></w:pict></w:r>
<w:r><w:drawing><wp:inline><wp:extent cx="1270000" cy="635000"/><wp:docPr id="3" name="P" descr="alt text"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rIdImg"/><a:srcRect l="10000"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>
<w:r><w:drawing><wp:inline><wp:extent cx="1" cy="1"/><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rIdMissing"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let doc = format!(r#"<w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let r = rels(&[("rIdImg", "image", "media/pic.png"), ("rIdMissing", "image", "media/nope.png")]);
    let bytes = zip(&[
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", r.as_bytes()),
        ("word/media/pic.png", png),
    ]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    let p = paras(&d);
    assert_eq!(p[0].objects.len(), 2);
    match (&p[0].objects[0], &p[0].objects[1]) {
        (InlineObject::Image { media: m1, w: w1, h: h1, alt: a1, .. }, InlineObject::Image { media: m2, w: w2, h: h2, alt: a2, crop, .. }) => {
            assert_eq!((m1.as_str(), *w1, *h1, a1.as_str()), ("pic.png", 72.0, 36.0, "vml pic"));
            assert_eq!((m2.as_str(), *w2, *h2, a2.as_str()), ("pic.png", 100.0, 50.0, "alt text"));
            assert_eq!(crop[0], 0.1);
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(d.media.get("pic.png").unwrap().as_slice(), png);
}

#[test]
fn chart_and_diagram_drawings_are_graphic_objects() {
    // A diagram's data holds a blip: the URI decides first, so no picture is read from it.
    let drawing = |uri: &str, data: &str| {
        format!(
            r#"<w:r><w:drawing><wp:inline><wp:extent cx="2743200" cy="1828800"/><wp:docPr id="5" name="G" descr="sales"/><a:graphic><a:graphicData uri="{uri}">{data}</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
        )
    };
    let body = format!(
        "<w:p>{}{}</w:p>",
        drawing("http://schemas.openxmlformats.org/drawingml/2006/chart", ""),
        drawing("http://schemas.openxmlformats.org/drawingml/2006/diagram", r#"<a:blip r:embed="rIdImg"/>"#)
    );
    let d = read_body(&body);
    let p = paras(&d);
    assert_eq!(p[0].objects.len(), 2);
    match (&p[0].objects[0], &p[0].objects[1]) {
        (InlineObject::Graphic { w: w1, h: h1, alt, graphic: g1, .. }, InlineObject::Graphic { graphic: g2, .. }) => {
            assert_eq!((*w1, *h1, alt.as_str()), (216.0, 144.0, "sales"));
            assert_eq!(g1.kind, wordcraft_doc::graphic::GraphicKind::Chart);
            assert_eq!(g2.kind, wordcraft_doc::graphic::GraphicKind::Diagram);
            assert!(g1.items.is_empty() && g2.items.is_empty());
        }
        o => panic!("{o:?}"),
    }
}

#[test]
fn a_chart_part_is_built_once_and_only_through_a_chart_relationship() {
    let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let drawing = |id: &str| {
        format!(
            r#"<w:r><w:drawing><wp:inline><wp:extent cx="2743200" cy="1828800"/><wp:docPr id="5" name="G"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="{id}"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
        )
    };
    let body = format!("<w:p>{}{}{}</w:p>", drawing("rIdChart"), drawing("rIdChart"), drawing("rIdNotChart"));
    let bytes = docx(
        &body,
        &[("rIdChart", "chart", "charts/chart1.xml"), ("rIdNotChart", "image", "charts/chart1.xml")],
        &[("word/charts/chart1.xml", chart)],
    );
    let d = wordcraft_docx::read(&bytes).unwrap();
    let graphics: Vec<_> = paras(&d)[0]
        .objects
        .iter()
        .filter_map(|o| if let InlineObject::Graphic { graphic, .. } = o { Some(graphic.clone()) } else { None })
        .collect();
    assert_eq!(graphics.len(), 3);
    assert!(!graphics[0].items.is_empty(), "the chart is drawn");
    assert!(std::sync::Arc::ptr_eq(&graphics[0], &graphics[1]), "one build for both references");
    assert!(graphics[2].items.is_empty(), "an image relationship is not a chart");
}

#[test]
fn strict_namespace_and_bad_numbers() {
    let doc = r#"<w:document xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main"><w:body>
<w:p><w:pPr><w:jc w:val="end"/><w:ind w:start="1in" w:hanging="abc"/><w:spacing w:before="-50" w:line="99999999999999999999" w:lineRule="exact"/><w:outlineLvl w:val="300"/><w:numPr><w:numId w:val="-4"/></w:numPr></w:pPr>
<w:r><w:rPr><w:sz w:val="NaN"/><w:w w:val="999999"/><w:position w:val="-1e30"/><w:color w:val="GGGGGG"/><w:highlight w:val="mauve"/><w:u w:val="squiggle"/></w:rPr><w:t>strict</w:t></w:r></w:p>
<w:tbl><w:tblGrid><w:gridCol w:w="-5"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="99999"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>
<w:sectPr><w:pgSz w:w="0" w:h="x"/><w:cols w:num="-3"/><w:pgNumType w:start="-1" w:fmt="klingon"/></w:sectPr>
</w:body></w:document>"#;
    let bytes = zip(&[("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes())]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    let p = paras(&d);
    assert_eq!(p[0].text, "strict");
    assert_eq!(p[0].props.align, Some(Align::Right));
    assert_eq!(p[0].props.indent_left, Some(72.0));
    assert_eq!(p[0].props.space_before, Some(0.0));
    assert_eq!(p[0].props.outline_level, Some(9));
    let c = p[0].props_of_char(0);
    assert_eq!(c.size, None);
    assert_eq!(c.scale, Some(600.0));
    assert_eq!(c.color, None);
    let Block::Table(t) = &*d.body[1] else { panic!() };
    assert_eq!(t.rows[0].cells[0].span(), 63);
    assert_eq!(d.last_section.page_w, 612.0);
    assert_eq!(d.last_section.columns.count, 1);
    // The document can be written back.
    wordcraft_docx::write(&d).unwrap();
}

#[test]
fn unbalanced_fields_and_markers() {
    let body = r#"
<w:p><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>a</w:t></w:r><w:bookmarkEnd w:id="77"/><w:commentRangeEnd w:id="3"/></w:p>
<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>IF </w:instrText></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>MERGEFIELD x</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>X</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:instrText> = "y"</w:instrText></w:r></w:p>
<w:bookmarkStart w:id="1" w:name="between"/>
<w:p><w:r><w:t>after</w:t></w:r></w:p>
<w:bookmarkEnd w:id="1"/>"#;
    let d = read_body(body);
    let p = paras(&d);
    assert_eq!(p[0].text, "a");
    assert_eq!(
        p[1].objects,
        vec![InlineObject::Field {
            instr: "IF X = \"y\"".into(),
            result: String::new(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new()
        }]
    );
    assert_eq!(p[2].objects, vec![InlineObject::BookmarkStart { name: "between".into() }, InlineObject::BookmarkEnd { name: "between".into() }]);
    assert_eq!(p[2].plain_text(), "after");
}

#[test]
fn field_code_spans_paragraphs() {
    let body = r#"
<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>IF</w:instrText></w:r></w:p>
<w:p><w:r><w:instrText> MERGEFIELD x</w:instrText><w:br/><w:instrText> = "y"</w:instrText></w:r></w:p>
<w:p><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:p><w:r><w:t>Visible</w:t></w:r></w:p>"#;
    let doc = read_body(body);
    let p = paras(&doc);
    assert_eq!(p.len(), 4);
    for code in &p[..2] {
        assert!(code.plain_text().is_empty());
        assert_eq!(code.mark.hidden, Some(true));
    }
    assert_eq!(p[2].mark.hidden, None);
    assert_eq!(p[3].plain_text(), "Visible");
    assert_eq!(p[3].mark.hidden, None);
}

#[test]
fn empty_field_results_keep_line() {
    let body = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/><w:instrText>PAGE</w:instrText><w:fldChar w:fldCharType="separate"/><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:p><w:r><w:fldChar w:fldCharType="begin"/><w:instrText>IF</w:instrText></w:r></w:p>
<w:p/>
<w:p><w:r><w:instrText> x = y</w:instrText><w:fldChar w:fldCharType="separate"/><w:t>Result</w:t><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let doc = read_body(body);
    let p = paras(&doc);
    assert_eq!(p[0].mark.hidden, None);
    assert!(matches!(&p[0].objects[0], InlineObject::Field { instr, .. } if instr == "PAGE"));
    assert_eq!(p[1].mark.hidden, Some(true));
    assert_eq!(p[2].mark.hidden, Some(true));
    assert_eq!(p[3].mark.hidden, None);
    assert_eq!(p[3].plain_text(), "Result");
}

#[test]
fn header_self_reference_and_escaping_targets() {
    let hdr = format!(
        r#"<w:hdr {W_NS}><w:p><w:pPr><w:sectPr><w:headerReference w:type="default" r:id="rId1"/></w:sectPr></w:pPr><w:r><w:t>loop</w:t></w:r></w:p></w:hdr>"#
    );
    let hdr_rels = rels(&[("rId1", "header", "header1.xml")]);
    let body = r#"<w:p><w:r><w:drawing><wp:inline><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rIdEvil"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/><w:headerReference w:type="even" r:id="rIdH"/><w:footerReference w:type="first" r:id="rIdNone"/></w:sectPr>"#;
    let doc = format!(r#"<w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let r = rels(&[("rIdH", "header", "header1.xml"), ("rIdEvil", "image", "../../../../etc/passwd"), ("rIdNone", "footer", "missing.xml")]);
    let bytes = zip(&[
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", r.as_bytes()),
        ("word/header1.xml", hdr.as_bytes()),
        ("word/_rels/header1.xml.rels", hdr_rels.as_bytes()),
    ]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(d.last_section.headers.default, d.last_section.headers.even);
    assert!(d.media.is_empty());
    let out = wordcraft_docx::write(&d).unwrap();
    wordcraft_docx::read(&out).unwrap();
}

/// A document whose settings part is `settings` (None: no settings part at all).
fn with_settings(settings: Option<&str>) -> Document {
    let body = "<w:p><w:r><w:t>x</w:t></w:r></w:p>";
    match settings {
        None => read_body(body),
        Some(inner) => {
            let s = format!(r#"<w:settings {W_NS}>{inner}</w:settings>"#);
            wordcraft_docx::read(&docx(body, &[("rId1", "settings", "settings.xml")], &[("word/settings.xml", &s)])).unwrap()
        }
    }
}

#[test]
fn table_contextual_spacing_compatibility_flag_is_read() {
    assert!(!with_settings(None).settings.allow_space_of_same_style_in_table);
    for (flag, expected) in [
        ("", false),
        (r#"<w:allowSpaceOfSameStyleInTable w:val="0"/>"#, false),
        (r#"<w:allowSpaceOfSameStyleInTable/>"#, true),
        (r#"<w:allowSpaceOfSameStyleInTable w:val="1"/>"#, true),
    ] {
        let settings = format!("<w:compat>{flag}</w:compat>");
        assert_eq!(with_settings(Some(&settings)).settings.allow_space_of_same_style_in_table, expected, "{flag}");
    }
}

#[test]
fn compatibility_mode_is_read_and_defaults_to_word_2007() {
    let compat = |v: &str| {
        format!(r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="{v}"/></w:compat>"#)
    };
    assert_eq!(with_settings(Some(&compat("15"))).settings.compat_mode, 15);
    assert_eq!(with_settings(Some(&compat("14"))).settings.compat_mode, 14);
    // Unnamed, out of range or junk values.
    assert_eq!(with_settings(Some("<w:compat/>")).settings.compat_mode, wordcraft_doc::LEGACY_COMPAT_MODE);
    assert_eq!(with_settings(None).settings.compat_mode, wordcraft_doc::LEGACY_COMPAT_MODE);
    assert_eq!(with_settings(Some(&compat("-3"))).settings.compat_mode, 11);
    assert_eq!(with_settings(Some(&compat("99999"))).settings.compat_mode, 99);
    assert_eq!(with_settings(Some(&compat("abc"))).settings.compat_mode, wordcraft_doc::LEGACY_COMPAT_MODE);
    // Another vendor's setting of the same name is ignored.
    let other = r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="urn:example" w:val="15"/></w:compat>"#;
    assert_eq!(with_settings(Some(other)).settings.compat_mode, wordcraft_doc::LEGACY_COMPAT_MODE);
}

#[test]
fn line_rule_is_read_case_insensitively() {
    // Word accepts `atleast` as well as the spec's `atLeast`.
    let d = read_body(
        r#"<w:p><w:pPr><w:spacing w:line="280" w:lineRule="atleast"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:line="300" w:lineRule="EXACT"/></w:pPr></w:p>"#,
    );
    let p = paras(&d);
    assert_eq!(p[0].props.line_spacing, Some(wordcraft_doc::props::LineSpacing::AtLeast(14.0)));
    assert_eq!(p[1].props.line_spacing, Some(wordcraft_doc::props::LineSpacing::Exactly(15.0)));
}

#[test]
fn char_border_reads_and_nil_resolves_off() {
    let d = read_body(
        r#"<w:p><w:r><w:rPr><w:bdr w:val="single" w:sz="8" w:space="1" w:color="FF0000"/></w:rPr><w:t>a</w:t></w:r><w:r><w:rPr><w:bdr w:val="nil"/></w:rPr><w:t>b</w:t></w:r></w:p>"#,
    );
    let p = paras(&d)[0];
    assert_eq!(p.props_of_char(0).border, Some(Border { style: BorderStyle::Single, width: 1.0, color: Some(Rgb(0xFF, 0, 0)), space: 1.0 }));
    let nil = p.props_of_char(1);
    assert_eq!(nil.border.map(|b| b.style), Some(BorderStyle::None));
    assert_eq!(d.styles.resolve_char(None, nil).border, None);
}

#[test]
fn anchor_alignment_reference_areas_and_distances() {
    let shape = |pos: &str, dist: &str| {
        format!(
            r#"<w:r><w:drawing><wp:anchor behindDoc="0" {dist}>{pos}<wp:extent cx="127000" cy="127000"/><wp:wrapTopAndBottom/><wp:docPr id="1" name="S"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="ellipse"/></wps:spPr></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
        )
    };
    let body = format!(
        "<w:p>{}{}</w:p>",
        shape(
            r#"<wp:positionH relativeFrom="margin"><wp:align>center</wp:align></wp:positionH><wp:positionV relativeFrom="topMargin"><wp:posOffset>-12700</wp:posOffset></wp:positionV>"#,
            r#"distT="25400" distB="50800" distL="0" distR="114300""#
        ),
        shape(
            r#"<wp:positionH relativeFrom="leftMargin"><wp:posOffset>63500</wp:posOffset></wp:positionH><wp:positionV relativeFrom="line"><wp:align>bottom</wp:align></wp:positionV>"#,
            ""
        ),
    );
    let d = read_body(&body);
    let p = paras(&d);
    let floats: Vec<_> =
        p[0].objects.iter().filter_map(|o| if let InlineObject::Shape { float, .. } = o { Some(float.clone()) } else { None }).collect();
    let [a, b] = floats.as_slice() else { panic!("{floats:?}") };
    assert_eq!((a.h_rel, a.h_align, a.v_rel, a.v_align, a.y), (Anchor::Margin, Some(FloatAlign::Center), Anchor::TopMargin, None, -1.0));
    assert_eq!((a.dist, a.dist_top, a.dist_bottom), (9.0, 2.0, 4.0));
    assert_eq!((b.h_rel, b.h_align, b.x, b.v_rel, b.v_align), (Anchor::LeftMargin, None, 5.0, Anchor::Line, Some(FloatAlign::End)));
    assert_eq!((b.dist, b.dist_top, b.dist_bottom), (0.0, 0.0, 0.0));
}

#[test]
fn documents_without_a_compatibility_mode_are_laid_out_as_word_2007() {
    assert_eq!(read_body("<w:p/>").settings.compat_mode, wordcraft_doc::LEGACY_COMPAT_MODE);
}

#[test]
fn style_rfonts_without_ascii_inherits_doc_defaults_font() {
    // Issue #93: Normal names only East Asian / complex-script fonts, so Latin text keeps the
    // docDefaults font. A run hinted as East Asian still uses its East Asian font.
    let styles = format!(
        r#"<w:styles {W_NS}>
 <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:eastAsia="Calibri" w:hAnsi="Calibri" w:cs="Calibri"/></w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>
 <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/><w:rPr><w:rFonts w:eastAsia="Times New Roman" w:cs="Times New Roman"/></w:rPr></w:style>
</w:styles>"#
    );
    let body = r#"<w:p><w:r><w:t>Plain</w:t></w:r></w:p><w:p><w:r><w:rPr><w:rFonts w:hint="eastAsia" w:eastAsia="SimSun"/></w:rPr><w:t>漢字</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body, &[("rId1", "styles", "styles.xml")], &[("word/styles.xml", &styles)])).unwrap();
    assert_eq!(d.styles.get("Normal").unwrap().chr.font, None);
    let ps = paras(&d);
    let font = |p: &Paragraph| d.styles.resolve_char(p.props.style.as_deref(), &p.runs[0].props).font;
    assert_eq!(font(ps[0]), "Calibri");
    assert_eq!(font(ps[1]), "SimSun");
}

#[test]
fn bare_break_directly_in_paragraph() {
    let body = r#"<w:p><w:r><w:t>First</w:t></w:r><w:br/><w:r><w:t>Second</w:t></w:r><w:cr/><w:br w:type="page"/><w:r><w:t>Third</w:t></w:r></w:p>"#;
    let d = read_body(body);
    assert_eq!(paras(&d)[0].text, "First\nSecond\n\u{C}Third");
    let out = wordcraft_docx::write(&d).unwrap();
    let d2 = wordcraft_docx::read(&out).unwrap();
    assert_eq!(paras(&d2)[0].text, "First\nSecond\n\u{C}Third");
}

/// Issue #1290: Word's text watermark, a VML shape with a `v:textpath` in a header or footer,
/// becomes the document's watermark (drawn on every page) instead of being dropped; other WordArt
/// is not a watermark, and hostile values don't panic.
#[test]
fn vml_textpath_in_header_is_the_watermark() {
    let read = |shape: &str, in_header: bool| {
        let pict = format!("<w:p><w:r><w:t>Header label</w:t></w:r><w:r><w:pict>{shape}</w:pict></w:r></w:p>");
        let header = format!(r#"<w:hdr {W_NS}>{}</w:hdr>"#, if in_header { pict.as_str() } else { "<w:p/>" });
        let body = format!(
            r#"{}<w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#,
            if in_header { "" } else { pict.as_str() }
        );
        let d = wordcraft_docx::read(&docx(&body, &[("rIdH", "header", "header1.xml")], &[("word/header1.xml", &header)])).unwrap();
        let hid = d.last_section.headers.default.unwrap();
        let header_objects = d.parts.get(&hid).unwrap().blocks.iter().filter_map(|b| b.as_para()).map(|p| p.objects.len()).sum::<usize>();
        let header_text = d.parts.get(&hid).unwrap().blocks.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default();
        (d.settings.watermark.clone(), header_objects, header_text)
    };

    // Word's Design › Watermark shape: turned 315°, silver, semitransparent, quoted font name.
    let word = r##"<v:shape id="PowerPlusWaterMarkObject1" o:spid="_x0000_s1025" type="#_x0000_t136" style="position:absolute;margin-left:0;margin-top:0;width:412.4pt;height:247.4pt;rotation:315;z-index:-251657216;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical:center;mso-position-vertical-relative:margin" o:allowincell="f" fillcolor="silver" stroked="f"><v:fill opacity=".5"/><v:textpath style="font-family:&quot;Calibri Light&quot;;font-size:1pt" string="CONFIDENTIAL"/></v:shape>"##;
    let (wm, objects, text) = read(word, true);
    let wm = wm.expect("watermark");
    assert_eq!((wm.text.as_str(), wm.font.as_str(), wm.color), ("CONFIDENTIAL", "Calibri Light", Rgb(0xC0, 0xC0, 0xC0)));
    assert!(wm.diagonal && wm.semitransparent, "{wm:?}");
    // Consumed: not also drawn as a shape in the header, whose text stays.
    assert_eq!((objects, text.as_str()), (0, "Header label"));

    // A page-centred, gray, opaque shape turned -40° (no `v:fill`).
    let gray = r##"<v:shape id="WaterMark" type="#_x0000_t136" style="width:500pt;height:100pt;margin-top:0;margin-left:0;mso-position-horizontal:center;mso-position-horizontal-relative:page;mso-position-vertical:center;mso-position-vertical-relative:page;position:absolute;rotation:-40" fillcolor="gray" strokecolor="gray"><v:textpath style="font-family:Arial" string="BOZZA"/></v:shape>"##;
    let wm = read(gray, true).0.expect("watermark");
    assert_eq!(
        (wm.text.as_str(), wm.font.as_str(), wm.color, wm.diagonal, wm.semitransparent),
        ("BOZZA", "Arial", Rgb(0x80, 0x80, 0x80), true, false)
    );

    // Horizontal, centred both ways without Word's id, opacity in 65536ths.
    let flat = r##"<v:shape type="#_x0000_t136" style="position:absolute;width:300pt;height:60pt;mso-position-horizontal:center;mso-position-vertical:center" fillcolor="#336699"><v:fill opacity="32768f"/><v:textpath string="ASAP"/></v:shape>"##;
    let wm = read(flat, true).0.expect("watermark");
    assert_eq!((wm.color, wm.diagonal, wm.semitransparent), (Rgb(0x33, 0x66, 0x99), false, true));

    // Not watermarks: the same shape in the body (WordArt); header WordArt that isn't centred
    // and isn't Word's watermark (a logo); a textpath turned off; a watermark-like shape inside a
    // header's text box.
    assert_eq!(read(gray, false).0, None);
    let logo = r##"<v:shape type="#_x0000_t136" style="position:absolute;margin-left:0;margin-top:0;width:120pt;height:30pt;mso-position-horizontal:left;mso-position-horizontal-relative:margin" fillcolor="navy"><v:textpath string="ACME"/></v:shape>"##;
    assert_eq!(read(logo, true).0, None);
    assert_eq!(read(&word.replace(r#"<v:textpath "#, r#"<v:textpath on="f" "#), true).0, None);
    let boxed = format!(
        r##"<v:shape type="#_x0000_t202" style="width:200pt;height:100pt"><v:textbox><w:txbxContent><w:p><w:r><w:pict>{word}</w:pict></w:r></w:p></w:txbxContent></v:textbox></v:shape>"##
    );
    assert_eq!(read(&boxed, true).0, None);

    // A footer's watermark counts too, and the first one found wins.
    let footer = format!(r#"<w:ftr {W_NS}><w:p><w:r><w:pict>{gray}</w:pict></w:r></w:p></w:ftr>"#);
    let header = format!(r#"<w:hdr {W_NS}><w:p><w:r><w:pict>{word}</w:pict></w:r></w:p></w:hdr>"#);
    let body = r#"<w:p/><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/><w:footerReference w:type="default" r:id="rIdF"/></w:sectPr>"#;
    let only_footer = r#"<w:p/><w:sectPr><w:footerReference w:type="default" r:id="rIdF"/></w:sectPr>"#;
    let parts = [("word/header1.xml", header.as_str()), ("word/footer1.xml", footer.as_str())];
    let rels = [("rIdH", "header", "header1.xml"), ("rIdF", "footer", "footer1.xml")];
    let wm = |body: &str| wordcraft_docx::read(&docx(body, &rels, &parts)).unwrap().settings.watermark.map(|w| w.text);
    assert_eq!(wm(only_footer).as_deref(), Some("BOZZA"));
    assert_eq!(wm(body).as_deref(), Some("CONFIDENTIAL"));

    // Hostile values: an oversized string is capped, garbage rotation/opacity/colour don't panic,
    // an empty string is no watermark.
    let long = "W".repeat(10_000);
    let hostile = format!(
        r##"<v:shape id="PowerPlusWaterMarkObject9" style="rotation:NaNfd;width:-1e40pt" fillcolor="#zz" ><v:fill opacity="1e99f"/><v:textpath style="font-family:;" string="{long}"/></v:shape>"##
    );
    let wm = read(&hostile, true).0.expect("watermark");
    assert_eq!(wm.text.chars().count(), 200);
    assert!(!wm.font.is_empty());
    assert_eq!(read(r#"<v:shape id="PowerPlusWaterMarkObject2"><v:textpath string="  "/></v:shape>"#, true).0, None);
}

/// Issue #1290: a file with Word's watermark keeps it through save and reopen: the same
/// watermark, the same headers with the same text and paragraphs, one shape per header written;
/// header WordArt that isn't a watermark doesn't become one on the way.
#[test]
fn word_watermark_survives_save() {
    let shape = |id: &str, text: &str| {
        format!(
            r##"<w:r><w:pict><v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136" path="m0,0l21600,0m0,21600l21600,21600e"><v:textpath on="t" fitshape="t"/></v:shapetype><v:shape id="{id}" type="#_x0000_t136" style="position:absolute;margin-left:0;margin-top:0;width:412pt;height:247pt;rotation:315;z-index:-251657216;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical:center;mso-position-vertical-relative:margin" o:allowincell="f" fillcolor="silver" stroked="f"><v:fill opacity=".5"/><v:textpath style="font-family:&quot;Calibri Light&quot;;font-size:1pt" string="{text}"/></v:shape></w:pict></w:r>"##
        )
    };
    let hdr = |inner: &str| {
        format!(r#"<w:hdr {W_NS}><w:p>{inner}<w:r><w:t>Header label</w:t></w:r></w:p><w:p><w:r><w:t>Second line</w:t></w:r></w:p></w:hdr>"#)
    };
    let h1 = hdr(&shape("PowerPlusWaterMarkObject1", "CONFIDENTIAL"));
    let h2 = hdr(&shape("PowerPlusWaterMarkObject2", "CONFIDENTIAL"));
    let body = r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/><w:headerReference w:type="first" r:id="rIdF"/><w:titlePg/></w:sectPr>"#;
    let rels = [("rIdH", "header", "header1.xml"), ("rIdF", "header", "header2.xml")];
    let d = wordcraft_docx::read(&docx(body, &rels, &[("word/header1.xml", &h1), ("word/header2.xml", &h2)])).unwrap();
    let wm = d.settings.watermark.clone().expect("watermark");
    let bytes = wordcraft_docx::write(&d).unwrap();
    let d2 = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(d2.settings.watermark, Some(wm));
    let headers = |d: &Document| {
        let mut v: Vec<Vec<String>> = [d.last_section.headers.default, d.last_section.headers.first]
            .iter()
            .map(|id| d.parts[&id.unwrap()].blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect())
            .collect();
        v.push(vec![d.parts.len().to_string()]);
        v
    };
    assert_eq!(headers(&d2), headers(&d));
    assert_eq!(headers(&d2)[0], vec!["Header label".to_string(), "Second line".to_string()]);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    for name in ["word/header1.xml", "word/header2.xml"] {
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut z.by_name(name).unwrap(), &mut xml).unwrap();
        assert_eq!(xml.matches("<v:shape ").count(), 1, "{name}");
    }

    // A header logo in WordArt isn't a watermark, so none is written.
    let logo = r##"<w:r><w:pict><v:shape type="#_x0000_t136" style="position:absolute;width:120pt;height:30pt;mso-position-horizontal:left" fillcolor="navy"><v:textpath string="ACME"/></v:shape></w:pict></w:r>"##;
    let d = wordcraft_docx::read(&docx(body, &rels, &[("word/header1.xml", &hdr(logo)), ("word/header2.xml", &hdr(""))])).unwrap();
    assert_eq!(d.settings.watermark, None);
    let d2 = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(d2.settings.watermark, None);
}

/// Issue #149: an absolutely positioned VML text box in a header floats where its style puts it
/// (relative to the page here, behind the text at a negative z-index, no wrapping) instead of
/// sitting inline in the header's flow, and it keeps that placement when saved and read back.
#[test]
fn absolute_vml_text_box_in_header_floats() {
    let w10 = r#"xmlns:w10="urn:schemas-microsoft-com:office:word""#;
    let boxes = r##"<w:p><w:r><w:pict><v:shape id="Box" type="#_x0000_t202" style="position:absolute;left:0;margin-left:0pt;margin-top:80pt;width:405pt;height:491.4pt;z-index:-1;mso-position-horizontal-relative:page;mso-position-vertical-relative:page"><v:textbox><w:txbxContent><w:p><w:r><w:t>WATERMARK</w:t></w:r></w:p></w:txbxContent></v:textbox><w10:wrap type="none"/></v:shape></w:pict></w:r></w:p>
<w:p><w:r><w:pict><v:rect style="position:absolute;margin-left:12pt;margin-top:-6pt;width:1in;height:36pt;z-index:3;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical-relative:top-margin-area;mso-wrap-distance-left:9pt;mso-wrap-distance-bottom:4pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>square</w:t></w:r></w:p></w:txbxContent></v:textbox><w10:wrap type="square"/></v:rect></w:pict></w:r>
<w:r><w:pict><v:shape style="position:absolute;margin-left:1in;margin-top:2pt;width:72pt;height:20pt;z-index:5"><v:textbox><w:txbxContent><w:p><w:r><w:t>front</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>
<w:r><w:pict><v:shape style="width:72pt;height:20pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>inline</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>""##;
    let header = format!(r#"<w:hdr {W_NS} {w10}><w:p><w:r><w:t>Header label</w:t></w:r></w:p>{boxes}</w:hdr>"#);
    let body = r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#;
    let bytes = docx(body, &[("rIdH", "header", "header1.xml")], &[("word/header1.xml", &header)]);
    let check = |d: &Document| {
        let hid = d.last_section.headers.default.unwrap();
        let blocks = &d.parts.get(&hid).unwrap().blocks;
        let shapes: Vec<_> = blocks
            .iter()
            .filter_map(|b| b.as_para())
            .flat_map(|p| &p.objects)
            .filter_map(|o| {
                if let InlineObject::Shape { kind, w, h, float, story, .. } = o { Some((*kind, *w, *h, float.clone(), *story)) } else { None }
            })
            .collect();
        let [(k0, w0, h0, f0, s0), (_, w1, h1, f1, _), (_, _, _, f2, _), (_, _, _, f3, _)] = shapes.as_slice() else { panic!("{shapes:?}") };
        assert_eq!(*k0, ShapeKind::TextBox);
        assert!((w0 - 405.0).abs() < 0.01 && (h0 - 491.4).abs() < 0.01, "{w0} x {h0}");
        assert_eq!((f0.wrap, f0.h_rel, f0.v_rel, f0.h_align, f0.v_align), (Wrap::BehindText, Anchor::Page, Anchor::Page, None, None));
        assert!(f0.x.abs() < 0.01 && (f0.y - 80.0).abs() < 0.01, "offset {} {}", f0.x, f0.y);
        let text = d.parts.get(&s0.unwrap()).unwrap().blocks[0].as_para().unwrap().text.clone();
        assert_eq!(text, "WATERMARK");
        assert_eq!((*w1, *h1), (72.0, 36.0));
        assert_eq!(
            (f1.wrap, f1.h_rel, f1.h_align, f1.v_rel, f1.v_align),
            (Wrap::Square, Anchor::Margin, Some(FloatAlign::Center), Anchor::TopMargin, None)
        );
        assert!((f1.y + 6.0).abs() < 0.01 && (f1.dist - 9.0).abs() < 0.01 && (f1.dist_bottom - 4.0).abs() < 0.01, "{f1:?}");
        // No wrap element: in front of the text, relative to the column and paragraph.
        assert_eq!((f2.wrap, f2.h_rel, f2.v_rel), (Wrap::InFrontOfText, Anchor::Column, Anchor::Paragraph));
        assert!((f2.x - 72.0).abs() < 0.01 && (f2.y - 2.0).abs() < 0.01, "{f2:?}");
        // Not absolutely positioned: still inline.
        assert_eq!(f3.wrap, Wrap::Inline);
    };
    let d = wordcraft_docx::read(&bytes).unwrap();
    check(&d);
    // Saved (as DrawingML anchors) and read back, the boxes keep their placement.
    check(&wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap());
}

#[test]
fn hostile_vml_style_values_stay_finite() {
    let body = r#"<w:p><w:r><w:pict><v:shape style="position:ABSOLUTE;margin-left:1e39pt;left:-1e39pt;margin-top:NaNpt;top:;width:-5pt;height:1e30in;z-index:99999999999999999999999;mso-position-horizontal:bogus;mso-wrap-distance-left:-3pt;mso-wrap-distance-top:1e9pt;;:;position"><v:textbox><w:txbxContent><w:p/></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>"#;
    let d = read_body(body);
    let p = paras(&d);
    let Some(InlineObject::Shape { w, h, float, .. }) = p[0].objects.first() else { panic!("{:?}", p[0].objects) };
    assert_eq!(float.wrap, Wrap::InFrontOfText);
    assert!([*w, *h, float.x, float.y, float.dist, float.dist_top].iter().all(|v| v.is_finite()), "{w} {h} {float:?}");
    assert_eq!((float.h_align, float.dist, float.dist_top), (None, 0.0, 1584.0));
}

// ---- charts and OLE objects written back (#319) ----

const CHART_XML: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart><c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"#;
/// Stands in for the chart's workbook: opaque bytes, never opened.
const WORKBOOK: &str = "PK\u{3}\u{4} not really a workbook";
const COLORS_XML: &str = r#"<cs:colorStyle xmlns:cs="http://schemas.microsoft.com/office/drawing/2012/chartStyle" meth="cycle" id="10"/>"#;
const CT_CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
const CT_XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/// A package holding one inline chart (3 × 1.5 in) with its workbook and colour style parts.
fn chart_docx() -> Vec<u8> {
    let body = r#"<w:p><w:r><w:t>Sales</w:t></w:r><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="2743200" cy="1371600"/><wp:docPr id="1" name="Chart 1"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId7"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let chart_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="../embeddings/Microsoft_Excel_Worksheet.xlsx"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2011/relationships/chartColorStyle" Target="colors1.xml"/></Relationships>"#;
    let types = format!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="xlsx" ContentType="{CT_XLSX}"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/charts/chart1.xml" ContentType="{CT_CHART}"/><Override PartName="/word/charts/colors1.xml" ContentType="application/vnd.ms-office.chartcolorstyle+xml"/></Types>"#
    );
    docx(
        body,
        &[("rId7", "chart", "charts/chart1.xml")],
        &[
            ("[Content_Types].xml", &types),
            ("word/charts/chart1.xml", CHART_XML),
            ("word/charts/_rels/chart1.xml.rels", chart_rels),
            ("word/charts/colors1.xml", COLORS_XML),
            ("word/embeddings/Microsoft_Excel_Worksheet.xlsx", WORKBOOK),
        ],
    )
}

/// The entries of a zip, by name.
fn unzip(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = std::collections::BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        out.insert(f.name().to_string(), b);
    }
    out
}

fn text(files: &std::collections::BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(files.get(name).unwrap_or_else(|| panic!("no {name} in {:?}", files.keys())).clone()).unwrap()
}

/// Values of attribute `attr` on the elements named `el` (in written order).
fn attr_values(xml: &str, el: &str, attr: &str) -> Vec<String> {
    let start = |s: &str| format!(" {}", s.split('>').next().unwrap_or(""));
    xml.split(&format!("<{el} "))
        .skip(1)
        .filter_map(|s| start(s).split(&format!(" {attr}=\"")).nth(1)?.split('"').next().map(str::to_string))
        .collect()
}

/// (type, target) of relationship `id` in a relationships part.
fn rel(rels: &str, id: &str) -> (String, String) {
    let r = rels.split("<Relationship ").find(|r| r.contains(&format!("Id=\"{id}\""))).unwrap_or_else(|| panic!("no {id} in {rels}"));
    let get = |a: &str| r.split(&format!("{a}=\"")).nth(1).unwrap().split('"').next().unwrap().to_string();
    (get("Type"), get("Target"))
}

/// Package part a relationship target of `from_dir` names.
fn part_at(from_dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Checks a written chart: its part and the parts it relates to are all in the package, with the
/// original bytes and content types. Returns the chart's part name.
fn check_chart(files: &std::collections::BTreeMap<String, Vec<u8>>, rid: &str) -> String {
    let (kind, target) = rel(&text(files, "word/_rels/document.xml.rels"), rid);
    assert!(kind.ends_with("/chart"), "{kind}");
    let chart = part_at("word", &target);
    assert_eq!(text(files, &chart), CHART_XML);
    let (dir, file) = chart.rsplit_once('/').unwrap();
    let chart_rels = text(files, &format!("{dir}/_rels/{file}.rels"));
    let (kind, wb) = rel(&chart_rels, "rId1");
    assert!(kind.ends_with("/package"), "{kind}");
    let wb = part_at(dir, &wb);
    assert_eq!(files.get(&wb).map(Vec::as_slice), Some(WORKBOOK.as_bytes()), "{wb}");
    let (kind, colors) = rel(&chart_rels, "rId2");
    assert!(kind.ends_with("/chartColorStyle"), "{kind}");
    assert_eq!(text(files, &part_at(dir, &colors)), COLORS_XML);
    let types = text(files, "[Content_Types].xml");
    assert!(types.contains(&format!(r#"PartName="/{chart}" ContentType="{CT_CHART}""#)), "{types}");
    assert!(types.contains(&format!(r#"PartName="/{wb}" ContentType="{CT_XLSX}""#)), "{types}");
    chart
}

fn graphics(d: &Document) -> Vec<InlineObject> {
    paras(d).iter().flat_map(|p| p.objects.iter()).filter(|o| matches!(o, InlineObject::Graphic { .. })).cloned().collect()
}

#[test]
fn chart_round_trips_with_its_parts() {
    let d = wordcraft_docx::read(&chart_docx()).unwrap();
    let out = wordcraft_docx::write(&d).unwrap();
    let files = unzip(&out);
    let body = text(&files, "word/document.xml");
    let ids = attr_values(&body, "c:chart", "r:id");
    assert_eq!(ids.len(), 1, "{body}");
    assert_eq!(check_chart(&files, &ids[0]), "word/charts/chart1.xml");
    assert_eq!(attr_values(&body, "wp:extent", "cx"), ["2743200"]);

    // Read back: still drawn, and a second save still carries it.
    let back = wordcraft_docx::read(&out).unwrap();
    let g = graphics(&back);
    let [InlineObject::Graphic { w, graphic, .. }] = g.as_slice() else { panic!("{g:?}") };
    assert_eq!(*w, 216.0);
    assert!(!graphic.items.is_empty() && graphic.source.is_some());
    let again = unzip(&wordcraft_docx::write(&back).unwrap());
    let ids = attr_values(&text(&again, "word/document.xml"), "c:chart", "r:id");
    check_chart(&again, &ids[0]);
}

// ---- Office 2016 charts (cx:chart) written back with their fallback picture (#997) ----

const CHARTEX_NS: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";
const CHARTEX_XML: &str = r#"<cx:chartSpace xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><cx:chartData><cx:externalData r:id="rId1" cx:autoUpdate="0"/></cx:chartData><cx:chart><cx:plotArea><cx:plotAreaRegion><cx:series layoutId="boxWhisker"/></cx:plotAreaRegion></cx:plotArea></cx:chart></cx:chartSpace>"#;
const CT_CHARTEX: &str = "application/vnd.ms-office.chartex+xml";

/// A package holding one box & whisker chart the way Word writes it: the `cx:chart` in an
/// `mc:Choice` requiring `requires` (bound to `requires_ns`), a picture of it in the `mc:Fallback`.
fn chartex_docx(requires: &str, requires_ns: &str) -> Vec<u8> {
    let frame = r#"<wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="2743200" cy="1371600"/><wp:docPr id="1" name="Chart 1"/><wp:cNvGraphicFramePr/>"#;
    let body = format!(
        r#"<w:p><w:r><w:t>Spread</w:t></w:r><w:r><mc:AlternateContent><mc:Choice xmlns:{requires}="{requires_ns}" Requires="{requires}"><w:drawing>{frame}<a:graphic><a:graphicData uri="{CHARTEX_NS}"><cx:chart xmlns:cx="{CHARTEX_NS}" r:id="rIdCx"/></a:graphicData></a:graphic></wp:inline></w:drawing></mc:Choice><mc:Fallback><w:drawing>{frame}<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="Chart 1"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdPic"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2743200" cy="1371600"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></mc:Fallback></mc:AlternateContent></w:r></w:p>"#
    );
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let doc_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdCx" Type="http://schemas.microsoft.com/office/2014/relationships/chartEx" Target="charts/chartEx1.xml"/><Relationship Id="rIdPic" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/></Relationships>"#;
    let chart_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="../embeddings/Microsoft_Excel_Worksheet.xlsx"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2011/relationships/chartColorStyle" Target="colors1.xml"/></Relationships>"#;
    let types = format!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="xlsx" ContentType="{CT_XLSX}"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/charts/chartEx1.xml" ContentType="{CT_CHARTEX}"/><Override PartName="/word/charts/colors1.xml" ContentType="application/vnd.ms-office.chartcolorstyle+xml"/></Types>"#
    );
    zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/charts/chartEx1.xml", CHARTEX_XML.as_bytes()),
        ("word/charts/_rels/chartEx1.xml.rels", chart_rels.as_bytes()),
        ("word/charts/colors1.xml", COLORS_XML.as_bytes()),
        ("word/embeddings/Microsoft_Excel_Worksheet.xlsx", WORKBOOK.as_bytes()),
        ("word/media/image1.png", b"fallback picture bytes"),
    ])
}

#[test]
fn office_2016_chart_keeps_its_chart_part_and_fallback_through_saves() {
    // Word binds `cx1`… to the chart type's own namespace; Apache POI's sample binds `cx`.
    for (requires, requires_ns) in [("cx1", "http://schemas.microsoft.com/office/drawing/2015/9/8/chartex"), ("cx", CHARTEX_NS)] {
        let mut d = wordcraft_docx::read(&chartex_docx(requires, requires_ns)).unwrap();
        for _ in 0..2 {
            let g = graphics(&d);
            let [InlineObject::Graphic { w, graphic, .. }] = g.as_slice() else { panic!("{requires}: {g:?}") };
            assert_eq!(*w, 216.0);
            assert!(
                graphic.items.iter().any(|i| matches!(i, wordcraft_doc::graphic::GraphicItem::Image { media, .. } if d.media.contains_key(media))),
                "{requires}: the chart shows its fallback picture"
            );
            let files = unzip(&wordcraft_docx::write(&d).unwrap());
            let body = text(&files, "word/document.xml");
            assert_eq!(attr_values(&body, "mc:Choice", "Requires"), [requires], "{body}");
            assert!(body.contains(&format!(r#"xmlns:{requires}="{requires_ns}""#)), "{requires} is declared: {body}");
            let doc_rels = text(&files, "word/_rels/document.xml.rels");
            let [cx_id] = attr_values(&body, "cx:chart", "r:id").try_into().unwrap_or_else(|_| panic!("chart lost: {body}"));
            let (kind, target) = rel(&doc_rels, &cx_id);
            assert!(kind.ends_with("/chartEx"), "{kind}");
            let chart = part_at("word", &target);
            assert_eq!(text(&files, &chart), CHARTEX_XML);
            let types = text(&files, "[Content_Types].xml");
            assert!(types.contains(&format!(r#"PartName="/{chart}" ContentType="{CT_CHARTEX}""#)), "{types}");
            let (dir, file) = chart.rsplit_once('/').unwrap();
            let (_, wb) = rel(&text(&files, &format!("{dir}/_rels/{file}.rels")), "rId1");
            assert_eq!(files.get(&part_at(dir, &wb)).map(Vec::as_slice), Some(WORKBOOK.as_bytes()));
            let fallback = body.split("<mc:Fallback>").nth(1).unwrap_or_else(|| panic!("no fallback: {body}"));
            let [pic_id] = attr_values(fallback, "a:blip", "r:embed").try_into().unwrap_or_else(|_| panic!("fallback picture lost: {body}"));
            assert_eq!(files.get(&part_at("word", &rel(&doc_rels, &pic_id).1)).map(Vec::as_slice), Some(&b"fallback picture bytes"[..]));
            assert!(attr_values(&body, "wp:docPr", "name").iter().all(|n| n.starts_with("Chart ")), "{body}");
            let entries: Vec<_> = files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
            d = wordcraft_docx::read(&zip(&entries)).unwrap();
        }
    }
}

#[test]
fn resized_office_2016_chart_resizes_its_fallback_too() {
    let mut d = wordcraft_docx::read(&chartex_docx("cx1", "http://schemas.microsoft.com/office/drawing/2015/9/8/chartex")).unwrap();
    if let Some(Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut)
        && let Some(InlineObject::Graphic { w, h, .. }) = p.objects.first_mut()
    {
        (*w, *h) = (288.0, 144.0);
    }
    let body = text(&unzip(&wordcraft_docx::write(&d).unwrap()), "word/document.xml");
    assert_eq!(attr_values(&body, "wp:extent", "cx"), ["3657600", "3657600"], "{body}");
}

#[test]
fn moved_and_copied_chart_keeps_its_parts() {
    let d = wordcraft_docx::read(&chart_docx()).unwrap();
    let g = graphics(&d);
    let [InlineObject::Graphic { alt, graphic, .. }] = g.as_slice() else { panic!("{g:?}") };
    // Resized and dragged to float on the page in WordCraft, then copied and pasted.
    let float = wordcraft_doc::para::Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 36.0, y: 18.0, ..Default::default() };
    let moved = InlineObject::Graphic { w: 288.0, h: 144.0, alt: alt.clone(), float, graphic: graphic.clone() };
    let mut p = Paragraph::new();
    p.insert_object(0, moved.clone(), &Default::default()).unwrap();
    p.insert_object(p.len(), moved, &Default::default()).unwrap();
    let mut doc = d.clone();
    doc.body = vec![wordcraft_doc::para_block(p)];

    let out = wordcraft_docx::write(&doc).unwrap();
    let files = unzip(&out);
    let body = text(&files, "word/document.xml");
    assert_eq!(attr_values(&body, "wp:extent", "cx"), ["3657600", "3657600"], "the new size");
    assert_eq!(body.matches("<wp:posOffset>457200</wp:posOffset>").count(), 2, "the new position");
    let ids = attr_values(&body, "c:chart", "r:id");
    assert_eq!(ids.len(), 2);
    let charts: Vec<String> = ids.iter().map(|id| check_chart(&files, id)).collect();
    assert_ne!(charts[0], charts[1], "each copy has its own chart part");

    let back = wordcraft_docx::read(&out).unwrap();
    let g = graphics(&back);
    assert_eq!(g.len(), 2);
    for o in &g {
        let InlineObject::Graphic { w, float, graphic, .. } = o else { panic!() };
        assert_eq!((*w, float.wrap, float.x), (288.0, Wrap::Square, 36.0));
        assert!(!graphic.items.is_empty() && graphic.source.is_some());
    }
}

#[test]
fn ole_object_round_trips() {
    let body = r##"<w:p><w:r><w:object w:dxaOrig="1440" w:dyaOrig="720"><v:shape id="_x0000_i1025" type="#_x0000_t75" style="width:72pt;height:36pt" o:ole=""><v:imagedata r:id="rId8" o:title=""/></v:shape><o:OLEObject Type="Embed" ProgID="Package" ShapeID="_x0000_i1025" DrawAspect="Content" ObjectID="_1000000001" r:id="rId9"/></w:object></w:r></w:p>"##;
    let picture = "\u{89}PNG stand-in";
    let ole = "\u{d0}\u{cf} opaque OLE storage";
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/><Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.oleObject"/></Types>"#;
    let bytes = docx(
        body,
        &[("rId8", "image", "media/image1.png"), ("rId9", "oleObject", "embeddings/oleObject1.bin")],
        &[("[Content_Types].xml", types), ("word/media/image1.png", picture), ("word/embeddings/oleObject1.bin", ole)],
    );
    let d = wordcraft_docx::read(&bytes).unwrap();
    let Some(InlineObject::Image { w, ole: Some(_), .. }) = paras(&d)[0].objects.first() else { panic!("{:?}", paras(&d)[0].objects) };
    assert_eq!(*w, 72.0);

    // Resized in WordCraft.
    let mut doc = d.clone();
    if let Some(wordcraft_doc::Block::Para(p)) = doc.body.first_mut().map(std::sync::Arc::make_mut)
        && let Some(InlineObject::Image { w, h, .. }) = p.objects.first_mut()
    {
        (*w, *h) = (144.0, 72.0);
    }
    let out = wordcraft_docx::write(&doc).unwrap();
    let files = unzip(&out);
    let body = text(&files, "word/document.xml");
    let rels = text(&files, "word/_rels/document.xml.rels");
    assert_eq!(attr_values(&body, "v:shape", "style"), ["width:144pt;height:72pt"]);
    let [ole_id] = attr_values(&body, "o:OLEObject", "r:id").try_into().unwrap();
    let (kind, target) = rel(&rels, &ole_id);
    assert!(kind.ends_with("/oleObject"), "{kind}");
    let ole_part = part_at("word", &target);
    assert_eq!(files.get(&ole_part).map(Vec::as_slice), Some(ole.as_bytes()));
    assert!(
        text(&files, "[Content_Types].xml")
            .contains(&format!(r#"PartName="/{ole_part}" ContentType="application/vnd.openxmlformats-officedocument.oleObject""#))
    );
    let [pic_id] = attr_values(&body, "v:imagedata", "r:id").try_into().unwrap();
    let (kind, target) = rel(&rels, &pic_id);
    assert!(kind.ends_with("/image"), "{kind}");
    assert_eq!(files.get(&part_at("word", &target)).map(Vec::as_slice), Some(picture.as_bytes()));
    assert!(!body.contains("<w:drawing>"), "the object, not a plain picture");

    let back = wordcraft_docx::read(&out).unwrap();
    assert!(matches!(paras(&back)[0].objects.first(), Some(InlineObject::Image { w: 144.0, ole: Some(_), .. })));
}

#[test]
fn hostile_embedded_relationships_stay_bounded() {
    // The chart relates to itself, the main document, a relationships part and a missing part;
    // another drawing uses an id with no relationship. Reading and saving neither loop nor carry
    // those along, and the chart is still written back.
    let chart_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x/package" Target="chart1.xml"/><Relationship Id="rId2" Type="x/a" Target="../document.xml"/><Relationship Id="rId3" Type="x/b" Target="../_rels/document.xml.rels"/><Relationship Id="rId4" Type="x/c" Target="gone.bin"/><Relationship Id="rId4" Type="x/d" Target="chart1.xml"/></Relationships>"#;
    let drawing = |id: &str| {
        format!(
            r#"<w:r><w:drawing><wp:inline><wp:extent cx="2743200" cy="1371600"/><wp:docPr id="1" name="C"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="{id}"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
        )
    };
    let body = format!("<w:p>{}{}</w:p>", drawing("rId7"), drawing("rIdNowhere"));
    let bytes = docx(
        &body,
        &[("rId7", "chart", "charts/chart1.xml")],
        &[("word/charts/chart1.xml", CHART_XML), ("word/charts/_rels/chart1.xml.rels", chart_rels)],
    );
    let d = wordcraft_docx::read(&bytes).unwrap();
    let g = graphics(&d);
    assert_eq!(g.len(), 2);
    let sources: Vec<bool> = g.iter().map(|o| matches!(o, InlineObject::Graphic { graphic, .. } if graphic.source.is_some())).collect();
    assert_eq!(sources, [true, false], "a dangling id can't be written back");
    let files = unzip(&wordcraft_docx::write(&d).unwrap());
    let body = text(&files, "word/document.xml");
    let [id] = attr_values(&body, "c:chart", "r:id").try_into().unwrap();
    let chart = part_at("word", &rel(&text(&files, "word/_rels/document.xml.rels"), &id).1);
    let rels = text(&files, "word/charts/_rels/chart1.xml.rels");
    assert_eq!(rel(&rels, "rId1").1, "chart1.xml", "its relationship to itself, to the same written part");
    assert!(!rels.contains("document.xml") && !rels.contains("gone.bin") && rels.matches("Id=\"rId4\"").count() <= 1, "{rels}");
    assert_eq!(text(&files, &chart), CHART_XML);
    assert_eq!(files.keys().filter(|k| k.starts_with("word/charts/") && !k.contains("_rels")).count(), 1);
}

/// A group as Word saves one (in `mc:AlternateContent`, needing `wpg`): its members' space
/// starts at `a:chOff`, and a nested group is flattened into it through its own transform.
#[test]
fn word_group_with_nested_group() {
    let body = r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wpg"><w:drawing><wp:anchor behindDoc="0" distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="1" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="2540000" cy="1270000"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="1" name="Group 1"/><wp:cNvGraphicFramePr/>
<a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1270000"/><a:chOff x="127000" y="127000"/><a:chExt cx="2540000" cy="1270000"/></a:xfrm></wpg:grpSpPr>
<wps:wsp><wps:cNvPr id="2" name="Rectangle 2"/><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="127000" y="127000"/><a:ext cx="1270000" cy="635000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></wps:spPr><wps:bodyPr/></wps:wsp>
<wpg:grpSp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="1397000" y="762000"/><a:ext cx="1270000" cy="635000"/><a:chOff x="0" y="0"/><a:chExt cx="2540000" cy="1270000"/></a:xfrm></wpg:grpSpPr>
<wps:wsp><wps:cNvPr id="3" name="Oval 3"/><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="254000" y="254000"/><a:ext cx="508000" cy="254000"/></a:xfrm><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom></wps:spPr><wps:bodyPr/></wps:wsp>
</wpg:grpSp></wpg:wgp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent></w:r></w:p>"#;
    let d = read_body(body);
    let p = paras(&d);
    let InlineObject::Group { w, h, float, ch_w, ch_h, children, .. } = &p[0].objects[0] else { panic!("{:?}", p[0].objects) };
    assert_eq!((*w, *h, *ch_w, *ch_h), (200.0, 100.0, 200.0, 100.0));
    assert_eq!(float.wrap, Wrap::Square);
    let got: Vec<(f32, f32, ShapeKind, f32, f32)> = children
        .iter()
        .map(|c| match &c.obj {
            InlineObject::Shape { kind, w, h, .. } => (c.x, c.y, *kind, *w, *h),
            o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(got, [(0.0, 0.0, ShapeKind::Rectangle, 100.0, 50.0), (110.0, 60.0, ShapeKind::Ellipse, 20.0, 10.0)]);
}

/// The first object of the first paragraph, made mutable.
fn first_object(doc: &mut Document) -> &mut InlineObject {
    let Some(wordcraft_doc::Block::Para(p)) = doc.body.first_mut().map(std::sync::Arc::make_mut) else { panic!("a paragraph") };
    p.objects.first_mut().expect("an object")
}

/// #319 with #332: an OLE object turned or flipped in WordCraft is written back turned, in its VML
/// shape (`rotation`, `flip`) and in a DrawingML picture (`a:xfrm` rot/flipH/flipV, with the
/// effect extent covering the rotated bounds like other drawings).
#[test]
fn rotated_ole_objects_round_trip() {
    let picture = "\u{89}PNG stand-in";
    let ole = "\u{d0}\u{cf} opaque OLE storage";
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/><Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.oleObject"/></Types>"#;
    let parts = [("[Content_Types].xml", types), ("word/media/image1.png", picture), ("word/embeddings/oleObject1.bin", ole)];
    let doc_rels = [("rId8", "image", "media/image1.png"), ("rId9", "oleObject", "embeddings/oleObject1.bin")];
    let spin = |o: &InlineObject| o.frame().map(|f| f.2.spin()).unwrap();

    // VML: read turned, written with the new turn.
    let vml = r##"<w:p><w:r><w:object w:dxaOrig="1440" w:dyaOrig="720"><v:shape id="_x0000_i1025" type="#_x0000_t75" style="width:72pt;height:36pt;rotation:30;flip:x" o:ole=""><v:imagedata r:id="rId8" o:title=""/></v:shape><o:OLEObject Type="Embed" ProgID="Package" ShapeID="_x0000_i1025" DrawAspect="Content" ObjectID="_1000000001" r:id="rId9"/></w:object></w:r></w:p>"##;
    let mut d = wordcraft_docx::read(&docx(vml, &doc_rels, &parts)).unwrap();
    assert_eq!(spin(&paras(&d)[0].objects[0]), wordcraft_geom::Spin::new(30.0, true, false));
    if let Some(f) = first_object(&mut d).float_mut() {
        f.set_spin(wordcraft_geom::Spin::new(45.0, false, true));
    }
    let out = wordcraft_docx::write(&d).unwrap();
    let body = text(&unzip(&out), "word/document.xml");
    assert_eq!(attr_values(&body, "v:shape", "style"), ["width:72pt;height:36pt;rotation:45;flip:y"]);
    let back = wordcraft_docx::read(&out).unwrap();
    assert!(matches!(&paras(&back)[0].objects[0], InlineObject::Image { ole: Some(_), .. }));
    assert_eq!(spin(&paras(&back)[0].objects[0]), wordcraft_geom::Spin::new(45.0, false, true));

    // DrawingML: the picture's `a:xfrm` and the drawing's effect extent follow the turn.
    let dml = r##"<w:p><w:r><w:object w:dxaOrig="1440" w:dyaOrig="720"><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="914400" cy="457200"/><wp:docPr id="7" name="Object 7"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name=""/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId8"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm flipH="1"><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing><o:OLEObject Type="Embed" ProgID="Package" DrawAspect="Content" ObjectID="_1000000002" r:id="rId9"/></w:object></w:r></w:p>"##;
    let mut d = wordcraft_docx::read(&docx(dml, &doc_rels, &parts)).unwrap();
    assert_eq!(spin(&paras(&d)[0].objects[0]), wordcraft_geom::Spin::new(0.0, true, false));
    if let Some(f) = first_object(&mut d).float_mut() {
        f.set_spin(wordcraft_geom::Spin::new(90.0, true, false));
    }
    let out = wordcraft_docx::write(&d).unwrap();
    let body = text(&unzip(&out), "word/document.xml");
    assert_eq!(attr_values(&body, "a:xfrm", "rot"), ["5400000"]);
    assert_eq!(attr_values(&body, "a:xfrm", "flipH"), ["1"]);
    // 72 × 36 pt turned 90°: 18 pt narrower each side, 18 pt taller each side.
    assert_eq!(attr_values(&body, "wp:effectExtent", "l"), ["-228600"]);
    assert_eq!(attr_values(&body, "wp:effectExtent", "t"), ["228600"]);
    let back = wordcraft_docx::read(&out).unwrap();
    let o = &paras(&back)[0].objects[0];
    assert!(matches!(o, InlineObject::Image { ole: Some(_), .. }), "{o:?}");
    assert_eq!(spin(o), wordcraft_geom::Spin::new(90.0, true, false));
    assert_eq!(o.frame().unwrap().2.effect, [0.0; 4], "the rotated overhang is not kept as effects room");
}

/// Issue #41: tracked formatting changes (ECMA-376 §17.13.5) survive open and save — run,
/// paragraph mark, paragraph, numbering, table, row, cell and section — each written last in
/// its parent, holding the properties before the change.
#[test]
fn formatting_revisions_round_trip() {
    let body = r#"
<w:p><w:r><w:t xml:space="preserve">Plain, </w:t></w:r><w:r><w:rPr><w:b/><w:rPrChange w:id="5" w:author="Alice" w:date="2026-10-01T00:00:00Z"><w:rPr/></w:rPrChange></w:rPr><w:t>bold by Alice</w:t></w:r><w:r><w:rPr><w:sz w:val="28"/><w:rPrChange w:id="99999999999" w:author="Bob"><w:rPr><w:i/><w:sz w:val="24"/><w:rPrChange w:id="1" w:author="Nested"><w:rPr/></w:rPrChange></w:rPr></w:rPrChange></w:rPr><w:t xml:space="preserve"> end.</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/><w:numberingChange w:id="6" w:author="Alice" w:date="2026-10-01T00:00:00Z" w:original="1."/></w:numPr><w:jc w:val="center"/><w:rPr><w:i/><w:rPrChange w:id="7" w:author="Alice" w:date="2026-10-01T00:00:00Z"><w:rPr/></w:rPrChange></w:rPr><w:pPrChange w:id="8" w:author="Alice" w:date="2026-10-01T00:00:00Z"><w:pPr><w:ind w:left="720"/></w:pPr></w:pPrChange></w:pPr><w:r><w:t>Centred</w:t></w:r></w:p>
<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:jc w:val="center"/><w:tblPrChange w:id="9" w:author="Bob"><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr></w:tblPrChange></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>
<w:tr><w:trPr><w:cantSplit/><w:trPrChange w:id="10" w:author="Bob"><w:trPr/></w:trPrChange></w:trPr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:shd w:val="clear" w:color="auto" w:fill="FFFF00"/><w:tcPrChange w:id="11" w:author="Bob"><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr></w:tcPrChange></w:tcPr><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p/>
<w:sectPr><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/><w:sectPrChange w:id="12" w:author="Carol" w:date="2026-10-02T00:00:00Z"><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr></w:sectPrChange></w:sectPr>"#;
    let d = read_body(body);
    let rev = |i: u32| d.revisions.get(i as usize).map(|r| (r.kind, r.author.as_str(), r.date.as_str()));
    let p = paras(&d);
    let runs: Vec<_> = p[0].run_ranges().map(|(r, c)| (r, c.clone())).collect();
    assert_eq!(runs.len(), 3);
    let alice = runs[1].1.fmt_change.as_deref().expect("Alice's rPrChange");
    assert_eq!(rev(alice.rev), Some((wordcraft_doc::RevisionKind::Format, "Alice", "2026-10-01T00:00:00Z")));
    assert_eq!((runs[1].1.bold, alice.old.bold), (Some(true), None));
    let bob = runs[2].1.fmt_change.as_deref().expect("Bob's rPrChange");
    assert_eq!((bob.old.italic, bob.old.size, bob.old.fmt_change.is_none()), (Some(true), Some(12.0), true), "a nested change is dropped");
    assert!(runs[0].1.fmt_change.is_none());
    let pc = p[1].props.fmt_change.as_deref().expect("pPrChange");
    assert_eq!((p[1].props.align, pc.old.align, pc.old.indent_left), (Some(Align::Center), None, Some(36.0)));
    assert_eq!(p[1].props.num_change.as_deref().map(|n| n.original.as_str()), Some("1."));
    assert!(p[1].mark.fmt_change.is_some());
    let Some(Block::Table(t)) = d.body.get(2).map(|b| &**b) else { panic!("table") };
    assert_eq!(t.props.fmt_change.as_deref().map(|c| c.old.align), Some(None));
    assert_eq!(t.rows[0].props.fmt_change.as_deref().map(|c| c.old.cant_split), Some(false));
    assert_eq!(t.rows[0].cells[0].props.fmt_change.as_deref().map(|c| c.old.shading), Some(None));
    let sc = d.last_section.fmt_change.as_deref().expect("sectPrChange");
    assert_eq!((d.last_section.landscape, sc.old.landscape), (true, false));
    assert_eq!(rev(sc.rev).map(|r| r.1), Some("Carol"));

    // Written back: each change is the last child of its parent, and reads back the same.
    let bytes = wordcraft_docx::write(&d).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    for (tag, parent) in [
        ("w:rPrChange", "</w:rPr></w:rPrChange></w:rPr>"),
        ("w:pPrChange", "</w:pPr></w:pPrChange></w:pPr>"),
        ("w:tblPrChange", "</w:tblPr></w:tblPrChange></w:tblPr>"),
        ("w:trPrChange", "</w:trPrChange></w:trPr>"),
        ("w:tcPrChange", "</w:tcPr></w:tcPrChange></w:tcPr>"),
        ("w:sectPrChange", "</w:sectPr></w:sectPrChange></w:sectPr>"),
    ] {
        assert!(xml.contains(&format!("<{tag} w:id=")), "{tag}: {xml}");
        assert!(xml.contains(parent), "{tag} last: {xml}");
    }
    assert!(xml.contains(r#"<w:numberingChange w:id="#) && xml.contains(r#"w:original="1."/></w:numPr>"#), "{xml}");
    assert!(xml.contains(r#"<w:b/><w:rPrChange w:id="#), "{xml}");
    assert!(!xml.contains("<w:bCs"), "Saving must not add an absent complex-script bold: {xml}");
    let again = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(again.body, d.body);
    assert_eq!(again.last_section, d.last_section);
    assert_eq!(again.revisions, d.revisions);
}

/// A package with `styles`, a theme (accent1 4472C4, accent2 ED7D31, major font "Major Face") and,
/// when given, `settings`.
fn themed_docx(body: &str, styles: &str, settings: Option<&str>) -> Document {
    let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="T"><a:themeElements><a:clrScheme name="T"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="T"><a:majorFont><a:latin typeface="Major Face"/></a:majorFont><a:minorFont><a:latin typeface="Minor Face"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;
    let styles = format!(
        r#"<w:styles {W_NS}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>{styles}</w:styles>"#
    );
    let mut doc_rels = vec![("rId1", "styles", "styles.xml"), ("rId2", "theme", "theme/theme1.xml")];
    let mut extra = vec![("word/styles.xml", styles.as_str()), ("word/theme/theme1.xml", theme)];
    let settings = settings.map(|s| format!(r#"<w:settings {W_NS}>{s}</w:settings>"#));
    if let Some(s) = &settings {
        doc_rels.push(("rId3", "settings", "settings.xml"));
        extra.push(("word/settings.xml", s.as_str()));
    }
    wordcraft_docx::read(&docx(body, &doc_rels, &extra)).unwrap()
}

/// Within one step per channel: Word's tint and shade round differently from ours in the last bit.
fn near(c: Option<TextColor>, want: Rgb) -> bool {
    matches!(c, Some(TextColor::Rgb(Rgb(r, g, b))) if r.abs_diff(want.0) <= 1 && g.abs_diff(want.1) <= 1 && b.abs_diff(want.2) <= 1)
}

#[test]
fn theme_colour_wins_over_the_fixed_colour() {
    // Heading 1 as Word writes it: the stale w:val is ignored, accent1 darkened 25% is 2F5496.
    let styles = r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:rPr><w:color w:val="FF0000" w:themeColor="accent1" w:themeShade="BF"/><w:u w:val="single" w:color="FF0000" w:themeColor="accent2"/></w:rPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>H</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:color w:val="000000" w:themeColor="accent1" w:themeTint="99"/></w:rPr><w:t>a</w:t></w:r><w:r><w:rPr><w:color w:val="000000" w:themeColor="accent1" w:themeTint="66"/></w:rPr><w:t>b</w:t></w:r><w:r><w:rPr><w:color w:val="000000" w:themeColor="accent1" w:themeTint="33"/></w:rPr><w:t>c</w:t></w:r><w:r><w:rPr><w:color w:val="000000" w:themeColor="accent1" w:themeShade="7F"/></w:rPr><w:t>d</w:t></w:r><w:r><w:rPr><w:color w:val="000000" w:themeColor="accent2" w:themeShade="BF"/></w:rPr><w:t>e</w:t></w:r><w:r><w:rPr><w:color w:val="000000" w:themeColor="text2"/></w:rPr><w:t>f</w:t></w:r><w:r><w:rPr><w:color w:val="auto" w:themeColor="hyperlink"/></w:rPr><w:t>g</w:t></w:r><w:r><w:rPr><w:shd w:val="clear" w:color="auto" w:fill="000000" w:themeFill="accent2" w:themeFillTint="33"/></w:rPr><w:t>h</w:t></w:r></w:p>"#;
    let d = themed_docx(body, styles, None);
    let h = d.styles.get("Heading1").unwrap();
    assert_eq!(h.chr.color, Some(TextColor::Rgb(Rgb(0x2F, 0x54, 0x96))));
    assert_eq!(h.chr.underline_color, Some(Rgb(0xED, 0x7D, 0x31)));
    let p = paras(&d);
    let at = |i: usize| p[1].props_of_char(i);
    // Colours Word writes into w:val next to these theme colours (Grid Table and heading styles).
    assert!(near(at(0).color, Rgb(0x8E, 0xAA, 0xDB)), "{:?}", at(0).color);
    assert!(near(at(1).color, Rgb(0xB4, 0xC6, 0xE7)), "{:?}", at(1).color);
    assert!(near(at(2).color, Rgb(0xD9, 0xE2, 0xF3)), "{:?}", at(2).color);
    assert!(near(at(3).color, Rgb(0x1F, 0x37, 0x63)), "{:?}", at(3).color);
    assert!(near(at(4).color, Rgb(0xC4, 0x59, 0x11)), "{:?}", at(4).color);
    // text2 maps to dk2 without a w:clrSchemeMapping; hyperlink to hlink.
    assert_eq!(at(5).color, Some(TextColor::Rgb(Rgb(0x44, 0x54, 0x6A))));
    assert_eq!(at(6).color, Some(TextColor::Rgb(Rgb(0x05, 0x63, 0xC1))));
    let fill = at(7).shading.unwrap();
    assert!(near(Some(TextColor::Rgb(fill)), Rgb(0xFB, 0xE4, 0xD5)), "{fill:?}");
}

#[test]
fn theme_colours_follow_the_colour_scheme_mapping() {
    // Styles are read before settings; the mapping still applies to them.
    let styles =
        r#"<w:style w:type="character" w:styleId="T1"><w:name w:val="T1"/><w:rPr><w:color w:val="000000" w:themeColor="text1"/></w:rPr></w:style>"#;
    let body = r#"<w:p><w:r><w:rPr><w:rStyle w:val="T1"/></w:rPr><w:t>a</w:t></w:r><w:r><w:rPr><w:color w:val="000000" w:themeColor="background1"/></w:rPr><w:t>b</w:t></w:r><w:r><w:rPr><w:color w:val="000000" w:themeColor="dark1"/></w:rPr><w:t>c</w:t></w:r></w:p>"#;
    let mapping = r#"<w:clrSchemeMapping w:bg1="dark1" w:t1="accent2" w:bg2="light2" w:t2="dark2" w:accent1="accent1" w:accent2="accent2" w:accent3="accent3" w:accent4="accent4" w:accent5="accent5" w:accent6="accent6" w:hyperlink="hyperlink" w:followedHyperlink="followedHyperlink"/>"#;
    let d = themed_docx(body, styles, Some(mapping));
    assert_eq!(d.styles.get("T1").unwrap().chr.color, Some(TextColor::Rgb(Rgb(0xED, 0x7D, 0x31))));
    let p = paras(&d);
    assert_eq!(p[0].props_of_char(1).color, Some(TextColor::Rgb(Rgb(0, 0, 0))), "background1 → dark1");
    assert_eq!(p[0].props_of_char(2).color, Some(TextColor::Rgb(Rgb(0, 0, 0))), "dark1 isn't remapped");
}

#[test]
fn theme_colour_without_a_theme_keeps_the_fixed_colour() {
    let d = read_body(r#"<w:p><w:r><w:rPr><w:color w:val="00FF00" w:themeColor="accent1" w:themeShade="BF"/></w:rPr><w:t>a</w:t></w:r></w:p>"#);
    assert_eq!(paras(&d)[0].props_of_char(0).color, Some(TextColor::Rgb(Rgb(0, 0xFF, 0))));
}

#[test]
fn theme_font_wins_over_the_fixed_font() {
    let styles = r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:rPr><w:rFonts w:ascii="Calibri Light" w:hAnsi="Calibri Light" w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorHAnsi"/></w:rPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>H</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:rFonts w:hAnsi="Arial" w:hAnsiTheme="minorHAnsi"/></w:rPr><w:t>a</w:t></w:r><w:r><w:rPr><w:rFonts w:ascii="Arial" w:asciiTheme="minorHAnsi"/></w:rPr><w:t>b</w:t></w:r><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsiTheme="majorHAnsi"/></w:rPr><w:t>c</w:t></w:r></w:p>"#;
    let d = themed_docx(body, styles, None);
    assert_eq!(d.styles.get("Heading1").unwrap().chr.font.as_deref(), Some("Major Face"));
    let p = paras(&d);
    assert_eq!(p[1].props_of_char(0).font.as_deref(), Some("Minor Face"));
    assert_eq!(p[1].props_of_char(1).font.as_deref(), Some("Minor Face"));
    // An explicit ascii font still beats a theme font given only for hAnsi.
    assert_eq!(p[1].props_of_char(2).font.as_deref(), Some("Arial"));
}

/// Issue #400: moved text (ECMA-376 §17.13.5.21–28). `w:moveFrom` / `w:moveTo` inside named
/// move ranges are the two ends of one move, paragraph marks included; the ranges are written
/// back around them. A move missing an end, or whose range never ends, is a plain deletion and
/// insertion.
#[test]
fn move_revisions_round_trip() {
    let body = r#"
<w:p><w:pPr><w:rPr><w:moveTo w:id="1" w:author="Ana" w:date="2026-10-03T00:00:00Z"/></w:rPr></w:pPr><w:moveToRangeStart w:id="2" w:name="move1" w:author="Ana" w:date="2026-10-03T00:00:00Z"/><w:moveTo w:id="3" w:author="Ana" w:date="2026-10-03T00:00:00Z"><w:r><w:t>Moved here.</w:t></w:r></w:moveTo></w:p>
<w:moveToRangeEnd w:id="2"/>
<w:p><w:r><w:t xml:space="preserve">Stays. </w:t></w:r><w:moveFromRangeStart w:id="4" w:name="move1" w:author="Ana" w:date="2026-10-03T00:00:00Z"/><w:moveFrom w:id="5" w:author="Ana" w:date="2026-10-03T00:00:00Z"><w:r><w:t>Moved here.</w:t></w:r></w:moveFrom><w:moveFromRangeEnd w:id="4"/></w:p>
<w:p><w:moveFromRangeStart w:id="6" w:name="lonely" w:author="Bo"/><w:moveFrom w:id="7" w:author="Bo"><w:r><w:t>No destination</w:t></w:r></w:moveFrom><w:moveFromRangeEnd w:id="6"/><w:moveToRangeStart w:id="8" w:name="open" w:author="Bo"/><w:moveTo w:id="9" w:author="Bo"><w:r><w:t>never ends</w:t></w:r></w:moveTo><w:moveTo w:id="10" w:author="Cy"><w:r><w:t>no range</w:t></w:r></w:moveTo><w:moveFromRangeEnd w:id="404"/></w:p>"#;
    let check = |d: &Document| {
        let p = paras(d);
        let name = |r: Option<u32>| d.move_name(r).map(str::to_string);
        assert_eq!(name(p[0].mark.ins).as_deref(), Some("move1"), "a moved paragraph mark");
        let to = p[0].runs[0].props.ins;
        assert_eq!((name(to).as_deref(), d.revisions[to.unwrap() as usize].kind), (Some("move1"), RevisionKind::Insert));
        let from = p[1].run_ranges().find(|(_, c)| c.del.is_some()).map(|(_, c)| c.del).unwrap();
        assert_eq!((name(from).as_deref(), d.revisions[from.unwrap() as usize].kind), (Some("move1"), RevisionKind::Delete));
        assert_eq!(d.revisions[from.unwrap() as usize].author, "Ana");
        // Hostile ends: each is a plain revision, still tracked.
        for (r, c) in p[2].run_ranges() {
            assert!(c.ins.or(c.del).is_some(), "{r:?} keeps its revision");
            assert_eq!(name(c.ins.or(c.del)), None, "{r:?} is no move");
        }
    };
    let d = read_body(body);
    check(&d);
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = text(&unzip(&out), "word/document.xml");
    assert_eq!(attr_values(&xml, "w:moveFromRangeStart", "w:name"), ["move1"]);
    assert_eq!(attr_values(&xml, "w:moveToRangeStart", "w:name"), ["move1"]);
    assert_eq!(attr_values(&xml, "w:moveToRangeStart", "w:author"), ["Ana"]);
    assert_eq!(attr_values(&xml, "w:moveToRangeStart", "w:id"), attr_values(&xml, "w:moveToRangeEnd", "w:id"));
    assert_eq!(attr_values(&xml, "w:moveFromRangeStart", "w:id"), attr_values(&xml, "w:moveFromRangeEnd", "w:id"));
    assert!(xml.contains("<w:moveFrom ") && !xml.contains("<w:delText xml:space=\"preserve\">Moved"), "moved-away text stays in w:t");
    // The range around the moved paragraph includes its mark: it ends in the next paragraph.
    let to_start = xml.find("<w:moveToRangeStart").unwrap();
    let to_end = xml.find("<w:moveToRangeEnd").unwrap();
    assert!(xml[to_start..to_end].contains("</w:p>"), "{xml}");
    check(&wordcraft_docx::read(&out).unwrap());
}

/// A document's own footnote separators and continuation notice are kept (ECMA-376 §17.11), and
/// written back as Word numbers them: -1, 0 and 1, the notes after them.
#[test]
fn note_separators_and_continuation_notice_round_trip() {
    let footnotes = format!(
        r#"<w:footnotes {W_NS}><w:footnote w:type="separator" w:id="-1"><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:t xml:space="preserve">More: </w:t></w:r><w:r><w:continuationSeparator/></w:r></w:p></w:footnote><w:footnote w:type="continuationNotice" w:id="1"><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>(continued)</w:t></w:r></w:p></w:footnote><w:footnote w:id="2"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> The note.</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    let body = r#"<w:p><w:r><w:t>Text</w:t></w:r><w:r><w:footnoteReference w:id="2"/></w:r></w:p>"#;
    let bytes = docx(body, &[("rId1", "footnotes", "footnotes.xml")], &[("word/footnotes.xml", &footnotes)]);
    let check = |d: &Document| {
        let s = &d.footnote_separators;
        let first = |b: &Option<wordcraft_doc::Blocks>| b.as_ref().unwrap()[0].as_para().unwrap().clone();
        let sep = first(&s.separator);
        assert_eq!(sep.objects.iter().map(InlineObject::as_note_separator).collect::<Vec<_>>(), vec![Some(false)]);
        assert_eq!(sep.props.space_after, Some(0.0));
        let cont = first(&s.continuation_separator);
        assert_eq!(cont.plain_text(), "More: ");
        assert_eq!(cont.objects.iter().map(InlineObject::as_note_separator).collect::<Vec<_>>(), vec![Some(true)]);
        let notice = first(&s.continuation_notice);
        assert_eq!(notice.plain_text(), "(continued)");
        assert_eq!(notice.props.align, Some(Align::Right));
        assert!(d.endnote_separators.is_empty());
        // The note itself is read as before.
        let InlineObject::NoteRef { id, .. } = paras(d)[0].objects[0] else { panic!() };
        assert_eq!(d.parts[&id].blocks[0].as_para().unwrap().plain_text(), " The note.");
        assert_eq!(d.parts.len(), 1, "special notes aren't notes");
    };
    let d = wordcraft_docx::read(&bytes).unwrap();
    check(&d);

    let out = wordcraft_docx::write(&d).unwrap();
    let part = |name: &str| {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&out)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut z.by_name(name).unwrap(), &mut xml).unwrap();
        xml
    };
    let notes = part("word/footnotes.xml");
    for (kind, id) in [("separator", "-1"), ("continuationSeparator", "0"), ("continuationNotice", "1")] {
        assert!(notes.contains(&format!(r#"<w:footnote w:type="{kind}" w:id="{id}">"#)), "{kind}: {notes}");
    }
    assert!(notes.contains(r#"<w:footnote w:id="2">"#), "the note follows the notice: {notes}");
    assert!(part("word/document.xml").contains(r#"<w:footnoteReference w:id="2"/>"#));
    let settings = part("word/settings.xml");
    assert!(settings.contains(r#"<w:footnote w:id="-1"/><w:footnote w:id="0"/><w:footnote w:id="1"/>"#), "{settings}");
    check(&wordcraft_docx::read(&out).unwrap());
}

/// Issue #534: East Asian number formats are read, labelled and written back as they were; a
/// value the format doesn't define is decimal, and a level without `w:ind` has no indent.
#[test]
fn east_asian_number_formats_read_label_and_round_trip() {
    use wordcraft_doc::numbering::Counters;
    use wordcraft_doc::section::NumFormat;
    let lvl = |i: u8, fmt: &str, text: &str| {
        format!(r#"<w:lvl w:ilvl="{i}"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/><w:suff w:val="nothing"/><w:lvlText w:val="{text}"/></w:lvl>"#)
    };
    let levels = [
        lvl(0, "chineseCounting", "%1、"),
        lvl(1, "chineseCountingThousand", "第%2章"),
        lvl(2, "japaneseLegal", "（%3）"),
        lvl(3, "decimalEnclosedCircle", "%4"),
        lvl(4, "hebrew1", "%5."),
        lvl(5, "notAFormat", "%6."),
    ]
    .concat();
    let numbering = format!(
        r#"<w:numbering {W_NS}><w:abstractNum w:abstractNumId="0">{levels}</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
    );
    let item = |lv: u8| format!(r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{lv}"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
    let body: String = [0, 0, 0, 1, 2, 3, 4, 5].map(item).concat();
    let bytes = docx(&body, &[("rId1", "numbering", "numbering.xml")], &[("word/numbering.xml", &numbering)]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    let formats = |d: &Document| (0..6).map(|i| d.numbering.level(1, i).map(|l| l.format)).collect::<Vec<_>>();
    let want = [
        NumFormat::ChineseCounting,
        NumFormat::ChineseCountingThousand,
        NumFormat::JapaneseLegal,
        NumFormat::DecimalEnclosedCircle,
        NumFormat::Hebrew1,
        NumFormat::Decimal,
    ]
    .map(Some);
    assert_eq!(formats(&d), want);
    assert_eq!(d.numbering.level(1, 0).map(|l| (l.indent, l.hanging)), Some((0.0, 0.0)));
    let mut c = Counters::default();
    let labels: Vec<String> =
        paras(&d).iter().filter_map(|p| p.props.numbering).filter_map(|n| c.next_label(&d.numbering, n.num, n.level)).map(|l| l.0).collect();
    assert_eq!(labels, ["一、", "二、", "三、", "第一章", "（壱）", "①", "1.", "1."]);
    // Saved and read again, every format is the one the file had (hebrew1 too, which shows as
    // decimal for now).
    let out = wordcraft_docx::write(&d).unwrap();
    let back = wordcraft_docx::read(&out).unwrap();
    assert_eq!(formats(&back), want);
}

/// #464: a bookmark whose end follows the document's final table keeps its end (it goes into
/// the table's last cell) and is saved as a matched pair.
#[test]
fn bookmark_around_final_table_keeps_its_end() {
    let body = r#"<w:bookmarkStart w:id="7" w:name="TableRange"/><w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:bookmarkEnd w:id="7"/><w:sectPr/>"#;
    let d = read_body(body);
    let marks = |d: &Document| {
        let Block::Table(t) = &*d.body[0] else { panic!("{:?}", d.body[0]) };
        t.rows[0].cells[0].blocks[0].as_para().unwrap().objects.clone()
    };
    let want = vec![InlineObject::BookmarkStart { name: "TableRange".into() }, InlineObject::BookmarkEnd { name: "TableRange".into() }];
    assert_eq!(marks(&d), want);
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(marks(&again), want);
    assert_eq!(again.bookmarks().len(), 1);
}

/// #465: a field whose cached result is an embedded picture (`INCLUDEPICTURE`, simple, complex or
/// locked) keeps the picture through reading and saving; the linked file is never read.
#[test]
fn includepicture_keeps_its_embedded_picture() {
    let png: &[u8] = b"\x89PNG\r\n\x1a\nstand-in";
    let pic = r#"<w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="P"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rIdPic"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#;
    let cases = [
        (format!(r#"<w:fldSimple w:instr="INCLUDEPICTURE &quot;picture.png&quot;">{pic}</w:fldSimple>"#), false),
        (format!(r#"<w:fldSimple w:instr="INCLUDEPICTURE &quot;picture.png&quot;" w:fldLock="1">{pic}</w:fldSimple>"#), true),
        (
            format!(
                r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> INCLUDEPICTURE "picture.png" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{pic}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
            ),
            false,
        ),
    ];
    for (field, locked) in cases {
        let bytes = docx(&format!("<w:p>{field}</w:p><w:sectPr/>"), &[("rIdPic", "image", "media/picture.png")], &[]);
        let bytes = {
            // `docx` takes text parts; add the picture bytes as a binary part.
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
            for i in 0..z.len() {
                let mut f = z.by_index(i).unwrap();
                let mut v = Vec::new();
                std::io::Read::read_to_end(&mut f, &mut v).unwrap();
                entries.push((f.name().to_string(), v));
            }
            entries.push(("word/media/picture.png".into(), png.to_vec()));
            zip(&entries.iter().map(|(n, v)| (n.as_str(), v.as_slice())).collect::<Vec<_>>())
        };
        let check = |d: &Document| {
            let p = paras(d);
            let objs = &p[0].objects;
            assert_eq!(objs.len(), 3, "{objs:?}");
            assert_eq!(objs[0], InlineObject::FieldStart { instr: "INCLUDEPICTURE \"picture.png\"".into(), locked, dirty: false });
            let InlineObject::Image { media, .. } = &objs[1] else { panic!("{objs:?}") };
            assert_eq!(d.media.get(media).unwrap().as_slice(), png);
            assert_eq!(objs[2], InlineObject::FieldEnd);
        };
        let d = wordcraft_docx::read(&bytes).unwrap();
        check(&d);
        check(&wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap());
    }
}

/// One anchored Word shape (`wps:wsp` inside `mc:AlternateContent`, as Word writes them) with
/// shape properties `sppr`, style `style` and extra `wsp` content.
fn anchored_wsp(name: &str, cx: u32, cy: u32, sppr: &str, style: &str, extra: &str) -> String {
    format!(
        r#"<w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>12700</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>25400</wp:posOffset></wp:positionV><wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="1" name="{name}"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp>{extra}<wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>{sppr}</wps:spPr>{style}<wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:line from="0,0" to="1pt,2pt"/></w:pict></mc:Fallback></mc:AlternateContent></w:r>"#
    )
}

/// #611: diagrams drawn with Word's shapes were mostly missing. Connectors and boxes take their
/// outline and fill from the theme through their style references, or name preset and scheme
/// colours; all of these are read, with line widths and arrowheads.
#[test]
fn theme_styled_connectors_and_callout_boxes_keep_their_paint() {
    let style = |ln: u8, fill: u8, clr: &str| {
        format!(
            r#"<wps:style><a:lnRef idx="{ln}"><a:schemeClr val="{clr}"/></a:lnRef><a:fillRef idx="{fill}"><a:schemeClr val="{clr}"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="{clr}"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="tx1"/></a:fontRef></wps:style>"#
        )
    };
    let body = [
        // A connector with only a style: accent4, the theme's thinnest line, an arrowhead.
        anchored_wsp("Arrow", 673100, 889000, r#"<a:prstGeom prst="straightConnector1"><a:avLst/></a:prstGeom><a:ln><a:tailEnd type="triangle"/></a:ln>"#, &style(1, 0, "accent4"), "<wps:cNvCnPr/>"),
        // A callout text box: scheme fill, preset black outline.
        anchored_wsp(
            "Text Box",
            3073400,
            673100,
            r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="lt1"/></a:solidFill><a:ln w="6350"><a:solidFill><a:prstClr val="black"/></a:solidFill></a:ln>"#,
            "",
            r#"<wps:cNvSpPr txBox="1"/><wps:txbx><w:txbxContent><w:p><w:r><w:t>Callout</w:t></w:r></w:p></w:txbxContent></wps:txbx>"#,
        ),
        // A filled rectangle from its style alone.
        anchored_wsp("Rect", 292100, 647700, r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"#, &style(2, 1, "accent1"), "<wps:cNvSpPr/>"),
        // An elbow connector.
        anchored_wsp("Elbow", 1270000, 635000, r#"<a:prstGeom prst="bentConnector3"><a:avLst/></a:prstGeom>"#, &style(1, 0, "accent2"), "<wps:cNvCnPr/>"),
        // A rounded-rectangle callout.
        anchored_wsp("Callout", 1270000, 635000, r#"<a:prstGeom prst="wedgeRoundRectCallout"><a:avLst/></a:prstGeom>"#, &style(2, 1, "accent1"), "<wps:cNvSpPr/>"),
    ]
    .concat();
    let d = read_body(&format!("<w:p>{body}</w:p>"));
    let theme = &d.settings.theme_colors;
    let objs = &paras(&d)[0].objects;
    assert_eq!(objs.len(), 5);
    let shape = |i: usize| match &objs[i] {
        InlineObject::Shape { kind, fill, stroke, stroke_width, extra, story, .. } => {
            (*kind, *fill, *stroke, *stroke_width, extra.arrow_end, story.is_some())
        }
        o => panic!("{o:?}"),
    };
    assert_eq!(shape(0), (ShapeKind::StraightConnector, None, Some(theme[7]), 0.5, true, false));
    assert_eq!(shape(1), (ShapeKind::TextBox, Some(theme[1]), Some(Rgb(0, 0, 0)), 0.5, false, true));
    assert_eq!(shape(2), (ShapeKind::Rectangle, Some(theme[4]), Some(theme[4]), 1.0, false, false));
    assert_eq!(shape(3), (ShapeKind::ElbowConnector, None, Some(theme[5]), 0.5, false, false));
    assert_eq!(shape(4).0, ShapeKind::RoundedRectangle);
    // The themed outline and the arrowhead survive saving.
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert!(matches!(&paras(&again)[0].objects[0], InlineObject::Shape { stroke, extra, .. } if *stroke == Some(theme[7]) && extra.arrow_end));
}

/// A rounded rectangle's corner is its `adj` (100000ths of the shorter side), read, capped and
/// written back; without one, and for presets only drawn as rounded rectangles, the default.
#[test]
fn rounded_rectangle_corners_are_read_and_saved() {
    let geom = |prst: &str, av: &str| {
        format!(r#"<a:prstGeom prst="{prst}"><a:avLst>{av}</a:avLst></a:prstGeom><a:solidFill><a:srgbClr val="D9D9D9"/></a:solidFill>"#)
    };
    let gd = |fmla: &str| format!(r#"<a:gd name="adj" fmla="{fmla}"/>"#);
    let body = [
        geom("roundRect", &gd("val 3565")),
        geom("roundRect", ""),
        geom("roundRect", &gd("val 90000")),
        geom("roundRect", &gd("val -20")),
        geom("roundRect", &gd("*/ 1 2 3")),
        geom("roundRect", r#"<a:gd name="adj1" fmla="val 100"/>"#),
        geom("wedgeRoundRectCallout", &gd("val 3565")),
    ]
    .iter()
    .map(|sppr| anchored_wsp("Rounded", 2540000, 1270000, sppr, "", "<wps:cNvSpPr/>"))
    .collect::<String>();
    let corners = |d: &Document| -> Vec<(ShapeKind, Option<u32>)> {
        paras(d)[0]
            .objects
            .iter()
            .filter_map(|o| match o {
                InlineObject::Shape { kind, extra, .. } => Some((*kind, extra.corner)),
                _ => None,
            })
            .collect()
    };
    let rounded = ShapeKind::RoundedRectangle;
    let expected =
        vec![(rounded, Some(3565)), (rounded, None), (rounded, Some(50_000)), (rounded, Some(0)), (rounded, None), (rounded, None), (rounded, None)];
    let d = read_body(&format!("<w:p>{body}</w:p>"));
    assert_eq!(corners(&d), expected);
    let bytes = wordcraft_docx::write(&d).unwrap();
    let mut xml = String::new();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"<a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 3565"/></a:avLst></a:prstGeom>"#), "{xml}");
    assert_eq!(corners(&wordcraft_docx::read(&bytes).unwrap()), expected);
}

/// #611: legacy VML lines and rectangles (documents without DrawingML).
#[test]
fn vml_lines_and_rectangles() {
    let body = r##"<w:p><w:r><w:pict><v:line style="position:absolute;flip:y;z-index:1" from="10pt,20pt" to="40pt,60pt" strokecolor="#4472c4 [3204]" strokeweight=".5pt"><v:stroke endarrow="block"/></v:line></w:pict></w:r>
<w:r><w:pict><v:rect style="position:absolute;margin-left:5pt;margin-top:6pt;width:30pt;height:20pt" fillcolor="#ff0000" stroked="f"/></w:pict></w:r></w:p>"##;
    let d = read_body(body);
    let objs = &paras(&d)[0].objects;
    assert_eq!(objs.len(), 2, "{objs:?}");
    match &objs[0] {
        InlineObject::Shape { kind: ShapeKind::Line, w, h, stroke, stroke_width, float, extra, .. } => {
            assert_eq!((*w, *h, *stroke, *stroke_width), (30.0, 40.0, Some(Rgb(0x44, 0x72, 0xC4)), 0.5));
            assert_eq!((float.x, float.y, float.flip_h, float.flip_v), (10.0, 20.0, false, true));
            assert!(extra.arrow_end && !extra.arrow_start);
        }
        o => panic!("{o:?}"),
    }
    assert!(matches!(&objs[1], InlineObject::Shape { kind: ShapeKind::Rectangle, fill: Some(Rgb(255, 0, 0)), stroke: None, w: 30.0, h: 20.0, .. }));
}

/// The right arrow's shapetype as Word writes it (ECMA-376 Part 4 `v:shapetype`, `o:spt` 13).
const VML_RIGHT_ARROW_TYPE: &str = r#"<v:shapetype id="_x0000_t13" coordsize="21600,21600" o:spt="13" adj="16200,5400" path="m@0,l@0@1,0@1,0@2@0@2@0,21600,21600,10800xe"><v:stroke joinstyle="miter"/><v:formulas><v:f eqn="val #0"/><v:f eqn="val #1"/></v:formulas><v:path o:connecttype="custom"/></v:shapetype>"#;

/// #1377: a VML shape drawn from a preset shapetype (`type="#_x0000_t13"`) is the DrawingML
/// preset WordCraft draws, with its fill, outline and size: from the shapetype beside it, from
/// Word's shapetype id when the shapetype was written at an earlier use, or from its own `o:spt`.
#[test]
fn vml_preset_shapes_are_drawn() {
    let pict = |inner: &str| format!("<w:p><w:r><w:pict>{inner}</w:pict></w:r></w:p>");
    let body = [
        pict(&format!(r##"{VML_RIGHT_ARROW_TYPE}<v:shape id="_x0000_i1026" type="#_x0000_t13" style="width:120pt;height:40pt" fillcolor="#ffa500"/>"##)),
        pict(r##"<v:shape type="#_x0000_t13" style="position:absolute;margin-left:10pt;margin-top:5pt;width:60pt;height:20pt;z-index:2" fillcolor="#00ff00" strokecolor="#0000ff" strokeweight="2pt"/>"##),
        pict(r##"<v:shape o:spt="74" style="width:30pt;height:30pt" fillcolor="red" stroked="f"/>"##),
        pict(r##"<v:shapetype id="tri" o:spt="5" coordsize="21600,21600"/><v:shape type="#tri" style="width:20pt;height:10pt" filled="f"/>"##),
        pict(r##"<v:shape type="#_x0000_t4" style="width:50pt;height:50pt" fillcolor="#ffff00"><v:textbox><w:txbxContent><w:p><w:r><w:t>yes?</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape>"##),
        // A preset WordCraft has no shape for is drawn as the nearest one, as for DrawingML: a
        // right triangle (`o:spt` 6) as a triangle.
        pict(r##"<v:shape type="#_x0000_t6" style="width:20pt;height:20pt" fillcolor="red"/>"##),
        // Its own path wins over the shapetype's: not the preset.
        pict(r##"<v:shape type="#_x0000_t13" path="m,l21600,21600e" style="width:20pt;height:20pt"/>"##),
        // Lines from a shapetype (`o:spt` 20 and Word's line tool's 32) take the size of their box.
        pict(r##"<v:shape type="#_x0000_t20" style="width:40pt;height:10pt"/>"##),
        pict(r##"<v:shapetype id="_x0000_t32" coordsize="21600,21600" o:spt="32" o:oned="t" path="m,l21600,21600e" filled="f"/><v:shape type="#_x0000_t32" style="position:absolute;width:30pt;height:15pt" strokecolor="#ff0000"><v:stroke endarrow="block"/></v:shape>"##),
        // A text box's shapetype without text is still a text box.
        pict(r##"<v:shape type="#_x0000_t202" style="width:30pt;height:15pt"/>"##),
        // A shape that isn't a preset keeps its text, in a text box.
        pict(r##"<v:shape o:spt="0" style="width:50pt;height:50pt"><v:textbox><w:txbxContent><w:p/></w:txbxContent></v:textbox></v:shape>"##),
        // Text in a preset with neither fill nor outline is kept.
        pict(r##"<v:shape type="#_x0000_t3" style="width:50pt;height:50pt" filled="f" stroked="f"><v:textbox><w:txbxContent><w:p/></w:txbxContent></v:textbox></v:shape>"##),
        pict(r##"<v:oval style="width:50pt;height:40pt"><v:textbox><w:txbxContent><w:p/></w:txbxContent></v:textbox></v:oval>"##),
        // Paint the shape leaves out comes from its shapetype.
        pict(r##"<v:shapetype id="_x0000_t1" o:spt="1" fillcolor="#0000ff" stroked="f"/><v:shape type="#_x0000_t1" style="width:9pt;height:9pt"/>"##),
        r##"<w:p><w:r><w:pict><v:shape type="#_x0000_t2" style="width:9pt;height:9pt"/></w:pict></w:r><w:r><w:pict><v:shape type="#_x0000_t12" style="width:9pt;height:9pt"/></w:pict></w:r></w:p>"##.to_string(),
        pict(r##"<v:shape type="#_x0000_t109" style="width:9pt;height:9pt"/>"##),
        pict(r##"<v:shape type="#_x0000_t110" style="width:9pt;height:9pt"/>"##),
        pict(r##"<v:shape type="#_x0000_t120" style="width:9pt;height:9pt"/>"##),
        // Nearest shapes: left arrow, rectangular callout, four-point star.
        pict(r##"<v:shape type="#_x0000_t66" style="width:60pt;height:20pt" fillcolor="#ffa500"/>"##),
        pict(r##"<v:shape type="#_x0000_t61" style="width:60pt;height:20pt"/>"##),
        pict(r##"<v:shape type="#_x0000_t187" style="width:20pt;height:20pt"/>"##),
        // Not drawn: WordArt text (136) and a picture frame without its picture (75).
        pict(r##"<v:shape type="#_x0000_t136" style="width:60pt;height:20pt" fillcolor="silver"/>"##),
        pict(r##"<v:shape type="#_x0000_t75" style="width:60pt;height:20pt"/>"##),
    ]
    .concat();
    let d = read_body(&body);
    let ps = paras(&d);
    let shape = |i: usize| match ps[i].objects.first() {
        Some(InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, story, .. }) => {
            Some((*kind, *w, *h, *fill, *stroke, *stroke_width, float.wrap == Wrap::Inline, story.is_some()))
        }
        _ => None,
    };
    let kind = |i: usize| shape(i).map(|s| (s.0, s.7));
    let (black, white) = (Some(Rgb::BLACK), Some(Rgb::WHITE));
    assert_eq!(shape(0), Some((ShapeKind::Arrow, 120.0, 40.0, Some(Rgb(0xFF, 0xA5, 0x00)), black, 0.75, true, false)));
    assert_eq!(shape(1), Some((ShapeKind::Arrow, 60.0, 20.0, Some(Rgb(0, 0xFF, 0)), Some(Rgb(0, 0, 0xFF)), 2.0, false, false)));
    assert_eq!(shape(2), Some((ShapeKind::Heart, 30.0, 30.0, Some(Rgb(0xFF, 0, 0)), None, 0.75, true, false)));
    assert_eq!(shape(3), Some((ShapeKind::Triangle, 20.0, 10.0, None, black, 0.75, true, false)));
    // A preset holding text keeps its geometry around the text.
    assert_eq!(shape(4), Some((ShapeKind::Diamond, 50.0, 50.0, Some(Rgb(0xFF, 0xFF, 0)), black, 0.75, true, true)));
    assert_eq!(shape(5), Some((ShapeKind::RightTriangle, 20.0, 20.0, Some(Rgb(0xFF, 0, 0)), black, 0.75, true, false)));
    // Its own path is drawn as a freeform, not as the shapetype's arrow.
    assert_eq!(kind(6), Some((ShapeKind::Freeform, false)));
    assert_eq!(shape(7), Some((ShapeKind::Line, 40.0, 10.0, None, black, 0.75, true, false)));
    assert_eq!(shape(8), Some((ShapeKind::Line, 30.0, 15.0, None, Some(Rgb(0xFF, 0, 0)), 0.75, false, false)));
    assert!(matches!(ps[8].objects.first(), Some(InlineObject::Shape { extra, .. }) if extra.arrow_end && !extra.arrow_start));
    assert_eq!(shape(9), Some((ShapeKind::TextBox, 30.0, 15.0, white, black, 0.75, true, false)));
    assert_eq!(kind(10), Some((ShapeKind::TextBox, true)));
    assert_eq!(shape(11), Some((ShapeKind::Ellipse, 50.0, 50.0, None, None, 0.75, true, true)));
    assert_eq!(kind(12), Some((ShapeKind::Ellipse, true)));
    assert_eq!(shape(13), Some((ShapeKind::Rectangle, 9.0, 9.0, Some(Rgb(0, 0, 0xFF)), None, 0.75, true, false)));
    let kinds: Vec<_> = ps[14].objects.iter().filter_map(|o| if let InlineObject::Shape { kind, .. } = o { Some(*kind) } else { None }).collect();
    assert_eq!(kinds, [ShapeKind::RoundedRectangle, ShapeKind::Star]);
    assert_eq!(kind(15), Some((ShapeKind::Rectangle, false)));
    assert_eq!(kind(16), Some((ShapeKind::Diamond, false)));
    assert_eq!(kind(17), Some((ShapeKind::Ellipse, false)));
    assert_eq!(shape(18), Some((ShapeKind::Arrow, 60.0, 20.0, Some(Rgb(0xFF, 0xA5, 0x00)), black, 0.75, true, false)));
    assert_eq!(kind(19), Some((ShapeKind::Rectangle, false)));
    assert_eq!(kind(20), Some((ShapeKind::Star, false)));
    assert_eq!(shape(21), None);
    assert_eq!(shape(22), None);
}

/// #1377: hostile shape type numbers and references are skipped, never a panic.
#[test]
fn vml_preset_hostile_types_never_panic() {
    let shapes = [
        r##"<v:shape o:spt="99999999999999999999999" style="width:9pt;height:9pt"/>"##,
        r##"<v:shape o:spt="-13" style="width:9pt;height:9pt"/>"##,
        r##"<v:shape o:spt="" type="#_x0000_t" style="width:9pt;height:9pt"/>"##,
        r##"<v:shape type="#_x0000_t13abc" style="width:9pt;height:9pt"/>"##,
        r##"<v:shape type="#" style="width:9pt;height:9pt"/>"##,
        r##"<v:shape type="#_x0000_tépée" style="width:9pt;height:9pt"/>"##,
        r##"<v:shapetype id="x" o:spt="nope"/><v:shape type="#x" style="width:9pt;height:9pt"/>"##,
        r##"<v:shape type="#_x0000_t13" style="width:-1e39pt;height:NaNpt" fillcolor="#zzz" strokeweight="1e30pt"/>"##,
    ];
    let body: String = shapes.iter().map(|s| format!("<w:p><w:r><w:pict>{s}</w:pict></w:r></w:p>")).collect();
    let d = read_body(&body);
    for (i, p) in paras(&d).iter().enumerate().take(7) {
        assert!(p.objects.is_empty(), "shape {i}: {:?}", p.objects);
    }
    match paras(&d)[7].objects.first() {
        Some(InlineObject::Shape { kind: ShapeKind::Arrow, w, h, stroke_width, .. }) => {
            assert!(w.is_finite() && h.is_finite() && *w >= 0.0 && *h >= 0.0 && *stroke_width <= 100.0)
        }
        o => panic!("{o:?}"),
    }
}

/// #1377: a VML preset shape is kept on save, as its DrawingML preset with its fill.
#[test]
fn vml_preset_shape_survives_saving() {
    let body = format!(
        r##"<w:p><w:r><w:pict>{VML_RIGHT_ARROW_TYPE}<v:shape type="#_x0000_t13" style="width:120pt;height:40pt" fillcolor="#ffa500"/></w:pict></w:r></w:p>"##
    );
    let saved = wordcraft_docx::write(&read_body(&body)).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved.as_slice())).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"prst="rightArrow""#), "{xml}");
    let again = wordcraft_docx::read(&saved).unwrap();
    assert!(
        matches!(
            &paras(&again)[0].objects[..],
            [InlineObject::Shape { kind: ShapeKind::Arrow, w: 120.0, h: 40.0, fill: Some(Rgb(0xFF, 0xA5, 0x00)), stroke: Some(Rgb::BLACK), .. }]
        ),
        "{:?}",
        paras(&again)[0].objects
    );
}

/// #1377: a VML preset drawn as its nearest shape (a left arrow, `o:spt` 66) is kept on save as
/// that shape's DrawingML preset, as a DrawingML left arrow is.
#[test]
fn vml_nearest_preset_shape_survives_saving() {
    let body = r##"<w:p><w:r><w:pict><v:shape type="#_x0000_t66" style="width:60pt;height:20pt" fillcolor="#ffa500"/></w:pict></w:r></w:p>"##;
    let saved = wordcraft_docx::write(&read_body(body)).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved.as_slice())).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"prst="rightArrow""#), "{xml}");
    let again = wordcraft_docx::read(&saved).unwrap();
    assert!(
        matches!(
            &paras(&again)[0].objects[..],
            [InlineObject::Shape { kind: ShapeKind::Arrow, w: 60.0, h: 20.0, fill: Some(Rgb(0xFF, 0xA5, 0x00)), .. }]
        ),
        "{:?}",
        paras(&again)[0].objects
    );
}

/// #743: a cover page from Word: a group holding a rectangle filled through its style, a
/// freeform band and a title box, beside a box that resizes to fit its two lines and has no
/// internal margins. The fills, the band's outline, the margins and the resizing are all read
/// and survive saving.
#[test]
fn cover_page_group_fills_freeforms_and_text_box_margins() {
    let style = r#"<wps:style><a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="50000"/></a:schemeClr></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></wps:style>"#;
    let member = |id: u32, txbox: bool, off: (u32, u32), ext: (u32, u32), geom: &str, paint: &str, inner: &str| {
        let nv = if txbox { r#"<wps:cNvSpPr txBox="1"/>"# } else { "<wps:cNvSpPr/>" };
        format!(
            r#"<wps:wsp><wps:cNvPr id="{id}" name="Member {id}"/>{nv}<wps:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm>{geom}{paint}</wps:spPr>{style}{inner}</wps:wsp>"#,
            off.0, off.1, ext.0, ext.1
        )
    };
    let rect = r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"#;
    let band = r#"<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="0" t="0" r="r" b="b"/><a:pathLst><a:path w="100" h="100"><a:moveTo><a:pt x="0" y="100"/></a:moveTo><a:lnTo><a:pt x="100" y="0"/></a:lnTo><a:lnTo><a:pt x="100" y="100"/></a:lnTo><a:close/></a:path></a:pathLst></a:custGeom>"#;
    let title = r#"<wps:txbx><w:txbxContent><w:p><w:r><w:t>Cover Title</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr rot="0" vert="horz" wrap="square" lIns="0" tIns="0" rIns="0" bIns="0" anchor="t"><a:noAutofit/></wps:bodyPr>"#;
    let members = [
        member(11, false, (0, 0), (5486400, 5486400), rect, r#"<a:ln><a:noFill/></a:ln>"#, "<wps:bodyPr/>"),
        member(
            12,
            false,
            (914400, 4572000),
            (4572000, 914400),
            band,
            r#"<a:solidFill><a:schemeClr val="bg1"/></a:solidFill><a:ln><a:noFill/></a:ln>"#,
            "<wps:bodyPr/>",
        ),
        member(13, true, (457200, 457200), (4572000, 1828800), rect, r#"<a:noFill/><a:ln><a:noFill/></a:ln>"#, title),
    ]
    .concat();
    let group = format!(
        r##"<w:r><mc:AlternateContent><mc:Choice Requires="wpg"><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="1" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="margin"><wp:align>center</wp:align></wp:positionH><wp:positionV relativeFrom="margin"><wp:align>top</wp:align></wp:positionV><wp:extent cx="5486400" cy="5486400"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="1" name="Group 1"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="5486400" cy="5486400"/><a:chOff x="0" y="0"/><a:chExt cx="5486400" cy="5486400"/></a:xfrm></wpg:grpSpPr>{members}</wpg:wgp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:rect style="width:10pt;height:10pt" fillcolor="#156082 [3204]" stroked="f"/></w:pict></mc:Fallback></mc:AlternateContent></w:r>"##
    );
    let subtitle = anchored_wsp(
        "Subtitle",
        5753100,
        420000,
        r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln>"#,
        "",
        r#"<wps:cNvSpPr txBox="1"/><wps:txbx><w:txbxContent><w:p><w:r><w:t>Plan</w:t></w:r></w:p><w:p><w:r><w:t>Author</w:t></w:r></w:p></w:txbxContent></wps:txbx>"#,
    )
    // `anchored_wsp` writes an empty `wps:bodyPr`: give this one margins and Resize to fit.
    .replace("<wps:bodyPr/>", r#"<wps:bodyPr lIns="0" tIns="12700" rIns="0" bIns="0"><a:spAutoFit/></wps:bodyPr>"#);
    let check = |d: &Document| {
        let theme = &d.settings.theme_colors;
        let objs = &paras(d)[0].objects;
        let InlineObject::Group { children, .. } = &objs[0] else { panic!("{:?}", objs[0]) };
        assert_eq!(children.len(), 3);
        let shape = |i: usize| match &children[i].obj {
            InlineObject::Shape { kind, fill, freeform, story, .. } => (*kind, *fill, freeform.is_some(), *story),
            o => panic!("{o:?}"),
        };
        let (kind, fill, free, _) = shape(0);
        assert_eq!((kind, fill, free), (ShapeKind::Rectangle, Some(theme[4]), false), "the style's accent fill");
        let (kind, fill, free, _) = shape(1);
        assert_eq!((kind, fill, free), (ShapeKind::Freeform, Some(theme[1]), true), "a freeform in a group keeps its outline");
        let (kind, _, _, story) = shape(2);
        assert_eq!(kind, ShapeKind::TextBox);
        let body = d.parts[&story.unwrap()].body;
        assert_eq!((body.inset, body.auto_fit), (Some([0; 4]), false), "the title box has no margins");
        let InlineObject::Shape { story: Some(sub), .. } = &objs[1] else { panic!("{:?}", objs[1]) };
        let body = d.parts[sub].body;
        assert_eq!((body.inset, body.auto_fit), (Some([0, 12700, 0, 0]), true), "the subtitle resizes to fit its text");
    };
    let d = read_body(&format!("<w:p>{group}{subtitle}</w:p>"));
    check(&d);
    check(&wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap());
}

/// #886: saving dropped Word's DrawingML shapes (rectangles, ellipses, triangles, arrows,
/// stars, connectors and their theme fills). Each kind the report lists is read, saved and read
/// back with the same kind, fill, outline, size and position, and the saved part still holds a
/// `wps:wsp` with its preset geometry.
#[test]
fn drawingml_shapes_survive_saving() {
    let style = r#"<wps:style><a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="15000"/></a:schemeClr></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></wps:style>"#;
    let geom = |prst: &str| format!(r#"<a:prstGeom prst="{prst}"><a:avLst/></a:prstGeom>"#);
    let presets = ["rect", "ellipse", "triangle", "rightArrow", "star5"];
    let mut body: String = presets.iter().map(|p| anchored_wsp(p, 2009775, 819150, &geom(p), style, "<wps:cNvSpPr/>")).collect();
    body.push_str(&anchored_wsp(
        "Connector",
        2362200,
        390525,
        r#"<a:prstGeom prst="straightConnector1"><a:avLst/></a:prstGeom><a:ln><a:tailEnd type="triangle"/></a:ln>"#,
        style,
        "<wps:cNvCnPr/>",
    ));
    body.push_str(&anchored_wsp(
        "Text Box",
        2324100,
        571500,
        r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="lt1"/></a:solidFill><a:ln w="9525"><a:solidFill><a:srgbClr val="7F7F7F"/></a:solidFill></a:ln>"#,
        "",
        r#"<wps:cNvSpPr txBox="1"/><wps:txbx><w:txbxContent><w:p><w:r><w:t>Floating text box</w:t></w:r></w:p></w:txbxContent></wps:txbx>"#,
    ));
    let shapes = |d: &Document| {
        paras(d)[0]
            .objects
            .iter()
            .map(|o| match o {
                InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, extra, .. } => {
                    (*kind, *w, *h, *fill, *stroke, *stroke_width, float.x, float.y, float.wrap, extra.arrow_end)
                }
                o => panic!("{o:?}"),
            })
            .collect::<Vec<_>>()
    };
    let d = read_body(&format!("<w:p>{body}</w:p>"));
    let accent = d.settings.theme_colors[4];
    let read = shapes(&d);
    let kinds: Vec<ShapeKind> = read.iter().map(|s| s.0).collect();
    assert_eq!(
        kinds,
        [
            ShapeKind::Rectangle,
            ShapeKind::Ellipse,
            ShapeKind::Triangle,
            ShapeKind::Arrow,
            ShapeKind::Star,
            ShapeKind::StraightConnector,
            ShapeKind::TextBox
        ]
    );
    for s in &read[..5] {
        assert_eq!(s.3, Some(accent), "{:?} is filled from the theme", s.0);
        assert!(s.4.is_some() && s.5 > 0.0, "{:?} has its outline", s.0);
        assert_eq!((s.6, s.7, s.8), (1.0, 2.0, Wrap::InFrontOfText), "{:?} keeps its place", s.0);
    }
    assert!(read[5].9, "the connector keeps its arrowhead");
    let bytes = wordcraft_docx::write(&d).unwrap();
    let again = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(shapes(&again), read, "every shape survives saving");
    assert_eq!(part_text_box(&again), "Floating text box");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert_eq!(xml.matches("<wps:wsp>").count(), 7);
    for p in presets.iter().chain(&["straightConnector1"]) {
        assert!(xml.contains(&format!(r#"<a:prstGeom prst="{p}">"#)), "{p} is written");
    }
}

/// The text of the first text box in `d`.
fn part_text_box(d: &Document) -> String {
    let story = paras(d)[0].objects.iter().find_map(|o| match o {
        InlineObject::Shape { kind: ShapeKind::TextBox, story, .. } => *story,
        _ => None,
    });
    story.and_then(|id| d.parts.get(&id)).map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect()).unwrap_or_default()
}

/// #743: Word's cover pages place their shapes relative to the page's size, wrapped in
/// `mc:AlternateContent` with an offset as the Fallback, and draw their freeforms back to the
/// start without `a:close`. Both shapes are read: placed at the offset, and filled.
#[test]
fn cover_page_relative_position_and_freeform_drawn_back_to_its_start() {
    let geom = r#"<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="0" t="0" r="r" b="b"/><a:pathLst><a:path w="100" h="100"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="0" y="90"/></a:lnTo><a:cubicBezTo><a:pt x="30" y="100"/><a:pt x="70" y="100"/><a:pt x="100" y="90"/></a:cubicBezTo><a:lnTo><a:pt x="100" y="0"/></a:lnTo><a:lnTo><a:pt x="0" y="0"/></a:lnTo></a:path><a:path w="100" h="100"><a:moveTo><a:pt x="0" y="50"/></a:moveTo><a:lnTo><a:pt x="100" y="50"/></a:lnTo></a:path></a:pathLst></a:custGeom><a:solidFill><a:srgbClr val="0E2841"/></a:solidFill>"#;
    let pct = r#"<mc:AlternateContent><mc:Choice Requires="wp14"><wp:positionV relativeFrom="page"><wp14:pctPosVOffset>4500</wp14:pctPosVOffset></wp:positionV></mc:Choice><mc:Fallback><wp:positionV relativeFrom="page"><wp:posOffset>480695</wp:posOffset></wp:positionV></mc:Fallback></mc:AlternateContent>"#;
    let shape = anchored_wsp("Freeform 1", 5486400, 5334000, geom, "", "<wps:cNvSpPr/>")
        .replace(r#"<wp:positionV relativeFrom="paragraph"><wp:posOffset>25400</wp:posOffset></wp:positionV>"#, pct);
    let check = |d: &Document| {
        let objs = &paras(d)[0].objects;
        let InlineObject::Shape { kind, float, freeform: Some(f), .. } = &objs[0] else { panic!("{:?}", objs[0]) };
        assert_eq!(*kind, ShapeKind::Freeform);
        assert_eq!((float.v_rel, float.v_align, float.y), (Anchor::Page, None, 480695.0 / 12700.0), "the Fallback's offset");
        let closed: Vec<bool> = f.paths.iter().map(|p| p.closed).collect();
        assert_eq!(closed, [true, false], "a path back at its start is closed and filled; an open line isn't");
    };
    let d = read_body(&format!("<w:p>{shape}</w:p>"));
    check(&d);
    check(&wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap());
}

#[test]
fn embedded_fonts_survive_a_round_trip() {
    let table = r#"<w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:font w:name="Roboto"><w:embedRegular r:id="rId1"/><w:embedBold r:id="rId2"/></w:font></w:fonts>"#;
    let table_rels = rels(&[("rId1", "font", "fonts/Roboto-regular.ttf"), ("rId2", "font", "fonts/Roboto-bold.ttf")]);
    let bytes = docx(
        "<w:p><w:r><w:t>Hi</w:t></w:r></w:p>",
        &[("rId9", "fontTable", "fontTable.xml")],
        &[
            ("word/fontTable.xml", table),
            ("word/_rels/fontTable.xml.rels", &table_rels),
            ("word/fonts/Roboto-regular.ttf", "regular font bytes"),
            ("word/fonts/Roboto-bold.ttf", "bold font bytes"),
        ],
    );
    let out = wordcraft_docx::write(&wordcraft_docx::read(&bytes).unwrap()).unwrap();
    let files = unzip(&out);
    assert_eq!(text(&files, "word/fontTable.xml"), table);
    assert_eq!(text(&files, "word/fonts/Roboto-regular.ttf"), "regular font bytes");
    assert_eq!(text(&files, "word/fonts/Roboto-bold.ttf"), "bold font bytes");
    let table_rels = text(&files, "word/_rels/fontTable.xml.rels");
    assert!(table_rels.contains(r#"Id="rId1""#) && table_rels.contains("fonts/Roboto-regular.ttf"), "{table_rels}");
    assert!(text(&files, "word/_rels/document.xml.rels").contains("fontTable.xml"));
    // A second save keeps them too.
    let again = unzip(&wordcraft_docx::write(&wordcraft_docx::read(&out).unwrap()).unwrap());
    assert_eq!(text(&again, "word/fonts/Roboto-bold.ttf"), "bold font bytes");
}

// ---- native Word ink: opaque preservation, never re-encoded as WordCraft freeforms ----

const INK_NS: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingInk";
const INK_XML: &str = r##"<ink xmlns="http://www.w3.org/2003/InkML"><definitions><context xml:id="ctx"><inkSource xml:id="src"><traceFormat><channel name="X" type="integer" units="cm"/><channel name="Y" type="integer" units="cm"/></traceFormat></inkSource></context><brush xml:id="blue"><brushProperty name="color" value="#2563EB"/><brushProperty name="width" value="0.08" units="cm"/></brush></definitions><trace contextRef="#ctx" brushRef="#blue">0 0, 1 1, 2 0</trace></ink>"##;
const INK_TYPES: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/ink/ink1.xml" ContentType="application/inkml+xml"/><Default Extension="png" ContentType="image/png"/></Types>"#;

fn ink_docx(fallback: bool, alias: bool) -> Vec<u8> {
    let content = r#"<q:contentPart xmlns:q="http://schemas.microsoft.com/office/word/2010/wordml" r:id="rInk"><q:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="914400"/></q:xfrm></q:contentPart>"#;
    let drawing = format!(
        r#"<w:drawing><wp:inline><wp:extent cx="1828800" cy="914400"/><wp:docPr id="1" name="Study ink"/><a:graphic><a:graphicData uri="{INK_NS}">{content}</a:graphicData></a:graphic></wp:inline></w:drawing>"#
    );
    let prefix = if alias { "pen" } else { "wpi" };
    let markup = if fallback {
        format!(
            r##"<mc:AlternateContent><mc:Choice xmlns:{prefix}="{INK_NS}" Requires="{prefix}">{drawing}</mc:Choice><mc:Fallback><w:pict><v:shape id="preview" style="width:144pt;height:72pt"><v:imagedata r:id="rPreview"/></v:shape></w:pict></mc:Fallback></mc:AlternateContent>"##
        )
    } else {
        drawing
    };
    let body = format!("<w:p><w:r><w:t>Original</w:t></w:r></w:p><w:p><w:r>{markup}</w:r></w:p>");
    let mut relationships = vec![("rInk", "customXml", "ink/ink1.xml")];
    let mut extra = vec![("[Content_Types].xml", INK_TYPES), ("word/ink/ink1.xml", INK_XML)];
    if fallback {
        relationships.push(("rPreview", "image", "media/preview.png"));
        extra.push(("word/media/preview.png", "original preview bytes"));
    }
    docx(&body, &relationships, &extra)
}

fn check_ink(files: &std::collections::BTreeMap<String, Vec<u8>>) -> String {
    let body = text(files, "word/document.xml");
    let [id] = attr_values(&body, "w14:contentPart", "r:id").try_into().unwrap_or_else(|_| panic!("ink reference lost: {body}"));
    let (kind, target) = rel(&text(files, "word/_rels/document.xml.rels"), &id);
    assert!(kind.ends_with("/customXml"));
    let part = part_at("word", &target);
    assert_eq!(text(files, &part), INK_XML);
    assert!(text(files, "[Content_Types].xml").contains(&format!(r#"PartName="/{part}" ContentType="application/inkml+xml""#)));
    part
}

#[test]
fn native_ink_survives_unrelated_edit_and_two_saves() {
    let mut d = wordcraft_docx::read(&ink_docx(false, false)).unwrap();
    let g = graphics(&d);
    assert_eq!(g.len(), 1, "native ink must remain a model object");
    let InlineObject::Graphic { graphic, .. } = &g[0] else { panic!() };
    assert_eq!(graphic.kind.noun(), "ink");
    assert!(graphic.source.is_some());
    assert!(!graphic.items.is_empty(), "a missing preview must not silently leave a blank frame");
    let p = std::sync::Arc::make_mut(&mut d.body[0]);
    let Block::Para(p) = p else { panic!() };
    p.delete(0, p.len()).unwrap();
    p.insert_text(0, "Edited", &Default::default()).unwrap();
    for _ in 0..2 {
        let out = wordcraft_docx::write(&d).unwrap();
        let files = unzip(&out);
        check_ink(&files);
        assert!(text(&files, "word/document.xml").contains("Edited"));
        d = wordcraft_docx::read(&out).unwrap();
        assert_eq!(graphics(&d).len(), 1);
    }
}

#[test]
fn direct_native_ink_content_part_survives_unrelated_edit_and_two_saves() {
    let body = r#"<w:p><w:r><w:t>Original</w:t></w:r><w:r><w:contentPart r:id="rInk"/></w:r></w:p>"#;
    let mut d = wordcraft_docx::read(&docx(
        body,
        &[("rInk", "customXml", "ink/ink1.xml")],
        &[("[Content_Types].xml", INK_TYPES), ("word/ink/ink1.xml", INK_XML)],
    ))
    .unwrap();
    assert_eq!(graphics(&d).len(), 1, "direct native ink must remain a model object");
    let p = std::sync::Arc::make_mut(&mut d.body[0]);
    let Block::Para(p) = p else { panic!() };
    p.delete(0, 8).unwrap();
    p.insert_text(0, "Edited", &Default::default()).unwrap();
    for _ in 0..2 {
        let out = wordcraft_docx::write(&d).unwrap();
        let files = unzip(&out);
        let body = text(&files, "word/document.xml");
        assert!(body.contains("<w:contentPart"), "direct content part must stay direct: {body}");
        let [id] = attr_values(&body, "w:contentPart", "r:id").try_into().unwrap();
        let (kind, target) = rel(&text(&files, "word/_rels/document.xml.rels"), &id);
        assert!(kind.ends_with("/customXml"));
        let part = part_at("word", &target);
        assert_eq!(text(&files, &part), INK_XML);
        assert!(text(&files, "[Content_Types].xml").contains(&format!(r#"PartName="/{part}" ContentType="application/inkml+xml""#)));
        d = wordcraft_docx::read(&out).unwrap();
        assert_eq!(graphics(&d).len(), 1);
    }
}

#[test]
fn native_ink_keeps_alternate_content_and_preview_with_renamed_prefixes() {
    for alias in [false, true] {
        let mut d = wordcraft_docx::read(&ink_docx(true, alias)).unwrap();
        let g = graphics(&d);
        assert_eq!(g.len(), 1, "fallback must not replace native ink");
        let InlineObject::Graphic { graphic, .. } = &g[0] else { panic!() };
        assert!(
            graphic.items.iter().any(|item| matches!(item, wordcraft_doc::graphic::GraphicItem::Image { media, .. } if d.media.contains_key(media)))
        );
        for _ in 0..2 {
            let out = wordcraft_docx::write(&d).unwrap();
            let files = unzip(&out);
            check_ink(&files);
            let body = text(&files, "word/document.xml");
            assert!(body.contains("<mc:AlternateContent>") && body.contains("<mc:Fallback>"));
            let [id] = attr_values(&body, "v:imagedata", "r:id").try_into().unwrap();
            let (kind, target) = rel(&text(&files, "word/_rels/document.xml.rels"), &id);
            assert!(kind.ends_with("/image"));
            assert_eq!(text(&files, &part_at("word", &target)), "original preview bytes");
            d = wordcraft_docx::read(&out).unwrap();
            assert_eq!(graphics(&d).len(), 1);
        }
    }
}

#[test]
fn native_ink_missing_part_uses_available_fallback() {
    let bytes = ink_docx(true, true);
    let mut files = unzip(&bytes);
    files.remove("word/ink/ink1.xml");
    let entries: Vec<_> = files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
    let d = wordcraft_docx::read(&zip(&entries)).unwrap();
    assert!(graphics(&d).is_empty());
    assert!(paras(&d).iter().flat_map(|p| &p.objects).any(|o| matches!(o, InlineObject::Image { .. })));
    let files = unzip(&wordcraft_docx::write(&d).unwrap());
    assert!(!text(&files, "word/document.xml").contains("w14:contentPart"));
    assert!(files.values().any(|bytes| bytes == b"original preview bytes"));
}

#[test]
fn native_ink_missing_optional_preview_keeps_native_part() {
    let mut files = unzip(&ink_docx(true, false));
    files.remove("word/media/preview.png");
    let entries: Vec<_> = files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
    let d = wordcraft_docx::read(&zip(&entries)).unwrap();
    assert_eq!(graphics(&d).len(), 1);
    let files = unzip(&wordcraft_docx::write(&d).unwrap());
    check_ink(&files);
    assert!(!text(&files, "word/document.xml").contains("<mc:Fallback>"));
}

#[test]
fn native_ink_requires_the_bound_namespace_and_ink_content_type() {
    for wrong_namespace in [true, false] {
        let mut files = unzip(&ink_docx(true, false));
        let name = if wrong_namespace { "word/document.xml" } else { "[Content_Types].xml" };
        let old = text(&files, name);
        let changed = if wrong_namespace {
            old.replace(&format!(r#"xmlns:wpi="{INK_NS}""#), r#"xmlns:wpi="urn:future:unsupported""#)
        } else {
            old.replace("application/inkml+xml", "application/xml")
        };
        files.insert(name.into(), changed.into_bytes());
        let entries: Vec<_> = files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
        let d = wordcraft_docx::read(&zip(&entries)).unwrap();
        assert!(graphics(&d).is_empty());
        assert!(paras(&d).iter().flat_map(|p| &p.objects).any(|o| matches!(o, InlineObject::Image { .. })));
    }
}

#[test]
fn native_ink_copies_have_independent_parts_and_current_frames() {
    let mut d = wordcraft_docx::read(&ink_docx(true, false)).unwrap();
    let [mut object] = graphics(&d).try_into().unwrap();
    let InlineObject::Graphic { w, h, float, .. } = &mut object else { panic!() };
    (*w, *h) = (288.0, 144.0);
    *float = wordcraft_doc::para::Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 36.0, ..Default::default() };
    let mut p = Paragraph::new();
    p.insert_object(0, object.clone(), &Default::default()).unwrap();
    p.insert_object(p.len(), object, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let out = wordcraft_docx::write(&d).unwrap();
    let files = unzip(&out);
    let body = text(&files, "word/document.xml");
    let ids = attr_values(&body, "w14:contentPart", "r:id");
    assert_eq!(ids.len(), 2);
    let parts: Vec<_> = ids.iter().map(|id| part_at("word", &rel(&text(&files, "word/_rels/document.xml.rels"), id).1)).collect();
    assert_ne!(parts[0], parts[1]);
    for part in parts {
        assert_eq!(text(&files, &part), INK_XML);
    }
    assert_eq!(attr_values(&body, "wp:extent", "cx"), ["3657600", "3657600"]);
    assert_eq!(attr_values(&body, "a:ext", "cx"), ["3657600", "3657600"]);
    assert_eq!(attr_values(&body, "v:shape", "style"), ["width:288pt;height:144pt", "width:288pt;height:144pt"]);
    assert_eq!(body.matches("<wp:posOffset>914400</wp:posOffset>").count(), 2);
    let back = wordcraft_docx::read(&out).unwrap();
    let g = graphics(&back);
    assert_eq!(g.len(), 2);
    for object in g {
        let InlineObject::Graphic { w, h, float, .. } = object else { panic!() };
        assert_eq!((w, h, float.x, float.y), (288.0, 144.0, 72.0, 36.0));
    }
}

#[test]
fn native_ink_markup_limit_does_not_write_a_dangling_reference() {
    let mut files = unzip(&ink_docx(false, false));
    let body = text(&files, "word/document.xml").replace("<a:graphicData ", &format!(r#"<a:graphicData padding="{}" "#, "x".repeat(1024 * 1024 + 1)));
    files.insert("word/document.xml".into(), body.into_bytes());
    let entries: Vec<_> = files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
    let d = wordcraft_docx::read(&zip(&entries)).unwrap();
    let files = unzip(&wordcraft_docx::write(&d).unwrap());
    assert!(!text(&files, "word/document.xml").contains("w14:contentPart"));
}

#[test]
fn native_ink_preserves_inner_transform_and_rotation_padding_on_unrelated_saves() {
    let mut files = unzip(&ink_docx(false, false));
    let body = text(&files, "word/document.xml")
        .replace("<wp:docPr", r#"<wp:effectExtent l="55468" t="512668" r="55468" b="512668"/><wp:docPr"#)
        .replace("<q:xfrm>", r#"<q:xfrm rot="2700000">"#)
        .replace(r#"<a:off x="0" y="0"/><a:ext cx="1828800" cy="914400"/>"#, r#"<a:off x="127000" y="254000"/><a:ext cx="914400" cy="457200"/>"#);
    files.insert("word/document.xml".into(), body.into_bytes());
    let entries: Vec<_> = files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
    let mut d = wordcraft_docx::read(&zip(&entries)).unwrap();
    for _ in 0..2 {
        let [object] = graphics(&d).try_into().unwrap();
        let InlineObject::Graphic { float, .. } = object else { panic!() };
        assert_eq!(float.spin().deg, 45.0);
        assert!(float.effect.iter().all(|v| v.abs() < 0.001), "rotation padding is not counted twice: {:?}", float.effect);
        let out = wordcraft_docx::write(&d).unwrap();
        let files = unzip(&out);
        check_ink(&files);
        let body = text(&files, "word/document.xml");
        assert_eq!(attr_values(&body, "a:off", "x"), ["127000"]);
        assert_eq!(attr_values(&body, "a:ext", "cx"), ["914400"]);
        assert_eq!(attr_values(&body, "w14:xfrm", "rot"), ["2700000"]);
        d = wordcraft_docx::read(&out).unwrap();
    }
}

/// Word draws a table of contents' entry links without the Hyperlink character style (measured in
/// Word for Windows); the import drops the style from them, keeping the link. A Hyperlink-styled run
/// outside a hyperlink, and a hyperlink in another field, keep it.
#[test]
fn toc_entry_links_lose_the_hyperlink_style() {
    let styles = format!(
        r#"<w:styles {W_NS}><w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style>
        <w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="0066FF"/><w:u w:val="single"/></w:rPr></w:style></w:styles>"#
    );
    let field = |instr: &str, inner: &str| {
        format!(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve">{instr}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{inner}</w:p><w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#
        )
    };
    let link =
        |t: &str| format!(r#"<w:hyperlink w:anchor="_Toc1"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>{t}</w:t></w:r></w:hyperlink>"#);
    let body = [
        field(r#" TOC \o "1-3" \h \z \u "#, &link("entry")),
        field(" TOC ", r#"<w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>styled</w:t></w:r>"#),
        field(" QUOTE x ", &link("quoted")),
        format!("<w:p>{}</w:p>", link("plain")),
        r#"<w:p><w:bookmarkStart w:id="1" w:name="_Toc1"/><w:r><w:t>Target</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p>"#.to_string(),
    ]
    .concat();
    let d = wordcraft_docx::read(&docx(&body, &[("rId1", "styles", "styles.xml")], &[("word/styles.xml", &styles)])).unwrap();
    let run = |word: &str| {
        paras(&d).into_iter().find_map(|p| p.text.find(word).map(|at| p.props_of_char(at).clone())).unwrap_or_else(|| panic!("no run with {word}"))
    };
    let entry = run("entry");
    assert_eq!((entry.style.as_deref(), entry.link.as_deref()), (None, Some("#_Toc1")), "a TOC entry: plain, still a link");
    assert_eq!(run("styled").style.as_deref(), Some("Hyperlink"), "not in a hyperlink: kept");
    assert_eq!(run("quoted").style.as_deref(), Some("Hyperlink"), "another field: kept");
    assert_eq!(run("plain").style.as_deref(), Some("Hyperlink"), "outside fields: kept");
}

/// A style or paragraph whose `w:numPr` gives only a level (`w:ilvl`) takes the list from its
/// style chain (ECMA-376 §17.3.1.19): a Heading 3 based on a numbered Heading 2 is level 3 of the
/// same list. WordCraft dropped the level, so Heading 3s numbered as Heading 2s (6.15 where Word
/// shows 6.4.5 in NIH's monitoring-plan template).
#[test]
fn a_list_level_without_a_list_takes_the_style_s_list() {
    let styles = format!(
        r#"<w:styles {W_NS}>
        <w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="15"/></w:numPr></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Heading2"/><w:pPr><w:numPr><w:ilvl w:val="2"/></w:numPr></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/><w:basedOn w:val="Heading3"/><w:pPr><w:numPr><w:ilvl w:val="3"/></w:numPr></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="Loose"><w:name w:val="Loose"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:ilvl w:val="4"/></w:numPr></w:pPr></w:style>
        </w:styles>"#
    );
    let body = r#"<w:p><w:pPr><w:pStyle w:val="Heading3"/></w:pPr><w:r><w:t>three</w:t></w:r></w:p>
        <w:p><w:pPr><w:pStyle w:val="Heading2"/><w:numPr><w:ilvl w:val="2"/></w:numPr></w:pPr><w:r><w:t>direct level</w:t></w:r></w:p>
        <w:p><w:pPr><w:numPr><w:ilvl w:val="1"/></w:numPr></w:pPr><w:r><w:t>no list anywhere</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body, &[("rId1", "styles", "styles.xml")], &[("word/styles.xml", &styles)])).unwrap();
    let num = |id: &str| d.styles.get(id).and_then(|s| s.para.numbering).map(|n| (n.num, n.level));
    assert_eq!(num("Heading3"), Some((15, 2)), "Heading 3 is level 3 of Heading 2's list");
    assert_eq!(num("Heading4"), Some((15, 3)), "through two level-only styles");
    assert_eq!(num("Loose"), None, "no list in the chain: no numbering");
    let p = paras(&d);
    assert_eq!(d.styles.resolve_para(&p[0].props).numbering.map(|n| (n.num, n.level)), Some((15, 2)));
    assert_eq!(p[1].props.numbering.map(|n| (n.num, n.level)), Some((15, 2)), "a paragraph's own level, its style's list");
    assert_eq!(p[2].props.numbering, None);
    // Saved and read again it is an ordinary list reference.
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(again.styles.get("Heading3").and_then(|s| s.para.numbering).map(|n| (n.num, n.level)), Some((15, 2)));
}

/// `w:hideMark` on a cell is read and written back (it was dropped, and the empty rows NIH's
/// tables collapse with it showed as blank lines).
#[test]
fn cell_hide_mark_reads_and_round_trips() {
    let body = r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:hideMark/></w:tcPr><w:p/></w:tc><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl><w:p/>"#;
    let hides = |d: &Document| -> Vec<bool> {
        d.body
            .iter()
            .find_map(|b| {
                if let wordcraft_doc::Block::Table(t) = &**b { Some(t.rows[0].cells.iter().map(|c| c.props.hide_mark).collect()) } else { None }
            })
            .unwrap()
    };
    let d = read_body(body);
    assert_eq!(hides(&d), [true, false]);
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(hides(&again), [true, false]);
}

/// Text frames (`w:framePr` without a drop cap), as NIH's templates put a footer's page number in
/// one: read with their size, anchors, alignment and wrap, and written back (they were dropped on
/// save).
#[test]
fn text_frames_read_and_round_trip() {
    let body = r#"<w:p><w:pPr><w:framePr w:wrap="around" w:vAnchor="text" w:hAnchor="margin" w:xAlign="center" w:y="1"/></w:pPr><w:r><w:t>3</w:t></w:r></w:p>
        <w:p><w:pPr><w:framePr w:w="2880" w:h="720" w:hRule="exact" w:hSpace="180" w:wrap="notBeside" w:hAnchor="page" w:vAnchor="page" w:x="1440" w:y="2880"/></w:pPr><w:r><w:t>boxed</w:t></w:r></w:p>
        <w:p><w:pPr><w:framePr w:wrap="sideways" w:hAnchor="nowhere" w:w="-5" w:x="NaN"/></w:pPr><w:r><w:t>junk</w:t></w:r></w:p>
        <w:p><w:pPr><w:framePr w:dropCap="drop" w:lines="2" w:wrap="around" w:vAnchor="text" w:hAnchor="text"/></w:pPr><w:r><w:t>D</w:t></w:r></w:p>
        <w:p><w:r><w:t>rop cap text</w:t></w:r></w:p>"#;
    let d = read_body(body);
    let p = paras(&d);
    let f = p[0].props.frame.clone().expect("a frame");
    assert_eq!(
        (f.wrap.as_str(), f.v_anchor.as_str(), f.h_anchor.as_str(), f.x_align.as_deref(), f.y),
        ("around", "text", "margin", Some("center"), 0.05)
    );
    let g = p[1].props.frame.clone().expect("a frame");
    assert_eq!((g.w, g.h, g.h_exact, g.h_space, g.wrap.as_str(), g.x, g.y), (Some(144.0), Some(36.0), true, 9.0, "notBeside", 72.0, 144.0));
    let cap = p.iter().find(|q| q.props.drop_cap.is_some()).expect("the drop cap");
    assert!(cap.props.frame.is_none(), "a drop cap stays a drop cap");
    let junk = p.iter().find(|q| q.text.contains("junk")).and_then(|q| q.props.frame.clone()).expect("a frame");
    assert_eq!((junk.wrap.as_str(), junk.h_anchor.as_str(), junk.w, junk.x), ("auto", "page", None, 0.0), "unknown values fall back");
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    let q = paras(&again);
    assert_eq!(q[0].props.frame, p[0].props.frame);
    assert_eq!(q[1].props.frame, p[1].props.frame);
    assert!(q.iter().any(|x| x.props.drop_cap == Some(2) && x.props.frame.is_none()));
}

/// A legacy VML freeform (`v:shape` with a `path`, ECMA-376 Part 4): a rule across a header,
/// drawn in a 1000 × 1000 coordinate space, outlined, its fill transparent. It is read as a
/// freeform of the shape's size and kept when saved (as DrawingML custom geometry) and read back.
#[test]
fn vml_freeform_rule_in_header() {
    let w10 = r#"xmlns:w10="urn:schemas-microsoft-com:office:word""#;
    let rule = r##"<w:p><w:r><w:pict><v:shape style="position:absolute;margin-left:0;margin-top:30pt;width:360pt;height:12pt;z-index:2;mso-position-horizontal-relative:margin;mso-position-vertical-relative:margin" coordsize="1000,1000" path="m,500l1000,500xe" fillcolor="this" stroked="t" strokecolor="#3366cc" strokeweight="1.5pt"><v:fill opacity="0"/><w10:wrap anchorx="margin"/></v:shape></w:pict></w:r>
<w:r><w:pict><v:shape style="width:20pt;height:10pt" coordorigin="100,100" coordsize="200,100" path="t100,100r100,0,0,100nfx e" strokecolor="red"><v:stroke opacity="32768f"/></v:shape></w:pict></w:r></w:p>"##;
    let header = format!(r#"<w:hdr {W_NS} {w10}><w:p><w:r><w:t>Header</w:t></w:r></w:p>{rule}</w:hdr>"#);
    let body = r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#;
    let bytes = docx(body, &[("rIdH", "header", "header1.xml")], &[("word/header1.xml", &header)]);
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    let check = |d: &Document| {
        let hid = d.last_section.headers.default.unwrap();
        let shapes: Vec<&InlineObject> = d.parts.get(&hid).unwrap().blocks.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.objects).collect();
        let [rule, tri] = shapes.as_slice() else { panic!("{shapes:?}") };
        let InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, freeform: Some(f), .. } = rule else { panic!("{rule:?}") };
        assert_eq!((*kind, *fill, *stroke), (ShapeKind::Freeform, None, Some(Rgb(0x33, 0x66, 0xCC))));
        assert!(near(*w, 360.0) && near(*h, 12.0) && near(*stroke_width, 1.5), "{w} x {h}, {stroke_width}");
        assert_eq!((float.h_rel, float.v_rel, float.wrap), (Anchor::Margin, Anchor::Margin, Wrap::InFrontOfText));
        assert!(near(float.y, 30.0), "{float:?}");
        let [p] = f.paths.as_slice() else { panic!("{f:?}") };
        let pts = f.placed(0.0, 0.0, *w, *h);
        let [(pts, _)] = pts.as_slice() else { panic!("{pts:?}") };
        assert!(p.closed && pts.len() == 2, "{pts:?}");
        assert!(near(pts[0][0], 0.0) && near(pts[0][1], 6.0) && near(pts[1][0], 360.0) && near(pts[1][1], 6.0), "{pts:?}");
        // Relative moves and lines from the origin, `nf`: an outlined triangle, half opaque.
        let InlineObject::Shape { kind: ShapeKind::Freeform, fill: None, stroke: Some(Rgb(255, 0, 0)), w, h, freeform: Some(f), .. } = tri else {
            panic!("{tri:?}")
        };
        let placed = f.placed(0.0, 0.0, *w, *h);
        let [(pts, true)] = placed.as_slice() else { panic!("{placed:?}") };
        let want = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        assert!(pts.len() == 3 && pts.iter().zip(want).all(|(a, b)| near(a[0], b[0]) && near(a[1], b[1])), "{pts:?}");
        assert!(near(f.alpha, 0.5), "{}", f.alpha);
    };
    let d = wordcraft_docx::read(&bytes).unwrap();
    check(&d);
    check(&wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap());
}

/// Hostile VML paths and coordinate spaces read without panicking and stay finite.
#[test]
fn hostile_vml_paths_stay_finite() {
    let paths = [
        "m,",
        "l",
        "mxe",
        "m99999999999999999999999999,0l-99999999999999999999,1e",
        "minf,nanlNaN,-inf",
        "m@1,@2l@3,@4nfnse",
        "wa1",
        "c1",
        "m١٢,٣l٤,٥",
        "m0,0c",
        "m0,0v1,2,3",
        "ar0,0,10,10,0,0,10,10at",
        "m0,0r1,1,1,1,1,1,1,1,1,1x x x l",
        "--..,,@@ ,,, xm1.5.5,2..3l+-+,-e",
        "\u{1F600}m1,1l2,2",
        "qbqxqyaeal1,2,3,4,5,6",
    ];
    let spaces = [("0,0", "0,0"), ("-5,-5", "x,y"), ("1e39,x", "-99999999999999999999,99999999999999999999"), ("", ""), ("1,1", "1e308,1e308")];
    for path in paths {
        for (size, origin) in spaces {
            let body = format!(
                r#"<w:p><w:r><w:pict><v:shape style="width:1e30pt;height:-4pt" coordsize="{size}" coordorigin="{origin}" path="{path}" strokecolor="black"/></w:pict></w:r></w:p>"#
            );
            let d = read_body(&body);
            // A path with formulas isn't drawn (they aren't evaluated).
            if path.contains('@') {
                assert!(paras(&d)[0].objects.iter().all(|o| !matches!(o, InlineObject::Shape { freeform: Some(_), .. })), "{path}");
            }
            for o in &paras(&d)[0].objects {
                if let InlineObject::Shape { w, h, freeform: Some(f), .. } = o {
                    assert!(w.is_finite() && h.is_finite() && f.alpha.is_finite(), "{path}");
                    assert!(f.paths.iter().flat_map(|p| &p.pts).all(|[x, y]| x.is_finite() && y.is_finite()), "{path} {size} {origin}");
                    for (pts, _) in f.placed(0.0, 0.0, *w, *h) {
                        assert!(pts.iter().all(|[x, y]| x.is_finite() && y.is_finite()), "{path}");
                    }
                }
            }
            wordcraft_docx::write(&d).unwrap();
        }
    }
}

/// Measured in Word: `w:asciiTheme="minorBidi"` names the theme's complex-script font (`a:cs`);
/// when that is empty it names nothing, and the run keeps the font it inherits.
#[test]
fn bidi_theme_slots_name_the_complex_script_font() {
    let font = |cs: &str| {
        let theme = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="T"><a:themeElements><a:fontScheme name="F"><a:majorFont><a:latin typeface="Cambria"/><a:ea typeface=""/><a:cs typeface="{cs}"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface="{cs}"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#
        );
        let rels = [("rId9", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme", "theme/theme1.xml")];
        let body = r#"<w:p><w:r><w:rPr><w:rFonts w:asciiTheme="minorBidi" w:hAnsiTheme="minorBidi"/></w:rPr><w:t>x</w:t></w:r></w:p>"#;
        let d = wordcraft_docx::read(&docx(body, &rels, &[("word/theme/theme1.xml", &theme)])).unwrap();
        paras(&d)[0].runs[0].props.font.clone()
    };
    assert_eq!(font(""), None);
    assert_eq!(font("Times New Roman").as_deref(), Some("Times New Roman"));
}

/// The font table's family classes: Word draws a missing roman font in a serif face.
#[test]
fn roman_fonts_are_read_from_the_font_table() {
    let fonts = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:font w:name="Brand Serif"><w:family w:val="roman"/></w:font><w:font w:name="Brand Sans"><w:family w:val="swiss"/></w:font></w:fonts>"#;
    let rels = [("rId9", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable", "fontTable.xml")];
    let d = wordcraft_docx::read(&docx("<w:p/>", &rels, &[("word/fontTable.xml", fonts)])).unwrap();
    assert_eq!(d.settings.roman_fonts, ["Brand Serif"]);
}

/// `w:doNotUseHTMLParagraphAutoSpacing` in the settings' `w:compat`: paragraph spacing adds up.
#[test]
fn html_auto_spacing_off_is_read_and_kept() {
    let settings = |compat: &str| {
        let s = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings {W_NS}><w:compat>{compat}</w:compat></w:settings>"#);
        let rels = [("rId9", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings", "settings.xml")];
        let d = wordcraft_docx::read(&docx("<w:p/>", &rels, &[("word/settings.xml", &s)])).unwrap();
        let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
        (d.settings.sum_paragraph_spacing, again.settings.sum_paragraph_spacing)
    };
    assert_eq!(settings("<w:doNotUseHTMLParagraphAutoSpacing/>"), (true, true));
    assert_eq!(settings(""), (false, false));
}

/// A right triangle (`rtTriangle`) has its right angle at the bottom left: a cover page's white
/// corner cut-outs drawn as isosceles triangles pointed the wrong way.
#[test]
fn a_right_triangle_keeps_its_right_angle() {
    let body = anchored_wsp("Corner", 1270000, 635000, r#"<a:prstGeom prst="rtTriangle"><a:avLst/></a:prstGeom>"#, "", "<wps:cNvSpPr/>");
    let d = read_body(&format!("<w:p>{body}</w:p>"));
    let kind = |d: &Document| match &paras(d)[0].objects[0] {
        InlineObject::Shape { kind, .. } => *kind,
        o => panic!("{o:?}"),
    };
    assert_eq!(kind(&d), ShapeKind::RightTriangle);
    let saved = wordcraft_docx::write(&d).unwrap();
    assert_eq!(kind(&wordcraft_docx::read(&saved).unwrap()), ShapeKind::RightTriangle);
}

#[test]
fn hyperlink_with_empty_target_is_no_link() {
    // LibreOffice writes `Target=""` for the Hyperlink character style without an address (#1005).
    let body = r#"<w:p><w:hyperlink r:id="rId2"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>plain</w:t></w:r></w:hyperlink><w:hyperlink r:id="rId3"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>mail</w:t></w:r></w:hyperlink><w:hyperlink w:anchor="top"><w:r><w:t>jump</w:t></w:r></w:hyperlink></w:p>"#;
    let bytes = docx(body, &[("rId2", "hyperlink", "https://x"), ("rId3", "hyperlink", "mailto:a@example.com")], &[]);
    // Blank the first target and mark the mailto one external (the helper only does that for http).
    let bytes = {
        let mut zr = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut entries = Vec::new();
        for i in 0..zr.len() {
            let mut f = zr.by_index(i).unwrap();
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
            if f.name() == "word/_rels/document.xml.rels" {
                buf = String::from_utf8(buf)
                    .unwrap()
                    .replace("Target=\"https://x\"", "Target=\"\"")
                    .replace("a@example.com\"", "a@example.com\" TargetMode=\"External\"")
                    .into_bytes();
            }
            entries.push((f.name().to_string(), buf));
        }
        let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        zip(&refs)
    };
    let doc = wordcraft_docx::read(&bytes).unwrap();
    let p = doc.body.iter().find_map(|b| b.as_para()).unwrap();
    assert_eq!(p.plain_text(), "plainmailjump");
    assert_eq!(p.props_of_char(0).link, None);
    assert_eq!(p.props_of_char(0).style.as_deref(), Some("Hyperlink"));
    assert_eq!(p.props_of_char(5).link.as_deref(), Some("mailto:a@example.com"));
    assert_eq!(p.props_of_char(9).link.as_deref(), Some("#top"));
    // Round trip keeps exactly the real links.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    let p = back.body.iter().find_map(|b| b.as_para()).unwrap();
    assert_eq!(p.props_of_char(0).link, None);
    assert_eq!(p.props_of_char(5).link.as_deref(), Some("mailto:a@example.com"));
}

#[test]
fn row_table_borders_round_trip() {
    let d = read_body(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>
<w:tr><w:tblPrEx><w:tblBorders><w:top w:val="single" w:sz="8"/><w:insideV w:val="single" w:sz="4"/></w:tblBorders></w:tblPrEx>
<w:trPr><w:trHeight w:val="400"/></w:trPr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr>
<w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    let saved = wordcraft_docx::write(&d).unwrap();
    let xml = text(&unzip(&saved), "word/document.xml");
    assert!(xml.contains("<w:tblPrEx>"), "row table borders were dropped: {xml}");
    assert!(xml.contains("<w:insideV"));
    let again = wordcraft_docx::read(&saved).unwrap();
    assert_eq!(d.body, again.body);
}
