//! Spreadsheets as mail-merge recipient lists (#334): the sheet names of an Excel workbook
//! (`.xlsx` / `.xlsm`, ECMA-376 Part 1 §18 SpreadsheetML) or an OpenDocument spreadsheet (`.ods`,
//! OASIS ODF 1.3 §9.1), and one sheet's cells as rows of display strings.
//!
//! Values: SpreadsheetML shared strings (rich-text runs joined, phonetic runs skipped), inline
//! strings, formula results (the cached `<v>`), booleans (`TRUE`/`FALSE`), errors as their text
//! and numbers. A number whose cell format is a date or time (built-in formats 14–22, 27–36,
//! 45–47 and 50–58, or a custom format with `d`/`y`, `h`/`s` outside quotes and brackets) is
//! written in the locale-neutral ISO 8601 form `yyyy-mm-dd`, `yyyy-mm-dd hh:mm[:ss]` or
//! `hh:mm[:ss]` (both the 1900 and 1904 date systems); other numbers keep up to 15 significant
//! digits, as the spreadsheet stores them. OpenDocument cells use `office:value-type`: floats
//! from `office:value`, dates and times from `office:date-value` / `office:time-value` (same ISO
//! forms), booleans as `TRUE`/`FALSE`, everything else (strings, percentages, currencies) as the
//! cell's text. Merged and covered cells keep only the top-left value; blank rows are dropped.
//!
//! Every file is hostile: zip entries are inflated up to [`MAX_ENTRY`] (and [`MAX_TOTAL`] in
//! all), sheets, rows, columns, cells and string lengths are capped, repeat counts are clamped to
//! what is left, and nothing recurses, so XML depth costs nothing.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

/// File extensions read as spreadsheets.
pub const EXTENSIONS: &[&str] = &["xlsx", "xlsm", "ods"];

/// Largest zip entry inflated.
pub const MAX_ENTRY: u64 = 64 << 20;
/// Most bytes inflated from one workbook.
pub const MAX_TOTAL: u64 = 160 << 20;
/// Most sheets listed.
pub const MAX_SHEETS: usize = 4_096;
/// Most non-blank rows kept (as many as a recipient list holds).
pub const MAX_ROWS: usize = 100_000;
/// Most columns kept; cells further right are ignored.
pub const MAX_COLS: usize = 1_000;
/// Most cells kept in all (blank cells between values count).
pub const MAX_CELLS: usize = 2_000_000;
/// Longest cell text in characters (a spreadsheet cell's own limit).
pub const MAX_CELL_CHARS: usize = 32_767;
/// Most shared strings kept.
const MAX_SHARED: usize = 2_000_000;
/// Most cell formats kept.
const MAX_XFS: usize = 65_536;
/// Elements nested deeper than this are skipped.
const MAX_XML_DEPTH: usize = 256;

/// The spreadsheet kinds read here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Xlsx,
    Ods,
}

/// Which sheet to read.
#[derive(Clone, Copy, Debug)]
pub enum Pick<'a> {
    /// 0-based position in the workbook's sheet list.
    Index(usize),
    /// A sheet name (exact match first, then ignoring case).
    Name(&'a str),
}

/// Whether `ext` (no dot, any case) names a spreadsheet read here.
pub fn is_extension(ext: &str) -> bool {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str())
}

/// What kind of spreadsheet `bytes` holds, from the package contents (`None`: not one).
pub fn detect(bytes: &[u8]) -> Option<Kind> {
    if !bytes.starts_with(b"PK") {
        return None;
    }
    let mut pkg = Package::open(bytes).ok()?;
    pkg.kind()
}

/// The sheet names, in workbook order.
pub fn sheet_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut pkg = Package::open(bytes)?;
    match pkg.kind().ok_or("not an Excel or OpenDocument spreadsheet")? {
        Kind::Xlsx => Ok(xlsx_sheets(&mut pkg)?.into_iter().map(|s| s.name).collect()),
        Kind::Ods => Ok(ods_read(&pkg.read("content.xml")?, None)?.0),
    }
}

/// One sheet's non-blank rows of display strings (trailing blank cells trimmed), with the
/// workbook's sheet names.
pub fn read_rows(bytes: &[u8], pick: Pick<'_>) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut pkg = Package::open(bytes)?;
    match pkg.kind().ok_or("not an Excel or OpenDocument spreadsheet")? {
        Kind::Xlsx => {
            let sheets = xlsx_sheets(&mut pkg)?;
            let names: Vec<String> = sheets.iter().map(|s| s.name.clone()).collect();
            let i = resolve(&names, pick)?;
            let sheet = sheets.get(i).ok_or("no such sheet")?;
            let rows = xlsx_rows(&mut pkg, &sheet.path)?;
            Ok((names, rows))
        }
        Kind::Ods => {
            let content = pkg.read("content.xml")?;
            // Names first, so a name can be resolved to its position.
            let (names, _) = ods_read(&content, None)?;
            let i = resolve(&names, pick)?;
            let (_, rows) = ods_read(&content, Some(i))?;
            Ok((names, rows))
        }
    }
}

