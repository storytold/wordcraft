//! Spreadsheets as mail-merge recipient lists via `craft-ods`, `craft-xls`, and `craft-xlsx`.

use std::collections::{BTreeMap, HashMap, HashSet};

use craft_sheet::{CellRef, Value};

pub const EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xls", "ods"];
pub const MAX_ROWS: usize = 100_000;
pub const MAX_COLS: usize = 1_000;
pub const MAX_CELL_CHARS: usize = 32_767;
/// Most cells kept in all (blank cells between values count).
pub const MAX_CELLS: usize = 2_000_000;
/// Most bytes of cell text kept from one sheet: a shared string (or a repeated cell) is stored
/// once in the file but copied into every cell that uses it, so a small file could otherwise
/// ask for many gigabytes. Cells past the budget are ignored.
pub const MAX_TEXT_BYTES: usize = 64 << 20;
/// Largest zip entry inflated.
pub const MAX_ENTRY: u64 = 64 << 20;
/// Most bytes inflated from one workbook.
pub const MAX_TOTAL: u64 = 160 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Xlsx,
    Xls,
    Ods,
}

#[derive(Clone, Copy, Debug)]
pub enum Pick<'a> {
    Index(usize),
    Name(&'a str),
}

pub fn is_extension(ext: &str) -> bool {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str())
}

pub fn detect(bytes: &[u8]) -> Option<Kind> {
    if craft_xls::is_xls(bytes) {
        return Some(Kind::Xls);
    }
    if bytes.starts_with(b"PK") {
        if let Ok(mut pkg) = craft_xlsx::Package::open(bytes, &xlsx_limits())
            && pkg.workbook().is_ok()
        {
            return Some(Kind::Xlsx);
        }
        // The mimetype entry alone, without parsing the cells.
        if craft_ods::is_ods(bytes) {
            return Some(Kind::Ods);
        }
    }
    None
}

pub fn sheet_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    if craft_xls::is_xls(bytes) {
        let read = craft_xls::read(bytes).map_err(|e| e.to_string())?;
        return Ok(read.workbook.sheets.into_iter().map(|s| s.name).collect());
    }
    if let Ok(mut pkg) = craft_xlsx::Package::open(bytes, &xlsx_limits())
        && let Ok(wb) = pkg.workbook()
    {
        return Ok(wb.sheets.into_iter().map(|s| s.name).collect());
    }
    if let Ok(book) = craft_ods::read(bytes, &ods_options(Some(usize::MAX))) {
        return Ok(book.sheets.into_iter().map(|s| s.name).collect());
    }
    Err("not an Excel or OpenDocument spreadsheet".into())
}

/// The package and XML limits for an `.xlsx`: the parts a recipient list reads, inflated up to
/// [`MAX_ENTRY`] each and [`MAX_TOTAL`] in all.
fn xlsx_limits() -> craft_xlsx::Limits {
    let mut limits = craft_xlsx::Limits::default();
    limits.package.max_entries = crate::MAX_ZIP_ENTRIES as usize;
    limits.package.max_part = MAX_ENTRY;
    limits.package.max_total = MAX_TOTAL;
    limits.package.max_work = MAX_TOTAL;
    limits.xml.max_input = MAX_ENTRY as usize;
    limits
}

/// The package and text limits for an `.ods`, keeping the cells of `only_sheet` alone
/// (`Some(usize::MAX)`: none, for the sheet names).
fn ods_options(only_sheet: Option<usize>) -> craft_ods::Options {
    craft_ods::Options {
        package: craft_odf::Limits { max_entries: crate::MAX_ZIP_ENTRIES, max_entry: MAX_ENTRY, max_total: MAX_TOTAL },
        max_text: MAX_TEXT_BYTES as u64,
        only_sheet,
        ..craft_ods::Options::default()
    }
}

