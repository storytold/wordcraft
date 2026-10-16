//! Legacy Excel 97–2003 workbooks (`.xls`, BIFF8 in a compound file; #778), read from the
//! published [MS-XLS] structure: the `Workbook` stream's globals (BOUNDSHEET sheet list,
//! shared-string table SST with its CONTINUE records, FORMAT and XF number formats, DATEMODE)
//! and each worksheet's cell records (LABELSST, LABEL, RSTRING, NUMBER, RK, MULRK, BOOLERR and
//! FORMULA cached results with their STRING records). Values become the same display strings
//! as `.xlsx` cells (see the parent module). Excel 5.0/95 (BIFF5, a `Book` stream) and
//! password-protected workbooks are refused with a message suggesting `.xlsx`.
//!
//! Every file is hostile: the stream is read up to [`super::MAX_ENTRY`], every record is bounds
//! checked, and rows, columns, cells, strings and sheets are capped as for `.xlsx`.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};

use super::{
    DateKind, Grid, MAX_CELL_CHARS, MAX_CELLS, MAX_COLS, MAX_ENTRY, MAX_SHARED, MAX_SHEETS, MAX_XFS, builtin_date, custom_date, number, serial_date,
};

/// The compound-file signature every `.xls` (and `.doc`) starts with.
pub const MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
const CONTINUE: u16 = 0x003C;
const FILEPASS: u16 = 0x002F;
const DATEMODE: u16 = 0x0022;
const FORMAT: u16 = 0x041E;
const XF: u16 = 0x00E0;
const BOUNDSHEET: u16 = 0x0085;
const SST: u16 = 0x00FC;
const LABELSST: u16 = 0x00FD;
const LABEL: u16 = 0x0204;
const RSTRING: u16 = 0x00D6;
const NUMBER: u16 = 0x0203;
const RK: u16 = 0x027E;
const MULRK: u16 = 0x00BD;
const BOOLERR: u16 = 0x0205;
const FORMULA: u16 = 0x0006;
const STRING: u16 = 0x0207;

/// Whether `bytes` is a compound file holding an Excel workbook stream.
pub fn is_xls(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&MAGIC) {
        return false;
    }
    let Ok(comp) = cfb::CompoundFile::open(Cursor::new(bytes)) else { return false };
    comp.is_stream("/Workbook") || comp.is_stream("/Book")
}

/// The worksheet names, in workbook order.
pub fn sheet_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    Ok(globals(&workbook_stream(bytes)?)?.sheets.into_iter().map(|s| s.0).collect())
}

/// One worksheet's non-blank rows, with the workbook's worksheet names.
pub fn read_rows(bytes: &[u8], pick: super::Pick<'_>) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let stream = workbook_stream(bytes)?;
    let g = globals(&stream)?;
    let names: Vec<String> = g.sheets.iter().map(|s| s.0.clone()).collect();
    let i = super::resolve(&names, pick)?;
    let offset = g.sheets.get(i).map(|s| s.1).ok_or("no such sheet")?;
    let rows = sheet_rows(&stream, offset, &g)?;
    Ok((names, rows))
}

fn workbook_stream(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut comp = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|e| format!("not a valid Excel 97–2003 workbook ({e})"))?;
    if !comp.is_stream("/Workbook") {
        if comp.is_stream("/Book") {
            return Err("this is an Excel 5.0/95 workbook, which isn't supported; save it as .xlsx (Excel Workbook) and select it again".into());
        }
        return Err("not an Excel workbook (no Workbook stream)".into());
    }
    let mut s = comp.open_stream("/Workbook").map_err(|e| format!("the Workbook stream can't be read ({e})"))?;
    let mut v = Vec::new();
    s.by_ref().take(MAX_ENTRY + 1).read_to_end(&mut v).map_err(|e| format!("the Workbook stream can't be read ({e})"))?;
    if v.len() as u64 > MAX_ENTRY {
        return Err("the workbook is too large".into());
    }
    Ok(v)
}

