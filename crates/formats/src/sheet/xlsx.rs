use super::date::{DateKind, builtin_date, custom_date, iso_datetime, serial_date};
use super::{
    BTreeMap, Ev, Grid, HashMap, MAX_CELLS, MAX_COLS, MAX_ROWS, MAX_SHARED, MAX_SHEETS, MAX_XFS, Package, TextBudget, attr, push_capped, walk,
};

// ---------------------------------------------------------------------------------------------
// SpreadsheetML

pub(super) struct Rel {
    id: String,
    pub(super) kind: String,
    pub(super) target: String,
}

/// The relationships of a part in `dir` (targets resolved to package paths).
pub(super) fn relationships(xml: &[u8], dir: &str) -> Result<Vec<Rel>, String> {
    let mut v = Vec::new();
    walk(xml, |ev| {
        if let Ev::Start(e) = ev
            && e.local_name().as_ref() == "Relationship"
            && v.len() < MAX_SHEETS * 2
        {
            if attr(e, "TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External")) {
                return;
            }
            let target = attr(e, "Target").unwrap_or_default();
            v.push(Rel { id: attr(e, "Id").unwrap_or_default(), kind: attr(e, "Type").unwrap_or_default(), target: join(dir, &target) });
        }
    })?;
    Ok(v)
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

pub(super) struct SheetRef {
    pub(super) name: String,
    pub(super) path: String,
}

struct Workbook {
    sheets: Vec<SheetRef>,
    date1904: bool,
    shared: Option<String>,
    styles: Option<String>,
}

