//! HWP 5.0 (binary) reader.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
//! (Hancom's "한글 문서 파일 형식 5.0", Hangul document file format 5.0, revision 1.3; the compound-file container through the `cfb`
//! crate, [MS-CFB].)
//!
//! The records of an HWP 5.0 file are rebuilt as the same element tree the HWPX reader produces
//! ("hh:head" / "hs:sec" with "hp:p", "hp:run", ...), so the converter handles both formats with
//! one code path. Where the specification and real files disagree, the comment says what the
//! files showed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read};

use crate::error::{Error, Result};
use crate::header::{FONT_LANGS, LANGS};
use crate::package::{Item, Package};
use crate::units::round_even;
use crate::xml::{Element, ORDINAL};

pub(crate) const OLE_SIG: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const HEADER_NAME: &str = "Contents/header.xml";
/// One stream, compressed or not.
const MAX_STREAM: u64 = 256 * 1024 * 1024;
/// All streams read, as stored and once uncompressed (each counts against this).
const MAX_TOTAL: usize = 1024 * 1024 * 1024;
const MAX_STREAMS: usize = 65_536;
/// Most records in DocInfo and all sections together. A record takes 4 bytes of a stream but
/// about 50 in memory, so a large stream of empty records alone could take gigabytes. The
/// largest test document (145 pages) has under 20,000.
pub(crate) const MAX_RECORDS: usize = 2_000_000;
/// Deepest nesting of paragraphs in tables, text boxes, notes and groups followed.
const MAX_DEPTH: usize = 64;

const TAG_BEGIN: u32 = 0x10;
// DocInfo
const DOCUMENT_PROPERTIES: u32 = TAG_BEGIN;
pub(crate) const ID_MAPPINGS: u32 = TAG_BEGIN + 1;
pub(crate) const BIN_DATA: u32 = TAG_BEGIN + 2;
pub(crate) const FACE_NAME: u32 = TAG_BEGIN + 3;
pub(crate) const BORDER_FILL: u32 = TAG_BEGIN + 4;
pub(crate) const CHAR_SHAPE: u32 = TAG_BEGIN + 5;
const TAB_DEF: u32 = TAG_BEGIN + 6;
const NUMBERING: u32 = TAG_BEGIN + 7;
const BULLET: u32 = TAG_BEGIN + 8;
pub(crate) const PARA_SHAPE: u32 = TAG_BEGIN + 9;
const STYLE: u32 = TAG_BEGIN + 10;
// BodyText
pub(crate) const PARA_HEADER: u32 = TAG_BEGIN + 50;
pub(crate) const PARA_TEXT: u32 = TAG_BEGIN + 51;
pub(crate) const PARA_CHAR_SHAPE: u32 = TAG_BEGIN + 52;
pub(crate) const PARA_LINE_SEG: u32 = TAG_BEGIN + 53;
pub(crate) const PARA_RANGE_TAG: u32 = TAG_BEGIN + 54;
pub(crate) const CTRL_HEADER: u32 = TAG_BEGIN + 55;
pub(crate) const LIST_HEADER: u32 = TAG_BEGIN + 56;
const PAGE_DEF: u32 = TAG_BEGIN + 57;
const PAGE_BORDER_FILL: u32 = TAG_BEGIN + 59;
pub(crate) const SHAPE_COMPONENT: u32 = TAG_BEGIN + 60;
pub(crate) const TABLE: u32 = TAG_BEGIN + 61;
const SC_LINE: u32 = TAG_BEGIN + 62;
const SC_RECTANGLE: u32 = TAG_BEGIN + 63;
const SC_POLYGON: u32 = TAG_BEGIN + 66;
const SC_CURVE: u32 = TAG_BEGIN + 67;
pub(crate) const SC_PICTURE: u32 = TAG_BEGIN + 69;
const CTRL_DATA: u32 = TAG_BEGIN + 71;
const EQEDIT: u32 = TAG_BEGIN + 72;
pub(crate) const MEMO_LIST: u32 = TAG_BEGIN + 77;

/// Line types. Real files use 0 for "no line" and shift the specification's table (where 0 is
/// solid) up by one: border fills without lines store 0, solid lines 1.
pub(crate) const LINE_TYPES: &[&str] = &[
    "NONE",
    "SOLID",
    "DASH",
    "DOT",
    "DASH_DOT",
    "DASH_DOT_DOT",
    "LONG_DASH",
    "CIRCLE",
    "DOUBLE_SLIM",
    "SLIM_THICK",
    "THICK_SLIM",
    "SLIM_THICK_SLIM",
    "WAVE",
    "DOUBLEWAVE",
    "THICK3D",
    "THICKREV3D",
    "3D",
    "REV3D",
];
pub(crate) const LINE_WIDTHS: &[&str] = &[
    "0.1 mm", "0.12 mm", "0.15 mm", "0.2 mm", "0.25 mm", "0.3 mm", "0.4 mm", "0.5 mm", "0.6 mm", "0.7 mm", "1.0 mm", "1.5 mm", "2.0 mm", "3.0 mm",
    "4.0 mm", "5.0 mm",
];
pub(crate) const ALIGNS: &[&str] = &["JUSTIFY", "LEFT", "RIGHT", "CENTER", "DISTRIBUTE", "DISTRIBUTE_SPACE"];
pub(crate) const LINE_SPACING: &[&str] = &["PERCENT", "FIXED", "BETWEEN_LINES", "AT_LEAST"];
pub(crate) const HEADINGS: &[&str] = &["NONE", "OUTLINE", "NUMBER", "BULLET"];
pub(crate) const NUM_FORMATS: &[&str] = &[
    "DIGIT",
    "CIRCLED_DIGIT",
    "ROMAN_CAPITAL",
    "ROMAN_SMALL",
    "LATIN_CAPITAL",
    "LATIN_SMALL",
    "CIRCLED_LATIN_CAPITAL",
    "CIRCLED_LATIN_SMALL",
    "HANGUL_SYLLABLE",
    "CIRCLED_HANGUL_SYLLABLE",
    "HANGUL_JAMO",
    "CIRCLED_HANGUL_JAMO",
    "HANGUL_PHONETIC",
    "IDEOGRAPH",
    "CIRCLED_IDEOGRAPH",
];
const TAB_TYPES: &[&str] = &["LEFT", "RIGHT", "CENTER", "DECIMAL"];
const AUTO_NUM_TYPES: &[&str] = &["PAGE", "FOOTNOTE", "ENDNOTE", "PICTURE", "TABLE", "EQUATION"];
const PAGE_NUM_POS: &[&str] = &[
    "NONE",
    "TOP_LEFT",
    "TOP_CENTER",
    "TOP_RIGHT",
    "BOTTOM_LEFT",
    "BOTTOM_CENTER",
    "BOTTOM_RIGHT",
    "OUTSIDE_TOP",
    "OUTSIDE_BOTTOM",
    "INSIDE_TOP",
    "INSIDE_BOTTOM",
];
const VALIGNS: &[&str] = &["TOP", "CENTER", "BOTTOM"];
pub(crate) const ARROWS: &[&str] =
    &["NORMAL", "ARROW", "CONCAVE_ARROW", "EMPTY_DIAMOND", "EMPTY_CIRCLE", "EMPTY_BOX", "FILLED_DIAMOND", "FILLED_CIRCLE", "FILLED_BOX"];

/// Character controls (one WCHAR); the rest take eight (table 6).
pub(crate) const CHAR_CONTROLS: &[u16] = &[0, 10, 13, 24, 25, 26, 27, 28, 29, 30, 31];
pub(crate) const INLINE_CONTROLS: &[u16] = &[4, 5, 6, 7, 8, 9, 19, 20];

fn shape_tag(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "$rec" => "hp:rect",
        "$ell" => "hp:ellipse",
        "$arc" => "hp:arc",
        "$pol" => "hp:polygon",
        "$cur" => "hp:curve",
        "$lin" => "hp:line",
        "$pic" => "hp:pic",
        "$con" => "hp:container",
        "$ole" => "hp:ole",
        _ => return None,
    })
}

fn form_tag(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "RadioButton" => "hp:radioBtn",
        "CheckBox" => "hp:checkBtn",
        "PushButton" => "hp:btn",
        "Edit" => "hp:edit",
        "ComboBox" => "hp:comboBox",
        "ListBox" => "hp:listBox",
        _ => return None,
    })
}

fn field_kind(cid: &str) -> &'static str {
    match cid {
        "%hlk" => "HYPERLINK",
        "%clk" => "CLICK_HERE",
        "%bmk" => "BOOKMARK",
        "%fmu" => "FORMULA",
        "%dte" => "DATE",
        "%ddt" => "DOC_DATE",
        "%pat" => "PATH",
        "%xrf" => "CROSSREF",
        "%mmg" => "MAILMERGE",
        "%smr" => "SUMMERY",
        "%usr" => "USER_INFO",
        "%toc" => "TABLEOFCONTENTS",
        _ => "UNKNOWN",
    }
}

fn image_mode(v: i64) -> &'static str {
    match v {
        5 => "TOTAL",
        6 => "CENTER",
        _ => "TILE", // 0; others tile along one edge or sit in a corner
    }
}

// ---- little-endian readers (0 past the end) ------------------------------------------------

fn bytes<const N: usize>(d: &[u8], o: usize) -> Option<[u8; N]> {
    d.get(o..o.checked_add(N)?)?.try_into().ok()
}

fn u8_(d: &[u8], o: usize) -> i64 {
    d.get(o).copied().map(i64::from).unwrap_or(0)
}

fn i8_(d: &[u8], o: usize) -> i64 {
    d.get(o).map(|b| i64::from(*b as i8)).unwrap_or(0)
}

fn u16_(d: &[u8], o: usize) -> i64 {
    bytes::<2>(d, o).map(|b| i64::from(u16::from_le_bytes(b))).unwrap_or(0)
}

fn i16_(d: &[u8], o: usize) -> i64 {
    bytes::<2>(d, o).map(|b| i64::from(i16::from_le_bytes(b))).unwrap_or(0)
}

