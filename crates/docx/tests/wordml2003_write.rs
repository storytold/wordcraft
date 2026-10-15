//! Saving as Word 2003 XML (#1081): documents built in code, written as WordprocessingML 2003
//! and read back with the 2003 reader.

use std::sync::Arc;

use wordcraft_doc::numbering::ListKind;
use wordcraft_doc::para::NoteKind;
use wordcraft_doc::props::{Align, CharProps, NumRef, Rgb, TextColor};
use wordcraft_doc::table::{Cell, Table};
use wordcraft_doc::{Block, Comment, Document, InlineObject, Paragraph, PartKind, Revision, RevisionKind, para_block};
use wordcraft_docx::{XmlKind, read_xml, sniff_xml, write_wordml2003};

fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8 * 80, y as u8 * 120, 200, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn paras(doc: &Document) -> Vec<&Paragraph> {
    doc.body.iter().filter_map(|b| b.as_para()).collect()
}

fn part_text(doc: &Document, id: Option<u32>) -> String {
    id.and_then(|i| doc.parts.get(&i))
        .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

fn sample() -> Document {
    let mut d = Document::new();
    d.core.title = "Garden <notes> & more".into();
    d.core.creator = "Ada".into();
    let key = d.add_media(tiny_png(), "png");
    let num = d.numbering.add_list(ListKind::Numbered);
    let head = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Head", CharProps::default()))]);
    let foot = d.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Sown in May.", CharProps::default()))]);
    let com = d.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Too short?", CharProps::default()))]);
    d.comments.insert(3, Comment { author: "Bo".into(), part: com, ..Default::default() });
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "Ada".into(), date: String::new(), move_name: None });
    d.last_section.headers.default = Some(head);

    let mut p = Paragraph::new();
    for (t, c) in [
        ("Plain ", CharProps::default()),
        (
            "loud",
            CharProps { bold: Some(true), italic: Some(true), size: Some(14.0), color: Some(TextColor::Rgb(Rgb(255, 0, 0))), ..Default::default() },
        ),
        (" link", CharProps { link: Some("https://example.org/beans?a=1&b=2".into()), ..Default::default() }),
    ] {
        let off = p.len();
        p.insert_text(off, t, &c).unwrap();
    }
    p.props.align = Some(Align::Center);
    p.props.indent_left = Some(36.0);
    p.props.space_after = Some(12.0);
    p.insert_object(0, InlineObject::CommentStart { id: 3 }, &CharProps::default()).unwrap();
    let end = p.len();
    p.insert_object(end, InlineObject::CommentEnd { id: 3 }, &CharProps::default()).unwrap();
    let at = p.text.find("loud").unwrap();
    p.insert_object(at, InlineObject::NoteRef { kind: NoteKind::Footnote, id: foot, custom: String::new() }, &CharProps::default()).unwrap();
    let mut blocks = vec![para_block(p)];

    let mut li = Paragraph::with_text("listed", CharProps { ins: Some(0), ..Default::default() });
    li.props.numbering = Some(NumRef { num, level: 0 });
    blocks.push(para_block(li));

    let mut img = Paragraph::new();
    img.insert_object(
        0,
        InlineObject::Image { media: key, w: 30.0, h: 20.0, alt: "dot".into(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    blocks.push(para_block(img));

    let mut t = Table::new(2, 2, 300.0);
    t.rows[0].cells[0] = Cell::with_text("A1");
    t.rows[0].cells[1] = Cell::with_text("B1");
    t.rows[1].cells[0].props.shading = Some(Rgb(0, 128, 0));
    blocks.push(Arc::new(Block::Table(t)));
    blocks.push(para_block(Paragraph::new()));
    d.body = blocks;
    d
}

#[test]
fn round_trips_through_the_2003_reader() {
    let d = sample();
    let xml = write_wordml2003(&d).expect("write");
    let text = std::str::from_utf8(&xml).expect("utf-8");
    assert!(text.contains("<w:wordDocument") && text.contains("http://schemas.microsoft.com/office/word/2003/wordml"));
    assert!(text.contains("<w:binData") && text.contains("<w:hlink w:dest=\"https://example.org/beans?a=1&amp;b=2\""));
    assert!(text.contains("<w:footnote>") && text.contains("<wx:sect>") && text.contains("<aml:annotation"));
    assert!(!text.contains("w:szCs") && !text.contains("<w:drawing"));
    if std::env::var_os("WML2003_DUMP").is_some() {
        println!("{text}");
    }
    assert_eq!(sniff_xml(&xml), XmlKind::WordMl2003);

    let back = read_xml(&xml).expect("read");
    assert_eq!(back.core.title, "Garden <notes> & more");
    assert_eq!(back.core.creator, "Ada");
    let ps = paras(&back);
    assert_eq!(ps[0].plain_text(), "Plain loud link");
    assert_eq!(ps[0].props.align, Some(Align::Center));
    assert_eq!(ps[0].props.indent_left, Some(36.0));
    assert_eq!(ps[0].props.space_after, Some(12.0));
    let c = ps[0].props_of_char(ps[0].text.find("loud").unwrap());
    assert_eq!((c.bold, c.italic, c.size), (Some(true), Some(true), Some(14.0)));
    assert!(c.color.is_some());
    assert_eq!(ps[0].props_at(ps[0].text.find("link").unwrap()).link.as_deref(), Some("https://example.org/beans?a=1&b=2"));
    let note = ps[0].objects.iter().find_map(|o| match o {
        InlineObject::NoteRef { id, .. } => Some(*id),
        _ => None,
    });
    assert_eq!(part_text(&back, note), "Sown in May.");
    let comment = back.comments.values().next().expect("comment");
    assert_eq!((comment.author.as_str(), part_text(&back, Some(comment.part)).as_str()), ("Bo", "Too short?"));

    let listed = ps[1];
    assert!(listed.props.numbering.is_some(), "list paragraph keeps its list");
    let ins = listed.props_of_char(0).ins.and_then(|i| back.revisions.get(i as usize));
    assert_eq!(ins.map(|r| r.author.as_str()), Some("Ada"));

    let pic = ps.iter().flat_map(|p| p.objects.iter()).find(|o| matches!(o, InlineObject::Image { .. }));
    assert!(matches!(pic, Some(InlineObject::Image { w, h, alt, .. }) if *w == 30.0 && *h == 20.0 && alt == "dot"));
    assert_eq!(back.media.len(), 1);

    let table = back.body.iter().find_map(|b| match b.as_ref() {
        Block::Table(t) => Some(t),
        _ => None,
    });
    let table = table.expect("table");
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells[1].blocks.first().and_then(|b| b.as_para()).map(|p| p.plain_text()).as_deref(), Some("B1"));
    assert_eq!(table.rows[1].cells[0].props.shading, Some(Rgb(0, 128, 0)));
    assert_eq!(part_text(&back, back.last_section.headers.default), "Head");
}

/// Features 2003 XML can't carry drop out without an error, and the text stays.
#[test]
fn control_characters_and_unsupported_content_do_not_break_the_file() {
    let mut d = Document::new();
    let mut p = Paragraph::with_text("a\u{1}b\u{FFFE}c <&> \"q\"", CharProps { size: Some(f32::NAN), ..Default::default() });
    p.insert_object(
        0,
        InlineObject::Opaque { format: "docx".into(), xml: "<w:r><w:t>raw</w:t></w:r>".into(), text: "raw".into() },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let xml = write_wordml2003(&d).expect("write");
    assert!(read_xml(&xml).is_ok());
    let text = String::from_utf8(xml).expect("utf-8");
    assert!(!text.contains('\u{1}') && !text.contains('\u{FFFE}'));
    assert!(!text.contains("<w:drawing") && !text.contains("<mc:"));
}

#[test]
fn big_and_deep_documents_write() {
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text(&"word ".repeat(400_000), CharProps::default()))];
    let xml = write_wordml2003(&d).expect("write");
    assert!(read_xml(&xml).is_ok());

    // Tables nested deeper than the writers go: no panic, whatever comes out.
    let mut inner = Cell::with_text("deep");
    for _ in 0..60 {
        let mut t = Table::new(1, 1, 100.0);
        t.rows[0].cells[0] = inner;
        inner = Cell::empty();
        inner.blocks = vec![Arc::new(Block::Table(t)), para_block(Paragraph::new())];
    }
    let mut d = Document::new();
    d.body = inner.blocks;
    if let Ok(xml) = write_wordml2003(&d) {
        let _ = read_xml(&xml);
    }
}