/// Field names from a header row: trimmed, blanks named `Column N` (N = 1-based column) and
/// repeats numbered (`Name`, `Name 2`), ignoring case as merge fields do.
pub fn field_names(row: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(row.len());
    for (i, h) in row.iter().enumerate() {
        let base = match h.trim() {
            "" => format!("Column {}", i + 1),
            t => t.to_string(),
        };
        let taken = |n: &str, out: &[String]| out.iter().any(|o| o.eq_ignore_ascii_case(n));
        let mut name = base.clone();
        let mut k = 2usize;
        while taken(&name, &out) {
            name = format!("{base} {k}");
            k = k.saturating_add(1);
        }
        out.push(name);
    }
    out
}

fn resolve(names: &[String], pick: Pick<'_>) -> Result<usize, String> {
    if names.is_empty() {
        return Err("the workbook has no sheets".into());
    }
    match pick {
        Pick::Index(i) if i < names.len() => Ok(i),
        Pick::Index(i) => Err(format!("the workbook has {} sheets; there is no sheet {}", names.len(), i.saturating_add(1))),
        Pick::Name(n) => names
            .iter()
            .position(|s| s == n)
            .or_else(|| names.iter().position(|s| s.eq_ignore_ascii_case(n.trim())))
            .ok_or_else(|| format!("no sheet named \"{n}\"")),
    }
}

// ---------------------------------------------------------------------------------------------
// Package

struct Package<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    total: u64,
}

impl<'a> Package<'a> {
    fn open(bytes: &'a [u8]) -> Result<Self, String> {
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a valid spreadsheet package ({e})"))?;
        Ok(Package { zip, total: 0 })
    }

    fn has(&self, name: &str) -> bool {
        self.zip.index_for_name(name).is_some()
    }

    /// Inflate an entry, refusing ones (or a workbook) larger than the caps.
    fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        let f = self.zip.by_name(name).map_err(|_| format!("{name} is missing"))?;
        if f.size() > MAX_ENTRY {
            return Err(format!("{name} is too large"));
        }
        let mut out = Vec::new();
        f.take(MAX_ENTRY + 1).read_to_end(&mut out).map_err(|e| format!("{name}: {e}"))?;
        let len = out.len() as u64;
        self.total = self.total.saturating_add(len);
        if len > MAX_ENTRY || self.total > MAX_TOTAL {
            return Err(format!("{name} is too large"));
        }
        Ok(out)
    }

    fn kind(&mut self) -> Option<Kind> {
        if self.workbook_path().is_some() {
            return Some(Kind::Xlsx);
        }
        if self.has("content.xml") {
            let ods = match self.read("mimetype") {
                Ok(m) => String::from_utf8_lossy(&m).trim().starts_with("application/vnd.oasis.opendocument.spreadsheet"),
                // No mimetype entry: only a package without a text document is taken as one.
                Err(_) => !self.has("word/document.xml"),
            };
            if ods {
                return Some(Kind::Ods);
            }
        }
        None
    }

    /// The SpreadsheetML workbook part: the package's main part when it is a workbook,
    /// otherwise `xl/workbook.xml` when present.
    fn workbook_path(&mut self) -> Option<String> {
        if let Ok(rels) = self.read("_rels/.rels") {
            for r in relationships(&rels, "") {
                if r.kind.ends_with("/officeDocument") && r.target.to_ascii_lowercase().contains("workbook") && self.has(&r.target) {
                    return Some(r.target);
                }
            }
        }
        self.has("xl/workbook.xml").then(|| "xl/workbook.xml".to_string())
    }
}

// ---------------------------------------------------------------------------------------------
// XML

/// A streamed XML event with namespace prefixes dropped; elements deeper than
/// [`MAX_XML_DEPTH`] (and their text) are skipped.
enum Ev<'e> {
    Start(&'e BytesStart<'e>),
    End(Vec<u8>),
    Text(String),
}

