//! Hangul word processor documents, HWP 5.0 (`.hwp`), read-only.
//!
//! Written from Hancom's published "HWP 5.0 file format" document: a compound file (OLE2/CFB)
//! whose `FileHeader` stream names the format and its flags, `DocInfo` holds the fonts, border
//! fills, character shapes and paragraph shapes, `BodyText/Section{n}` the text, and
//! `BinData/BIN{xxxx}.{ext}` the pictures. `DocInfo` and the sections are streams of records
//! (tag, level, size); with the "compressed" flag each stream is raw deflate. Paragraphs carry
//! UTF-16 text in which characters 0–31 are controls: tables, pictures, headers, footers and notes
//! are extended controls whose details follow as `CTRL_HEADER` records, in order.
//!
//! Read: text, character formatting (faces, size, bold, italic, underline, strikeout, colour,
//! shading, super/subscript, letter spacing, width ratio), paragraph formatting (alignment,
//! indents, spacing, line spacing, keep options, borders and shading), tables (merged cells, cell
//! borders and shading, column widths, row heights, cell margins and vertical alignment, nested
//! tables), page size and margins, columns, pictures, headers, footers, page numbers, footnotes
//! and endnotes, and the text of text boxes. Password-protected and distribution (DRM) documents
//! are refused with a clear error. Bounded like every WordCraft reader: stream sizes, inflated
//! sizes, nesting and parts are capped, so hostile input gives an error or a best-effort
//! document, never a panic.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use wordcraft_doc::para::{Float, InlineObject, LINE_BREAK, NBSP, NoteKind, SOFT_HYPHEN};
use wordcraft_doc::props::{CharProps, TabStop, TableProps, VAlign};
use wordcraft_doc::section::Columns;
use wordcraft_doc::{Align, Blocks, Document, PartKind, SectionProps, Table};

use crate::hangul::{self, BorderFill, MAX_DEPTH, MAX_MEDIA, MAX_PARTS, PageNum, ParaBuf, ParaShape, RawCell, RawChar, RawLine, RawPara, Story, pt};

/// What the `FileHeader` stream starts with.
const SIGNATURE: &[u8] = b"HWP Document File";
/// Largest stream read, and largest inflated stream (bytes).
const MAX_STREAM: u64 = 256 << 20;
const MAX_INFLATE: usize = 512 << 20;
/// Most `BodyText/Section{n}` streams read.
const MAX_SECTIONS: usize = 1000;

// Record tags (HWPTAG_BEGIN = 0x10).
const ID_MAPPINGS: u16 = 0x11;
const BIN_DATA: u16 = 0x12;
const FACE_NAME: u16 = 0x13;
const BORDER_FILL: u16 = 0x14;
const CHAR_SHAPE: u16 = 0x15;
const TAB_DEF: u16 = 0x16;
const PARA_SHAPE: u16 = 0x19;
const PARA_HEADER: u16 = 0x42;
const PARA_TEXT: u16 = 0x43;
const PARA_CHAR_SHAPE: u16 = 0x44;
const CTRL_HEADER: u16 = 0x47;
const LIST_HEADER: u16 = 0x48;
const PAGE_DEF: u16 = 0x49;
const TABLE: u16 = 0x4D;
const SHAPE_PICTURE: u16 = 0x55;

/// A control id: four characters, stored as a little-endian number of their big-endian value.
const fn ctrl(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}
const SECD: u32 = ctrl(b"secd");
const COLD: u32 = ctrl(b"cold");
const TBL: u32 = ctrl(b"tbl ");
const GSO: u32 = ctrl(b"gso ");
const HEAD: u32 = ctrl(b"head");
const FOOT: u32 = ctrl(b"foot");
const FN: u32 = ctrl(b"fn  ");
const EN: u32 = ctrl(b"en  ");
const ATNO: u32 = ctrl(b"atno");
const PGNP: u32 = ctrl(b"pgnp");