/// A record: its type, its data and where the next one starts.
fn record(stream: &[u8], pos: usize) -> Option<(u16, &[u8], usize)> {
    let head = stream.get(pos..pos.checked_add(4)?)?;
    let kind = u16::from_le_bytes([head[0], head[1]]);
    let len = usize::from(u16::from_le_bytes([head[2], head[3]]));
    let start = pos + 4;
    let end = start.checked_add(len)?;
    // A truncated last record keeps what is there.
    let data = stream.get(start..end.min(stream.len()))?;
    Some((kind, data, end))
}

fn u16_at(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*d.get(i)?, *d.get(i + 1)?]))
}

fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*d.get(i)?, *d.get(i + 1)?, *d.get(i + 2)?, *d.get(i + 3)?]))
}

fn f64_at(d: &[u8], i: usize) -> Option<f64> {
    let b = d.get(i..i + 8)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(b);
    Some(f64::from_le_bytes(a))
}

/// Reads across a record and the CONTINUE records after it; a string's characters that cross
/// into a CONTINUE record carry a fresh option byte there ([MS-XLS] 2.5.293).
struct Pieces<'a> {
    parts: Vec<&'a [u8]>,
    part: usize,
    at: usize,
}

impl<'a> Pieces<'a> {
    fn new(parts: Vec<&'a [u8]>) -> Self {
        Pieces { parts, part: 0, at: 0 }
    }

    /// Move past exhausted parts; false at the end.
    fn settle(&mut self) -> bool {
        while let Some(p) = self.parts.get(self.part) {
            if self.at < p.len() {
                return true;
            }
            self.part += 1;
            self.at = 0;
        }
        false
    }

    fn byte(&mut self) -> Option<u8> {
        if !self.settle() {
            return None;
        }
        let b = *self.parts.get(self.part)?.get(self.at)?;
        self.at += 1;
        Some(b)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes([self.byte()?, self.byte()?]))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes([self.byte()?, self.byte()?, self.byte()?, self.byte()?]))
    }

    fn skip(&mut self, mut n: usize) -> Option<()> {
        while n > 0 {
            if !self.settle() {
                return None;
            }
            let p = self.parts.get(self.part)?;
            let step = n.min(p.len() - self.at);
            self.at += step;
            n -= step;
        }
        Some(())
    }

    /// `cch` characters, one byte each (`high` false: Latin-1) or UTF-16LE; at a part boundary
    /// a new option byte says which for the rest.
    fn chars(&mut self, cch: usize, mut high: bool) -> Option<String> {
        let mut units: Vec<u16> = Vec::with_capacity(cch.min(MAX_CELL_CHARS));
        let mut left = cch;
        while left > 0 {
            let at_end = self.parts.get(self.part).is_none_or(|p| self.at >= p.len());
            if at_end {
                if !self.settle() {
                    break;
                }
                // The first byte of the CONTINUE record repeats the high-byte flag.
                if self.at == 0 {
                    high = self.byte()? & 1 != 0;
                    continue;
                }
            }
            let u = if high { self.u16()? } else { u16::from(self.byte()?) };
            if units.len() < MAX_CELL_CHARS {
                units.push(u);
            }
            left -= 1;
        }
        Some(String::from_utf16_lossy(&units))
    }

    /// An XLUnicodeRichExtendedString (SST entries).
    fn rich_string(&mut self) -> Option<String> {
        let cch = usize::from(self.u16()?);
        let flags = self.byte()?;
        let runs = if flags & 0x08 != 0 { usize::from(self.u16()?) } else { 0 };
        let ext = if flags & 0x04 != 0 { self.u32()? as usize } else { 0 };
        let s = self.chars(cch, flags & 1 != 0)?;
        self.skip(runs.saturating_mul(4))?;
        self.skip(ext)?;
        Some(s)
    }
}

/// An XLUnicodeString (u16 count) inside one record.
fn unicode_string(d: &[u8], at: usize) -> Option<String> {
    let cch = usize::from(u16_at(d, at)?);
    let flags = *d.get(at + 2)?;
    let mut p = Pieces::new(vec![d.get(at + 3..)?]);
    p.chars(cch, flags & 1 != 0)
}

struct Globals {
    /// Worksheet names with the stream offsets of their BOF records.
    sheets: Vec<(String, usize)>,
    sst: Vec<String>,
    /// Each XF's date kind, by XF index.
    xfs: Vec<Option<DateKind>>,
    date1904: bool,
}