fn walk(xml: &[u8], mut f: impl FnMut(Ev<'_>)) {
    let mut r = quick_xml::Reader::from_reader(xml);
    let cfg = r.config_mut();
    cfg.trim_text(false);
    cfg.check_end_names = false;
    cfg.allow_unmatched_ends = true;
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        buf.clear();
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth = depth.saturating_add(1);
                if depth <= MAX_XML_DEPTH {
                    f(Ev::Start(&e));
                }
            }
            Ok(Event::Empty(e)) => {
                if depth < MAX_XML_DEPTH {
                    f(Ev::Start(&e));
                    f(Ev::End(e.local_name().as_ref().to_vec()));
                }
            }
            Ok(Event::End(e)) => {
                if depth <= MAX_XML_DEPTH {
                    f(Ev::End(e.local_name().as_ref().to_vec()));
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Text(t)) if depth <= MAX_XML_DEPTH => f(Ev::Text(t.decode().map(|c| c.into_owned()).unwrap_or_default())),
            Ok(Event::CData(t)) if depth <= MAX_XML_DEPTH => f(Ev::Text(String::from_utf8_lossy(&t).into_owned())),
            Ok(Event::GeneralRef(g)) if depth <= MAX_XML_DEPTH => {
                let s = match &g[..] {
                    b"amp" => "&".to_string(),
                    b"lt" => "<".to_string(),
                    b"gt" => ">".to_string(),
                    b"quot" => "\"".to_string(),
                    b"apos" => "'".to_string(),
                    _ => g.resolve_char_ref().ok().flatten().map(|c| c.to_string()).unwrap_or_default(),
                };
                f(Ev::Text(s));
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

/// An attribute's value by local name.
fn attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes().with_checks(false).flatten().take(64).find(|a| a.key.local_name().as_ref() == name).map(|a| {
        a.normalized_value(XmlVersion::Explicit1_0).map(|c| c.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned())
    })
}

/// `s` + `t`, keeping `s` at most [`MAX_CELL_CHARS`] characters.
fn push_capped(s: &mut String, t: &str) {
    let have = s.chars().count();
    if have < MAX_CELL_CHARS {
        s.extend(t.chars().take(MAX_CELL_CHARS - have));
    }
}

/// Rows collected under the caps: blank rows are dropped, rows past [`MAX_ROWS`] and cells past
/// [`MAX_CELLS`] are ignored.
#[derive(Default)]
struct Grid {
    rows: Vec<Vec<String>>,
    cells: usize,
}

impl Grid {
    fn full(&self) -> bool {
        self.rows.len() >= MAX_ROWS || self.cells >= MAX_CELLS
    }

    /// Keep `row` (trailing blanks trimmed) `times` times, as far as the caps allow.
    fn push(&mut self, mut row: Vec<String>, times: u64) {
        while row.last().is_some_and(String::is_empty) {
            row.pop();
        }
        if row.is_empty() {
            return;
        }
        let mut left = times;
        while left > 0 && !self.full() {
            let room = MAX_CELLS.saturating_sub(self.cells);
            let mut r = row.clone();
            r.truncate(room);
            self.cells = self.cells.saturating_add(r.len());
            self.rows.push(r);
            left -= 1;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// SpreadsheetML

struct Rel {
    id: String,
    kind: String,
    target: String,
}

/// The relationships of a part in `dir` (targets resolved to package paths).
fn relationships(xml: &[u8], dir: &str) -> Vec<Rel> {
    let mut v = Vec::new();
    walk(xml, |ev| {
        if let Ev::Start(e) = ev
            && e.local_name().as_ref() == b"Relationship"
            && v.len() < MAX_SHEETS * 2
        {
            if attr(e, b"TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External")) {
                return;
            }
            let target = attr(e, b"Target").unwrap_or_default();
            v.push(Rel { id: attr(e, b"Id").unwrap_or_default(), kind: attr(e, b"Type").unwrap_or_default(), target: join(dir, &target) });
        }
    });
    v
}

/// A relative part name resolved against `dir` (`/` = package root).
fn join(dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|p| !p.is_empty()).collect() };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn rels_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((d, f)) => format!("{d}/_rels/{f}.rels"),
        None => format!("_rels/{path}.rels"),
    }
}

struct SheetRef {
    name: String,
    path: String,
}

struct Workbook {
    sheets: Vec<SheetRef>,
    date1904: bool,
    shared: Option<String>,
    styles: Option<String>,
}

fn workbook(pkg: &mut Package<'_>) -> Result<Workbook, String> {
    let path = pkg.workbook_path().ok_or("the workbook part is missing")?;
    let xml = pkg.read(&path)?;
    let rels = pkg.read(&rels_of(&path)).map(|r| relationships(&r, dir_of(&path))).unwrap_or_default();
    let mut listed: Vec<(String, String)> = Vec::new();
    let mut date1904 = false;
    walk(&xml, |ev| {
        if let Ev::Start(e) = ev {
            match e.local_name().as_ref() {
                b"sheet" if listed.len() < MAX_SHEETS => listed.push((attr(e, b"name").unwrap_or_default(), attr(e, b"id").unwrap_or_default())),
                b"workbookPr" => date1904 = attr(e, b"date1904").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
                _ => {}
            }
        }
    });
    let sheets = listed
        .into_iter()
        .filter_map(|(name, id)| {
            // Worksheets only: chart and dialog sheets hold no cells.
            let r = rels.iter().find(|r| r.id == id && r.kind.ends_with("/worksheet"))?;
            Some(SheetRef { name, path: r.target.clone() })
        })
        .collect();
    let part = |suffix: &str| rels.iter().find(|r| r.kind.ends_with(suffix)).map(|r| r.target.clone());
    Ok(Workbook { sheets, date1904, shared: part("/sharedStrings"), styles: part("/styles") })
}

fn xlsx_sheets(pkg: &mut Package<'_>) -> Result<Vec<SheetRef>, String> {
    Ok(workbook(pkg)?.sheets)
}

/// The shared string table (§18.4): each `si`'s text, rich-text runs joined, phonetic runs
/// (`rPh`) left out.
fn shared_strings(xml: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let (mut in_t, mut phonetic) = (false, 0usize);
    walk(xml, |ev| match ev {
        Ev::Start(e) => match e.local_name().as_ref() {
            b"si" => cur = Some(String::new()),
            b"t" => in_t = true,
            b"rPh" => phonetic += 1,
            _ => {}
        },
        Ev::End(n) => match n.as_slice() {
            b"si" => {
                if let Some(s) = cur.take()
                    && out.len() < MAX_SHARED
                {
                    out.push(s);
                }
            }
            b"t" => in_t = false,
            b"rPh" => phonetic = phonetic.saturating_sub(1),
            _ => {}
        },
        Ev::Text(t) => {
            if in_t
                && phonetic == 0
                && let Some(s) = cur.as_mut()
            {
                push_capped(s, &t);
            }
        }
    });
    out
}

/// Whether a number format shows a date, a time or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DateKind {
    Date,
    Time,
    DateTime,
}