fn u32_(d: &[u8], o: usize) -> i64 {
    bytes::<4>(d, o).map(|b| i64::from(u32::from_le_bytes(b))).unwrap_or(0)
}

fn i32_(d: &[u8], o: usize) -> i64 {
    bytes::<4>(d, o).map(|b| i64::from(i32::from_le_bytes(b))).unwrap_or(0)
}

fn f64_(d: &[u8], o: usize) -> f64 {
    bytes::<8>(d, o).map(f64::from_le_bytes).unwrap_or(0.0)
}

/// An offset from a record field (never negative).
fn at(o: i64) -> usize {
    usize::try_from(o).unwrap_or(usize::MAX)
}

/// UTF-16LE with replacement characters, as Python's `decode("utf-16-le", "replace")`.
fn utf16(d: &[u8]) -> String {
    let units: Vec<u16> = d.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let mut s = String::from_utf16_lossy(&units);
    if d.len() % 2 == 1 {
        s.push('\u{FFFD}');
    }
    s
}

/// WORD length + WCHAR array: (text, next offset).
fn wstr(d: &[u8], o: usize) -> (String, usize) {
    let n = at(u16_(d, o)) * 2;
    let start = o.saturating_add(2);
    let end = start.saturating_add(n);
    let s = d.get(start.min(d.len())..end.min(d.len())).map(utf16).unwrap_or_default();
    (s, end)
}

/// COLORREF 0x00bbggrr -> "#RRGGBB"; 0xFFFFFFFF (unset) -> "none". Some files set flag bits in
/// the top byte (seen on shape outlines), so only the low 24 bits count.
fn color(v: i64) -> String {
    if v == 0xFFFF_FFFF {
        return "none".into();
    }
    format!("#{:02X}{:02X}{:02X}", v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF)
}

fn pick(table: &[&'static str], i: i64, default: &'static str) -> &'static str {
    usize::try_from(i).ok().and_then(|i| table.get(i)).copied().unwrap_or(default)
}

/// 4-character control id; stored as a little-endian DWORD of MAKE_4CHID(a, b, c, d).
pub(crate) fn ctrl_id(d: &[u8]) -> String {
    match d.get(..4) {
        Some(b) => b.iter().rev().map(|&c| char::from(c)).collect(),
        None => String::new(),
    }
}

fn bit(v: i64, shift: u32) -> i64 {
    (v >> shift) & 1
}

/// An element with attributes (values written as text).
macro_rules! el {
    ($tag:expr $(, $k:expr => $v:expr)* $(,)?) => {{
        #[allow(unused_mut)]
        let mut e = Element::new($tag);
        $( e.attrs.push(($k.to_string(), ($v).to_string())); )*
        e
    }};
}

fn push(parent: &mut Element, child: Element) {
    parent.children.push(child);
}

/// A child wrapped in hp:ctrl, as HWPX writes controls.
fn push_ctrl(run: &mut Element, child: Element) {
    let mut c = Element::new("hp:ctrl");
    c.children.push(child);
    run.children.push(c);
}

/// Append text where the next text of `t` goes (after its last child, or its own text).
fn add_text(t: &mut Element, s: &str) {
    match t.children.last_mut() {
        Some(last) => last.tail.push_str(s),
        None => t.text.push_str(s),
    }
}

// ---- records -------------------------------------------------------------------------------

pub(crate) struct Rec {
    pub(crate) tag: u32,
    pub(crate) level: u32,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) children: Vec<usize>,
}

/// A record stream as a tree (a record's children follow it with a deeper level).
pub(crate) struct Records<'a> {
    data: &'a [u8],
    pub(crate) recs: Vec<Rec>,
    pub(crate) roots: Vec<usize>,
}

/// Each record of a stream: (tag, level, data start, data end), the data cut at the stream's end.
/// Stops after the last whole header; never reads or steps past the end.
fn record_spans(data: &[u8]) -> impl Iterator<Item = (u32, u32, usize, usize)> + '_ {
    let mut i = 0usize;
    std::iter::from_fn(move || {
        let head_end = i.checked_add(4).filter(|&e| e <= data.len())?;
        let h = u32_(data, i);
        let mut start = head_end;
        let mut size = at(h >> 20);
        if size == 0xFFF {
            size = at(u32_(data, start));
            start = start.saturating_add(4).min(data.len());
        }
        let end = start.saturating_add(size).min(data.len());
        // past the end: no more records (`end` never goes back, so this stops)
        i = if start.saturating_add(size) > data.len() { data.len() } else { end };
        Some(((h & 0x3FF) as u32, ((h >> 10) & 0x3FF) as u32, start, end))
    })
}

/// The records of a stream counted against `left` (the document's [`MAX_RECORDS`]).
fn count_records(data: &[u8], left: &mut usize) -> Result<()> {
    for _ in record_spans(data) {
        *left = left.checked_sub(1).ok_or(Error::TooLarge("records"))?;
    }
    Ok(())
}

impl<'a> Records<'a> {
    /// The record tree of a stream. [`open`] has counted the records of every stream it reads
    /// against [`MAX_RECORDS`]; past that many, this stops (never reached for an opened file).
    pub(crate) fn parse(data: &'a [u8]) -> Self {
        let mut r = Records { data, recs: Vec::new(), roots: Vec::new() };
        let mut stack: Vec<usize> = Vec::new();
        for (tag, level, start, end) in record_spans(data).take(MAX_RECORDS) {
            let id = r.recs.len();
            r.recs.push(Rec { tag, level, start, end, children: Vec::new() });
            while stack.last().and_then(|&s| r.recs.get(s)).is_some_and(|s| s.level >= level) {
                stack.pop();
            }
            match stack.last().copied() {
                Some(p) => {
                    if let Some(p) = r.recs.get_mut(p) {
                        p.children.push(id);
                    }
                }
                None => r.roots.push(id),
            }
            stack.push(id);
        }
        r
    }

    pub(crate) fn tag(&self, i: usize) -> u32 {
        self.recs.get(i).map(|r| r.tag).unwrap_or(0)
    }

    pub(crate) fn data(&self, i: usize) -> &'a [u8] {
        self.recs.get(i).and_then(|r| self.data.get(r.start..r.end)).unwrap_or(&[])
    }

    pub(crate) fn children(&self, i: usize) -> &[usize] {
        self.recs.get(i).map(|r| r.children.as_slice()).unwrap_or(&[])
    }

    fn child(&self, i: usize, tag: u32) -> Option<usize> {
        self.children(i).iter().copied().find(|&c| self.tag(c) == tag)
    }

    /// Pair each LIST_HEADER with its paragraphs. In the files they follow it as siblings (same
    /// level), as many as the header's paragraph count.
    pub(crate) fn lists(&self, children: &[usize]) -> Vec<(usize, Vec<usize>)> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(&x) = children.get(i) {
            if self.tag(x) != LIST_HEADER {
                i += 1;
                continue;
            }
            let n = at(u16_(self.data(x), 0));
            let mut paras: Vec<usize> = self.children(x).iter().copied().filter(|&q| self.tag(q) == PARA_HEADER).collect();
            let mut j = i + 1;
            while let Some(&c) = children.get(j) {
                if self.tag(c) != PARA_HEADER || paras.len() >= n {
                    break;
                }
                paras.push(c);
                j += 1;
            }
            out.push((x, paras));
            i = j;
        }
        out
    }
}

/// Raw deflate (the specification says zlib; the streams have no zlib header). A stream cut short
/// gives what it holds, as zlib's decompressobj does.
pub(crate) fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    use miniz_oxide::inflate::TINFLStatus;
    match miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_STREAM as usize) {
        Ok(v) => Ok(v),
        Err(e) if e.status == TINFLStatus::FailedCannotMakeProgress => Ok(e.output),
        Err(e) if e.status == TINFLStatus::HasMoreOutput => Err(Error::TooLarge("stream")),
        Err(e) => Err(Error::format(format!("cannot decompress: {:?}", e.status))),
    }
}

/// An HWP 5.0 compound file (by its FileHeader).
pub(crate) fn is_hwp(bytes: &[u8]) -> bool {
    if !bytes.starts_with(OLE_SIG) {
        return false;
    }
    let Ok(mut c) = cfb::CompoundFile::open(Cursor::new(bytes)) else { return false };
    let Ok(mut s) = c.open_stream("/FileHeader") else { return false };
    let mut head = [0u8; 17];
    s.read_exact(&mut head).is_ok() && &head == b"HWP Document File"
}

