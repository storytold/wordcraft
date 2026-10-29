use super::filter::refresh_shown;
use super::{CmdError, CmdResult, Session, Value, json, p};

/// Parse CSV (RFC 4180-ish: quotes, doubled quotes, commas/semicolons/tabs).
/// The field separator: whichever of `,` `;` and tab the first line (outside quotes) uses most.
fn csv_separator(text: &str) -> char {
    let first = text.lines().next().unwrap_or("");
    let mut quoted = false;
    let mut unquoted = String::new();
    for c in first.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted {
            unquoted.push(c);
        }
    }
    [',', ';', '\t'].into_iter().max_by_key(|c| unquoted.matches(*c).count()).unwrap_or(',')
}

/// Ends the current row: kept when it has any non-empty field, otherwise dropped.
fn push_csv_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>) {
    if row.iter().any(|x| !x.is_empty()) {
        rows.push(std::mem::take(row));
    } else {
        row.clear();
    }
}

pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let sep = csv_separator(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut q = false;
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
            row.push(std::mem::take(&mut cell));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut cell));
            push_csv_row(&mut rows, &mut row);
        } else {
            cell.push(c);
        }
        if rows.len() > 100_000 {
            break;
        }
    }
    row.push(cell);
    push_csv_row(&mut rows, &mut row);
    rows
}

/// Most records and fields a recipient list keeps.
const MAX_RECORDS: usize = 100_000;

const MAX_FIELDS: usize = 1_000;

pub(super) fn cell(x: &Value) -> String {
    match x {
        Value::Null => String::new(),
        Value::String(t) => t.clone(),
        other => other.to_string(),
    }
}

/// A recipient table from `rows` (objects keyed by field, or arrays in `fields` order). Without
/// `fields`, the columns are the objects' keys.
fn table_from_rows(fields: Option<&Vec<Value>>, arr: &[Value]) -> Result<(Vec<String>, Vec<Vec<String>>), CmdError> {
    let headers: Vec<String> = match fields {
        Some(f) => f.iter().take(MAX_FIELDS).map(|h| cell(h).trim().to_string()).collect(),
        None => {
            if arr.iter().any(Value::is_array) {
                return Err(CmdError::Params("`fields` is required when `rows` are arrays".into()));
            }
            let mut headers: Vec<String> = Vec::new();
            for o in arr.iter().filter_map(Value::as_object) {
                for k in o.keys() {
                    if !headers.contains(k) && headers.len() < MAX_FIELDS {
                        headers.push(k.clone());
                    }
                }
            }
            headers
        }
    };
    let rows = arr
        .iter()
        .take(MAX_RECORDS)
        .filter_map(|r| match r {
            Value::Array(cells) => Some((0..headers.len()).map(|i| cells.get(i).map(cell).unwrap_or_default()).collect()),
            Value::Object(o) => Some(headers.iter().map(|h| o.get(h).map(cell).unwrap_or_default()).collect()),
            _ => None,
        })
        .collect();
    Ok((headers, rows))
}

pub(super) fn recipients(s: &mut Session, v: &Value) -> CmdResult {
    let mut sheet = Value::Null;
    let (headers, rows) = if let Some(arr) = v.get("rows").and_then(Value::as_array) {
        table_from_rows(v.get("fields").and_then(Value::as_array), arr)?
    } else {
        let all = if let Some(c) = p::str(v, "csv") {
            parse_csv(c)
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
                parse_csv(&decode(&bytes, v)?)
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
        wordcraft_formats::sheet::field_names(&first[..first.len().min(MAX_FIELDS)])
    } else {
        let width = all.iter().map(Vec::len).max().unwrap_or(0).min(MAX_FIELDS);
        (1..=width).map(|i| format!("Column {i}")).collect()
    };
    all.truncate(MAX_RECORDS);
    for r in &mut all {
        r.resize(fields.len(), String::new());
    }
    Ok((fields, all))
}
