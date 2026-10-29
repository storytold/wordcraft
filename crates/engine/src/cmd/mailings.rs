//! Mailings tab: mail merge (recipients from CSV/JSON or an Excel/OpenDocument spreadsheet,
//! filtered and sorted; merge fields, preview, finish), envelopes and labels.

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Align, CharProps};
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{Block, Blocks, Document, Paragraph, Part, PartKind, Pos, StoryRef, Table, para_block};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

mod filter;
mod labels;
mod merge;
mod recipients;

#[cfg(test)]
use filter::compare_cells;
pub use filter::{FilterOp, FilterRule, SortKey, select_records};
use filter::{apply_filter, filter_result, parse_rules, parse_sort, rules_json, sort_json};
pub use labels::{ENVELOPE_SIZES, LABEL_PRODUCTS, LabelProduct};
use labels::{envelopes, labels};
use merge::{address_map, finish, go_to, merge_fields, step};
pub use merge::{merge_range, refresh};
#[cfg(test)]
use merge::{mergefield_name, placeholder};
use recipients::recipients;
pub use recipients::{RECIPIENT_EXTENSIONS, is_spreadsheet_name, parse_csv};

/// The mail-merge data source and preview state (lives in the session, not the document).
#[derive(Clone, Debug, Default)]
pub struct MergeState {
    pub kind: String,
    pub headers: Vec<String>,
    /// The records the merge uses: [`MergeState::source`] filtered and sorted.
    pub rows: Vec<Vec<String>>,
    /// Every record of the list, in file order (Edit Recipient List edits these).
    pub source: Vec<Vec<String>>,
    /// Filter Recipients: which records merge (none: all).
    pub filter: Vec<FilterRule>,
    /// Sort Recipients: the order they merge in (none: the list's order).
    pub sort: Vec<SortKey>,
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
            .params(r#"{"csv"?: string, "path"?: string (.csv/.tsv/.txt, or a .xlsx/.xlsm/.xls/.ods workbook), "data"?: base64 (a file's bytes, as for path), "encoding"?: string (a text list's encoding, e.g. "gbk", "big5", "shift_jis", "utf-16le"; default: detected), "sheet"?: name | number (1 = first; a workbook with several sheets and no sheet answers {"sheets": [...], "chooseSheet": true} and loads nothing), "headers"?: bool (default true: the first non-blank row names the fields; false: fields are Column 1…N), "rows"?: [{field: value}] | [[value]], "fields"?: [string] (column order; required with array rows)}"#)
            .pure(),
        CommandSpec::new("mailings.editRecipients", "Edit Recipient List", "Mailings › Start Mail Merge", |s, v| {
            if v.get("rows").and_then(Value::as_array).is_some() {
                // Edited records keep the list's filter and sort (rules on removed fields drop).
                // Copies, so a list that fails to load leaves the filter and sort as they were.
                let (filter, sort) = (s.merge.filter.clone(), s.merge.sort.clone());
                let r = recipients(s, &json!({"rows": v.get("rows"), "fields": v.get("fields")}))?;
                s.merge.filter = filter;
                s.merge.sort = sort;
                apply_filter(s);
                return Ok(json!({"fields": r.get("fields"), "records": s.merge.rows.len(), "of": s.merge.source.len()}));
            }
            Ok(json!({
                "headers": s.merge.headers,
                "rows": s.merge.source,
                "records": s.merge.rows.len(),
                "rules": rules_json(&s.merge),
                "sort": sort_json(&s.merge),
            }))
        })
        .params(r#"{"rows"?: [{field: value}] | [[value]], "fields"?: [string]} (no params: the list as {headers, rows (every record), records (how many merge), rules, sort})"#)
        .key(super::non_mac("Alt+Shift+E"))
        .when(has_list)
        .pure(),
        CommandSpec::new("mailings.filterRecipients", "Filter Recipients", "Mailings › Start Mail Merge › Edit Recipient List", |s, v| {
            let filter = parse_rules(&s.merge.headers, v.get("rules"))?;
            let sort = match v.get("sort") {
                Some(x) => parse_sort(&s.merge.headers, Some(x))?,
                None if v.get("rules").is_none() => Vec::new(),
                None => s.merge.sort.clone(),
            };
            s.merge.filter = filter;
            s.merge.sort = sort;
            apply_filter(s);
            filter_result(s)
        })
        .params(r#"{"rules"?: [{"field": string, "op": "equal|notEqual|less|greater|lessOrEqual|greaterOrEqual|blank|notBlank|contains|notContains", "value"?: string, "join"?: "and|or" (with the rule before; And binds tighter)}], "sort"?: [{"field": string, "descending"?: bool}]} (replaces the filter; `sort` too when given; {} clears both). Preview, Finish & Merge and labels use only the matching records, in sort order"#)
        .when(has_list)
        .pure(),
        CommandSpec::new("mailings.sortRecipients", "Sort Recipients", "Mailings › Start Mail Merge › Edit Recipient List", |s, v| {
            s.merge.sort = parse_sort(&s.merge.headers, v.get("sort").or_else(|| v.get("field").map(|_| v)))?;
            apply_filter(s);
            filter_result(s)
        })
        .params(r#"{"sort"?: [{"field": string, "descending"?: bool}]} | {"field": string, "descending"?: bool} (no params: the list's own order)"#)
        .when(has_list)
        .pure(),
        CommandSpec::new("mailings.insertField", "Insert Merge Field", "Mailings › Write & Insert Fields", |s, v| {
            let f = p::req_str(v, "field")?.to_string();
            insert_field(s, &format!("MERGEFIELD {}", quote(&f)), &format!("«{f}»"))
        })
        .params(r#"{"field": string}"#)
        .key(super::non_mac("Alt+Shift+F")),
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
                    quote(p::str(v, "field").unwrap_or("Field")),
                    p::str(v, "value").unwrap_or(""),
                    p::str(v, "then").unwrap_or(""),
                    p::str(v, "else").unwrap_or("")
                ),
                "SKIPIF" | "NEXTIF" => {
                    format!("{rule} {{ MERGEFIELD {} }} = \"{}\"", quote(p::str(v, "field").unwrap_or("Field")), p::str(v, "value").unwrap_or(""))
                }
                "MERGEREC" => "MERGEREC".into(),
                _ => "NEXT".into(),
            };
            insert_field(s, &instr, "")
        })
        .params(r#"{"rule": "IF|SKIPIF|NEXTIF|NEXT|MERGEREC", "field"?, "value"?, "then"?, "else"?}"#),
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
        .key(super::non_mac("Alt+Shift+K"))
        .when(has_recipients),
        CommandSpec::new("mailings.next", "Next Record", "Mailings › Preview Results", |s, _| step(s, 1)).when(has_recipients),
        CommandSpec::new("mailings.previous", "Previous Record", "Mailings › Preview Results", |s, _| step(s, -1)).when(has_recipients),
        CommandSpec::new("mailings.first", "First Record", "Mailings › Preview Results", |s, _| go_to(s, 0)).when(has_recipients),
        CommandSpec::new("mailings.last", "Last Record", "Mailings › Preview Results", |s, _| go_to(s, i64::MAX)).when(has_recipients),
        CommandSpec::new("mailings.goToRecord", "Go to Record", "Mailings › Preview Results", |s, v| {
            let n = v.get("record").and_then(|r| r.as_i64().or_else(|| r.as_str().and_then(|t| t.trim().parse().ok())));
            let n = n.ok_or_else(|| CmdError::Params("`record` must be a record number (1 = the first)".into()))?;
            go_to(s, n.saturating_sub(1))
        })
        .params(r#"{"record": n (1 = the first; past either end goes to the first or last record)}"#)
        .when(has_recipients),
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
            .params(r#"{"path"?: string (save the merged document), "records"?: "all|current" (current: the previewed record; default all, or from/to), "from"?: n, "to"?: n (record numbers, 1 = the first)} → {records: merged copies made; requested: records in the from–to range; skipped: copies SKIPIF dropped; omitted: records not reached because the merged document hit its size limit (then also limit, in paragraphs and tables, and warning)}. Copies and records differ (NEXT puts several records in one copy, and omitted records were never tested against SKIPIF), so the numbers need not add up to requested"#)
            .key(super::non_mac("Alt+Shift+N"))
            .when(has_recipients),
        CommandSpec::new("mailings.printDocuments", "Print Documents", "Mailings › Finish › Finish & Merge", |s, v| {
            let mut v = v.clone();
            if let Some(o) = v.as_object_mut() {
                o.remove("path");
            }
            let r = finish(s, &v)?;
            s.push_ui_request(json!({"open": "print"}));
            Ok(r)
        })
        .params(r#"{"records"?: "all|current", "from"?: n, "to"?: n} (merges into a new document, as Finish & Merge does, and opens Print for it)"#)
        .key(super::non_mac("Alt+Shift+M"))
        .when(has_recipients),
        CommandSpec::new("mailings.envelopes", "Envelopes", "Mailings › Create", envelopes).params(
            r#"{"delivery": string, "return"?: string, "size"?: "Envelope #10|#9|Monarch|DL|C4|C5|C6|B5", "width"?: pt, "height"?: pt, "add"?: bool (add to this document instead of a new one)}"#,
        ),
        CommandSpec::new("mailings.labels", "Labels", "Mailings › Create", labels).params(
            r#"{"text"?: string, "product"?: "letter-3x10|letter-2x10|letter-2x7|letter-2x5|letter-4x20|a4-3x7|a4-2x7|a4-3x8|a4-2x4", "rows"?: n, "cols"?: n, "fromRecipients"?: bool (one label per selected recipient, continuing across sheets), "single"?: {"row": n, "col": n}}"#,
        ),
    ]
}

fn quote(f: &str) -> String {
    if f.contains(' ') { format!("\"{f}\"") } else { f.to_string() }
}

fn insert_field(s: &mut Session, instr: &str, placeholder: &str) -> CmdResult {
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let end = s.doc.insert_object(
        &at,
        InlineObject::Field {
            instr: instr.to_string(),
            result: placeholder.to_string(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        },
        &props,
    )?;
    s.sel = Selection::caret(end);
    refresh(s);
    sel_result(s)
}

#[cfg(test)]
mod tests;