fn workbook(pkg: &mut Package<'_>) -> Result<Workbook, String> {
    let path = pkg.workbook_path()?.ok_or("the workbook part is missing")?;
    let xml = pkg.read(&path)?;
    let rels = match pkg.read(&rels_of(&path)) {
        Ok(r) => relationships(&r, dir_of(&path))?,
        Err(_) => Vec::new(),
    };
    let mut listed: Vec<(String, String)> = Vec::new();
    let mut date1904 = false;
    walk(&xml, |ev| {
        if let Ev::Start(e) = ev {
            match e.local_name().as_ref() {
                "sheet" if listed.len() < MAX_SHEETS => listed.push((attr(e, "name").unwrap_or_default(), attr(e, "id").unwrap_or_default())),
                "workbookPr" => date1904 = attr(e, "date1904").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
                _ => {}
            }
        }
    })?;
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

pub(super) fn xlsx_sheets(pkg: &mut Package<'_>) -> Result<Vec<SheetRef>, String> {
    Ok(workbook(pkg)?.sheets)
}

/// The shared string table (§18.4): each `si`'s text, rich-text runs joined, phonetic runs
/// (`rPh`) left out.
fn shared_strings(xml: &[u8]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let (mut in_t, mut phonetic) = (false, 0usize);
    walk(xml, |ev| match ev {
        Ev::Start(e) => match e.local_name().as_ref() {
            "si" => cur = Some(String::new()),
            "t" => in_t = true,
            "rPh" => phonetic += 1,
            _ => {}
        },
        Ev::End(n) => match n.as_str() {
            "si" => {
                if let Some(s) = cur.take()
                    && out.len() < MAX_SHARED
                {
                    out.push(s);
                }
            }
            "t" => in_t = false,
            "rPh" => phonetic = phonetic.saturating_sub(1),
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
    })?;
    Ok(out)
}

/// Each cell format's (`cellXfs/xf`, by `s` index) date kind.
fn cell_formats(xml: &[u8]) -> Result<Vec<Option<DateKind>>, String> {
    let mut custom: HashMap<u32, Option<DateKind>> = HashMap::new();
    let mut xfs: Vec<u32> = Vec::new();
    let mut in_xfs = false;
    walk(xml, |ev| match ev {
        Ev::Start(e) => match e.local_name().as_ref() {
            "numFmt" if custom.len() < MAX_XFS => {
                if let Some(id) = attr(e, "numFmtId").and_then(|v| v.trim().parse().ok()) {
                    custom.insert(id, custom_date(&attr(e, "formatCode").unwrap_or_default()));
                }
            }
            "cellXfs" => in_xfs = true,
            "xf" if in_xfs && xfs.len() < MAX_XFS => xfs.push(attr(e, "numFmtId").and_then(|v| v.trim().parse().ok()).unwrap_or(0)),
            _ => {}
        },
        Ev::End(n) if n.as_str() == "cellXfs" => in_xfs = false,
        _ => {}
    })?;
    Ok(xfs.into_iter().map(|id| custom.get(&id).copied().unwrap_or_else(|| builtin_date(id))).collect())
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

pub(super) fn xlsx_rows(pkg: &mut Package<'_>, path: &str) -> Result<Vec<Vec<String>>, String> {
    let wb = workbook(pkg)?;
    let shared = match wb.shared.as_deref().or(pkg.has("xl/sharedStrings.xml").then_some("xl/sharedStrings.xml")).map(str::to_string) {
        Some(p) => shared_strings(&pkg.read(&p)?).map_err(|e| format!("{p}: {e}"))?,
        None => Vec::new(),
    };
    let formats = match wb.styles.as_deref().or(pkg.has("xl/styles.xml").then_some("xl/styles.xml")).map(str::to_string) {
        Some(p) => cell_formats(&pkg.read(&p)?).map_err(|e| format!("{p}: {e}"))?,
        None => Vec::new(),
    };
    let xml = pkg.read(path)?;
    // Rows by number, so out-of-order rows still come out in order.
    let mut rows: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    let mut cells = 0usize;
    let mut budget = TextBudget::default();
    let (mut in_data, mut row_no, mut next_col) = (false, 0u64, 0usize);
    let mut cell: Option<XCell> = None;
    let (mut in_v, mut in_is_t, mut phonetic) = (false, false, 0usize);
    walk(&xml, |ev| match ev {
        Ev::Start(e) => match e.local_name().as_ref() {
            "sheetData" => in_data = true,
            "row" if in_data => {
                row_no = attr(e, "r").and_then(|v| v.trim().parse().ok()).unwrap_or(row_no.saturating_add(1));
                next_col = 0;
            }
            "c" if in_data => {
                // A reference that isn't one (or is past the last column) drops the cell.
                let col = match attr(e, "r") {
                    Some(r) => ref_col(r.trim()).unwrap_or(usize::MAX),
                    None => next_col,
                };
                next_col = col.saturating_add(1);
                let style = attr(e, "s").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                cell = Some(XCell { col, kind: attr(e, "t").unwrap_or_default(), style, ..Default::default() });
            }
            "v" => in_v = cell.is_some(),
            "t" => in_is_t = cell.is_some(),
            "rPh" => phonetic += 1,
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
        Ev::End(n) => match n.as_str() {
            "sheetData" => in_data = false,
            "v" => in_v = false,
            "t" => in_is_t = false,
            "rPh" => phonetic = phonetic.saturating_sub(1),
            "c" => {
                let Some(c) = cell.take() else { return };
                let text = xlsx_value(&c, &shared, &formats, wb.date1904);
                if text.is_empty() || c.col >= MAX_COLS {
                    return;
                }
                if !rows.contains_key(&row_no) && rows.len() >= MAX_ROWS {
                    return;
                }
                if !budget.take(text.len()) {
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
    })?;
    let mut grid = Grid::default();
    for (_, r) in rows {
        grid.push(r, 1);
    }
    Ok(grid.rows)
}

fn xlsx_value(c: &XCell, shared: &[String], formats: &[Option<DateKind>], date1904: bool) -> String {
    let v = c.value.trim();
    let kind = c.kind.trim();
    if kind.eq_ignore_ascii_case("s") {
        v.parse::<usize>().ok().and_then(|i| shared.get(i)).cloned().unwrap_or_default()
    } else if kind.eq_ignore_ascii_case("inlineStr") {
        if c.inline.is_empty() { c.value.clone() } else { c.inline.clone() }
    } else if kind.eq_ignore_ascii_case("str") || kind.eq_ignore_ascii_case("e") {
        c.value.clone()
    } else if kind.eq_ignore_ascii_case("b") {
        if v == "1" || v.eq_ignore_ascii_case("true") { "TRUE" } else { "FALSE" }.into()
    } else if kind.eq_ignore_ascii_case("d") {
        iso_datetime(v).unwrap_or_else(|| v.to_string())
    } else if v.is_empty() {
        c.inline.clone()
    } else {
        let Ok(x) = v.parse::<f64>() else { return v.to_string() };
        match formats.get(c.style).copied().flatten() {
            Some(kind) => serial_date(x, date1904, kind).unwrap_or_else(|| number(x)),
            None => number(x),
        }
    }
}

/// A number as a spreadsheet shows it in General format: up to 15 significant digits, no
/// trailing zeros.
pub(super) fn number(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        // `+ 0.0` turns negative zero into positive zero, which would print as "-0".
        return format!("{:.0}", x + 0.0);
    }
    let rounded: f64 = format!("{x:.14e}").parse().unwrap_or(x);
    if rounded.abs() >= 1e15 || rounded.abs() < 1e-9 { format!("{rounded:E}") } else { rounded.to_string() }
}