/// Little-endian reader that yields 0 past the end (the format grows fields at the end of
/// records; missing ones are defaults).
struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(d: &'a [u8]) -> Self {
        Rd { d, p: 0 }
    }
    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let end = self.p.checked_add(N)?;
        let b = self.d.get(self.p..end).and_then(|s| <[u8; N]>::try_from(s).ok());
        self.p = end;
        b
    }
    fn u8(&mut self) -> u8 {
        self.bytes::<1>().map_or(0, |b| b[0])
    }
    fn i8(&mut self) -> i8 {
        self.u8() as i8
    }
    fn u16(&mut self) -> u16 {
        self.bytes().map_or(0, u16::from_le_bytes)
    }
    fn i16(&mut self) -> i16 {
        self.u16() as i16
    }
    fn u32(&mut self) -> u32 {
        self.bytes().map_or(0, u32::from_le_bytes)
    }
    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    fn skip(&mut self, n: usize) {
        self.p = self.p.saturating_add(n);
    }
    fn remaining(&self) -> usize {
        self.d.len().saturating_sub(self.p)
    }
    /// A string: a UTF-16 unit count, then the units.
    fn string(&mut self) -> String {
        let n = usize::from(self.u16());
        let units: Vec<u16> = (0..n.min(self.remaining() / 2)).map(|_| self.u16()).collect();
        String::from_utf16_lossy(&units)
    }
}

/// One record: tag, nesting level, payload.
#[derive(Clone, Copy)]
struct Rec<'a> {
    tag: u16,
    level: u16,
    data: &'a [u8],
}

/// The records of a stream; a truncated record ends the list.
fn records(buf: &[u8]) -> Vec<Rec<'_>> {
    let mut out = Vec::new();
    let mut r = Rd::new(buf);
    while r.remaining() >= 4 {
        let h = r.u32();
        let mut size = (h >> 20) as usize;
        if size == 0xFFF {
            size = r.u32() as usize;
        }
        let Some(data) = r.p.checked_add(size).and_then(|end| buf.get(r.p..end)) else { break };
        out.push(Rec { tag: (h & 0x3FF) as u16, level: ((h >> 10) & 0x3FF) as u16, data });
        r.skip(size);
    }
    out
}

type Comp<'a> = cfb::CompoundFile<Cursor<&'a [u8]>>;

fn stream(comp: &mut Comp<'_>, path: &str) -> Option<Vec<u8>> {
    let s = comp.open_stream(path).ok()?;
    if s.len() > MAX_STREAM {
        return None;
    }
    let mut v = Vec::new();
    s.take(MAX_STREAM).read_to_end(&mut v).ok()?;
    Some(v)
}

/// Inflate a compressed stream (raw deflate; zlib-wrapped streams are accepted too).
fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_INFLATE)
        .or_else(|_| miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, MAX_INFLATE))
        .map_err(|e| format!("HWP: a compressed stream is damaged ({:?})", e.status))
}

/// The `FileHeader` flags of a HWP 5.0 file, or `None` when `bytes` isn't one.
fn header_flags(bytes: &[u8]) -> Option<u32> {
    let mut comp = cfb::CompoundFile::open(Cursor::new(bytes)).ok()?;
    let h = stream(&mut comp, "/FileHeader")?;
    if !h.starts_with(SIGNATURE) {
        return None;
    }
    let mut r = Rd::new(h.get(36..)?);
    Some(r.u32())
}

/// Is this a HWP 5.0 document (a compound file with a Hangul `FileHeader`)?
pub fn is_hwp(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) && header_flags(bytes).is_some()
}

/// What `DocInfo` gives the body.
#[derive(Default)]
struct Info {
    chars: Vec<CharProps>,
    paras: Vec<ParaShape>,
    fills: Vec<BorderFill>,
    bins: Vec<Bin>,
}

