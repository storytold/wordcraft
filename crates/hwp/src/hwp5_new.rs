//! A new HWP 5.0 file from a WordCraft document (one not opened from an HWP, so there is no
//! original to write edits into; see [`crate::write_back_hwp`] for those).
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
//!
//! The document is first written as a new HWPX ([`crate::hwpx_out`]), whose header and sections
//! hold everything this writer keeps; that XML becomes records: the header DocInfo (fonts,
//! border fills, character and paragraph shapes, tabs, styles, made with the shape encoders of
//! [`crate::hwp5_docinfo`] from empty records), each section a BodyText stream (paragraphs,
//! tables, section and column definitions, with [`crate::hwp5_enc`]). The streams are
//! compressed and put in a compound file with the file header. What the HWPX writer leaves out
//! is reported the same way; objects this writer cannot write yet are reported too.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use wordcraft_doc::Document;

use crate::error::{Error, Result};
use crate::header::LANGS;
use crate::hwp5::{BORDER_FILL, CHAR_SHAPE, FACE_NAME, ID_MAPPINGS, NUM_FORMATS, PARA_SHAPE};
use crate::hwp5_docinfo::{border_fill, face_name, patch_char, patch_para};
use crate::hwp5_enc::{DEFAULT_TAB, Encoder, Rec, TableModel, encode, num, put, set_last_in_list};
use crate::writer::SaveReport;
use crate::xml::Element;

const DOCUMENT_PROPERTIES: u32 = 0x10;
const BIN_DATA: u32 = 0x10 + 2;
const TAB_DEF: u32 = 0x10 + 6;
const NUMBERING: u32 = 0x10 + 7;
const BULLET: u32 = 0x10 + 8;
const STYLE: u32 = 0x10 + 10;
/// The version written: 5.0.4.0, as Hancom's files of 2014 and later carry.
const VERSION: [u8; 4] = [0, 4, 0, 5];

/// A length-prefixed UTF-16 string, as HWP records hold names.
fn wstr(s: &str) -> Vec<u8> {
    let w: Vec<u16> = s.encode_utf16().take(u16::MAX as usize).collect();
    let mut out = (w.len() as u16).to_le_bytes().to_vec();
    out.extend(w.iter().flat_map(|c| c.to_le_bytes()));
    out
}

/// The `tag` children of the list `list` in `head`, in id order.
fn items<'a>(head: &'a Element, list: &'a str, tag: &str) -> Vec<&'a Element> {
    let mut v: Vec<&Element> = head.iter(list).flat_map(|l| l.children.iter()).filter(|e| e.tag == tag).collect();
    v.sort_by_key(|e| num(e, "id"));
    v
}

/// A character shape record before its values are set: 10 pt, full width and size, no shade.
fn char_base() -> Vec<u8> {
    let mut d = vec![0u8; 74];
    d[14..21].fill(100);
    d[28..35].fill(100);
    put(&mut d, 42, &1000i32.to_le_bytes());
    put(&mut d, 60, &u32::MAX.to_le_bytes());
    put(&mut d, 64, &0x00B2_B2B2u32.to_le_bytes()); // shadow colour
    d
}

/// A paragraph head (a list level's number or bullet settings) as the 12 bytes the records hold:
/// attributes (alignment, instance width, auto indent, offset kind, number format), width
/// adjustment, offset from the text, character shape (none: the paragraph's).
fn para_head(ph: Option<&Element>) -> Vec<u8> {
    let a = |n: &str| ph.and_then(|p| p.get(n)).unwrap_or("");
    let align = ["LEFT", "CENTER", "RIGHT"].iter().position(|x| *x == a("align")).unwrap_or(0) as u32;
    let format = NUM_FORMATS.iter().position(|x| *x == a("numFormat")).unwrap_or(0) as u32;
    let attr = align
        | (u32::from(a("useInstWidth") == "1") << 2)
        | (u32::from(a("autoIndent") == "1") << 3)
        | (u32::from(a("textOffsetType") == "HWPUNIT") << 4)
        | ((format & 0xF) << 5);
    let mut d = attr.to_le_bytes().to_vec();
    d.extend_from_slice(&(ph.map(|p| num(p, "widthAdjust")).unwrap_or(0) as u16).to_le_bytes());
    d.extend_from_slice(&(ph.map(|p| num(p, "textOffset")).unwrap_or(50) as u16).to_le_bytes());
    let cpr = ph.and_then(|p| p.get("charPrIDRef")).and_then(|v| v.parse::<u32>().ok()).unwrap_or(u32::MAX);
    d.extend_from_slice(&cpr.to_le_bytes());
    d
}

