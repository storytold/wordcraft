//! Tests of the crate.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::io::Write;

use crate::xml::{self, ORDINAL};
use crate::{Package, sniff};

/// The ZIP limits of these tests.
const TEST_ZIP: craft_zip::Limits = craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 };

/// Entry `name` of a ZIP package, as text.
pub(crate) fn zip_part(bytes: &[u8], name: &str) -> String {
    let data = craft_zip::Archive::open(bytes, TEST_ZIP).unwrap().read(name, 64 << 20).unwrap().unwrap_or_else(|| panic!("no {name}"));
    String::from_utf8(data).unwrap()
}

/// The entry names of a ZIP package, in order.
pub(crate) fn zip_names(bytes: &[u8]) -> Vec<String> {
    craft_zip::Archive::open(bytes, TEST_ZIP).unwrap().names().map(|n| n.into_owned()).collect()
}

/// A minimal HWPX archive with the given section XML.
pub(crate) fn hwpx(section: &str) -> Vec<u8> {
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
    z.add("Contents/header.xml", br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, craft_zip::Compression::Deflated).unwrap();
    z.add("Contents/section0.xml", section.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    z.finish().unwrap()
}

const NS: &str = r#"xmlns:p="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section""#;

#[test]
fn names_are_normalised_and_paragraphs_numbered() {
    // A file using its own prefix ("p") for the paragraph namespace.
    let sec = format!(r#"<hs:sec {NS}><p:p><p:run><p:t>a&amp;b</p:t></p:run></p:p><p:p/></hs:sec>"#);
    let root = xml::parse(sec.as_bytes(), true).unwrap();
    let ps: Vec<_> = root.iter("hp:p").collect();
    assert_eq!(ps.len(), 2);
    assert_eq!(ps[0].get(ORDINAL), Some("0"));
    assert_eq!(ps[1].get(ORDINAL), Some("1"));
    assert_eq!(root.iter("hp:t").next().unwrap().text, "a&b");
}

#[test]
fn switch_keeps_the_known_case() {
    let sec = format!(
        r#"<hs:sec {NS}><p:switch><p:case p:required-namespace="http://www.hancom.co.kr/hwpml/2016/HwpUnitChar"><p:p id="case"/></p:case><p:default><p:p id="default"/></p:default></p:switch></hs:sec>"#
    );
    let root = xml::parse(sec.as_bytes(), true).unwrap();
    let ids: Vec<_> = root.iter("hp:p").map(|p| p.get("id").unwrap_or("")).collect();
    assert_eq!(ids, ["case"]);
    // numbered before the switch was resolved: the dropped default copy keeps its number
    assert_eq!(root.iter("hp:p").next().unwrap().get(ORDINAL), Some("0"));
}

#[test]
fn package_finds_sections() {
    let bytes = hwpx(&format!(r#"<hs:sec {NS}><p:p/></hs:sec>"#));
    assert!(sniff(&bytes));
    let pkg = Package::open(&bytes).unwrap();
    assert_eq!(pkg.section_names, ["Contents/section0.xml"]);
    assert_eq!(pkg.header_name.as_deref(), Some("Contents/header.xml"));
    assert!(pkg.entries[0].stored);
}

/// The document part of a converted section.
fn document_xml(section: &str) -> String {
    let (docx, _) = crate::to_docx(&hwpx(section)).unwrap();
    zip_part(&docx, "word/document.xml")
}

#[test]
fn paragraphs_carry_their_source_marks() {
    let doc = document_xml(&format!(
        r#"<hs:sec {NS}><p:p><p:run><p:t>첫째<p:tab/>a&lt;b</p:t></p:run></p:p><p:p><p:run><p:tbl rowCnt="1" colCnt="1"><p:tr><p:tc><p:cellAddr rowAddr="0" colAddr="0"/><p:cellSz width="5000" height="1000"/><p:subList><p:p><p:run><p:t>칸</p:t></p:run></p:p></p:subList></p:tc></p:tr></p:tbl></p:run></p:p></hs:sec>"#
    ));
    assert!(doc.contains(r#"w:name="_hp_0_0"/>"#), "{doc}");
    assert!(doc.contains(r#"<w:t xml:space="preserve">첫째</w:t><w:tab/><w:t xml:space="preserve">a&lt;b</w:t>"#), "{doc}");
    // the paragraph holding only a table: its mark goes before the table
    let mark = doc.find(r#"w:name="_ht_0_1""#).unwrap();
    assert!(mark < doc.find("<w:tbl>").unwrap());
    assert!(doc.contains(r#"<w:gridCol w:w="1000"/>"#), "{doc}");
    assert!(doc.contains("칸"));
}

#[test]
fn text_scanners_match_their_patterns() {
    use crate::convert as t;
    assert_eq!(t::w_t_texts(r#"<w:tab/><w:t xml:space="preserve">a</w:t><w:t>b</w:t>"#), "ab");
    assert_eq!(
        t::no_wrap(r#"<x/><wp:wrapSquare wrapText="bothSides"/><wp:wrapTopAndBottom/><wp:wrapNone/>"#),
        "<x/><wp:wrapNone/><wp:wrapNone/><wp:wrapNone/>"
    );
    assert_eq!(t::level_placeholders("^1.^2)^x"), "%1.%2)^x");
    assert_eq!(t::unescape_backslash(r"a\;b\\c"), r"a;b\c");
}

#[test]
fn hostile_inputs_are_errors_not_panics() {
    assert!(!sniff(b""));
    assert!(Package::open(b"PK\x03\x04garbage").is_err());
    assert!(xml::parse(b"", false).is_err());
    let deep = "<a>".repeat(10_000);
    assert!(xml::parse(deep.as_bytes(), false).is_err());
    assert!(xml::parse(b"<a><b>unclosed", false).is_ok());
}

/// One HWP 5.0 record: header (tag, level, size) and data. A size of 4095 bytes or more is
/// written as 4095 in the header and the size itself in the 4 bytes after it.
fn record(tag: u32, level: u32, data: &[u8]) -> Vec<u8> {
    let size = data.len() as u32;
    let mut out = (tag | (level << 10) | (size.min(0xFFF) << 20)).to_le_bytes().to_vec();
    if size >= 0xFFF {
        out.extend_from_slice(&size.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

/// Records of 4095 bytes or more carry their size after the header, so the helper makes such
/// records as the reader takes them.
#[test]
fn the_record_helper_writes_extended_sizes() {
    for n in [0usize, 10, 0xFFE, 0xFFF, 5000] {
        let data = record(0x42, 0, &vec![7u8; n]);
        let r = crate::hwp5::Records::parse(&data);
        assert_eq!((r.recs.len(), r.data(0).len()), (1, n), "{n} bytes");
    }
}

/// A minimal uncompressed HWP 5.0 file with one paragraph of `text`.
pub(crate) fn hwp(text: &str, flags: u32) -> Vec<u8> {
    let mut head = b"HWP Document File".to_vec();
    head.resize(256, 0);
    head[36..40].copy_from_slice(&flags.to_le_bytes());
    let mut chars: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    chars.extend_from_slice(&13u16.to_le_bytes());
    let mut section = record(0x42, 0, &[0u8; 22]); // PARA_HEADER
    section.extend(record(0x43, 1, &chars)); // PARA_TEXT
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
    comp.create_storage("/BodyText").unwrap();
    for (name, data) in [("/FileHeader", head), ("/DocInfo", Vec::new()), ("/BodyText/Section0", section)] {
        comp.create_stream(name).unwrap().write_all(&data).unwrap();
    }
    comp.into_inner().into_inner()
}

/// An uncompressed HWP 5.0 file whose one section holds `section` (records).
fn hwp_with(section: &[u8]) -> Vec<u8> {
    let mut head = b"HWP Document File".to_vec();
    head.resize(256, 0);
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
    comp.create_storage("/BodyText").unwrap();
    for (name, data) in [("/FileHeader", head), ("/DocInfo", Vec::new()), ("/BodyText/Section0", section.to_vec())] {
        comp.create_stream(name).unwrap().write_all(&data).unwrap();
    }
    comp.into_inner().into_inner()
}

/// A bullet that is a symbol font's private-use glyph can't be drawn as it is. U+F0FC is drawn
/// as a check mark in Hancom's viewer, so it becomes one; other private-use bullets stay "•".
#[test]
fn private_use_bullets_become_what_hancom_draws() {
    let bullets = |ch: char| {
        let head = format!(
            r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:bullets><hh:bullet id="1" char="{ch}"/></hh:bullets><hh:paraProperties><hh:paraPr id="1"><hh:heading type="BULLET" idRef="1" level="0"/></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#
        );
        let sec = format!(r#"<hs:sec {NS}><p:p paraPrIDRef="1"><p:run><p:t>항목</p:t></p:run></p:p></hs:sec>"#);
        let mut z = craft_zip::Writer::new();
        z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
        z.add("Contents/header.xml", head.as_bytes(), craft_zip::Compression::Deflated).unwrap();
        z.add("Contents/section0.xml", sec.as_bytes(), craft_zip::Compression::Deflated).unwrap();
        let (docx, _) = crate::to_docx(&z.finish().unwrap()).unwrap();
        zip_part(&docx, "word/numbering.xml")
    };
    assert!(bullets('\u{F0FC}').contains(r#"<w:lvlText w:val="✓"/>"#), "{}", bullets('\u{F0FC}'));
    assert!(bullets('\u{F06C}').contains(r#"<w:lvlText w:val="•"/>"#), "{}", bullets('\u{F06C}'));
    assert!(bullets('▶').contains(r#"<w:lvlText w:val="▶"/>"#));
}

/// A table on a text line goes where its paragraph aligns it, and floating objects (with only
/// spaces) before it in the same paragraph are placed from the table's line, as in Hancom's
/// viewer: the table takes the right alignment of its paragraph, and the piece of paragraph
/// holding the objects takes (almost) no height instead of a line above the table.
#[test]
fn table_on_a_text_line_follows_its_paragraph() {
    let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:paraProperties><hh:paraPr id="1"><hh:align horizontal="RIGHT"/></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#;
    let rect = r#"<p:rect id="1" zOrder="1" textWrap="IN_FRONT_OF_TEXT"><p:sz width="2000" height="8000"/><p:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="1800" horzOffset="600"/><p:curSz width="2000" height="8000"/><p:orgSz width="2000" height="8000"/></p:rect>"#;
    let tbl = r#"<p:tbl rowCnt="1" colCnt="1"><p:sz width="20000" height="2000"/><p:pos treatAsChar="1" vertRelTo="PARA" horzRelTo="PARA"/><p:tr><p:tc><p:cellAddr rowAddr="0" colAddr="0"/><p:cellSz width="20000" height="2000"/><p:subList><p:p><p:run><p:t>칸</p:t></p:run></p:p></p:subList></p:tc></p:tr></p:tbl>"#;
    let sec =
        format!(r#"<hs:sec {NS}><p:p paraPrIDRef="1"><p:run>{rect}<p:t>  </p:t>{tbl}</p:run></p:p><p:p><p:run><p:t>뒤</p:t></p:run></p:p></hs:sec>"#);
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
    z.add("Contents/header.xml", head.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    z.add("Contents/section0.xml", sec.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    let (docx, _) = crate::to_docx(&z.finish().unwrap()).unwrap();
    let body = zip_part(&docx, "word/document.xml");
    let tbl_at = body.find("<w:tbl>").unwrap_or_else(|| panic!("no table: {body}"));
    let tblpr = &body[tbl_at..body[tbl_at..].find("</w:tblPr>").map(|e| tbl_at + e).unwrap()];
    assert!(tblpr.contains(r#"<w:jc w:val="right"/>"#), "{tblpr}");
    let anchor = body.find("<wp:anchor ").unwrap_or_else(|| panic!("no floating object: {body}"));
    assert!(anchor < tbl_at, "{body}");
    let para = &body[body[..anchor].rfind("<w:p>").unwrap()..anchor];
    assert!(para.contains(crate::convert::TINY_PPR), "the objects' piece takes no height: {para}");
}

/// A group's members are placed in final units (their rendering matrices multiplied in), so the
/// outermost group's drawing space is its current size, as a nested group's is. Its stored
/// original size (before the group was resized) squeezed the members: a group of lines 159 pt
/// tall was drawn in 220 pt of space, 0.72 times too short.
#[test]
fn hwp5_outer_group_draws_its_members_at_their_size() {
    use crate::hwp5::{CTRL_HEADER, PARA_HEADER, PARA_TEXT, SHAPE_COMPONENT};
    let put = |d: &mut Vec<u8>, at: usize, v: &[u8]| d[at..at + v.len()].copy_from_slice(v);
    // the component: id(s), then offset, group level, original and current size, matrices
    let component = |ids: &[&[u8; 4]], org: (u32, u32), cur: (u32, u32), at: (f64, f64)| {
        let o = 4 * ids.len();
        let mut d = vec![0u8; o + 44 + 48 * 3];
        for (k, id) in ids.iter().enumerate() {
            put(&mut d, 4 * k, &id[..]);
        }
        put(&mut d, o + 12, &org.0.to_le_bytes());
        put(&mut d, o + 16, &org.1.to_le_bytes());
        put(&mut d, o + 20, &cur.0.to_le_bytes());
        put(&mut d, o + 24, &cur.1.to_le_bytes());
        put(&mut d, o + 42, &1u16.to_le_bytes()); // one scale and rotation pair
        // translation, scale, rotation: each a b c / d e f
        for (m, v) in [[1.0, 0.0, at.0, 0.0, 1.0, at.1], [1.0, 0.0, 0.0, 0.0, 1.0, 0.0], [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]].iter().enumerate() {
            for (k, x) in v.iter().enumerate() {
                put(&mut d, o + 44 + 48 * m + 8 * k, &x.to_le_bytes());
            }
        }
        d
    };
    let mut gso = b" osg".to_vec();
    gso.resize(46, 0);
    put(&mut gso, 16, &15000u32.to_le_bytes());
    put(&mut gso, 20, &10000u32.to_le_bytes());
    let text: Vec<u8> = [11u16, 0, 0, 0, 0, 0, 0, 11, 13].iter().flat_map(|c| c.to_le_bytes()).collect();
    let mut sec = record(PARA_HEADER, 0, &[0; 24]);
    sec.extend(record(PARA_TEXT, 1, &text));
    sec.extend(record(CTRL_HEADER, 1, &gso));
    // a group resized to half its original size; its member already placed at the final size
    sec.extend(record(SHAPE_COMPONENT, 2, &component(&[b"noc$", b"noc$"], (30000, 20000), (15000, 10000), (0.0, 0.0))));
    sec.extend(record(SHAPE_COMPONENT, 3, &component(&[b"cer$"], (15000, 10000), (15000, 10000), (0.0, 0.0))));
    let root = crate::hwp5::section(&sec);
    let group = root.iter("hp:container").next().unwrap_or_else(|| panic!("no group: {}", root.to_xml("")));
    let size = |tag: &str| group.find(tag).map(|e| (e.get("width").unwrap_or("").to_string(), e.get("height").unwrap_or("").to_string()));
    assert_eq!(size("hp:curSz"), Some(("15000".into(), "10000".into())));
    assert_eq!(size("hp:orgSz"), size("hp:curSz"), "{}", group.to_xml(""));
    assert_eq!(group.children.iter().filter(|c| c.tag == "hp:rect").count(), 1, "{}", group.to_xml(""));
}

/// One paragraph's records as Hancom writes them: header (character count, the last one of
/// the section marked), text, character shape.
fn hwp_paragraph(text: &str, last: bool) -> Vec<u8> {
    let mut chars: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    chars.extend_from_slice(&13u16.to_le_bytes());
    let mut head = [0u8; 24];
    let n = (chars.len() / 2) as u32 | if last { 0x8000_0000 } else { 0 };
    head[0..4].copy_from_slice(&n.to_le_bytes());
    head[12..14].copy_from_slice(&1u16.to_le_bytes());
    let mut out = record(0x42, 0, &head);
    out.extend(record(0x43, 1, &chars));
    out.extend(record(0x44, 1, &[0u8; 8]));
    out
}

/// The section stream of an HWP file.
fn hwp_section(bytes: &[u8]) -> Vec<u8> {
    let mut c = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut c.open_stream("/BodyText/Section0").unwrap(), &mut data).unwrap();
    data
}

/// The DOCX of `bytes` with `from` replaced by `to` in its text.
fn edited_docx(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let (docx, _) = crate::to_docx(bytes).unwrap();
    rezip(&docx, |xml| {
        assert!(xml.contains(from), "{xml}");
        xml.replace(from, to)
    })
}

/// Text edited in WordCraft goes back into the HWP file: the edited paragraph's records are
/// rebuilt (no layout cache, counts set), the other paragraph keeps its bytes, the file reads.
#[test]
fn hwp5_write_back_rewrites_edited_paragraphs() {
    let first = hwp_paragraph("첫째 문단", false);
    let second = hwp_paragraph("둘째", true);
    let original = hwp_with(&[first.clone(), second.clone()].concat());
    // unchanged: the very same bytes
    let (docx, _) = crate::to_docx(&original).unwrap();
    let (same, report) = crate::write_back_hwp(&original, &docx, None).unwrap();
    assert_eq!(same, original);
    assert_eq!(report.modified, 0);
    // the first paragraph edited
    let docx = edited_docx(&original, "첫째 문단", "고친 첫 문단입니다");
    let (out, report) = crate::write_back_hwp(&original, &docx, None).unwrap();
    assert_eq!(report.modified, 1, "{report:?}");
    assert!(report.skipped.is_empty(), "{report:?}");
    let section = hwp_section(&out);
    assert!(section.ends_with(&second), "the untouched paragraph keeps its records");
    let text: Vec<u8> = "고친 첫 문단입니다\r".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let n = text.len() / 2;
    let head = section.get(4..28).unwrap();
    assert_eq!(u32::from_le_bytes(head[0..4].try_into().unwrap()), n as u32, "count, not the last of its list");
    assert_eq!(&section[28..32], &(0x43u32 | (1 << 10) | ((text.len() as u32) << 20)).to_le_bytes());
    assert_eq!(section.get(32..32 + text.len()), Some(text.as_slice()));
    let (docx, _) = crate::to_docx(&out).unwrap();
    let doc = String::from_utf8_lossy(&docx).into_owned();
    let z = docx;
    let body = zip_part(&z, "word/document.xml");
    assert!(body.contains("고친 첫 문단입니다") && body.contains("둘째"), "{doc}");
}

#[test]
fn hwp5_paragraphs_convert() {
    let bytes = hwp("안녕 HWP", 0);
    assert!(sniff(&bytes));
    let (docx, _) = crate::to_docx(&bytes).unwrap();
    let z = docx;
    let doc = zip_part(&z, "word/document.xml");
    assert!(doc.contains(r#"<w:t xml:space="preserve">안녕 HWP</w:t>"#), "{doc}");
}

#[test]
fn hwp5_protected_and_damaged_files_are_errors() {
    assert_eq!(crate::to_docx(&hwp("x", 0x02)).unwrap_err(), crate::Error::Protected);
    assert_eq!(crate::to_docx(&hwp("x", 0x04)).unwrap_err(), crate::Error::Protected);
    let mut cut = hwp("x", 0);
    cut.truncate(700);
    assert!(crate::to_docx(&cut).is_err());
    assert!(crate::to_docx(&crate::hwp5::OLE_SIG.repeat(64)).is_err());
    // records that claim more than they hold, or nest absurdly deep, still read
    let mut deep = Vec::new();
    for level in 0..1000 {
        deep.extend(record(0x42, level, &[0u8; 22]));
    }
    deep.extend(record(0x43, 0, &[0xFF; 3]));
    deep.extend((0x42u32 | (0xFFF << 20)).to_le_bytes());
    deep.extend(u32::MAX.to_le_bytes());
    let _ = crate::hwp5::section(&deep);
}

/// Cell margins are signed (HWPUNIT16): a cell with its own margins and some sides -1 reads them
/// as 0. Before, -1 read as 65,535, and the cell's text width collapsed to its minimum.
#[test]
fn hwp5_cell_margins_are_signed() {
    use crate::hwp5::{CTRL_HEADER, LIST_HEADER, PARA_HEADER, PARA_TEXT, TABLE};
    let text: Vec<u8> = [11u16, 0, 0, 0, 0, 0, 0, 11, 13].iter().flat_map(|c| c.to_le_bytes()).collect();
    let mut tbl = b" lbt".to_vec();
    tbl.resize(46, 0);
    let mut td = vec![0u8; 24];
    td[4..6].copy_from_slice(&1u16.to_le_bytes());
    td[6..8].copy_from_slice(&1u16.to_le_bytes());
    let margins = |sides: [u16; 4]| {
        let mut cell = vec![0u8; 47];
        cell[0..2].copy_from_slice(&1u16.to_le_bytes());
        cell[16..20].copy_from_slice(&5000u32.to_le_bytes());
        for (k, v) in sides.iter().enumerate() {
            cell[24 + 2 * k..26 + 2 * k].copy_from_slice(&v.to_le_bytes());
        }
        let mut sec = record(PARA_HEADER, 0, &[0; 24]);
        sec.extend(record(PARA_TEXT, 1, &text));
        sec.extend(record(CTRL_HEADER, 1, &tbl));
        sec.extend(record(TABLE, 2, &td));
        sec.extend(record(LIST_HEADER, 2, &cell));
        sec.extend(record(PARA_HEADER, 2, &[0; 24]));
        let root = crate::hwp5::section(&sec);
        let m = root.iter("hp:cellMargin").next().map(|m| ["left", "right", "top", "bottom"].map(|s| m.get(s).unwrap_or("").to_string()));
        m.unwrap_or_else(|| panic!("no cell margin: {}", root.to_xml("")))
    };
    assert_eq!(margins([0xFFFF, 141, 0xFFFF, 141]), ["0", "141", "0", "141"]);
    // all four -1: the table's margins, kept as they are for a save
    assert_eq!(margins([0xFFFF; 4]), ["65535", "65535", "65535", "65535"]);
}

/// A section part starting with a comment that holds tags is still found by its root element,
/// and one whose first 4 KB don't tell (a long comment) still counts when the manifest lists it.
#[test]
fn sections_starting_with_comments_are_read() {
    let sec = |lead: &str| format!(r#"<?xml version="1.0"?>{lead}<hs:sec {NS}><p:p><p:run><p:t>본문</p:t></p:run></p:p></hs:sec>"#);
    let package = |section: String| {
        let mut z = craft_zip::Writer::new();
        z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
        z.add(
            "Contents/content.hpf",
            br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="s" href="Contents/body.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="header"/><opf:itemref idref="s"/></opf:spine></opf:package>"#,
            craft_zip::Compression::Deflated,
        )
        .unwrap();
        z.add("Contents/header.xml", br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, craft_zip::Compression::Deflated).unwrap();
        z.add("Contents/body.xml", section.as_bytes(), craft_zip::Compression::Deflated).unwrap();
        let (docx, _) = crate::to_docx(&z.finish().unwrap()).unwrap();
        zip_part(&docx, "word/document.xml")
    };
    assert!(package(sec("<!-- <hh:head> -->")).contains("본문"));
    assert!(package(sec(&format!("<!-- {} -->", "x".repeat(5000)))).contains("본문"));
}

/// Offsets are signed 32-bit values written unsigned: 4294966596 is -700 HWPUNIT, a little left
/// of the column, not 40 km to the right.
#[test]
fn drawing_offsets_are_signed() {
    let rect = r#"<p:rect id="1" zOrder="1" textWrap="IN_FRONT_OF_TEXT"><p:sz width="2000" height="2000"/><p:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="4294966596"/><p:curSz width="2000" height="2000"/><p:orgSz width="2000" height="2000"/></p:rect>"#;
    let (docx, _) = crate::to_docx(&hwpx(&format!(r#"<hs:sec {NS}><p:p><p:run>{rect}<p:t>글</p:t></p:run></p:p></hs:sec>"#))).unwrap();
    let body = zip_part(&docx, "word/document.xml");
    let h = &body[body.find("<wp:positionH").unwrap_or_else(|| panic!("{body}"))..];
    let off: i64 = h[h.find("<wp:posOffset>").unwrap() + 14..h.find("</wp:posOffset>").unwrap()].parse().unwrap();
    assert!(off < 0, "{h}");
}

/// A field command's backslashes escape only separators and themselves: a Windows path written
/// as it is keeps its backslashes, and one written escaped reads the same.
#[test]
fn backslashes_in_paths_are_kept() {
    use crate::convert as t;
    assert_eq!(t::unescape_backslash(r"E:\Work\other.hwp"), r"E:\Work\other.hwp");
    assert_eq!(t::unescape_backslash(r"E:\\Work\\other.hwp"), r"E:\Work\other.hwp");
    assert_eq!(t::unescape_backslash(r"a\;b\/c"), "a;b/c");
}

/// A picture of a type the converter doesn't know by name is named by its bytes: a WebP picture
/// is a .webp part of type image/webp, not WebP data called PNG.
#[test]
fn pictures_are_named_by_what_they_hold() {
    let mut pkg = crate::docx::DocxPackage::default();
    let webp = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
    assert!(pkg.add_media("a", &webp, "bin").ends_with(".webp"));
    assert!(pkg.add_media("b", b"\x89PNG\r\n", "").ends_with(".png"));
    assert!(pkg.add_media("c", b"???", "xyz").ends_with(".png"));
}

/// A WebP or TIFF picture in a new HWPX (or one added on a save) is named and listed as what it
/// is. Before, every unknown kind became a .png said to be image/png.
#[test]
fn hwpx_pictures_are_named_by_what_they_hold() {
    let webp = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
    assert_eq!(crate::hwpx_out::image_kind(&webp), ("webp", "image/webp"));
    assert_eq!(crate::hwpx_out::image_kind(b"II*\0rest"), ("tif", "image/tiff"));
    assert_eq!(crate::hwpx_out::image_kind(b"\x89PNG\r\n"), ("png", "image/png"));
}

/// A header of 100,000 styles: a style is found by its id directly, and a later definition of an
/// id replaces the first in place. Before, every lookup went through the styles before it, and
/// writing the styles looked each one's next style up that way.
#[test]
fn many_styles_are_found_by_id() {
    let n = 100_000;
    let mut head = String::from(r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:styles>"#);
    for k in 0..n {
        head.push_str(&format!(r#"<hh:style id="{k}" type="PARA" name="s{k}" nextStyleIDRef="{}"/>"#, (k + 1) % n));
    }
    head.push_str(r#"<hh:style id="7" type="PARA" name="again"/></hh:styles></hh:refList></hh:head>"#);
    let root = xml::parse(head.as_bytes(), false).unwrap();
    let h = crate::header::Header::new(Some(&root));
    assert_eq!(h.styles.len(), n);
    assert!(h.styles.iter().all(|s| h.style(s.next).is_some_and(|t| t.id == s.next)));
    assert_eq!(h.style(7).map(|s| s.name.as_str()), Some("again"));
    assert!(h.style(-1).is_none());
}

/// An HWP 5.0 DocInfo of 60,000 BIN_DATA records (120,000 picture items, by stream name and by
/// number) opens, and its pictures are found by either id. Before, each item went through every
/// item before it.
#[test]
fn hwp5_many_picture_records_open() {
    use crate::hwp5::BIN_DATA;
    let n: u16 = 60_000;
    let mut doc_info = Vec::new();
    for k in 1..=n {
        let mut d = Vec::new();
        d.extend(1u16.to_le_bytes()); // embedded
        d.extend(k.to_le_bytes());
        d.extend(3u16.to_le_bytes());
        d.extend("png".encode_utf16().flat_map(u16::to_le_bytes));
        doc_info.extend(record(BIN_DATA, 0, &d));
    }
    let mut head = b"HWP Document File".to_vec();
    head.resize(256, 0);
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
    comp.create_storage("/BodyText").unwrap();
    comp.create_storage("/BinData").unwrap();
    for (name, data) in [
        ("/FileHeader", head),
        ("/DocInfo", doc_info),
        ("/BodyText/Section0", record(crate::hwp5::PARA_HEADER, 0, &[0; 24])),
        ("/BinData/BIN0001.png", b"picture".to_vec()),
    ] {
        comp.create_stream(name).unwrap().write_all(&data).unwrap();
    }
    let pkg = crate::hwp5::open(&comp.into_inner().into_inner()).unwrap();
    assert_eq!(pkg.items.len(), 2 * usize::from(n));
    assert_eq!(pkg.binary("BIN0001").map(|b| b.1), Some(&b"picture"[..]));
    assert_eq!(pkg.binary("1").map(|b| b.1), Some(&b"picture"[..]));
    assert!(pkg.binary("2").is_none(), "listed, but no stream holds it");
}

/// An HWPX manifest of 100,000 items (all naming one picture, so all kept), all in the spine:
/// its section is found, and the picture by its last id. Before, each spine entry and each
/// picture went through the whole manifest.
#[test]
fn hwpx_many_manifest_items_open() {
    let n = 100_000;
    let mut items = String::from(
        r#"<opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="s" href="Contents/section0.xml" media-type="application/xml"/>"#,
    );
    let mut spine = String::from(r#"<opf:itemref idref="header"/><opf:itemref idref="s"/>"#);
    for k in 0..n {
        items.push_str(&format!(r#"<opf:item id="img{k}" href="BinData/img.png" media-type="image/png"/>"#));
        spine.push_str(&format!(r#"<opf:itemref idref="img{k}"/>"#));
    }
    let hpf = format!(
        r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest>{items}</opf:manifest><opf:spine>{spine}</opf:spine></opf:package>"#
    );
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
    z.add("Contents/content.hpf", hpf.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    z.add("Contents/header.xml", br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, craft_zip::Compression::Deflated).unwrap();
    let sec = format!(r#"<hs:sec {NS}><p:p><p:run><p:t>본문</p:t></p:run></p:p></hs:sec>"#);
    z.add("Contents/section0.xml", sec.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    z.add("BinData/img.png", b"picture", craft_zip::Compression::Stored).unwrap();
    let pkg = crate::package::Package::open(&z.finish().unwrap()).unwrap();
    assert_eq!(pkg.section_names, ["Contents/section0.xml"]);
    assert_eq!(pkg.binary(&format!("img{}", n - 1)).map(|b| b.1), Some(&b"picture"[..]));
}

/// An HWP 5.0 file of 20,000 picture streams and their BIN_DATA records opens, each picture
/// read with its own bytes. Before, every stream read went through the whole stream list.
#[test]
fn hwp5_many_picture_streams_open() {
    use crate::hwp5::BIN_DATA;
    let n: u16 = 20_000;
    let mut doc_info = Vec::new();
    for k in 1..=n {
        let mut d = Vec::new();
        d.extend(1u16.to_le_bytes());
        d.extend(k.to_le_bytes());
        d.extend(3u16.to_le_bytes());
        d.extend("png".encode_utf16().flat_map(u16::to_le_bytes));
        doc_info.extend(record(BIN_DATA, 0, &d));
    }
    let mut head = b"HWP Document File".to_vec();
    head.resize(256, 0);
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
    comp.create_storage("/BodyText").unwrap();
    comp.create_storage("/BinData").unwrap();
    for (name, data) in [("/FileHeader", head), ("/DocInfo", doc_info), ("/BodyText/Section0", record(crate::hwp5::PARA_HEADER, 0, &[0; 24]))] {
        comp.create_stream(name).unwrap().write_all(&data).unwrap();
    }
    for k in 1..=n {
        comp.create_stream(format!("/BinData/BIN{k:04X}.png")).unwrap().write_all(&k.to_le_bytes()).unwrap();
    }
    let pkg = crate::hwp5::open(&comp.into_inner().into_inner()).unwrap();
    assert_eq!(pkg.binary("BIN4E20").map(|b| b.1), Some(&n.to_le_bytes()[..]));
    assert_eq!(pkg.binary("1").map(|b| b.1), Some(&1u16.to_le_bytes()[..]));
}

/// Pictures named by file stem only (no manifest item) are found by the stem directly: 30,000
/// of them, among as many entries. Before, each went through every entry of the package.
#[test]
fn hwpx_pictures_named_by_stem_are_found_directly() {
    let n = 30_000;
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
    z.add("Contents/header.xml", br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, craft_zip::Compression::Deflated).unwrap();
    let sec = format!(r#"<hs:sec {NS}><p:p><p:run><p:t>본문</p:t></p:run></p:p></hs:sec>"#);
    z.add("Contents/section0.xml", sec.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    for k in 0..n {
        z.add(&format!("BinData/p{k}.png"), format!("{k}").as_bytes(), craft_zip::Compression::Stored).unwrap();
    }
    let pkg = crate::package::Package::open(&z.finish().unwrap()).unwrap();
    for k in 0..n {
        assert_eq!(pkg.binary(&format!("p{k}")).map(|b| b.1.to_vec()), Some(format!("{k}").into_bytes()));
    }
    assert!(pkg.binary("missing").is_none());
}

/// A new HWPX of a document that names no East Asian font (WordCraft's default Aptos) writes
/// its Hangul in the system's Hangul font, not in Aptos, which has no Hangul: the run's character
/// shape points at that font in its Hangul slot. (Which font has Hangul is checked with given
/// fonts in `fonts::tests`; here the machine's own.) Before, the Hangul slot named Aptos.
#[test]
fn new_hwpx_hangul_follows_the_system_font() {
    let doc = wordcraft_doc::Document::from_text("한글 문단");
    let (bytes, _) = crate::new_hwpx(&doc).unwrap();
    let head = xml::parse(zip_part(&bytes, "Contents/header.xml").as_bytes(), false).unwrap();
    let sec = xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), true).unwrap();
    // the Hangul text's run -> its character shape -> the Hangul font it names
    let run = sec.iter("hp:run").find(|r| r.iter("hp:t").any(|t| t.text.contains('한'))).expect("the Hangul run");
    let shape = run.get("charPrIDRef").expect("a character shape");
    let char_pr = head.iter("hh:charPr").find(|c| c.get("id") == Some(shape)).expect("the character shape");
    let font_id = char_pr.find("hh:fontRef").and_then(|f| f.get("hangul")).expect("a Hangul font");
    let hangul = head.iter("hh:fontface").find(|f| f.get("lang") == Some("HANGUL")).expect("the Hangul fonts");
    let face = hangul.iter("hh:font").find(|f| f.get("id") == Some(font_id)).and_then(|f| f.get("face")).expect("the font").to_string();
    match crate::fonts::hangul_default("Aptos") {
        Some(system) => assert_eq!(face, system),
        // no Hangul font on this machine: the run's font, as before
        None => assert_eq!(face, "Aptos"),
    }
}

/// The body's only paragraph replaced by a new one (no source mark): the new text can't be placed
/// by a source paragraph, so the original is kept and the save says so. Before, the original was
/// cleared and the new text dropped: an empty document, with nothing reported.
#[test]
fn write_back_keeps_a_body_whose_new_text_cant_be_placed() {
    let original = hwpx(&format!(r#"<hs:sec {NS}><p:p><p:run><p:t>원래 글</p:t></p:run></p:p></hs:sec>"#));
    let (docx, _) = crate::to_docx(&original).unwrap();
    let replaced = rezip(&docx, |xml| {
        let start = xml.find("<w:p>").or_else(|| xml.find("<w:p ")).unwrap();
        let end = xml[start..].find("</w:p>").unwrap() + start + "</w:p>".len();
        format!("{}<w:p><w:r><w:t>새 글</w:t></w:r></w:p>{}", &xml[..start], &xml[end..])
    });
    let (out, report) = crate::write_back(&original, &replaced).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("every paragraph of the body")), "{report:?}");
    let (back, _) = crate::to_docx(&out).unwrap();
    assert!(zip_part(&back, "word/document.xml").contains("원래 글"), "the original is kept");
    // everything deleted (an empty paragraph left): deleted, as before
    let emptied = rezip(&docx, |xml| {
        let start = xml.find("<w:p>").or_else(|| xml.find("<w:p ")).unwrap();
        let end = xml[start..].find("</w:p>").unwrap() + start + "</w:p>".len();
        format!("{}<w:p/>{}", &xml[..start], &xml[end..])
    });
    let (out, report) = crate::write_back(&original, &emptied).unwrap();
    assert!(!report.skipped.iter().any(|s| s.contains("every paragraph")), "{report:?}");
    let (back, _) = crate::to_docx(&out).unwrap();
    assert!(!zip_part(&back, "word/document.xml").contains("원래 글"), "deleted");
}

/// Two paragraphs merged into one whose text can't be rewritten (it holds a hyphen control the
/// writer doesn't rewrite): both are kept and the save says so. Before, the second was deleted
/// while the merge was reported as not written, so its text was lost.
#[test]
fn write_back_keeps_merged_paragraphs_when_the_merge_isnt_written() {
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><p:p><p:run><p:t>첫째<p:hypen/>문단</p:t></p:run></p:p><p:p><p:run><p:t>둘째 문단</p:t></p:run></p:p></hs:sec>"#
    ));
    let (docx, _) = crate::to_docx(&original).unwrap();
    let merged = rezip(&docx, |xml| {
        let paras: Vec<(usize, usize)> = xml.match_indices("<w:p>").map(|(i, _)| (i, i + xml[i..].find("</w:p>").unwrap() + 6)).collect();
        let first = paras.iter().find(|p| xml[p.0..p.1].contains("첫째")).copied().unwrap();
        let second = paras.iter().find(|p| xml[p.0..p.1].contains("둘째")).copied().unwrap();
        // the second paragraph's content (its source mark included) moved to the end of the first
        let inner = &xml[second.0 + "<w:p>".len()..second.1 - "</w:p>".len()];
        format!("{}{inner}</w:p>{}{}", &xml[..first.1 - "</w:p>".len()], &xml[first.1..second.0], &xml[second.1..])
    });
    let (out, report) = crate::write_back(&original, &merged).unwrap();
    let (back, _) = crate::to_docx(&out).unwrap();
    let body = zip_part(&back, "word/document.xml");
    assert!(body.contains("둘째 문단"), "the second paragraph is kept: {report:?}");
    assert!(report.skipped.iter().any(|s| s.contains("merged into it are kept")), "{report:?}");
}

/// Attributes in single quotes, or with spaces around `=`, are read and rewritten as XML allows.
/// A run whose shape is written `charPrIDRef='7'` keeps it when text is added to it. Before,
/// the shape read as 0 and the run's existing text took shape 0 too.
#[test]
fn single_quoted_attributes_are_read() {
    let attrs = crate::writer::tag_attrs(r#"<hp:run charPrIDRef='7' a = "1" b='x"y'>"#);
    assert_eq!(attrs.get("charPrIDRef").map(String::as_str), Some("7"));
    assert_eq!(attrs.get("a").map(String::as_str), Some("1"));
    assert_eq!(attrs.get("b").map(String::as_str), Some(r#"x"y"#));
    let original = hwpx(&format!(r#"<hs:sec {NS}><p:p paraPrIDRef='0'><p:run charPrIDRef='7'><p:t>원래</p:t></p:run></p:p></hs:sec>"#));
    let (docx, _) = crate::to_docx(&original).unwrap();
    let edited = rezip(&docx, |xml| xml.replacen("원래</w:t>", "원래 글</w:t>", 1));
    let (out, report) = crate::write_back(&original, &edited).unwrap();
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert!(sec.contains("원래 글"), "{sec}");
    let runs: Vec<String> = sec.match_indices("<p:run").map(|(i, _)| sec[i..i + sec[i..].find('>').unwrap()].to_string()).collect();
    assert!(runs.iter().all(|r| r.contains("charPrIDRef=\"7\"") || r.contains("charPrIDRef='7'")), "{runs:?}");
}

/// A section naming the paragraph namespace with two prefixes (`p`, declared first, and `hp`):
/// the writer counts paragraphs as the reader does, so an edit to the first paragraph changes the
/// first. Before, it indexed only `p:p` and the edit went to the second paragraph of the same text.
#[test]
fn write_back_counts_paragraphs_of_every_prefix() {
    let original = hwpx(
        r#"<hs:sec xmlns:p="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><hp:p><hp:run><hp:t>같은 글</hp:t></hp:run></hp:p><p:p><p:run><p:t>같은 글</p:t></p:run></p:p></hs:sec>"#,
    );
    let (docx, _) = crate::to_docx(&original).unwrap();
    let edited = rezip(&docx, |xml| xml.replacen("같은 글</w:t>", "같은 글 첫째</w:t>", 1));
    let (out, report) = crate::write_back(&original, &edited).unwrap();
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    let first = sec.find("<hp:p>").unwrap();
    let second = sec.find("<p:p>").unwrap();
    assert!(sec[first..second].contains("같은 글 첫째"), "the first paragraph is edited: {sec}");
    assert!(!sec[second..].contains("첫째"), "the second is not: {sec}");
}

/// Text boxes that keep naming the same stories (each level two boxes of the next story, 20
/// levels deep: about a million expansions within the depth limit) are written within a budget,
/// the rest reported. Before, every expansion was written again.
#[test]
fn new_hwpx_text_boxes_naming_the_same_stories_are_bounded() {
    use wordcraft_doc::{CharProps, InlineObject, Paragraph, PartKind, para::ShapeKind, para_block};
    let boxes = |story: Option<u32>| {
        let mut p = Paragraph::with_text("x", CharProps::default());
        if let Some(story) = story {
            for _ in 0..2 {
                let shape = InlineObject::Shape {
                    kind: ShapeKind::TextBox,
                    w: 50.0,
                    h: 20.0,
                    fill: None,
                    stroke: None,
                    stroke_width: 0.0,
                    float: Default::default(),
                    story: Some(story),
                    freeform: None,
                    effects: Default::default(),
                    extra: Default::default(),
                };
                p.insert_object(0, shape, &CharProps::default()).unwrap();
            }
        }
        vec![para_block(p)]
    };
    let mut doc = wordcraft_doc::Document::from_text("body");
    let mut next = None;
    for _ in 0..20 {
        let id = doc.add_part(PartKind::TextBox, Vec::new());
        if let Some(part) = doc.parts.get_mut(&id) {
            part.blocks = boxes(next);
        }
        next = Some(id);
    }
    doc.body.extend(boxes(next));
    let started = std::time::Instant::now();
    let (_, report) = crate::new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("past the most one file writes")), "{report:?}");
    assert!(started.elapsed().as_secs() < 60, "bounded: {:?}", started.elapsed());
}

/// A first-page header with content and no default header (HWPX has no first-page header of its
/// own) is reported as not written. Before, nothing was written and nothing reported.
#[test]
fn new_hwpx_reports_a_first_page_header_alone() {
    use wordcraft_doc::{CharProps, Paragraph, PartKind, para_block};
    let mut doc = wordcraft_doc::Document::from_text("body");
    let first = doc.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("first page only", CharProps::default()))]);
    doc.last_section.title_page = true;
    doc.last_section.headers.first = Some(first);
    doc.last_section.headers.default = None;
    let (_, report) = crate::new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("first-page headers")), "{report:?}");
}

/// A stand-in's original name goes back only on runs the reader swapped: the swapped run names
/// HY헤드라인M again, and a run that names NanumGothic itself keeps it. Before, the stand-in's
/// reverse mapping renamed both.
#[test]
fn new_hwpx_restores_font_names_only_on_swapped_runs() {
    use wordcraft_doc::{CharProps, Paragraph, para_block};
    let props =
        |cs: &str| CharProps {
            font: Some("나눔고딕".into()), font_ea: Some("나눔고딕".into()), font_cs: Some(cs.into()), ..Default::default()
        };
    let mut doc = wordcraft_doc::Document::from_text("");
    doc.body.clear();
    doc.body.push(para_block(Paragraph::with_text("바뀐 글", props("HY헤드라인M"))));
    doc.body.push(para_block(Paragraph::with_text("원래 글", props("나눔고딕"))));
    doc.custom_props.push(wordcraft_doc::CustomProp {
        name: crate::fonts::PROPERTY.into(),
        kind: "lpwstr".into(),
        value: r#"{"HY헤드라인M": "나눔고딕"}"#.into(),
        ..Default::default()
    });
    let (bytes, _) = crate::new_hwpx(&doc).unwrap();
    let head = xml::parse(zip_part(&bytes, "Contents/header.xml").as_bytes(), false).unwrap();
    let sec = xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), true).unwrap();
    let hangul_of = |text: &str| {
        let run = sec.iter("hp:run").find(|r| r.iter("hp:t").any(|t| t.text.contains(text))).expect("the run");
        let shape = run.get("charPrIDRef").unwrap();
        let cp = head.iter("hh:charPr").find(|c| c.get("id") == Some(shape)).unwrap();
        let id = cp.find("hh:fontRef").and_then(|f| f.get("hangul")).unwrap();
        let list = head.iter("hh:fontface").find(|f| f.get("lang") == Some("HANGUL")).unwrap();
        list.iter("hh:font").find(|f| f.get("id") == Some(id)).and_then(|f| f.get("face")).unwrap().to_string()
    };
    assert_eq!(hangul_of("바뀐"), "HY헤드라인M");
    assert_eq!(hangul_of("원래"), "나눔고딕", "a run naming the stand-in itself keeps it");
}

/// A table of 10,001 rows (the document model allows 32,767) written to a new HWPX reads back
/// whole. Before, reading stopped at 10,000 rows and the last row's text was gone.
#[test]
fn new_hwpx_long_tables_read_back_whole() {
    let mut doc = wordcraft_doc::Document::from_text("before");
    let mut t = wordcraft_doc::Table::new(10_001, 1, 200.0);
    if let Some(cell) = t.rows.last_mut().and_then(|r| r.cells.first_mut()) {
        cell.blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("last row", wordcraft_doc::CharProps::default()))];
    }
    doc.body.push(std::sync::Arc::new(wordcraft_doc::Block::Table(t)));
    let (bytes, report) = crate::new_hwpx(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{report:?}");
    let (docx, _) = crate::to_docx(&bytes).unwrap();
    let body = zip_part(&docx, "word/document.xml");
    assert_eq!(body.matches("<w:tr>").count() + body.matches("<w:tr ").count(), 10_001);
    // the space between the words goes in a run of its own (its width): the words, each
    assert!(body.contains(">last</w:t>") && body.contains(">row</w:t>"));
}

/// A section using the paragraph namespace as its default (no prefix): an edit writes `hp:`
/// runs, and the section root binds `hp`, so the saved part reads with the edit. Before, the
/// runs named a prefix the part never declared.
#[test]
fn write_back_binds_hp_in_default_namespace_sections() {
    let original = hwpx(
        r#"<hs:sec xmlns="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><p><run><t>original text</t></run></p></hs:sec>"#,
    );
    let (docx, _) = crate::to_docx(&original).unwrap();
    assert!(zip_part(&docx, "word/document.xml").contains("original text"), "the reader takes the default namespace");
    let edited = rezip(&docx, |xml| xml.replacen("original text</w:t>", "original text, edited</w:t>", 1));
    let (out, report) = crate::write_back(&original, &edited).unwrap();
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    let root = &sec[sec.find("<hs:sec").unwrap()..];
    assert!(root[..root.find('>').unwrap()].contains(r#"xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph""#), "{sec}");
    let (back, _) = crate::to_docx(&out).unwrap();
    assert!(zip_part(&back, "word/document.xml").contains("edited"), "the edit reads back");
}

/// A paragraph merged into a plain one, the merged one holding a hyphen control the writer can't
/// clear: the merge isn't written, both stay as they were, and the save says so. Before, the
/// first took both texts while the second couldn't be removed: its text was there twice.
#[test]
fn write_back_writes_no_merge_it_cant_finish() {
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><p:p><p:run><p:t>첫째 문단</p:t></p:run></p:p><p:p><p:run><p:t>둘째<p:hypen/>문단</p:t></p:run></p:p></hs:sec>"#
    ));
    let (docx, _) = crate::to_docx(&original).unwrap();
    let merged = rezip(&docx, |xml| {
        let paras: Vec<(usize, usize)> = xml.match_indices("<w:p>").map(|(i, _)| (i, i + xml[i..].find("</w:p>").unwrap() + 6)).collect();
        let first = paras.iter().find(|p| xml[p.0..p.1].contains("첫째")).copied().unwrap();
        let second = paras.iter().find(|p| xml[p.0..p.1].contains("둘째")).copied().unwrap();
        let inner = &xml[second.0 + "<w:p>".len()..second.1 - "</w:p>".len()];
        format!("{}{inner}</w:p>{}{}", &xml[..first.1 - "</w:p>".len()], &xml[first.1..second.0], &xml[second.1..])
    });
    let (out, report) = crate::write_back(&original, &merged).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("the merge is not written")), "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert_eq!(sec.matches("둘째").count(), 1, "the second text once: {sec}");
    assert!(sec.contains("<p:t>첫째 문단</p:t>"), "the first as it was: {sec}");
}

/// A paragraph added after an `hp:p` in a part that declares `p` first for the same namespace is
/// closed by the name its copied start tag opens. Before, it opened `<hp:p` and closed `</p:p>`,
/// which no XML reader accepts.
#[test]
fn write_back_closes_new_paragraphs_by_their_own_name() {
    let original = hwpx(
        r#"<hs:sec xmlns:p="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><hp:p><hp:run><hp:t>첫째</hp:t></hp:run></hp:p><p:p><p:run><p:t>둘째</p:t></p:run></p:p></hs:sec>"#,
    );
    let (docx, _) = crate::to_docx(&original).unwrap();
    let added = rezip(&docx, |xml| {
        let first = xml.find("첫째").unwrap();
        let end = first + xml[first..].find("</w:p>").unwrap() + "</w:p>".len();
        format!("{}<w:p><w:r><w:t>새 문단</w:t></w:r></w:p>{}", &xml[..end], &xml[end..])
    });
    let (out, report) = crate::write_back(&original, &added).unwrap();
    assert_eq!(report.inserted, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert!(sec.contains("새 문단"), "{sec}");
    for pfx in ["hp", "p"] {
        let opens = sec.matches(&format!("<{pfx}:p>")).count() + sec.matches(&format!("<{pfx}:p ")).count();
        assert_eq!(opens, sec.matches(&format!("</{pfx}:p>")).count(), "{pfx}:p opened and closed alike: {sec}");
    }
}

/// A section root binding `hp` in single quotes keeps its one binding when edited, and one
/// binding `hp` to another namespace keeps its text (the writer's `hp:` would mean something
/// else there) and the save says so. Before, a second `xmlns:hp` made the root ill-formed.
#[test]
fn write_back_reads_the_roots_hp_binding() {
    let edit = |section: &str| {
        let original = hwpx(section);
        let (docx, _) = crate::to_docx(&original).unwrap();
        let edited = rezip(&docx, |xml| xml.replacen("본문</w:t>", "본문 고침</w:t>", 1));
        let (out, report) = crate::write_back(&original, &edited).unwrap();
        (zip_part(&out, "Contents/section0.xml"), report)
    };
    let (sec, report) = edit(
        r#"<hs:sec xmlns:hp='http://www.hancom.co.kr/hwpml/2011/paragraph' xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><hp:p><hp:run><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#,
    );
    assert_eq!(report.modified, 1, "{report:?}");
    let root = &sec[..sec.find('>').unwrap()];
    assert_eq!(root.matches("xmlns:hp").count(), 1, "{root}");
    let (sec, report) = edit(
        r#"<hs:sec xmlns:hp="urn:other" xmlns:p="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><p:p><p:run><p:t>본문</p:t></p:run></p:p></hs:sec>"#,
    );
    assert!(report.skipped.iter().any(|s| s.contains("another namespace")), "{report:?}");
    assert!(!sec.contains("고침"), "kept as it was: {sec}");
}

/// A paragraph that starts with a document's own bookmark named like the source marks ("_heading")
/// still gets its source mark, so its edit is written back. Before, the bookmark's "_h" read as a
/// mark already there: the paragraph got none, and its edit had no source paragraph.
#[test]
fn user_bookmarks_starting_with_h_are_not_source_marks() {
    let original =
        hwpx(&format!(r#"<hs:sec {NS}><p:p><p:run><p:ctrl><p:bookmark name="_heading"/></p:ctrl><p:t>책갈피 문단</p:t></p:run></p:p></hs:sec>"#));
    let (docx, _) = crate::to_docx(&original).unwrap();
    let body = zip_part(&docx, "word/document.xml");
    assert!(body.contains(r#"w:name="_hp_0_0""#), "{body}");
    let edited = rezip(&docx, |xml| xml.replacen("책갈피 문단</w:t>", "책갈피 문단 고침</w:t>", 1));
    let (out, report) = crate::write_back(&original, &edited).unwrap();
    assert_eq!(report.modified, 1, "{report:?}");
    assert!(zip_part(&out, "Contents/section0.xml").contains("고침"));
}

/// A section asking for a page number at the bottom while it inherits a text-only footer gets a
/// footer of its own: the inherited text and the page number. Before, the inherited footer was
/// taken for one showing the number, and the number was left out.
#[test]
fn an_inherited_footer_without_a_number_takes_one() {
    let first = format!(
        r#"<hs:sec {NS}><p:p><p:run><p:secPr/><p:ctrl><p:footer applyPageType="BOTH"><p:subList><p:p><p:run><p:t>꼬리말 글</p:t></p:run></p:p></p:subList></p:footer></p:ctrl><p:t>첫 구역</p:t></p:run></p:p></hs:sec>"#
    );
    let second = format!(
        r#"<hs:sec {NS}><p:p><p:run><p:secPr/><p:ctrl><p:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></p:ctrl><p:t>둘째 구역</p:t></p:run></p:p></hs:sec>"#
    );
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
    z.add("Contents/header.xml", br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, craft_zip::Compression::Deflated).unwrap();
    z.add("Contents/section0.xml", first.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    z.add("Contents/section1.xml", second.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    let (docx, _) = crate::to_docx(&z.finish().unwrap()).unwrap();
    let footers: Vec<String> = zip_names(&docx).into_iter().filter(|n| n.starts_with("word/footer")).map(|n| zip_part(&docx, &n)).collect();
    assert!(footers.iter().any(|f| f.contains("꼬리말 글") && f.contains("PAGE")), "a footer with the text and the number: {footers:?}");
}

/// Spaces are sized by the shape of the run they're in, also after a form inside the run used
/// another shape giving the same rPr (one that keeps the font's space width). Before, the form's
/// shape stood for the rPr from then on, and the run's spaces lost their Hangul width.
#[test]
fn spaces_follow_their_runs_shape_after_a_form() {
    let header = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:fontfaces itemCnt="2"><hh:fontface lang="HANGUL" fontCnt="1"><hh:font id="0" face="Arial" type="TTF"/></hh:fontface><hh:fontface lang="LATIN" fontCnt="1"><hh:font id="0" face="Arial" type="TTF"/></hh:fontface></hh:fontfaces><hh:charProperties itemCnt="2"><hh:charPr id="1" height="2000"><hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/></hh:charPr><hh:charPr id="2" height="2000" useFontSpace="1"><hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/></hh:charPr></hh:charProperties></hh:refList></hh:head>"#;
    let doc = |form: &str| {
        let sec = format!(r#"<hs:sec {NS}><p:p><p:run charPrIDRef="1">{form}<p:t>가 나 다</p:t></p:run></p:p></hs:sec>"#);
        let mut z = craft_zip::Writer::new();
        z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
        z.add("Contents/header.xml", header.as_bytes(), craft_zip::Compression::Deflated).unwrap();
        z.add("Contents/section0.xml", sec.as_bytes(), craft_zip::Compression::Deflated).unwrap();
        let (docx, _) = crate::to_docx(&z.finish().unwrap()).unwrap();
        let body = zip_part(&docx, "word/document.xml");
        // the runs of the text after the form
        body[body.find("가").unwrap()..].matches("<w:spacing w:val=").count()
    };
    let alone = doc("");
    assert!(alone > 0, "the spaces get the Hangul width");
    let after_form = doc(r#"<p:checkBtn charPrIDRef="2" caption="x" value="0"><p:sz width="1000" height="1000"/></p:checkBtn>"#);
    assert_eq!(after_form, alone, "after a form of another shape too");
}

/// Tables nested past the writer's depth limit are left out and reported. Before, they and
/// their cells' text went without a notice.
#[test]
fn new_hwpx_reports_tables_nested_too_deep() {
    use wordcraft_doc::{Block, Table};
    let mut table = Table::new(1, 1, 100.0);
    for _ in 0..40 {
        let mut outer = Table::new(1, 1, 100.0);
        if let Some(cell) = outer.rows.first_mut().and_then(|r| r.cells.first_mut()) {
            cell.blocks = vec![std::sync::Arc::new(Block::Table(table))];
        }
        table = outer;
    }
    let mut doc = wordcraft_doc::Document::from_text("before");
    doc.body.push(std::sync::Arc::new(Block::Table(table)));
    let (_, report) = crate::new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("tables nested too deep")), "{report:?}");
}

/// A freeform of two paths inside groups as deep as the writer goes (where no group is left to
/// hold its paths apart) writes its first path and reports the other. Before, the second path
/// went without a notice.
#[test]
fn new_hwpx_reports_freeform_paths_at_the_group_limit() {
    use wordcraft_doc::freeform::{FreePath, Freeform};
    use wordcraft_doc::para::ShapeKind;
    use wordcraft_doc::{CharProps, InlineObject, Paragraph, para_block};
    let path = |x: f32| FreePath { pts: vec![[x, 0.0], [x + 50.0, 50.0]], closed: false };
    let ff = Freeform { w: 100.0, h: 100.0, paths: vec![path(0.0), path(20.0)], ..Default::default() };
    let mut obj = InlineObject::Shape {
        kind: ShapeKind::Freeform,
        w: 50.0,
        h: 50.0,
        fill: None,
        stroke: Some(wordcraft_doc::props::Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float: Default::default(),
        story: None,
        freeform: Some(std::sync::Arc::new(ff)),
        effects: Default::default(),
        extra: Default::default(),
    };
    for _ in 0..8 {
        obj = InlineObject::Group {
            w: 50.0,
            h: 50.0,
            float: Default::default(),
            ch_w: 50.0,
            ch_h: 50.0,
            children: vec![wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj }],
            canvas: None,
        };
    }
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, obj, &CharProps::default()).unwrap();
    let mut doc = wordcraft_doc::Document::from_text("");
    doc.body.clear();
    doc.body.push(para_block(p));
    let (_, report) = crate::new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("freeform paths past the first")), "{report:?}");
}

/// Source paragraphs moved in WordCraft can't be moved in the original file (they keep their
/// places): the save says so. Before, both texts read unchanged and nothing was reported.
#[test]
fn write_back_reports_moved_paragraphs() {
    let original = hwpx(&format!(r#"<hs:sec {NS}><p:p><p:run><p:t>첫째</p:t></p:run></p:p><p:p><p:run><p:t>둘째</p:t></p:run></p:p></hs:sec>"#));
    let (docx, _) = crate::to_docx(&original).unwrap();
    let swapped = rezip(&docx, |xml| {
        let paras: Vec<(usize, usize)> = xml.match_indices("<w:p>").map(|(i, _)| (i, i + xml[i..].find("</w:p>").unwrap() + 6)).collect();
        let first = paras.iter().find(|p| xml[p.0..p.1].contains("첫째")).copied().unwrap();
        let second = paras.iter().find(|p| xml[p.0..p.1].contains("둘째")).copied().unwrap();
        format!("{}{}{}{}{}", &xml[..first.0], &xml[second.0..second.1], &xml[first.1..second.0], &xml[first.0..first.1], &xml[second.1..])
    });
    let (_, report) = crate::write_back(&original, &swapped).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("moved")), "{report:?}");
    let (_, report) = crate::write_back(&original, &docx).unwrap();
    assert!(report.skipped.is_empty(), "{report:?}");
}

/// A stream of nothing but empty records (4 bytes each, about 50 in memory) is refused past
/// [`crate::hwp5::MAX_RECORDS`] instead of taking gigabytes. Before, 256 MB of zeros made 67
/// million records.
#[test]
fn hwp5_too_many_records_is_an_error() {
    let zeros = vec![0u8; 4 * (crate::hwp5::MAX_RECORDS + 1)];
    assert_eq!(crate::to_docx(&hwp_with(&zeros)).unwrap_err(), crate::Error::TooLarge("records"));
    assert!(crate::to_docx(&hwp_with(&zeros[..4 * crate::hwp5::MAX_RECORDS])).is_ok());
}

/// A record header claiming more than the stream holds (the largest extended size) ends the
/// records there: no step past the end, which on a 32-bit target overflowed the offset.
#[test]
fn hwp5_record_sizes_past_the_end_stop_the_records() {
    let mut data = vec![0xFF; 11];
    let r = crate::hwp5::Records::parse(&data);
    assert_eq!(r.recs.len(), 1);
    data = record(0x42, 0, &[0; 22]);
    data.extend((0x43u32 | (0xFFF << 20)).to_le_bytes());
    data.extend(u32::MAX.to_le_bytes());
    data.extend(record(0x44, 0, &[0; 8])); // inside the oversized record: not a record of its own
    let r = crate::hwp5::Records::parse(&data);
    assert_eq!(r.recs.iter().map(|x| x.tag).collect::<Vec<_>>(), [0x42, 0x43]);
    assert_eq!(r.data(1).len(), 12, "the oversized record keeps what the stream holds");
}

/// A TABLE record gives its cell zone count (up to 65,535) and then 10 bytes a zone; only the
/// zones it holds are read. Before, a count with no zone bytes made 65,535 empty zones a table.
#[test]
fn hwp5_table_zones_are_only_those_the_record_holds() {
    use crate::hwp5::{CTRL_HEADER, PARA_HEADER, PARA_TEXT, TABLE};
    let text: Vec<u8> = [11u16, 0, 0, 0, 0, 0, 0, 11, 13].iter().flat_map(|c| c.to_le_bytes()).collect();
    let mut tbl = b" lbt".to_vec();
    tbl.resize(46, 0);
    // attributes, 1 row, 1 column, spacing, margins, one row size, border fill, 65,535 zones
    let mut td = vec![0u8; 24];
    td[4..6].copy_from_slice(&1u16.to_le_bytes());
    td[6..8].copy_from_slice(&1u16.to_le_bytes());
    td[22..24].copy_from_slice(&u16::MAX.to_le_bytes());
    let zones = |td: &[u8]| {
        let mut sec = record(PARA_HEADER, 0, &[0; 24]);
        sec.extend(record(PARA_TEXT, 1, &text));
        sec.extend(record(CTRL_HEADER, 1, &tbl));
        sec.extend(record(TABLE, 2, td));
        crate::hwp5::section(&sec).iter("hp:cellzone").count()
    };
    assert_eq!(zones(&td), 0);
    let mut one = td.clone();
    one.extend([0u8, 0, 0, 0, 1, 0, 1, 0, 2, 0]); // one zone, and a few bytes short of a second
    one.extend([0u8; 9]);
    assert_eq!(zones(&one), 1);
}

/// Table cells a file declares but doesn't hold are made up to a budget for the whole document
/// ([`crate::convert::MAX_MADE_CELLS`]); past it, declared rows without cells are left out. A
/// table can declare 10,000 rows of 64 columns with one cell; before, that alone made 640,000
/// cells, and repeating it took the memory of millions.
#[test]
fn declared_but_missing_table_cells_are_bounded() {
    let tbl = |col: u32, text: &str| {
        format!(
            r#"<p:tbl rowCnt="10000" colCnt="64"><p:sz width="20000" height="2000"/><p:pos treatAsChar="1"/><p:tr><p:tc><p:cellAddr rowAddr="0" colAddr="{col}"/><p:cellSz width="300" height="300"/><p:subList><p:p><p:run><p:t>{text}</p:t></p:run></p:p></p:subList></p:tc></p:tr></p:tbl>"#
        )
    };
    // the first table spends the budget; the second holds its one cell after a gap (column 1)
    let sec = format!(r#"<hs:sec {NS}><p:p><p:run>{}</p:run></p:p><p:p><p:run>{}</p:run></p:p></hs:sec>"#, tbl(0, "첫째 칸"), tbl(1, "둘째 칸"));
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
    z.add("Contents/section0.xml", sec.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    let (docx, report) = crate::to_docx(&z.finish().unwrap()).unwrap();
    let body = zip_part(&docx, "word/document.xml");
    let cells = body.matches("<w:tc>").count() as i64;
    assert!(cells <= crate::convert::MAX_MADE_CELLS + 4, "{cells} cells");
    // every cell the file holds is kept, the one after a gap once the budget is spent too
    assert!(body.contains("첫째 칸") && body.contains("둘째 칸"), "the cells the file holds are kept");
    assert!(report.stats.contains_key("table rows left out (too many empty cells)"), "{:?}", report.stats);
}

/// A text box whose story holds a text box showing that same story: written once, the inner
/// one left out and reported. Before, each story started its depth over and the writing
/// recursed until the stack overflowed.
#[test]
fn new_hwpx_a_text_box_holding_itself_is_left_out() {
    use wordcraft_doc::{CharProps, InlineObject, Paragraph, PartKind, para::ShapeKind, para_block};
    let box_para = |story: u32| {
        let shape = InlineObject::Shape {
            kind: ShapeKind::TextBox,
            w: 50.0,
            h: 20.0,
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            float: Default::default(),
            story: Some(story),
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        let mut p = Paragraph::with_text("상자 글", CharProps::default());
        p.insert_object(0, shape, &CharProps::default()).unwrap();
        vec![para_block(p)]
    };
    let mut doc = wordcraft_doc::Document::from_text("본문");
    let id = doc.add_part(PartKind::TextBox, Vec::new());
    if let Some(part) = doc.parts.get_mut(&id) {
        part.blocks = box_para(id);
    }
    doc.body.extend(box_para(id));
    let (bytes, report) = crate::new_hwpx(&doc).unwrap();
    assert!(zip_part(&bytes, "Contents/section0.xml").contains("상자 글"));
    assert!(report.skipped.iter().any(|s| s.contains("nested in themselves")), "{report:?}");
}

/// `a` rebuilt from the edit list into `b`, checking that the list covers both texts in order.
fn apply_opcodes(a: &str, b: &str) -> (String, usize) {
    use crate::writer::tests_api::opcodes;
    let (ac, bc): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut out, mut edits, mut i, mut j) = (String::new(), 0, 0, 0);
    for (op, a1, a2, b1, b2) in opcodes(a, b) {
        assert_eq!((a1, b1), (i, j), "{a:?} -> {b:?}: in order");
        // each kind does what the writer does with it: equal spans come from the source, a
        // delete takes only source characters and writes none, an insert only target ones
        match op {
            "equal" => {
                assert_eq!(ac[a1..a2], bc[b1..b2], "{a:?} -> {b:?}");
                out.extend(&ac[a1..a2]);
            }
            "delete" => {
                assert_eq!(b1, b2, "{a:?} -> {b:?}: a delete takes no target characters");
                edits += a2 - a1;
            }
            "insert" => {
                assert_eq!(a1, a2, "{a:?} -> {b:?}: an insert takes no source characters");
                out.extend(&bc[b1..b2]);
                edits += b2 - b1;
            }
            "replace" => {
                out.extend(&bc[b1..b2]);
                edits += (a2 - a1) + (b2 - b1);
            }
            other => panic!("{a:?} -> {b:?}: unknown edit {other}"),
        }
        (i, j) = (a2, b2);
    }
    assert_eq!((i, j), (ac.len(), bc.len()), "{a:?} -> {b:?}: covers both");
    (out, edits)
}

/// The fewest characters deleted plus inserted to turn `a` into `b` (by the longest common
/// subsequence, worked out cell by cell).
fn fewest_edits(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row = vec![0usize; b.len() + 1];
    for x in &a {
        let mut diag = 0;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diag + 1 } else { up.max(row[j]) };
            diag = up;
        }
    }
    a.len() + b.len() - 2 * row[b.len()]
}

#[test]
fn write_back_finds_the_fewest_edits_and_keeps_shapes() {
    use crate::writer::tests_api::{opcodes, rewrite_text};
    assert_eq!(
        opcodes("abcd", "acbd"),
        [("equal", 0, 1, 0, 1), ("delete", 1, 2, 1, 1), ("equal", 2, 3, 1, 2), ("insert", 3, 3, 2, 3), ("equal", 3, 4, 3, 4)]
    );
    assert_eq!(opcodes("", "x"), [("insert", 0, 0, 0, 1)]);
    assert_eq!(opcodes("x", ""), [("delete", 0, 1, 0, 0)]);
    assert_eq!(opcodes("", ""), []);
    assert_eq!(opcodes("가나다", "가나다"), [("equal", 0, 3, 0, 3)]);
    assert_eq!(opcodes("가나다", "가X다"), [("equal", 0, 1, 0, 1), ("replace", 1, 2, 1, 2), ("equal", 2, 3, 2, 3)]);
    // Every pair of short texts over a small alphabet: the list rebuilds `b` with no more
    // edits than the fewest possible.
    let texts: Vec<String> = (0..4)
        .flat_map(|len| {
            (0..3usize.pow(len)).map(move |mut v| {
                (0..len)
                    .map(|_| {
                        let c = ['a', 'b', '가'][v % 3];
                        v /= 3;
                        c
                    })
                    .collect()
            })
        })
        .collect();
    for a in &texts {
        for b in &texts {
            let (out, edits) = apply_opcodes(a, b);
            assert_eq!(&out, b);
            assert_eq!(edits, fewest_edits(a, b), "{a:?} -> {b:?}");
        }
    }
    for (a, b) in [("the quick brown fox", "the quack brown box"), ("문단의 글자를 고쳤다", "문단 글자를 많이 고쳤다"), ("abcabba", "cbabac")]
    {
        let (out, edits) = apply_opcodes(a, b);
        assert_eq!((out.as_str(), edits), (b, fewest_edits(a, b)), "{a:?} -> {b:?}");
    }
    // typed text takes the shape of the character before it
    let shapes = |after: &str| rewrite_text(&[("가", "1"), ("나", "2")], after).into_iter().map(|(c, k)| format!("{c}{k}")).collect::<Vec<_>>();
    assert_eq!(shapes("가X나"), ["가1", "X1", "나2"]);
    assert_eq!(shapes("Y가나"), ["Y1", "가1", "나2"]);
}

/// Long or very different texts stay within the edit budget: a few edits in a long paragraph
/// are still found one by one; past the budget the middle is one replaced block.
#[test]
fn write_back_compares_long_texts_within_bounds() {
    use crate::writer::tests_api::opcodes;
    let long: String = "가나다라마바사 ".repeat(20_000);
    let mut edited = long.clone();
    edited.insert_str("가나다라마바사 ".len() * 3_636, "새 글");
    edited.insert(3, 'X');
    let (out, edits) = apply_opcodes(&long, &edited);
    assert_eq!((out, edits), (edited.clone(), 4));
    let (a, b) = ("a".repeat(3_000), "b".repeat(3_000));
    assert_eq!(opcodes(&a, &b), [("replace", 0, 3_000, 0, 3_000)]);
    let (a, b) = (format!("<{}>", "ab".repeat(2_000)), format!("<{}>", "ba".repeat(2_000).replace('a', "c")));
    let ops = opcodes(&a, &b);
    assert_eq!(ops.first(), Some(&("equal", 0, 1, 0, 1)));
    assert_eq!(apply_opcodes(&a, &b).0, b);
}

/// A DOCX with the source marks renamed, as one never converted from a Hangul file.
fn without_marks(docx: &[u8]) -> Vec<u8> {
    rezip(docx, |xml| xml.replace("\"_h", "\"_x"))
}

/// A DOCX package with its `word/document.xml` changed by `edit`.
fn rezip(docx: &[u8], edit: impl FnOnce(String) -> String) -> Vec<u8> {
    let mut edit = Some(edit);
    let mut src = craft_zip::Archive::open(docx, TEST_ZIP).unwrap();
    let mut z = craft_zip::Writer::new();
    for name in zip_names(docx) {
        let mut data = src.read(&name, 64 << 20).unwrap().unwrap();
        if name == "word/document.xml"
            && let Some(edit) = edit.take()
        {
            data = edit(String::from_utf8(data).unwrap()).into_bytes();
        }
        z.add(&name, &data, craft_zip::Compression::Deflated).unwrap();
    }
    z.finish().unwrap()
}

/// Text split by an object (two groups, "ab" and "cd"): an edit inside one group stays in it,
/// text typed where the object is goes with the group before, an edit across it is refused.
#[test]
fn write_back_edits_text_around_objects() {
    use crate::writer::tests_api::segment_spans;
    let groups = [(0, 2), (2, 4)];
    assert_eq!(segment_spans("abcd", "aXbcd", &groups), Some(vec![(0, 3), (3, 5)]));
    assert_eq!(segment_spans("abcd", "abcYd", &groups), Some(vec![(0, 2), (2, 5)]));
    assert_eq!(segment_spans("abcd", "abZcd", &groups), Some(vec![(0, 3), (3, 5)]));
    assert_eq!(segment_spans("abcd", "ad", &groups), None);
}

/// A page control in the run of a paragraph's text (Hancom writes page hiding after the text
/// it is typed in): the edit is written and the control kept, once, in a run after the text.
/// Without this the paragraph was skipped as "text and objects in one run".
#[test]
fn write_back_edits_text_beside_page_controls() {
    let hiding = r#"<p:ctrl><p:pageHiding hideHeader="1" hideFooter="0" hideMasterPage="1" hideBorder="0" hideFill="0" hidePageNum="0"/></p:ctrl>"#;
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><p:p><p:run charPrIDRef="2"><p:t> 2. 지도</p:t>{hiding}<p:t/></p:run></p:p><p:p><p:run><p:t>둘째</p:t></p:run></p:p></hs:sec>"#
    ));
    let docx = edited_docx(&original, "2. 지도", "새 2. 지도");
    let (out, report) = crate::write_back(&original, &docx).unwrap();
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert_eq!(sec.matches("<p:pageHiding ").count(), 1, "{sec}");
    let (text, ctrl) = (sec.find("새 2. 지도").unwrap(), sec.find("<p:pageHiding ").unwrap());
    assert!(text < ctrl, "the control follows the text: {sec}");
    assert!(sec.contains(&format!(r#"<p:run charPrIDRef="2">{hiding}</p:run>"#)), "{sec}");
    let (back, _) = crate::to_docx(&out).unwrap();
    assert!(crate::convert::w_t_texts(&zip_part(&back, "word/document.xml")).contains("새 2. 지도"));
}

/// A paragraph holding two tables reads as several DOCX paragraphs, the further ones all
/// marked `_hq_` with the paragraph's number. Each is compared with the piece at the same
/// place, not with the first: saved unchanged, nothing reads as edited.
#[test]
fn write_back_compares_each_piece_of_a_paragraph() {
    let tbl = |t: &str| {
        format!(
            r#"<p:tbl rowCnt="1" colCnt="1"><p:tr><p:tc><p:cellAddr rowAddr="0" colAddr="0"/><p:cellSz width="5000" height="1000"/><p:subList><p:p><p:run><p:t>{t}</p:t></p:run></p:p></p:subList></p:tc></p:tr></p:tbl>"#
        )
    };
    let original =
        hwpx(&format!(r#"<hs:sec {NS}><p:p><p:run><p:t>앞</p:t>{}<p:t>가운데</p:t>{}<p:t>뒤</p:t></p:run></p:p></hs:sec>"#, tbl("칸1"), tbl("칸2")));
    let (docx, _) = crate::to_docx(&original).unwrap();
    let z = docx.clone();
    let body = zip_part(&z, "word/document.xml");
    assert!(body.matches("\"_hq_0_0\"").count() >= 2, "{body}");
    let (_, report) = crate::write_back(&original, &docx).unwrap();
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(report.modified, 0, "{report:?}");
}

#[test]
fn write_back_hostile_inputs_are_errors() {
    let original = hwpx(&format!(r#"<hs:sec {NS}><p:p><p:run><p:t>a</p:t></p:run></p:p></hs:sec>"#));
    assert!(crate::write_back(&original, b"not a zip").is_err());
    assert!(crate::write_back(b"garbage", &original).is_err());
    // an unrelated DOCX: nothing is marked, nothing changes
    let (docx, _) = crate::to_docx(&hwp("x", 0)).unwrap();
    let docx = without_marks(&docx);
    let (out, report) = crate::write_back(&original, &docx).unwrap();
    assert_eq!(report.modified + report.inserted, 0);
    assert!(!out.is_empty());
}

/// A document that never was an HWPX becomes a package our reader (and Hancom's viewer) opens:
/// text, character shapes, paragraph shapes and page breaks come back.
#[test]
fn new_hwpx_holds_the_document() {
    let mut doc = wordcraft_doc::Document::from_text("첫 문단 & <기호>\n둘째 문단");
    if let Some(wordcraft_doc::Block::Para(p)) = doc.body.first_mut().map(std::sync::Arc::make_mut) {
        for r in &mut p.runs {
            r.props.bold = Some(true);
            r.props.size = Some(20.0);
        }
        p.props.align = Some(wordcraft_doc::Align::Center);
        p.text.push(wordcraft_doc::para::PAGE_BREAK);
        if let Some(r) = p.runs.last_mut() {
            r.len += wordcraft_doc::para::PAGE_BREAK.len_utf8();
        }
    }
    let (bytes, report) = crate::new_hwpx(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let z = bytes.clone();
    assert_eq!(zip_names(&z).first().map(String::as_str), Some("mimetype"));
    assert_eq!(craft_zip::Archive::open(&z, TEST_ZIP).unwrap().is_stored("mimetype"), Some(true));
    let head = zip_part(&z, "Contents/header.xml");
    assert!(head.contains(r#"height="2000""#) && head.contains("<hh:bold/>") && head.contains(r#"horizontal="CENTER""#), "{head}");
    let (docx, _) = crate::to_docx(&bytes).unwrap();
    let z = docx;
    let body = zip_part(&z, "word/document.xml");
    // (spaces come in runs of their own: Hangul's space width, see convert::Converter::space_rpr)
    let text = crate::convert::w_t_texts(&body);
    assert!(text.contains("첫 문단 &amp; &lt;기호&gt;") && text.contains("둘째 문단"), "{body}");
    assert!(body.contains("<w:b/>") && body.contains(r#"<w:jc w:val="center"/>"#) && body.contains("<w:pageBreakBefore/>"), "{body}");
}

/// Numbered and bulleted lists become HWP paragraph headings, and paragraph styles keep their
/// names; both read back.
#[test]
fn new_hwpx_writes_lists_and_styles() {
    use wordcraft_doc::props::NumRef;
    let mut doc = wordcraft_doc::Document::from_text("제목\n하나\n둘\n점");
    let num = std::sync::Arc::make_mut(&mut doc.numbering).add_list(wordcraft_doc::ListKind::Numbered);
    let bul = std::sync::Arc::make_mut(&mut doc.numbering).add_list(wordcraft_doc::ListKind::Bullet);
    let props = [(Some("Heading1"), None), (None, Some((num, 0))), (None, Some((num, 1))), (None, Some((bul, 0)))];
    for (b, (style, list)) in doc.body.iter_mut().zip(props) {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.style = style.map(String::from);
            p.props.numbering = list.map(|(num, level)| NumRef { num, level });
        }
    }
    let (bytes, report) = crate::new_hwpx(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let z = bytes.clone();
    let head = zip_part(&z, "Contents/header.xml");
    assert!(head.contains(r#"<hh:heading type="NUMBER" idRef="1" level="0"/>"#), "{head}");
    assert!(head.contains(r#"<hh:heading type="NUMBER" idRef="1" level="1"/>"#), "{head}");
    assert!(head.contains(r#"<hh:heading type="BULLET" idRef="1" level="0"/>"#), "{head}");
    assert!(head.contains(r#"numFormat="LATIN_SMALL" charPrIDRef="4294967295" checkable="0">^2.</hh:paraHead>"#), "{head}");
    assert!(head.contains(r#"<hh:bullet id="1" char="•""#), "{head}");
    assert!(head.contains(r#"name="heading 1""#) || head.contains(r#"name="Heading 1""#), "{head}");
    let (docx, _) = crate::to_docx(&bytes).unwrap();
    let z = docx;
    let body = zip_part(&z, "word/document.xml");
    assert_eq!(body.matches("<w:numPr>").count(), 3, "{body}");
}

/// A document that never was an HWP becomes a new HWP 5.0 file: a compound file with the file
/// header, DocInfo and a section, whose text, table and list read back.
#[test]
fn new_hwp_holds_the_document() {
    use wordcraft_doc::props::NumRef;
    let mut doc = wordcraft_doc::Document::from_text("새 HWP 문서\n목록 항목");
    let num = std::sync::Arc::make_mut(&mut doc.numbering).add_list(wordcraft_doc::ListKind::Numbered);
    if let Some(wordcraft_doc::Block::Para(p)) = doc.body.get_mut(1).map(std::sync::Arc::make_mut) {
        p.props.numbering = Some(NumRef { num, level: 0 });
    }
    let mut t = wordcraft_doc::Table::new(2, 2, 200.0);
    if let Some(c) = t.rows.first_mut().and_then(|r| r.cells.first_mut()) {
        c.blocks = wordcraft_doc::Document::from_text("칸").body;
    }
    doc.body.push(std::sync::Arc::new(wordcraft_doc::Block::Table(t)));
    doc.body.push(std::sync::Arc::new(wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::default())));
    let (bytes, report) = crate::new_hwp(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    assert!(sniff(&bytes) && bytes.starts_with(crate::hwp5::OLE_SIG));
    let (docx, _) = crate::to_docx(&bytes).unwrap();
    let z = docx;
    let body = zip_part(&z, "word/document.xml");
    let text = crate::convert::w_t_texts(&body);
    assert!(text.contains("새 HWP 문서") && text.contains("목록 항목") && text.contains("칸"), "{body}");
    assert!(body.contains("<w:tbl>") && body.contains("<w:numPr>"), "{body}");
}

/// A row without a height of its own keeps none in the file: its cells are their margins tall,
/// as in Hancom's files, and Hancom grows the row to fit the text. The estimated text height
/// goes only into the table's size. Written as the cell's height it came back as the row's
/// height on reopening, so every save and reopen made such rows taller.
#[test]
fn table_rows_without_a_height_stay_without_one() {
    let mut doc = wordcraft_doc::Document::from_text("표 앞");
    let mut t = wordcraft_doc::Table::new(1, 1, 200.0);
    if let Some(c) = t.rows.first_mut().and_then(|r| r.cells.first_mut()) {
        c.blocks = wordcraft_doc::Document::from_text(&["여러 줄의 칸 글"; 12].join("\n")).body;
    }
    doc.body.insert(1, std::sync::Arc::new(wordcraft_doc::Block::Table(t)));
    doc.body.push(std::sync::Arc::new(wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::default())));
    let (bytes, _) = crate::new_hwpx(&doc).unwrap();
    let sec = zip_part(&bytes, "Contents/section0.xml");
    let attr = |tag: &str| -> i64 {
        let at = sec.find(tag).unwrap_or_else(|| panic!("no {tag}: {sec}"));
        let rest = &sec[at..];
        let h = &rest[rest.find("height=\"").unwrap() + 8..];
        h[..h.find('"').unwrap()].parse().unwrap()
    };
    let (cell, size) = (attr("<hp:cellSz "), attr("<hp:sz "));
    assert!(cell < 1000, "the cell is its margins tall, not the text's estimate: {cell}");
    assert!(size > 12 * 1000, "the table's size holds the estimated text: {size}");
    let (docx, _) = crate::to_docx(&bytes).unwrap();
    assert!(!zip_part(&docx, "word/document.xml").contains("<w:trHeight"), "no row height on reopening");
}

/// Tables become HWPX tables: size, merged cells, cell borders and fill, and they read back.
#[test]
fn new_hwpx_writes_tables() {
    let mut doc = wordcraft_doc::Document::from_text("표 앞");
    let mut t = wordcraft_doc::Table::new(2, 3, 300.0);
    if let Some(row) = t.rows.first_mut() {
        row.cells.truncate(2);
        if let Some(c) = row.cells.first_mut() {
            c.props.span = 2;
            c.props.shading = Some(wordcraft_doc::Rgb(0xEE, 0xCC, 0xAA));
        }
    }
    doc.body.insert(1, std::sync::Arc::new(wordcraft_doc::Block::Table(t)));
    doc.body.push(std::sync::Arc::new(wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::default())));
    let (bytes, report) = crate::new_hwpx(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let z = bytes.clone();
    let (sec, head) = (zip_part(&z, "Contents/section0.xml"), zip_part(&z, "Contents/header.xml"));
    assert!(sec.contains(r#"rowCnt="2" colCnt="3""#), "{sec}");
    assert!(sec.contains(r#"<hp:cellSpan colSpan="2" rowSpan="1"/>"#), "{sec}");
    assert_eq!(sec.matches("<hp:tc ").count(), 5);
    assert!(head.contains(r##"faceColor="#EECCAA""##), "{head}");
    let (docx, _) = crate::to_docx(&bytes).unwrap();
    let z = docx;
    let body = zip_part(&z, "word/document.xml");
    assert!(body.contains("<w:tbl>") && body.contains(r#"<w:gridSpan w:val="2"/>"#) && body.contains("EECCAA"), "{body}");
}

/// An HWP table cell whose four margins are -1 (0xFFFF; HWPUNIT16 is signed) has no margins of
/// its own: it takes the table's, as HWPX's hasMargin="0" does, instead of 655.35 pt each.
#[test]
fn hwp_cell_margins_of_minus_one_take_the_tables() {
    let u16s = |v: &[u16]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut text = u16s(&[0x0B, 0x6C20, 0x7462, 0, 0, 0, 0, 0x0B, 13]);
    let mut head = [0u8; 24];
    head[0..4].copy_from_slice(&(0x8000_0009u32).to_le_bytes());
    head[12..14].copy_from_slice(&1u16.to_le_bytes());
    let mut s = record(0x42, 0, &head);
    s.extend(record(0x43, 1, &text));
    s.extend(record(0x44, 1, &[0u8; 8]));
    let mut ctrl = 0x7462_6C20u32.to_le_bytes().to_vec();
    ctrl.extend([0u8; 12]);
    ctrl.extend(20000u32.to_le_bytes()); // width
    ctrl.extend(2000u32.to_le_bytes()); // height
    ctrl.extend([0u8; 22]);
    s.extend(record(0x47, 1, &ctrl)); // CTRL_HEADER
    let mut tbl = 0u32.to_le_bytes().to_vec();
    tbl.extend(u16s(&[1, 1, 0, 141, 141, 141, 141, 1, 1, 0]));
    s.extend(record(0x4D, 2, &tbl)); // TABLE
    let mut cell = u16s(&[1, 0]);
    cell.extend(0u32.to_le_bytes());
    cell.extend(u16s(&[0, 0, 1, 1]));
    cell.extend(20000u32.to_le_bytes());
    cell.extend(2000u32.to_le_bytes());
    cell.extend(u16s(&[0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 1]));
    cell.extend([0u8; 8]);
    s.extend(record(0x48, 2, &cell)); // LIST_HEADER
    text = u16s(&[0xCE78, 13]); // 칸
    let mut head = [0u8; 24];
    head[0..4].copy_from_slice(&(0x8000_0002u32).to_le_bytes());
    head[12..14].copy_from_slice(&1u16.to_le_bytes());
    s.extend(record(0x42, 2, &head));
    s.extend(record(0x43, 3, &text));
    s.extend(record(0x44, 3, &[0u8; 8]));
    let (docx, _) = crate::to_docx(&hwp_with(&s)).unwrap();
    let z = docx;
    let body = zip_part(&z, "word/document.xml");
    assert!(body.contains("<w:tbl>") && crate::convert::w_t_texts(&body).contains('칸'), "{body}");
    assert!(!body.contains("13107"), "655.35 pt cell margins: {body}");
}

/// A continued cell under a cell that doesn't start a merge (a row inserted inside a merge)
/// joins that cell instead of leaving a hole in the table.
#[test]
fn new_hwpx_joins_a_continued_cell_to_the_one_above() {
    let mut doc = wordcraft_doc::Document::from_text("표");
    let mut t = wordcraft_doc::Table::new(3, 2, 200.0);
    if let Some(c) = t.rows.first_mut().and_then(|r| r.cells.first_mut()) {
        c.props.vmerge = wordcraft_doc::props::VMerge::Restart;
    }
    if let Some(c) = t.rows.get_mut(2).and_then(|r| r.cells.first_mut()) {
        c.props.vmerge = wordcraft_doc::props::VMerge::Continue;
    }
    doc.body.insert(1, std::sync::Arc::new(wordcraft_doc::Block::Table(t)));
    doc.body.push(std::sync::Arc::new(wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::default())));
    let (bytes, _) = crate::new_hwpx(&doc).unwrap();
    let z = bytes;
    let sec = zip_part(&z, "Contents/section0.xml");
    assert_eq!(sec.matches("<hp:tc ").count(), 5, "{sec}");
    assert!(sec.contains(r#"<hp:cellAddr colAddr="0" rowAddr="1"/><hp:cellSpan colSpan="1" rowSpan="2"/>"#), "{sec}");
}

/// Page margins at the edge of the number range convert without overflowing their sums (top and
/// header, bottom and footer), across and down the page. Before, a debug build panicked and a
/// release build wrote negative margins.
#[test]
fn huge_page_margins_do_not_overflow() {
    for dir in ["HORIZONTAL", "VERTICAL"] {
        let big = i64::MAX;
        let sec = format!(
            r#"<hs:sec {NS}><p:p><p:run><p:secPr textDirection="{dir}"><p:pagePr width="59528" height="84188"><p:margin left="{big}" right="{big}" top="{big}" bottom="{big}" header="{big}" footer="{big}" gutter="{big}"/></p:pagePr></p:secPr><p:t>가</p:t></p:run></p:p></hs:sec>"#
        );
        let (docx, _) = crate::to_docx(&hwpx(&sec)).unwrap();
        let body = zip_part(&docx, "word/document.xml");
        let mar = &body[body.find("<w:pgMar").unwrap()..];
        assert!(!mar[..mar.find("/>").unwrap()].contains('-'), "{dir}: no wrapped margin: {mar}");
    }
}

/// Shape ids are remapped in tags only: a cell whose text spells an id reference keeps its text.
/// Before, the text `charPrIDRef="0"` became the header's new id.
#[test]
fn remapped_table_ids_leave_cell_text_alone() {
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let xml = r#"<hp:tc borderFillIDRef="1"><hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>charPrIDRef="0" paraPrIDRef="0"</hp:t></hp:run></hp:p></hp:tc>"#;
    let out = crate::restyle::remap_ids(xml, &ids(&["4"]), &ids(&["7"]), &ids(&["9"]));
    assert_eq!(
        out,
        r#"<hp:tc borderFillIDRef="9"><hp:p paraPrIDRef="7"><hp:run charPrIDRef="4"><hp:t>charPrIDRef="0" paraPrIDRef="0"</hp:t></hp:run></hp:p></hp:tc>"#
    );
}

/// A drawing inline in the story of a text box that is a group member is placed in the story's
/// text, with its own size and position, not as another member of the group. Before, it took
/// the box's member state and lost them.
#[test]
fn new_hwpx_text_box_story_in_a_group_places_its_own_drawings() {
    use wordcraft_doc::{CharProps, InlineObject, Paragraph, PartKind, para::ShapeKind, para_block};
    let shape = |kind: ShapeKind, story: Option<u32>| InlineObject::Shape {
        kind,
        w: 20.0,
        h: 10.0,
        fill: None,
        stroke: Some(wordcraft_doc::props::Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float: Default::default(),
        story,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let mut doc = wordcraft_doc::Document::from_text("");
    let mut inner = Paragraph::with_text("상자", CharProps::default());
    inner.insert_object(0, shape(ShapeKind::Rectangle, None), &CharProps::default()).unwrap();
    let id = doc.add_part(PartKind::TextBox, vec![para_block(inner)]);
    // shown at twice its own coordinates
    let group = InlineObject::Group {
        w: 100.0,
        h: 100.0,
        float: Default::default(),
        ch_w: 50.0,
        ch_h: 50.0,
        children: vec![wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: shape(ShapeKind::TextBox, Some(id)) }],
        canvas: None,
    };
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, group, &CharProps::default()).unwrap();
    doc.body.clear();
    doc.body.push(para_block(p));
    let (bytes, _) = crate::new_hwpx(&doc).unwrap();
    let sec = zip_part(&bytes, "Contents/section0.xml");
    let text = &sec[sec.find("<hp:drawText").unwrap()..sec.find("</hp:drawText>").unwrap()];
    assert!(text.contains("<hp:sz ") && text.contains("<hp:pos "), "the story's drawing has its own size and place: {text}");
}

/// Text typed into a source paragraph written as an empty tag (`<p:p/>`) goes inside it: the tag
/// is opened. Before, the new run went after the tag, outside the paragraph, and reopening the
/// file showed the paragraph empty.
#[test]
fn write_back_opens_an_empty_paragraph_tag() {
    let original = hwpx(&format!(r#"<hs:sec {NS}><p:p><p:run><p:t>첫째</p:t></p:run></p:p><p:p/></hs:sec>"#));
    let (docx, _) = crate::to_docx(&original).unwrap();
    let body = zip_part(&docx, "word/document.xml");
    // the empty paragraph is the second one; type into it
    let second = body.match_indices("<w:p>").chain(body.match_indices("<w:p ")).map(|(i, _)| i).max().unwrap();
    let end = second + body[second..].find("</w:p>").unwrap();
    let edited = rezip(&docx, |xml| format!("{}<w:r><w:t>새 글</w:t></w:r>{}", &xml[..end], &xml[end..]));
    let (out, report) = crate::write_back(&original, &edited).unwrap();
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    let root = crate::xml::parse(sec.as_bytes(), false).unwrap();
    let paras: Vec<_> = root.children.iter().filter(|c| c.tag.ends_with(":p")).collect();
    assert_eq!(paras.len(), 2, "{sec}");
    assert!(paras[1].iter("hp:t").any(|t| t.text.contains("새 글")), "the text is in the paragraph: {sec}");
}

/// In a section whose paragraph namespace is the default one, editing the text of a run that
/// holds settings before its text closes that run with its own name (`</run>`). Before, it was
/// closed as `</hp:run>`, which doesn't match `<run>`.
#[test]
fn write_back_closes_an_unprefixed_run_by_its_name() {
    let sec = r#"<hs:sec xmlns="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><p><run><secPr/><t>원래 글</t></run></p></hs:sec>"#;
    let original = hwpx(sec);
    let (docx, _) = crate::to_docx(&original).unwrap();
    let edited = rezip(&docx, |xml| xml.replacen("원래 글", "바뀐 글", 1));
    let (out, report) = crate::write_back(&original, &edited).unwrap();
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert!(!sec.contains("</hp:run>"), "{sec}");
    let (again, _) = crate::to_docx(&out).unwrap();
    assert!(zip_part(&again, "word/document.xml").contains("바뀐 글"), "{sec}");
}

/// A hyperlink in a document saved as a new .hwpx keeps its text, and the save reports that its
/// destination is not written. Before, the destination went without a notice.
#[test]
fn new_hwpx_reports_hyperlink_destinations() {
    use wordcraft_doc::{CharProps, Paragraph, para_block};
    let linked = CharProps { link: Some("https://example.com/".into()), ..CharProps::default() };
    let mut doc = wordcraft_doc::Document::from_text("");
    doc.body.clear();
    doc.body.push(para_block(Paragraph::with_text("연결된 글", linked)));
    let (bytes, report) = crate::new_hwpx(&doc).unwrap();
    assert!(zip_part(&bytes, "Contents/section0.xml").contains("연결된 글"));
    assert!(report.skipped.iter().any(|s| s.contains("hyperlink destinations")), "{report:?}");
}

/// A page background tiled with a picture too small to lay tile by tile (1 x 1 pixel over an A4
/// page) is one box with a tiling fill that covers the page, and the conversion says so.
/// Before, the tiles stopped partway down the page without a notice.
#[test]
fn a_tiny_tiled_page_background_covers_the_page() {
    let header = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:borderFills itemCnt="1"><hh:borderFill id="1"><hc:fillBrush><hc:imgBrush mode="TILE"><hc:img binaryItemIDRef="image1"/></hc:imgBrush></hc:fillBrush></hh:borderFill></hh:borderFills></hh:refList></hh:head>"#;
    let sec = format!(
        r#"<hs:sec {NS}><p:p><p:run><p:secPr><p:pagePr width="59528" height="84188"/><p:pageBorderFill type="BOTH" borderFillIDRef="1" fillArea="PAPER"/></p:secPr><p:t>가</p:t></p:run></p:p></hs:sec>"#
    );
    // a PNG header: signature, then IHDR with a width and height of 1
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(1u32.to_be_bytes());
    png.extend(1u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
    z.add("Contents/header.xml", header.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    z.add("Contents/section0.xml", sec.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    z.add("BinData/image1.png", &png, craft_zip::Compression::Stored).unwrap();
    let (docx, report) = crate::to_docx(&z.finish().unwrap()).unwrap();
    let all: String = zip_names(&docx).into_iter().filter(|n| n.starts_with("word/header")).map(|n| zip_part(&docx, &n)).collect();
    assert_eq!(all.matches("Page background").count(), 1, "one box");
    assert!(all.contains("<a:tile "), "a tiling fill");
    assert!(report.unsupported.keys().any(|k| k.contains("page background tiles")), "{:?}", report.unsupported);
}

/// Two tables set like characters in a section's first paragraph, with text: on a page too narrow
/// for both side by side they go one under the other, as DOCX tables. The section's page size,
/// in that same paragraph, is read before the tables are measured. Before, they were measured
/// against an A4 page, fit there, and became inline boxes overflowing the narrow page.
#[test]
fn inline_tables_are_measured_against_their_sections_page() {
    let table = |t: &str| {
        format!(
            r#"<p:tbl rowCnt="1" colCnt="1"><p:sz width="15000" height="1000"/><p:pos treatAsChar="1"/><p:tr><p:tc><p:subList><p:p><p:run><p:t>{t}</p:t></p:run></p:p></p:subList><p:cellAddr colAddr="0" rowAddr="0"/><p:cellSpan colSpan="1" rowSpan="1"/><p:cellSz width="15000" height="1000"/></p:tc></p:tr></p:tbl>"#
        )
    };
    let sec = format!(
        r#"<hs:sec {NS}><p:p><p:run><p:secPr><p:pagePr width="24000" height="34000"><p:margin left="2000" right="2000" top="2000" bottom="2000" header="0" footer="0" gutter="0"/></p:pagePr></p:secPr><p:t>표 둘</p:t>{}{}</p:run></p:p></hs:sec>"#,
        table("왼쪽"),
        table("오른쪽")
    );
    let (docx, _) = crate::to_docx(&hwpx(&sec)).unwrap();
    let body = zip_part(&docx, "word/document.xml");
    assert!(body.contains("왼쪽") && body.contains("오른쪽"));
    assert!(!body.contains("txbxContent"), "not inline boxes on the narrow page: {body}");
}

/// A section that continues an earlier section's numbered footer and hides the page number on
/// its first page shows that footer's text there without the number. Before, the first page
/// showed the inherited footer as it was, number included.
#[test]
fn a_hidden_first_page_number_is_taken_from_an_inherited_footer() {
    let first = format!(
        r#"<hs:sec {NS}><p:p><p:run><p:secPr/><p:ctrl><p:footer applyPageType="BOTH"><p:subList><p:p><p:run><p:t>꼬리말 글</p:t></p:run></p:p></p:subList></p:footer></p:ctrl><p:ctrl><p:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></p:ctrl><p:t>첫 구역</p:t></p:run></p:p></hs:sec>"#
    );
    let second = format!(
        r#"<hs:sec {NS}><p:p><p:run><p:secPr/><p:ctrl><p:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></p:ctrl><p:ctrl><p:pageHiding hideHeader="0" hideFooter="0" hidePageNum="1"/></p:ctrl><p:t>둘째 구역</p:t></p:run></p:p></hs:sec>"#
    );
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored).unwrap();
    z.add("Contents/header.xml", br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, craft_zip::Compression::Deflated).unwrap();
    z.add("Contents/section0.xml", first.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    z.add("Contents/section1.xml", second.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    let (docx, _) = crate::to_docx(&z.finish().unwrap()).unwrap();
    let footers: Vec<String> = zip_names(&docx).into_iter().filter(|n| n.starts_with("word/footer")).map(|n| zip_part(&docx, &n)).collect();
    assert!(footers.iter().any(|f| f.contains("꼬리말 글") && !f.contains("PAGE")), "a first-page footer without the number: {footers:?}");
}
