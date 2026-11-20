//! HWP 5.0 records of paragraphs and tables made by the write-back or the new-document writer:
//! the element tree of an HWPX section (as [`crate::hwp5`] reads HWP into it) back into records.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use crate::hwp5::{CHAR_CONTROLS, CTRL_HEADER, HWP_REC, INLINE_CONTROLS, LIST_HEADER, MEMO_LIST, PARA_CHAR_SHAPE, PARA_HEADER, PARA_TEXT, TABLE};
use crate::xml::Element;

/// High bit of PARA_HEADER's character count: the last paragraph of its list.
pub(crate) const LAST_IN_LIST: u32 = 0x8000_0000;
/// A tab (an inline control of eight WCHARs) when the document has none to copy.
pub(crate) const DEFAULT_TAB: [u16; 8] = [9, 0, 0, 0, 0, 0, 0, 9];
/// A table's control character: code 11 with the control id "tbl " (as the files store it).
pub(crate) const TABLE_CHAR: [u16; 8] = [11, 0x6c20, 0x7462, 0, 0, 0, 0, 11];
/// The control id "tbl " as CTRL_HEADER stores it.
pub(crate) const TABLE_ID: [u8; 4] = [0x20, b'l', b'b', b't'];
/// The section and column definitions' control characters (code 2, ids "secd", "cold").
const SECTION_CHAR: [u16; 8] = [2, 0x6364, 0x7365, 0, 0, 0, 0, 2];
const COLUMN_CHAR: [u16; 8] = [2, 0x6c64, 0x636f, 0, 0, 0, 0, 2];
/// A footnote and an endnote shape as Hancom writes them by default: numbers 1, 2, 3 after the
/// text, a 0.12 mm separator line 5 cm long (observed in the files).
const NOTE_SHAPE: [u8; 28] = [0, 0, 0, 0, 0, 0, 0, 0, 0x29, 0, 1, 0, 0xff, 0xff, 0xff, 0xff, 0x37, 2, 0x37, 2, 0x52, 3, 1, 0, 0, 0, 0, 0];
/// A page border fill with no border (border fill 1), 5 mm offsets, counted from the body.
const PAGE_BORDER: [u8; 14] = [1, 0, 0, 0, 0x89, 5, 0x89, 5, 0x89, 5, 0x89, 5, 1, 0];
/// Header, footer, footnote, endnote and automatic number control characters (codes 16, 17, 18)
/// and the ids their control records store.
const HEADER_CHAR: [u16; 8] = [16, 0x6164, 0x6865, 0, 0, 0, 0, 16];
const FOOTER_CHAR: [u16; 8] = [16, 0x6f74, 0x666f, 0, 0, 0, 0, 16];
const FOOTNOTE_CHAR: [u16; 8] = [17, 0x2020, 0x666e, 0, 0, 0, 0, 17];
const ENDNOTE_CHAR: [u16; 8] = [17, 0x2020, 0x656e, 0, 0, 0, 0, 17];
const AUTONUM_CHAR: [u16; 8] = [18, 0x6e6f, 0x6174, 0, 0, 0, 0, 18];
/// Page hiding's control character (code 21, id "pghd").
const PAGE_HIDING_CHAR: [u16; 8] = [21, 0x6864, 0x7067, 0, 0, 0, 0, 21];
/// A drawing object's control character (code 11, id "gso ").
pub(crate) const OBJECT_CHAR: [u16; 8] = [11, 0x6f20, 0x6773, 0, 0, 0, 0, 11];
const SHAPE_COMPONENT: u32 = 0x10 + 60;
const SC_LINE: u32 = 0x10 + 62;
const SC_RECTANGLE: u32 = 0x10 + 63;
const SC_ELLIPSE: u32 = 0x10 + 64;
const SC_POLYGON: u32 = 0x10 + 66;
const SC_CURVE: u32 = 0x10 + 67;
const SC_PICTURE: u32 = 0x10 + 69;
/// The drawing objects this encoder writes, by their HWPX element: the component id (as the
/// records store it, the specification's 4-character id backwards) and the geometry record.
const DRAWINGS: [(&str, &[u8; 4], u32); 6] = [
    ("hp:rect", b"cer$", SC_RECTANGLE),
    ("hp:ellipse", b"lle$", SC_ELLIPSE),
    ("hp:line", b"nil$", SC_LINE),
    ("hp:polygon", b"lop$", SC_POLYGON),
    ("hp:curve", b"ruc$", SC_CURVE),
    ("hp:container", b"noc$", 0),
];
/// The shadow colour Hancom stores in drawings without a shadow (observed in the files).
const NO_SHADOW_COLOR: u32 = 0x00B2_B2B2;
const PAGE_DEF: u32 = 0x10 + 57;
const FOOTNOTE_SHAPE: u32 = 0x10 + 58;
const PAGE_BORDER_FILL: u32 = 0x10 + 59;
/// Deepest nesting followed.
pub(crate) const MAX_DEPTH: usize = 256;

/// One record: tag, level, data.
pub(crate) type Rec = (u32, u32, Vec<u8>);

pub(crate) fn u16_at(d: &[u8], o: usize) -> u16 {
    d.get(o..o + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes).unwrap_or(0)
}

pub(crate) fn u32_at(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).unwrap_or(0)
}

pub(crate) fn put(d: &mut [u8], o: usize, b: &[u8]) {
    if let Some(t) = d.get_mut(o..o + b.len()) {
        t.copy_from_slice(b);
    }
}

pub(crate) fn put_u16(d: &mut [u8], o: usize, v: u16) {
    put(d, o, &v.to_le_bytes());
}

pub(crate) fn put_u32(d: &mut [u8], o: usize, v: u32) {
    put(d, o, &v.to_le_bytes());
}

/// Set a field of `width` bits at `shift` of the DWORD at `o`.
pub(crate) fn put_bits(d: &mut [u8], o: usize, shift: u32, width: u32, v: u32) {
    let mask = ((1u32 << width) - 1) << shift;
    put_u32(d, o, (u32_at(d, o) & !mask) | ((v << shift) & mask));
}

/// An attribute as a number (0 when missing or not a number).
pub(crate) fn num(e: &Element, name: &str) -> i64 {
    e.get(name).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// The position of `v` in `table` (0 when it is not there).
pub(crate) fn pos_in(table: &[&str], v: &str) -> u32 {
    table.iter().position(|t| *t == v).unwrap_or(0) as u32
}

pub(crate) fn child<'a>(e: &'a Element, tag: &str) -> Option<&'a Element> {
    e.children.iter().find(|c| c.tag == tag)
}

/// What a character of a paragraph's text is, as the editor sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unit {
    Char(char),
    Tab,
    /// A character control: line break (10), non-breaking (30) or fixed-width (31) space.
    Code(u16),
}

/// A paragraph's text runs: each unit with its character shape id.
pub(crate) type Text = Vec<(Unit, u32)>;

/// The units of an hp:t (`None`: an element in it the writer cannot place).
pub(crate) fn t_units(t: &Element, shape: u32, out: &mut Text) -> Option<()> {
    out.extend(t.text.chars().map(|c| (Unit::Char(c), shape)));
    for c in &t.children {
        let unit = match c.tag.as_str() {
            "hp:tab" => Unit::Tab,
            "hp:lineBreak" => Unit::Code(10),
            "hp:nbSpace" => Unit::Code(30),
            "hp:fwSpace" => Unit::Code(31),
            _ => return None,
        };
        out.push((unit, shape));
        out.extend(c.tail.chars().map(|ch| (Unit::Char(ch), shape)));
    }
    Some(())
}

pub(crate) fn run_shape(run: &Element) -> u32 {
    num(run, "charPrIDRef").clamp(0, i64::from(u32::MAX)) as u32
}

/// The text of a paragraph's text runs (`None`: something in them the writer cannot place).
pub(crate) fn element_text(p: &Element) -> Option<Text> {
    let mut out = Vec::new();
    for run in p.children.iter().filter(|c| c.tag == "hp:run") {
        let shape = run_shape(run);
        for t in run.children.iter().filter(|c| c.tag == "hp:t") {
            t_units(t, shape, &mut out)?;
        }
    }
    Some(out)
}

/// The tables a paragraph holds (in its runs), in order.
pub(crate) fn tables_of(p: &Element) -> Vec<&Element> {
    p.children.iter().filter(|c| c.tag == "hp:run").flat_map(|r| r.children.iter().filter(|c| c.tag == "hp:tbl")).collect()
}

pub(crate) fn rec_of(p: &Element) -> Option<usize> {
    p.get(HWP_REC).and_then(|v| v.parse().ok())
}

/// The record of the first source paragraph inside `el` (none: a table the write-back made).
pub(crate) fn first_rec(el: &Element) -> Option<usize> {
    el.iter("hp:p").find_map(rec_of)
}

/// One piece of a paragraph's PARA_TEXT: its WCHARs, where it starts, and whether it is text
/// (characters, tabs, line breaks, special spaces) or a control kept as it is.
pub(crate) struct Tok {
    pub(crate) w: Vec<u16>,
    pub(crate) pos: usize,
    pub(crate) unit: Option<Unit>,
}

impl Tok {
    /// An extended control: one with a CTRL_HEADER (section settings, tables, objects ...).
    pub(crate) fn extended(&self) -> bool {
        self.unit.is_none() && self.w.len() == 8 && self.w.first().is_some_and(|c| *c < 32 && !INLINE_CONTROLS.contains(c))
    }
}