/// A numbering (seven levels: head, number text with ^n; then the starts) as its record.
fn numbering(n: &Element) -> Vec<u8> {
    let heads: Vec<&Element> = n.children.iter().filter(|c| c.tag == "hh:paraHead").collect();
    let level = |k: i64| heads.iter().copied().find(|h| num(h, "level") == k);
    let mut d = Vec::new();
    for k in 1..=7 {
        let ph = level(k);
        d.extend(para_head(ph));
        d.extend(wstr(ph.map(|p| p.text.as_str()).unwrap_or("")));
    }
    d.extend_from_slice(&(num(n, "start").clamp(0, i64::from(u16::MAX)) as u16).to_le_bytes());
    for k in 1..=7 {
        let st = level(k).map(|p| num(p, "start")).unwrap_or(1).max(0) as u32;
        d.extend_from_slice(&st.to_le_bytes());
    }
    d
}

/// A bullet (head, character, no image, no check character) as its record.
fn bullet(b: &Element) -> Vec<u8> {
    let mut d = para_head(b.children.iter().find(|c| c.tag == "hh:paraHead"));
    let ch = b.get("char").and_then(|c| c.encode_utf16().next()).unwrap_or(0x2022);
    d.extend_from_slice(&ch.to_le_bytes());
    d.extend_from_slice(&[0u8; 9]);
    d
}