/// Open an HWP 5.0 document: the header is rebuilt now, sections when they are read.
pub(crate) fn open(bytes: &[u8]) -> Result<Package> {
    let mut c = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|e| Error::format(format!("damaged or incomplete HWP file ({e})")))?;
    let streams: Vec<(String, u64)> = c
        .walk()
        .filter(|e| e.is_stream())
        .map(|e| (e.path().to_string_lossy().replace('\\', "/").trim_start_matches('/').to_string(), e.len()))
        .collect();
    if streams.len() > MAX_STREAMS {
        return Err(Error::TooLarge("streams"));
    }
    // stream sizes by name: a read doesn't go through every stream (65,536 picture streams)
    let sizes: HashMap<&str, u64> = streams.iter().map(|(n, s)| (n.as_str(), *s)).collect();
    let mut total = 0usize;
    let mut read = |c: &mut cfb::CompoundFile<Cursor<&[u8]>>, name: &str| -> Result<Vec<u8>> {
        let size = sizes.get(name).copied().ok_or_else(|| Error::format(format!("no stream {name}")))?;
        if size > MAX_STREAM {
            return Err(Error::TooLarge("stream"));
        }
        let mut s = c.open_stream(format!("/{name}")).map_err(|e| Error::format(format!("{name}: {e}")))?;
        let mut data = Vec::new();
        s.by_ref().take(size).read_to_end(&mut data).map_err(|e| Error::format(format!("{name}: {e}")))?;
        total = total.saturating_add(data.len());
        if total > MAX_TOTAL {
            return Err(Error::TooLarge("document"));
        }
        Ok(data)
    };
    if !streams.iter().any(|(n, _)| n == "FileHeader") {
        return Err(Error::format("not an HWP 5.0 document (no FileHeader)"));
    }
    let head = read(&mut c, "FileHeader")?;
    if !head.starts_with(b"HWP Document File") {
        return Err(Error::format("not an HWP 5.0 document"));
    }
    let flags = u32_(&head, 36);
    if flags & 0x06 != 0 {
        return Err(Error::Protected); // 0x02 password, 0x04 distribution-only (배포용)
    }
    let compressed = flags & 0x01 != 0;
    // what streams expand to counts too: six sections each inflating to 200 MB pass every
    // single-stream check
    let expanded = std::cell::Cell::new(0usize);
    let grow = |data: &[u8]| -> Result<()> {
        expanded.set(expanded.get().saturating_add(data.len()));
        if expanded.get() > MAX_TOTAL { Err(Error::TooLarge("document")) } else { Ok(()) }
    };
    let mut records_left = MAX_RECORDS;
    let mut stream = |raw: Vec<u8>| -> Result<Vec<u8>> {
        let data = if compressed { inflate(&raw)? } else { raw };
        if compressed {
            grow(&data)?;
        }
        count_records(&data, &mut records_left)?;
        Ok(data)
    };
    let mut pkg = Package::default();
    pkg.header_name = Some(HEADER_NAME.to_string());
    let mut sections: Vec<(i64, String)> =
        streams.iter().filter_map(|(n, _)| n.strip_prefix("BodyText/Section").map(|k| (k.parse::<i64>().unwrap_or(0), n.clone()))).collect();
    sections.sort_by_key(|(k, _)| *k);
    if sections.is_empty() {
        return Err(Error::format("no BodyText sections"));
    }
    let doc_info = stream(read(&mut c, "DocInfo")?)?;
    let (header, items) = doc_info_header(&doc_info);
    for (_, name) in &sections {
        let data = stream(read(&mut c, name)?)?;
        pkg.push_entry(name.clone(), data);
    }
    // Pictures: decompressed unless stored as is; a stream that does not inflate was stored.
    // sets, not lists: each picture item would otherwise go through every stream and every
    // picture read so far
    let stream_names: HashSet<&str> = streams.iter().map(|(n, _)| n.as_str()).collect();
    let mut done: HashSet<&str> = HashSet::new();
    for it in &items {
        if !stream_names.contains(it.href.as_str()) || !done.insert(it.href.as_str()) {
            continue;
        }
        let raw = read(&mut c, &it.href)?;
        let data = if compressed && it.media_type != "stored" {
            match inflate(&raw) {
                Ok(d) => {
                    grow(&d)?;
                    d
                }
                Err(_) => raw,
            }
        } else {
            raw
        };
        pkg.push_entry(it.href.clone(), data);
    }
    pkg.section_names = sections.into_iter().map(|(_, n)| n).collect();
    pkg.set_items(items);
    pkg.hwp5_header = Some(header);
    Ok(pkg)
}

/// A section stream -> hs:sec.
pub(crate) fn section(data: &[u8]) -> Element {
    let r = Records::parse(data);
    let mut body = Body { r: &r, field_ids: Vec::new(), next_field: 1 };
    let mut sec = Element::new("hs:sec");
    for &x in &r.roots {
        if r.tag(x) == PARA_HEADER {
            body.paragraph(&mut sec, x, 0);
        }
    }
    let roots = r.roots.clone();
    body.memos(&mut sec, &roots, 0);
    let mut ordinal = 0;
    number(&mut sec, &mut ordinal, 0);
    sec
}

/// The attribute naming the PARA_HEADER record a paragraph came from (for writing edits back).
pub(crate) const HWP_REC: &str = "hwprec";

/// Number the paragraphs in document order ([`ORDINAL`], as the HWPX reader numbers them by their
/// start tags), so converted paragraphs carry marks for write-back.
fn number(el: &mut Element, ordinal: &mut usize, depth: usize) {
    if depth > 4 * MAX_DEPTH {
        return;
    }
    if el.tag == "hp:p" {
        el.set(ORDINAL, ordinal.to_string());
        *ordinal += 1;
    }
    for c in &mut el.children {
        number(c, ordinal, depth + 1);
    }
}

// ==== DocInfo -> hh:head ====================================================================

/// Python-style slice bounds of `len` items.
fn py_range(len: usize, start: i64, stop: i64) -> std::ops::Range<usize> {
    let norm = |v: i64| -> usize {
        let l = len as i64;
        let v = if v < 0 { v + l } else { v };
        v.clamp(0, l) as usize
    };
    let (a, b) = (norm(start), norm(stop));
    a..b.max(a)
}

fn doc_info_header(data: &[u8]) -> (Element, Vec<Item>) {
    let r = Records::parse(data);
    let mut by_tag: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    let mut stack: Vec<usize> = r.roots.iter().rev().copied().collect();
    while let Some(x) = stack.pop() {
        by_tag.entry(r.tag(x)).or_default().push(x);
        stack.extend(r.children(x).iter().rev());
    }
    let recs = |t: u32| by_tag.get(&t).cloned().unwrap_or_default();
    let mut head = Element::new("hh:head");
    if let Some(&props) = recs(DOCUMENT_PROPERTIES).first() {
        let page = u16_(r.data(props), 2);
        push(&mut head, el!("hh:beginNum", "page" => if page == 0 { 1 } else { page }));
    }
    let counts: Vec<i64> = match recs(ID_MAPPINGS).first() {
        Some(&m) => {
            let d = r.data(m);
            (0..d.len() / 4).map(|k| i32_(d, 4 * k)).collect()
        }
        None => Vec::new(),
    };
    let mut items = Vec::new();
    bin_data(&r, &recs(BIN_DATA), &mut items);
    let mut rl = Element::new("hh:refList");
    fonts(&r, &mut rl, &recs(FACE_NAME), counts.get(1..8).unwrap_or(&[]));
    border_fills(&r, &mut rl, &recs(BORDER_FILL));
    char_shapes(&r, &mut rl, &recs(CHAR_SHAPE));
    tab_defs(&r, &mut rl, &recs(TAB_DEF));
    numberings(&r, &mut rl, &recs(NUMBERING));
    bullets(&r, &mut rl, &recs(BULLET));
    para_shapes(&r, &mut rl, &recs(PARA_SHAPE));
    styles(&r, &mut rl, &recs(STYLE));
    push(&mut head, rl);
    (head, items)
}

/// Adds `item`, or replaces the item with its id. `at` is where each id is in `items`, so a
/// DocInfo of a million BIN_DATA records isn't a million scans of the list.
fn set_item(items: &mut Vec<Item>, at: &mut HashMap<String, usize>, item: Item) {
    match at.get(&item.id).and_then(|&k| items.get_mut(k)) {
        Some(i) => *i = item,
        None => {
            at.insert(item.id.clone(), items.len());
            items.push(item);
        }
    }
}

fn bin_data(r: &Records, recs: &[usize], items: &mut Vec<Item>) {
    let mut at = HashMap::new();
    for (i, &x) in recs.iter().enumerate() {
        let d = r.data(x);
        let attr = u16_(d, 0);
        let kind = attr & 0x0F;
        if kind == 0 {
            continue; // LINK: an external file, nothing to embed
        }
        let sid = u16_(d, 2);
        let ext = if kind == 1 { wstr(d, 4).0 } else { "ole".to_string() };
        // Stream names in real files are BinData/BIN%04X.<ext> (the specification shows
        // BinaryData%d).
        let id = format!("BIN{sid:04X}");
        let href = format!("BinData/{id}.{ext}");
        let media_type = if attr & 0x30 == 0x20 { "stored" } else { "" }.to_string();
        set_item(items, &mut at, Item { id: id.clone(), href: href.clone(), media_type: media_type.clone() });
        set_item(items, &mut at, Item { id: (i + 1).to_string(), href, media_type });
    }
}

fn fonts(r: &Records, ref_list: &mut Element, recs: &[usize], counts: &[i64]) {
    let mut faces = Element::new("hh:fontfaces");
    let all = [recs.len() as i64];
    let counts = if counts.is_empty() { &all[..] } else { counts };
    let mut pos = 0i64;
    for (lang, &n) in FONT_LANGS.iter().zip(counts) {
        let mut ff = el!("hh:fontface", "lang" => lang, "fontCnt" => n);
        let range = py_range(recs.len(), pos, pos.saturating_add(n));
        for (k, &x) in recs.get(range).unwrap_or(&[]).iter().enumerate() {
            push(&mut ff, el!("hh:font", "id" => k, "face" => wstr(r.data(x), 1).0));
        }
        push(&mut faces, ff);
        pos = pos.saturating_add(n);
    }
    push(ref_list, faces);
}

/// hc:gradation from a fill at `f`: type, angle, centre x, y, blur, count, (positions,) colours.
/// Returns it and the offset after it.
fn gradation(d: &[u8], f: usize) -> (Element, usize) {
    let n = i32_(d, f + 17).clamp(0, 16);
    let mut g = el!("hc:gradation", "type" => u8_(d, f), "angle" => i32_(d, f + 1), "colorNum" => n);
    let n = at(n);
    let c = f + 21 + if n > 2 { 4 * n } else { 0 };
    for k in 0..n {
        push(&mut g, el!("hc:color", "value" => color(u32_(d, c + 4 * k))));
    }
    (g, c + 4 * n)
}

