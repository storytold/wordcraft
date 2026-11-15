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
        let formats = &read.workbook.formats;
        let date_1904 = read.workbook.date_1904;
        let kind = |c: &craft_xls::Cell| match (&c.value, c.format.and_then(|f| formats.get(usize::from(f)))) {
            (Value::Number(n), Some(f)) => NumFmt::Code(&f.number_format).date_kind(*n),
            _ => None,
        };
        return Ok((names, cells_to_rows(&sheet.cells, |c| display(&c.value, kind(c), date_1904))));
    }
    if let Ok(mut pkg) = craft_xlsx::Package::open(bytes, &craft_xlsx::Limits::default())
        && let Ok(wb) = pkg.workbook()
    {
        let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
        let i = resolve(&names, pick)?;
        let strings = pkg.shared_strings().map_err(|e| e.to_string())?;
        let styles = pkg.styles().ok().flatten();
        let formats = styles.as_ref().map(xlsx_formats).unwrap_or_default();
        let mut grid: BTreeMap<(u32, u32), String> = BTreeMap::new();
        pkg.sheet(&wb.sheets[i], |row| {
            if (row.index as usize) < MAX_ROWS {
                for cell in &row.cells {
                    if (cell.at.col as usize) < MAX_COLS {
                        let value = cell.value.resolve(&strings, wb.date_1904);
                        let kind = match (&value, &cell.value) {
                            // A `t="d"` cell is shown as written, whatever its style: a date, a time, or both.
                            (Value::Number(_), craft_xlsx::CellValue::Date(text)) => {
                                Some(match (text.split(['T', ':']).next().is_some_and(|d| d.contains('-')), text.contains(':')) {
                                    (true, true) => DateKind::DateTime,
                                    (false, true) => DateKind::Time,
                                    _ => DateKind::Date,
                                })
                            }
                            (Value::Number(n), _) => formats.get(cell.style as usize).and_then(|f| f.date_kind(*n)),
                            _ => None,
                        };
                        let s = match (&value, &cell.value, kind) {
                            // A `t="d"` cell whose serial has no calendar day, such as 1900-02-29,
                            // keeps its text.
                            (Value::Number(n), craft_xlsx::CellValue::Date(text), Some(kind)) => {
                                serial_date(*n, wb.date_1904, kind).unwrap_or_else(|| clamp_cell(text.clone()))
                            }
                            // A `t="d"` cell the workbook's date system cannot hold, such as a date
                            // before 1904 in the 1904 system, keeps its text.
                            (Value::Empty, craft_xlsx::CellValue::Date(text), _) if iso_date_text(text) => clamp_cell(text.clone()),
                            _ => display(&value, kind, wb.date_1904),
                        };
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
                number(*n)
            } else {
                String::new()
            }
        }
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Error(e) => e.to_string(),
    }
}

/// A cell's display string: a number whose format shows a date or time in ISO 8601 form, anything
/// else as [`format_value`] gives it.
fn display(v: &Value, kind: Option<DateKind>, date_1904: bool) -> String {
    match (v, kind) {
        (Value::Number(n), Some(kind)) => serial_date(*n, date_1904, kind).unwrap_or_else(|| format_value(v)),
        _ => format_value(v),
    }
}

/// Whether `text` is an ISO 8601 date, with or without a time, in any year: its day exists in its
/// month and year, or it is 1900-02-29, which Excel counts as a day, and
/// `craft_xlsx::iso_date_serial` accepts it with its year replaced by 2000.
fn iso_date_text(text: &str) -> bool {
    text.trim().split_once('-').is_some_and(|(year, rest)| {
        !year.is_empty()
            && year.bytes().all(|b| b.is_ascii_digit())
            && day_in_month(year, rest)
            && craft_xlsx::iso_date_serial(&format!("2000-{rest}"), false).is_some()
    })
}

/// Whether the `MM-DD` at the start of `rest` names a day that `year` (ASCII digits) has.
fn day_in_month(year: &str, rest: &str) -> bool {
    let Some((month, day)) = rest.split(['T', ' ']).next().and_then(|md| md.split_once('-')) else {
        return false;
    };
    let (Ok(month), Ok(day)) = (month.parse::<u32>(), day.parse::<u32>()) else {
        return false;
    };
    // The last four digits give the year modulo 10,000, which decides the leap year.
    let y = year[year.len().saturating_sub(4)..].parse::<u32>().unwrap_or(0);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let last = match month {
        2 if leap || year.trim_start_matches('0') == "1900" => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=last).contains(&day)
}

/// A number as a spreadsheet shows it in General format: up to 15 significant digits, no
/// trailing zeros.
fn number(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    // Whole numbers keep every digit the stored value has, so identifiers such as account
    // numbers are unchanged. From 1E21 on they are written as the shortest exponent that reads
    // back as the same value.
    if x.fract() == 0.0 {
        // `+ 0.0` turns negative zero into positive zero, which would print as "-0".
        return if x.abs() < 1e21 { format!("{:.0}", x + 0.0) } else { format!("{x:E}") };
    }
    let rounded: f64 = format!("{x:.14e}").parse().unwrap_or(x);
    if rounded.abs() >= 1e15 || rounded.abs() < 1e-9 { format!("{rounded:E}") } else { rounded.to_string() }
}

/// Most cell formats read from an `.xlsx` styles part.
const MAX_XFS: usize = 65_536;

/// The number format of each `.xlsx` cell format (`cellXfs`, by a cell's `s`): its custom format
/// code, else its built-in format.
fn xlsx_formats(styles: &craft_xlsx::Styles) -> Vec<NumFmt<'_>> {
    let custom: HashMap<u32, &str> = styles.num_fmts.iter().map(|(id, code)| (*id, code.as_str())).collect();
    styles
        .cell_xfs
        .iter()
        .take(MAX_XFS)
        .map(|xf| custom.get(&xf.num_fmt_id).map_or(NumFmt::Builtin(xf.num_fmt_id), |code| NumFmt::Code(code)))
        .collect()
}

/// A cell's number format: a built-in format (ECMA-376 Part 1 §18.8.30) or a format code.
#[derive(Clone, Copy, Debug)]
enum NumFmt<'a> {
    Builtin(u32),
    Code(&'a str),
}

impl NumFmt<'_> {
    /// Whether the format shows `value` as a date, a time or an elapsed time.
    fn date_kind(self, value: f64) -> Option<DateKind> {
        match self {
            NumFmt::Builtin(id) => builtin_date(id),
            NumFmt::Code(code) => format_section(code, value).and_then(custom_date),
        }
    }
}

