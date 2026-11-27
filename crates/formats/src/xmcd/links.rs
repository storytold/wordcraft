use std::collections::HashMap;
use std::io::{Cursor, Read};

use base64::Engine as _;
use craft_sheet::Value;

use super::Node;

const FILE_IMPORT: &str = "01350083-1122-11DB-9380-000D56C6051A";
const COMPOUND_SIGNATURE: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";
const MAX_LINK_BYTES: usize = 256 * 1024;

pub(super) fn relative_link(component: &Node, binary: &HashMap<&str, &Node>) -> Option<String> {
    if !component.attr("clsid-buddy").eq_ignore_ascii_case(FILE_IMPORT) || component.attr("disable-calc") == "true" {
        return None;
    }
    let item = binary.get(component.attr("item-idref"))?;
    if item.text.len() > MAX_LINK_BYTES {
        return None;
    }
    let encoded: Vec<_> = item.text.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let bytes = match item.attr("content-encoding") {
        "gzip" => read_link(flate2::read::GzDecoder::new(bytes.as_slice()))?,
        "" => bytes,
        _ => return None,
    };
    let start = bytes.windows(COMPOUND_SIGNATURE.len()).position(|window| window == COMPOUND_SIGNATURE)?;
    let mut storage = cfb::CompoundFile::open(Cursor::new(bytes.get(start..)?)).ok()?;
    let record = read_link(storage.open_stream("/Contents").ok()?)?;
    link_path(&record)
}

fn read_link(reader: impl Read) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_LINK_BYTES as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= MAX_LINK_BYTES).then_some(bytes)
}

fn link_path(record: &[u8]) -> Option<String> {
    if !record.starts_with(&[5, 0, 13, 0]) {
        return None;
    }
    let mut strings = record.get(66..)?;
    let path = take_string(&mut strings)?;
    for _ in 0..3 {
        if !take_string(&mut strings)?.is_empty() {
            return None;
        }
    }
    if !strings.is_empty() {
        return None;
    }
    let path = path.replace('\\', "/");
    let absolute = path.starts_with('/') || path.get(1..2) == Some(":");
    let path = if path.get(1..2) == Some(":") { path.get(2..)? } else { &path };
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    if parts.iter().any(|part| part.is_empty() || matches!(*part, "." | "..") || part.contains([':', '%', '?', '#', '\0']))
        || !parts.last()?.to_ascii_lowercase().ends_with(".xls")
    {
        return None;
    }
    let parts = if absolute { &parts[parts.len().saturating_sub(2)..] } else { &parts };
    Some(parts.join("/"))
}

fn take_string(record: &mut &[u8]) -> Option<String> {
    let bytes = record.strip_prefix(&[0xff, 0xfe, 0xff])?;
    let len = usize::from(*bytes.first()?);
    let text = bytes.get(1..1 + 2 * len)?;
    *record = bytes.get(1 + 2 * len..)?;
    String::from_utf16(&text.as_chunks::<2>().0.iter().copied().map(u16::from_le_bytes).collect::<Vec<_>>()).ok()
}

pub(super) fn matrix(bytes: &[u8]) -> Option<Node> {
    let workbook =
        craft_xls::read_with(bytes, craft_xls::Limits { max_records: 100_000, max_cells: 16_384, max_text: 2 * 1024 * 1024 }).ok()?.workbook;
    let mut sheets = workbook.sheets.iter().filter(|sheet| sheet.cells.values().any(|cell| !cell.value.is_empty()));
    let sheet = sheets.next()?;
    if sheets.next().is_some() {
        return None;
    }
    let mut rows = 0;
    let mut cols = 0;
    for (at, cell) in &sheet.cells {
        if !cell.value.is_empty() {
            rows = rows.max(at.row as usize + 1);
            cols = cols.max(at.col as usize + 1);
        }
    }
    if rows.checked_mul(cols)? > 4096 || cols > wordcraft_doc::table::MAX_COLS {
        return None;
    }
    let mut cells = vec![Node { ns: 2, name: "empty".into(), ..Node::default() }; rows * cols];
    for (at, cell) in &sheet.cells {
        let (name, text) = match &cell.value {
            Value::Number(number) => ("real", number.to_string()),
            Value::Text(text) => ("str", text.to_string()),
            Value::Bool(value) => ("str", value.to_string()),
            Value::Error(error) => ("str", error.to_string()),
            Value::Empty => continue,
        };
        cells[at.col as usize * rows + at.row as usize] = Node { ns: 2, name: name.into(), text, ..Node::default() };
    }
    let (rows, cols, cells) = coefficient_block(rows, cols, &cells).unwrap_or((rows, cols, cells));
    Some(Node {
        ns: 2,
        name: "matrix".into(),
        attrs: HashMap::from([("rows".into(), rows.to_string()), ("cols".into(), cols.to_string())]),
        children: cells,
        ..Node::default()
    })
}