fn border_fills(r: &Records, ref_list: &mut Element, recs: &[usize]) {
    let mut bfs = Element::new("hh:borderFills");
    for (i, &x) in recs.iter().enumerate() {
        let d = r.data(x);
        let attr = u16_(d, 0);
        let mut bf = el!("hh:borderFill", "id" => i + 1);
        push(&mut bf, el!("hh:slash", "type" => if (attr >> 2) & 7 != 0 { "CENTER" } else { "NONE" }));
        push(&mut bf, el!("hh:backSlash", "type" => if (attr >> 5) & 7 != 0 { "CENTER" } else { "NONE" }));
        // Per side: type, width, colour (interleaved; the specification lists them as three
        // separate arrays). Order: left, right, top, bottom.
        for (k, tag) in ["hh:leftBorder", "hh:rightBorder", "hh:topBorder", "hh:bottomBorder"].iter().enumerate() {
            let o = 2 + 6 * k;
            push(
                &mut bf,
                el!(tag, "type" => pick(LINE_TYPES, u8_(d, o), "SOLID"), "width" => pick(LINE_WIDTHS, u8_(d, o + 1), "0.1 mm"), "color" => color(u32_(d, o + 2))),
            );
        }
        push(&mut bf, el!("hh:diagonal", "type" => "SOLID", "width" => pick(LINE_WIDTHS, u8_(d, 27), "0.1 mm"), "color" => color(u32_(d, 28))));
        let fill = u32_(d, 32);
        let mut fb = Element::new("hc:fillBrush");
        let mut f = 36usize;
        if fill & 1 != 0 {
            push(&mut fb, el!("hc:winBrush", "faceColor" => color(u32_(d, 36)), "hatchColor" => color(u32_(d, 40)), "alpha" => 0));
            f += 12;
        }
        if fill & 4 != 0 && f + 21 <= d.len() {
            let (g, next) = gradation(d, f);
            push(&mut fb, g);
            f = next;
        }
        if fill & 2 != 0 && f + 6 <= d.len() {
            // Image: mode (1 byte), brightness, contrast, effect, BinData id (2 bytes).
            let mut ib = el!("hc:imgBrush", "mode" => image_mode(u8_(d, f)));
            push(&mut ib, el!("hc:img", "binaryItemIDRef" => u16_(d, f + 4)));
            push(&mut fb, ib);
        }
        if fill & 7 != 0 {
            push(&mut bf, fb);
        }
        push(&mut bfs, bf);
    }
    push(ref_list, bfs);
}

fn lang_attrs(tag: &str, v: impl Fn(usize) -> i64) -> Element {
    let mut e = Element::new(tag);
    for (k, a) in LANGS.iter().enumerate() {
        e.attrs.push((a.to_string(), v(k).to_string()));
    }
    e
}

fn char_shapes(r: &Records, ref_list: &mut Element, recs: &[usize]) {
    let mut cps = Element::new("hh:charProperties");
    for (i, &x) in recs.iter().enumerate() {
        let d = r.data(x);
        let (height, attr) = (i32_(d, 42), u32_(d, 46));
        let (text, under, shade) = (u32_(d, 52), u32_(d, 56), u32_(d, 60));
        let emph = (attr >> 21) & 0xF;
        let mut cp = el!(
            "hh:charPr",
            "id" => i,
            "height" => height,
            "textColor" => color(text),
            "shadeColor" => color(shade),
            "symMark" => if emph == 0 { "NONE" } else { "DOT_ABOVE" },
            "borderFillIDRef" => u16_(d, 68),
            // bit 25: a space as wide as the font's (Hancom: 글꼴에 어울리는 빈칸; public specification, table 35)
            "useFontSpace" => (attr >> 25) & 1,
        );
        push(&mut cp, lang_attrs("hh:fontRef", |k| u16_(d, 2 * k)));
        push(&mut cp, lang_attrs("hh:ratio", |k| u8_(d, 14 + k)));
        push(&mut cp, lang_attrs("hh:spacing", |k| i8_(d, 21 + k)));
        push(&mut cp, lang_attrs("hh:relSz", |k| u8_(d, 28 + k)));
        push(&mut cp, lang_attrs("hh:offset", |k| i8_(d, 35 + k)));
        if attr & 1 != 0 {
            push(&mut cp, Element::new("hh:italic"));
        }
        if attr & 2 != 0 {
            push(&mut cp, Element::new("hh:bold"));
        }
        let ut = (attr >> 2) & 3;
        if ut != 0 {
            // Underline/strikeout shapes use the specification's table as is (0 = solid); their
            // presence is a separate field.
            push(
                &mut cp,
                el!("hh:underline", "type" => if ut == 3 { "TOP" } else { "BOTTOM" }, "shape" => pick(LINE_TYPES, ((attr >> 4) & 0xF) + 1, "SOLID"), "color" => color(under)),
            );
        }
        if (attr >> 18) & 7 != 0 {
            push(&mut cp, el!("hh:strikeout", "shape" => pick(LINE_TYPES, ((attr >> 26) & 0xF) + 1, "SOLID"), "color" => color(u32_(d, 70))));
        }
        if (attr >> 8) & 7 != 0 {
            push(&mut cp, el!("hh:outline", "type" => "SOLID"));
        }
        if (attr >> 11) & 3 != 0 {
            push(&mut cp, el!("hh:shadow", "type" => "DROP"));
        }
        for (b, tag) in [(13, "hh:emboss"), (14, "hh:engrave"), (15, "hh:supscript"), (16, "hh:subscript")] {
            if bit(attr, b) != 0 {
                push(&mut cp, Element::new(tag));
            }
        }
        push(&mut cps, cp);
    }
    push(ref_list, cps);
}

fn tab_defs(r: &Records, ref_list: &mut Element, recs: &[usize]) {
    let mut tps = Element::new("hh:tabProperties");
    for (i, &x) in recs.iter().enumerate() {
        let d = r.data(x);
        let a = u32_(d, 0);
        let mut tp = el!("hh:tabPr", "id" => i, "autoTabLeft" => a & 1, "autoTabRight" => bit(a, 1));
        for k in 0..at(i32_(d, 4).max(0)) {
            let o = 8 + 8 * k;
            if o + 6 > d.len() {
                break;
            }
            // Tab positions are stored doubled, like paragraph margins (measured).
            push(
                &mut tp,
                el!("hh:tabItem", "pos" => u32_(d, o) / 2, "type" => pick(TAB_TYPES, u8_(d, o + 4), "LEFT"), "leader" => pick(LINE_TYPES, u8_(d, o + 5), "NONE")),
            );
        }
        push(&mut tps, tp);
    }
    push(ref_list, tps);
}

fn numberings(r: &Records, ref_list: &mut Element, recs: &[usize]) {
    let mut nums = Element::new("hh:numberings");
    for (i, &x) in recs.iter().enumerate() {
        let d = r.data(x);
        let mut o = 0usize;
        let mut levels = Vec::new();
        for lv in 1..8 {
            // Paragraph head info is 12 bytes in real files (the specification's table says 8
            // but lists 12 bytes of fields).
            let attr = u32_(d, o);
            o += 12;
            let (fmt, next) = wstr(d, o);
            o = next;
            levels.push((lv, attr, fmt));
        }
        let start = u16_(d, o);
        let mut n = el!("hh:numbering", "id" => i + 1, "start" => start);
        for (k, (lv, attr, fmt)) in levels.into_iter().enumerate() {
            let st = u32_(d, o + 2 + 4 * k);
            // The number format code is in bits 5-8 of the head attributes (not in the
            // specification's table 40; with it the default outline numbering reads 1. / 가. /
            // 1) / 가) / (1) / (가) / circled 1, where 가 counts in Hangul letters).
            let mut ph = el!(
                "hh:paraHead",
                "start" => if st == 0 { 1 } else { st },
                "level" => lv,
                "align" => pick(&["LEFT", "CENTER", "RIGHT"], attr & 3, "LEFT"),
                "numFormat" => pick(NUM_FORMATS, (attr >> 5) & 0xF, "DIGIT"),
            );
            ph.text = fmt;
            push(&mut n, ph);
        }
        push(&mut nums, n);
    }
    push(ref_list, nums);
}

fn bullets(r: &Records, ref_list: &mut Element, recs: &[usize]) {
    let mut bs = Element::new("hh:bullets");
    for (i, &x) in recs.iter().enumerate() {
        let d = r.data(x);
        let ch = match d.get(12..14) {
            Some(b) => utf16(b),
            None => "•".to_string(),
        };
        push(&mut bs, el!("hh:bullet", "id" => i + 1, "char" => if ch.is_empty() { "•".to_string() } else { ch }));
    }
    push(ref_list, bs);
}

fn para_shapes(r: &Records, ref_list: &mut Element, recs: &[usize]) {
    let mut pps = Element::new("hh:paraProperties");
    for (i, &x) in recs.iter().enumerate() {
        let d = r.data(x);
        let a1 = u32_(d, 0);
        let [left, right, indent, prev, nxt, old_ls] = [4, 8, 12, 16, 20, 24].map(|o| i32_(d, o));
        let (a3, ls) = if d.len() >= 54 { (u32_(d, 46), u32_(d, 50)) } else { (a1 & 3, old_ls) };
        // bits 9-15: the minimum space width (Hancom: 공백 최소값; public specification, table 44)
        let mut pp = el!("hh:paraPr", "id" => i, "tabPrIDRef" => u16_(d, 28), "condense" => (a1 >> 9) & 0x7F);
        push(&mut pp, el!("hh:align", "horizontal" => pick(ALIGNS, (a1 >> 2) & 7, "JUSTIFY")));
        push(&mut pp, el!("hh:heading", "type" => pick(HEADINGS, (a1 >> 23) & 3, "NONE"), "idRef" => u16_(d, 30), "level" => (a1 >> 25) & 7));
        push(
            &mut pp,
            el!(
                "hh:breakSetting",
                "widowOrphan" => bit(a1, 16),
                "keepWithNext" => bit(a1, 17),
                "keepLines" => bit(a1, 18),
                "pageBreakBefore" => bit(a1, 19),
                "breakNonLatinWord" => if a1 & 0x80 != 0 { "BREAK_WORD" } else { "KEEP_WORD" },
            ),
        );
        // Margins are stored doubled, like the hp:default copy in HWPX (measured: the body text
        // style's (본문) 15pt left margin is 3000).
        let mut m = Element::new("hh:margin");
        for (tag, v) in [("hc:intent", indent), ("hc:left", left), ("hc:right", right), ("hc:prev", prev), ("hc:next", nxt)] {
            push(&mut m, el!(tag, "value" => v / 2, "unit" => "HWPUNIT"));
        }
        push(&mut pp, m);
        push(&mut pp, el!("hh:lineSpacing", "type" => pick(LINE_SPACING, a3 & 0x1F, "PERCENT"), "value" => ls, "unit" => "HWPUNIT"));
        push(&mut pp, el!("hh:border", "borderFillIDRef" => u16_(d, 32)));
        push(&mut pps, pp);
    }
    push(ref_list, pps);
}