/// The DocInfo stream for the header `head`, `sections` sections and the pictures `bins` (file
/// extensions, BinData id order).
fn doc_info(head: &Element, sections: usize, bins: &[String]) -> Vec<Rec> {
    let mut counts = [0i32; 18];
    let mut recs: Vec<Rec> = Vec::new();
    // document properties: sections, numbers start at 1, the caret at the start
    let mut props = vec![0u8; 26];
    put(&mut props, 0, &(sections.min(u16::MAX as usize) as u16).to_le_bytes());
    let begin = head.find("hh:beginNum");
    for (k, a) in ["page", "footnote", "endnote", "pic", "tbl", "equation"].iter().enumerate() {
        let v = begin.map(|b| num(b, a)).filter(|v| *v > 0).unwrap_or(1);
        put(&mut props, 2 + 2 * k, &(v.min(i64::from(u16::MAX)) as u16).to_le_bytes());
    }
    recs.push((DOCUMENT_PROPERTIES, 0, props));
    let mut body: Vec<Rec> = Vec::new();
    // pictures: embedded (1), compressed as the file is, BinData id, extension
    for (i, ext) in bins.iter().enumerate() {
        let mut d = 1u16.to_le_bytes().to_vec();
        d.extend_from_slice(&((i + 1).min(u16::MAX as usize) as u16).to_le_bytes());
        d.extend(wstr(ext));
        body.push((BIN_DATA, 1, d));
    }
    counts[0] = bins.len() as i32;
    for (k, lang) in LANGS.iter().enumerate() {
        let ff = head.iter("hh:fontface").find(|f| f.get("lang").is_some_and(|l| l.eq_ignore_ascii_case(lang)));
        let mut fonts: Vec<&Element> = ff.map(|f| f.children.iter().filter(|c| c.tag == "hh:font").collect()).unwrap_or_default();
        fonts.sort_by_key(|f| num(f, "id"));
        if fonts.is_empty() {
            body.push((FACE_NAME, 1, face_name("함초롬바탕")));
            counts[1 + k] = 1;
            continue;
        }
        for f in &fonts {
            body.push((FACE_NAME, 1, face_name(f.get("face").unwrap_or(""))));
        }
        counts[1 + k] = fonts.len() as i32;
    }
    let blank = Element::new("");
    let fills = items(head, "hh:borderFills", "hh:borderFill");
    for f in &fills {
        body.push((BORDER_FILL, 1, border_fill(&[], f)));
    }
    counts[8] = fills.len() as i32;
    let chars = items(head, "hh:charProperties", "hh:charPr");
    for c in &chars {
        let mut d = char_base();
        patch_char(&mut d, &blank, c);
        body.push((CHAR_SHAPE, 1, d));
    }
    counts[9] = chars.len() as i32;
    let tabs = items(head, "hh:tabProperties", "hh:tabPr");
    for t in &tabs {
        let a = u32::from(t.get("autoTabLeft") == Some("1")) | (u32::from(t.get("autoTabRight") == Some("1")) << 1);
        let mut d = a.to_le_bytes().to_vec();
        d.extend_from_slice(&0i32.to_le_bytes());
        body.push((TAB_DEF, 1, d));
    }
    counts[10] = tabs.len() as i32;
    let nums = items(head, "hh:numberings", "hh:numbering");
    for n in &nums {
        body.push((NUMBERING, 1, numbering(n)));
    }
    counts[11] = nums.len() as i32;
    let bullets = items(head, "hh:bullets", "hh:bullet");
    for b in &bullets {
        body.push((BULLET, 1, bullet(b)));
    }
    counts[12] = bullets.len() as i32;
    let paras = items(head, "hh:paraProperties", "hh:paraPr");
    for p in &paras {
        let mut d = vec![0u8; 54];
        patch_para(&mut d, &blank, p);
        // the old line spacing field holds the same value
        let v: [u8; 4] = d.get(50..54).and_then(|b| b.try_into().ok()).unwrap_or([0; 4]);
        put(&mut d, 24, &v);
        body.push((PARA_SHAPE, 1, d));
    }
    counts[13] = paras.len() as i32;
    let styles = items(head, "hh:styles", "hh:style");
    for s in &styles {
        let mut d = wstr(s.get("name").unwrap_or(""));
        d.extend(wstr(s.get("engName").unwrap_or("")));
        d.push(u8::from(s.get("type") == Some("CHAR")));
        d.push(num(s, "nextStyleIDRef").clamp(0, 255) as u8);
        d.extend_from_slice(&1042i16.to_le_bytes());
        d.extend_from_slice(&(num(s, "paraPrIDRef").clamp(0, i64::from(u16::MAX)) as u16).to_le_bytes());
        d.extend_from_slice(&(num(s, "charPrIDRef").clamp(0, i64::from(u16::MAX)) as u16).to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        body.push((STYLE, 1, d));
    }
    counts[14] = styles.len() as i32;
    recs.push((ID_MAPPINGS, 0, counts.iter().flat_map(|c| c.to_le_bytes()).collect()));
    recs.extend(body);
    recs
}

/// A paragraph header new paragraphs start from: no text yet, one character shape.
fn para_header() -> Vec<u8> {
    vec![0u8; 24]
}

/// A new HWP 5.0 file holding `doc`, and what it could not hold yet.
pub fn from_document(doc: &Document) -> Result<(Vec<u8>, SaveReport)> {
    let (hwpx, mut report) = crate::hwpx_out::package(doc, true)?;
    let pkg = crate::package::Package::open(&hwpx)?;
    let head = pkg.xml(pkg.header_name.as_deref().ok_or_else(|| Error::format("no header"))?)?;
    // the package's pictures become BinData streams BIN0001.<ext> ...
    let pictures: Vec<&crate::package::Item> = pkg.items.iter().filter(|i| i.href.starts_with("BinData/")).collect();
    let exts: Vec<String> =
        pictures.iter().map(|i| i.href.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_else(|| "png".into())).collect();
    let bins = pictures.iter().enumerate().map(|(k, i)| (i.id.clone(), (k + 1).min(u16::MAX as usize) as u16)).collect();
    let mut enc = Encoder {
        tab: DEFAULT_TAB.to_vec(),
        model: TableModel::default(),
        para_model: para_header(),
        left_out: Default::default(),
        bins,
        picture_keys: Default::default(),
        strict: false,
        memos: Default::default(),
        memo_fields: Default::default(),
        merged_shapes: Default::default(),
    };
    let mut sections: Vec<Vec<u8>> = Vec::new();
    for name in &pkg.section_names {
        let sec = pkg.xml(name)?;
        enc.memo_fields = crate::hwp5_enc::memo_fields(&sec);
        let mut recs: Vec<Rec> = Vec::new();
        let mut left_out = 0;
        for p in sec.children.iter().filter(|c| c.tag == "hp:p") {
            match enc.paragraph(p, &para_header(), 0, 0) {
                Some(r) => recs.extend(r),
                None => {
                    // keep the text, leave the objects out
                    let mut plain = p.clone();
                    for run in plain.children.iter_mut().filter(|c| c.tag == "hp:run") {
                        run.children.retain(|c| c.tag == "hp:t");
                    }
                    recs.extend(enc.paragraph(&plain, &para_header(), 0, 0).unwrap_or_default());
                    left_out += 1;
                }
            }
        }
        if left_out > 0 {
            report.skipped.push(format!("objects in {left_out} paragraph(s) are not written to .hwp yet"));
        }
        let objects = enc.left_out.replace(0);
        if objects > 0 {
            report.skipped.push(format!("{objects} picture(s) or shape(s) are not written to .hwp yet"));
        }
        let merged = enc.merged_shapes.replace(0);
        if merged > 0 {
            report
                .skipped
                .push(format!("{merged} paragraph(s) change character shape more than 65,535 times; the rest of each keeps the last shape"));
        }
        // the section's memos hang under its last paragraph
        recs.append(&mut enc.memos.borrow_mut());
        set_last_in_list(&mut recs);
        sections.push(encode(&recs));
    }
    let info = encode(&doc_info(&head, sections.len(), &exts));
    let mut header = b"HWP Document File".to_vec();
    header.resize(256, 0);
    put(&mut header, 32, &VERSION);
    put(&mut header, 36, &1u32.to_le_bytes()); // compressed
    let mut streams: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    streams.insert("/FileHeader".into(), header);
    streams.insert("/DocInfo".into(), miniz_oxide::deflate::compress_to_vec(&info, 6));
    for (i, s) in sections.iter().enumerate() {
        streams.insert(format!("/BodyText/Section{i}"), miniz_oxide::deflate::compress_to_vec(s, 6));
    }
    for (k, (item, ext)) in pictures.iter().zip(&exts).enumerate() {
        if let Some(data) = pkg.read(&item.href) {
            streams.insert(format!("/BinData/BIN{:04X}.{ext}", k + 1), miniz_oxide::deflate::compress_to_vec(data, 6));
        }
    }
    let failed = |e: std::io::Error| Error::format(format!("cannot write the compound file ({e})"));
    let mut c = cfb::CompoundFile::create(Cursor::new(Vec::new())).map_err(failed)?;
    c.create_storage("/BodyText").map_err(failed)?;
    if !pictures.is_empty() {
        c.create_storage("/BinData").map_err(failed)?;
    }
    for (name, data) in &streams {
        c.create_stream(name).map_err(failed)?.write_all(data).map_err(failed)?;
    }
    c.flush().map_err(failed)?;
    Ok((c.into_inner().into_inner(), report))
}
