//! A chart's embedded workbook (an `.xlsx` package in the document): the chart's cells written
//! into its sheet, so the workbook and the chart's caches agree.
//!
//! Only the sheet the chart refers to changes, and in it only the cells of the old data and the
//! new: categories down column A from row 2, series names along row 1 from column B, values
//! under them. Each cell keeps its style. A table (`xl/tables/…`) over the old data is resized to
//! the new data, its columns named after the header row, as Word keeps it. Every other part of
//! the package is copied as it was.

use craft_xml::{El, Local, Node};

use super::{ChartData, EditError, column, err, number_text, sheet_name};

/// Most entries read from a workbook package.
const MAX_ENTRIES: u64 = 4096;
/// Most bytes all its entries may expand to.
const MAX_TOTAL: u64 = 256 << 20;
/// Largest workbook part read.
const MAX_ENTRY: u64 = 64 << 20;

/// The workbook `xlsx` with `data` written into sheet `sheet` (a reference as the chart writes
/// it, `Sheet1` or `'My data'`; `None`: the first sheet), clearing the cells of the old data
/// (`old_rows` × `old_cols`, header row and category column included) the new data doesn't
/// cover.
pub fn edit_workbook(xlsx: &[u8], sheet: Option<&str>, data: &ChartData, old_rows: usize, old_cols: usize) -> Result<Vec<u8>, EditError> {
    let mut entries = read_entries(xlsx)?;
    let Some(path) = sheet_path(&entries, sheet) else { return err("the chart's sheet isn't in its workbook") };
    let new_rows = data.points() + 1;
    let new_cols = data.series.len() + 1;
    let sheet_bytes = entry(&entries, &path).ok_or_else(|| EditError("the chart's sheet is missing from its workbook".into()))?;
    let mut ws = craft_xml::parse(sheet_bytes).map_err(|_| EditError("the chart's sheet can't be read".into()))?;
    let tables = table_paths(&entries, &path);
    let mut header: Vec<String> = std::iter::once(header_a1(&ws)).chain(data.series.iter().map(|s| s.name.clone())).collect();
    let resized_table = !tables.is_empty();
    if resized_table {
        unique_names(&mut header);
    }
    {
        let Some(sheet_data) = ws.child_mut(Local("sheetData")) else { return err("the chart's sheet has no cells") };
        if sheet_data.children(Local("row")).any(|r| r.attr("r").is_none()) {
            return err("the chart's sheet numbers its rows in a way WordCraft can't edit");
        }
        clear(sheet_data, old_rows.max(new_rows), old_cols.max(new_cols), new_rows, new_cols);
        write_cells(sheet_data, data, if resized_table { Some(&header) } else { None });
    }
    set_dimension(&mut ws);
    let mut out = craft_xml::DECL_STANDALONE.to_string();
    out.push_str(&ws.to_xml());
    put(&mut entries, &path, out.into_bytes());
    for t in tables {
        if let Some(bytes) = entry(&entries, &t)
            && let Some(new) = resize_table(bytes, new_rows, &header)
        {
            put(&mut entries, &t, new);
        }
    }
    write_entries(&entries)
}

fn read_entries(xlsx: &[u8]) -> Result<Vec<(String, Vec<u8>)>, EditError> {
    let limits = craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: MAX_TOTAL };
    let mut zip = craft_zip::Archive::open(xlsx, limits).map_err(|_| EditError("the chart's workbook can't be opened".into()))?;
    if zip.has_duplicate_names() {
        return err("the chart's workbook has two parts of one name");
    }
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let Ok(Some(name)) = zip.entry_name(i) else { continue };
        if zip.entry_is_dir(i).ok().flatten().unwrap_or(false) {
            continue;
        }
        match zip.read_index(i, MAX_ENTRY) {
            Ok(Some(bytes)) => out.push((name, bytes)),
            _ => return err("a part of the chart's workbook can't be read"),
        }
    }
    Ok(out)
}