fn globals(stream: &[u8]) -> Result<Globals, String> {
    let Some((BOF, bof, mut pos)) = record(stream, 0) else { return Err("not an Excel 97–2003 workbook (no BIFF8 BOF record)".into()) };
    if u16_at(bof, 0) != Some(0x0600) {
        return Err("this Excel workbook is older than Excel 97 and isn't supported; save it as .xlsx and select it again".into());
    }
    let mut g = Globals { sheets: Vec::new(), sst: Vec::new(), xfs: Vec::new(), date1904: false };
    let mut formats: HashMap<u16, Option<DateKind>> = HashMap::new();
    let mut xf_formats: Vec<u16> = Vec::new();
    while let Some((kind, data, next)) = record(stream, pos) {
        match kind {
            EOF => break,
            FILEPASS => {
                return Err("the workbook is password-protected; remove the password in Excel (or save it as .xlsx) and select it again".into());
            }
            DATEMODE => g.date1904 = u16_at(data, 0) == Some(1),
            FORMAT if formats.len() < MAX_XFS => {
                if let (Some(id), Some(code)) = (u16_at(data, 0), unicode_string(data, 2)) {
                    formats.insert(id, custom_date(&code));
                }
            }
            XF if xf_formats.len() < MAX_XFS => xf_formats.push(u16_at(data, 2).unwrap_or(0)),
            BOUNDSHEET if g.sheets.len() < MAX_SHEETS => {
                // Worksheets only (dt 0); charts, macro and VB module sheets have no cells.
                if let (Some(off), Some(0)) = (u32_at(data, 0), data.get(5).copied()) {
                    let cch = usize::from(data.get(6).copied().unwrap_or(0));
                    let high = data.get(7).is_some_and(|f| f & 1 != 0);
                    let name = data.get(8..).and_then(|rest| Pieces::new(vec![rest]).chars(cch, high)).unwrap_or_default();
                    g.sheets.push((name, off as usize));
                }
            }
            SST => {
                let mut parts = vec![data.get(8..).unwrap_or(&[])];
                let mut p = next;
                while let Some((CONTINUE, d, n)) = record(stream, p) {
                    parts.push(d);
                    p = n;
                }
                let unique = u32_at(data, 4).unwrap_or(0) as usize;
                let mut r = Pieces::new(parts);
                while g.sst.len() < unique.min(MAX_SHARED) {
                    match r.rich_string() {
                        Some(s) => g.sst.push(s),
                        None => break,
                    }
                }
                pos = p;
                continue;
            }
            _ => {}
        }
        pos = next;
    }
    g.xfs = xf_formats.into_iter().map(|id| formats.get(&id).copied().unwrap_or_else(|| builtin_date(u32::from(id)))).collect();
    Ok(g)
}

/// An RK number ([MS-XLS] 2.5.217): a 30-bit integer or the top of a double, maybe ÷ 100.
fn rk(v: u32) -> f64 {
    let x = if v & 2 != 0 { f64::from((v as i32) >> 2) } else { f64::from_bits(u64::from(v & 0xFFFF_FFFC) << 32) };
    if v & 1 != 0 { x / 100.0 } else { x }
}

fn error_text(code: u8) -> &'static str {
    match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        _ => "#N/A",
    }
}

