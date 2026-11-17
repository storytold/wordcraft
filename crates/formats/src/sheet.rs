//! Spreadsheets as mail-merge recipient lists via `craft-ods`, `craft-xls`, and `craft-xlsx`.

use std::collections::{BTreeMap, HashMap, HashSet};

use craft_sheet::{CellRef, Value};

pub const EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xls", "ods"];
pub const MAX_ROWS: usize = 100_000;
pub const MAX_COLS: usize = 1_000;
pub const MAX_CELL_CHARS: usize = 32_767;

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
        if let Ok(mut pkg) = craft_xlsx::Package::open(bytes, &craft_xlsx::Limits::default())
            && pkg.workbook().is_ok()
        {
            return Some(Kind::Xlsx);
        }
        if craft_ods::read(bytes, &craft_ods::Options::default()).is_ok() {
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
    if let Ok(mut pkg) = craft_xlsx::Package::open(bytes, &craft_xlsx::Limits::default())
        && let Ok(wb) = pkg.workbook()
    {
        return Ok(wb.sheets.into_iter().map(|s| s.name).collect());
    }
    if let Ok(book) = craft_ods::read(bytes, &craft_ods::Options::default()) {
        return Ok(book.sheets.into_iter().map(|s| s.name).collect());
    }
    Err("not an Excel or OpenDocument spreadsheet".into())
}

pub fn read_rows(bytes: &[u8], pick: Pick<'_>) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    if craft_xls::is_xls(bytes) {
        let read = craft_xls::read(bytes).map_err(|e| e.to_string())?;
        let names: Vec<String> = read.workbook.sheets.iter().map(|s| s.name.clone()).collect();
        let i = resolve(&names, pick)?;
        let sheet = &read.workbook.sheets[i];
        return Ok((names, cells_to_rows(&sheet.cells, |c| format_value(&c.value))));
    }
    if let Ok(mut pkg) = craft_xlsx::Package::open(bytes, &craft_xlsx::Limits::default())
        && let Ok(wb) = pkg.workbook()
    {
        let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
        let i = resolve(&names, pick)?;
        let strings = pkg.shared_strings().map_err(|e| e.to_string())?;
        let mut grid: BTreeMap<(u32, u32), String> = BTreeMap::new();
        pkg.sheet(&wb.sheets[i], |row| {
            if (row.index as usize) < MAX_ROWS {
                for cell in &row.cells {
                    if (cell.at.col as usize) < MAX_COLS {
                        let s = format_value(&cell.value.resolve(&strings, wb.date_1904));
                        if !s.is_empty() {
                            grid.insert((cell.at.row, cell.at.col), s);
                        }
                    }
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        return Ok((names, grid_to_rows(&grid)));
    }
    if let Ok(book) = craft_ods::read(bytes, &craft_ods::Options::default()) {
        let names: Vec<String> = book.sheets.iter().map(|s| s.name.clone()).collect();
        let i = resolve(&names, pick)?;
        let sheet = &book.sheets[i];
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
        Value::Text(s) => clamp_cell(s.to_string()),
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
    let mut grid: BTreeMap<(u32, u32), String> = BTreeMap::new();
    for (r, c) in cells {
        if (r.row as usize) < MAX_ROWS && (r.col as usize) < MAX_COLS {
            let s = fmt(c);
            if !s.is_empty() {
                grid.insert((r.row, r.col), s);
            }
        }
    }
    grid_to_rows(&grid)
}

fn grid_to_rows(grid: &BTreeMap<(u32, u32), String>) -> Vec<Vec<String>> {
    let mut by_row: BTreeMap<u32, BTreeMap<u32, String>> = BTreeMap::new();
    for ((r, c), v) in grid {
        by_row.entry(*r).or_default().insert(*c, v.clone());
    }
    by_row
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