/// The built-in number formats (§18.8.30) that show dates or times.
fn builtin_date(id: u32) -> Option<DateKind> {
    match id {
        14..=17 | 27..=36 | 50..=58 => Some(DateKind::Date),
        18..=21 | 45..=47 => Some(DateKind::Time),
        22 => Some(DateKind::DateTime),
        _ => None,
    }
}

/// Whether a custom format code (§18.8.31) shows a date or time: its first section, without
/// quoted text, escaped characters, `[…]` (colours, locales, conditions) and `_x` / `*x`
/// padding, has `d` or `y` (date), `h` or `s` (time), or `m` alone (a month).
fn custom_date(code: &str) -> Option<DateKind> {
    let mut plain = String::new();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            ';' => break,
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                }
            }
            '[' => {
                for q in chars.by_ref() {
                    if q == ']' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            c => plain.extend(c.to_lowercase()),
        }
    }
    let has = |c: char| plain.contains(c);
    let date = has('d') || has('y');
    let time = has('h') || has('s');
    match (date, time) {
        (true, true) => Some(DateKind::DateTime),
        (true, false) => Some(DateKind::Date),
        (false, true) => Some(DateKind::Time),
        (false, false) if has('m') && !has('0') && !has('#') => Some(DateKind::Date),
        _ => None,
    }
}

/// Each cell format's (`cellXfs/xf`, by `s` index) date kind.
fn cell_formats(xml: &[u8]) -> Vec<Option<DateKind>> {
    let mut custom: HashMap<u32, Option<DateKind>> = HashMap::new();
    let mut xfs: Vec<u32> = Vec::new();
    let mut in_xfs = false;
    walk(xml, |ev| match ev {
        Ev::Start(e) => match e.local_name().as_ref() {
            b"numFmt" if custom.len() < MAX_XFS => {
                if let Some(id) = attr(e, b"numFmtId").and_then(|v| v.trim().parse().ok()) {
                    custom.insert(id, custom_date(&attr(e, b"formatCode").unwrap_or_default()));
                }
            }
            b"cellXfs" => in_xfs = true,
            b"xf" if in_xfs && xfs.len() < MAX_XFS => xfs.push(attr(e, b"numFmtId").and_then(|v| v.trim().parse().ok()).unwrap_or(0)),
            _ => {}
        },
        Ev::End(n) if n.as_slice() == b"cellXfs" => in_xfs = false,
        _ => {}
    });
    xfs.into_iter().map(|id| custom.get(&id).copied().unwrap_or_else(|| builtin_date(id))).collect()
}

/// A cell reference's 0-based column (`C5` → 2); `None` when it isn't one.
fn ref_col(r: &str) -> Option<usize> {
    let mut col = 0usize;
    let mut letters = 0;
    for c in r.chars() {
        if c.is_ascii_alphabetic() {
            letters += 1;
            if letters > 3 {
                return None;
            }
            col = col * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1);
        } else {
            break;
        }
    }
    col.checked_sub(1)
}

#[derive(Default)]
struct XCell {
    col: usize,
    kind: String,
    style: usize,
    value: String,
    inline: String,
}

fn xlsx_rows(pkg: &mut Package<'_>, path: &str) -> Result<Vec<Vec<String>>, String> {
    let wb = workbook(pkg)?;
    let shared = match wb.shared.as_deref().or(pkg.has("xl/sharedStrings.xml").then_some("xl/sharedStrings.xml")).map(str::to_string) {
        Some(p) => shared_strings(&pkg.read(&p)?),
        None => Vec::new(),
    };
    let formats = match wb.styles.as_deref().or(pkg.has("xl/styles.xml").then_some("xl/styles.xml")).map(str::to_string) {
        Some(p) => cell_formats(&pkg.read(&p)?),
        None => Vec::new(),
    };
    let xml = pkg.read(path)?;
    // Rows by number, so out-of-order rows still come out in order.
    let mut rows: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    let mut cells = 0usize;
    let (mut in_data, mut row_no, mut next_col) = (false, 0u64, 0usize);
    let mut cell: Option<XCell> = None;
    let (mut in_v, mut in_is_t, mut phonetic) = (false, false, 0usize);
    walk(&xml, |ev| match ev {
        Ev::Start(e) => match e.local_name().as_ref() {
            b"sheetData" => in_data = true,
            b"row" if in_data => {
                row_no = attr(e, b"r").and_then(|v| v.trim().parse().ok()).unwrap_or(row_no.saturating_add(1));
                next_col = 0;
            }
            b"c" if in_data => {
                // A reference that isn't one (or is past the last column) drops the cell.
                let col = match attr(e, b"r") {
                    Some(r) => ref_col(r.trim()).unwrap_or(usize::MAX),
                    None => next_col,
                };
                next_col = col.saturating_add(1);
                let style = attr(e, b"s").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                cell = Some(XCell { col, kind: attr(e, b"t").unwrap_or_default(), style, ..Default::default() });
            }
            b"v" => in_v = cell.is_some(),
            b"t" => in_is_t = cell.is_some(),
            b"rPh" => phonetic += 1,
            _ => {}
        },
        Ev::Text(t) => {
            if let Some(c) = cell.as_mut() {
                if in_v {
                    push_capped(&mut c.value, &t);
                } else if in_is_t && phonetic == 0 {
                    push_capped(&mut c.inline, &t);
                }
            }
        }
        Ev::End(n) => match n.as_slice() {
            b"sheetData" => in_data = false,
            b"v" => in_v = false,
            b"t" => in_is_t = false,
            b"rPh" => phonetic = phonetic.saturating_sub(1),
            b"c" => {
                let Some(c) = cell.take() else { return };
                let text = xlsx_value(&c, &shared, &formats, wb.date1904);
                if text.is_empty() || c.col >= MAX_COLS {
                    return;
                }
                if !rows.contains_key(&row_no) && rows.len() >= MAX_ROWS {
                    return;
                }
                let row = rows.entry(row_no).or_default();
                if row.len() <= c.col {
                    let grow = c.col + 1 - row.len();
                    if cells.saturating_add(grow) > MAX_CELLS {
                        return;
                    }
                    cells += grow;
                    row.resize(c.col + 1, String::new());
                }
                if let Some(slot) = row.get_mut(c.col) {
                    *slot = text;
                }
            }
            _ => {}
        },
    });
    let mut grid = Grid::default();
    for (_, r) in rows {
        grid.push(r, 1);
    }
    Ok(grid.rows)
}