/// PARA_TEXT as pieces, up to the paragraph end mark.
pub(crate) fn tokens(data: &[u8]) -> Vec<Tok> {
    let w: Vec<u16> = data.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = w.get(i) {
        if c == 13 {
            break;
        }
        let (len, unit) = if c >= 32 {
            let lo = w.get(i + 1).copied().filter(|lo| (0xD800..=0xDBFF).contains(&c) && (0xDC00..=0xDFFF).contains(lo));
            match lo {
                Some(lo) => (2, char::from_u32(0x10000 + ((u32::from(c) - 0xD800) << 10) + u32::from(lo) - 0xDC00).map(Unit::Char)),
                None => (1, char::from_u32(u32::from(c)).filter(|_| !(0xD800..=0xDFFF).contains(&c)).map(Unit::Char)),
            }
        } else if c == 9 {
            (8, Some(Unit::Tab))
        } else if CHAR_CONTROLS.contains(&c) {
            (1, matches!(c, 10 | 30 | 31).then_some(Unit::Code(c)))
        } else {
            (8, None)
        };
        let end = (i + len).min(w.len());
        out.push(Tok { w: w.get(i..end).unwrap_or(&[]).to_vec(), pos: i, unit });
        i = end;
    }
    out
}

/// PARA_CHAR_SHAPE as (start, shape id), in order.
pub(crate) fn shape_runs(data: &[u8]) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = data.as_chunks::<8>().0.iter().map(|b| (u32_at(b, 0), u32_at(b, 4))).collect();
    v.sort_by_key(|s| s.0);
    v
}

/// The shape of the last run starting at or before `pos` (the first run's before it), `runs`
/// in order as [`shape_runs`] gives them: found by halving, as a paragraph can have a hundred
/// thousand runs and a picture edit asks for every character's.
pub(crate) fn shape_at(runs: &[(u32, u32)], pos: usize) -> u32 {
    let k = runs.partition_point(|r| r.0 as usize <= pos);
    k.checked_sub(1).and_then(|k| runs.get(k)).or(runs.first()).map(|r| r.1).unwrap_or(0)
}

/// A paragraph's records rebuilt: PARA_HEADER data (counts and mask set, the last-in-list bit
/// left for later), PARA_TEXT (none for an empty paragraph) and PARA_CHAR_SHAPE, and whether
/// shapes were merged. The header counts character shapes in 16 bits: past 65,535 changes of
/// shape, the rest of the text keeps the last one (no character is lost, and the count stays
/// the number of shapes written).
pub(crate) fn paragraph_records(header: &[u8], wchars: &[(u16, u32)]) -> (Vec<u8>, Option<Vec<u8>>, Vec<u8>, bool) {
    let mut text = Vec::with_capacity(wchars.len() * 2);
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut merged = false;
    let mut mask = 0u32;
    let mut i = 0;
    while let Some(&(w, shape)) = wchars.get(i) {
        if runs.last().is_none_or(|r| r.1 != shape) {
            if runs.len() < usize::from(u16::MAX) {
                runs.push((i as u32, shape));
            } else {
                merged = true;
            }
        }
        text.extend_from_slice(&w.to_le_bytes());
        if w < 32 && w != 13 {
            mask |= 1 << w;
        }
        // an eight-WCHAR control is one piece: its closing code is not counted again
        let len = if w < 32 && !CHAR_CONTROLS.contains(&w) { 8 } else { 1 };
        for k in 1..len {
            if let Some(&(w2, _)) = wchars.get(i + k) {
                text.extend_from_slice(&w2.to_le_bytes());
            }
        }
        i += len;
    }
    let mut head = header.to_vec();
    let n = u32::try_from(wchars.len()).unwrap_or(u32::MAX >> 1) & !LAST_IN_LIST;
    put_u32(&mut head, 0, n | (u32_at(header, 0) & LAST_IN_LIST));
    put_u32(&mut head, 4, mask);
    put_u16(&mut head, 12, u16::try_from(runs.len()).unwrap_or(u16::MAX)); // at most u16::MAX (above)
    put_u16(&mut head, 14, 0); // range tags
    put_u16(&mut head, 16, 0); // line layout
    let shapes: Vec<u8> = runs.iter().flat_map(|(p, s)| p.to_le_bytes().into_iter().chain(s.to_le_bytes())).collect();
    (head, (wchars.len() > 1).then_some(text), shapes, merged)
}

/// The units of new text as WCHARs with their shape; tabs take `tab`.
pub(crate) fn units_wchars(text: &Text, tab: &[u16]) -> Vec<(u16, u32)> {
    let mut out = Vec::new();
    for &(u, shape) in text {
        match u {
            Unit::Char(c) => {
                let mut buf = [0u16; 2];
                out.extend(c.encode_utf16(&mut buf).iter().map(|&w| (w, shape)));
            }
            Unit::Tab => out.extend(tab.iter().map(|&w| (w, shape))),
            Unit::Code(c) => out.push((c, shape)),
        }
    }
    out
}

/// Records of the section's first table, the model for new ones.
#[derive(Default)]
pub(crate) struct TableModel {
    pub(crate) ctrl: Option<Vec<u8>>,
    pub(crate) table: Option<Vec<u8>>,
    pub(crate) cell: Option<Vec<u8>>,
    /// The cells of the table being written again, as they were: a cell that still reads the
    /// same is written from these, keeping its fields and objects (see [`OrigCell`]).
    pub(crate) cells: Vec<OrigCell>,
    /// The original table's caption (its list header and paragraphs, levels counted from the
    /// table record's), written back as it was.
    pub(crate) caption: Vec<Rec>,
    /// Original cells holding controls that no cell of the new table took (changed or removed):
    /// their fields and objects are gone. Set by [`Encoder::table_records`].
    pub(crate) lost: std::cell::Cell<usize>,
    /// Cells written again, not from their original records, that had numbers or bullets the
    /// edited table couldn't carry ([`crate::hwpx_out::NUMBERED_CELL`]): those are gone. Set by
    /// [`Encoder::table_records`].
    pub(crate) numbers_lost: std::cell::Cell<usize>,
}

/// A cell of an original table: its paragraphs' records (levels counted from the cell's list
/// header), what they read and how many drawing objects they hold, and whether they hold
/// controls (fields, objects, bookmarks ...) a cell written from the edited document would not.
pub(crate) struct OrigCell {
    pub(crate) recs: Vec<Rec>,
    pub(crate) paras: usize,
    pub(crate) look: CellLook,
    pub(crate) controls: bool,
}

/// What a cell shows that both the original records and the edited section tree tell: the text
/// of each paragraph (those of tables in it too, in order; a cell's empty paragraphs at its end
/// left out, as WordCraft adds one after a table ending a cell), the drawing objects and form
/// controls, and the cells of tables in it. `None`: not known (never matched).
///
/// A paragraph with no text that holds a table gives no text entry ([`counts_text`]): a
/// paragraph holding text and a table reads as two, the text and then the table on its own,
/// so the edited tree has that empty paragraph where the records have none.
pub(crate) type CellLook = Option<CellShows>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellShows {
    pub(crate) text: Vec<Vec<Unit>>,
    pub(crate) objects: usize,
    pub(crate) cells: usize,
    /// The stored image (BinData id) of each picture, in order: a cell whose picture was
    /// replaced doesn't read as it did, though its text and object count do.
    pub(crate) pictures: Vec<String>,
    /// The shown size (width, height HWPUNIT) of each picture and drawing, in order: a cell
    /// whose picture was resized doesn't read as it did either.
    pub(crate) sizes: Vec<(u32, u32)>,
}

/// A unit as both sides of a [`CellLook`] have it: a fixed-width space reads as U+3000, and a
/// table written from the edited document has that character.
pub(crate) fn look_unit(u: Unit) -> Unit {
    if u == Unit::Code(31) { Unit::Char('\u{3000}') } else { u }
}

/// Whether a paragraph's text is an entry of [`CellShows::text`] (see [`CellLook`]).
pub(crate) fn counts_text<T>(text: &[T], holds_table: bool) -> bool {
    !(text.is_empty() && holds_table)
}

/// The look of an edited cell's paragraphs.
pub(crate) fn cell_look(paras: &[&Element]) -> CellLook {
    let mut shows = CellShows::default();
    paras_look(paras, &mut shows, 0)?;
    Some(shows)
}

/// Drop the empty paragraphs at the end of a cell whose text starts at `from` in `text`.
pub(crate) fn trim_cell_end(text: &mut Vec<Vec<Unit>>, from: usize) {
    while text.len() > from && text.last().is_some_and(Vec::is_empty) {
        text.pop();
    }
}

/// The tables of an invisible text box around a table on a text line, as reading makes it
/// and the HWPX writer writes it back: a rectangle with no outline, no fill and no margins,
/// holding nothing but tables. `None` for any other object.
fn boxed_tables(obj: &Element) -> Option<Vec<&Element>> {
    if obj.tag != "hp:rect" || obj.children.iter().any(|c| c.tag == "hc:fillBrush") || child(obj, "hp:lineShape")?.get("style") != Some("NONE") {
        return None;
    }
    let text = child(obj, "hp:drawText")?;
    let margin = child(text, "hp:textMargin")?;
    if ["left", "right", "top", "bottom"].iter().any(|side| margin.get(side).unwrap_or("0") != "0") {
        return None;
    }
    let mut tables = Vec::new();
    for p in child(text, "hp:subList")?.children.iter().filter(|c| c.tag == "hp:p") {
        for c in p.children.iter().filter(|c| c.tag == "hp:run").flat_map(|r| &r.children) {
            match c.tag.as_str() {
                "hp:tbl" => tables.push(c),
                "hp:t" if c.text.is_empty() && c.children.is_empty() => {}
                "hp:ctrl" | "hp:linesegarray" => {}
                _ => return None,
            }
        }
    }
    (!tables.is_empty()).then_some(tables)
}

/// An object's shown size (`hp:curSz`, HWPUNIT), which pictures and drawings have.
fn shown_size(obj: &Element) -> Option<(u32, u32)> {
    let sz = child(obj, "hp:curSz")?;
    let n = |a: &str| sz.get(a).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    Some((n("width"), n("height")))
}

