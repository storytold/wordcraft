//! Spreadsheets as mail-merge recipient lists (#334): the sheet names of an Excel workbook
//! (`.xlsx` / `.xlsm`, ECMA-376 Part 1 §18 SpreadsheetML; legacy `.xls` BIFF8 in [`xls`], #778)
//! or an OpenDocument spreadsheet (`.ods`, OASIS ODF 1.3 §9.1), and one sheet's cells as rows of
//! display strings.
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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read};

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

mod xls;

#[doc(hidden)]
pub use xls::build as xls_fixture;

mod date;
mod ods;
mod xlsx;

pub(crate) use date::civil;
use date::{DateKind, builtin_date, custom_date, serial_date};
#[cfg(test)]
use date::{iso_datetime, iso_time};
use ods::ods_read;
use xlsx::{number, relationships, xlsx_rows, xlsx_sheets};

/// File extensions read as spreadsheets.
pub const EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xls", "ods"];

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
/// Most bytes of cell text kept from one sheet: a shared string (or a repeated cell) is stored
/// once in the file but copied into every cell that uses it, so a small file could otherwise
/// ask for many gigabytes. Cells past the budget are ignored.
pub const MAX_TEXT_BYTES: usize = 64 << 20;
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
    /// Excel 97–2003 (BIFF8 in a compound file).
    Xls,
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
    if xls::is_xls(bytes) {
        return Some(Kind::Xls);
    }
    if !bytes.starts_with(b"PK") {
        return None;
    }
    let mut pkg = Package::open(bytes).ok()?;
    pkg.kind()
}

/// The sheet names, in workbook order.
pub fn sheet_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.starts_with(&xls::MAGIC) {
        return xls::sheet_names(bytes);
    }
    let mut pkg = Package::open(bytes)?;
    match pkg.kind().ok_or("not an Excel or OpenDocument spreadsheet")? {
        Kind::Xlsx => Ok(xlsx_sheets(&mut pkg)?.into_iter().map(|s| s.name).collect()),
        Kind::Ods => Ok(ods_read(&pkg.read("content.xml")?, None)?.0),
        Kind::Xls => xls::sheet_names(bytes),
    }
}