fn xlsx_value(c: &XCell, shared: &[String], formats: &[Option<DateKind>], date1904: bool) -> String {
    let v = c.value.trim();
    match c.kind.as_str() {
        "s" => v.parse::<usize>().ok().and_then(|i| shared.get(i)).cloned().unwrap_or_default(),
        "inlineStr" => c.inline.clone(),
        "str" | "e" => c.value.clone(),
        "b" => if v == "1" || v.eq_ignore_ascii_case("true") { "TRUE" } else { "FALSE" }.into(),
        "d" => iso_datetime(v).unwrap_or_else(|| v.to_string()),
        _ if v.is_empty() => c.inline.clone(),
        _ => {
            let Ok(x) = v.parse::<f64>() else { return v.to_string() };
            match formats.get(c.style).copied().flatten() {
                Some(kind) => serial_date(x, date1904, kind).unwrap_or_else(|| number(x)),
                None => number(x),
            }
        }
    }
}

/// A number as a spreadsheet shows it in General format: up to 15 significant digits, no
/// trailing zeros.
fn number(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        return format!("{x:.0}");
    }
    let rounded: f64 = format!("{x:.14e}").parse().unwrap_or(x);
    if rounded.abs() >= 1e15 || rounded.abs() < 1e-9 { format!("{rounded:E}") } else { rounded.to_string() }
}