fn paras_look(paras: &[&Element], shows: &mut CellShows, depth: usize) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    let from = shows.text.len();
    for p in paras {
        let mut units = Text::new();
        let mut tables = Vec::new();
        for run in p.children.iter().filter(|c| c.tag == "hp:run") {
            for c in &run.children {
                match c.tag.as_str() {
                    "hp:t" => t_units(c, 0, &mut units)?,
                    "hp:tbl" => tables.push(c),
                    "hp:ctrl" | "hp:linesegarray" | "hp:secPr" => {}
                    "hp:pic" => {
                        shows.objects += 1;
                        shows.pictures.push(c.find("hc:img").and_then(|i| i.get("binaryItemIDRef")).unwrap_or("").to_string());
                        shows.sizes.extend(shown_size(c));
                    }
                    _ => match boxed_tables(c) {
                        // the records hold the table itself
                        Some(inner) => tables.extend(inner),
                        None => {
                            shows.objects += 1;
                            shows.sizes.extend(shown_size(c));
                        }
                    },
                }
            }
        }
        if counts_text(&units, !tables.is_empty()) {
            shows.text.push(units.into_iter().map(|u| look_unit(u.0)).collect());
        }
        for t in tables {
            for tc in t.children.iter().filter(|c| c.tag == "hp:tr").flat_map(|r| r.children.iter().filter(|c| c.tag == "hp:tc")) {
                shows.cells += 1;
                let sub = child(tc, "hp:subList")?;
                let inner: Vec<&Element> = sub.children.iter().filter(|c| c.tag == "hp:p").collect();
                paras_look(&inner, shows, depth + 1)?;
            }
        }
    }
    trim_cell_end(&mut shows.text, from);
    Some(())
}

/// Which original cell each new cell is (a longest common subsequence of their looks, so a cell
/// added or removed shifts none of the others); only cells with controls are worth taking, so
/// only they are matched (empty cells elsewhere would otherwise outnumber them).
///
/// Pictures are compared by `picture`: what a picture's `binaryItemIDRef` shows (its image), as
/// a table written again names even its unchanged pictures anew.
fn match_cells(orig: &[OrigCell], new: &[CellLook], picture: &dyn Fn(&str) -> String) -> Vec<Option<usize>> {
    match_cells_by(orig, new, picture, true)
}

