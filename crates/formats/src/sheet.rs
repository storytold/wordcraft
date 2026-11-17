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
        let rows = cells_to_rows(&sheet.cells, ods_value);
        return Ok((names, rows));
    }
    Err("not an Excel or OpenDocument spreadsheet".into())
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn xls_fixture(sheets: &[(&str, Vec<Vec<String>>)]) -> Result<Vec<u8>, String> {
    craft_xls::testing::workbook(sheets).map_err(|e| e.to_string())
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

/// An OpenDocument cell's display string, from `office:value-type`: floats from `office:value`,
/// dates and times from `office:date-value` / `office:time-value` in ISO 8601 form (`yyyy-mm-dd`,
/// `yyyy-mm-dd hh:mm[:ss]`, `hh:mm[:ss]`), booleans as `TRUE`/`FALSE`, and strings, percentages
/// and currencies as the cell's text (their value when the text is blank). A value that does not
/// parse falls back to the text. A covered cell under a merged cell is blank.
fn ods_value(c: &craft_ods::Cell) -> String {
    use craft_ods::ValueType as T;
    if c.covered {
        return String::new();
    }
    let typed = match (c.value_type, c.raw.as_deref().map(str::trim)) {
        (Some(T::Float | T::Boolean), Some(_)) => c.value().map(|v| format_value(&v)),
        (Some(T::Date), Some(v)) => iso_datetime(v),
        (Some(T::Time), Some(v)) => iso_time(v),
        (Some(T::Percentage | T::Currency | T::String), Some(_)) if c.text.is_empty() => c.value().map(|v| format_value(&v)),
        _ => None,
    };
    typed.unwrap_or_else(|| clamp_cell(c.text.clone()))
}

/// An ISO 8601 date or date-time (`2024-03-05`, `2024-03-05T13:45:00.000`) as `yyyy-mm-dd`,
/// with ` hh:mm[:ss]` when the time isn't midnight. `None` unless the whole value is a valid
/// calendar date and clock time.
fn iso_datetime(v: &str) -> Option<String> {
    use chrono::Datelike;
    let v = v.trim();
    let (date, time) = match v.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (v, None),
    };
    // A date alone may carry a zone (`Z` or `±hh:mm`), which is left out.
    let date = match (time, date.strip_suffix('Z')) {
        (None, Some(d)) => d,
        (None, None) if date.len() > 6 && date.get(date.len() - 6..).is_some_and(|z| z.starts_with(['+', '-']) && z.get(3..4) == Some(":")) => {
            zone_offset(date.get(date.len() - 5..)?)?;
            date.get(..date.len() - 6)?
        }
        _ => date,
    };
    let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let mut out = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());
    if let Some(time) = time {
        let clock = clock_time(time)?;
        if clock != chrono::NaiveTime::MIN {
            out.push(' ');
            out.push_str(&hh_mm(clock));
        }
    }
    Some(out)
}

/// A clock time `hh:mm[:ss[.s…]]` with an optional `Z` or `±hh:mm` zone (one, not both), which is
/// left out.
fn clock_time(t: &str) -> Option<chrono::NaiveTime> {
    let t = match (t.strip_suffix('Z'), t.rfind(['+', '-'])) {
        (Some(t), _) => t,
        (None, Some(i)) => {
            zone_offset(t.get(i + 1..)?)?;
            t.get(..i)?
        }
        (None, None) => t,
    };
    let clock = chrono::NaiveTime::parse_from_str(t, "%H:%M:%S%.f").or_else(|_| chrono::NaiveTime::parse_from_str(t, "%H:%M")).ok()?;
    // A leap second (`23:59:60`) is not a time a cell holds.
    (chrono::Timelike::nanosecond(&clock) < 1_000_000_000).then_some(clock)
}