fn styles(r: &Records, ref_list: &mut Element, recs: &[usize]) {
    let mut sts = Element::new("hh:styles");
    for (i, &x) in recs.iter().enumerate() {
        let d = r.data(x);
        let (name, o) = wstr(d, 0);
        let (eng, o) = wstr(d, o);
        let (attr, nxt) = if o.saturating_add(1) < d.len() { (u8_(d, o), u8_(d, o + 1)) } else { (0, i as i64) };
        push(
            &mut sts,
            el!(
                "hh:style",
                "id" => i,
                "type" => if attr & 7 == 1 { "CHAR" } else { "PARA" },
                "name" => name,
                "engName" => eng,
                "paraPrIDRef" => u16_(d, o.saturating_add(4)),
                "charPrIDRef" => u16_(d, o.saturating_add(6)),
                "nextStyleIDRef" => nxt,
            ),
        );
    }
    push(ref_list, sts);
}

// ==== BodyText/Section -> hs:sec ============================================================

/// Flatten a form object's "Key:type:value" string; wstring and set carry a length (a set's
/// members simply follow, so they are read as top-level keys).
fn form_props(s: &str) -> BTreeMap<String, String> {
    const TYPES: [&str; 5] = ["wstring", "int", "bool", "set", "uint"];
    let c: Vec<char> = s.chars().collect();
    let word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let starts = |at: usize, p: &str| p.chars().enumerate().all(|(k, pc)| c.get(at + k) == Some(&pc));
    let mut out = BTreeMap::new();
    let mut i = 0usize;
    let mut guard = 0;
    while guard < 1_000_000 {
        guard += 1;
        // the next "<word>:<type>:" at or after i
        let mut found = None;
        let mut j = i;
        while j < c.len() {
            if !c.get(j).is_some_and(|&ch| word(ch)) {
                j += 1;
                continue;
            }
            let mut k = j;
            while c.get(k).is_some_and(|&ch| word(ch)) {
                k += 1;
            }
            if c.get(k) == Some(&':')
                && let Some(t) = TYPES.iter().find(|t| starts(k + 1, t) && c.get(k + 1 + t.len()) == Some(&':'))
            {
                found = Some((c.get(j..k).unwrap_or(&[]).iter().collect::<String>(), *t, k + 2 + t.len()));
                break;
            }
            j = k;
        }
        let Some((key, typ, end)) = found else { return out };
        i = end;
        if matches!(typ, "wstring" | "set") {
            let n: String = c.get(i..).unwrap_or(&[]).iter().take_while(|&&ch| ch != ':').collect();
            if n.is_empty() || !n.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }
            i += n.chars().count() + 1;
            if typ == "wstring" {
                let len: usize = n.parse().unwrap_or(0);
                let v: String = c.get(i.min(c.len())..i.saturating_add(len).min(c.len())).unwrap_or(&[]).iter().collect();
                out.insert(key, v);
                i = i.saturating_add(len);
            }
        } else {
            let mut k = i;
            if c.get(k) == Some(&'-') {
                k += 1;
            }
            let w = c.get(k..).unwrap_or(&[]).iter().take_while(|&&ch| word(ch)).count();
            let v: String = if w > 0 { c.get(i..k + w).unwrap_or(&[]).iter().collect() } else { String::new() };
            i += v.chars().count();
            out.insert(key, v);
        }
    }
    out
}

/// Box of a grouped object in the outermost group's space, from its rendering info: a
/// translation matrix, then a scale and a rotation matrix per group level (each 3x2, a b c / d
/// e f). The box is unrotated; hp:rotationInfo turns it.
fn placed(d: &[u8], o: usize, org_w: f64, org_h: f64) -> Option<(f64, f64, f64, f64)> {
    let n = at(u16_(d, o + 42));
    if n == 0 || o + 44 + 48 * (1 + 2 * n) > d.len() {
        return None;
    }
    let (mut a, mut b, mut c, mut dd, mut e, mut f) = (1.0, 0.0, 0.0, 0.0, 1.0, 0.0);
    for i in 0..(1 + 2 * n) {
        let base = o + 44 + 48 * i;
        let m: [f64; 6] = std::array::from_fn(|k| f64_(d, base + 8 * k));
        // M = T * S1 * R1 * S2 * R2 ...
        (a, b, c, dd, e, f) =
            (a * m[0] + b * m[3], a * m[1] + b * m[4], a * m[2] + b * m[5] + c, dd * m[0] + e * m[3], dd * m[1] + e * m[4], dd * m[2] + e * m[5] + f);
    }
    let (w, h) = (org_w * (a * a + dd * dd).sqrt(), org_h * (b * b + e * e).sqrt());
    let (cx, cy) = (a * org_w / 2.0 + b * org_h / 2.0 + c, dd * org_w / 2.0 + e * org_h / 2.0 + f);
    Some((cx - w / 2.0, cy - h / 2.0, w, h))
}

/// Object common properties (after the control id) -> hp:sz, hp:pos, hp:outMargin.
fn common(obj: &mut Element, d: &[u8]) {
    let attr = u32_(d, 4);
    obj.set("zOrder", i32_(d, 24).to_string());
    obj.set("textFlow", pick(&["BOTH_SIDES", "LEFT_ONLY", "RIGHT_ONLY", "LARGEST_ONLY"], (attr >> 24) & 3, "BOTH_SIDES"));
    // 0 square, 1 top and bottom, 2 behind text, 3 in front of text (HWPX adds TIGHT and THROUGH).
    obj.set("textWrap", pick(&["SQUARE", "TOP_AND_BOTTOM", "BEHIND_TEXT", "IN_FRONT_OF_TEXT"], (attr >> 21) & 7, "TOP_AND_BOTTOM"));
    push(obj, el!("hp:sz", "width" => u32_(d, 16), "height" => u32_(d, 20)));
    push(
        obj,
        el!(
            "hp:pos",
            "treatAsChar" => attr & 1,
            "affectLSpacing" => bit(attr, 2),
            "vertRelTo" => pick(&["PAPER", "PAGE", "PARA"], (attr >> 3) & 3, "PARA"),
            // the specification names 0 and 1 both "page"; Hancom's viewer places 0 from the
            // paper's edge (checked with a picture and lines whose value is 0)
            "horzRelTo" => pick(&["PAPER", "PAGE", "COLUMN", "PARA"], (attr >> 8) & 3, "PARA"),
            "vertAlign" => pick(&["TOP", "CENTER", "BOTTOM", "INSIDE", "OUTSIDE"], (attr >> 5) & 7, "TOP"),
            "horzAlign" => pick(&["LEFT", "CENTER", "RIGHT", "INSIDE", "OUTSIDE"], (attr >> 10) & 7, "LEFT"),
            "vertOffset" => i32_(d, 8),
            "horzOffset" => i32_(d, 12),
        ),
    );
    push(obj, el!("hp:outMargin", "left" => u16_(d, 28), "right" => u16_(d, 30), "top" => u16_(d, 32), "bottom" => u16_(d, 34)));
}

/// Line and fill information after the component's rendering matrices.
///
/// Measured on real files: width is a 4-byte value (the specification says 2), line types use
/// 0 for none like border fills, gradients store their type in one byte and the other fields
/// in four.
fn outline_and_fill(obj: &mut Element, d: &[u8], o: usize) {
    let mats = at(u16_(d, o + 42));
    let b = o + 44 + 48 + 96 * mats;
    if b + 13 > d.len() {
        return;
    }
    let (lc, lw, la) = (u32_(d, b), i32_(d, b + 4), u32_(d, b + 8));
    push(
        obj,
        el!(
            "hp:lineShape",
            "color" => color(lc),
            "width" => lw.max(0),
            "style" => pick(LINE_TYPES, la & 0x3F, "SOLID"),
            "headStyle" => pick(ARROWS, (la >> 10) & 0x3F, "NORMAL"),
            "tailStyle" => pick(ARROWS, (la >> 16) & 0x3F, "NORMAL"),
        ),
    );
    let mut f = b + 13;
    let kind = u32_(d, f);
    f += 4;
    let mut fb = Element::new("hc:fillBrush");
    if kind & 1 != 0 {
        push(&mut fb, el!("hc:winBrush", "faceColor" => color(u32_(d, f)), "hatchColor" => color(u32_(d, f + 4)), "alpha" => 0));
        f += 12;
    }
    if kind & 4 != 0 && f + 21 <= d.len() {
        push(&mut fb, gradation(d, f).0);
    }
    push(obj, fb);
}

/// Points of lines, polygons and curves (pairs of x, y; the specification lists all x then all
/// y, and a 2-byte count where files have 4).
fn geometry(r: &Records, obj: &mut Element, tag: &str, children: &[usize]) {
    for &x in children {
        let g = r.data(x);
        let t = r.tag(x);
        if t == SC_LINE && tag == "hp:line" {
            push(obj, el!("hc:startPt", "x" => i32_(g, 0), "y" => i32_(g, 4)));
            push(obj, el!("hc:endPt", "x" => i32_(g, 8), "y" => i32_(g, 12)));
        } else if t == SC_RECTANGLE && tag == "hp:rect" {
            obj.set("ratio", u8_(g, 0).to_string());
        } else if (t == SC_POLYGON || t == SC_CURVE) && matches!(tag, "hp:polygon" | "hp:curve") {
            let cap = crate::convert::floor_div(g.len() as i64 - 4, 8);
            let n = at(i32_(g, 0).min(cap).max(0));
            let pts: Vec<(i64, i64)> = (0..n).map(|k| (i32_(g, 4 + 8 * k), i32_(g, 8 + 8 * k))).collect();
            if tag == "hp:polygon" {
                for (px, py) in pts {
                    push(obj, el!("hc:pt", "x" => px, "y" => py));
                }
            } else {
                let types = g.get((4 + 8 * n).min(g.len())..(4 + 8 * n + n.saturating_sub(1)).min(g.len())).unwrap_or(&[]);
                for k in 1..n {
                    let (Some(a), Some(b)) = (pts.get(k - 1), pts.get(k)) else { continue };
                    let curve = types.get(k - 1).is_some_and(|t| *t != 0);
                    push(obj, el!("hp:seg", "type" => if curve { "CURVE" } else { "LINE" }, "x1" => a.0, "y1" => a.1, "x2" => b.0, "y2" => b.1));
                }
            }
        }
    }
}