impl Info {
    /// Border fill `id` (1-based; 0 = none).
    fn fill(&self, id: u16) -> Option<&BorderFill> {
        self.fills.get(usize::from(id).checked_sub(1)?)
    }
}

/// An embedded picture: its `BinData` stream and whether it is compressed (0 = as the document).
struct Bin {
    storage: u16,
    ext: String,
    compress: u8,
}

fn doc_info(data: &[u8]) -> Info {
    let recs = records(data);
    let mut info = Info::default();
    // Faces come in seven language groups (Hangul, Latin, Hanja, Japanese, other, symbol, user);
    // ID_MAPPINGS says how many each has.
    let mut counts = [0u32; 7];
    let mut faces: [Vec<String>; 2] = Default::default();
    let mut face_no = 0u64;
    let mut tabs: Vec<Vec<TabStop>> = Vec::new();
    for r in &recs {
        let mut rd = Rd::new(r.data);
        match r.tag {
            ID_MAPPINGS => {
                rd.u32();
                for c in &mut counts {
                    *c = rd.u32();
                }
            }
            FACE_NAME => {
                rd.u8();
                let name = rd.string();
                let (hangul, latin) = (u64::from(counts[0]), u64::from(counts[0]) + u64::from(counts[1]));
                if face_no < hangul {
                    faces[0].push(name);
                } else if face_no < latin {
                    faces[1].push(name);
                }
                face_no += 1;
            }
            BORDER_FILL => info.fills.push(border_fill(&mut rd)),
            CHAR_SHAPE => info.chars.push(char_shape(&mut rd, &faces).props()),
            TAB_DEF => {
                rd.u32();
                let n = (rd.u32() as usize).min(rd.remaining() / 8);
                tabs.push(
                    (0..n)
                        .map(|_| {
                            let pos = pt(f64::from(rd.u32()) / 2.0);
                            let (kind, leader) = (rd.u8(), rd.u8());
                            rd.skip(2);
                            hangul::tab(pos, kind, leader)
                        })
                        .collect(),
                );
            }
            PARA_SHAPE => {
                let p = para_shape(&mut rd, &info.fills, &tabs);
                info.paras.push(p.shape());
            }
            BIN_DATA => {
                let attr = rd.u16();
                if attr & 0xF != 0 {
                    let storage = rd.u16();
                    info.bins.push(Bin { storage, ext: rd.string(), compress: ((attr >> 4) & 3) as u8 });
                } else {
                    // A linked file: nothing to show, but it keeps its place in the numbering.
                    info.bins.push(Bin { storage: 0, ext: String::new(), compress: 0 });
                }
            }
            _ => {}
        }
    }
    info
}

fn border_fill(rd: &mut Rd) -> BorderFill {
    rd.u16();
    let mut side = || {
        let code = rd.u8();
        let w = hangul::width_code_pt(rd.u8());
        hangul::BorderFill::side(code, w, hangul::colorref(rd.u32()))
    };
    // Left, right, top, bottom.
    let (left, right, top, bottom) = (side(), side(), side(), side());
    rd.skip(6); // the diagonal
    let kind = rd.u32();
    let fill = if kind & 1 != 0 { hangul::colorref(rd.u32()).filter(|c| *c != wordcraft_doc::Rgb(255, 255, 255)) } else { None };
    BorderFill {
        borders: wordcraft_doc::props::Borders {
            top: Some(top),
            left: Some(left),
            bottom: Some(bottom),
            right: Some(right),
            between: None,
            inside_v: None,
        },
        fill,
    }
}