/// Some coefficient workbooks repeat the same titled table in transposed and annotated
/// forms below the original. Their first block is the rectangular calculation input.
fn coefficient_block(rows: usize, cols: usize, cells: &[Node]) -> Option<(usize, usize, Vec<Node>)> {
    let at = |r: usize, c: usize| cells.get(c.checked_mul(rows)?.checked_add(r)?);
    let title = at(0, 0)?;
    let subtitle = at(1, 0)?;
    let marker = at(2, 0)?;
    if title.name != "str"
        || subtitle.name != "str"
        || marker.name != "str"
        || marker.text.trim() != "Coefficients"
        || title.text.trim().is_empty()
        || subtitle.text.trim().is_empty()
    {
        return None;
    }
    let end = (4..rows.saturating_sub(2)).find(|&row| {
        at(row, 0).is_some_and(|cell| cell.text == title.text)
            && at(row + 1, 0).is_some_and(|cell| cell.text == subtitle.text)
            && at(row + 2, 0).is_some_and(|cell| cell.text == marker.text)
    })?;
    let horizontal_width = (1..cols).take_while(|&col| at(2, col).is_some_and(|cell| cell.name == "real")).count() + 1;
    if horizontal_width >= 2
        && (3..end).all(|row| {
            at(row, 0).is_some_and(|cell| cell.name == "str") && (1..horizontal_width).all(|col| at(row, col).is_some_and(|cell| cell.name == "real"))
        })
        && (0..end).all(|row| (horizontal_width..cols).all(|col| at(row, col).is_none_or(|cell| cell.name == "empty")))
    {
        let block = (0..horizontal_width).flat_map(|col| (0..end).filter_map(move |row| at(row, col).cloned())).collect();
        return Some((end, horizontal_width, block));
    }
    // In some files the first block has formula cells without readable cached values. A later
    // copy has numeric cells, with the same heading and the coefficient vectors laid out as rows.
    let marker_row = end + 2;
    let coefficients =
        (1..cols).take_while(|&col| at(marker_row, col).is_some_and(|cell| cell.name == "str" && !cell.text.trim().is_empty())).count();
    if coefficients == 0 {
        return None;
    }
    let first_numeric = marker_row + 1;
    let numeric_rows =
        (first_numeric..rows).take_while(|&row| (0..=coefficients).all(|col| at(row, col).is_some_and(|cell| cell.name == "real"))).count();
    if numeric_rows < 2 || at(first_numeric + numeric_rows, 0).is_some_and(|cell| cell.name == "real") {
        return None;
    }
    let height = coefficients + 3;
    let width = numeric_rows + 1;
    let mut block = vec![Node { ns: 2, name: "empty".into(), ..Node::default() }; height * width];
    block[0] = title.clone();
    block[1] = subtitle.clone();
    for col in 0..width {
        for row in 2..height {
            block[col * height + row] = at(marker_row + col, row - 2)?.clone();
        }
    }
    Some((height, width, block))
}

#[cfg(test)]
mod tests {
    use super::link_path;

    fn record(path: &str, range: &str) -> Vec<u8> {
        let mut bytes = vec![0; 66];
        bytes[..4].copy_from_slice(&[5, 0, 13, 0]);
        for field in [path, range, "", ""] {
            let units: Vec<_> = field.encode_utf16().collect();
            bytes.extend([0xff, 0xfe, 0xff, units.len() as u8]);
            bytes.extend(units.into_iter().flat_map(u16::to_le_bytes));
        }
        bytes
    }

    #[test]
    fn relocates_legacy_windows_links_but_rejects_unverifiable_selections() {
        assert_eq!(link_path(&record(r"X:\Archive\WRC Data\Table7.xls", "")).as_deref(), Some("WRC Data/Table7.xls"));
        assert_eq!(link_path(&record(r"tables\2026\Table7.xls", "")).as_deref(), Some("tables/2026/Table7.xls"));
        assert_eq!(link_path(&record(r"..\Table7.xls", "")), None);
        assert_eq!(link_path(&record(r"tables\Table7.xls", "A1:B4")), None);
        assert_eq!(link_path(&record(r"tables\Table7.xlsx", "")), None);
    }
}
