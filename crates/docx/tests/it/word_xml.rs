//! Word XML files (#548): Flat OPC (Word XML Document) and Word 2003 XML, from hand-written
//! samples (never files produced by Word).

use base64::Engine as _;
use wordcraft_doc::props::Align;
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::{Block, Document, InlineObject, Paragraph};
use wordcraft_docx::{DocxError, XmlKind, read_xml, sniff_xml};

fn paras(doc: &Document) -> Vec<&Paragraph> {
    doc.body.iter().filter_map(|b| b.as_para()).collect()
}

fn part_text(doc: &Document, id: Option<u32>) -> String {
    id.and_then(|i| doc.parts.get(&i))
        .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

fn tiny_png_b64() -> String {
    let img = image::RgbaImage::from_fn(2, 2, |x, y| image::Rgba([x as u8 * 100, y as u8 * 100, 50, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(out.into_inner());
    // Line breaks inside base64, as the formats write it.
    b64.as_bytes().chunks(20).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("\n")
}

/// A minimal Flat OPC package written by hand: namespaces declared on the package element only,
/// a hyperlink relationship, a picture in base64 and an empty part.
#[test]
fn reads_a_hand_written_flat_opc_package() {
    let png = tiny_png_b64();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<?mso-application progid="Word.Document"?>
<pkg:package xmlns:pkg="http://schemas.microsoft.com/office/2006/xmlPackage" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <pkg:part pkg:name="/_rels/.rels" pkg:contentType="application/vnd.openxmlformats-package.relationships+xml">
    <pkg:xmlData>
      <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>
    </pkg:xmlData>
  </pkg:part>
  <pkg:part pkg:name="/word/_rels/document.xml.rels" pkg:contentType="application/vnd.openxmlformats-package.relationships+xml">
    <pkg:xmlData><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.org/flat" TargetMode="External"/><Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/dot.png"/></Relationships></pkg:xmlData>
  </pkg:part>
  <pkg:part pkg:name="/word/document.xml" pkg:contentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml">
    <pkg:xmlData>
      <w:document xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:v="urn:schemas-microsoft-com:vml"><w:body>
        <w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Seed &amp; soil</w:t></w:r></w:p>
        <w:p><w:hyperlink r:id="rId5"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p>
        <w:p><w:r><w:pict><v:shape style="width:24pt;height:12pt"><v:imagedata r:id="rId6"/></v:shape></w:pict></w:r></w:p>
      </w:body></w:document>
    </pkg:xmlData>
  </pkg:part>
  <pkg:part pkg:name="/word/media/dot.png" pkg:contentType="image/png" pkg:compression="store">
    <pkg:binaryData>{png}</pkg:binaryData>
  </pkg:part>
  <pkg:part pkg:name="/word/empty.xml" pkg:contentType="application/xml"><pkg:xmlData/></pkg:part>
</pkg:package>"#
    );
    assert_eq!(sniff_xml(xml.as_bytes()), XmlKind::FlatOpc);
    let doc = read_xml(xml.as_bytes()).unwrap();
    let ps = paras(&doc);
    assert_eq!(ps[0].plain_text(), "Seed & soil");
    assert_eq!(ps[0].props_at(0).bold, Some(true));
    assert_eq!(ps[1].props_at(0).link.as_deref(), Some("https://example.org/flat"));
    assert!(matches!(ps[2].objects.first(), Some(InlineObject::Image { w, h, .. }) if *w == 24.0 && *h == 12.0));
    assert_eq!(doc.media.len(), 1);
}

/// A Word 2003 XML Document, hand-written from the Office 2003 XML Reference Schemas.
fn wordml_2003_sample() -> String {
    let png = tiny_png_b64();
    format!(
        r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<?mso-application progid="Word.Document"?>
<w:wordDocument xmlns:w="http://schemas.microsoft.com/office/word/2003/wordml" xmlns:wx="http://schemas.microsoft.com/office/word/2003/auxHint" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:aml="http://schemas.microsoft.com/aml/2001/core" w:macrosPresent="no">
  <o:DocumentProperties><o:Title>Garden notes</o:Title><o:Author>Ada Gardener</o:Author><o:Keywords>beans</o:Keywords></o:DocumentProperties>
  <w:fonts><w:defaultFonts w:ascii="Georgia" w:fareast="Georgia" w:h-ansi="Georgia" w:cs="Georgia"/></w:fonts>
  <w:lists>
    <w:listDef w:listDefId="3"><w:plt w:val="HybridMultilevel"/>
      <w:lvl w:ilvl="0"><w:start w:val="1"/><w:nfc w:val="4"/><w:lvlText w:val="%1)"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>
    </w:listDef>
    <w:list w:ilfo="2"><w:ilst w:val="3"/></w:list>
  </w:lists>
  <w:styles>
    <w:versionOfBuiltInStyleNames w:val="4"/>
    <w:style w:type="paragraph" w:default="on" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><wx:font wx:val="Georgia"/><w:sz w:val="24"/><w:sz-cs w:val="24"/></w:rPr></w:style>
    <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><wx:uiName wx:val="Heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="60"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
  </w:styles>
  <w:docPr><w:view w:val="print"/><w:zoom w:percent="100"/><w:footnotePr><w:footnote w:type="separator"><w:p><w:r><w:separator/></w:r></w:p></w:footnote></w:footnotePr></w:docPr>
  <w:body>
    <wx:sect>
      <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><aml:annotation aml:id="0" w:type="Word.Bookmark.Start" w:name="plan"/><aml:annotation aml:id="1" w:type="Word.Comment.Start"/><w:r><w:t>Plan</w:t></w:r><aml:annotation aml:id="1" w:type="Word.Comment.End"/><w:r><aml:annotation aml:id="1" aml:author="Bo" w:initials="B" w:type="Word.Comment"><aml:content><w:p><w:r><w:t>Too short?</w:t></w:r></w:p></aml:content></aml:annotation></w:r><aml:annotation aml:id="0" w:type="Word.Bookmark.End"/></w:p>
      <w:p><w:pPr><w:jc w:val="center"/><w:ind w:first-line="360"/><w:spacing w:line="360" w:line-rule="auto"/></w:pPr>
        <w:r><w:t>Plain </w:t></w:r>
        <w:r><w:rPr><w:rFonts w:ascii="Courier New" w:h-ansi="Courier New"/><w:b/><w:i/><w:u w:val="single"/><w:sz w:val="28"/><w:color w:val="FF0000"/></w:rPr><w:t>loud</w:t></w:r>
        <w:r><w:tab/><w:t>then</w:t><w:br/><w:t>more</w:t></w:r>
        <w:r><w:footnote><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t>Sown in May.</w:t></w:r></w:p></w:footnote></w:r>
      </w:p>
      <w:p><w:hlink w:dest="https://example.org/beans" w:screenTip="Beans"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>bean guide</w:t></w:r></w:hlink></w:p>
      <w:p><w:pPr><w:listPr><w:ilvl w:val="0"/><w:ilfo w:val="2"/><wx:t wx:val="a)"/></w:listPr></w:pPr><w:r><w:t>Water the beans</w:t></w:r><aml:annotation aml:id="7" aml:author="Ada" aml:createdate="2004-05-01T10:00:00Z" w:type="Word.Insertion"><aml:content><w:r><w:t> daily</w:t></w:r></aml:content></aml:annotation></w:p>
      <w:tbl>
        <w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>
        <w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>
        <w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:vmerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>B1</w:t></w:r></w:p></w:tc></w:tr>
        <w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:vmerge/></w:tcPr><w:p/></w:tc><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>B2</w:t></w:r></w:p></w:tc></w:tr>
      </w:tbl>
      <w:p><w:r><w:pict><w:binData w:name="wordml://02000001.png">{png}</w:binData><v:shape id="_x0000_i1025" type="#_x0000_t75" style="width:36pt;height:18pt"><v:imagedata src="wordml://02000001.png" o:title="dot" gain="19661f" blacklevel="22938f"/></v:shape></w:pict></w:r></w:p>
      <w:sectPr>
        <w:hdr w:type="odd"><w:p><w:r><w:t>Garden header</w:t></w:r></w:p></w:hdr>
        <w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1800" w:bottom="1440" w:left="1800" w:header="720" w:footer="720" w:gutter="0"/>
      </w:sectPr>
    </wx:sect>
  </w:body>
</w:wordDocument>"##
    )
}

/// Word 2003 XML pictures keep Word's Washout (gain 0.3, black level 0.35) as brightness and
/// contrast (#1711).
#[test]
fn word_2003_xml_picture_keeps_washout() {
    let doc = read_xml(wordml_2003_sample().as_bytes()).unwrap();
    let ps = paras(&doc);
    let pic = ps.iter().flat_map(|p| p.objects.iter()).find(|o| matches!(o, InlineObject::Image { .. }));
    assert!(matches!(pic, Some(InlineObject::Image { fx, .. }) if (fx.bright - 70_000).abs() <= 2 && (fx.contrast + 70_000).abs() <= 2), "{pic:?}");
}

#[test]
fn imports_word_2003_xml() {
    let xml = wordml_2003_sample();
    assert_eq!(sniff_xml(xml.as_bytes()), XmlKind::WordMl2003);
    let doc = read_xml(xml.as_bytes()).unwrap();
    assert_eq!(doc.core.title, "Garden notes");
    assert_eq!(doc.core.creator, "Ada Gardener");

    let h1 = doc.styles.get("Heading1").expect("Heading1 style");
    assert_eq!(h1.chr.bold, Some(true));
    assert_eq!(h1.chr.size, Some(16.0));

    let ps = paras(&doc);
    assert_eq!(ps[0].plain_text(), "Plan");
    assert_eq!(ps[0].props.style.as_deref(), Some("Heading1"));
    assert!(ps[0].objects.iter().any(|o| matches!(o, InlineObject::BookmarkStart { name, .. } if name == "plan")));

    let p = ps[1];
    assert_eq!(p.props.align, Some(Align::Center));
    assert_eq!(p.props.indent_first, Some(18.0));
    let loud = p.text.find("loud").unwrap();
    let c = p.props_of_char(loud);
    assert_eq!((c.bold, c.italic, c.size, c.font.as_deref()), (Some(true), Some(true), Some(14.0), Some("Courier New")));
    assert!(c.underline.is_some() && c.color.is_some());
    assert!(p.plain_text().contains("Plain loud\tthen"));
    let note = p.objects.iter().find_map(|o| match o {
        InlineObject::NoteRef { id, .. } => Some(*id),
        _ => None,
    });
    assert_eq!(part_text(&doc, note), "Sown in May.");

    assert_eq!(ps[2].props_at(0).link.as_deref(), Some("https://example.org/beans"));
    let listed = ps[3];
    let num = listed.props.numbering.expect("list paragraph");
    let lvl = doc.numbering.level(num.num, num.level).expect("list level");
    assert_eq!((lvl.format, lvl.text.as_str(), lvl.indent), (NumFormat::LowerLetter, "%1)", 36.0));
    let ins = listed.props_of_char(listed.text.find("daily").unwrap()).ins.and_then(|i| doc.revisions.get(i as usize));
    assert_eq!(ins.map(|r| r.author.as_str()), Some("Ada"));
    let comment = doc.comments.values().next().expect("comment");
    assert_eq!((comment.author.as_str(), part_text(&doc, Some(comment.part)).as_str()), ("Bo", "Too short?"));

    let table = doc.body.iter().find_map(|b| match b.as_ref() {
        Block::Table(t) => Some(t),
        _ => None,
    });
    let table = table.expect("table");
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells[1].blocks.first().and_then(|b| b.as_para()).map(|p| p.plain_text()).as_deref(), Some("B1"));

    let pic = ps.iter().flat_map(|p| p.objects.iter()).find(|o| matches!(o, InlineObject::Image { .. }));
    assert!(matches!(pic, Some(InlineObject::Image { w, h, .. }) if *w == 36.0 && *h == 18.0));
    assert_eq!(doc.media.len(), 1);

    assert_eq!(doc.last_section.page_w, 612.0);
    assert_eq!(doc.last_section.margin_left, 90.0);
    assert_eq!(part_text(&doc, doc.last_section.headers.default), "Garden header");
}

#[test]
fn hostile_word_xml_is_an_error_not_a_panic() {
    let flat = r#"<pkg:package xmlns:pkg="http://schemas.microsoft.com/office/2006/xmlPackage">"#;
    let w2003 = r#"<w:wordDocument xmlns:w="http://schemas.microsoft.com/office/word/2003/wordml">"#;
    // Other XML (and non-XML) is refused with the Word XML error.
    for other in ["<html><body>hi</body></html>", "", "not xml at all", "<package/>"] {
        assert_eq!(read_xml(other.as_bytes()), Err(DocxError::NotWordXml), "{other:?}");
    }
    // Broken packages: errors.
    for bad in [
        flat.to_string(),
        format!("{flat}</pkg:package>"),
        format!("{flat}<pkg:part pkg:name=\"/word/document.xml\"><pkg:binaryData>!!!not base64</pkg:binaryData></pkg:part></pkg:package>"),
        format!("{flat}<pkg:part pkg:name=\"/word/document.xml\"><pkg:xmlData><w:document></pkg:xmlData></pkg:part></pkg:package>"),
    ] {
        assert!(read_xml(bad.as_bytes()).is_err(), "{bad}");
    }
    // Lenient leftovers never panic.
    for odd in [
        format!("{flat}<pkg:part/><pkg:part pkg:name=\"\"><pkg:xmlData/></pkg:part><x/></pkg:package>"),
        format!(
            "{w2003}<w:body><w:p><w:r><w:footnote w:type=\"separator\"/><w:pict><w:binData w:name=\"a\">%%%</w:binData><v:imagedata src=\"a\"/></w:pict></w:r></w:p></w:body></w:wordDocument>"
        ),
        format!(
            "{w2003}<w:lists><w:list/><w:listDef/></w:lists><w:body><aml:annotation xmlns:aml=\"http://schemas.microsoft.com/aml/2001/core\" aml:id=\"x\" w:type=\"Word.Comment\"/><w:sectPr><w:hdr/><w:ftr w:type=\"first\"/></w:sectPr></w:body></w:wordDocument>"
        ),
        format!("{w2003}{}", "<w:p>".repeat(5000)),
    ] {
        let _ = read_xml(odd.as_bytes());
    }
    // Too many parts.
    let mut many = String::from(flat);
    for i in 0..20_001 {
        many.push_str(&format!("<pkg:part pkg:name=\"/p{i}.bin\"><pkg:binaryData>AA==</pkg:binaryData></pkg:part>"));
    }
    many.push_str("</pkg:package>");
    assert!(matches!(read_xml(many.as_bytes()), Err(DocxError::Limit(_))));
}

/// A Flat OPC package of one paragraph with a picture, whose `pkg:binaryData` holds `data` as
/// written, and `extra` after the parts.
fn picture_package(data: &str, extra: &str) -> String {
    format!(
        r#"<pkg:package xmlns:pkg="http://schemas.microsoft.com/office/2006/xmlPackage" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <pkg:part pkg:name="/_rels/.rels" pkg:contentType="application/vnd.openxmlformats-package.relationships+xml"><pkg:xmlData><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships></pkg:xmlData></pkg:part>
  <pkg:part pkg:name="/word/_rels/document.xml.rels" pkg:contentType="application/vnd.openxmlformats-package.relationships+xml"><pkg:xmlData><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/dot.png"/></Relationships></pkg:xmlData></pkg:part>
  <pkg:part pkg:name="/word/document.xml" pkg:contentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"><pkg:xmlData><w:document xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:v="urn:schemas-microsoft-com:vml"><w:body><w:p><w:r><w:t>Seed</w:t></w:r><w:r><w:pict><v:shape style="width:24pt;height:12pt"><v:imagedata r:id="rId6"/></v:shape></w:pict></w:r></w:p></w:body></w:document></pkg:xmlData></pkg:part>
  <pkg:part pkg:name="/word/media/dot.png" pkg:contentType="image/png" pkg:compression="store"><pkg:binaryData>{data}</pkg:binaryData></pkg:part>{extra}
</pkg:package>"#
    )
}

/// A binary part's base64 reads the same as plain text, as CDATA or with character references;
/// an entity in it is an error rather than bytes gone missing (#1353).
#[test]
fn flat_opc_binary_data_reads_the_same_in_cdata_and_character_references() {
    let png = tiny_png_b64();
    let media = |data: &str| read_xml(picture_package(data, "").as_bytes()).map(|d| d.media.values().cloned().collect::<Vec<_>>());
    let plain = media(&png).unwrap();
    assert_eq!(plain.len(), 1);
    let (head, tail) = png.split_at(png.len() / 2);
    let first = png.chars().next().unwrap();
    for other in
        [format!("<![CDATA[{png}]]>"), format!("{head}<![CDATA[{tail}]]>"), format!("&#{};{}", u32::from(first), &png[1..]).replace('\n', "&#xA;")]
    {
        assert_eq!(media(&other).unwrap(), plain, "{other}");
    }
    assert!(media(&format!("&amp;{png}")).is_err());
}

/// Work anywhere in a Flat OPC file is bounded, not only in the parts that are kept: nesting
/// and elements no part reader looks at, and repeated part names, count too (#1353).
#[test]
fn flat_opc_limits_count_what_is_not_kept() {
    let png = tiny_png_b64();
    let limited = |extra: &str| matches!(read_xml(picture_package(&png, extra).as_bytes()), Err(DocxError::Limit(_)));
    assert!(read_xml(picture_package(&png, "<x/>").as_bytes()).is_ok());
    // Deep nesting outside the parts.
    assert!(limited(&format!("<x>{}{}</x>", "<y>".repeat(1000), "</y>".repeat(1000))));
    // One part name over and over, or other elements, past the part count.
    assert!(limited(&r#"<pkg:part pkg:name="/word/media/dot.png"><pkg:binaryData>AA==</pkg:binaryData></pkg:part>"#.repeat(20_000)));
    assert!(limited(&"<x/>".repeat(20_000)));
    // A part nothing reads, one element over what the XML tree reader takes in a part (8 000 000).
    assert!(limited(&format!(r#"<pkg:part pkg:name="/word/unused.xml"><pkg:xmlData><r>{}</r></pkg:xmlData></pkg:part>"#, "<y/>".repeat(8_000_000))));
}