fn char_shape(rd: &mut Rd, faces: &[Vec<String>; 2]) -> RawChar {
    let mut ids = [0u16; 7];
    for id in &mut ids {
        *id = rd.u16();
    }
    let mut ratio = [0u8; 7];
    for v in &mut ratio {
        *v = rd.u8();
    }
    let mut spacing = [0i8; 7];
    for v in &mut spacing {
        *v = rd.i8();
    }
    rd.skip(14); // relative sizes, offsets
    let height = rd.i32();
    let attr = rd.u32();
    rd.skip(2); // shadow offsets
    let color = hangul::colorref(rd.u32());
    rd.u32(); // underline colour
    let shade = hangul::colorref(rd.u32());
    let face = |g: usize| faces.get(g).and_then(|f| f.get(usize::from(ids[g]))).cloned();
    RawChar {
        hangul: face(0),
        latin: face(1),
        height,
        ratio: i32::from(ratio[0]),
        spacing: i32::from(spacing[0]),
        italic: attr & 1 != 0,
        bold: attr & 2 != 0,
        underline: ((attr >> 2) & 3 != 0).then_some(((attr >> 4) & 0xF) as u8),
        outline: (attr >> 8) & 7 != 0,
        shadow: (attr >> 11) & 3 != 0,
        emboss: attr & (1 << 13) != 0,
        engrave: attr & (1 << 14) != 0,
        sup: attr & (1 << 15) != 0,
        sub: attr & (1 << 16) != 0,
        strike: (attr >> 18) & 7 != 0 && (attr >> 26) & 0xF <= 12,
        color,
        shade,
    }
}

fn para_shape(rd: &mut Rd, fills: &[BorderFill], tabs: &[Vec<TabStop>]) -> RawPara {
    let attr = rd.u32();
    // Margins and spacing are stored at twice their HWPUNIT size.
    let mut half = || rd.i32() as f64 / 2.0;
    let (left, right, indent, before, after) = (half(), half(), half(), half(), half());
    let line = rd.i32();
    let tab_def = rd.u16();
    rd.u16(); // numbering
    let fill = rd.u16();
    RawPara {
        align: ((attr >> 2) & 7) as u8,
        left: pt(left),
        right: pt(right),
        indent: pt(indent),
        before: pt(before),
        after: pt(after),
        line: match attr & 3 {
            0 => RawLine::Percent(line as f32),
            1 => RawLine::Fixed(pt(f64::from(line) / 2.0)),
            2 => RawLine::Between(pt(f64::from(line) / 2.0)),
            _ => RawLine::AtLeast(pt(f64::from(line) / 2.0)),
        },
        widow: attr & (1 << 16) != 0,
        keep_next: attr & (1 << 17) != 0,
        keep_lines: attr & (1 << 18) != 0,
        break_before: attr & (1 << 19) != 0,
        border: usize::from(fill).checked_sub(1).and_then(|i| fills.get(i)).cloned(),
        tabs: tabs.get(usize::from(tab_def)).cloned().unwrap_or_default(),
    }
}

/// Read a HWP 5.0 document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    if bytes.starts_with(SIGNATURE) {
        return Err("HWP: Hangul 97 and older (HWP 3) documents are not supported, only HWP 5.0".into());
    }
    let mut comp = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|e| format!("HWP: not a Hangul document ({e})"))?;
    let header = stream(&mut comp, "/FileHeader").ok_or("HWP: not a Hangul document (no FileHeader)")?;
    if !header.starts_with(SIGNATURE) {
        return Err("HWP: not a Hangul document".into());
    }
    let mut hr = Rd::new(header.get(32..).unwrap_or_default());
    let version = hr.u32();
    let flags = hr.u32();
    if version >> 24 != 5 {
        return Err(format!("HWP: version {} documents are not supported (only HWP 5.0)", version >> 24));
    }
    if flags & 0x02 != 0 {
        return Err("HWP: this document is protected with a password, which WordCraft can't open yet".into());
    }
    if flags & 0x14 != 0 {
        return Err("HWP: this is a distribution (DRM) document, which WordCraft can't open".into());
    }
    let compressed = flags & 1 != 0;
    let read = |comp: &mut Comp<'_>, path: &str| -> Result<Option<Vec<u8>>, String> {
        match stream(comp, path) {
            Some(d) if compressed => inflate(&d).map(Some),
            other => Ok(other),
        }
    };
    let info = doc_info(&read(&mut comp, "/DocInfo")?.ok_or("HWP: the document has no DocInfo stream")?);
    let mut doc = Document::new();
    doc.body.clear();
    let mut sections = Vec::new();
    for n in 0..MAX_SECTIONS {
        let Some(data) = read(&mut comp, &format!("/BodyText/Section{n}"))? else { break };
        let recs = records(&data);
        let mut body = Body {
            info: &info,
            doc: &mut doc,
            comp: &mut comp,
            compressed,
            pics: HashMap::new(),
            room: MAX_MEDIA,
            sec: hangul::default_page(),
            cols_set: false,
            page_num: None,
        };
        let blocks = body.paras(&recs, 0);
        let (mut sec, num) = (body.sec, body.page_num);
        hangul::finish_section(&mut doc, &mut sec, num);
        sections.push((blocks, sec));
    }
    if sections.is_empty() {
        return Err("HWP: the document has no body text".into());
    }
    hangul::assemble(&mut doc, sections);
    doc.ensure_nonempty();
    Ok(doc)
}