/// A serial date-time (days since the workbook's epoch) in ISO 8601 form.
fn serial_date(serial: f64, date1904: bool, kind: DateKind) -> Option<String> {
    // Day 2958465 is 9999-12-31.
    if !serial.is_finite() || !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let mut days = serial.floor() as i64;
    let mut secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    if secs >= 86_400 {
        days += 1;
        secs -= 86_400;
    }
    // Days from 1970-01-01. The 1900 system counts a 29 February 1900 that never was, so days
    // before it start a day later.
    let epoch = if date1904 {
        -24_107
    } else if days < 60 {
        -25_568
    } else {
        -25_569
    };
    let (y, m, d) = civil(days + epoch);
    let (h, mi, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let time = if s == 0 { format!("{h:02}:{mi:02}") } else { format!("{h:02}:{mi:02}:{s:02}") };
    Some(match kind {
        DateKind::Date => format!("{y:04}-{m:02}-{d:02}"),
        DateKind::Time => time,
        DateKind::DateTime => format!("{y:04}-{m:02}-{d:02} {time}"),
    })
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// An ISO 8601 date or date-time (`2024-03-05`, `2024-03-05T13:45:00.000`) as `yyyy-mm-dd`,
/// with ` hh:mm[:ss]` when the time isn't midnight.
fn iso_datetime(v: &str) -> Option<String> {
    let (date, time) = v.split_once('T').unwrap_or((v, ""));
    let mut it = date.splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.get(..2)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut out = format!("{y:04}-{m:02}-{d:02}");
    let hms: Vec<u32> = time.split(['Z', '+', '.']).next().unwrap_or("").split(':').filter_map(|p| p.parse().ok()).collect();
    if let [h, mi, rest @ ..] = hms.as_slice() {
        let s = rest.first().copied().unwrap_or(0);
        if (*h, *mi, s) != (0, 0, 0) {
            out.push_str(&if s == 0 { format!(" {h:02}:{mi:02}") } else { format!(" {h:02}:{mi:02}:{s:02}") });
        }
    }
    Some(out)
}

/// An ISO 8601 duration time (`PT13H45M00S`) as `hh:mm[:ss]`.
fn iso_time(v: &str) -> Option<String> {
    let t = v.trim().strip_prefix("PT")?;
    let (mut h, mut m, mut s) = (0u64, 0u64, 0f64);
    let mut num = String::new();
    for c in t.chars() {
        match c {
            'H' => h = std::mem::take(&mut num).parse().ok()?,
            'M' => m = std::mem::take(&mut num).parse().ok()?,
            'S' => s = std::mem::take(&mut num).parse().ok()?,
            c if num.len() < 20 => num.push(c),
            _ => return None,
        }
    }
    let s = s.round() as u64;
    Some(if s == 0 { format!("{h:02}:{m:02}") } else { format!("{h:02}:{m:02}:{s:02}") })
}

// ---------------------------------------------------------------------------------------------
// OpenDocument

#[derive(Default)]
struct OCell {
    repeat: u64,
    kind: String,
    value: Option<String>,
    text: String,
    paras: usize,
}

/// The sheet names of `content.xml`, and with `want` that sheet's rows (ODF 1.3 §9.1).
fn ods_read(xml: &[u8], want: Option<usize>) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut names = Vec::new();
    let mut grid = Grid::default();
    // Nesting of `table:table` (sub-tables inside cells are skipped), and whether the outermost
    // one is the wanted sheet.
    let mut tables = 0usize;
    let mut collecting = false;
    let mut row: Option<(Vec<String>, u64, u64)> = None; // cells, pending blank cells, repeat
    let mut cell: Option<OCell> = None;
    let (mut in_p, mut skip) = (0usize, 0usize);
    walk(xml, |ev| match ev {
        Ev::Start(e) => {
            let n = e.local_name();
            let n = n.as_ref();
            if n == b"table" {
                if tables == 0 && names.len() < MAX_SHEETS {
                    collecting = want == Some(names.len());
                    names.push(attr(e, b"name").unwrap_or_else(|| format!("Sheet{}", names.len() + 1)));
                }
                tables += 1;
                return;
            }
            if !collecting || tables != 1 {
                return;
            }
            match n {
                b"table-row" => {
                    let repeat = attr(e, b"number-rows-repeated").and_then(|v| v.trim().parse().ok()).unwrap_or(1u64).max(1);
                    row = Some((Vec::new(), 0, repeat));
                }
                b"table-cell" | b"covered-table-cell" if row.is_some() => {
                    let repeat = attr(e, b"number-columns-repeated").and_then(|v| v.trim().parse().ok()).unwrap_or(1u64).max(1);
                    let kind = attr(e, b"value-type").unwrap_or_default();
                    let value = match kind.as_str() {
                        "float" => attr(e, b"value"),
                        "date" => attr(e, b"date-value"),
                        "time" => attr(e, b"time-value"),
                        "boolean" => attr(e, b"boolean-value"),
                        _ => None,
                    };
                    // A covered cell's own content is hidden by the merge.
                    let covered = n == b"covered-table-cell";
                    cell = Some(OCell {
                        repeat,
                        kind: if covered { String::new() } else { kind },
                        value: if covered { None } else { value },
                        ..Default::default()
                    });
                    if covered {
                        skip += 1;
                    }
                }
                b"annotation" if cell.is_some() => skip += 1,
                b"p" | b"h" if cell.is_some() => {
                    if let Some(c) = cell.as_mut()
                        && skip == 0
                    {
                        if c.paras > 0 {
                            push_capped(&mut c.text, "\n");
                        }
                        c.paras += 1;
                    }
                    in_p += 1;
                }
                b"s" if in_p > 0 && skip == 0 => {
                    let k = attr(e, b"c").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 1_000);
                    if let Some(c) = cell.as_mut() {
                        push_capped(&mut c.text, &" ".repeat(k));
                    }
                }
                b"tab" if in_p > 0 && skip == 0 => {
                    if let Some(c) = cell.as_mut() {
                        push_capped(&mut c.text, "\t");
                    }
                }
                b"line-break" if in_p > 0 && skip == 0 => {
                    if let Some(c) = cell.as_mut() {
                        push_capped(&mut c.text, "\n");
                    }
                }
                _ => {}
            }
        }
        Ev::Text(t) => {
            if in_p > 0
                && skip == 0
                && let Some(c) = cell.as_mut()
            {
                push_capped(&mut c.text, &t);
            }
        }
        Ev::End(n) => {
            let n = n.as_slice();
            if n == b"table" {
                tables = tables.saturating_sub(1);
                if tables == 0 {
                    collecting = false;
                }
                return;
            }
            if !collecting || tables != 1 {
                return;
            }
            match n {
                b"p" | b"h" => in_p = in_p.saturating_sub(1),
                b"annotation" => skip = skip.saturating_sub(1),
                b"table-cell" | b"covered-table-cell" => {
                    if n == b"covered-table-cell" {
                        skip = skip.saturating_sub(1);
                    }
                    let (Some(c), Some((cells, pending, _))) = (cell.take(), row.as_mut()) else { return };
                    let text = ods_value(&c);
                    if text.is_empty() {
                        *pending = pending.saturating_add(c.repeat);
                        return;
                    }
                    // Blank cells before this one, then the value as often as it repeats, as far
                    // as the columns allow.
                    let blanks = (*pending).min(MAX_COLS.saturating_sub(cells.len()) as u64) as usize;
                    cells.resize(cells.len() + blanks, String::new());
                    *pending = 0;
                    let times = c.repeat.min(MAX_COLS.saturating_sub(cells.len()) as u64) as usize;
                    cells.extend(std::iter::repeat_n(text, times));
                }
                b"table-row" => {
                    if let Some((cells, _, repeat)) = row.take() {
                        grid.push(cells, repeat);
                    }
                }
                _ => {}
            }
        }
    });
    if names.is_empty() {
        return Err("the spreadsheet has no sheets".into());
    }
    Ok((names, grid.rows))
}