struct Body<'a, 'r> {
    r: &'r Records<'a>,
    field_ids: Vec<i64>,
    next_field: i64,
}

/// The paragraph being rebuilt: its element and where text goes.
struct ParaState {
    p: Element,
    /// Index into `bounds` of the current (last) run.
    k_shape: Option<usize>,
    /// The current run's last child is the hp:t that takes text.
    t_open: bool,
}

impl ParaState {
    /// A run of its own for a control: a new one (with the same character shape) when the
    /// current run holds text, and text after the control starts another, so text and objects
    /// never share a run (what write-back can edit around them).
    fn control_run(&mut self) {
        if self.run().children.iter().any(|c| c.tag == "hp:t") {
            let attrs = self.run().attrs.clone();
            self.p.children.push(Element { tag: "hp:run".into(), attrs, ..Default::default() });
        }
        self.t_open = false;
    }

    fn run(&mut self) -> &mut Element {
        if self.p.children.is_empty() {
            self.p.children.push(Element::new("hp:run"));
        }
        let n = self.p.children.len() - 1;
        &mut self.p.children[n]
    }

    fn text(&mut self) -> &mut Element {
        let open = self.t_open;
        self.t_open = true;
        let run = self.run();
        if !open || run.children.is_empty() {
            run.children.push(Element::new("hp:t"));
        }
        let n = run.children.len() - 1;
        &mut run.children[n]
    }
}

impl Body<'_, '_> {
    fn memos(&mut self, sec: &mut Element, recs: &[usize], depth: usize) {
        // Memos (Hancom: 메모): a MEMO_LIST record (the memo's number) followed by a LIST_HEADER and its
        // paragraphs, kept under the section's last paragraph. -> hp:memo id=<number>.
        if depth > MAX_DEPTH {
            return;
        }
        let r = self.r;
        for &x in recs {
            let kids = r.children(x);
            for (i, &m) in kids.iter().enumerate() {
                if r.tag(m) != MEMO_LIST {
                    continue;
                }
                let groups = match kids.get(i + 1) {
                    Some(&lh) => {
                        let end = (i + 2).saturating_add(at(u16_(r.data(lh), 0))).min(kids.len());
                        r.lists(kids.get(i + 1..end).unwrap_or(&[]))
                    }
                    None => Vec::new(),
                };
                let mut memo = el!("hp:memo", "id" => u32_(r.data(m), 0));
                if let Some((_, paras)) = groups.first() {
                    let mut sub = Element::new("hp:subList");
                    for &q in paras {
                        self.paragraph(&mut sub, q, depth + 1);
                    }
                    push(&mut memo, sub);
                }
                push(sec, memo);
            }
            self.memos(sec, kids, depth + 1);
        }
    }

    fn paragraph(&mut self, parent: &mut Element, ph: usize, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let r = self.r;
        let d = r.data(ph);
        let brk = u8_(d, 11);
        let p = el!("hp:p", "paraPrIDRef" => u16_(d, 8), "styleIDRef" => u8_(d, 10), "pageBreak" => bit(brk, 2), "columnBreak" => bit(brk, 3), HWP_REC => ph);
        let (mut text, mut shapes, mut ctrls, mut segs): (&[u8], Vec<(i64, i64)>, Vec<usize>, Option<usize>) = (&[], Vec::new(), Vec::new(), None);
        for &c in r.children(ph) {
            match r.tag(c) {
                PARA_TEXT => text = r.data(c),
                PARA_CHAR_SHAPE => {
                    let cd = r.data(c);
                    shapes = (0..cd.len().saturating_sub(7)).step_by(8).map(|k| (u32_(cd, k), u32_(cd, k + 4))).collect();
                }
                CTRL_HEADER => ctrls.push(c),
                PARA_LINE_SEG => segs = Some(c),
                _ => {}
            }
        }
        if shapes.is_empty() {
            shapes.push((0, 0));
        }
        let chars: Vec<u16> = text.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        // Walk the text: runs change where the character shape changes (positions count WCHARs,
        // a non-character control taking eight).
        let mut bounds = shapes;
        bounds.sort();
        let first_shape = bounds.first().copied().unwrap_or((0, 0));
        let mut st = ParaState { p, k_shape: None, t_open: false };
        let mut ctrl_iter = ctrls.into_iter();
        let mut i = 0usize;
        while let Some(&ch) = chars.get(i) {
            if ch == 13 {
                break; // paragraph end (before a run is opened for it)
            }
            let pos = i as i64;
            let k = if pos >= first_shape.0 { bounds.partition_point(|b| b.0 <= pos).saturating_sub(1) } else { 0 };
            if st.k_shape != Some(k) {
                st.p.children.push(el!("hp:run", "charPrIDRef" => bounds.get(k).map(|b| b.1).unwrap_or(0)));
                st.k_shape = Some(k);
                st.t_open = false;
            }
            if ch >= 32 || CHAR_CONTROLS.contains(&ch) {
                if ch == 13 {
                    break; // paragraph end
                }
                let next = chars.get(i + 1).copied();
                let t = st.text();
                if (0xD800..=0xDBFF).contains(&ch)
                    && let Some(lo) = next.filter(|lo| (0xDC00..=0xDFFF).contains(lo))
                {
                    // UTF-16 surrogate pair: one character in two WCHARs
                    let cp = 0x10000 + ((u32::from(ch) - 0xD800) << 10) + u32::from(lo) - 0xDC00;
                    add_text(t, &char::from_u32(cp).unwrap_or('\u{FFFD}').to_string());
                    i += 2;
                    continue;
                }
                match ch {
                    32.. => {
                        let bad = (0xD800..=0xDFFF).contains(&ch) || ch == 0xFFFE || ch == 0xFFFF;
                        let c = if bad { '\u{FFFD}' } else { char::from_u32(u32::from(ch)).unwrap_or('\u{FFFD}') };
                        add_text(t, &c.to_string());
                    }
                    10 => push(t, Element::new("hp:lineBreak")),
                    24 => push(t, Element::new("hp:hypen")),
                    30 => push(t, Element::new("hp:nbSpace")),
                    31 => push(t, Element::new("hp:fwSpace")),
                    _ => {}
                }
                i += 1;
                continue;
            }
            // eight-WCHAR control
            if ch == 9 {
                push(st.text(), Element::new("hp:tab"));
            } else if ch == 4 {
                if let Some(fid) = self.field_ids.pop() {
                    st.control_run();
                    push_ctrl(st.run(), el!("hp:fieldEnd", "beginIDRef" => fid));
                    st.k_shape = None;
                }
                st.t_open = false;
            } else if INLINE_CONTROLS.contains(&ch) {
                // title mark and reserved ones carry no text
            } else {
                if let Some(c) = ctrl_iter.next() {
                    st.control_run();
                    let mut run = std::mem::take(st.run());
                    let had = run.children.len();
                    self.control(&mut run, c, depth);
                    if run.children.len() == had {
                        // a control with nothing to show still holds its place: write-back counts
                        // the text on either side of it as two pieces, as the record has them
                        push(&mut run, Element::new("hp:ctrl"));
                    }
                    *st.run() = run;
                    st.k_shape = None;
                }
                st.t_open = false;
            }
            i += 8;
        }
        let mut p = st.p;
        if p.children.is_empty() {
            push(&mut p, el!("hp:run", "charPrIDRef" => first_shape.1));
        }
        if let Some(s) = segs {
            let sd = r.data(s);
            let mut la = Element::new("hp:linesegarray");
            for k in (0..sd.len().saturating_sub(35)).step_by(36) {
                let v = |n: usize| i32_(sd, k + 4 * n);
                push(
                    &mut la,
                    el!(
                        "hp:lineseg",
                        "textpos" => u32_(sd, k),
                        "vertpos" => v(1),
                        "vertsize" => v(2),
                        "textheight" => v(3),
                        "baseline" => v(4),
                        "spacing" => v(5),
                        "horzpos" => v(6),
                        "horzsize" => v(7),
                        "flags" => u32_(sd, k + 32),
                    ),
                );
            }
            push(&mut p, la);
        }
        push(parent, p);
    }