/// The state of reading one section.
struct Body<'a, 'c> {
    info: &'a Info,
    doc: &'a mut Document,
    comp: &'a mut Comp<'c>,
    compressed: bool,
    /// Bin id → media key (`None`: unreadable), so each picture loads once.
    pics: HashMap<u16, Option<String>>,
    /// Picture bytes still allowed.
    room: u64,
    sec: SectionProps,
    cols_set: bool,
    page_num: Option<PageNum>,
}

impl Body<'_, '_> {
    /// The paragraphs of a list: the `PARA_HEADER` records at the level of the first one, each
    /// with the records nested below it.
    fn paras(&mut self, recs: &[Rec], depth: usize) -> Blocks {
        let mut story = Story::default();
        if let Some(base) = recs.iter().find(|r| r.tag == PARA_HEADER).map(|r| r.level) {
            let mut i = 0;
            while let Some(r) = recs.get(i) {
                if r.level < base {
                    break;
                }
                let end = recs.get(i + 1..).and_then(|rest| rest.iter().position(|x| x.level <= base)).map_or(recs.len(), |p| i + 1 + p);
                if r.tag == PARA_HEADER && r.level == base {
                    self.paragraph(recs.get(i..end).unwrap_or_default(), depth, &mut story);
                }
                i = end;
            }
        }
        story.finish()
    }

    /// The paragraphs after the first `LIST_HEADER` of a control (a header, note or text box).
    fn list(&mut self, kids: &[Rec], depth: usize) -> Option<Blocks> {
        let i = kids.iter().position(|r| r.tag == LIST_HEADER)?;
        Some(self.paras(kids.get(i + 1..).unwrap_or_default(), depth))
    }