/// Whether a number format shows a date, a time, both, or an elapsed time (`[h]`, `[m]`, `[s]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DateKind {
    Date,
    Time,
    DateTime,
    Elapsed(Elapsed),
}

/// The unit an elapsed-time format counts the whole duration in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Elapsed {
    Hours,
    Minutes,
    Seconds,
}

/// The built-in number formats (ECMA-376 Part 1 §18.8.30) that show dates or times.
fn builtin_date(id: u32) -> Option<DateKind> {
    match id {
        // 32 to 35, 55 and 56 are the East Asian time formats.
        14..=17 | 27..=31 | 36 | 50..=54 | 57 | 58 => Some(DateKind::Date),
        18..=21 | 32..=35 | 45 | 47 | 55 | 56 => Some(DateKind::Time),
        46 => Some(DateKind::Elapsed(Elapsed::Hours)),
        22 => Some(DateKind::DateTime),
        _ => None,
    }
}

/// The sections of a format code, split at `;` outside quotes, brackets, escapes and the
/// character after `_` or `*` padding.
fn format_sections(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut bracket, mut escaped) = (0, false, false, false);
    for (i, c) in code.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if !quoted => escaped = true,
            '_' | '*' if !quoted && !bracket => escaped = true,
            '"' if !bracket => quoted = !quoted,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            ';' if !quoted && !bracket => {
                out.push(code.get(start..i).unwrap_or_default());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(code.get(start..).unwrap_or_default());
    out
}