    fn control(&mut self, run: &mut Element, c: usize, depth: usize) {
        let r = self.r;
        let d = r.data(c);
        let cid = ctrl_id(d);
        match cid.as_str() {
            "secd" => self.section_def(run, c, depth),
            "cold" => {
                let a = u16_(d, 4);
                push_ctrl(
                    run,
                    el!("hp:colPr", "type" => "NEWSPAPER", "colCount" => ((a >> 2) & 0xFF).max(1), "sameSz" => bit(a, 12), "sameGap" => u16_(d, 6)),
                );
            }
            "head" | "foot" => {
                let apply = pick(&["BOTH", "EVEN", "ODD"], u32_(d, 4) & 3, "BOTH");
                let mut hf = el!(if cid == "head" { "hp:header" } else { "hp:footer" }, "applyPageType" => apply);
                self.sub_list(&mut hf, r.children(c), depth);
                push_ctrl(run, hf);
            }
            "fn  " | "en  " => {
                let mut note = Element::new(if cid == "fn  " { "hp:footNote" } else { "hp:endNote" });
                self.sub_list(&mut note, r.children(c), depth);
                push_ctrl(run, note);
            }
            "atno" | "nwno" => {
                let tag = if cid == "atno" { "hp:autoNum" } else { "hp:newNum" };
                push_ctrl(run, el!(tag, "num" => u16_(d, 8), "numType" => pick(AUTO_NUM_TYPES, u32_(d, 4) & 0xF, "PAGE")));
            }
            "pghd" => {
                let a = u32_(d, 4);
                push_ctrl(
                    run,
                    el!(
                        "hp:pageHiding",
                        "hideHeader" => a & 1,
                        "hideFooter" => bit(a, 1),
                        "hideMasterPage" => bit(a, 2),
                        "hideBorder" => bit(a, 3),
                        "hideFill" => bit(a, 4),
                        "hidePageNum" => bit(a, 5),
                    ),
                );
            }
            "pgnp" => {
                let a = u32_(d, 4);
                // User symbol, prefix, suffix, then the side character; the viewer draws the
                // last one on both sides ("- 1 -") when the others are empty.
                let side = [10usize, 14]
                    .iter()
                    .filter_map(|&o| d.get(o..o + 2))
                    .map(|b| utf16(b).trim_matches('\0').to_string())
                    .find(|s| !s.is_empty())
                    .unwrap_or_default();
                push_ctrl(
                    run,
                    el!(
                        "hp:pageNum",
                        "pos" => pick(PAGE_NUM_POS, (a >> 8) & 0xF, "BOTTOM_CENTER"),
                        "formatType" => pick(NUM_FORMATS, a & 0xFF, "DIGIT"),
                        "sideChar" => side,
                    ),
                );
            }
            "bokm" => {
                let name = self.param_string(c);
                push_ctrl(run, el!("hp:bookmark", "name" => if name.is_empty() { "bookmark".to_string() } else { name }));
            }
            "tcps" => {
                let (text, o) = wstr(d, 4);
                push(
                    run,
                    el!(
                        "hp:compose",
                        "circleType" => if u8_(d, o) != 0 { "CHAR" } else { "NONE" },
                        "charSz" => i8_(d, o.saturating_add(1)),
                        "composeType" => "OVERLAP",
                        "composeText" => text,
                    ),
                );
            }
            "tbl " => self.table(run, c, depth),
            "gso " => {
                if let Some(mut shape) = self.shape_object(c, depth) {
                    // its control record, so a save matches it to its own records
                    shape.set(HWP_REC, c.to_string());
                    push(run, shape);
                }
            }
            "eqed" => {
                let mut eq = Element::new("hp:equation");
                common(&mut eq, d);
                if let Some(rec) = r.child(c, EQEDIT) {
                    let mut s = Element::new("hp:script");
                    s.text = wstr(r.data(rec), 4).0;
                    push(&mut eq, s);
                }
                push(run, eq);
            }
            "form" => self.form(run, c),
            "tdut" => {
                // Ruby text (Hancom: 덧말): keep the main text
                let text = wstr(d, 4).0;
                if !text.is_empty() {
                    let mut t = Element::new("hp:t");
                    add_text(&mut t, &text);
                    push(run, t);
                }
            }
            _ if cid.starts_with('%') => self.field(run, c, &cid),
            _ => {
                let tag = match cid.as_str() {
                    "tcmt" => "hp:hiddenComment",
                    "idxm" => "hp:indexmark",
                    "pgct" => "hp:pageNumCtrl",
                    _ => "hp:unknownCtrl",
                };
                let mut u = Element::new(tag);
                if tag == "hp:unknownCtrl" {
                    u.set("id", cid.as_str());
                }
                push_ctrl(run, u);
            }
        }
    }

    /// First string item of a CTRL_DATA parameter set (bookmark names, field names).
    fn param_string(&self, c: usize) -> String {
        let r = self.r;
        for &x in r.children(c) {
            if r.tag(x) != CTRL_DATA {
                continue;
            }
            let d = r.data(x);
            let mut o = 4usize;
            for _ in 0..i16_(d, 2).max(0) {
                let kind = u16_(d, o + 2);
                o += 4;
                if kind == 1 {
                    return wstr(d, o).0;
                }
                o += if (2..10).contains(&kind) { 4 } else { 2 };
            }
        }
        String::new()
    }

    fn field(&mut self, run: &mut Element, c: usize, cid: &str) {
        let command = wstr(self.r.data(c), 9).0;
        let fid = self.next_field;
        self.next_field += 1;
        self.field_ids.push(fid);
        let mut fb = el!("hp:fieldBegin", "id" => fid, "type" => field_kind(cid));
        let mut params = el!("hp:parameters", "cnt" => 1);
        let mut sp = el!("hp:stringParam", "name" => "Command");
        sp.text = command;
        push(&mut params, sp);
        push(&mut fb, params);
        push_ctrl(run, fb);
    }

    /// LIST_HEADER + its paragraphs -> hp:subList.
    fn sub_list(&mut self, parent: &mut Element, children: &[usize], depth: usize) {
        let r = self.r;
        let mut sub = Element::new("hp:subList");
        let groups = r.lists(children);
        let paras = match groups.first() {
            Some((lh, paras)) => {
                sub.set("vertAlign", pick(VALIGNS, (u32_(r.data(*lh), 4) >> 5) & 3, "TOP"));
                paras.clone()
            }
            None => children.iter().copied().filter(|&x| r.tag(x) == PARA_HEADER).collect(),
        };
        for x in paras {
            self.paragraph(&mut sub, x, depth + 1);
        }
        push(parent, sub);
    }

    fn section_def(&mut self, run: &mut Element, c: usize, depth: usize) {
        let r = self.r;
        let d = r.data(c);
        let ts = u32_(d, 14);
        let mut sp = el!("hp:secPr", "tabStop" => if ts == 0 { 8000 } else { ts }, "outlineShapeIDRef" => u16_(d, 18));
        push(&mut sp, el!("hp:startNum", "page" => u16_(d, 20)));
        // Not in the 5.0 document's field table: byte 36 is non-zero in a section the viewer
        // lays out top to bottom (vertical writing), black-box comparison.
        if u8_(d, 36) != 0 {
            sp.set("textDirection", "VERTICAL");
        }
        if let Some(pd) = r.child(c, PAGE_DEF) {
            let pdd = r.data(pd);
            let v: [i64; 9] = std::array::from_fn(|k| u32_(pdd, 4 * k));
            let attr = u32_(pdd, 36);
            // bit 0: the specification's "0 narrow / 1 wide"; HWPX names the same flag WIDELY
            // (portrait, the usual value) / NARROWLY (landscape).
            let mut pg = el!("hp:pagePr", "landscape" => if attr & 1 != 0 { "NARROWLY" } else { "WIDELY" }, "width" => v[0], "height" => v[1]);
            push(
                &mut pg,
                el!("hp:margin", "left" => v[2], "right" => v[3], "top" => v[4], "bottom" => v[5], "header" => v[6], "footer" => v[7], "gutter" => v[8]),
            );
            push(&mut sp, pg);
        }
        // The master page (Hancom: 바탕쪽): a LIST_HEADER (and its paragraphs) among the definition's own records.
        if let Some((_, paras)) = r.lists(r.children(c)).first() {
            let mut mp = el!("hp:masterPage", "type" => "BOTH");
            let mut sub = Element::new("hp:subList");
            for &q in paras {
                self.paragraph(&mut sub, q, depth + 1);
            }
            push(&mut mp, sub);
            push(&mut sp, mp);
        }
        // Page border and background (Hancom: 쪽 테두리/배경): three records, for both, even and odd pages (only "both" is used).
        let pbs = r.children(c).iter().copied().filter(|&x| r.tag(x) == PAGE_BORDER_FILL);
        for (kind, pb) in ["BOTH", "EVEN", "ODD"].into_iter().zip(pbs) {
            let pd = r.data(pb);
            let a = u32_(pd, 0);
            // Bit 0 set: the border's offsets count from the body (the viewer draws such
            // borders 14pt outside the text area), the reverse of the document's
            // "0 body / 1 paper".
            let mut pbf = el!(
                "hp:pageBorderFill",
                "type" => kind,
                "borderFillIDRef" => u16_(pd, 12),
                "textBorder" => if a & 1 != 0 { "CONTENT" } else { "PAPER" },
                "headerInside" => bit(a, 1),
                "footerInside" => bit(a, 2),
                "fillArea" => pick(&["PAPER", "PAGE", "BORDER"], (a >> 3) & 3, "PAPER"),
            );
            push(&mut pbf, el!("hp:offset", "left" => u16_(pd, 4), "right" => u16_(pd, 6), "top" => u16_(pd, 8), "bottom" => u16_(pd, 10)));
            push(&mut sp, pbf);
        }
        push(run, sp);
    }

    fn caption(&mut self, obj: &mut Element, lh: usize, paras: &[usize], depth: usize) {
        let mut cap = el!("hp:caption", "side" => pick(&["LEFT", "RIGHT", "TOP", "BOTTOM"], u32_(self.r.data(lh), 8) & 3, "BOTTOM"));
        let mut sub = Element::new("hp:subList");
        for &x in paras {
            self.paragraph(&mut sub, x, depth + 1);
        }
        push(&mut cap, sub);
        push(obj, cap);
    }

