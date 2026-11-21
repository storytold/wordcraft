use super::filter::refresh_shown;
use super::{CmdError, CmdResult, Session, Value, json, p};

/// The field separator: whichever of `,` `;` and tab the first non-blank record (outside quotes) uses most.
fn csv_separator(text: &str) -> char {
    let mut quoted = false;
    let mut counts = [0usize; 3];
    // A quoted header may span physical lines; infer from the first complete record.
    for c in text.split_inclusive(['\n', '\r']).skip_while(|line| line.trim().is_empty()).flat_map(str::chars) {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted {
            if c == '\n' || c == '\r' {
                break;
            }
            if let Some(i) = [',', ';', '\t'].iter().position(|candidate| *candidate == c)
                && let Some(count) = counts.get_mut(i)
            {
                *count = count.saturating_add(1);
            }
        }
    }
    [',', ';', '\t'].into_iter().zip(counts).max_by_key(|(_, count)| *count).map_or(',', |(c, _)| c)
}

/// What a CSV may hold while it is read: the parser refuses a list that is over these, so a
/// hostile file is never built and then cut.
#[derive(Clone, Copy)]
pub struct CsvLimits {
    /// Most columns in a record.
    pub fields: usize,
    /// Most cells in the list: its records times its widest record.
    pub cells: usize,
    /// Most bytes of cell text in all.
    pub text: usize,
}

/// Parse CSV (RFC 4180-ish: quotes, doubled quotes, commas/semicolons/tabs).
///
/// The limits are kept while the list is read, not after it: a record wider than
/// `limits.fields`, a list of more than `limits.cells` cells, or more than `limits.text` bytes
/// of cell text is refused, rather than built and then cut.
fn parse_csv_limited(text: &str, limits: &CsvLimits) -> Result<Vec<Vec<String>>, CmdError> {
    let sep = csv_separator(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut q = false;
    // The widest record read so far, and the bytes of cell text read so far.
    let mut widest = 0usize;
    let mut text_bytes = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    q = false;
                }
            } else {
                cell.push(c);
            }
        } else if c == '"' {
            q = true;
        } else if c == sep {
            end_cell(&mut row, &mut cell, &mut text_bytes, limits)?;
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            end_cell(&mut row, &mut cell, &mut text_bytes, limits)?;
            end_row(&mut rows, &mut row, &mut widest, limits)?;
        } else {
            cell.push(c);
        }
        // The text read so far: the finished cells and the one being read.
        if text_bytes.saturating_add(cell.len()) > limits.text {
            return Err(CmdError::Failed(format!("the CSV can't have more than {} of text", text_size(limits.text))));
        }
        if rows.len() > MAX_RECORDS + 1 {
            return Err(CmdError::Failed("the CSV has too many records".into()));
        }
    }
    // The last cell, when the file doesn't end with a line break.
    end_cell(&mut row, &mut cell, &mut text_bytes, limits)?;
    end_row(&mut rows, &mut row, &mut widest, limits)?;
    Ok(rows)
}

/// A finished cell joins the record: the record's width stays inside the limit.
fn end_cell(row: &mut Vec<String>, cell: &mut String, text_bytes: &mut usize, limits: &CsvLimits) -> Result<(), CmdError> {
    *text_bytes = text_bytes.saturating_add(cell.len());
    row.push(std::mem::take(cell));
    if row.len() > limits.fields {
        return Err(CmdError::Failed(format!("the CSV can't have more than {} columns", limits.fields)));
    }
    Ok(())
}

/// A finished record joins the list (an empty one is dropped). The list costs its records times
/// its widest record — a recipient list pads every record to the header's width — so a list
/// over the limit is refused here, while it is read.
fn end_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>, widest: &mut usize, limits: &CsvLimits) -> Result<(), CmdError> {
    if row.iter().any(|x| !x.is_empty()) {
        *widest = (*widest).max(row.len());
        rows.push(std::mem::take(row));
    } else {
        row.clear();
    }
    if rows.len().saturating_mul(*widest) > limits.cells {
        return Err(CmdError::Failed(format!("the CSV can't have more than {} cells (its records times its widest record)", limits.cells)));
    }
    Ok(())
}

/// A byte budget, said the way a reader says it.
fn text_size(bytes: usize) -> String {
    if bytes >= 1 << 20 { format!("{} MB", bytes >> 20) } else { format!("{bytes} bytes") }
}

/// Most records and fields a recipient list keeps.
const MAX_RECORDS: usize = 100_000;

const MAX_FIELDS: usize = 1_000;
const MAX_CELLS: usize = 2_000_000;
const MAX_TEXT_BYTES: usize = 64 << 20;
const RECIPIENT_LIMITS: CsvLimits = CsvLimits { fields: MAX_FIELDS, cells: MAX_CELLS, text: MAX_TEXT_BYTES };