/// A section's leading condition (`[>=40000]`) applied to `value`; `None` when it has none.
fn section_condition(section: &str, value: f64) -> Option<bool> {
    // The condition may follow other leading brackets, such as a color.
    let mut rest = section.trim_start();
    let inner = loop {
        let (inner, after) = rest.strip_prefix('[')?.split_once(']')?;
        if inner.starts_with(['<', '>', '=']) {
            break inner;
        }
        rest = after.trim_start();
    };
    let (op, rest) = ["<=", ">=", "<>", "<", ">", "="].iter().find_map(|op| inner.strip_prefix(op).map(|r| (*op, r)))?;
    let limit: f64 = rest.trim().parse().ok()?;
    Some(match op {
        "<=" => value <= limit,
        ">=" => value >= limit,
        "<>" => value != limit,
        "<" => value < limit,
        ">" => value > limit,
        _ => value == limit,
    })
}

/// The section of a format code that shows `value` (§18.8.31): the first section whose condition
/// holds, else the first without one, else none; with no conditions, the positive, negative and
/// zero sections in that order.
fn format_section(code: &str, value: f64) -> Option<&str> {
    let sections = format_sections(code);
    let numeric = &sections[..sections.len().min(3)];
    if numeric.iter().any(|s| section_condition(s, value).is_some()) {
        return numeric
            .iter()
            .find(|s| section_condition(s, value) == Some(true))
            .or_else(|| numeric.iter().find(|s| section_condition(s, value).is_none()))
            .copied();
    }
    let i = if value < 0.0 && numeric.len() > 1 {
        1
    } else if value == 0.0 && numeric.len() > 2 {
        2
    } else {
        0
    };
    Some(numeric.get(i).copied().unwrap_or(code))
}