/// One sheet's non-blank rows of display strings (trailing blank cells trimmed), with the
/// workbook's sheet names.
pub fn read_rows(bytes: &[u8], pick: Pick<'_>) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    if bytes.starts_with(&xls::MAGIC) {
        return xls::read_rows(bytes, pick);
    }
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
        Kind::Xls => xls::read_rows(bytes, pick),
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
    // The names given so far, ASCII-lowercased, and per base name the next number to try: a set
    // lookup per candidate, and each base resumes where it stopped, instead of rescanning every
    // name from `2` for every repeat (cubic on a row of equal headers).
    let mut taken: HashSet<String> = HashSet::with_capacity(row.len());
    let mut next: HashMap<String, usize> = HashMap::new();
    for (i, h) in row.iter().enumerate() {
        let base = match h.trim() {
            "" => format!("Column {}", i + 1),
            t => t.to_string(),
        };
        let mut name = base.clone();
        if taken.contains(&name.to_ascii_lowercase()) {
            let k = next.entry(base.to_ascii_lowercase()).or_insert(2);
            loop {
                name = format!("{base} {k}");
                *k = k.saturating_add(1);
                if !taken.contains(&name.to_ascii_lowercase()) {
                    break;
                }
            }
        }
        taken.insert(name.to_ascii_lowercase());
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

/// Running total of the cell text kept, against [`MAX_TEXT_BYTES`].
#[derive(Default, Clone, Copy)]
struct TextBudget(usize);

impl TextBudget {
    /// Spend `bytes`; false (and nothing spent) when they don't fit.
    fn take(&mut self, bytes: usize) -> bool {
        match self.0.checked_add(bytes) {
            Some(t) if t <= MAX_TEXT_BYTES => {
                self.0 = t;
                true
            }
            _ => false,
        }
    }
}

/// Rows collected under the caps: blank rows are dropped, rows past [`MAX_ROWS`], cells past
/// [`MAX_CELLS`] and rows past [`MAX_TEXT_BYTES`] of text are ignored.
#[derive(Default)]
struct Grid {
    rows: Vec<Vec<String>>,
    cells: usize,
    text: TextBudget,
    /// A row has been left out for its text (and logged, once per sheet).
    refused: bool,
}

impl Grid {
    fn full(&self) -> bool {
        self.rows.len() >= MAX_ROWS || self.cells >= MAX_CELLS || self.text.0 >= MAX_TEXT_BYTES
    }

    /// Note that a row is left out because its text is over what is left of [`MAX_TEXT_BYTES`]:
    /// logged once per sheet, not once per row.
    fn refuse(&mut self) {
        if !self.refused {
            self.refused = true;
            log::warn!("sheet: rows with more text than the {MAX_TEXT_BYTES} byte budget has left are left out of the recipient list");
        }
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
            // What fits is measured on `row` itself; only a copy that will be kept is made.
            let keep = row.len().min(MAX_CELLS.saturating_sub(self.cells));
            let r = row.get(..keep).unwrap_or_default();
            if !self.text.take(r.iter().map(String::len).sum()) {
                self.refuse();
                return;
            }
            self.cells = self.cells.saturating_add(r.len());
            self.rows.push(r.to_vec());
            left -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn negative_zero_formats_as_zero() {
        assert_eq!(number(-0.0), "0");
    }

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

    /// #1287: one long shared string in thousands of cells stays under the text budget.
    #[test]
    fn xlsx_shared_strings_cannot_blow_up_memory() {
        let wb = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        let wb_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;
        let sst =
            format!(r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>{}</t></si></sst>"#, "x".repeat(MAX_CELL_CHARS));
        let mut sheet = String::from("<worksheet><sheetData>");
        for r in 1..=4_000 {
            sheet.push_str(&format!(r#"<row r="{r}"><c r="A{r}" t="s"><v>0</v></c></row>"#));
        }
        sheet.push_str("</sheetData></worksheet>");
        let book = zip(&[
            ("_rels/.rels", RELS),
            ("xl/workbook.xml", wb),
            ("xl/_rels/workbook.xml.rels", wb_rels),
            ("xl/sharedStrings.xml", &sst),
            ("xl/worksheets/sheet1.xml", &sheet),
        ]);
        let (_, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        let total: usize = rows.iter().flatten().map(String::len).sum();
        assert!(total <= MAX_TEXT_BYTES, "{total} bytes of cell text");
        assert!(rows.len() > 1_000, "the budget still keeps a real list: {}", rows.len());
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
        assert_eq!(iso_datetime("2024-03-05T13:45:30-05:00").as_deref(), Some("2024-03-05 13:45:30"));
        assert_eq!(iso_datetime("2024-03-5T13:45:00").as_deref(), Some("2024-03-05 13:45"));
        assert_eq!(iso_datetime("2024-3-1").as_deref(), Some("2024-03-01"));
        assert_eq!(iso_datetime("2024-03-05Z").as_deref(), Some("2024-03-05"));
    }

    #[test]
    fn civil_is_exact_beyond_chronos_range() {
        assert_eq!(civil(0), (1970, 1, 1));
        // Past `NaiveDate`'s ±262143 the 400-year cycles carry the year, exactly, both sides of
        // the epoch; values from the pre-chrono proleptic Gregorian formula.
        assert_eq!(civil(95_026_602), (262_144, 1, 1));
        assert_eq!(civil(99_000_000), (273_022, 10, 18));
        assert_eq!(civil(-96_465_964), (-262_145, 3, 1));
        assert_eq!(civil(-99_000_000), (-269_083, 3, 17));
        // The extremes must not overflow or panic (i128 intermediates).
        let _ = (civil(i64::MAX), civil(i64::MIN));
    }

    #[test]
    fn test_sheet_extended_value_parsing() {
        assert_eq!(iso_time("13:45:00").as_deref(), Some("13:45"));
        assert_eq!(iso_time("13:45:30.500").as_deref(), Some("13:45:30"));
        assert_eq!(iso_time("13:45:30Z").as_deref(), Some("13:45:30"));
        assert_eq!(iso_time("PT13H45M").as_deref(), Some("13:45"));

        // ODS with string-value and percentage without text:p, and case-insensitive float
        let content = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <office:body><office:spreadsheet><table:table table:name="Sheet1">
                <table:table-row>
                    <table:table-cell office:value-type="STRING" office:string-value="Hello"/>
                    <table:table-cell office:value-type="percentage" office:value="0.15"/>
                    <table:table-cell office:value-type="FLOAT" office:value="42"/>
                    <table:table-cell office:value-type="time" office:time-value="14:30:00"/>
                </table:table-row>
            </table:table></office:spreadsheet></office:body></office:document-content>"#;
        let book = zip(&[("mimetype", "application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", content)]);
        let (_, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], vec!["Hello", "0.15", "42", "14:30"]);

        // XLSX with case-insensitive cell types
        let sheet = r#"<worksheet><sheetData><row r="1">
            <c r="A1" t="B"><v>1</v></c>
            <c r="B1" t="INLINESTR"><is><t>World</t></is></c>
            <c r="C1" t="S"><v>0</v></c>
        </row></sheetData></worksheet>"#;
        let (_, rows) = read_rows(&xlsx(sheet), Pick::Index(0)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], vec!["TRUE", "World", "Name"]);
    }

    #[test]
    fn repeated_headers_are_numbered_in_linear_time() {
        // Every repeat used to restart at `Name 2` and compare each candidate with every name
        // so far: cubic in the number of equal headers.
        let row = vec!["Name".to_string(); 5_000];
        let t = std::time::Instant::now();
        let names = field_names(&row);
        // Milliseconds; the old numbering ran past 6 minutes.
        assert!(t.elapsed() < std::time::Duration::from_secs(10), "{:?}", t.elapsed());
        assert_eq!(names.len(), 5_000);
        assert_eq!(names[0], "Name");
        assert_eq!(names[4_999], "Name 5000");
        // Names a header already has stay collision-free, ignoring case.
        let row: Vec<String> = ["Name", "Name 2", "Name", "name", "", "Column 5", "NAME 2"].iter().map(|s| s.to_string()).collect();
        assert_eq!(field_names(&row), ["Name", "Name 2", "Name 3", "name 4", "Column 5", "Column 5 2", "NAME 2 2"]);
    }

    fn ods(rows: &str) -> Vec<u8> {
        let content = format!(
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Sheet1">{rows}</table:table></office:spreadsheet></office:body></office:document-content>"#
        );
        zip(&[("mimetype", "application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", &content)])
    }

    #[test]
    fn ods_string_attributes_are_capped_before_repeating() {
        // An `office:string-value` is a cell's text: it obeys MAX_CELL_CHARS like `<text:p>`
        // does, so repeating it across the columns can't build a huge row first.
        let long = "v".repeat(MAX_CELL_CHARS * 4);
        let book = ods(&format!(
            r#"<table:table-row><table:table-cell office:value-type="string" office:string-value="{long}" table:number-columns-repeated="3"/></table:table-row>"#
        ));
        let (_, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 3);
        assert!(rows[0].iter().all(|c| c.chars().count() == MAX_CELL_CHARS));

        // A row whose repeated text alone is over the text budget is dropped as it is built
        // (it was built in full, then cloned, then refused); the rows after it still count.
        let wide = "€".repeat(MAX_CELL_CHARS);
        let book = ods(&format!(
            r#"<table:table-row><table:table-cell office:value-type="string" office:string-value="{wide}" table:number-columns-repeated="1000"/></table:table-row><table:table-row><table:table-cell><text:p>next</text:p></table:table-cell></table:table-row>"#
        ));
        let ((_, rows), warned) = crate::test_log::warnings(|| read_rows(&book, Pick::Index(0)).unwrap());
        assert_eq!(rows, vec![vec!["next".to_string()]]);
        // The row left out is said to be (review of #3873).
        assert_eq!(warned.iter().filter(|w| w.contains("are left out of the recipient list")).count(), 1, "{warned:?}");
    }

    #[test]
    fn rows_over_the_text_budget_are_logged_once_per_sheet() {
        let wide = "€".repeat(MAX_CELL_CHARS);
        let over = format!(
            r#"<table:table-row><table:table-cell office:value-type="string" office:string-value="{wide}" table:number-columns-repeated="1000"/></table:table-row>"#
        );
        let book = ods(&format!("{over}{over}{over}<table:table-row><table:table-cell><text:p>next</text:p></table:table-cell></table:table-row>"));
        let ((_, rows), warned) = crate::test_log::warnings(|| read_rows(&book, Pick::Index(0)).unwrap());
        assert_eq!(rows, vec![vec!["next".to_string()]]);
        assert_eq!(warned.iter().filter(|w| w.contains("recipient list")).count(), 1, "one warning, not one per row: {warned:?}");
        let book = ods("<table:table-row><table:table-cell><text:p>fine</text:p></table:table-cell></table:table-row>");
        let (_, warned) = crate::test_log::warnings(|| read_rows(&book, Pick::Index(0)).unwrap());
        assert!(warned.is_empty(), "{warned:?}");
    }

    #[test]
    fn ods_repeated_text_is_budgeted_for_the_cells_the_grid_keeps() {
        // One cell left in the grid: of a long value repeated a thousand times only the first
        // copy is kept, and it fits the text budget, so the row isn't refused for the others.
        let wide = "€".repeat(MAX_CELL_CHARS);
        let full = MAX_CELLS / MAX_COLS - 1;
        let book = ods(&format!(
            r#"<table:table-row table:number-rows-repeated="{full}"><table:table-cell office:value-type="string" office:string-value="x" table:number-columns-repeated="{MAX_COLS}"/></table:table-row><table:table-row><table:table-cell office:value-type="string" office:string-value="x" table:number-columns-repeated="{}"/></table:table-row><table:table-row><table:table-cell office:value-type="string" office:string-value="{wide}" table:number-columns-repeated="1000"/></table:table-row>"#,
            MAX_COLS - 1
        ));
        let (_, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        assert_eq!(rows.len(), full + 2);
        assert_eq!(rows.last(), Some(&vec![wide]));
        assert_eq!(rows.iter().map(Vec::len).sum::<usize>(), MAX_CELLS);
    }
}