/// Refuse excessive CSV input instead of returning a silently truncated list.
pub fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, CmdError> {
    parse_csv_limited(text, &RECIPIENT_LIMITS)
}

// A sparse list is padded and cloned: charge its normalized shape BEFORE allocating it.
fn check_shape(records: usize, fields: usize, max_cells: usize) -> Result<(), CmdError> {
    if records > MAX_RECORDS || fields > MAX_FIELDS || records.checked_mul(fields).is_none_or(|n| n > max_cells) {
        return Err(CmdError::Failed("the recipient list exceeds its record, field or aggregate cell limit".into()));
    }
    Ok(())
}

fn spend_text(left: &mut usize, n: usize) -> Result<(), CmdError> {
    *left = left.checked_sub(n).ok_or_else(|| CmdError::Failed("the recipient list exceeds its text budget".into()))?;
    Ok(())
}

pub(super) fn cell(x: &Value) -> String {
    match x {
        Value::Null => String::new(),
        Value::String(t) => t.clone(),
        other => other.to_string(),
    }
}

// Serialize non-string JSON cells through a bounded sink, not to_string() before charging.
fn budget_cell(value: &Value, left: &mut usize) -> Result<String, CmdError> {
    match value {
        Value::Null => Ok(String::new()),
        Value::String(text) => {
            spend_text(left, text.len())?;
            Ok(text.clone())
        }
        _ => {
            struct Sink {
                bytes: Vec<u8>,
                limit: usize,
            }
            impl std::io::Write for Sink {
                fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                    if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                        return Err(std::io::Error::other("recipient text budget"));
                    }
                    self.bytes.try_reserve(bytes.len()).map_err(std::io::Error::other)?;
                    self.bytes.extend_from_slice(bytes);
                    Ok(bytes.len())
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }
            let mut sink = Sink { bytes: Vec::new(), limit: *left };
            serde_json::to_writer(&mut sink, value).map_err(|_| CmdError::Failed("the recipient list exceeds its text budget".into()))?;
            spend_text(left, sink.bytes.len())?;
            String::from_utf8(sink.bytes).map_err(|_| CmdError::Failed("invalid recipient text".into()))
        }
    }
}

/// A recipient table from `rows` (objects keyed by field, or arrays in `fields` order). Without
/// `fields`, the columns are the objects' keys.
fn table_from_rows(fields: Option<&Vec<Value>>, arr: &[Value]) -> Result<(Vec<String>, Vec<Vec<String>>), CmdError> {
    let mut left = MAX_TEXT_BYTES;
    let headers: Vec<String> = match fields {
        Some(f) => {
            check_shape(0, f.len(), MAX_CELLS)?;
            let mut headers = Vec::new();
            for h in f {
                let text = budget_cell(h, &mut left)?.trim().to_string();
                headers.push(text);
            }
            headers
        }
        None => {
            if arr.iter().any(Value::is_array) {
                return Err(CmdError::Params("`fields` is required when `rows` are arrays".into()));
            }
            let mut headers: Vec<String> = Vec::new();
            for o in arr.iter().filter_map(Value::as_object) {
                for k in o.keys() {
                    if !headers.contains(k) {
                        check_shape(0, headers.len().saturating_add(1), MAX_CELLS)?;
                        spend_text(&mut left, k.len())?;
                        headers.push(k.clone());
                    }
                }
            }
            headers
        }
    };
    let count = arr.iter().filter(|v| v.is_array() || v.is_object()).count();
    check_shape(count, headers.len(), MAX_CELLS)?;
    let mut rows = Vec::new();
    for value in arr {
        let mut row = Vec::new();
        if !value.is_array() && !value.is_object() {
            continue;
        }
        for (i, h) in headers.iter().enumerate() {
            let v = match value {
                Value::Array(a) => a.get(i),
                Value::Object(o) => o.get(h),
                _ => None,
            };
            let text = match v {
                Some(v) => budget_cell(v, &mut left)?,
                None => String::new(),
            };
            row.push(text);
        }
        rows.push(row);
    }
    Ok((headers, rows))
}

/// Skip unquoted blank prefix lines only when a text recipient list has a header.
/// Headerless lists and the shared CSV reader keep whitespace records.
fn recipient_csv(text: &str, headers: bool) -> Result<Vec<Vec<String>>, CmdError> {
    let prefix = text
        .split_inclusive(['\n', '\r'])
        .take_while(|line| headers && line.trim().is_empty())
        .fold(0_usize, |bytes, line| bytes.saturating_add(line.len()));
    parse_csv(text.get(prefix..).unwrap_or_default())
}

