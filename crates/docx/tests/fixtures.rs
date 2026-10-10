//! Hand-written minimal WordprocessingML fixtures (written for these tests from the spec).

use std::io::Write;

use wordcraft_doc::para::{Anchor, FloatAlign, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{Align, Border, BorderStyle, Rgb, TextColor, VMerge};
use wordcraft_doc::{Block, Document, InlineObject, Paragraph};

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#;

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

fn paras(d: &Document) -> Vec<&Paragraph> {
    d.body.iter().filter_map(|b| b.as_para()).collect()
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
    assert_eq!(p[0].objects[0], InlineObject::Field { instr: r#"TOC \o "1-3" \h \z \u"#.into(), result: String::new(), locked: false });
    assert_eq!(p[0].objects[1], InlineObject::Field { instr: r#"PAGEREF _Toc1 \h"#.into(), result: "1".into(), locked: false });
    assert_eq!(p[0].props_of_char(3).link.as_deref(), Some("#_Toc1"));
    assert_eq!(p[0].props.style.as_deref(), Some("TOC1"));
    assert_eq!(p[1].text, "Body\t2");
    assert_eq!(p[1].props_of_char(0).link.as_deref(), Some("#_Toc2"));
    assert_eq!(p[2].text, "");
    assert_eq!(p[3].objects, vec![InlineObject::BookmarkStart { name: "_Toc1".into() }, InlineObject::BookmarkEnd { name: "_Toc1".into() }]);
    assert_eq!(p[4].plain_text(), "Page 7");
    assert_eq!(p[4].objects[0], InlineObject::Field { instr: "PAGE".into(), result: "7".into(), locked: false });
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
    assert_eq!(p[0].text, "siteXABC\u{F04A}\u{2011}\u{AD}\n\u{C}\u{E} e&T");
    assert_eq!(p[0].props_of_char(0).link.as_deref(), Some("https://example.org/"));
    assert_eq!(p[0].props_of_char(0).style.as_deref(), Some("Hyperlink"));
    assert_eq!(p[0].props_of_char(4).link.as_deref(), Some("https://example.org/#frag"));
    let sym_off = p[0].text.find('\u{F04A}').unwrap();
    assert_eq!(p[0].props_of_char(sym_off).font.as_deref(), Some("Wingdings"));
    let t = p[0].props_of_char(p[0].text.len() - 1);
    assert_eq!((t.bold, t.italic, t.caps, t.strike, t.color), (Some(false), Some(false), Some(false), Some(true), Some(TextColor::Auto)));
    assert_eq!(p[1].text, "in block sdt");
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
        r#"<w:settings {W_NS}><w:trackRevisions/><w:defaultTabStop w:val="708"/><w:evenAndOddHeaders w:val="1"/><w:footnotePr><w:numFmt w:val="lowerRoman"/></w:footnotePr></w:settings>"#
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
    assert_eq!(p[1].objects, vec![InlineObject::Field { instr: "IF X = \"y\"".into(), result: String::new(), locked: false }]);
    assert_eq!(p[2].objects, vec![InlineObject::BookmarkStart { name: "between".into() }, InlineObject::BookmarkEnd { name: "between".into() }]);
    assert_eq!(p[2].plain_text(), "after");
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
    let floats: Vec<_> = p[0].objects.iter().filter_map(|o| if let InlineObject::Shape { float, .. } = o { Some(*float) } else { None }).collect();
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
            .filter_map(|o| if let InlineObject::Shape { kind, w, h, float, story, .. } = o { Some((*kind, *w, *h, *float, *story)) } else { None })
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