pub fn read_rows(bytes: &[u8], pick: Pick<'_>) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    if craft_xls::is_xls(bytes) {
        let read = craft_xls::read(bytes).map_err(|e| e.to_string())?;
        let names: Vec<String> = read.workbook.sheets.iter().map(|s| s.name.clone()).collect();
        let i = resolve(&names, pick)?;
        let sheet = &read.workbook.sheets[i];
        return Ok((names, cells_to_rows(&sheet.cells, |c| format_value(&c.value))));
    }
    if let Ok(mut pkg) = craft_xlsx::Package::open(bytes, &xlsx_limits())
        && let Ok(wb) = pkg.workbook()
    {
        let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
        let i = resolve(&names, pick)?;
        let strings = pkg.shared_strings().map_err(|e| e.to_string())?;
        let mut grid = Grid::default();
        pkg.sheet(&wb.sheets[i], |row| {
            for cell in &row.cells {
                grid.insert(cell.at.row, cell.at.col, || format_value(&cell.value.resolve(&strings, wb.date_1904)));
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        return Ok((names, grid.into_rows()));
    }
    if let Ok(listed) = craft_ods::read(bytes, &ods_options(Some(usize::MAX))) {
        let names: Vec<String> = listed.sheets.into_iter().map(|s| s.name).collect();
        let i = resolve(&names, pick)?;
        let book = craft_ods::read(bytes, &ods_options(Some(i))).map_err(|e| e.to_string())?;
        let sheet = book.sheets.get(i).ok_or("no such sheet")?;
        let rows = cells_to_rows(&sheet.cells, |c| {
            if !c.text.is_empty() {
                clamp_cell(c.text.clone())
            } else if let Some(v) = c.value() {
                format_value(&v)
            } else {
                String::new()
            }
        });
        return Ok((names, rows));
    }
    Err("not an Excel or OpenDocument spreadsheet".into())
}

#[doc(hidden)]
pub fn xls_fixture(sheets: &[(&str, Vec<Vec<String>>)]) -> Vec<u8> {
    craft_xls::testing::workbook(sheets)
}

pub fn field_names(row: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(row.len());
    let mut taken: HashSet<String> = HashSet::with_capacity(row.len());
    let mut next: HashMap<String, usize> = HashMap::new();
    for (i, h) in row.iter().enumerate() {
        let base = match h.trim() {
            "" => format!("Column {}", i + 1),
            t => t.to_string(),
        };
        let key = base.to_ascii_lowercase();
        if taken.insert(key.clone()) {
            out.push(base);
        } else {
            let mut n = next.get(&key).copied().unwrap_or(2);
            loop {
                let candidate = format!("{base} {n}");
                let ck = candidate.to_ascii_lowercase();
                if taken.insert(ck) {
                    next.insert(key, n + 1);
                    out.push(candidate);
                    break;
                }
                n += 1;
            }
        }
    }
    out
}

fn resolve(names: &[String], pick: Pick<'_>) -> Result<usize, String> {
    match pick {
        Pick::Index(i) => (i < names.len()).then_some(i).ok_or_else(|| "no such sheet".into()),
        Pick::Name(want) => names
            .iter()
            .position(|n| n == want)
            .or_else(|| names.iter().position(|n| n.eq_ignore_ascii_case(want)))
            .ok_or_else(|| format!("no sheet named `{want}`")),
    }
}

fn clamp_cell(mut s: String) -> String {
    if s.chars().count() > MAX_CELL_CHARS {
        s = s.chars().take(MAX_CELL_CHARS).collect();
    }
    s
}

fn format_value(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Text(s) => s.chars().take(MAX_CELL_CHARS).collect(),
        Value::Number(n) => {
            if n.is_finite() {
                if (*n - n.trunc()).abs() == 0.0 && n.abs() < 1e15 { format!("{:.0}", n) } else { format!("{n}") }
            } else {
                String::new()
            }
        }
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Error(e) => e.to_string(),
    }
}

fn cells_to_rows<C>(cells: &BTreeMap<CellRef, C>, fmt: impl Fn(&C) -> String) -> Vec<Vec<String>> {
    let mut grid = Grid::default();
    for (r, c) in cells {
        grid.insert(r.row, r.col, || fmt(c));
    }
    grid.into_rows()
}

/// Cells kept under the caps: cells past [`MAX_ROWS`] or [`MAX_COLS`] are ignored, and once a cell
/// would take the cells kept past [`MAX_CELLS`] (blank cells before a value in its row count) or
/// their text past [`MAX_TEXT_BYTES`], it and every cell after it are ignored.
#[derive(Default)]
struct Grid {
    rows: BTreeMap<u32, BTreeMap<u32, String>>,
    /// Cells the rows hold, blank ones included: each row is as wide as its last value.
    cells: usize,
    text: usize,
    full: bool,
}

impl Grid {
    /// Keep the text `value` makes at `row`, `col` if it fits. `value` is not called for a cell
    /// that is ignored for its position or after the grid is full.
    fn insert(&mut self, row: u32, col: u32, value: impl FnOnce() -> String) {
        if self.full || row as usize >= MAX_ROWS || col as usize >= MAX_COLS {
            return;
        }
        let s = value();
        if s.is_empty() {
            return;
        }
        let cols = self.rows.get(&row);
        let width = cols.and_then(|cols| cols.keys().next_back()).map_or(0, |&c| c as usize + 1);
        // A cell written twice keeps its last text.
        let replaced = cols.and_then(|cols| cols.get(&col)).map_or(0, String::len);
        let cells = self.cells.saturating_add((col as usize + 1).saturating_sub(width));
        let text = self.text.saturating_sub(replaced).saturating_add(s.len());
        if cells > MAX_CELLS || text > MAX_TEXT_BYTES {
            self.full = true;
            log::warn!("sheet: cells past {MAX_CELLS} cells or {MAX_TEXT_BYTES} bytes of text are left out of the recipient list");
            return;
        }
        self.cells = cells;
        self.text = text;
        self.rows.entry(row).or_default().insert(col, s);
    }

    /// The rows with a value, each as wide as its last value.
    fn into_rows(self) -> Vec<Vec<String>> {
        self.rows
            .into_values()
            .filter_map(|cols| {
                let last = *cols.keys().next_back()?;
                let mut row = vec![String::new(); (last as usize) + 1];
                for (c, v) in cols {
                    row[c as usize] = v;
                }
                Some(row)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One long shared string in thousands of cells stays under the text budget.
    #[test]
    fn a_shared_string_in_many_cells_stays_under_the_text_budget() {
        let long = "x".repeat(MAX_CELL_CHARS);
        let rows: String = (1..=3_000).map(|r| format!(r#"<row r="{r}"><c r="A{r}" t="s"><v>0</v></c></row>"#)).collect();
        let book = craft_xlsx::testing::workbook(&[("S", &rows)], &[&long]);
        assert_eq!(detect(&book), Some(Kind::Xlsx));
        let (_, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        let total: usize = rows.iter().flatten().map(String::len).sum();
        assert!(total <= MAX_TEXT_BYTES, "{total} bytes of cell text");
        assert!(rows.len() > 1_000, "the budget still keeps a real list: {}", rows.len());
    }

    /// Blank cells before a value far to the right count against the cell budget.
    #[test]
    fn values_far_right_stay_under_the_cell_budget() {
        // Column ALL is the 1,000th column.
        let rows: String = (1..=2_100).map(|r| format!(r#"<row r="{r}"><c r="ALL{r}"><v>1</v></c></row>"#)).collect();
        let book = craft_xlsx::testing::workbook(&[("S", &rows)], &[]);
        let (_, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        assert!(rows.iter().all(|r| r.len() == MAX_COLS && r[MAX_COLS - 1] == "1"));
        let cells: usize = rows.iter().map(Vec::len).sum();
        assert!(cells <= MAX_CELLS, "{cells} cells");
        assert_eq!(rows.len(), MAX_CELLS / MAX_COLS);
    }

    /// A cell written twice is charged for its last text only.
    #[test]
    fn a_rewritten_cell_is_charged_once() {
        let mut grid = Grid::default();
        grid.insert(0, 0, || "x".repeat(MAX_TEXT_BYTES - 1));
        grid.insert(0, 0, || "short".to_string());
        grid.insert(1, 0, || "next".to_string());
        assert!(!grid.full);
        assert_eq!(grid.into_rows(), [["short"], ["next"]]);
    }

    #[test]
    fn ods_names_and_one_sheet_read() {
        let book = craft_ods::testing::spreadsheet(
            r#"<table:table table:name="First"><table:table-row><table:table-cell office:value-type="string"><text:p>a</text:p></table:table-cell></table:table-row></table:table><table:table table:name="Guests"><table:table-row><table:table-cell office:value-type="string"><text:p>Name</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell office:value-type="string"><text:p>Ada</text:p></table:table-cell></table:table-row></table:table>"#,
        );
        assert_eq!(detect(&book), Some(Kind::Ods));
        assert_eq!(sheet_names(&book).unwrap(), ["First", "Guests"]);
        assert_eq!(
            read_rows(&book, Pick::Name("guests")).unwrap(),
            (vec!["First".to_string(), "Guests".to_string()], vec![vec!["Name".to_string()], vec!["Ada".to_string()]])
        );
        assert_eq!(read_rows(&book, Pick::Index(0)).unwrap().1, [["a"]]);
        assert!(read_rows(&book, Pick::Index(2)).is_err());
    }
}