    fn paragraph(&mut self, recs: &[Rec], depth: usize, story: &mut Story) {
        let Some((head, kids)) = recs.split_first() else { return };
        let mut rd = Rd::new(head.data);
        rd.u32(); // characters
        rd.u32(); // control mask
        let shape_id = rd.u16();
        rd.u8(); // style
        let brk = rd.u8();
        let mut shape = self.info.paras.get(usize::from(shape_id)).cloned().unwrap_or_else(|| RawPara::default().shape());
        if brk & 0x04 != 0 {
            shape.props.page_break_before = Some(true);
        }
        let mut pb = ParaBuf::new(shape);
        // The direct children: text, character shape positions and controls (each control with
        // the records nested below it).
        let mut text: &[u8] = &[];
        let mut shapes: Vec<(u32, u32)> = Vec::new();
        let mut ctrls: Vec<&[Rec]> = Vec::new();
        let mut i = 0;
        while let Some(r) = kids.get(i) {
            let end = kids.get(i + 1..).and_then(|rest| rest.iter().position(|x| x.level <= r.level)).map_or(kids.len(), |p| i + 1 + p);
            if r.level == head.level.saturating_add(1) {
                match r.tag {
                    PARA_TEXT if text.is_empty() => text = r.data,
                    PARA_CHAR_SHAPE => {
                        let mut s = Rd::new(r.data);
                        while s.remaining() >= 8 {
                            shapes.push((s.u32(), s.u32()));
                        }
                    }
                    CTRL_HEADER => ctrls.push(kids.get(i..end).unwrap_or_default()),
                    _ => {}
                }
            }
            i = end;
        }
        let units: Vec<u16> = text.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let info = self.info;
        let char_props = |id: u32| info.chars.get(id as usize).cloned().unwrap_or_default();
        let mut ctrls = ctrls.into_iter();
        let mut split = false;
        let mut after: Vec<Blocks> = Vec::new();
        // The character shape in force: the last one starting at or before the position.
        let mut props = char_props(shapes.first().map_or(0, |s| s.1));
        let mut next_shape = 0;
        let mut k = 0;
        while let Some(&u) = units.get(k) {
            let mut changed = None;
            while let Some(&(_, id)) = shapes.get(next_shape).filter(|(pos, _)| *pos as usize <= k) {
                changed = Some(id);
                next_shape += 1;
            }
            if let Some(id) = changed {
                props = char_props(id);
            }
            let mut step = 1;
            match u {
                13 => break,
                10 => pb.push_char(LINE_BREAK, &props),
                9 => {
                    pb.push_char('\t', &props);
                    step = 8;
                }
                24 => pb.push_char(SOFT_HYPHEN, &props),
                30 => pb.push_char(NBSP, &props),
                31 => pb.push_char('\u{2007}', &props),
                25..=29 => pb.push_char(' ', &props),
                1..=3 | 11 | 12 | 14..=18 | 21..=23 => {
                    if let Some(c) = ctrls.next() {
                        split |= self.control(c, &mut pb, story, &props, depth, &mut after);
                    }
                    step = 8;
                }
                4..=8 | 19 | 20 => step = 8,
                0..=31 => {}
                0xD800..=0xDBFF => {
                    let c = units.get(k + 1).and_then(|&lo| char::decode_utf16([u, lo]).next()?.ok());
                    if let Some(c) = c {
                        pb.push_char(c, &props);
                        step = 2;
                    } else {
                        pb.push_char('\u{FFFD}', &props);
                    }
                }
                _ => pb.push_char(char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}'), &props),
            }
            k += step;
        }
        // Controls the text didn't place (a paragraph without text keeps its section definition).
        for c in ctrls {
            split |= self.control(c, &mut pb, story, &props, depth, &mut after);
        }
        pb.set_mark(&props);
        if split && pb.is_empty() {
            story.tail(pb.finish());
        } else {
            story.para(pb.finish());
        }
        for b in after {
            story.extend(b);
        }
    }