fn ods_value(c: &OCell) -> String {
    let v = c.value.as_deref().map(str::trim);
    match (c.kind.as_str(), v) {
        ("float", Some(v)) => v.parse::<f64>().map(number).unwrap_or_else(|_| c.text.clone()),
        ("date", Some(v)) => iso_datetime(v).unwrap_or_else(|| c.text.clone()),
        ("time", Some(v)) => iso_time(v).unwrap_or_else(|| c.text.clone()),
        ("boolean", Some(v)) => if v.eq_ignore_ascii_case("true") || v == "1" { "TRUE" } else { "FALSE" }.into(),
        _ => c.text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            z.start_file(*name, o).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    const RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;

    fn xlsx(sheet1: &str) -> Vec<u8> {
        let wb = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr/><sheets><sheet name="People" sheetId="1" r:id="rId1"/><sheet name="Chart" sheetId="2" r:id="rId9"/><sheet name="Notes" sheetId="3" r:id="rId2"/></sheets></workbook>"#;
        let wb_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="/xl/worksheets/sheet2.xml"/><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
        let sst = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Name</t></si><si><t>Born</t></si><si><r><t>Ada </t></r><r><rPr><b/></rPr><t xml:space="preserve">Lovelace</t></r><rPh sb="0" eb="1"><t>ignored</t></rPh></si><si><t>Name</t></si></sst>"#;
        let styles = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="164" formatCode="dd/mm/yyyy;@"/><numFmt numFmtId="165" formatCode="&quot;Day&quot; 0.00"/></numFmts><cellStyleXfs><xf numFmtId="14"/></cellStyleXfs><cellXfs><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="164"/><xf numFmtId="165"/><xf numFmtId="22"/></cellXfs></styleSheet>"#;
        let sheet2 = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>x</t></is></c></row></sheetData></worksheet>"#;
        zip(&[
            ("_rels/.rels", RELS),
            ("xl/workbook.xml", wb),
            ("xl/_rels/workbook.xml.rels", wb_rels),
            ("xl/sharedStrings.xml", sst),
            ("xl/styles.xml", styles),
            ("xl/worksheets/sheet1.xml", sheet1),
            ("xl/worksheets/sheet2.xml", sheet2),
        ])
    }

    #[test]
    fn xlsx_shared_strings_dates_numbers_and_gaps() {
        let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
            <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="D1" t="s"><v>3</v></c><c r="E1" t="s"><v>3</v></c></row>
            <row r="3"><c r="A3" t="s"><v>2</v></c><c r="B3" s="1"><v>45356</v></c><c r="D3"><v>0.30000000000000004</v></c><c r="E3" t="b"><v>1</v></c></row>
            <row r="4"><c r="A4" t="inlineStr"><is><t>Alan</t></is></c><c r="B4" s="2"><v>1</v></c><c r="C4" s="3"><v>12</v></c><c r="D4" t="str"><f>A4&amp;"!"</f><v>Alan!</v></c><c r="E4" s="4"><v>45356.5</v></c></row>
            <row r="5"><c r="ZZZZ5"><v>1</v></c><c r="Z5" t="s"><v>999</v></c></row>
        </sheetData><mergeCells count="1"><mergeCell ref="A4:B4"/></mergeCells></worksheet>"#;
        let book = xlsx(sheet);
        assert_eq!(detect(&book), Some(Kind::Xlsx));
        assert_eq!(sheet_names(&book).unwrap(), ["People", "Notes"], "chart sheets aren't listed");
        let (names, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(rows.len(), 3, "blank rows (2, and 5 with nothing readable) are dropped: {rows:?}");
        assert_eq!(rows[0], ["Name", "Born", "", "Name", "Name"]);
        assert_eq!(field_names(&rows[0]), ["Name", "Born", "Column 3", "Name 2", "Name 3"]);
        assert_eq!(rows[1], ["Ada Lovelace", "2024-03-05", "", "0.3", "TRUE"]);
        assert_eq!(rows[2], ["Alan", "1900-01-01", "12", "Alan!", "2024-03-05 12:00"]);
        let (_, notes) = read_rows(&book, Pick::Name("notes")).unwrap();
        assert_eq!(notes, [["x"]]);
        assert!(read_rows(&book, Pick::Index(5)).is_err());
        assert!(read_rows(&book, Pick::Name("Missing")).is_err());
    }

    #[test]
    fn ods_repeats_types_and_covered_cells() {
        let content = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet>
            <table:table table:name="First"><table:table-row><table:table-cell office:value-type="string"><text:p>only</text:p></table:table-cell></table:table-row></table:table>
            <table:table table:name="Guests">
              <table:table-row table:number-rows-repeated="3"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>
              <table:table-row><table:table-cell office:value-type="string"><text:p>Name</text:p></table:table-cell><table:table-cell table:number-columns-repeated="2"/><table:table-cell office:value-type="string"><text:p>When</text:p></table:table-cell><table:table-cell office:value-type="string" table:number-columns-repeated="2"><text:p>Tag</text:p></table:table-cell><table:table-cell table:number-columns-repeated="16378"/></table:table-row>
              <table:table-row table:number-rows-repeated="2"><table:table-cell office:value-type="string" table:number-columns-spanned="2"><text:p>Ada<text:s text:c="2"/>L</text:p><text:p>x</text:p><office:annotation><text:p>note</text:p></office:annotation></table:table-cell><table:covered-table-cell office:value-type="string"><text:p>hidden</text:p></table:covered-table-cell><table:table-cell office:value-type="float" office:value="3.50"><text:p>3,5</text:p></table:table-cell><table:table-cell office:value-type="date" office:date-value="2024-03-05T00:00:00"/><table:table-cell office:value-type="boolean" office:boolean-value="false"/><table:table-cell office:value-type="time" office:time-value="PT13H45M00S"/></table:table-row>
              <table:table-row table:number-rows-repeated="1048570"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>
            </table:table></office:spreadsheet></office:body></office:document-content>"#;
        let book = zip(&[("mimetype", "application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", content)]);
        assert_eq!(detect(&book), Some(Kind::Ods));
        assert_eq!(sheet_names(&book).unwrap(), ["First", "Guests"]);
        let (_, rows) = read_rows(&book, Pick::Name("Guests")).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], ["Name", "", "", "When", "Tag", "Tag"]);
        assert_eq!(rows[1], ["Ada  L\nx", "", "3.5", "2024-03-05", "FALSE", "13:45"]);
        assert_eq!(rows[1], rows[2]);
    }

    #[test]
    fn hostile_workbooks_are_refused_or_capped_without_panicking() {
        // Huge repeat counts: rows and cells stay within the caps.
        let row = r#"<table:table-row table:number-rows-repeated="4000000000"><table:table-cell office:value-type="string" table:number-columns-repeated="4000000000"><text:p>x</text:p></table:table-cell></table:table-row>"#;
        let content = format!(
            r#"<office:document-content xmlns:office="o" xmlns:table="t" xmlns:text="x"><office:body><office:spreadsheet><table:table table:name="S">{row}</table:table></office:spreadsheet></office:body></office:document-content>"#
        );
        let book = zip(&[("mimetype", "application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", &content)]);
        let (_, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        assert!(rows.len() <= MAX_ROWS && rows.iter().all(|r| r.len() <= MAX_COLS));
        assert!(rows.iter().map(Vec::len).sum::<usize>() <= MAX_CELLS);
        // A zip bomb: a tiny entry that inflates past the entry cap is refused.
        let big = "<a>".to_string() + &"x".repeat((MAX_ENTRY + 10) as usize) + "</a>";
        let bomb = zip(&[("mimetype", "application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", &big)]);
        assert!(bomb.len() < 1 << 20, "the bomb is small: {}", bomb.len());
        assert!(read_rows(&bomb, Pick::Index(0)).unwrap_err().contains("too large"));
        // Huge cell references, long strings and junk.
        let long = "y".repeat(MAX_CELL_CHARS + 50);
        let sheet = format!(
            r#"<worksheet><sheetData><row r="99999999999999999999"><c r="XFD1"><v>1</v></c><c r="A1" t="inlineStr"><is><t>{long}</t></is></c><c r="B1" s="99999999999999999999" t="s"><v>-1</v></c><c r="C1" s="1"><v>1e308</v></c></row></sheetData></worksheet>"#
        );
        let (_, rows) = read_rows(&xlsx(&sheet), Pick::Index(0)).unwrap();
        assert_eq!(rows[0][0].chars().count(), MAX_CELL_CHARS);
        assert_eq!(rows[0], [long[..MAX_CELL_CHARS].to_string(), String::new(), "1E308".to_string()]);
        for junk in [&b""[..], b"PK\x03\x04junk", b"not a zip", &zip(&[("content.xml", "<<<")])] {
            let _ = sheet_names(junk);
            let _ = read_rows(junk, Pick::Index(0));
        }
    }

    #[test]
    fn dates_and_formats() {
        assert_eq!(custom_date("[$-409]mmmm d, yyyy;@"), Some(DateKind::Date));
        assert_eq!(custom_date("h:mm AM/PM"), Some(DateKind::Time));
        assert_eq!(custom_date("\"days\" 0.00"), None);
        assert_eq!(custom_date("General"), None);
        assert_eq!(custom_date("[Red]#,##0.00"), None);
        assert_eq!(serial_date(61.0, false, DateKind::Date).as_deref(), Some("1900-03-01"));
        assert_eq!(serial_date(0.0, true, DateKind::Date).as_deref(), Some("1904-01-01"));
        assert_eq!(serial_date(0.75, false, DateKind::Time).as_deref(), Some("18:00"));
        assert_eq!(serial_date(-1.0, false, DateKind::Date), None);
        assert_eq!(number(1234567.0), "1234567");
        assert_eq!(number(0.1 + 0.2), "0.3");
    }
}