/// [`match_cells`], comparing the objects' sizes or not (`sizes`): a cell that matches only
/// without them holds the same things resized, and is written again with them.
fn match_cells_by(orig: &[OrigCell], new: &[CellLook], picture: &dyn Fn(&str) -> String, sizes: bool) -> Vec<Option<usize>> {
    let mut out = vec![None; new.len()];
    let (n, m) = (orig.len(), new.len());
    let alike = |a: &CellShows, b: &CellShows| {
        (&a.text, a.objects, a.cells) == (&b.text, b.objects, b.cells)
            && (!sizes || a.sizes == b.sizes)
            && a.pictures.len() == b.pictures.len()
            && a.pictures.iter().zip(&b.pictures).all(|(x, y)| picture(x) == picture(y))
    };
    if n.saturating_mul(m) > 4_000_000 {
        return out;
    }
    // Cells that look alike can't be told apart by their look. When the table has a different
    // number of them than before and their records differ (a link to another address under the
    // same text), which one a remaining cell was is unknown: none of them is taken, so a cell is
    // written again from its edited form (and the loss reported) rather than given another
    // cell's fields or objects.
    let key = |s: &CellShows| {
        let pictures: Vec<String> = s.pictures.iter().map(|p| picture(p)).collect();
        let sizes = if sizes { format!("{:?}", s.sizes) } else { String::new() };
        format!("{:?}|{}|{}|{sizes}|{pictures:?}", s.text, s.objects, s.cells)
    };
    let mut groups: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
    for (i, o) in orig.iter().enumerate().filter(|(_, o)| o.controls) {
        if let Some(l) = &o.look {
            groups.entry(key(l)).or_default().push(i);
        }
    }
    let mut new_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for l in new.iter().flatten() {
        *new_count.entry(key(l)).or_insert(0) += 1;
    }
    let unsure: std::collections::HashSet<usize> = groups
        .iter()
        .filter(|(k, g)| {
            g.len() > 1
                && new_count.get(*k).copied().unwrap_or(0) != g.len()
                && g.iter().any(|&i| orig.get(i).map(|o| &o.recs) != g.first().and_then(|&f| orig.get(f)).map(|o| &o.recs))
        })
        .flat_map(|(_, g)| g.iter().copied())
        .collect();
    let same = |i: usize, j: usize| {
        !unsure.contains(&i)
            && orig.get(i).zip(new.get(j)).is_some_and(|(o, l)| o.controls && matches!((&o.look, l), (Some(a), Some(b)) if alike(a, b)))
    };
    let w = m + 1;
    let mut dp = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if same(i, j) { dp[(i + 1) * w + j + 1] + 1 } else { dp[(i + 1) * w + j].max(dp[i * w + j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if same(i, j) {
            if let Some(slot) = out.get_mut(j) {
                *slot = Some(i);
            }
            i += 1;
            j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Writes paragraphs and tables as records.
pub(crate) struct Encoder {
    /// The tab new text copies.
    pub(crate) tab: Vec<u16>,
    /// The model for new tables and the paragraph header new cell paragraphs copy.
    pub(crate) model: TableModel,
    pub(crate) para_model: Vec<u8>,
    /// Objects left out (pictures and shapes this encoder does not write yet, ...).
    pub(crate) left_out: std::cell::Cell<usize>,
    /// Picture files by their HWPX item id: their BinData ids.
    pub(crate) bins: std::collections::HashMap<String, u16>,
    /// What each picture item (`binaryItemIDRef`, the original's and the new ones) holds: a hash
    /// of its image, so an unchanged picture of a table written again is known as the same.
    pub(crate) picture_keys: std::collections::HashMap<String, u64>,
    /// Refuse (`None`) what cannot be written instead of leaving it out: a table written back
    /// into a file is then kept as it was, fields and objects included.
    pub(crate) strict: bool,
    /// The memos of the paragraphs written so far (each a MEMO_LIST record, its list header and
    /// paragraphs, at level 1): they go under the section's last paragraph.
    pub(crate) memos: std::cell::RefCell<Vec<Rec>>,
    /// The ids of the section's memo fields (see [`memo_fields`]): only their ends are memo
    /// ends. The reader numbers every field, hyperlinks included, and a field may end in a later
    /// paragraph than it begins in.
    pub(crate) memo_fields: std::collections::HashSet<String>,
    /// Paragraphs written with their character shapes merged (see [`paragraph_records`]).
    pub(crate) merged_shapes: std::cell::Cell<usize>,
}

/// The ids of the memo fields that begin in `sec`.
pub(crate) fn memo_fields(sec: &Element) -> std::collections::HashSet<String> {
    sec.iter("hp:fieldBegin").filter(|f| f.get("type") == Some("MEMO")).filter_map(|f| f.get("id").map(str::to_string)).collect()
}

/// A memo field's start as the text stores it: code 3 and the id "%%me" (the control record
/// says "%unk"; HWP sample).
const MEMO_BEGIN_CHAR: [u16; 8] = [3, 0x6d65, 0x2525, 0, 0, 0, 0, 3];

/// A memo field's end (code 4, an inline control): "%me", then the memo's number (HWP sample).
fn memo_end_char(n: u16) -> [u16; 8] {
    [4, 0x6d65, 0x0025, 0xff01, 0x00ff, n, 0, 4]
}

impl Encoder {
    /// An object this encoder cannot write: refused when strict, else left out and counted.
    fn cannot(&self) -> Option<()> {
        if self.strict {
            return None;
        }
        self.left_out.set(self.left_out.get() + 1);
        Some(())
    }
}

impl Encoder {
    /// The records of paragraph `p` (made by the write-back), its header modelled on `header`.
    pub(crate) fn paragraph(&self, p: &Element, header: &[u8], level: u32, depth: usize) -> Option<Vec<Rec>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let mut wchars: Vec<(u16, u32)> = Vec::new();
        // The controls, in the order of their characters, with a drawing's records made already.
        // What can't be written is decided here, before its character is added: the reader pairs
        // each control character with the next control record, so a character without its record
        // would shift every later control of the paragraph onto the wrong record.
        let mut objs: Vec<(&Element, Option<Vec<Rec>>)> = Vec::new();
        let mut shape = 0;
        let mut breaks = 0u8;
        for run in p.children.iter().filter(|c| c.tag == "hp:run") {
            shape = run_shape(run);
            for c in &run.children {
                let ctrl = |w: &mut Vec<(u16, u32)>, chars: &[u16; 8]| w.extend(chars.iter().map(|&x| (x, shape)));
                match c.tag.as_str() {
                    "hp:t" => {
                        let mut units = Vec::new();
                        t_units(c, shape, &mut units)?;
                        wchars.extend(units_wchars(&units, &self.tab));
                    }
                    "hp:tbl" => {
                        ctrl(&mut wchars, &TABLE_CHAR);
                        objs.push((c, None));
                    }
                    "hp:secPr" => {
                        ctrl(&mut wchars, &SECTION_CHAR);
                        objs.push((c, None));
                        breaks |= 0x01;
                    }
                    "hp:ctrl" => {
                        for k in &c.children {
                            if k.tag == "hp:fieldEnd"
                                && let Some(id) = k.get("beginIDRef").filter(|id| self.memo_fields.contains(*id))
                                && let Some(n) = self.memo_number(Some(id))
                            {
                                wchars.extend(memo_end_char(n).iter().map(|&x| (x, shape)));
                                continue;
                            }
                            let chars = match k.tag.as_str() {
                                "hp:fieldBegin" if k.get("type") == Some("MEMO") => &MEMO_BEGIN_CHAR,
                                "hp:colPr" => {
                                    breaks |= 0x02;
                                    &COLUMN_CHAR
                                }
                                "hp:header" => &HEADER_CHAR,
                                "hp:footer" => &FOOTER_CHAR,
                                "hp:footNote" => &FOOTNOTE_CHAR,
                                "hp:endNote" => &ENDNOTE_CHAR,
                                // HWP 5.0 automatic numbers have no total-page kind: not written
                                // as a page number
                                "hp:autoNum" if k.get("numType") == Some("TOTAL_PAGE") => {
                                    self.cannot()?;
                                    continue;
                                }
                                "hp:autoNum" => &AUTONUM_CHAR,
                                "hp:pageHiding" => &PAGE_HIDING_CHAR,
                                _ => {
                                    self.cannot()?;
                                    continue;
                                }
                            };
                            ctrl(&mut wchars, chars);
                            objs.push((k, None));
                        }
                    }
                    "hp:pic" if child(c, "hc:img").and_then(|i| i.get("binaryItemIDRef")).is_some_and(|b| self.bins.contains_key(b)) => {
                        ctrl(&mut wchars, &OBJECT_CHAR);
                        objs.push((c, None));
                    }
                    tag if DRAWINGS.iter().any(|d| d.0 == tag) => match self.drawing_records(c, level + 1, depth + 1) {
                        Some(recs) => {
                            ctrl(&mut wchars, &OBJECT_CHAR);
                            objs.push((c, Some(recs)));
                        }
                        None => self.cannot()?,
                    },
                    "hp:linesegarray" => {}
                    _ => self.cannot()?,
                }
            }
        }
        wchars.push((13, shape));
        let mut header = header.to_vec();
        header.resize(header.len().max(22), 0);
        put_u16(&mut header, 8, num(p, "paraPrIDRef").clamp(0, i64::from(u16::MAX)) as u16);
        if let Some(b) = header.get_mut(10) {
            *b = num(p, "styleIDRef").clamp(0, 255) as u8;
        }
        if let Some(b) = header.get_mut(11) {
            // a section's first paragraph starts the section (and its columns)
            *b = breaks | if p.get("pageBreak") == Some("1") { 0x04 } else { 0 } | if p.get("columnBreak") == Some("1") { 0x08 } else { 0 };
        }
        put_u32(&mut header, 18, 0); // instance id
        put_u16(&mut header, 22, 0); // track-change merge
        let (head, text, shapes, merged) = paragraph_records(&header, &wchars);
        if merged {
            self.merged_shapes.set(self.merged_shapes.get() + 1);
        }
        let mut out = vec![(PARA_HEADER, level, head)];
        if let Some(t) = text {
            out.push((PARA_TEXT, level + 1, t));
        }
        out.push((PARA_CHAR_SHAPE, level + 1, shapes));
        for (o, made) in objs {
            if let Some(recs) = made {
                out.extend(recs);
                continue;
            }
            match o.tag.as_str() {
                "hp:tbl" => out.extend(self.table_records(o, level + 1, &self.model, false, depth + 1)?),
                "hp:secPr" => out.extend(section_records(o, level + 1)),
                "hp:colPr" => out.push((CTRL_HEADER, level + 1, column_record(o))),
                "hp:autoNum" => out.push((CTRL_HEADER, level + 1, auto_number(o))),
                "hp:pageHiding" => out.push((CTRL_HEADER, level + 1, page_hiding(o))),
                "hp:pic" => out.extend(self.picture_records(o, level + 1)?),
                "hp:fieldBegin" => out.push((CTRL_HEADER, level + 1, self.memo(o, depth + 1)?)),
                tag => out.extend(self.story_control(o, tag, level + 1, depth + 1)?),
            }
        }
        Some(out)
    }

    /// The number of the memo field `id` names (our new HWPX gives memo fields their number as
    /// their id), when it is one.
    fn memo_number(&self, id: Option<&str>) -> Option<u16> {
        id?.parse::<u16>().ok().filter(|n| *n > 0)
    }

    /// Memo field `f`: its control record ("%unk": attributes, the command naming the memo,
    /// an instance id and the memo's number, as the HWP sample has it); its text is kept for
    /// the section's end (MEMO_LIST with the number, a list header, the paragraphs).
    fn memo(&self, f: &Element, depth: usize) -> Option<Vec<u8>> {
        let n = self.memo_number(f.get("id"))?;
        let command = f.iter("hp:stringParam").find(|p| p.get("name") == Some("Command")).map(|p| p.text.clone()).unwrap_or_default();
        let mut d = b"knu%".to_vec();
        d.extend_from_slice(&0x0000_8001u32.to_le_bytes());
        d.push(0);
        let w: Vec<u16> = command.encode_utf16().take(usize::from(u16::MAX)).collect();
        d.extend_from_slice(&(w.len() as u16).to_le_bytes());
        d.extend(w.iter().flat_map(|c| c.to_le_bytes()));
        d.extend_from_slice(&(0x4E00_0000u32 + u32::from(n)).to_le_bytes());
        d.extend_from_slice(&u32::from(n).to_le_bytes());
        let paras: Vec<&Element> = f.find("hp:subList").map(|s| s.children.iter().filter(|c| c.tag == "hp:p").collect()).unwrap_or_default();
        let mut recs: Vec<Rec> = vec![(MEMO_LIST, 1, u32::from(n).to_le_bytes().to_vec())];
        let mut head = vec![0u8; 16];
        put_u16(&mut head, 0, paras.len().max(1) as u16);
        recs.push((LIST_HEADER, 1, head));
        let model = self.para_model.clone();
        if paras.is_empty() {
            recs.extend(self.paragraph(&Element::new("hp:p"), &model, 1, depth)?);
        }
        for p in paras {
            recs.extend(self.paragraph(p, &model, 1, depth)?);
        }
        self.memos.borrow_mut().extend(recs);
        Some(d)
    }

    /// A picture at `level`: its control (placement), component (sizes, rendering matrices) and
    /// picture record (image rectangle, crop, margins, effects, BinData id).
    pub(crate) fn picture_records(&self, pic: &Element, level: u32) -> Option<Vec<Rec>> {
        let pd = self.picture_data(pic)?;
        let mut ctrl = vec![0u8; 46];
        ctrl[..4].copy_from_slice(b" osg");
        place_object(&mut ctrl, pic, 1);
        let size = |tag: &str| child(pic, tag).map(|e| (num(e, "width").max(0) as u32, num(e, "height").max(0) as u32)).unwrap_or((0, 0));
        let (w, h) = size("hp:sz");
        put_u32(&mut ctrl, 16, w);
        put_u32(&mut ctrl, 20, h);
        put_u32(&mut ctrl, 36, instance(pic));
        let sc = component(pic, b"cip$", true);
        Some(vec![(CTRL_HEADER, level, ctrl), (SHAPE_COMPONENT, level + 1, sc), (SC_PICTURE, level + 2, pd)])
    }

    /// A picture's own record: border (none), image rectangle, crop, inner margins, effects,
    /// BinData id.
    fn picture_data(&self, pic: &Element) -> Option<Vec<u8>> {
        let img = child(pic, "hc:img")?;
        let bin = *self.bins.get(img.get("binaryItemIDRef")?)?;
        // picture: border (none), image rectangle, crop, inner margins, effects, BinData id
        let mut pd = vec![0u8; 94];
        if let Some(r) = child(pic, "hp:imgRect") {
            for (k, pt) in ["hc:pt0", "hc:pt1", "hc:pt2", "hc:pt3"].iter().enumerate() {
                if let Some(p) = child(r, pt) {
                    put_u32(&mut pd, 12 + 8 * k, num(p, "x") as u32);
                    put_u32(&mut pd, 16 + 8 * k, num(p, "y") as u32);
                }
            }
        }
        if let Some(c) = child(pic, "hp:imgClip") {
            for (k, side) in ["left", "top", "right", "bottom"].iter().enumerate() {
                put_u32(&mut pd, 44 + 4 * k, num(c, side) as u32);
            }
        }
        if let Some(m) = child(pic, "hp:inMargin") {
            for (k, side) in ["left", "right", "top", "bottom"].iter().enumerate() {
                put_u16(&mut pd, 60 + 2 * k, num(m, side) as u16);
            }
        }
        if let Some(b) = pd.get_mut(68) {
            *b = num(img, "bright") as u8;
        }
        if let Some(b) = pd.get_mut(69) {
            *b = num(img, "contrast") as u8;
        }
        put_u16(&mut pd, 71, bin);
        put_u32(&mut pd, 74, instance(pic));
        Some(pd)
    }

    /// A drawing object (rectangle or text box, ellipse, line, polygon, curve or group) at
    /// `level`: its control, then its component (see [`Encoder::drawing_component`]).
    pub(crate) fn drawing_records(&self, el: &Element, level: u32, depth: usize) -> Option<Vec<Rec>> {
        let mut ctrl = vec![0u8; 46];
        ctrl[..4].copy_from_slice(b" osg");
        place_object(&mut ctrl, el, 0);
        let sz = child(el, "hp:sz")?;
        put_u32(&mut ctrl, 16, num(sz, "width").max(0) as u32);
        put_u32(&mut ctrl, 20, num(sz, "height").max(0) as u32);
        put_u32(&mut ctrl, 36, instance(el));
        let mut out = vec![(CTRL_HEADER, level, ctrl)];
        out.extend(self.drawing_component(el, level + 1, true, depth)?);
        Some(out)
    }

    /// A drawing's component record at `level` and what hangs under it: its text (a list header
    /// and paragraphs), its geometry record, or a group's members (each a component of its own,
    /// its id once). Line, fill and shadow follow the rendering matrices as the files have them.
    fn drawing_component(&self, el: &Element, level: u32, top: bool, depth: usize) -> Option<Vec<Rec>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let tag = el.tag.as_str();
        if tag == "hp:pic" {
            return Some(vec![(SHAPE_COMPONENT, level, component(el, b"cip$", top)), (SC_PICTURE, level + 1, self.picture_data(el)?)]);
        }
        let &(_, id, geometry) = DRAWINGS.iter().find(|d| d.0 == tag)?;
        let mut sc = component(el, id, top);
        let mut kids: Vec<Rec> = Vec::new();
        if tag == "hp:container" {
            let members: Vec<&Element> = el.children.iter().filter(|c| c.tag == "hp:pic" || DRAWINGS.iter().any(|d| d.0 == c.tag)).collect();
            sc.extend_from_slice(&(members.len() as u16).to_le_bytes());
            for m in &members {
                let mid = if m.tag == "hp:pic" { b"cip$" } else { DRAWINGS.iter().find(|d| d.0 == m.tag)?.1 };
                sc.extend_from_slice(mid);
            }
            sc.extend_from_slice(&instance(el).to_le_bytes());
            for m in members {
                kids.extend(self.drawing_component(m, level + 1, false, depth + 1)?);
            }
            let mut out = vec![(SHAPE_COMPONENT, level, sc)];
            out.extend(kids);
            return Some(out);
        }
        // only solid fills are written: a gradient or picture fill would be written as none, so
        // the drawing is refused (and so reported) instead
        if child(el, "hc:fillBrush").is_some_and(|f| child(f, "hc:gradation").is_some() || child(f, "hc:imgBrush").is_some()) {
            return None;
        }
        sc.extend(line_and_fill(el));
        if let Some(dt) = child(el, "hp:drawText") {
            let sub = child(dt, "hp:subList")?;
            let paras: Vec<&Element> = sub.children.iter().filter(|c| c.tag == "hp:p").collect();
            let mut lh = vec![0u8; 33];
            put_u16(&mut lh, 0, paras.len().max(1) as u16);
            put_bits(&mut lh, 4, 5, 2, pos_in(&["TOP", "CENTER", "BOTTOM"], sub.get("vertAlign").unwrap_or("")));
            if let Some(m) = child(dt, "hp:textMargin") {
                for (k, side) in ["left", "right", "top", "bottom"].iter().enumerate() {
                    put_u16(&mut lh, 8 + 2 * k, num(m, side).max(0) as u16);
                }
            }
            put_u32(&mut lh, 16, child(el, "hp:curSz").map(|s| num(s, "width")).unwrap_or(0).max(0) as u32);
            kids.push((LIST_HEADER, level + 1, lh));
            let para_model = self.para_model.clone();
            if paras.is_empty() {
                kids.extend(self.paragraph(&Element::new("hp:p"), &para_model, level + 1, depth + 1)?);
            }
            for p in paras {
                kids.extend(self.paragraph(p, &para_model, level + 1, depth + 1)?);
            }
        }
        kids.push((geometry, level + 1, geometry_record(el)?));
        let mut out = vec![(SHAPE_COMPONENT, level, sc)];
        out.extend(kids);
        Some(out)
    }

    /// A header, footer, footnote or endnote at `level`: its control, list header and paragraphs.
    fn story_control(&self, o: &Element, tag: &str, level: u32, depth: usize) -> Option<Vec<Rec>> {
        let (id, attr): (&[u8; 4], u32) = match tag {
            "hp:header" | "hp:footer" => {
                let apply = match o.get("applyPageType") {
                    Some("EVEN") => 1,
                    Some("ODD") => 2,
                    _ => 0,
                };
                (if tag == "hp:header" { b"daeh" } else { b"toof" }, apply)
            }
            "hp:footNote" => (b"  nf", 0),
            _ => (b"  ne", 0),
        };
        let mut ctrl = id.to_vec();
        ctrl.extend_from_slice(&attr.to_le_bytes());
        ctrl.extend_from_slice(&[0u8; 4]);
        let sub = child(o, "hp:subList");
        let paras: Vec<&Element> = sub.map(|s| s.children.iter().filter(|c| c.tag == "hp:p").collect()).unwrap_or_default();
        // list header: paragraphs, attributes; a header's or footer's text area
        let mut list = vec![0u8; if tag == "hp:header" || tag == "hp:footer" { 34 } else { 8 }];
        put_u16(&mut list, 0, paras.len().max(1) as u16);
        if list.len() > 8 {
            put_u32(&mut list, 8, sub.map(|s| num(s, "textWidth")).unwrap_or(0).max(0) as u32);
            put_u32(&mut list, 12, sub.map(|s| num(s, "textHeight")).unwrap_or(0).max(0) as u32);
        }
        let mut out = vec![(CTRL_HEADER, level, ctrl), (LIST_HEADER, level + 1, list)];
        if paras.is_empty() {
            out.extend(self.paragraph(&Element::new("hp:p"), &self.para_model, level + 1, depth)?);
        }
        for p in paras {
            out.extend(self.paragraph(p, &self.para_model, level + 1, depth)?);
        }
        Some(out)
    }

    /// The records of table `tbl` (made by the write-back) at `level`: its control, TABLE record
    /// and cells, modelled on `model`. A table in place of another (`keep_place`) keeps that one's
    /// position settings.
    pub(crate) fn table_records(&self, tbl: &Element, level: u32, model: &TableModel, keep_place: bool, depth: usize) -> Option<Vec<Rec>> {
        let sz = child(tbl, "hp:sz")?;
        let mut ctrl = model.ctrl.clone().unwrap_or_default();
        ctrl.resize(ctrl.len().max(46), 0);
        ctrl.get_mut(..4)?.copy_from_slice(&TABLE_ID);
        if !keep_place || model.ctrl.is_none() {
            place_object(&mut ctrl, tbl, 2);
            put_u32(&mut ctrl, 36, 0); // instance id
        }
        put_u32(&mut ctrl, 16, num(sz, "width") as u32);
        put_u32(&mut ctrl, 20, num(sz, "height") as u32);
        let rows: Vec<&Element> = tbl.children.iter().filter(|c| c.tag == "hp:tr").collect();
        // TABLE: attributes, rows, columns, spacing, inner margins, cells per row, border fill, no zones
        // (bit 26 is set in every table of the files)
        let mut attr = model.table.as_deref().map(|t| u32_at(t, 0)).unwrap_or(0x0400_0000);
        attr = (attr & !0x7)
            | match tbl.get("pageBreak") {
                Some("CELL") => 1,
                Some("TABLE") => 2,
                _ => 0,
            }
            | (u32::from(tbl.get("repeatHeader") == Some("1")) << 2);
        let mut table = attr.to_le_bytes().to_vec();
        for v in [num(tbl, "rowCnt"), num(tbl, "colCnt"), num(tbl, "cellSpacing")] {
            table.extend_from_slice(&(v as u16).to_le_bytes());
        }
        let inm = child(tbl, "hp:inMargin");
        for side in ["left", "right", "top", "bottom"] {
            table.extend_from_slice(&(inm.map(|m| num(m, side)).unwrap_or(0) as u16).to_le_bytes());
        }
        for row in &rows {
            table.extend_from_slice(&(row.children.iter().filter(|c| c.tag == "hp:tc").count() as u16).to_le_bytes());
        }
        table.extend_from_slice(&(num(tbl, "borderFillIDRef") as u16).to_le_bytes());
        table.extend_from_slice(&0u16.to_le_bytes());
        let mut out = vec![(CTRL_HEADER, level, ctrl)];
        out.extend(model.caption.iter().map(|(tag, rel, data)| (*tag, level + 1 + rel, data.clone())));
        out.push((TABLE, level + 1, table));
        let para_model = self.para_model.clone();
        let tcs: Vec<&Element> = rows.iter().flat_map(|r| r.children.iter().filter(|c| c.tag == "hp:tc")).collect();
        let subs: Vec<&Element> = tcs.iter().map(|tc| child(tc, "hp:subList")).collect::<Option<_>>()?;
        let cell_paras: Vec<Vec<&Element>> = subs.iter().map(|s| s.children.iter().filter(|c| c.tag == "hp:p").collect()).collect();
        // cells that read as they did take their original records, fields and objects with them
        let looks: Vec<CellLook> = cell_paras.iter().map(|p| cell_look(p)).collect();
        let picture = |id: &str| self.picture_keys.get(id).map_or_else(|| id.to_string(), |h| format!("#{h:x}"));
        let taken = match_cells(&model.cells, &looks, &picture);
        let reused = |j: usize| taken.get(j).copied().flatten().and_then(|i| model.cells.get(i)).filter(|o| o.controls);
        let kept: std::collections::HashSet<usize> = taken.iter().flatten().copied().collect();
        // a cell found again only with its objects resized is written again whole, its objects
        // included: nothing of it is lost
        let resized: std::collections::HashSet<usize> = match_cells_by(&model.cells, &looks, &picture, false).into_iter().flatten().collect();
        model.lost.set(model.cells.iter().enumerate().filter(|(i, o)| o.controls && !kept.contains(i) && !resized.contains(i)).count());
        model.numbers_lost.set(
            (0..tcs.len()).filter(|&j| reused(j).is_none() && tcs.get(j).is_some_and(|tc| tc.get(crate::hwpx_out::NUMBERED_CELL).is_some())).count(),
        );
        for (j, ((tc, sub), paras)) in tcs.iter().zip(&subs).zip(&cell_paras).enumerate() {
            let orig = reused(j);
            let mut cell = model.cell.clone().unwrap_or_default();
            cell.resize(cell.len().max(47), 0);
            put_u16(&mut cell, 0, orig.map_or(paras.len(), |o| o.paras).max(1) as u16);
            put_bits(&mut cell, 4, 5, 2, pos_in(&["TOP", "CENTER", "BOTTOM"], sub.get("vertAlign").unwrap_or("")));
            let at = |tag: &str, name: &str| child(tc, tag).map(|e| num(e, name)).unwrap_or(0);
            put_u16(&mut cell, 8, at("hp:cellAddr", "colAddr") as u16);
            put_u16(&mut cell, 10, at("hp:cellAddr", "rowAddr") as u16);
            put_u16(&mut cell, 12, at("hp:cellSpan", "colSpan").max(1) as u16);
            put_u16(&mut cell, 14, at("hp:cellSpan", "rowSpan").max(1) as u16);
            put_u32(&mut cell, 16, at("hp:cellSz", "width") as u32);
            put_u32(&mut cell, 20, at("hp:cellSz", "height") as u32);
            for (k, side) in ["left", "right", "top", "bottom"].iter().enumerate() {
                put_u16(&mut cell, 24 + 2 * k, at("hp:cellMargin", side) as u16);
            }
            put_u16(&mut cell, 32, num(tc, "borderFillIDRef") as u16);
            put_u32(&mut cell, 34, at("hp:cellSz", "width") as u32); // text width
            out.push((LIST_HEADER, level + 1, cell));
            if let Some(o) = orig {
                out.extend(o.recs.iter().map(|(tag, rel, data)| (*tag, level + 1 + rel, data.clone())));
                continue;
            }
            if paras.is_empty() {
                out.extend(self.paragraph(&Element::new("hp:p"), &para_model, level + 1, depth + 1)?);
            }
            for p in paras {
                out.extend(self.paragraph(p, &para_model, level + 1, depth + 1)?);
            }
        }
        Some(out)
    }
}

/// The records as a stream.
pub(crate) fn encode(recs: &[Rec]) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, level, data) in recs {
        let size = data.len();
        let short = size.min(0xFFF) as u32;
        out.extend_from_slice(&((tag & 0x3FF) | ((level & 0x3FF) << 10) | (short << 20)).to_le_bytes());
        if size >= 0xFFF {
            out.extend_from_slice(&u32::try_from(size).unwrap_or(u32::MAX).to_le_bytes());
        }
        out.extend_from_slice(data);
    }
    out
}

pub(crate) fn set_last_in_list(recs: &mut [Rec]) {
    for j in 0..recs.len() {
        let Some((tag, level)) = recs.get(j).map(|r| (r.0, r.1)) else { continue };
        if tag != PARA_HEADER {
            continue;
        }
        let next = recs.get(j + 1..).unwrap_or(&[]).iter().find(|r| r.1 <= level);
        let last = next.is_none_or(|r| r.1 < level || r.0 != PARA_HEADER);
        if let Some(rec) = recs.get_mut(j) {
            let n = u32_at(&rec.2, 0);
            put_u32(&mut rec.2, 0, if last { n | LAST_IN_LIST } else { n & !LAST_IN_LIST });
        }
    }
}

/// A section definition (hp:secPr) as records: the control, the page, the footnote and endnote
/// shapes and the page borders.
fn section_records(sp: &Element, level: u32) -> Vec<Rec> {
    let mut d = vec![0u8; 47];
    d[..4].copy_from_slice(b"dces");
    put_u16(&mut d, 8, num(sp, "spaceColumns").clamp(0, i64::from(u16::MAX)) as u16);
    put_u32(&mut d, 14, num(sp, "tabStop").clamp(0, i64::from(u32::MAX)) as u32);
    put_u16(&mut d, 18, 1); // outline numbering
    if let Some(n) = child(sp, "hp:startNum") {
        put_u16(&mut d, 20, num(n, "page").clamp(0, i64::from(u16::MAX)) as u16);
    }
    let mut page = vec![0u8; 40];
    if let Some(pg) = child(sp, "hp:pagePr") {
        put_u32(&mut page, 0, num(pg, "width") as u32);
        put_u32(&mut page, 4, num(pg, "height") as u32);
        if let Some(m) = child(pg, "hp:margin") {
            for (k, side) in ["left", "right", "top", "bottom", "header", "footer", "gutter"].iter().enumerate() {
                put_u32(&mut page, 8 + 4 * k, num(m, side).max(0) as u32);
            }
        }
        put_u32(&mut page, 36, u32::from(pg.get("landscape") == Some("NARROWLY")));
    }
    let mut out = vec![(CTRL_HEADER, level, d), (PAGE_DEF, level + 1, page)];
    out.extend([(FOOTNOTE_SHAPE, level + 1, NOTE_SHAPE.to_vec()), (FOOTNOTE_SHAPE, level + 1, NOTE_SHAPE.to_vec())]);
    out.extend((0..3).map(|_| (PAGE_BORDER_FILL, level + 1, PAGE_BORDER.to_vec())));
    out
}

/// A column definition (hp:colPr) as its control record.
fn column_record(col: &Element) -> Vec<u8> {
    let mut d = vec![0u8; 16];
    d[..4].copy_from_slice(b"dloc");
    let count = num(col, "colCount").clamp(1, 255) as u16;
    let same = u16::from(col.get("sameSz") != Some("0"));
    put_u16(&mut d, 4, (count << 2) | (same << 12));
    put_u16(&mut d, 6, num(col, "sameGap").clamp(0, i64::from(u16::MAX)) as u16);
    d
}

/// An automatic number (hp:autoNum: page, footnote, endnote ... number) as its control record:
/// kind, number format and superscript, the number, user, prefix and suffix characters.
/// Page hiding's control record: what the page hides, a bit each (as the reader takes them).
fn page_hiding(h: &Element) -> Vec<u8> {
    let names = ["hideHeader", "hideFooter", "hideMasterPage", "hideBorder", "hideFill", "hidePageNum"];
    let bits = names.iter().enumerate().filter(|(_, n)| matches!(h.get(n), Some("1" | "true"))).fold(0u32, |a, (k, _)| a | (1 << k));
    let mut d = b"dhgp".to_vec();
    d.extend_from_slice(&bits.to_le_bytes());
    d
}

fn auto_number(a: &Element) -> Vec<u8> {
    let mut d = b"onta".to_vec();
    let kind = ["PAGE", "FOOTNOTE", "ENDNOTE", "PICTURE", "TABLE", "EQUATION"].iter().position(|k| Some(*k) == a.get("numType")).unwrap_or(0) as u32;
    let f = child(a, "hp:autoNumFormat");
    let format = f.and_then(|f| f.get("type")).and_then(|t| crate::hwp5::NUM_FORMATS.iter().position(|x| *x == t)).unwrap_or(0) as u32;
    let sup = u32::from(f.and_then(|f| f.get("supscript")) == Some("1"));
    d.extend_from_slice(&(kind | (format << 4) | (sup << 12)).to_le_bytes());
    d.extend_from_slice(&(num(a, "num").clamp(0, i64::from(u16::MAX)) as u16).to_le_bytes());
    for name in ["userChar", "prefixChar", "suffixChar"] {
        let c = f.and_then(|f| f.get(name)).and_then(|v| v.encode_utf16().next()).unwrap_or(0);
        d.extend_from_slice(&c.to_le_bytes());
    }
    d
}

/// An object's placement (hp:pos, textWrap, textFlow, hp:outMargin) in its common properties
/// `ctrl`: anchored as a character or to the paper, page or paragraph, aligned and offset,
/// wrapped; absolute sizes; numbered as `kind` (1 picture, 2 table).
/// An object's instance id (`instid`), as a 4-byte value.
fn instance(el: &Element) -> u32 {
    num(el, "instid").clamp(0, i64::from(u32::MAX)) as u32
}

/// A component record's start: the id (twice for a top-level object, once for a group member),
/// the offset in the group, how deep it is grouped, the version, the original and current sizes,
/// flips, rotation and its centre, then the rendering matrices: the translation and a scale and
/// rotation pair per grouping level (the object's own, then identity for its groups, whose
/// scale this writer puts into the members).
fn component(el: &Element, id: &[u8; 4], top: bool) -> Vec<u8> {
    let size = |tag: &str| child(el, tag).map(|e| (num(e, "width").max(0) as u32, num(e, "height").max(0) as u32)).unwrap_or((0, 0));
    let mut sc = if top { [id.as_slice(), id.as_slice()].concat() } else { id.to_vec() };
    let o = sc.len();
    sc.resize(o + 44, 0);
    if let Some(off) = child(el, "hp:offset") {
        put_u32(&mut sc, o, num(off, "x") as u32);
        put_u32(&mut sc, o + 4, num(off, "y") as u32);
    }
    let grouped = num(el, "groupLevel").clamp(0, 16) as u16;
    put_u16(&mut sc, o + 8, grouped);
    put_u16(&mut sc, o + 10, 1);
    let (ow, oh) = size("hp:orgSz");
    let (cw, ch) = size("hp:curSz");
    for (k, v) in [ow, oh, cw, ch].into_iter().enumerate() {
        put_u32(&mut sc, o + 12 + 4 * k, v);
    }
    let flip = child(el, "hp:flip");
    let rot = child(el, "hp:rotationInfo");
    let flips = u32::from(flip.and_then(|f| f.get("horizontal")) == Some("1")) | (u32::from(flip.and_then(|f| f.get("vertical")) == Some("1")) << 1);
    // bit 26: a picture's image turns with it (rotateimage), as the files have it; bit 24: the
    // drawing holds text (set in every text box of the files; the viewer does not open one
    // without it)
    let turns = if el.tag == "hp:pic" { u32::from(rot.and_then(|r| r.get("rotateimage")) != Some("0")) << 26 } else { 0 };
    let text = u32::from(child(el, "hp:drawText").is_some()) << 24;
    put_u32(&mut sc, o + 28, flips | turns | text);
    put_u16(&mut sc, o + 32, rot.map(|r| num(r, "angle")).unwrap_or(0) as u16);
    put_u32(&mut sc, o + 34, rot.map(|r| num(r, "centerX")).unwrap_or(i64::from(cw / 2)) as u32);
    put_u32(&mut sc, o + 38, rot.map(|r| num(r, "centerY")).unwrap_or(i64::from(ch / 2)) as u32);
    let pairs = grouped + 1;
    put_u16(&mut sc, o + 42, pairs);
    let info = child(el, "hp:renderingInfo");
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let matrix = |tag: &str| -> [f64; 6] {
        let m = info.and_then(|i| child(i, tag));
        std::array::from_fn(|k| {
            m.and_then(|m| m.get(["e1", "e2", "e3", "e4", "e5", "e6"][k])).and_then(|v| v.parse::<f64>().ok()).unwrap_or(identity[k])
        })
    };
    let mut mats = vec![matrix("hc:transMatrix"), matrix("hc:scaMatrix"), matrix("hc:rotMatrix")];
    for _ in 1..pairs {
        mats.extend([identity, identity]);
    }
    for m in mats {
        for v in m {
            sc.extend_from_slice(&v.to_le_bytes());
        }
    }
    sc
}

/// A drawing's line (colour, 4-byte width, attributes, outline style), fill (solid or none) and
/// the shadow fields, after its rendering matrices.
fn line_and_fill(el: &Element) -> Vec<u8> {
    use crate::hwp5::{ARROWS, LINE_TYPES};
    let mut out = Vec::new();
    let line = child(el, "hp:lineShape");
    let a = |name: &str| line.and_then(|l| l.get(name)).unwrap_or("");
    let color = crate::hwp5_docinfo::colorref(a("color")).unwrap_or(0);
    out.extend_from_slice(&color.to_le_bytes());
    out.extend_from_slice(&(line.map(|l| num(l, "width")).unwrap_or(0).max(0) as u32).to_le_bytes());
    let size = |v: &str| {
        pos_in(
            &[
                "SMALL_SMALL",
                "SMALL_MEDIUM",
                "SMALL_LARGE",
                "MEDIUM_SMALL",
                "MEDIUM_MEDIUM",
                "MEDIUM_LARGE",
                "LARGE_SMALL",
                "LARGE_MEDIUM",
                "LARGE_LARGE",
            ],
            v,
        )
    };
    let style = if line.is_none() { 0 } else { pos_in(LINE_TYPES, a("style")) };
    let attr = style
        | (u32::from(a("endCap") != "ROUND") << 6)
        | (pos_in(ARROWS, a("headStyle")) << 10)
        | (pos_in(ARROWS, a("tailStyle")) << 16)
        | (size(a("headSz")) << 22)
        | (size(a("tailSz")) << 26)
        | (u32::from(a("headfill") != "0") << 30)
        | (u32::from(a("tailfill") != "0") << 31);
    out.extend_from_slice(&attr.to_le_bytes());
    out.push(0); // outline style: normal
    match child(el, "hc:fillBrush").and_then(|f| child(f, "hc:winBrush")) {
        Some(b) => {
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&crate::hwp5_docinfo::colorref(b.get("faceColor").unwrap_or("#FFFFFF")).unwrap_or(0x00FF_FFFF).to_le_bytes());
            out.extend_from_slice(&crate::hwp5_docinfo::colorref(b.get("hatchColor").unwrap_or("#000000")).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&u32::MAX.to_le_bytes()); // no hatch
            out.push(0); // alpha
        }
        None => out.extend_from_slice(&0u32.to_le_bytes()),
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // no further fill data
    // shadow: none, its colour, offsets; the instance id; two reserved bytes
    for v in [0, NO_SHADOW_COLOR, 0, 0, instance(el)] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// A drawing's geometry record: the rectangle's corners, the ellipse's centre and axes, the
/// line's ends, the polygon's points, or the curve's points and segment kinds. Points are x, y
/// pairs and counts 4 bytes, with 4 bytes after polygons and curves, as the files have them.
fn geometry_record(el: &Element) -> Option<Vec<u8>> {
    let pt = |tag: &str| child(el, tag).map(|p| (num(p, "x") as i32, num(p, "y") as i32)).unwrap_or((0, 0));
    let mut g = Vec::new();
    let push_pt = |g: &mut Vec<u8>, (x, y): (i32, i32)| {
        g.extend_from_slice(&x.to_le_bytes());
        g.extend_from_slice(&y.to_le_bytes());
    };
    match el.tag.as_str() {
        "hp:rect" => {
            g.push(num(el, "ratio").clamp(0, 50) as u8);
            for tag in ["hc:pt0", "hc:pt1", "hc:pt2", "hc:pt3"] {
                push_pt(&mut g, pt(tag));
            }
        }
        "hp:ellipse" => {
            g.extend_from_slice(&0u32.to_le_bytes());
            for tag in ["hc:center", "hc:ax1", "hc:ax2", "hc:start1", "hc:end1", "hc:start2", "hc:end2"] {
                push_pt(&mut g, pt(tag));
            }
        }
        "hp:line" => {
            push_pt(&mut g, pt("hc:startPt"));
            push_pt(&mut g, pt("hc:endPt"));
            g.extend_from_slice(&u32::from(el.get("isReverseHV") == Some("1")).to_le_bytes());
        }
        "hp:polygon" => {
            let pts: Vec<&Element> = el.children.iter().filter(|c| c.tag == "hc:pt").collect();
            g.extend_from_slice(&(pts.len() as u32).to_le_bytes());
            for p in pts {
                push_pt(&mut g, (num(p, "x") as i32, num(p, "y") as i32));
            }
            g.extend_from_slice(&[0; 4]);
        }
        "hp:curve" => {
            let segs: Vec<&Element> = el.children.iter().filter(|c| c.tag == "hp:seg").collect();
            let first = segs.first()?;
            g.extend_from_slice(&((segs.len() + 1) as u32).to_le_bytes());
            push_pt(&mut g, (num(first, "x1") as i32, num(first, "y1") as i32));
            for s in &segs {
                push_pt(&mut g, (num(s, "x2") as i32, num(s, "y2") as i32));
            }
            g.extend(segs.iter().map(|s| u8::from(s.get("type") == Some("CURVE"))));
            g.extend_from_slice(&[0; 4]);
        }
        _ => return None,
    }
    Some(g)
}

fn place_object(ctrl: &mut [u8], el: &Element, kind: u32) {
    let pos = child(el, "hp:pos");
    let a = |name: &str| pos.and_then(|p| p.get(name)).unwrap_or("");
    put_bits(ctrl, 4, 0, 1, u32::from(a("treatAsChar") == "1"));
    put_bits(ctrl, 4, 2, 1, u32::from(a("affectLSpacing") == "1"));
    put_bits(ctrl, 4, 3, 2, pos_in(&["PAPER", "PAGE", "PARA"], a("vertRelTo")));
    put_bits(ctrl, 4, 5, 3, pos_in(&["TOP", "CENTER", "BOTTOM", "INSIDE", "OUTSIDE"], a("vertAlign")));
    put_bits(ctrl, 4, 8, 2, pos_in(&["PAPER", "PAGE", "COLUMN", "PARA"], a("horzRelTo")));
    put_bits(ctrl, 4, 10, 3, pos_in(&["LEFT", "CENTER", "RIGHT", "INSIDE", "OUTSIDE"], a("horzAlign")));
    put_bits(ctrl, 4, 21, 3, pos_in(&["SQUARE", "TOP_AND_BOTTOM", "BEHIND_TEXT", "IN_FRONT_OF_TEXT"], el.get("textWrap").unwrap_or("")));
    put_bits(ctrl, 4, 24, 2, pos_in(&["BOTH_SIDES", "LEFT_ONLY", "RIGHT_ONLY", "LARGEST_ONLY"], el.get("textFlow").unwrap_or("")));
    // sizes are absolute (width 4, height 2: not relative to the paper), as in Hancom's files
    put_bits(ctrl, 4, 15, 3, 4);
    put_bits(ctrl, 4, 18, 2, 2);
    put_bits(ctrl, 4, 26, 3, kind);
    put_u32(ctrl, 8, pos.map(|p| num(p, "vertOffset")).unwrap_or(0) as u32);
    put_u32(ctrl, 12, pos.map(|p| num(p, "horzOffset")).unwrap_or(0) as u32);
    put_u32(ctrl, 24, num(el, "zOrder") as u32);
    if let Some(m) = child(el, "hp:outMargin") {
        for (k, side) in ["left", "right", "top", "bottom"].iter().enumerate() {
            put_u16(ctrl, 28 + 2 * k, num(m, side) as u16);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shows(text: &str, objects: usize) -> CellShows {
        CellShows { text: vec![text.chars().map(Unit::Char).collect()], objects, cells: 0, pictures: Vec::new(), sizes: Vec::new() }
    }

    fn same_id(id: &str) -> String {
        id.to_string()
    }

    fn orig(text: &str, objects: usize) -> OrigCell {
        OrigCell { recs: Vec::new(), paras: 1, look: Some(shows(text, objects)), controls: true }
    }

    /// A row added between two picture cells takes neither picture: the cells keep their order,
    /// and a cell whose text changed is matched to nothing.
    #[test]
    fn cells_are_matched_in_order_by_what_they_show() {
        let old = [orig("", 1), orig("가", 0), orig("", 1)];
        let new = [Some(shows("", 1)), Some(shows("", 0)), Some(shows("가", 0)), Some(shows("", 1)), Some(shows("나", 0))];
        assert_eq!(match_cells(&old, &new, &same_id), [Some(0), None, Some(1), Some(2), None]);
        let edited = [Some(shows("", 1)), Some(shows("가!", 0)), None];
        assert_eq!(match_cells(&old, &edited, &same_id), [Some(0), None, None]);
    }

    /// Cells without controls are written from the edited document anyway, so they never
    /// outnumber the one cell whose field only its records keep.
    #[test]
    fn only_cells_with_controls_are_matched() {
        let plain = |text: &str| OrigCell { controls: false, ..orig(text, 0) };
        let old = [orig("필드", 0), plain(""), plain("")];
        let new = [Some(shows("", 0)), Some(shows("", 0)), Some(shows("필드", 0))];
        assert_eq!(match_cells(&old, &new, &same_id), [None, None, Some(0)]);
    }

    /// A cell whose picture was replaced doesn't take its original records (which would bring
    /// the old picture back), though its text and object count are the same. Pictures are known
    /// by their image: a table written again names its unchanged pictures anew ("image7" for the
    /// original's "3"), and those still match.
    #[test]
    fn a_cell_with_another_picture_is_not_matched() {
        let with = |pic: &str| CellShows { pictures: vec![pic.to_string()], ..shows("", 1) };
        let old = [OrigCell { look: Some(with("3")), ..orig("", 1) }];
        let image = |id: &str| match id {
            "3" | "image7" => "photo".to_string(),
            "image8" => "another".to_string(),
            _ => id.to_string(),
        };
        assert_eq!(match_cells(&old, &[Some(with("image7"))], &image), [Some(0)]);
        assert_eq!(match_cells(&old, &[Some(with("image8"))], &image), [None]);
    }

    /// The column count is the attribute the HWPX writer and reader use (`colCount`): a new
    /// two-column section stays two columns in a new HWP. Before, `colCnt` read 0, so one.
    #[test]
    fn a_column_definition_keeps_its_count() {
        let col = crate::xml::parse(br#"<hp:colPr type="NEWSPAPER" colCount="2" sameSz="1" sameGap="1134"/>"#, false).unwrap();
        let d = column_record(&col);
        assert_eq!((u16::from_le_bytes([d[4], d[5]]) >> 2) & 0xFF, 2);
    }

    /// HWP 5.0 automatic numbers have no total-page kind: a total-page number is left out and
    /// counted (refused when writing strictly), not written as the current page number.
    #[test]
    fn a_total_page_number_is_not_written_as_a_page_number() {
        let p = crate::xml::parse(
            br#"<hp:p><hp:run><hp:ctrl><hp:autoNum num="1" numType="TOTAL_PAGE"><hp:autoNumFormat type="DIGIT"/></hp:autoNum></hp:ctrl></hp:run></hp:p>"#,
            false,
        )
        .unwrap();
        let enc = |strict: bool| Encoder {
            tab: Vec::new(),
            model: TableModel::default(),
            para_model: vec![0; 24],
            left_out: Default::default(),
            bins: Default::default(),
            picture_keys: Default::default(),
            strict,
            memos: Default::default(),
            memo_fields: Default::default(),
            merged_shapes: Default::default(),
        };
        let loose = enc(false);
        let recs = loose.paragraph(&p, &[0; 24], 0, 0).unwrap();
        assert!(!recs.iter().any(|r| r.0 == CTRL_HEADER && r.2.starts_with(b"onta")), "no automatic number written");
        assert_eq!(loose.left_out.get(), 1);
        assert!(enc(true).paragraph(&p, &[0; 24], 0, 0).is_none());
    }

    fn loose() -> Encoder {
        Encoder {
            tab: Vec::new(),
            model: TableModel::default(),
            para_model: vec![0; 24],
            left_out: Default::default(),
            bins: Default::default(),
            picture_keys: Default::default(),
            strict: false,
            memos: Default::default(),
            memo_fields: Default::default(),
            merged_shapes: Default::default(),
        }
    }

    /// The paragraph's text as WCHARs, and how many control records are directly under it.
    fn text_and_controls(recs: &[Rec]) -> (Vec<u16>, usize) {
        let text = recs
            .iter()
            .find(|r| r.0 == PARA_TEXT)
            .map(|r| r.2.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect())
            .unwrap_or_default();
        (text, recs.iter().filter(|r| r.0 == CTRL_HEADER && r.1 == 1).count())
    }

    /// The extended controls in `text` (each paired with a control record when read).
    fn extended(text: &[u16]) -> usize {
        let mut n = 0;
        let mut i = 0;
        while let Some(&c) = text.get(i) {
            if matches!(c, 1..=3 | 11 | 12 | 14..=18 | 21..=23) {
                n += 1;
            }
            i += if matches!(c, 1..=9 | 11 | 12 | 14..=23) { 8 } else { 1 };
        }
        n
    }

    fn para(xml: &str) -> Element {
        crate::xml::parse(format!("<hp:p><hp:run>{xml}</hp:run></hp:p>").as_bytes(), false).unwrap()
    }

    /// A total-page number left out leaves no character behind: every control character keeps
    /// its record. Before, its character stayed, and the page number after it was read from no
    /// record (every later control one record off).
    #[test]
    fn a_left_out_total_page_number_leaves_no_character() {
        let p = para(r#"<hp:ctrl><hp:autoNum num="1" numType="TOTAL_PAGE"/><hp:autoNum num="1" numType="PAGE"/></hp:ctrl>"#);
        let (text, controls) = text_and_controls(&loose().paragraph(&p, &[0; 24], 0, 0).unwrap());
        assert_eq!((extended(&text), controls), (1, 1));
    }

    /// A drawing that can't be written (no size) leaves no character behind either.
    #[test]
    fn a_left_out_drawing_leaves_no_character() {
        let p = para(r#"<hp:rect id="1"/><hp:ctrl><hp:autoNum num="1" numType="PAGE"/></hp:ctrl>"#);
        let enc = loose();
        let (text, controls) = text_and_controls(&enc.paragraph(&p, &[0; 24], 0, 0).unwrap());
        assert_eq!((extended(&text), controls), (1, 1));
        assert_eq!(enc.left_out.get(), 1);
    }

    /// Only a memo field's end is a memo end: a hyperlink's end isn't written as one (its start
    /// is left out). A memo's end in a later paragraph than its start still is.
    #[test]
    fn only_memo_fields_end_as_memos() {
        let memo_end = |text: &[u16]| text.windows(2).any(|w| w == [4, 0x6d65]);
        let p = para(r#"<hp:ctrl><hp:fieldBegin id="1" type="HYPERLINK"/></hp:ctrl><hp:t>a</hp:t><hp:ctrl><hp:fieldEnd beginIDRef="1"/></hp:ctrl>"#);
        let (text, _) = text_and_controls(&loose().paragraph(&p, &[0; 24], 0, 0).unwrap());
        assert!(!memo_end(&text), "{text:?}");
        let end_only = para(r#"<hp:t>a</hp:t><hp:ctrl><hp:fieldEnd beginIDRef="2"/></hp:ctrl>"#);
        let mut enc = loose();
        enc.memo_fields = memo_fields(
            &crate::xml::parse(br#"<hs:sec><hp:p><hp:run><hp:ctrl><hp:fieldBegin id="2" type="MEMO"/></hp:ctrl></hp:run></hp:p></hs:sec>"#, false)
                .unwrap(),
        );
        let (text, _) = text_and_controls(&enc.paragraph(&end_only, &[0; 24], 0, 0).unwrap());
        assert!(memo_end(&text), "{text:?}");
    }

    /// Page hiding (a title page without its header) is written as its control and record, and
    /// reads back as it was. Before, it was left out and reported as a picture or shape.
    #[test]
    fn page_hiding_is_written() {
        let p = para(
            r#"<hp:ctrl><hp:pageHiding hideHeader="1" hideFooter="0" hideMasterPage="0" hideBorder="0" hideFill="0" hidePageNum="1"/></hp:ctrl><hp:t>a</hp:t>"#,
        );
        let enc = loose();
        let mut recs = enc.paragraph(&p, &[0; 24], 0, 0).unwrap();
        assert_eq!(enc.left_out.get(), 0);
        let (text, controls) = text_and_controls(&recs);
        assert_eq!((extended(&text), controls), (1, 1));
        set_last_in_list(&mut recs);
        let sec = crate::hwp5::section(&encode(&recs));
        let h = sec.iter("hp:pageHiding").next().unwrap_or_else(|| panic!("{}", sec.to_xml("")));
        assert_eq!((h.get("hideHeader"), h.get("hideFooter"), h.get("hidePageNum")), (Some("1"), Some("0"), Some("1")));
    }
    /// The shape at a position is the last run's starting at or before it, the first run's
    /// before any: the same as going through the runs, for every position of a sorted list.
    #[test]
    fn shape_at_finds_the_run_by_halving() {
        let runs = shape_runs(
            &[(0u32, 7u32), (3, 8), (3, 9), (10, 4)]
                .iter()
                .flat_map(|(a, b)| a.to_le_bytes().into_iter().chain(b.to_le_bytes()))
                .collect::<Vec<u8>>(),
        );
        let scan = |pos: usize| runs.iter().take_while(|r| r.0 as usize <= pos).last().or(runs.first()).map(|r| r.1).unwrap_or(0);
        for pos in 0..14 {
            assert_eq!(shape_at(&runs, pos), scan(pos), "at {pos}");
        }
        assert_eq!(shape_at(&[(5, 2)], 0), 2, "before the first run");
        assert_eq!(shape_at(&[], 3), 0);
        let many: Vec<(u32, u32)> = (0..100_000).map(|k| (k, k % 2)).collect();
        assert!((0..100_000usize).all(|pos| shape_at(&many, pos) == (pos % 2) as u32));
    }
    /// A paragraph changing shape more often than the header can count keeps the count equal to
    /// the shapes written: past 65,535 the text keeps the last shape, every character written.
    /// Before, the count said 65,535 and more shapes followed.
    #[test]
    fn shape_counts_fit_the_header() {
        let wchars: Vec<(u16, u32)> = (0..70_000u32).map(|k| (u16::from(b'a'), k % 2)).chain([(13, 0)]).collect();
        let (head, text, shapes, merged) = paragraph_records(&[0; 24], &wchars);
        assert!(merged);
        assert_eq!(usize::from(u16::from_le_bytes([head[12], head[13]])), shapes.len() / 8);
        assert_eq!(text.map(|t| t.len() / 2), Some(70_001), "every character");
        let (head, _, shapes, merged) = paragraph_records(&[0; 24], &wchars[69_990..]);
        assert!(!merged);
        assert_eq!(usize::from(u16::from_le_bytes([head[12], head[13]])), shapes.len() / 8);
    }
    /// Two cells that look alike but hold different records (a link to another address under
    /// the same text), one of them deleted: the remaining cell can't be told to be either, so
    /// neither is taken (it is written again, and the loss reported). Before, it took the first
    /// one's records, the deleted row's link. Twins kept in the same number still match in order.
    #[test]
    fn look_alike_cells_with_other_records_are_not_guessed() {
        let link = |n: u8| OrigCell { recs: vec![(CTRL_HEADER, 1, vec![n])], ..orig("주소", 0) };
        let old = [link(1), link(2)];
        assert_eq!(match_cells(&old, &[Some(shows("주소", 0))], &same_id), [None]);
        assert_eq!(match_cells(&old, &[Some(shows("주소", 0)), Some(shows("주소", 0))], &same_id), [Some(0), Some(1)]);
        // twins with the same records: either is right
        assert_eq!(match_cells(&[link(1), link(1)], &[Some(shows("주소", 0))], &same_id), [Some(0)]);
    }
}