pub(super) fn recipients(s: &mut Session, v: &Value) -> CmdResult {
    let mut sheet = Value::Null;
    let (headers, rows) = if let Some(arr) = v.get("rows").and_then(Value::as_array) {
        table_from_rows(v.get("fields").and_then(Value::as_array), arr)?
    } else {
        let all = if let Some(c) = p::str(v, "csv") {
            recipient_csv(c, p::bool(v, "headers").unwrap_or(true))?
        } else {
            let (bytes, ext) = if let Some(d) = p::str(v, "data") {
                let b = crate::cmd::insert::base64_decode(d.trim()).ok_or_else(|| CmdError::Params("`data` isn't valid base64".into()))?;
                (b, p::str(v, "path").and_then(extension))
            } else if let Some(path) = p::str(v, "path") {
                (read_file(path)?, extension(path))
            } else {
                return Err(CmdError::Params("`csv`, `path`, `data` or `rows` required".into()));
            };
            // A spreadsheet by its contents, or by its name unless it is really text (a CSV or tab-
            // separated export named .xls, as some programs write).
            let named = ext.as_deref().is_some_and(wordcraft_formats::sheet::is_extension);
            let text = wordcraft_formats::txt::is_text(&bytes);
            let spreadsheet = wordcraft_formats::sheet::detect(&bytes).is_some() || (named && !text);
            if named && !spreadsheet {
                let text = decode(&bytes, v)?;
                let text = text.trim_start().trim_start_matches('\u{feff}');
                if text.starts_with("<?xml") {
                    return Err(CmdError::Failed(
                        "this spreadsheet file is an XML spreadsheet, not a workbook; open it in a spreadsheet program, save it as .xlsx or .csv and select it again"
                            .into(),
                    ));
                }
                if text.starts_with('<') {
                    return Err(CmdError::Failed(
                        "this spreadsheet file is a web page (HTML), not a workbook; open it in a spreadsheet program, save it as .xlsx or .csv and select it again"
                            .into(),
                    ));
                }
            }
            if spreadsheet {
                use wordcraft_formats::sheet::{Pick, read_rows, sheet_names};
                let pick = match v.get("sheet") {
                    Some(Value::String(n)) => Pick::Name(n.as_str()),
                    Some(x) if x.as_u64().is_some_and(|n| n >= 1) => {
                        Pick::Index(usize::try_from(x.as_u64().unwrap_or(1).saturating_sub(1)).unwrap_or(usize::MAX))
                    }
                    Some(Value::Null) | None => {
                        let names = sheet_names(&bytes).map_err(CmdError::Failed)?;
                        if names.len() > 1 {
                            return Ok(json!({"sheets": names, "chooseSheet": true}));
                        }
                        Pick::Index(0)
                    }
                    Some(_) => return Err(CmdError::Params("`sheet` must be a sheet name or number".into())),
                };
                let (names, rows) = read_rows(&bytes, pick).map_err(CmdError::Failed)?;
                let chosen = match pick {
                    Pick::Index(i) => names.get(i).cloned(),
                    Pick::Name(n) => {
                        names.iter().find(|s| s.as_str() == n).or_else(|| names.iter().find(|s| s.eq_ignore_ascii_case(n.trim()))).cloned()
                    }
                };
                sheet = json!(chosen);
                rows
            } else {
                recipient_csv(&decode(&bytes, v)?, p::bool(v, "headers").unwrap_or(true))?
            }
        };
        header_split(all, p::bool(v, "headers").unwrap_or(true))?
    };
    s.merge.headers = headers;
    s.merge.source = rows.clone();
    s.merge.rows = rows;
    s.merge.filter.clear();
    s.merge.sort.clear();
    s.merge.record = 0;
    refresh_shown(s);
    let mut r = json!({"fields": s.merge.headers, "records": s.merge.rows.len()});
    if !sheet.is_null() {
        r["sheet"] = sheet;
    }
    Ok(r)
}

/// A text recipient list's characters: in `encoding` when given (a label such as `gbk`,
/// `big5`, `shift_jis`, `utf-16le`, `windows-1251`), else as detected (UTF-8 or UTF-16 by BOM or
/// shape, else the legacy encoding the bytes look like: GBK, Big5, Shift_JIS, Windows-1252…).
fn decode(bytes: &[u8], v: &Value) -> Result<String, CmdError> {
    match p::str(v, "encoding").map(str::trim).filter(|e| !e.is_empty()) {
        Some(label) => wordcraft_formats::txt::decode_as(bytes, label).ok_or_else(|| CmdError::Params(format!("unknown `encoding` `{label}`"))),
        None => Ok(crate::io::decode_text(bytes)),
    }
}

/// File extensions Select Recipients › Use an Existing List… opens.
pub const RECIPIENT_EXTENSIONS: &[&str] = &["csv", "tsv", "txt", "xlsx", "xlsm", "xls", "ods"];