fn sheet_rows(stream: &[u8], offset: usize, g: &Globals) -> Result<Vec<Vec<String>>, String> {
    let Some((BOF, _, mut pos)) = record(stream, offset) else { return Err("the sheet can't be found in the workbook".into()) };
    let mut cells: BTreeMap<u16, BTreeMap<u16, String>> = BTreeMap::new();
    let mut count = 0usize;
    let num = |x: f64, xf: u16| match g.xfs.get(usize::from(xf)).copied().flatten() {
        Some(kind) => serial_date(x, g.date1904, kind).unwrap_or_else(|| number(x)),
        None => number(x),
    };
    let mut put = |row: u16, col: u16, v: String, count: &mut usize| {
        if usize::from(col) >= MAX_COLS || *count >= MAX_CELLS || v.is_empty() {
            return;
        }
        if cells.entry(row).or_default().insert(col, v).is_none() {
            *count += 1;
        }
    };
    // A FORMULA with a string result waits for the STRING record after it.
    let mut pending: Option<(u16, u16)> = None;
    let mut depth = 1usize;
    while let Some((kind, d, next)) = record(stream, pos) {
        pos = next;
        if kind == BOF {
            depth = depth.saturating_add(1);
            continue;
        }
        if kind == EOF {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
            continue;
        }
        if depth > 1 {
            continue;
        }
        if kind == STRING {
            let mut parts = vec![d];
            let mut p = pos;
            while let Some((CONTINUE, cd, n)) = record(stream, p) {
                parts.push(cd);
                p = n;
            }
            pos = p;
            if let Some((r, c)) = pending.take() {
                let cch = usize::from(u16_at(parts[0], 0).unwrap_or(0));
                let high = parts[0].get(2).is_some_and(|f| f & 1 != 0);
                if let Some(s) = parts[0].get(3..).and_then(|first| {
                    let mut all_parts = vec![first];
                    all_parts.extend_from_slice(&parts[1..]);
                    Pieces::new(all_parts).chars(cch, high)
                }) {
                    put(r, c, s, &mut count);
                }
            }
            continue;
        }
        pending = None;
        let (Some(row), Some(col), Some(xf)) = (u16_at(d, 0), u16_at(d, 2), u16_at(d, 4)) else { continue };
        match kind {
            LABELSST => {
                if let Some(s) = u32_at(d, 6).and_then(|i| g.sst.get(i as usize)) {
                    put(row, col, s.clone(), &mut count);
                }
            }
            LABEL | RSTRING => {
                if let Some(s) = unicode_string(d, 6) {
                    put(row, col, s, &mut count);
                }
            }
            NUMBER => {
                if let Some(x) = f64_at(d, 6) {
                    put(row, col, num(x, xf), &mut count);
                }
            }
            RK => {
                if let Some(v) = u32_at(d, 6) {
                    put(row, col, num(rk(v), xf), &mut count);
                }
            }
            MULRK => {
                // rw, colFirst, then (ixfe, rk) pairs, then colLast.
                let n = d.len().saturating_sub(6) / 6;
                for k in 0..n {
                    let at = 4 + k * 6;
                    if let (Some(x), Some(v)) = (u16_at(d, at), u32_at(d, at + 2)) {
                        put(row, col.saturating_add(k as u16), num(rk(v), x), &mut count);
                    }
                }
            }
            BOOLERR => {
                let (v, err) = (d.get(6).copied().unwrap_or(0), d.get(7).copied().unwrap_or(0));
                let s = if err != 0 {
                    error_text(v).to_string()
                } else if v != 0 {
                    "TRUE".into()
                } else {
                    "FALSE".into()
                };
                put(row, col, s, &mut count);
            }
            FORMULA => {
                if d.get(12..14) == Some(&[0xFF, 0xFF]) {
                    match d.get(6).copied() {
                        Some(0) => pending = Some((row, col)),
                        Some(1) => put(row, col, if d.get(8).is_some_and(|b| *b != 0) { "TRUE".into() } else { "FALSE".into() }, &mut count),
                        Some(2) => put(row, col, error_text(d.get(8).copied().unwrap_or(0x2A)).to_string(), &mut count),
                        _ => {}
                    }
                } else if let Some(x) = f64_at(d, 6) {
                    put(row, col, num(x, xf), &mut count);
                }
            }
            _ => {}
        }
    }
    let mut grid = Grid::default();
    for (_, r) in cells {
        let width = r.keys().next_back().map_or(0, |c| usize::from(*c) + 1);
        let mut row = vec![String::new(); width];
        for (c, v) in r {
            if let Some(slot) = row.get_mut(usize::from(c)) {
                *slot = v;
            }
        }
        grid.push(row, 1);
    }
    Ok(grid.rows)
}