/// Whether a format section shows a date or time: without quoted text, escaped characters,
/// `[…]` (colors, locales, conditions) and `_x` / `*x` padding, it has `d` or `y` (date), `h` or
/// `s` (time), or `m` alone (a month). `[h]`, `[m]` or `[s]` makes it an elapsed time.
fn custom_date(code: &str) -> Option<DateKind> {
    let mut plain = String::new();
    let mut elapsed = None;
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
                let mut inner = String::new();
                for q in chars.by_ref() {
                    if q == ']' {
                        break;
                    }
                    inner.extend(q.to_lowercase());
                }
                let unit = match inner.chars().next() {
                    Some('h') => Some(Elapsed::Hours),
                    Some('m') => Some(Elapsed::Minutes),
                    Some('s') => Some(Elapsed::Seconds),
                    _ => None,
                };
                if elapsed.is_none() && inner.chars().all(|q| inner.starts_with(q)) {
                    elapsed = unit;
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
    if let (Some(unit), false) = (elapsed, date) {
        return Some(DateKind::Elapsed(unit));
    }
    match (date, time) {
        (true, true) => Some(DateKind::DateTime),
        (true, false) => Some(DateKind::Date),
        (false, true) => Some(DateKind::Time),
        (false, false) if has('m') && !has('0') && !has('#') => Some(DateKind::Date),
        _ => None,
    }
}

/// A serial date-time (days since the workbook's epoch) in ISO 8601 form.
fn serial_date(serial: f64, date1904: bool, kind: DateKind) -> Option<String> {
    // Day 2958465 is 9999-12-31.
    if !serial.is_finite() || !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let mut days = serial.floor() as i64;
    // The time of day to the millisecond.
    let mut ms = ((serial - serial.floor()) * 86_400_000.0).round() as i64;
    // A time that rounds up to midnight moves to the next day; a date alone stays on its day.
    if ms >= 86_400_000 {
        ms -= 86_400_000;
        if kind != DateKind::Date {
            days += 1;
        }
    }
    let (secs, millis) = (ms / 1000, ms % 1000);
    // Day 60 of the 1900 system is 29 February 1900, which never was.
    if !date1904 && days == 60 && matches!(kind, DateKind::Date | DateKind::DateTime) {
        return None;
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
    // Seconds appear when they are not zero, with their fraction when it is not zero.
    let clock = |secs: i64| {
        let (h, mi, s) = (secs / 3600, secs / 60 % 60, secs % 60);
        if millis != 0 {
            let fraction = format!("{millis:03}");
            format!("{h:02}:{mi:02}:{s:02}.{}", fraction.trim_end_matches('0'))
        } else if s == 0 {
            format!("{h:02}:{mi:02}")
        } else {
            format!("{h:02}:{mi:02}:{s:02}")
        }
    };
    if let DateKind::Elapsed(unit) = kind {
        // The whole duration in the format's largest unit.
        let total = days.saturating_mul(86_400).saturating_add(secs);
        return Some(match unit {
            Elapsed::Hours => clock(total),
            Elapsed::Minutes if total % 60 == 0 && millis == 0 => (total / 60).to_string(),
            Elapsed::Minutes => format!("{}:{}", total / 60, clock(total % 60).trim_start_matches("00:00:")),
            Elapsed::Seconds if millis == 0 => total.to_string(),
            Elapsed::Seconds => format!("{total}.{}", format!("{millis:03}").trim_end_matches('0')),
        });
    }
    let (y, m, d) = civil(days + epoch);
    if y > 9999 {
        return None;
    }
    let time = clock(secs);
    Some(match kind {
        DateKind::Date => format!("{y:04}-{m:02}-{d:02}"),
        DateKind::Time | DateKind::Elapsed(_) => time,
        DateKind::DateTime => format!("{y:04}-{m:02}-{d:02} {time}"),
    })
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day). The serial guard in
/// [`serial_date`] keeps the day inside chrono's range.
fn civil(z: i64) -> (i64, i64, i64) {
    use chrono::Datelike;
    match i32::try_from(z + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(d) => (i64::from(d.year()), i64::from(d.month()), i64::from(d.day())),
        None => (1970, 1, 1),
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

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const DOC_RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

    /// An `.xlsx` with one sheet of `rows` and a styles part with `num_fmts` and `xfs`.
    fn styled_workbook(rows: &str, num_fmts: &str, xfs: &str) -> Vec<u8> {
        workbook_with(rows, num_fmts, xfs, "")
    }

    /// [`styled_workbook`] with `book_pr` (such as `<workbookPr date1904="1"/>`) in the workbook part.
    fn workbook_with(rows: &str, num_fmts: &str, xfs: &str, book_pr: &str) -> Vec<u8> {
        let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#;
        let root = format!(
            r#"<Relationships xmlns="{RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
        );
        let book = format!(
            r#"<workbook xmlns="{MAIN}" xmlns:r="{DOC_RELS}">{book_pr}<sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#
        );
        let book_rels = format!(
            r#"<Relationships xmlns="{RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{DOC_RELS}/styles" Target="styles.xml"/></Relationships>"#
        );
        let sheet = format!(r#"<worksheet xmlns="{MAIN}"><sheetData>{rows}</sheetData></worksheet>"#);
        let styles = format!(r#"<styleSheet xmlns="{MAIN}"><numFmts>{num_fmts}</numFmts><cellXfs>{xfs}</cellXfs></styleSheet>"#);
        craft_xlsx::testing::zip(&[
            ("[Content_Types].xml", types.as_bytes()),
            ("_rels/.rels", root.as_bytes()),
            ("xl/workbook.xml", book.as_bytes()),
            ("xl/_rels/workbook.xml.rels", book_rels.as_bytes()),
            ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
            ("xl/styles.xml", styles.as_bytes()),
        ])
    }

    #[test]
    fn xlsx_dates_and_times_read_as_iso_dates() {
        let rows = r#"<row r="1"><c r="A1" s="1"><v>45356</v></c><c r="B1" s="2"><v>45356.5729166667</v></c><c r="C1" s="3"><v>0.5</v></c><c r="D1"><v>0.30000000000000004</v></c><c r="E1" t="d"><v>2024-03-05T13:45:00</v></c><c r="F1" t="d"><v>2024-03-05</v></c><c r="G1"><v>45356</v></c></row>"#;
        let book = styled_workbook(
            rows,
            r#"<numFmt numFmtId="164" formatCode="yyyy/mm/dd\ hh:mm"/>"#,
            r#"<xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="164"/><xf numFmtId="20"/>"#,
        );
        let (_, rows) = read_rows(&book, Pick::Index(0)).unwrap();
        assert_eq!(rows, [["2024-03-05", "2024-03-05 13:45", "12:00", "0.3", "2024-03-05 13:45", "2024-03-05", "45356"]]);
    }

    #[test]
    fn number_formats_that_show_dates() {
        assert_eq!(custom_date("yyyy-mm-dd"), Some(DateKind::Date));
        assert_eq!(custom_date("h:mm:ss"), Some(DateKind::Time));
        assert_eq!(custom_date("m/d/yy h:mm"), Some(DateKind::DateTime));
        assert_eq!(custom_date("mmm"), Some(DateKind::Date));
        assert_eq!(custom_date(r#"0.00" days""#), None);
        assert_eq!(custom_date("General"), None);
        assert_eq!(builtin_date(22), Some(DateKind::DateTime));
        assert_eq!(serial_date(1.0, true, DateKind::Date).as_deref(), Some("1904-01-02"));
        assert_eq!(serial_date(59.0, false, DateKind::Date).as_deref(), Some("1900-02-28"));
        assert_eq!(serial_date(61.0, false, DateKind::Date).as_deref(), Some("1900-03-01"));
        assert_eq!(number(1.0 / 3.0), "0.333333333333333");
        assert_eq!(number(1_000_000_000_000_001.0), "1000000000000001");
        assert_eq!(number(10_000_000_000_000_002.0), "10000000000000002");
    }

    #[test]
    fn elapsed_times_bounds_and_sections() {
        assert_eq!(custom_date("[h]:mm:ss"), Some(DateKind::Elapsed(Elapsed::Hours)));
        assert_eq!(serial_date(1.5, false, DateKind::Elapsed(Elapsed::Hours)).as_deref(), Some("36:00"));
        assert_eq!(custom_date("[mm]"), Some(DateKind::Elapsed(Elapsed::Minutes)));
        assert_eq!(serial_date(0.5, false, DateKind::Elapsed(Elapsed::Minutes)).as_deref(), Some("720"));
        assert_eq!(custom_date("[s]"), Some(DateKind::Elapsed(Elapsed::Seconds)));
        assert_eq!(serial_date(0.5, false, DateKind::Elapsed(Elapsed::Seconds)).as_deref(), Some("43200"));
        assert_eq!(builtin_date(46), Some(DateKind::Elapsed(Elapsed::Hours)));
        // 2957003 is 9999-12-31 in the 1904 system; the next day is not a date.
        assert_eq!(serial_date(2_957_003.0, true, DateKind::Date).as_deref(), Some("9999-12-31"));
        assert_eq!(serial_date(2_957_004.0, true, DateKind::Date), None);
        assert_eq!(serial_date(60.0, false, DateKind::Date), None);
        assert_eq!(serial_date(0.5 + 0.5 / 86_400.0, false, DateKind::Time).as_deref(), Some("12:00:00.5"));
        assert_eq!(serial_date(0.5 + 30.25 / 86_400.0, false, DateKind::Elapsed(Elapsed::Minutes)).as_deref(), Some("720:30.25"));
        assert_eq!(serial_date(45_356.999_999_999_9, false, DateKind::Date).as_deref(), Some("2024-03-05"));
        assert_eq!(serial_date(45_356.999_999_999_9, false, DateKind::DateTime).as_deref(), Some("2024-03-06 00:00"));
        assert_eq!(builtin_date(32), Some(DateKind::Time));
        assert_eq!(builtin_date(57), Some(DateKind::Date));
        assert_eq!(NumFmt::Code("[Red][>=40000]yyyy-mm-dd;0").date_kind(12.0), None);
        assert_eq!(NumFmt::Code("[Red][>=40000]yyyy-mm-dd;0").date_kind(45356.0), Some(DateKind::Date));
        // The section that shows the value decides.
        assert_eq!(NumFmt::Code("[>=40000]yyyy-mm-dd;0").date_kind(45356.0), Some(DateKind::Date));
        assert_eq!(NumFmt::Code("[>=40000]yyyy-mm-dd;0").date_kind(12.0), None);
        assert_eq!(NumFmt::Code("0;yyyy-mm-dd").date_kind(5.0), None);
        assert_eq!(NumFmt::Code(r#""a;b"yyyy"#).date_kind(5.0), Some(DateKind::Date));
    }

    #[test]
    fn date_cells_read_as_written() {
        let rows = r#"<row r="1"><c r="A1" t="d"><v>06:00</v></c><c r="B1" t="d"><v>2024-03-05T00:00:00</v></c></row>"#;
        let book = styled_workbook(rows, "", r#"<xf numFmtId="0"/>"#);
        assert_eq!(read_rows(&book, Pick::Index(0)).unwrap().1, [["06:00", "2024-03-05 00:00"]]);
    }

    #[test]
    fn a_styled_date_cell_keeps_its_time() {
        let rows = r#"<row r="1"><c r="A1" s="1" t="d"><v>2024-03-05T13:45:00</v></c><c r="B1" s="2" t="d"><v>2024-03-05</v></c></row>"#;
        let book = styled_workbook(rows, "", r#"<xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="20"/>"#);
        assert_eq!(read_rows(&book, Pick::Index(0)).unwrap().1, [["2024-03-05 13:45", "2024-03-05"]]);
    }

    #[test]
    fn a_date_cell_outside_the_date_system_keeps_its_text() {
        // 1900-02-29 has a serial (60) but no calendar day.
        let rows = r#"<row r="1"><c r="A1" t="d"><v>1899-12-31</v></c><c r="B1" s="1" t="d"><v>1899-12-31T08:30:00</v></c><c r="C1" t="d"><v>1900-02-29</v></c></row>"#;
        let book = styled_workbook(rows, "", r#"<xf numFmtId="0"/><xf numFmtId="14"/>"#);
        assert_eq!(read_rows(&book, Pick::Index(0)).unwrap().1, [["1899-12-31", "1899-12-31T08:30:00", "1900-02-29"]]);
        let rows = r#"<row r="1"><c r="A1" t="d"><v>1903-12-31</v></c><c r="B1" t="d"><v>1904-01-02</v></c></row>"#;
        let book = workbook_with(rows, "", r#"<xf numFmtId="0"/>"#, r#"<workbookPr date1904="1"/>"#);
        assert_eq!(read_rows(&book, Pick::Index(0)).unwrap().1, [["1903-12-31", "1904-01-02"]]);
        // Before 1904, a day the month does not have stays unread; 1900-02-29 is kept, as Excel has it.
        let rows = r#"<row r="1"><c r="A1" t="d"><v>1900-02-29</v></c><c r="B1" t="d"><v>1896-02-29</v></c><c r="C1" t="d"><v>1903-02-29</v></c><c r="D1" t="d"><v>1903-04-31T08:30:00</v></c></row>"#;
        let book = workbook_with(rows, "", r#"<xf numFmtId="0"/>"#, r#"<workbookPr date1904="1"/>"#);
        assert_eq!(read_rows(&book, Pick::Index(0)).unwrap().1, [["1900-02-29", "1896-02-29"]]);
        // Text that is not an ISO date stays unread.
        let rows = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1" t="d"><v>not-a-date</v></c><c r="C1" t="d"><v>1899-13-01</v></c></row>"#;
        let book = styled_workbook(rows, "", r#"<xf numFmtId="0"/>"#);
        assert_eq!(read_rows(&book, Pick::Index(0)).unwrap().1, [["1"]]);
    }

    #[test]
    fn large_whole_numbers_keep_their_value() {
        let big = 1_000_000_000_000_000_131_072.0;
        assert_eq!(number(big), "1.0000000000000001E21");
        assert_eq!(number(big).parse::<f64>(), Ok(big));
        assert_eq!(number(1e21), "1E21");
        assert_eq!(number(-1e300), "-1E300");
    }

    #[test]
    fn padding_and_unmatched_conditions_in_sections() {
        assert_eq!(NumFmt::Code("mm_;ss").date_kind(0.5), Some(DateKind::Time));
        assert_eq!(NumFmt::Code("[>40000]0;[=0]yyyy-mm-dd").date_kind(12.0), None);
        assert_eq!(NumFmt::Code("[>40000]0;[=0]yyyy-mm-dd").date_kind(0.0), Some(DateKind::Date));
        assert_eq!(NumFmt::Code("[Red] [>=40000]yyyy-mm-dd;0").date_kind(12.0), None);
    }
}