/// Whether a file name is a spreadsheet read as a recipient list (.xlsx, .xlsm, .xls, .ods).
pub fn is_spreadsheet_name(name: &str) -> bool {
    std::path::Path::new(name).extension().is_some_and(|e| wordcraft_formats::sheet::is_extension(&e.to_string_lossy()))
}

/// A file's extension, lower case.
fn extension(path: &str) -> Option<String> {
    std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

fn read_file(path: &str) -> Result<Vec<u8>, CmdError> {
    crate::io::read_file(path).map_err(CmdError::Failed)
}

/// Field names and records from a table's rows: with `headers`, the first row names the fields
/// (blank names become `Column N`, repeats are numbered); otherwise every row is a record and
/// the fields are `Column 1`…`Column N`. Records are padded or cut to the fields, and capped.
fn header_split(mut all: Vec<Vec<String>>, headers: bool) -> Result<(Vec<String>, Vec<Vec<String>>), CmdError> {
    if all.is_empty() {
        return Err(CmdError::Failed("the recipient list is empty".into()));
    }
    let fields = if headers {
        let first = all.remove(0);
        check_shape(0, first.len(), MAX_CELLS)?;
        wordcraft_formats::sheet::field_names(&first)
    } else {
        let width = all.iter().map(Vec::len).max().unwrap_or(0);
        check_shape(0, width, MAX_CELLS)?;
        (1..=width).map(|i| format!("Column {i}")).collect()
    };
    check_shape(all.len(), fields.len(), MAX_CELLS)?;
    let mut left = MAX_TEXT_BYTES;
    for h in &fields {
        spend_text(&mut left, h.len())?;
    }
    for row in &all {
        for text in row.iter().take(fields.len()) {
            spend_text(&mut left, text.len())?;
        }
    }
    for r in &mut all {
        r.resize(fields.len(), String::new());
    }
    Ok((fields, all))
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn sparse_rows_are_charged_at_their_padded_width() {
        let limits = CsvLimits { fields: 3, cells: 8, text: 100 };
        assert!(parse_csv_limited("a,b,c\nx\nx\n", &limits).is_err());
        assert!(check_shape(3, 3, 8).is_err());
        assert!(check_shape(2, 3, 6).is_ok());
        assert!(check_shape(usize::MAX, usize::MAX, usize::MAX).is_err());
    }
    #[test]
    fn width_text_and_record_limits_refuse_instead_of_truncating() {
        let limits = CsvLimits { fields: 2, cells: 20, text: 4 };
        assert!(parse_csv_limited("a,b,c", &limits).is_err());
        assert!(parse_csv_limited("abcde", &limits).is_err());
        assert!(parse_csv_limited("a\nb\n", &limits).is_ok());
        assert!(parse_csv(&"x\n".repeat(MAX_RECORDS + 2)).is_err());
    }
    #[test]
    fn rejected_csv_and_json_keep_the_existing_list() {
        let mut session = Session::new(wordcraft_doc::Document::from_text("Hello"));
        recipients(&mut session, &json!({"csv": "Name\nKeep"})).unwrap();
        let fields: Vec<String> = (0..MAX_FIELDS).map(|i| format!("F{i}")).collect();
        let count = MAX_CELLS / MAX_FIELDS + 1;
        let csv = format!("{}\n{}", fields.join(","), "x\n".repeat(count));
        assert!(recipients(&mut session, &json!({"csv": csv})).is_err());
        let rows: Vec<Value> = (0..count).map(|_| json!({})).collect();
        assert!(recipients(&mut session, &json!({"fields": fields, "rows": rows})).is_err());
        assert_eq!(session.merge.headers, vec!["Name"]);
        assert_eq!(session.merge.rows, vec![vec!["Keep"]]);
        assert_eq!(session.merge.source, session.merge.rows);
    }
    #[test]
    fn complex_json_text_is_bounded_during_serialization() {
        let value = json!({"nested": ["abcdefgh", {"tail": true}]});
        let mut left = 5;
        assert!(budget_cell(&value, &mut left).is_err());
        let mut left = 100;
        let text = budget_cell(&value, &mut left).unwrap();
        assert_eq!(text, value.to_string());
        assert_eq!(left, 100 - text.len());
    }

    #[test]
    fn delimiter_counting_preserves_ties_and_multiline_quotes() {
        assert_eq!(csv_separator("A,B;C\n1,2;3"), ';');
        assert_eq!(csv_separator("A;B\tC\n1;2\t3"), '\t');
        assert_eq!(csv_separator("\n\"A,\nB\";C\nx;y"), ';');
        let limits = CsvLimits { fields: 2, cells: 8, text: 4 };
        assert!(parse_csv_limited(&"x".repeat(100_000), &limits).is_err());
    }
}