/// A minimal BIFF8 workbook (tests and fixtures): one worksheet per `(name, rows)`; cells that
/// read as numbers are NUMBER records, a cell `date:<serial>` is a NUMBER with a date format,
/// everything else goes through the shared-string table (as UTF-16 when it isn't Latin-1).
#[doc(hidden)]
pub fn build(sheets: &[(&str, Vec<Vec<String>>)]) -> Result<Vec<u8>, String> {
    fn rec(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(data.len().min(usize::from(u16::MAX)) as u16).to_le_bytes());
        out.extend_from_slice(data);
    }
    fn chars(out: &mut Vec<u8>, s: &str) {
        let wide = s.chars().any(|c| u32::from(c) > 0xFF);
        out.push(u8::from(wide));
        if wide {
            for u in s.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
        } else {
            out.extend(s.chars().map(|c| u32::from(c) as u8));
        }
    }
    let bof = |dt: u16| {
        let mut d = vec![0x00, 0x06];
        d.extend_from_slice(&dt.to_le_bytes());
        d.extend_from_slice(&[0; 12]);
        d
    };
    let mut sst: Vec<String> = Vec::new();
    let mut out = Vec::new();
    rec(&mut out, BOF, &bof(0x0005));
    for ifmt in [0u16, 14] {
        let mut xf = vec![0u8; 20];
        xf[2..4].copy_from_slice(&ifmt.to_le_bytes());
        rec(&mut out, XF, &xf);
    }
    let mut patches = Vec::new();
    for (name, _) in sheets {
        patches.push(out.len() + 4);
        let mut d = vec![0, 0, 0, 0, 0, 0, name.encode_utf16().count().min(255) as u8];
        chars(&mut d, name);
        rec(&mut out, BOUNDSHEET, &d);
    }
    for (_, rows) in sheets {
        for c in rows.iter().flatten() {
            if c.parse::<f64>().is_err() && !c.starts_with("date:") && !sst.contains(c) {
                sst.push(c.clone());
            }
        }
    }
    let mut d = Vec::new();
    d.extend_from_slice(&(sst.len() as u32).to_le_bytes());
    d.extend_from_slice(&(sst.len() as u32).to_le_bytes());
    for s in &sst {
        d.extend_from_slice(&(s.encode_utf16().count() as u16).to_le_bytes());
        chars(&mut d, s);
    }
    rec(&mut out, SST, &d);
    rec(&mut out, EOF, &[]);
    for ((_, rows), at) in sheets.iter().zip(patches) {
        let off = (out.len() as u32).to_le_bytes();
        if let Some(slot) = out.get_mut(at..at + 4) {
            slot.copy_from_slice(&off);
        }
        rec(&mut out, BOF, &bof(0x0010));
        for (r, row) in rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                if v.is_empty() {
                    continue;
                }
                let mut d = Vec::new();
                d.extend_from_slice(&(r as u16).to_le_bytes());
                d.extend_from_slice(&(c as u16).to_le_bytes());
                if let Some(x) = v.strip_prefix("date:").and_then(|x| x.parse::<f64>().ok()) {
                    d.extend_from_slice(&1u16.to_le_bytes());
                    d.extend_from_slice(&x.to_le_bytes());
                    rec(&mut out, NUMBER, &d);
                } else if let Ok(x) = v.parse::<f64>() {
                    d.extend_from_slice(&0u16.to_le_bytes());
                    d.extend_from_slice(&x.to_le_bytes());
                    rec(&mut out, NUMBER, &d);
                } else {
                    d.extend_from_slice(&0u16.to_le_bytes());
                    let i = sst.iter().position(|s| s == v).unwrap_or(0) as u32;
                    d.extend_from_slice(&i.to_le_bytes());
                    rec(&mut out, LABELSST, &d);
                }
            }
        }
        rec(&mut out, EOF, &[]);
    }
    let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let mut s = comp.create_stream("/Workbook").map_err(|e| e.to_string())?;
        s.write_all(&out).map_err(|e| e.to_string())?;
    }
    comp.flush().map_err(|e| e.to_string())?;
    Ok(comp.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::{Pick, detect, read_rows, sheet_names};

    fn rows(r: &[&[&str]]) -> Vec<Vec<String>> {
        r.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect()
    }

    #[test]
    fn reads_a_biff8_workbook() {
        let book = build(&[
            ("名单", rows(&[&["姓名", "城市", "Amount", "Since"], &["张三", "北京", "12", "date:45397"], &["Ann", "Paris", "2.5", ""]])),
            ("Other", rows(&[&["x"]])),
        ])
        .unwrap();
        assert_eq!(detect(&book), Some(crate::sheet::Kind::Xls));
        assert_eq!(sheet_names(&book).unwrap(), vec!["名单", "Other"]);
        let (_, r) = read_rows(&book, Pick::Index(0)).unwrap();
        assert_eq!(r, rows(&[&["姓名", "城市", "Amount", "Since"], &["张三", "北京", "12", "2024-04-15"], &["Ann", "Paris", "2.5"]]));
        let (_, r) = read_rows(&book, Pick::Name("other")).unwrap();
        assert_eq!(r, rows(&[&["x"]]));
    }

    #[test]
    fn rk_numbers_and_strings_across_continue_records() {
        assert_eq!(rk((42 << 2) | 2), 42.0);
        assert_eq!(rk((1234 << 2) | 3), 12.34);
        assert_eq!(rk(0x3FF0_0000), 1.0);
        // "AB" compressed, then the rest in UTF-16 after a CONTINUE's fresh option byte.
        let a: &[u8] = &[4, 0, 0, b'A', b'B'];
        let b: &[u8] = &[1, 0x2D, 0x4E, 0x87, 0x65];
        let mut p = Pieces::new(vec![a, b]);
        assert_eq!(p.rich_string().as_deref(), Some("AB中文"));
    }

    #[test]
    fn hostile_workbooks_fail_cleanly() {
        let book = build(&[("S", rows(&[&["a", "1"]]))]).unwrap();
        // Truncations and bit flips never panic.
        for cut in (0..book.len()).step_by(97) {
            let _ = sheet_names(&book[..cut]);
            let _ = read_rows(&book[..cut], Pick::Index(0));
        }
        let stream = workbook_stream(&book).unwrap();
        for i in 0..stream.len() {
            let mut s = stream.clone();
            s[i] ^= 0xFF;
            if let Ok(g) = globals(&s) {
                for (_, off) in &g.sheets {
                    let _ = sheet_rows(&s, *off, &g);
                }
            }
        }
        // Excel 5.0/95 and non-workbook compound files say what they are.
        let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        comp.create_stream("/Book").unwrap();
        let old = comp.into_inner().into_inner();
        assert!(sheet_names(&old).unwrap_err().contains("Excel 5.0/95"));
    }

    #[test]
    fn embedded_substream_does_not_truncate_sheet() {
        let mut stream = Vec::new();
        fn rec(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
            out.extend_from_slice(data);
        }
        rec(&mut stream, BOF, &[0x00, 0x06, 0x05, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let sheet_offset = 37u32;
        rec(&mut stream, BOUNDSHEET, &[sheet_offset as u8, 0, 0, 0, 0, 0, 1, 0, b'S']);
        rec(&mut stream, EOF, &[]);
        assert_eq!(stream.len(), sheet_offset as usize);
        // Worksheet BOF
        rec(&mut stream, BOF, &[0x00, 0x06, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        // Cell row 0, col 0 = NUMBER 1.0
        let mut c1 = vec![0, 0, 0, 0, 0, 0];
        c1.extend_from_slice(&1.0f64.to_le_bytes());
        rec(&mut stream, NUMBER, &c1);
        // Embedded chart substream: BOF(0x0020) ... EOF
        rec(&mut stream, BOF, &[0x00, 0x06, 0x20, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        rec(&mut stream, EOF, &[]);
        // Cell row 1, col 0 = NUMBER 2.0
        let mut c2 = vec![1, 0, 0, 0, 0, 0];
        c2.extend_from_slice(&2.0f64.to_le_bytes());
        rec(&mut stream, NUMBER, &c2);
        // Worksheet EOF
        rec(&mut stream, EOF, &[]);

        let g = globals(&stream).unwrap();
        let rows = sheet_rows(&stream, sheet_offset as usize, &g).unwrap();
        assert_eq!(rows, vec![vec!["1".to_string()], vec!["2".to_string()]]);
    }
}
