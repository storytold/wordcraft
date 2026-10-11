//! Mailings tab: mail merge (recipients from CSV/JSON, merge fields, preview, finish), envelopes
//! and labels.

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Align, CharProps};
use wordcraft_doc::section::SectionProps;
use wordcraft_doc::{Block, Document, Paragraph, Pos, StoryRef, Table, para_block};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// The mail-merge data source and preview state (lives in the session, not the document).
#[derive(Clone, Debug, Default)]
pub struct MergeState {
    pub kind: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub preview: bool,
    pub record: usize,
    pub highlight: bool,
}

fn has_recipients(s: &Session) -> Option<&'static str> {
    if s.merge.rows.is_empty() { Some("select recipients first") } else { None }
}

/// Edit Recipient List needs a list to edit (fields, even with no entries yet).
fn has_list(s: &Session) -> Option<&'static str> {
    if s.merge.headers.is_empty() && s.merge.rows.is_empty() { Some("select recipients first") } else { None }
}

/// The kinds of document Start Mail Merge makes. `normal` is an ordinary document again; only
/// `directory` changes how Finish & Merge joins the records (no page break between them).
pub const MERGE_KINDS: [&str; 6] = ["letters", "emails", "envelopes", "labels", "directory", "normal"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mailings.start", "Start Mail Merge", "Mailings › Start Mail Merge", |s, v| {
            let kind = p::str(v, "kind").unwrap_or("letters").trim().to_ascii_lowercase();
            if !MERGE_KINDS.contains(&kind.as_str()) {
                return Err(CmdError::Params(format!("`kind` must be one of {}", MERGE_KINDS.join(", "))));
            }
            s.merge.kind = kind;
            Ok(json!({"kind": s.merge.kind}))
        })
        .params(r#"{"kind": "letters|emails|envelopes|labels|directory|normal"}"#)
        .pure(),
        CommandSpec::new("mailings.recipients", "Select Recipients", "Mailings › Start Mail Merge", recipients)
            .params(r#"{"csv"?: string, "path"?: string, "rows"?: [{field: value}] | [[value]], "fields"?: [string] (column order; required with array rows)}"#)
            .pure(),
        CommandSpec::new("mailings.editRecipients", "Edit Recipient List", "Mailings › Start Mail Merge", |s, v| {
            if v.get("rows").and_then(Value::as_array).is_some() {
                return recipients(s, &json!({"rows": v.get("rows"), "fields": v.get("fields")}));
            }
            Ok(json!({"headers": s.merge.headers, "rows": s.merge.rows}))
        })
        .params(r#"{"rows"?: [{field: value}] | [[value]], "fields"?: [string]}"#)
        .when(has_list)
        .pure(),
        CommandSpec::new("mailings.insertField", "Insert Merge Field", "Mailings › Write & Insert Fields", |s, v| {
            let f = p::req_str(v, "field")?.to_string();
            insert_field(s, &format!("MERGEFIELD {}", quote(&f)), &format!("«{f}»"))
        })
        .params(r#"{"field": string}"#),
        CommandSpec::new("mailings.addressBlock", "Address Block", "Mailings › Write & Insert Fields", |s, _| {
            insert_field(s, "ADDRESSBLOCK", "«AddressBlock»")
        }),
        CommandSpec::new("mailings.greetingLine", "Greeting Line", "Mailings › Write & Insert Fields", |s, _| {
            insert_field(s, "GREETINGLINE", "«GreetingLine»")
        }),
        CommandSpec::new("mailings.rules", "Rules", "Mailings › Write & Insert Fields", |s, v| {
            let rule = p::str(v, "rule").unwrap_or("NEXT").to_ascii_uppercase();
            let instr = match rule.as_str() {
                "IF" => format!(
                    "IF {{ MERGEFIELD {} }} = \"{}\" \"{}\" \"{}\"",
                    p::str(v, "field").unwrap_or("Field"),
                    p::str(v, "value").unwrap_or(""),
                    p::str(v, "then").unwrap_or(""),
                    p::str(v, "else").unwrap_or("")
                ),
                "SKIPIF" => format!("SKIPIF {{ MERGEFIELD {} }} = \"{}\"", p::str(v, "field").unwrap_or("Field"), p::str(v, "value").unwrap_or("")),
                "MERGEREC" => "MERGEREC".into(),
                _ => "NEXT".into(),
            };
            insert_field(s, &instr, "")
        })
        .params(r#"{"rule": "IF|SKIPIF|NEXT|MERGEREC", "field"?, "value"?, "then"?, "else"?}"#),
        CommandSpec::new("mailings.matchFields", "Match Fields", "Mailings › Write & Insert Fields", |s, _| {
            Ok(json!({"fields": s.merge.headers, "address": address_map(&s.merge.headers)}))
        })
        .pure(),
        CommandSpec::new("mailings.highlightFields", "Highlight Merge Fields", "Mailings › Write & Insert Fields", |s, v| {
            s.merge.highlight = p::bool(v, "value").unwrap_or(!s.merge.highlight);
            Ok(json!({"value": s.merge.highlight}))
        })
        .pure(),
        CommandSpec::new("mailings.preview", "Preview Results", "Mailings › Preview Results", |s, v| {
            s.merge.preview = p::bool(v, "value").unwrap_or(!s.merge.preview);
            refresh(s);
            Ok(json!({"preview": s.merge.preview, "record": s.merge.record + 1}))
        })
        .when(has_recipients),
        CommandSpec::new("mailings.next", "Next Record", "Mailings › Preview Results", |s, _| step(s, 1)).when(has_recipients),
        CommandSpec::new("mailings.previous", "Previous Record", "Mailings › Preview Results", |s, _| step(s, -1)).when(has_recipients),
        CommandSpec::new("mailings.findRecipient", "Find Recipient", "Mailings › Preview Results", |s, v| {
            let q = p::req_str(v, "text")?.to_lowercase();
            let idx = s
                .merge
                .rows
                .iter()
                .position(|r| r.iter().any(|c| c.to_lowercase().contains(&q)))
                .ok_or_else(|| CmdError::Failed("no matching recipient".into()))?;
            s.merge.record = idx;
            s.merge.preview = true;
            refresh(s);
            Ok(json!({"record": idx + 1}))
        })
        .when(has_recipients),
        CommandSpec::new("mailings.checkErrors", "Check for Errors", "Mailings › Finish", |s, _| {
            let mut missing = Vec::new();
            for f in merge_fields(&s.doc) {
                if !s.merge.headers.iter().any(|h| h.eq_ignore_ascii_case(&f)) {
                    missing.push(f);
                }
            }
            Ok(json!({"ok": missing.is_empty(), "unknownFields": missing, "records": s.merge.rows.len()}))
        })
        .pure(),
        CommandSpec::new("mailings.finish", "Finish & Merge", "Mailings › Finish", finish)
            .params(r#"{"path"?: string (save the merged document), "from"?: n, "to"?: n}"#)
            .when(has_recipients),
        CommandSpec::new("mailings.envelopes", "Envelopes", "Mailings › Create", envelopes).params(
            r#"{"delivery": string, "return"?: string, "size"?: "Envelope #10|#9|Monarch|DL|C4|C5|C6|B5", "width"?: pt, "height"?: pt, "add"?: bool (add to this document instead of a new one)}"#,
        ),
        CommandSpec::new("mailings.labels", "Labels", "Mailings › Create", labels).params(
            r#"{"text"?: string, "product"?: "letter-3x10|letter-2x10|letter-2x7|letter-2x5|letter-4x20|a4-3x7|a4-2x7|a4-3x8|a4-2x4", "rows"?: n, "cols"?: n, "fromRecipients"?: bool, "single"?: {"row": n, "col": n}}"#,
        ),
    ]
}

fn quote(f: &str) -> String {
    if f.contains(' ') { format!("\"{f}\"") } else { f.to_string() }
}

fn insert_field(s: &mut Session, instr: &str, placeholder: &str) -> CmdResult {
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let end = s.doc.insert_object(&at, InlineObject::Field { instr: instr.to_string(), result: placeholder.to_string(), locked: false }, &props)?;
    s.sel = Selection::caret(end);
    refresh(s);
    sel_result(s)
}

/// Parse CSV (RFC 4180-ish: quotes, doubled quotes, commas/semicolons/tabs).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let first = text.lines().next().unwrap_or("");
    let sep = [',', ';', '\t'].into_iter().max_by_key(|c| first.matches(*c).count()).unwrap_or(',');
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
            if row.iter().any(|x| !x.is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            cell.push(c);
        }
        if rows.len() > 100_000 {
            break;
        }
    }
    row.push(cell);
    if row.iter().any(|x| !x.is_empty()) {
        rows.push(row);
    }
    rows
}

/// Most records and fields a recipient list keeps.
const MAX_RECORDS: usize = 100_000;
const MAX_FIELDS: usize = 1_000;

fn cell(x: &Value) -> String {
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

fn recipients(s: &mut Session, v: &Value) -> CmdResult {
    let (headers, rows) = if let Some(arr) = v.get("rows").and_then(Value::as_array) {
        table_from_rows(v.get("fields").and_then(Value::as_array), arr)?
    } else {
        let text = if let Some(c) = p::str(v, "csv") {
            c.to_string()
        } else if let Some(path) = p::str(v, "path") {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let b = std::fs::read(path).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
                crate::io::decode_text(&b)
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = path;
                return Err(CmdError::Failed("paths aren't available on the web".into()));
            }
        } else {
            return Err(CmdError::Params("`csv`, `path` or `rows` required".into()));
        };
        let mut all = parse_csv(&text);
        if all.is_empty() {
            return Err(CmdError::Failed("the recipient list is empty".into()));
        }
        let headers: Vec<String> = all.remove(0).into_iter().map(|h| h.trim().to_string()).collect();
        (headers, all)
    };
    s.merge.headers = headers;
    s.merge.rows = rows;
    s.merge.record = 0;
    refresh(s);
    Ok(json!({"fields": s.merge.headers, "records": s.merge.rows.len()}))
}

fn step(s: &mut Session, d: i64) -> CmdResult {
    let n = s.merge.rows.len().max(1) as i64;
    s.merge.record = (s.merge.record as i64 + d).clamp(0, n - 1) as usize;
    s.merge.preview = true;
    refresh(s);
    Ok(json!({"record": s.merge.record + 1, "of": n}))
}

/// Field names used by MERGEFIELD fields in the document.
fn merge_fields(doc: &Document) -> Vec<String> {
    let mut v = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        if let Some(p) = doc.para(StoryRef::Body, &path) {
            for o in &p.objects {
                if let InlineObject::Field { instr, .. } = o
                    && let Some(name) = mergefield_name(instr)
                    && !v.contains(&name)
                {
                    v.push(name);
                }
            }
        }
    }
    v
}

fn mergefield_name(instr: &str) -> Option<String> {
    let rest = instr.trim().strip_prefix("MERGEFIELD")?.trim();
    let name = if let Some(r) = rest.strip_prefix('"') { r.split('"').next().unwrap_or("") } else { rest.split_whitespace().next().unwrap_or("") };
    Some(name.to_string())
}

/// Which columns hold address parts.
fn address_map(headers: &[String]) -> Value {
    let find = |names: &[&str]| headers.iter().find(|h| names.iter().any(|n| h.replace([' ', '_'], "").eq_ignore_ascii_case(n))).cloned();
    json!({
        "firstName": find(&["FirstName", "First", "GivenName"]),
        "lastName": find(&["LastName", "Last", "Surname", "FamilyName"]),
        "company": find(&["Company", "Organization", "CompanyName"]),
        "address": find(&["Address", "AddressLine1", "Address1", "Street"]),
        "city": find(&["City", "Town"]),
        "state": find(&["State", "Province", "Region"]),
        "postal": find(&["PostalCode", "ZIP", "ZipCode", "Postcode"]),
        "country": find(&["Country"]),
        "title": find(&["Title", "CourtesyTitle"]),
    })
}

fn value(s: &Session, row: usize, field: &str) -> String {
    let Some(i) = s.merge.headers.iter().position(|h| h.eq_ignore_ascii_case(field)) else { return String::new() };
    s.merge.rows.get(row).and_then(|r| r.get(i)).cloned().unwrap_or_default()
}

fn address_block(s: &Session, row: usize) -> String {
    let m = address_map(&s.merge.headers);
    let g = |k: &str| m.get(k).and_then(Value::as_str).map(|f| value(s, row, f)).unwrap_or_default();
    let name = [g("title"), g("firstName"), g("lastName")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
    let city_line = [g("city"), [g("state"), g("postal")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ")]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    [name, g("company"), g("address"), city_line, g("country")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n")
}

fn greeting(s: &Session, row: usize) -> String {
    let m = address_map(&s.merge.headers);
    let g = |k: &str| m.get(k).and_then(Value::as_str).map(|f| value(s, row, f)).unwrap_or_default();
    let name = [g("firstName"), g("lastName")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
    if name.is_empty() { "Dear Sir or Madam,".into() } else { format!("Dear {name},") }
}

/// Evaluate a merge-related field for a record (None = not a merge field).
fn eval(s: &Session, instr: &str, row: usize) -> Option<String> {
    let t = instr.trim();
    if let Some(n) = mergefield_name(t) {
        return Some(value(s, row, &n));
    }
    let name = t.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    match name.as_str() {
        "ADDRESSBLOCK" => Some(address_block(s, row)),
        "GREETINGLINE" => Some(greeting(s, row)),
        "NEXT" | "MERGEREC" if name == "MERGEREC" => Some((row + 1).to_string()),
        "NEXT" | "SKIPIF" => Some(String::new()),
        "IF" => {
            // IF { MERGEFIELD X } = "v" "then" "else"
            let field =
                t.split("MERGEFIELD").nth(1).and_then(|r| r.split('}').next()).map(|x| x.trim().trim_matches('"').to_string()).unwrap_or_default();
            let quoted: Vec<&str> = t.split('"').skip(1).step_by(2).collect();
            let (cmp, then, els) =
                (quoted.first().copied().unwrap_or(""), quoted.get(1).copied().unwrap_or(""), quoted.get(2).copied().unwrap_or(""));
            Some(if value(s, row, &field) == cmp { then.to_string() } else { els.to_string() })
        }
        _ => None,
    }
}

/// Update merge field results: record values when previewing, «Field» placeholders otherwise.
pub fn refresh(s: &mut Session) {
    let row = s.merge.record;
    let preview = s.merge.preview && !s.merge.rows.is_empty();
    for path in s.doc.para_paths(StoryRef::Body) {
        let updates: Vec<(usize, String)> = match s.doc.para(StoryRef::Body, &path) {
            Some(p) => p
                .objects
                .iter()
                .enumerate()
                .filter_map(|(k, o)| match o {
                    InlineObject::Field { instr, .. } => {
                        let placeholder = match mergefield_name(instr) {
                            Some(n) => Some(format!("«{n}»")),
                            None => match instr.split_whitespace().next().map(str::to_ascii_uppercase).as_deref() {
                                Some("ADDRESSBLOCK") => Some("«AddressBlock»".into()),
                                Some("GREETINGLINE") => Some("«GreetingLine»".into()),
                                _ => None,
                            },
                        }?;
                        Some((k, if preview { eval(s, instr, row).unwrap_or(placeholder) } else { placeholder }))
                    }
                    _ => None,
                })
                .collect(),
            None => continue,
        };
        if updates.is_empty() {
            continue;
        }
        if let Ok(p) = s.doc.para_mut(StoryRef::Body, &path) {
            for (k, r) in updates {
                if let Some(InlineObject::Field { result, .. }) = p.objects.get_mut(k) {
                    *result = r;
                }
            }
            p.touch();
        }
    }
    s.touch();
}

/// Build one merged copy per record: fields become plain text; records separated by page breaks
/// (letters) or new paragraphs (directory).
fn finish(s: &mut Session, v: &Value) -> CmdResult {
    let n = s.merge.rows.len();
    let from = p::u64(v, "from").unwrap_or(1).max(1) as usize - 1;
    let to = (p::u64(v, "to").unwrap_or(n as u64) as usize).min(n);
    let directory = s.merge.kind == "directory";
    let mut out = s.doc.clone();
    out.body.clear();
    let mut record = from;
    while record < to {
        for b in &s.doc.body {
            let mut blk = (**b).clone();
            merge_block(s, &mut blk, record);
            out.body.push(std::sync::Arc::new(blk));
        }
        record += 1;
        if record < to
            && !directory
            && let Some(last) = out.body.last_mut().map(std::sync::Arc::make_mut)
            && let Block::Para(p) = last
        {
            let end = p.len();
            let props = p.props_at(end).clone();
            let _ = p.insert_text(end, "\u{000C}", &props);
        }
        if out.body.len() > 200_000 {
            break;
        }
    }
    out.ensure_nonempty();
    let count = to.saturating_sub(from);
    if let Some(path) = p::str(v, "path") {
        crate::io::save_path(std::path::Path::new(path), &out).map_err(CmdError::Failed)?;
        return Ok(json!({"records": count, "path": path}));
    }
    s.set_document(out);
    s.path = None;
    s.dirty = true;
    s.merge.preview = false;
    Ok(json!({"records": count}))
}

fn merge_block(s: &Session, b: &mut Block, row: usize) {
    match b {
        Block::Para(p) => {
            let offs = p.object_offsets();
            for off in offs.into_iter().rev() {
                let Some(InlineObject::Field { instr, .. }) = p.object_at(off).cloned() else { continue };
                if let Some(val) = eval(s, &instr, row) {
                    let props = p.props_of_char(off).clone();
                    let _ = p.delete(off, off + wordcraft_doc::para::OBJ.len_utf8());
                    let _ = p.insert_text(off, &val, &props);
                }
            }
        }
        Block::Table(t) => {
            for r in &mut t.rows {
                for c in &mut r.cells {
                    for blk in &mut c.blocks {
                        merge_block(s, std::sync::Arc::make_mut(blk), row);
                    }
                }
            }
        }
    }
}

/// Common envelope sizes: name, width and height in points (long side first).
pub const ENVELOPE_SIZES: [(&str, f32, f32); 8] = [
    ("Envelope #10", 684.0, 297.0),
    ("Envelope #9", 639.0, 279.0),
    ("Envelope Monarch", 540.0, 279.0),
    ("Envelope DL", 623.6, 311.8),
    ("Envelope C4", 918.4, 649.1),
    ("Envelope C5", 649.1, 459.2),
    ("Envelope C6", 459.2, 323.1),
    ("Envelope B5", 708.7, 498.9),
];

/// Envelope size by name (`Envelope DL`, or just `DL`, any case), or `width`/`height` in points.
fn envelope_size(v: &Value) -> Result<(f32, f32), CmdError> {
    if let (Some(w), Some(h)) = (p::f32(v, "width"), p::f32(v, "height")) {
        let (w, h) = (w.clamp(144.0, 1584.0), h.clamp(144.0, 1584.0));
        return Ok((w.max(h), w.min(h)));
    }
    let name = p::str(v, "size").unwrap_or("Envelope #10").trim();
    ENVELOPE_SIZES
        .iter()
        .find(|(n, ..)| n.eq_ignore_ascii_case(name) || n.strip_prefix("Envelope ").is_some_and(|short| short.eq_ignore_ascii_case(name)))
        .map(|(_, w, h)| (*w, *h))
        .ok_or_else(|| CmdError::Params(format!("unknown envelope size `{name}`")))
}

/// Envelopes: a landscape page with the return address at the top left and the delivery address
/// in the middle. As a new document, or (`add`) as a section of its own at the start of this one.
fn envelopes(s: &mut Session, v: &Value) -> CmdResult {
    let delivery = p::str(v, "delivery").map(str::to_string).unwrap_or_else(|| {
        if s.merge.rows.is_empty() { "Recipient Name\nStreet Address\nCity, ST 00000".into() } else { address_block(s, s.merge.record) }
    });
    let ret = p::str(v, "return").unwrap_or("").to_string();
    let (w, h) = envelope_size(v)?;
    let sect = SectionProps {
        page_w: w,
        page_h: h,
        landscape: true,
        margin_top: 22.0,
        margin_left: 22.0,
        margin_right: 22.0,
        margin_bottom: 22.0,
        ..Default::default()
    };
    // Addresses are capped: a pasted novel is not an address.
    let lines = |t: &str| t.split('\n').take(20).map(str::to_string).collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let ret_lines = if ret.is_empty() { Vec::new() } else { lines(&ret) };
    for l in &ret_lines {
        blocks.push(para_block(Paragraph::with_text(l, CharProps { size: Some(10.0), ..Default::default() }).styled("NoSpacing")));
    }
    let spacer = (6 - ret_lines.len().min(6)).max(1);
    for _ in 0..spacer + 2 {
        blocks.push(para_block(Paragraph::new().styled("NoSpacing")));
    }
    let mut last = None;
    for l in lines(&delivery) {
        let mut p = Paragraph::with_text(&l, CharProps::default()).styled("NoSpacing");
        p.props.indent_left = Some(w * 0.42);
        if let Some(prev) = last.replace(p) {
            blocks.push(para_block(prev));
        }
    }
    if p::bool(v, "add") == Some(true) {
        // The envelope's own section ends with its last address line.
        let mut end = last.unwrap_or_else(|| Paragraph::new().styled("NoSpacing"));
        end.section = Some(Box::new(sect));
        blocks.push(para_block(end));
        let n = blocks.len();
        s.doc.body.splice(0..0, blocks);
        s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
        return Ok(json!({"width": w, "height": h, "added": n}));
    }
    blocks.extend(last.map(para_block));
    let mut d = Document::new();
    d.last_section = sect;
    d.body = blocks;
    s.set_document(d);
    s.path = None;
    Ok(json!({"width": w, "height": h}))
}

/// A sheet of labels: the label size, how many across and down, and where the first one sits.
/// Lengths in points; the pitch is from one label's edge to the next one's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelProduct {
    pub id: &'static str,
    /// What the labels are for: `address`, `shipping` or `return`.
    pub kind: &'static str,
    /// Letter or A4.
    pub paper: &'static str,
    pub cols: usize,
    pub rows: usize,
    pub width: f32,
    pub height: f32,
    pub top: f32,
    pub side: f32,
    pub h_pitch: f32,
    pub v_pitch: f32,
}

const MM: f32 = 72.0 / 25.4;

/// Common label sheets (the sizes are the industry's; the names are ours).
pub const LABEL_PRODUCTS: [LabelProduct; 9] = [
    LabelProduct {
        id: "letter-3x10",
        kind: "address",
        paper: "Letter",
        cols: 3,
        rows: 10,
        width: 189.0,
        height: 72.0,
        top: 36.0,
        side: 13.5,
        h_pitch: 198.0,
        v_pitch: 72.0,
    },
    LabelProduct {
        id: "letter-2x10",
        kind: "address",
        paper: "Letter",
        cols: 2,
        rows: 10,
        width: 288.0,
        height: 72.0,
        top: 36.0,
        side: 11.25,
        h_pitch: 301.5,
        v_pitch: 72.0,
    },
    LabelProduct {
        id: "letter-2x7",
        kind: "address",
        paper: "Letter",
        cols: 2,
        rows: 7,
        width: 288.0,
        height: 96.0,
        top: 60.0,
        side: 11.25,
        h_pitch: 301.5,
        v_pitch: 96.0,
    },
    LabelProduct {
        id: "letter-2x5",
        kind: "shipping",
        paper: "Letter",
        cols: 2,
        rows: 5,
        width: 288.0,
        height: 144.0,
        top: 36.0,
        side: 11.25,
        h_pitch: 301.5,
        v_pitch: 144.0,
    },
    LabelProduct {
        id: "letter-4x20",
        kind: "return",
        paper: "Letter",
        cols: 4,
        rows: 20,
        width: 126.0,
        height: 36.0,
        top: 36.0,
        side: 21.6,
        h_pitch: 147.6,
        v_pitch: 36.0,
    },
    LabelProduct {
        id: "a4-3x7",
        kind: "address",
        paper: "A4",
        cols: 3,
        rows: 7,
        width: 63.5 * MM,
        height: 38.1 * MM,
        top: 15.15 * MM,
        side: 7.21 * MM,
        h_pitch: 66.04 * MM,
        v_pitch: 38.1 * MM,
    },
    LabelProduct {
        id: "a4-2x7",
        kind: "address",
        paper: "A4",
        cols: 2,
        rows: 7,
        width: 99.1 * MM,
        height: 38.1 * MM,
        top: 15.15 * MM,
        side: 4.65 * MM,
        h_pitch: 101.6 * MM,
        v_pitch: 38.1 * MM,
    },
    LabelProduct {
        id: "a4-3x8",
        kind: "address",
        paper: "A4",
        cols: 3,
        rows: 8,
        width: 63.5 * MM,
        height: 33.9 * MM,
        top: 12.9 * MM,
        side: 7.21 * MM,
        h_pitch: 66.04 * MM,
        v_pitch: 33.9 * MM,
    },
    LabelProduct {
        id: "a4-2x4",
        kind: "shipping",
        paper: "A4",
        cols: 2,
        rows: 4,
        width: 99.1 * MM,
        height: 67.7 * MM,
        top: 13.1 * MM,
        side: 4.65 * MM,
        h_pitch: 101.6 * MM,
        v_pitch: 67.7 * MM,
    },
];

impl LabelProduct {
    pub fn by_id(id: &str) -> Option<&'static LabelProduct> {
        LABEL_PRODUCTS.iter().find(|p| p.id == id)
    }
    /// Label size as people say it: inches on Letter, millimetres on A4.
    pub fn size_text(&self) -> String {
        if self.paper == "A4" {
            format!("{:.1} × {:.1} mm", self.width / MM, self.height / MM)
        } else {
            let i = |pt: f32| format!("{}", ((pt / 72.0) * 1000.0).round() / 1000.0);
            format!("{}\" × {}\"", i(self.width), i(self.height))
        }
    }
}

/// Labels: a new document with a table of labels laid out like the sheet (`product`; spacer
/// columns and rows stand for the gaps). A full page of the same `text` (or one recipient per
/// label), or (`single`: `{"row", "col"}`, from 1) one label at that place on the sheet.
fn labels(s: &mut Session, v: &Value) -> CmdResult {
    let prod = match p::str(v, "product") {
        Some(id) => *LabelProduct::by_id(id).ok_or_else(|| CmdError::Params(format!("unknown label product `{id}`")))?,
        None => LABEL_PRODUCTS[0],
    };
    let rows = p::u64(v, "rows").map_or(prod.rows, |n| n.clamp(1, 40) as usize);
    let cols = p::u64(v, "cols").map_or(prod.cols, |n| n.clamp(1, 10) as usize);
    let single = v.get("single").filter(|x| !x.is_null()).map(|x| {
        let at = |k: &str| x.get(k).and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
        (at("row").min(rows), at("col").min(cols))
    });
    let from_rec = single.is_none() && p::bool(v, "fromRecipients").unwrap_or(!s.merge.rows.is_empty() && p::str(v, "text").is_none());
    let text = p::str(v, "text").unwrap_or("Name\nAddress\nCity, ST 00000").to_string();
    let (page_w, page_h) = if prod.paper == "A4" { (595.3, 841.9) } else { (612.0, 792.0) };
    let h_gap = (prod.h_pitch - prod.width).max(0.0);
    let v_gap = (prod.v_pitch - prod.height).max(0.0);
    let mut d = Document::new();
    d.last_section.page_w = page_w;
    d.last_section.page_h = page_h;
    d.last_section.margin_top = prod.top;
    d.last_section.margin_bottom = 0.0;
    d.last_section.margin_left = prod.side;
    d.last_section.margin_right = (page_w - prod.side - prod.h_pitch * (cols - 1) as f32 - prod.width).max(0.0);
    d.last_section.header = 0.0;
    d.last_section.footer = 0.0;
    // Grid columns: label, gap, label, …; rows likewise.
    let grid_cols: Vec<Option<usize>> = (0..cols).flat_map(|c| [Some(c)].into_iter().chain((c + 1 < cols && h_gap > 0.5).then_some(None))).collect();
    let grid_rows: Vec<Option<usize>> = (0..rows).flat_map(|r| [Some(r)].into_iter().chain((r + 1 < rows && v_gap > 0.5).then_some(None))).collect();
    let width: f32 = grid_cols.iter().map(|c| if c.is_some() { prod.width } else { h_gap }).sum();
    let mut t = Table::new(grid_rows.len(), grid_cols.len(), width);
    t.props.style = None;
    t.props.borders = Some(wordcraft_doc::props::Borders::default());
    t.props.fixed = true;
    t.grid = grid_cols.iter().map(|c| if c.is_some() { prod.width } else { h_gap }).collect();
    let mut k = 0usize;
    let mut made = 0usize;
    for (row, r) in t.rows.iter_mut().zip(&grid_rows) {
        row.props.height = Some(if r.is_some() { prod.height } else { v_gap });
        row.props.height_rule = wordcraft_doc::props::HeightRule::Exact;
        for (cell, c) in row.cells.iter_mut().zip(&grid_cols) {
            cell.props.width = Some(if c.is_some() { prod.width } else { h_gap });
            let (Some(r), Some(c)) = (r, c) else { continue };
            let content = match single {
                Some((sr, sc)) if (sr, sc) != (r + 1, c + 1) => String::new(),
                _ if from_rec => {
                    let rec = if k < s.merge.rows.len() { address_block(s, k) } else { String::new() };
                    k += 1;
                    rec
                }
                _ => text.clone(),
            };
            if !content.is_empty() {
                made += 1;
            }
            cell.props.valign = wordcraft_doc::props::VAlign::Center;
            cell.blocks = content
                .split('\n')
                .take(20)
                .map(|l| {
                    let mut p = Paragraph::with_text(l, CharProps::default()).styled("NoSpacing");
                    p.props.align = Some(Align::Left);
                    p.props.indent_left = Some(9.0);
                    para_block(p)
                })
                .collect();
        }
    }
    d.body = vec![std::sync::Arc::new(Block::Table(t)), para_block(Paragraph::new())];
    s.set_document(d);
    s.path = None;
    let _ = Pos::body(0, 0);
    Ok(json!({"labels": if single.is_some() { 1 } else { rows * cols }, "filled": made, "product": prod.id}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parsing() {
        let r = parse_csv("Name,City\n\"Smith, Ann\",Paris\r\nBo,\"Oslo \"\"N\"\"\"\n\n");
        assert_eq!(r, vec![vec!["Name", "City"], vec!["Smith, Ann", "Paris"], vec!["Bo", "Oslo \"N\""]]);
        assert_eq!(parse_csv("a;b\n1;2")[1], vec!["1", "2"]);
    }

    #[test]
    fn merge_end_to_end() {
        let mut s = Session::new(Document::new());
        s.run("mailings.recipients", &json!({"csv": "First Name,Last Name,City\nAda,Lovelace,London\nAlan,Turing,Wilmslow"})).unwrap();
        s.run("mailings.greetingLine", &json!({})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "See you in "})).unwrap();
        s.run("mailings.insertField", &json!({"field": "City"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("«City»"));
        s.run("mailings.preview", &json!({"value": true})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("Dear Ada Lovelace,"));
        s.run("mailings.next", &json!({})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("Wilmslow"));
        let chk = s.run("mailings.checkErrors", &json!({})).unwrap();
        assert_eq!(chk["ok"], true);
        let r = s.run("mailings.finish", &json!({})).unwrap();
        assert_eq!(r["records"], 2);
        let t = s.doc.plain_text(StoryRef::Body);
        assert!(t.contains("See you in London") && t.contains("Dear Alan Turing,"), "{t}");
        assert_eq!(s.layout().pages.len(), 2);
    }

    /// A typed list (#240) keeps its column order, and rows may be arrays in that order.
    #[test]
    fn recipients_from_fields_and_array_rows() {
        let mut s = Session::new(Document::new());
        let r = s
            .run("mailings.recipients", &json!({"fields": ["Last Name", " City ", "Age"], "rows": [["Lovelace", "London", 36], ["Turing"], 7]}))
            .unwrap();
        assert_eq!(r, json!({"fields": ["Last Name", "City", "Age"], "records": 2}));
        assert_eq!(
            s.merge.rows,
            vec![vec!["Lovelace".to_string(), "London".into(), "36".into()], vec!["Turing".into(), String::new(), String::new()]]
        );
        // Objects follow `fields` too; missing and null cells are blank.
        s.run("mailings.editRecipients", &json!({"fields": ["Name", "City"], "rows": [{"City": "Paris", "Name": null}]})).unwrap();
        assert_eq!(s.merge.headers, vec!["Name", "City"]);
        assert_eq!(s.merge.rows, vec![vec![String::new(), "Paris".to_string()]]);
        // Array rows need the field names.
        let err = s.run("mailings.recipients", &json!({"rows": [["Ada"]]})).unwrap_err();
        assert!(matches!(err, CmdError::Params(_)), "{err}");
        assert!(s.ui_requests.is_empty(), "programmatic calls never open dialogs");
    }

    /// Start Mail Merge takes only the kinds it knows; Edit Recipient List needs a list.
    #[test]
    fn merge_kinds_and_edit_list_needs_recipients() {
        let mut s = Session::new(Document::new());
        for k in MERGE_KINDS {
            assert_eq!(s.run("mailings.start", &json!({"kind": k})).unwrap(), json!({"kind": k}));
        }
        assert_eq!(s.run("mailings.start", &json!({"kind": " Directory "})).unwrap(), json!({"kind": "directory"}));
        assert!(matches!(s.run("mailings.start", &json!({"kind": "fax"})), Err(CmdError::Params(_))));
        assert_eq!(s.merge.kind, "directory", "a rejected kind leaves the current one");
        assert!(matches!(s.run("mailings.editRecipients", &json!({})), Err(CmdError::Disabled(_))));
        s.run("mailings.recipients", &json!({"csv": "Name\nAda"})).unwrap();
        assert_eq!(s.run("mailings.editRecipients", &json!({})).unwrap(), json!({"headers": ["Name"], "rows": [["Ada"]]}));
    }

    #[test]
    fn envelopes_and_labels() {
        let mut s = Session::new(Document::new());
        s.run("mailings.envelopes", &json!({"delivery": "Jo Doe\n1 Main St"})).unwrap();
        assert!(s.doc.last_section.page_w > s.doc.last_section.page_h);
        s.run("mailings.labels", &json!({"text": "Hello"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).matches("Hello").count() == 30);
    }

    /// #407: Envelopes adds an envelope section of the chosen size before the letter; Labels
    /// lays out a sheet of the chosen product, or a single label at a row and column.
    #[test]
    fn envelope_sizes_add_to_document_and_label_products() {
        let mut s = Session::new(Document::from_text("Dear Jo,"));
        let r = s.run("mailings.envelopes", &json!({"delivery": "Jo Doe\n1 Main St", "return": "Me", "size": "C5", "add": true})).unwrap();
        assert_eq!((r["width"].as_f64().map(|w| w.round()), r["height"].as_f64().map(|h| h.round())), (Some(649.0), Some(459.0)));
        let secs = s.doc.sections();
        assert_eq!(secs.len(), 2, "an envelope section of its own");
        assert!(secs[0].1.landscape && (secs[0].1.page_w - 649.1).abs() < 0.1);
        let text = s.doc.plain_text(StoryRef::Body);
        assert!(text.starts_with("Me") && text.contains("1 Main St") && text.trim_end().ends_with("Dear Jo,"), "{text}");
        assert!(s.run("mailings.envelopes", &json!({"size": "Envelope X"})).is_err());

        let r = s.run("mailings.labels", &json!({"text": "Ada", "product": "a4-2x7"})).unwrap();
        assert_eq!(r["labels"], 14);
        assert!((s.doc.last_section.page_w - 595.3).abs() < 0.1, "A4 sheet");
        let Some(Block::Table(t)) = s.doc.body.first().map(|b| &**b) else { panic!("no table") };
        assert_eq!((t.rows.len(), t.grid.len()), (7, 3), "two label columns and the gap between");
        assert_eq!(s.doc.plain_text(StoryRef::Body).matches("Ada").count(), 14);
        let r = s.run("mailings.labels", &json!({"text": "Bo", "product": "letter-3x10", "single": {"row": 2, "col": 3}})).unwrap();
        assert_eq!(r["filled"], 1);
        let Some(Block::Table(t)) = s.doc.body.first().map(|b| &**b) else { panic!("no table") };
        // Row 2, column 3 (grid column 5: label, gap, label, gap, label).
        let cell = t.rows.get(1).and_then(|r| r.cells.get(4)).and_then(|c| c.blocks.first());
        assert!(matches!(cell.map(|b| &**b), Some(Block::Para(p)) if p.text == "Bo"));
        assert!(s.run("mailings.labels", &json!({"product": "nope"})).is_err());
    }
}