    /// Act on one control (its `CTRL_HEADER` record and the records nested below it). True when
    /// a table split the paragraph.
    fn control(&mut self, c: &[Rec], pb: &mut ParaBuf, story: &mut Story, props: &CharProps, depth: usize, after: &mut Vec<Blocks>) -> bool {
        let Some((head, kids)) = c.split_first() else { return false };
        let mut rd = Rd::new(head.data);
        let id = rd.u32();
        let data = head.data.get(4..).unwrap_or_default();
        match id {
            SECD => {
                if let Some(pd) = kids.iter().find(|r| r.tag == PAGE_DEF) {
                    let mut r = Rd::new(pd.data);
                    let (w, h) = (r.u32(), r.u32());
                    let m: [f64; 7] = std::array::from_fn(|_| f64::from(r.u32()));
                    let landscape = r.u32() & 1 != 0;
                    let page = hangul::page(f64::from(w), f64::from(h), landscape, m);
                    self.sec = SectionProps { columns: std::mem::take(&mut self.sec.columns), ..page };
                }
            }
            COLD if !self.cols_set => {
                self.cols_set = true;
                let attr = rd.u16();
                let count = u32::from((attr >> 2) & 0xFF);
                if count > 1 {
                    let gap = if attr & (1 << 12) != 0 { pt(f64::from(rd.i16())).clamp(0.0, 200.0) } else { 18.0 };
                    self.sec.columns = Columns { count: count.min(16), space: gap, ..Default::default() };
                }
            }
            TBL if depth < MAX_DEPTH => {
                if let Some(t) = self.table(data, kids, depth + 1) {
                    if !pb.is_empty() {
                        story.para(pb.finish());
                    }
                    story.table(t);
                    return true;
                }
            }
            GSO if depth < MAX_DEPTH => self.shape(data, kids, pb, props, depth + 1, after),
            HEAD | FOOT if depth < MAX_DEPTH => {
                let even = rd.u32() & 3 == 1;
                if let Some(blocks) = self.list(kids, depth + 1) {
                    hangul::add_header(self.doc, &mut self.sec, id == FOOT, even, blocks);
                }
            }
            FN | EN if depth < MAX_DEPTH => {
                if let Some(blocks) = self.list(kids, depth + 1)
                    && self.doc.parts.len() < MAX_PARTS
                {
                    let (kind, part) = if id == FN { (NoteKind::Footnote, PartKind::Footnote) } else { (NoteKind::Endnote, PartKind::Endnote) };
                    let id = self.doc.add_part(part, blocks);
                    pb.object(InlineObject::NoteRef { kind, id, custom: String::new() }, props);
                }
            }
            // An automatic number: the page number shows as a field.
            ATNO => {
                if rd.u32() & 0xF == 0 {
                    pb.object(hangul::page_field(), props);
                }
            }
            PGNP => {
                let attr = rd.u32();
                rd.skip(6); // user character, prefix, suffix
                let side = char::from_u32(u32::from(rd.u16()));
                self.page_num = Some(PageNum { pos: ((attr >> 8) & 0xF) as u8, side });
            }
            _ => {}
        }
        false
    }

    /// A table: the `TABLE` record (size, cell margins) and one `LIST_HEADER` per cell.
    fn table(&mut self, data: &[u8], kids: &[Rec], depth: usize) -> Option<Table> {
        let common = Rd::new(data).u32();
        let direct = kids.iter().map(|r| r.level).min()?;
        let ti = kids.iter().position(|r| r.tag == TABLE && r.level == direct)?;
        let mut r = Rd::new(kids.get(ti)?.data);
        r.u32();
        r.u16();
        r.u16();
        r.i16();
        let (l, rt, t, b) = (r.i16(), r.i16(), r.i16(), r.i16());
        let mut cells = Vec::new();
        let mut i = ti + 1;
        while let Some(rec) = kids.get(i) {
            let end = kids
                .get(i + 1..)
                .and_then(|rest| rest.iter().position(|x| x.level < direct || (x.level == direct && matches!(x.tag, LIST_HEADER | TABLE))))
                .map_or(kids.len(), |p| i + 1 + p);
            if rec.tag == LIST_HEADER && rec.level == direct {
                cells.push(self.cell(kids.get(i..end).unwrap_or_default(), depth));
            }
            i = end;
        }
        // A table placed apart from the text aligns by its horizontal alignment.
        let align = if common & 1 == 0 {
            match (common >> 10) & 7 {
                1 => Some(Align::Center),
                2 => Some(Align::Right),
                _ => None,
            }
        } else {
            None
        };
        let m = |v: i16| pt(f64::from(v)).clamp(0.0, 100.0);
        let props = TableProps { cell_margins: Some([m(t), m(l), m(b), m(rt)]), fixed: true, align, ..Default::default() };
        hangul::table(cells, props)
    }