#[test]
fn extended_nfc_and_custom_style_support() {
    use wordcraft_doc::section::NumFormat;

    let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:wordDocument xmlns:w="http://schemas.microsoft.com/office/word/2003/wordml" xmlns:wx="http://schemas.microsoft.com/office/word/2003/auxHint">
  <w:lists>
    <w:listDef w:listDefId="1">
      <w:lvl w:ilvl="0"><w:start w:val="1"/><w:nfc w:val="50"/><w:lvlText w:val="- %1 -"/><w:lvlJc w:val="left"/></w:lvl>
      <w:lvl w:ilvl="1"><w:start w:val="1"/><w:nfc w:val="51"/><w:lvlText w:val="%2)"/><w:lvlJc w:val="left"/></w:lvl>
    </w:listDef>
    <w:list w:ilfo="1"><w:ilst w:val="1"/></w:list>
  </w:lists>
  <w:styles>
    <w:style w:type="paragraph" w:styleId="MyCustom" w:customStyle="on">
      <w:name w:val="My Custom"/>
    </w:style>
  </w:styles>
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="MyCustom"/><w:listPr><w:ilvl w:val="0"/><w:ilfo w:val="1"/></w:listPr></w:pPr><w:r><w:t>Dashed</w:t></w:r></w:p>
  </w:body>
</w:wordDocument>"##;

    let doc = read_xml(xml.as_bytes()).expect("read");
    let st = doc.styles.get("MyCustom").expect("custom style");
    assert!(!st.builtin);

    let lvl0 = doc.numbering.level(1, 0).expect("level 0");
    assert_eq!(lvl0.format, NumFormat::NumberInDash);
    let lvl1 = doc.numbering.level(1, 1).expect("level 1");
    assert_eq!(lvl1.format, NumFormat::RussianLower);

    // Verify round-trip preserves NFC 50 and 51
    let written = write_wordml2003(&doc).expect("write");
    let text = String::from_utf8(written).expect("utf-8");
    assert!(text.contains(r#"<w:nfc w:val="50"/>"#), "nfc 50 in output");
    assert!(text.contains(r#"<w:nfc w:val="51"/>"#), "nfc 51 in output");
    assert!(text.contains(r#"w:customStyle="on""#), "customStyle in output");
}