    fn table(&mut self, run: &mut Element, c: usize, depth: usize) {
        let r = self.r;
        let kids = r.children(c);
        let Some(cut) = kids.iter().position(|&x| r.tag(x) == TABLE) else { return };
        let td = kids.get(cut).map(|&t| r.data(t)).unwrap_or(&[]);
        let (attr, rows, cols, spacing) = (u32_(td, 0), u16_(td, 4), u16_(td, 6), u16_(td, 8));
        let o = 18 + 2 * at(rows);
        let mut tbl = el!(
            "hp:tbl",
            "rowCnt" => rows,
            "colCnt" => cols,
            "cellSpacing" => spacing,
            "borderFillIDRef" => u16_(td, o),
            "repeatHeader" => bit(attr, 2),
            "pageBreak" => pick(&["NONE", "CELL", "NONE"], attr & 3, "NONE"),
        );
        common(&mut tbl, r.data(c));
        push(&mut tbl, el!("hp:inMargin", "left" => u16_(td, 10), "right" => u16_(td, 12), "top" => u16_(td, 14), "bottom" => u16_(td, 16)));
        // as many zones as the record holds (10 bytes each), whatever count it gives
        let zones = at(u16_(td, o + 2)).min(td.len().saturating_sub(o.saturating_add(4)) / 10);
        if zones > 0 {
            let mut zl = Element::new("hp:cellzoneList");
            for k in 0..zones {
                let z = o + 4 + 10 * k;
                push(
                    &mut zl,
                    el!(
                        "hp:cellzone",
                        "startColAddr" => u16_(td, z),
                        "startRowAddr" => u16_(td, z + 2),
                        "endColAddr" => u16_(td, z + 4),
                        "endRowAddr" => u16_(td, z + 6),
                        "borderFillIDRef" => u16_(td, z + 8),
                    ),
                );
            }
            push(&mut tbl, zl);
        }
        if let Some((lh, paras)) = r.lists(kids.get(..cut).unwrap_or(&[])).into_iter().next() {
            self.caption(&mut tbl, lh, &paras, depth);
        }
        let mut rows_el: BTreeMap<i64, Element> = BTreeMap::new();
        for (lh, paras) in r.lists(kids.get(cut + 1..).unwrap_or(&[])) {
            let d = r.data(lh);
            // A cell list header is 8 bytes (paragraph count, a reserved WORD, attributes),
            // then the 26-byte cell properties; the specification gives the header as 6.
            let (col, row, cs, rs) = (u16_(d, 8), u16_(d, 10), u16_(d, 12), u16_(d, 14));
            // Margins are HWPUNIT16 (signed). All four -1 (0xFFFF) means the cell has none of its
            // own and takes the table's, like HWPX's hasMargin="0" (observed, see cleanroom.md).
            let own = !(24..32).step_by(2).all(|o| u16_(d, o) == 0xFFFF);
            let mut tc = el!("hp:tc", "hasMargin" => u8::from(own), "borderFillIDRef" => u16_(d, 32));
            let mut sub = el!("hp:subList", "vertAlign" => pick(VALIGNS, (u32_(d, 4) >> 5) & 3, "TOP"));
            for x in paras {
                self.paragraph(&mut sub, x, depth + 1);
            }
            push(&mut tc, sub);
            push(&mut tc, el!("hp:cellAddr", "colAddr" => col, "rowAddr" => row));
            push(&mut tc, el!("hp:cellSpan", "colSpan" => cs.max(1), "rowSpan" => rs.max(1)));
            push(&mut tc, el!("hp:cellSz", "width" => u32_(d, 16), "height" => u32_(d, 20)));
            // a cell of its own margins: signed values, a negative one (some sides -1) taken as 0;
            // a cell without (all four -1) keeps them as they are, so a save writes them back so
            let side = |o: usize| if own { i16_(d, o).max(0) } else { u16_(d, o) };
            push(&mut tc, el!("hp:cellMargin", "left" => side(24), "right" => side(26), "top" => side(28), "bottom" => side(30)));
            push(rows_el.entry(row).or_insert_with(|| Element::new("hp:tr")), tc);
        }
        // rows in order
        tbl.children.extend(rows_el.into_values());
        push(run, tbl);
    }

    /// 'gso ' control: SHAPE_COMPONENT says which object it is.
    fn shape_object(&mut self, c: usize, depth: usize) -> Option<Element> {
        let r = self.r;
        let sc = r.child(c, SHAPE_COMPONENT)?;
        let mut obj = self.component(sc, Some(r.data(c)), (0.0, 0.0), depth)?;
        if let Some((lh, paras)) = r.lists(r.children(c)).into_iter().next() {
            self.caption(&mut obj, lh, &paras, depth);
        }
        Some(obj)
    }

    fn component(&mut self, sc: usize, common_data: Option<&[u8]>, origin: (f64, f64), depth: usize) -> Option<Element> {
        if depth > MAX_DEPTH {
            return None;
        }
        let r = self.r;
        let d = r.data(sc);
        let tag = shape_tag(&ctrl_id(d))?;
        let mut obj = Element::new(tag);
        // Top-level objects carry the GenShapeObject id twice (8 bytes) before the component
        // properties; grouped children carry it once.
        let o = if common_data.is_some() { 8 } else { 4 };
        let (mut cur_w, mut cur_h) = (u32_(d, o + 20), u32_(d, o + 24));
        let (mut org_w, mut org_h) = (u32_(d, o + 12), u32_(d, o + 16));
        let (mut off_x, mut off_y) = (i32_(d, o), i32_(d, o + 4));
        let mut origin = origin;
        let boxed = if common_data.is_none() { placed(d, o, org_w as f64, org_h as f64) } else { None };
        if let Some((bx, by, bw, bh)) = boxed {
            // A member's own sizes are those before the group was resized; its rendering
            // matrices give where it really is (offsets relative to its parent group). orgSz
            // stays the shape's own drawing space (polygon points); a nested group's members
            // are placed in final units, so its orgSz is its final size.
            off_x = round_even(bx - origin.0);
            off_y = round_even(by - origin.1);
            cur_w = round_even(bw);
            cur_h = round_even(bh);
            if tag == "hp:container" {
                (org_w, org_h) = (cur_w, cur_h);
            }
            origin = (bx, by);
        } else if tag == "hp:container" && common_data.is_some() {
            // The outermost group too: its members are placed in final units, so its drawing
            // space is its current size. Its stored original size (from before it was resized)
            // squeezed them (a group of lines 159 pt tall drawn in 220 pt of space).
            (org_w, org_h) = (cur_w, cur_h);
        }
        match common_data {
            Some(cd) => common(&mut obj, cd),
            None => push(&mut obj, el!("hp:sz", "width" => cur_w, "height" => cur_h)),
        }
        push(&mut obj, el!("hp:curSz", "width" => cur_w, "height" => cur_h));
        push(&mut obj, el!("hp:orgSz", "width" => org_w, "height" => org_h));
        push(&mut obj, el!("hp:offset", "x" => off_x, "y" => off_y));
        let flip = u32_(d, o + 28);
        push(&mut obj, el!("hp:flip", "horizontal" => flip & 1, "vertical" => bit(flip, 1)));
        push(&mut obj, el!("hp:rotationInfo", "angle" => i16_(d, o + 32)));
        if tag != "hp:pic" && tag != "hp:container" {
            outline_and_fill(&mut obj, d, o);
        }
        geometry(r, &mut obj, tag, r.children(sc));
        if tag != "hp:container"
            && let Some((lh, paras)) = r.lists(r.children(sc)).into_iter().next()
        {
            let ld = r.data(lh);
            let mut dt = Element::new("hp:drawText");
            // Text box margins follow the 8-byte list header (left, right, top, bottom).
            push(&mut dt, el!("hp:textMargin", "left" => u16_(ld, 8), "right" => u16_(ld, 10), "top" => u16_(ld, 12), "bottom" => u16_(ld, 14)));
            let mut sub = el!("hp:subList", "vertAlign" => pick(VALIGNS, (u32_(ld, 4) >> 5) & 3, "TOP"));
            for q in paras {
                self.paragraph(&mut sub, q, depth + 1);
            }
            push(&mut dt, sub);
            push(&mut obj, dt);
        }
        for &x in r.children(sc) {
            let t = r.tag(x);
            if t == SC_PICTURE {
                let pd = r.data(x);
                // Picture info (brightness, contrast, effect, BinItem id) follows the border
                // (12), image rectangle (32), crop (16) and inner margins (8).
                push(&mut obj, el!("hc:img", "binaryItemIDRef" => u16_(pd, 71), "bright" => i8_(pd, 68), "contrast" => 0, "effect" => "REAL_PIC"));
                // Border: colour, width, line attributes (type in the low 6 bits); crop: left,
                // top, right, bottom in the picture's own HWPUNIT space.
                if u32_(pd, 8) & 0x3F != 0 && i32_(pd, 4) > 0 {
                    push(
                        &mut obj,
                        el!("hp:lineShape", "color" => color(u32_(pd, 0)), "width" => i32_(pd, 4), "style" => pick(LINE_TYPES, u32_(pd, 8) & 0x3F, "SOLID")),
                    );
                }
                if pd.len() >= 60 {
                    push(
                        &mut obj,
                        el!("hp:imgClip", "left" => i32_(pd, 44), "top" => i32_(pd, 48), "right" => i32_(pd, 52), "bottom" => i32_(pd, 56)),
                    );
                }
            } else if t == SHAPE_COMPONENT
                && tag == "hp:container"
                && let Some(child) = self.component(x, None, origin, depth + 1)
            {
                push(&mut obj, child);
            }
        }
        if tag == "hp:pic" && obj.find("hc:img").is_none() {
            return None;
        }
        Some(obj)
    }

    /// Form objects (Hancom: 양식 개체): the object's properties are one self-describing parameter string
    /// ("Name:wstring:<n>:<text> Value:int:1 ..."), read here by its own syntax.
    fn form(&mut self, run: &mut Element, c: usize) {
        let r = self.r;
        let rec = r.children(c).iter().copied().find(|&x| r.data(x).len() > 12);
        let props = rec.map(|x| form_props(&utf16(r.data(x).get(12..).unwrap_or(&[])))).unwrap_or_default();
        let get = |k: &str, default: &str| props.get(k).cloned().unwrap_or_else(|| default.to_string());
        let name = get("Name", "");
        let kind = name.trim_end_matches(|ch: char| ch.is_ascii_digit());
        let tag = form_tag(kind).unwrap_or(if props.contains_key("Text") && !props.contains_key("Caption") { "hp:edit" } else { "hp:btn" });
        let mut f = el!(
            tag,
            "name" => name,
            "caption" => get("Caption", ""),
            "value" => get("Value", "0"),
            "text" => get("Text", ""),
            "charPrIDRef" => get("CharShapeID", "0"),
            "drawFrame" => get("DrawFrame", "1"),
            "backColor" => get("BackColor", "16777215"),
        );
        common(&mut f, r.data(c));
        push(run, f);
    }
}