    fn cell(&mut self, recs: &[Rec], depth: usize) -> RawCell {
        let mut r = Rd::new(recs.first().map_or(&[][..], |h| h.data));
        r.u16(); // paragraphs
        let attr = r.u32();
        let flags = r.u16();
        let (col, row, cols, rows) = (r.u16(), r.u16(), r.u16(), r.u16());
        let (w, h) = (r.u32(), r.u32());
        let (l, rt, t, b) = (r.i16(), r.i16(), r.i16(), r.i16());
        let fill = r.u16();
        let valign = match (attr >> 21) & 3 {
            1 => VAlign::Center,
            2 => VAlign::Bottom,
            _ => VAlign::Top,
        };
        let m = |v: i16| pt(f64::from(v)).clamp(0.0, 100.0);
        let margins = (flags & 1 != 0).then(|| [m(t), m(l), m(b), m(rt)]);
        RawCell {
            row: usize::from(row),
            col: usize::from(col),
            rows: usize::from(rows),
            cols: usize::from(cols),
            w: pt(f64::from(w)),
            h: pt(f64::from(h)),
            props: hangul::cell_props(self.info.fill(fill), valign, margins),
            blocks: self.paras(recs.get(1..).unwrap_or_default(), depth),
        }
    }

    /// A drawing object: a picture becomes an inline picture; a text box's paragraphs follow
    /// the paragraph.
    fn shape(&mut self, data: &[u8], kids: &[Rec], pb: &mut ParaBuf, props: &CharProps, depth: usize, after: &mut Vec<Blocks>) {
        let mut r = Rd::new(data);
        r.skip(12); // attributes, offsets
        let (w, h) = (pt(f64::from(r.u32())), pt(f64::from(r.u32())));
        if let Some(pic) = kids.iter().find(|r| r.tag == SHAPE_PICTURE) {
            let mut r = Rd::new(pic.data);
            r.skip(71); // border, corners, crop, padding, image effects
            let bin = r.u16();
            if let Some(media) = self.picture(bin) {
                let (w, h) = self.size(&media, w, h);
                pb.object(InlineObject::Image { media, w, h, alt: String::new(), float: Float::default(), crop: [0.0; 4], ole: None }, props);
            }
        } else if let Some(blocks) = self.list(kids, depth) {
            after.push(blocks);
        }
    }

    /// The display size of a picture: the file's, or its pixel size when the file gives none.
    fn size(&self, media: &str, w: f32, h: f32) -> (f32, f32) {
        if w >= 1.0 && h >= 1.0 {
            return (w, h);
        }
        let px = self.doc.media.get(media).and_then(|d| crate::model::image_px(d)).unwrap_or((96, 96));
        ((px.0 as f32 * 0.75).clamp(1.0, 2000.0), (px.1 as f32 * 0.75).clamp(1.0, 2000.0))
    }

    /// Load picture `bin` (1-based) into the document's media, once.
    fn picture(&mut self, bin: u16) -> Option<String> {
        if let Some(hit) = self.pics.get(&bin) {
            return hit.clone();
        }
        let key = self.load(bin);
        self.pics.insert(bin, key.clone());
        key
    }

    fn load(&mut self, bin: u16) -> Option<String> {
        let b = self.info.bins.get(usize::from(bin).checked_sub(1)?)?;
        if b.ext.is_empty() || self.room == 0 {
            return None;
        }
        let name = format!("BIN{:04X}", b.storage);
        let raw = [b.ext.clone(), b.ext.to_ascii_lowercase(), b.ext.to_ascii_uppercase()]
            .iter()
            .find_map(|ext| stream(self.comp, &format!("/BinData/{name}.{ext}")))?;
        let data = if b.compress == 1 || (b.compress == 0 && self.compressed) { inflate(&raw).unwrap_or(raw) } else { raw };
        let ext = crate::model::sniff_image(&data)?;
        self.room = self.room.checked_sub(data.len() as u64)?;
        Some(self.doc.add_media(data, ext))
    }
}