/// A zone offset's `hh:mm`, two digits each and at most `14:00` as XML Schema allows.
fn zone_offset(z: &str) -> Option<()> {
    use chrono::Timelike;
    let two_digit = z.len() == 5 && z.bytes().enumerate().all(|(i, b)| if i == 2 { b == b':' } else { b.is_ascii_digit() });
    if !two_digit {
        return None;
    }
    let z = chrono::NaiveTime::parse_from_str(z, "%H:%M").ok()?;
    (z.hour() < 14 || (z.hour() == 14 && z.minute() == 0)).then_some(())
}

/// `hh:mm`, with `:ss` when the seconds aren't zero and their fraction when it isn't.
fn hh_mm(t: chrono::NaiveTime) -> String {
    use chrono::Timelike;
    let (h, m, s, nanos) = (t.hour(), t.minute(), t.second(), t.nanosecond());
    if nanos != 0 {
        let fraction = format!("{nanos:09}");
        format!("{h:02}:{m:02}:{s:02}.{}", fraction.trim_end_matches('0'))
    } else if s == 0 {
        format!("{h:02}:{m:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

/// An ISO 8601 duration (`PT13H45M00S`, `P1DT2H`, `-PT1H`) or clock time (`13:45[:00]`) as
/// `hh:mm[:ss]`, a duration's days counted as hours and a negative one with a leading `-`.
/// `None` for anything else, years and months included.
fn iso_time(v: &str) -> Option<String> {
    let t = v.trim();
    let (negative, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let Some(t) = t.strip_prefix('P') else {
        return if negative { None } else { clock_time(t).map(hh_mm) };
    };
    let (days, t, has_t) = match t.split_once('T') {
        Some((days, time)) => (days, time, true),
        None => (t, "", false),
    };
    // A `T` needs a time after it.
    if has_t && t.is_empty() {
        return None;
    }
    let mut any = !days.is_empty();
    let days: u64 = if days.is_empty() { 0 } else { days.strip_suffix('D')?.parse().ok()? };
    let (mut h, mut m, mut s) = (0u64, 0u64, 0u64);
    // The seconds' fraction digits, kept as written.
    let mut fraction = String::new();
    let mut num = String::new();
    // Units come in the order H, M, S, each at most once.
    let mut next_unit = 0;
    for c in t.chars() {
        let unit = match c {
            'H' => 1,
            'M' => 2,
            'S' => 3,
            '0'..='9' | '.' if num.len() < 20 => {
                num.push(c);
                continue;
            }
            _ => return None,
        };
        if unit <= next_unit {
            return None;
        }
        next_unit = unit;
        let n = std::mem::take(&mut num);
        match unit {
            1 => h = n.parse().ok()?,
            2 => m = n.parse().ok()?,
            _ => {
                let (whole, frac) = n.split_once('.').unwrap_or((&n, ""));
                if whole.is_empty() || (n.contains('.') && frac.is_empty()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                s = whole.parse().ok()?;
                fraction = frac.trim_end_matches('0').to_string();
            }
        }
        any = true;
    }
    if !any || !num.is_empty() {
        return None;
    }
    // A duration too long to count in hours is not a time.
    m = m.checked_add(s / 60)?;
    s %= 60;
    h = h.checked_add(m / 60)?.checked_add(days.checked_mul(24)?)?;
    m %= 60;
    let sign = if negative && ((h, m, s) != (0, 0, 0) || !fraction.is_empty()) { "-" } else { "" };
    Some(if !fraction.is_empty() {
        format!("{sign}{h:02}:{m:02}:{s:02}.{fraction}")
    } else if s == 0 {
        format!("{sign}{h:02}:{m:02}")
    } else {
        format!("{sign}{h:02}:{m:02}:{s:02}")
    })
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

    #[test]
    fn ods_cells_read_their_typed_values() {
        let book = craft_ods::testing::spreadsheet(
            r#"<table:table table:name="Guests">
              <table:table-row><table:table-cell office:value-type="string"><text:p>Name</text:p></table:table-cell><table:table-cell table:number-columns-repeated="2"/><table:table-cell office:value-type="string"><text:p>When</text:p></table:table-cell><table:table-cell office:value-type="string" table:number-columns-repeated="2"><text:p>Tag</text:p></table:table-cell></table:table-row>
              <table:table-row table:number-rows-repeated="2"><table:table-cell office:value-type="string" table:number-columns-spanned="2"><text:p>Ada<text:s text:c="2"/>L</text:p><text:p>x</text:p><office:annotation><text:p>note</text:p></office:annotation></table:table-cell><table:covered-table-cell office:value-type="string"><text:p>hidden</text:p></table:covered-table-cell><table:table-cell office:value-type="float" office:value="3.50"><text:p>3,5</text:p></table:table-cell><table:table-cell office:value-type="date" office:date-value="2024-03-05T00:00:00"><text:p>05/03/24</text:p></table:table-cell><table:table-cell office:value-type="boolean" office:boolean-value="false"><text:p>NO</text:p></table:table-cell><table:table-cell office:value-type="time" office:time-value="PT13H45M00S"><text:p>01:45 PM</text:p></table:table-cell></table:table-row>
              <table:table-row><table:table-cell office:value-type="date" office:date-value="2024-03-05T13:45:30"><text:p>Tue</text:p></table:table-cell><table:table-cell office:value-type="date" office:date-value="not a date"><text:p>someday</text:p></table:table-cell><table:table-cell office:value-type="percentage" office:value="0.25"><text:p>25%</text:p></table:table-cell><table:table-cell office:value-type="currency" office:value="9"/></table:table-row>
            </table:table>"#,
        )
        .unwrap();
        let (_, rows) = read_rows(&book, Pick::Name("Guests")).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], ["Name", "", "", "When", "Tag", "Tag"]);
        assert_eq!(rows[1], ["Ada  L\nx", "", "3.5", "2024-03-05", "FALSE", "13:45"]);
        assert_eq!(rows[1], rows[2]);
        assert_eq!(rows[3], ["2024-03-05 13:45:30", "someday", "25%", "9"]);
    }

    #[test]
    fn an_ods_date_with_two_zones_reads_its_text() {
        let book = craft_ods::testing::spreadsheet(
            r#"<table:table table:name="Dates">
              <table:table-row><table:table-cell office:value-type="date" office:date-value="2024-03-05T13:45:00+02:00Z"><text:p>offset then Z</text:p></table:table-cell><table:table-cell office:value-type="date" office:date-value="2024-03-05T13:45:00Z+02:00"><text:p>Z then offset</text:p></table:table-cell><table:table-cell office:value-type="date" office:date-value="2024-03-05T13:45:00+02:00"><text:p>one zone</text:p></table:table-cell></table:table-row>
            </table:table>"#,
        )
        .unwrap();
        let (_, rows) = read_rows(&book, Pick::Name("Dates")).unwrap();
        assert_eq!(rows, [["offset then Z", "Z then offset", "2024-03-05 13:45"]]);
    }

    #[test]
    fn ods_dates_and_durations() {
        assert_eq!(iso_time("PT1H59M59.6S").as_deref(), Some("01:59:59.6"));
        assert_eq!(iso_time("PT0.25S").as_deref(), Some("00:00:00.25"));
        assert_eq!(iso_time("PT59M60.50S").as_deref(), Some("01:00:00.5"));
        assert_eq!(iso_time("PT0H0M59S").as_deref(), Some("00:00:59"));
        assert_eq!(iso_time("PT1H2M3S").as_deref(), Some("01:02:03"));
        assert_eq!(iso_time("PT1.S"), None);
        assert_eq!(iso_time("P1000000000000000000D"), None);
    }

    #[test]
    fn a_date_alone_drops_its_zone() {
        assert_eq!(iso_datetime("2024-03-05Z").as_deref(), Some("2024-03-05"));
        assert_eq!(iso_datetime("2024-03-05+02:00").as_deref(), Some("2024-03-05"));
        assert_eq!(iso_datetime("2024-03-05-05:00").as_deref(), Some("2024-03-05"));
        assert_eq!(iso_datetime("2024-03-05+02:99"), None);
    }

    #[test]
    fn a_zone_is_at_most_fourteen_hours() {
        assert_eq!(iso_datetime("2024-03-05+14:00").as_deref(), Some("2024-03-05"));
        assert_eq!(iso_datetime("2024-03-05-14:00").as_deref(), Some("2024-03-05"));
        assert_eq!(iso_datetime("2024-03-05+14:01"), None);
        assert_eq!(iso_datetime("2024-03-05-15:00"), None);
        assert_eq!(iso_datetime("2024-03-05T13:45:00+14:00").as_deref(), Some("2024-03-05 13:45"));
        assert_eq!(iso_datetime("2024-03-05T13:45:00+14:01"), None);
        assert_eq!(iso_time("13:45:00-15:00"), None);
    }

    #[test]
    fn a_zone_is_z_or_an_offset_not_both() {
        assert_eq!(iso_datetime("2024-03-05T13:45:00+02:00Z"), None);
        assert_eq!(iso_datetime("2024-03-05T13:45:00Z+02:00"), None);
        assert_eq!(iso_datetime("2024-03-05+02:00Z"), None);
        assert_eq!(iso_datetime("2024-03-05Z+02:00"), None);
        assert_eq!(iso_time("13:45:00+02:00Z"), None);
        assert_eq!(iso_time("13:45:00Z+02:00"), None);
        assert_eq!(iso_datetime("2024-03-05T13:45:00Z").as_deref(), Some("2024-03-05 13:45"));
        assert_eq!(iso_datetime("2024-03-05T13:45:00-05:00").as_deref(), Some("2024-03-05 13:45"));
        assert_eq!(iso_time("13:45:00+02:00").as_deref(), Some("13:45"));
    }

    #[test]
    fn a_zone_offset_has_two_digit_hours_and_minutes() {
        assert_eq!(iso_datetime("2024-03-05T13:45:00+2:00"), None);
        assert_eq!(iso_datetime("2024-03-05T13:45:00+ 2:00"), None);
        assert_eq!(iso_time("13:45:00-5:00"), None);
    }

    #[test]
    fn a_leap_second_is_not_a_time() {
        assert_eq!(iso_datetime("2024-03-05T23:59:60Z"), None);
        assert_eq!(iso_time("23:59:60"), None);
    }

    #[test]
    fn a_duration_with_t_needs_a_time() {
        assert_eq!(iso_time("P1DT"), None);
        assert_eq!(iso_time("P0DT"), None);
        assert_eq!(iso_time("P1D").as_deref(), Some("24:00"));
        assert_eq!(iso_time("PT.S"), None);
        assert_eq!(iso_time("13:45:00Z").as_deref(), Some("13:45"));
        assert_eq!(iso_time("-PT1H").as_deref(), Some("-01:00"));
        assert_eq!(iso_time("P2DT1H30M").as_deref(), Some("49:30"));
        assert_eq!(iso_time("P1D").as_deref(), Some("24:00"));
        assert_eq!(iso_time("P1Y2M"), None);
        assert_eq!(iso_time("PT"), None);
        assert_eq!(iso_time("PT5"), None);
        assert_eq!(iso_time("PT1H2H"), None);
        assert_eq!(iso_time("PT1S2H"), None);
        assert_eq!(iso_datetime("2024-03-05T"), None);
        assert_eq!(iso_datetime("2024-03-05T00:00:00.500").as_deref(), Some("2024-03-05 00:00:00.5"));
        assert_eq!(iso_time("00:00:00.250").as_deref(), Some("00:00:00.25"));
        assert_eq!(iso_time("25:00"), None);
        assert_eq!(iso_datetime("2024-03-05T00:00:00.000").as_deref(), Some("2024-03-05"));
        assert_eq!(iso_datetime("2024-03-05T13:45:30+02:00").as_deref(), Some("2024-03-05 13:45:30"));
        assert_eq!(iso_datetime("2024-13-05"), None);
        assert_eq!(iso_datetime("2023-02-29"), None);
        assert_eq!(iso_datetime("2024-03-05junk"), None);
        assert_eq!(iso_datetime("2024-03-05T13:99"), None);
    }
}