fn write_entries(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, EditError> {
    let mut w = craft_zip::Writer::new();
    for (name, bytes) in entries {
        w.add(name, bytes, craft_zip::Compression::Deflated).map_err(|_| EditError("the chart's workbook can't be written".into()))?;
    }
    w.finish().map_err(|_| EditError("the chart's workbook can't be written".into()))
}

fn entry<'a>(entries: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    entries.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, b)| b.as_slice())
}

fn put(entries: &mut [(String, Vec<u8>)], name: &str, bytes: Vec<u8>) {
    if let Some(slot) = entries.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        slot.1 = bytes;
    }
}

/// The part a relationship target names, from the part `from` (`xl/workbook.xml`).
fn resolve(from: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn rels_of(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// The relationships of `part`: (id, type, target part).
fn rels(entries: &[(String, Vec<u8>)], part: &str) -> Vec<(String, String, String)> {
    let Some(root) = entry(entries, &rels_of(part)).and_then(|b| craft_xml::parse(b).ok()) else { return Vec::new() };
    root.children(Local("Relationship"))
        .filter(|r| r.attr("TargetMode") != Some("External"))
        .filter_map(|r| Some((r.attr("Id")?.to_string(), r.attr("Type")?.to_string(), resolve(part, r.attr("Target")?))))
        .collect()
}

/// The workbook part: the package's office document.
fn workbook_part(entries: &[(String, Vec<u8>)]) -> String {
    rels(entries, "").into_iter().find(|(_, t, _)| t.ends_with("/officeDocument")).map(|(_, _, p)| p).unwrap_or_else(|| "xl/workbook.xml".to_string())
}

/// The part of sheet `sheet` (by name; `None`: the first sheet).
fn sheet_path(entries: &[(String, Vec<u8>)], sheet: Option<&str>) -> Option<String> {
    let wb_part = workbook_part(entries);
    let wb = craft_xml::parse(entry(entries, &wb_part)?).ok()?;
    let sheets = wb.child(Local("sheets"))?;
    let want = sheet.map(sheet_name);
    let el = sheets.children(Local("sheet")).find(|s| want.as_deref().is_none_or(|w| s.attr("name").is_some_and(|n| n.eq_ignore_ascii_case(w))))?;
    let id = el.attrs.iter().find(|(k, _)| k.ends_with(":id"))?.1.clone();
    rels(entries, &wb_part).into_iter().find(|(i, _, _)| *i == id).map(|(_, _, p)| p)
}

/// The table parts of the sheet at `sheet`.
fn table_paths(entries: &[(String, Vec<u8>)], sheet: &str) -> Vec<String> {
    rels(entries, sheet).into_iter().filter(|(_, t, _)| t.ends_with("/table")).map(|(_, _, p)| p).collect()
}

/// A cell reference (`B12`) as (column, row), both from 0.
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(char::is_ascii_alphabetic).collect();
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let col = letters.bytes().try_fold(0usize, |acc, b| Some(acc * 26 + usize::from(b.to_ascii_uppercase().checked_sub(b'A')?) + 1))?;
    let row: usize = r.get(letters.len()..)?.parse().ok()?;
    (row >= 1).then(|| (col - 1, row - 1))
}

/// The text cell A1 holds, when it is an inline or plain string (Word's sheets leave it empty).
fn header_a1(ws: &El) -> String {
    let Some(cell) = ws
        .child(Local("sheetData"))
        .and_then(|d| d.children(Local("row")).next())
        .and_then(|r| r.children(Local("c")).find(|c| c.attr("r") == Some("A1")))
    else {
        return String::new();
    };
    match cell.attr("t") {
        Some("inlineStr") => cell.find(Local("t")).map(|t| t.text().into_owned()).unwrap_or_default(),
        Some("str") => cell.child(Local("v")).map(|v| v.text().into_owned()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Make every name non-empty and unique (a table's column names must be), as Excel does:
/// `Column1`, `Series 1 2`.
fn unique_names(names: &mut [String]) {
    let mut seen: Vec<String> = Vec::new();
    for (i, n) in names.iter_mut().enumerate() {
        if n.trim().is_empty() {
            *n = if i == 0 { " ".to_string() } else { format!("Column{}", i + 1) };
        }
        let mut name = n.clone();
        let mut k = 2;
        while seen.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            name = format!("{n}{k}");
            k += 1;
        }
        *n = name.clone();
        seen.push(name);
    }
}

/// Clear the cells of the old data (`rows` × `cols`) the new data (`keep_rows` × `keep_cols`)
/// doesn't cover: their values go, their styles stay.
fn clear(sheet_data: &mut El, rows: usize, cols: usize, keep_rows: usize, keep_cols: usize) {
    for row in sheet_data.elements_mut().filter(|r| r.local() == "row") {
        let Some(r) = row.attr("r").and_then(|r| r.parse::<usize>().ok()).and_then(|r| r.checked_sub(1)) else { continue };
        if r >= rows {
            continue;
        }
        row.children.retain_mut(|n| {
            let Node::El(c) = n else { return true };
            let Some((col, _)) = c.attr("r").and_then(cell_ref) else { return true };
            let gone = col < cols && (r >= keep_rows || col >= keep_cols) && !(r == 0 && col == 0);
            if gone {
                empty_cell(c);
                return c.attr("s").is_some();
            }
            true
        });
    }
}

/// A cell without a value (it keeps its style).
fn empty_cell(c: &mut El) {
    c.remove_attr("t");
    c.children.retain(|n| !matches!(n, Node::El(e) if matches!(e.local(), "v" | "is" | "f")));
}

/// What a cell holds.
enum Cell<'a> {
    Text(&'a str),
    Number(f64),
    Empty,
}

fn write_cells(sheet_data: &mut El, data: &ChartData, header: Option<&[String]>) {
    let n = data.points();
    if let Some(h) = header
        && let Some(a1) = h.first()
    {
        set_cell(sheet_data, 0, 0, if a1.trim().is_empty() { Cell::Empty } else { Cell::Text(a1) });
    }
    for (j, s) in data.series.iter().enumerate() {
        let name = header.and_then(|h| h.get(j + 1)).map_or(s.name.as_str(), String::as_str);
        set_cell(sheet_data, 0, j + 1, Cell::Text(name));
    }
    for i in 0..n {
        let cat = data.categories.get(i).map_or("", String::as_str);
        let cell = match cat.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => Cell::Number(v),
            _ if cat.is_empty() => Cell::Empty,
            _ => Cell::Text(cat),
        };
        set_cell(sheet_data, i + 1, 0, cell);
        for (j, s) in data.series.iter().enumerate() {
            let v = s.values.get(i).copied().flatten();
            set_cell(sheet_data, i + 1, j + 1, v.map_or(Cell::Empty, Cell::Number));
        }
    }
}

/// Write `value` into the cell at (`row`, `col`), adding its row and cell in order when missing.
fn set_cell(sheet_data: &mut El, row: usize, col: usize, value: Cell<'_>) {
    let rnum = row + 1;
    let ri = match sheet_data.children.iter().position(|n| matches!(n, Node::El(r) if r.local() == "row" && row_num(r) >= rnum)) {
        Some(i) if matches!(sheet_data.children.get(i), Some(Node::El(r)) if row_num(r) == rnum) => i,
        found => {
            let at = found.unwrap_or(sheet_data.children.len());
            let name = row_name(sheet_data);
            sheet_data.children.insert(at, Node::El(El::new(name).with_attr("r", rnum)));
            at
        }
    };
    let Some(Node::El(r)) = sheet_data.children.get_mut(ri) else { return };
    let reference = format!("{}{rnum}", column(col));
    let cell_name = format!("{}c", prefix(&r.name));
    let ci = match r.children.iter().position(|n| matches!(n, Node::El(c) if c.local() == "c" && cell_col(c) >= col)) {
        Some(i) if matches!(r.children.get(i), Some(Node::El(c)) if cell_col(c) == col) => i,
        found => {
            if matches!(value, Cell::Empty) {
                return;
            }
            let at = found.unwrap_or(r.children.len());
            r.children.insert(at, Node::El(El::new(cell_name).with_attr("r", &reference)));
            at
        }
    };
    let Some(Node::El(c)) = r.children.get_mut(ci) else { return };
    let p = prefix(&c.name).to_string();
    empty_cell(c);
    match value {
        Cell::Text(t) => {
            c.set_attr("t", "inlineStr");
            let mut t_el = El::new(format!("{p}t")).with_text(t);
            if t.starts_with(char::is_whitespace) || t.ends_with(char::is_whitespace) {
                t_el.set_attr("xml:space", "preserve");
            }
            c.push(El::new(format!("{p}is")).with_child(t_el));
        }
        Cell::Number(v) => c.push(El::new(format!("{p}v")).with_text(number_text(v))),
        Cell::Empty => {}
    }
    // A cell without a value or a style is left out.
    if c.children.is_empty() && c.attr("s").is_none() {
        r.children.remove(ci);
    }
}

/// The name's prefix with its colon (`x:`), or empty.
fn prefix(name: &str) -> &str {
    match name.find(':') {
        Some(i) => name.get(..=i).unwrap_or(""),
        None => "",
    }
}

fn row_name(sheet_data: &El) -> String {
    format!("{}row", prefix(&sheet_data.name))
}

fn row_num(r: &El) -> usize {
    r.attr("r").and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn cell_col(c: &El) -> usize {
    c.attr("r").and_then(cell_ref).map_or(usize::MAX, |(col, _)| col)
}

/// Set `dimension` to the cells the sheet holds.
fn set_dimension(ws: &mut El) {
    let mut last: Option<(usize, usize)> = None;
    if let Some(d) = ws.child(Local("sheetData")) {
        for (col, row) in d.children(Local("row")).flat_map(|r| r.children(Local("c"))).filter_map(|c| c.attr("r").and_then(cell_ref)) {
            last = Some(last.map_or((col, row), |(c, r)| (c.max(col), r.max(row))));
        }
    }
    let Some((col, row)) = last else { return };
    if let Some(dim) = ws.child_mut(Local("dimension")) {
        dim.set_attr("ref", format!("A1:{}{}", column(col), row + 1));
    }
}

/// A table part resized to `rows` rows from A1, its columns named `header`. `None` when the
/// table doesn't start at A1 (it isn't the chart's).
fn resize_table(bytes: &[u8], rows: usize, header: &[String]) -> Option<Vec<u8>> {
    let mut t = craft_xml::parse(bytes).ok()?;
    let r = t.attr("ref")?;
    if !r.to_ascii_uppercase().starts_with("A1:") {
        return None;
    }
    let reference = format!("A1:{}{}", column(header.len().saturating_sub(1)), rows.max(2));
    t.set_attr("ref", &reference);
    if let Some(af) = t.child_mut(Local("autoFilter")) {
        af.set_attr("ref", &reference);
    }
    let p = prefix(&t.name).to_string();
    let cols = t.child_mut(Local("tableColumns"))?;
    let old: Vec<El> = cols.children(Local("tableColumn")).cloned().collect();
    cols.children.clear();
    for (i, name) in header.iter().enumerate() {
        let mut col = old.get(i).cloned().unwrap_or_else(|| El::new(format!("{p}tableColumn")));
        col.set_attr("id", i + 1);
        col.set_attr("name", name);
        cols.push(col);
    }
    cols.set_attr("count", header.len());
    let mut out = craft_xml::DECL_STANDALONE.to_string();
    out.push_str(&t.to_xml());
    Some(out.into_bytes())
}
