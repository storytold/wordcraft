//! Table Design › Table Styles: create and modify table styles (whole table, header row and
//! banded rows), stored as table styles in the document's style sheet (ECMA-376 §17.7.6).

use serde_json::{Value, json};
use wordcraft_doc::props::{Border, BorderStyle, Borders, CharProps, ParaProps, Rgb, TextColor};
use wordcraft_doc::styles::{Style, StyleKind, TableStyleParts};

use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The formatting params both commands share; each region (`wholeTable`, `headerRow`,
/// `bandedRows`) takes the same shortcuts.
macro_rules! style_params {
    ($head:literal) => {
        concat!(
            "{",
            $head,
            r#", "basedOn"?: string, "wholeTable"?: Region, "headerRow"?: Region, "bandedRows"?: Region, "bandSize"?: n, "para"?: ParaProps, "apply"?: bool}"#,
            r#" where Region = {"borders"?: bool | null (true: single lines, false: none, null: inherit), "borderColor"?: "RRGGBB", "borderWidth"?: pt, "fill"?: "RRGGBB" | null, "bold"?: bool, "italic"?: bool, "color"?: "RRGGBB" | null, "size"?: pt, "font"?: string}"#
        )
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("table.newStyle", "New Table Style", "Table Design › Table Styles", new_style).params(style_params!(r#""name": string"#)),
        CommandSpec::new("table.modifyStyle", "Modify Table Style", "Table Design › Table Styles", modify_style)
            .params(style_params!(r#""style"?: string (default: the current table's), "name"?: string"#)),
    ]
}

/// A table style's id by id or display name (case-insensitive); styles of other kinds don't count.
pub fn table_style_id(s: &Session, name: &str) -> Option<String> {
    let st = &s.doc.styles.styles;
    st.iter()
        .find(|x| x.kind == StyleKind::Table && x.id == name)
        .or_else(|| st.iter().find(|x| x.kind == StyleKind::Table && x.name.eq_ignore_ascii_case(name)))
        .map(|x| x.id.clone())
}

/// The style of the table at the caret.
fn current_table_style(s: &Session) -> Option<String> {
    let (tp, _, _) = s.sel.focus.path.cell()?;
    s.doc.table(s.sel.focus.story, &tp)?.props.style.clone()
}

fn new_style(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    if name.is_empty() {
        return Err(CmdError::Params("name is empty".into()));
    }
    if s.doc.styles.find(&name).is_some() {
        return Err(CmdError::Failed(format!("a style named `{name}` already exists")));
    }
    let based_on = match p::str(v, "basedOn") {
        Some(b) => Some(table_style_id(s, b).ok_or_else(|| CmdError::Params(format!("no table style `{b}`")))?),
        None => table_style_id(s, "TableGrid").or_else(|| table_style_id(s, "TableNormal")),
    };
    let id = s.doc.styles.new_id(&name);
    let mut st = Style { id: id.clone(), name, kind: StyleKind::Table, based_on, priority: Some(99), ..Default::default() };
    edit(&mut st, v)?;
    s.doc.styles.upsert(st);
    s.dirty = true;
    if p::bool(v, "apply").unwrap_or(true) {
        apply(s, &id)?;
    }
    Ok(json!({"id": id}))
}

fn modify_style(s: &mut Session, v: &Value) -> CmdResult {
    let id = match p::str(v, "style") {
        Some(n) => table_style_id(s, n).ok_or_else(|| CmdError::Params(format!("no table style `{n}`")))?,
        None => current_table_style(s).ok_or_else(|| CmdError::Disabled("the cursor isn't in a table with a style".into()))?,
    };
    let rename = p::str(v, "name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    if let Some(n) = &rename
        && s.doc.styles.find(n).is_some_and(|x| x.id != id)
    {
        return Err(CmdError::Failed(format!("a style named `{n}` already exists")));
    }
    let based_on = match p::str(v, "basedOn") {
        Some(b) => {
            let b = table_style_id(s, b).ok_or_else(|| CmdError::Params(format!("no table style `{b}`")))?;
            // A style can't be based on itself or on a style based on it.
            if s.doc.styles.chain(&b).iter().any(|x| x.id == id) {
                return Err(CmdError::Params("a style can't be based on itself".into()));
            }
            Some(b)
        }
        None => None,
    };
    let mut st = s.doc.styles.get(&id).cloned().ok_or_else(|| CmdError::Params("no such style".into()))?;
    if let Some(n) = rename {
        st.name = n;
    }
    if let Some(b) = based_on {
        st.based_on = Some(b);
    }
    edit(&mut st, v)?;
    s.doc.styles.upsert(st);
    s.dirty = true;
    if p::bool(v, "apply").unwrap_or(false) {
        apply(s, &id)?;
    }
    Ok(json!({"id": id}))
}

/// Apply style `id` to the table at the caret, if any.
fn apply(s: &mut Session, id: &str) -> Result<(), CmdError> {
    let Some((tp, _, _)) = s.sel.focus.path.cell() else { return Ok(()) };
    s.doc.table_mut(s.sel.focus.story, &tp)?.props.style = Some(id.to_string());
    super::table::touch_cells(s, &tp)
}

/// Apply the formatting params to `st`.
fn edit(st: &mut Style, v: &Value) -> Result<(), CmdError> {
    let mut parts = st.table.take().unwrap_or_default();
    if let Some(r) = v.get("wholeTable") {
        region(r, &mut parts.fill, &mut st.chr, &mut parts.borders)?;
    }
    if let Some(r) = v.get("headerRow") {
        region(r, &mut parts.header_fill, &mut parts.header_chr, &mut parts.header_borders)?;
    }
    if let Some(r) = v.get("bandedRows") {
        region(r, &mut parts.band_fill, &mut parts.band_chr, &mut parts.band_borders)?;
    }
    if let Some(n) = v.get("bandSize") {
        parts.band_size = n.as_u64().map(|n| n.clamp(1, 1000) as u32);
    }
    if let Some(pp) = v.get("para") {
        let pp: ParaProps = serde_json::from_value(pp.clone()).map_err(|e| CmdError::Params(format!("para: {e}")))?;
        st.para.overlay(&pp);
        st.para.style = None;
        st.para.numbering = None;
    }
    st.table = (parts != TableStyleParts::default()).then_some(parts);
    Ok(())
}

/// One region's shortcuts: borders, fill and character formatting.
fn region(r: &Value, fill: &mut Option<Rgb>, chr: &mut CharProps, borders: &mut Option<Borders>) -> Result<(), CmdError> {
    if !r.is_object() {
        return Err(CmdError::Params("a region is an object".into()));
    }
    let color = |k: &str| -> Result<Option<Rgb>, CmdError> {
        match r.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(h)) => Rgb::parse(h).map(Some).ok_or_else(|| CmdError::Params(format!("`{k}`: bad colour `{h}`"))),
            Some(_) => Err(CmdError::Params(format!("`{k}` is \"RRGGBB\" or null"))),
        }
    };
    if r.get("fill").is_some() {
        *fill = color("fill")?;
    }
    if r.get("color").is_some() {
        chr.color = color("color")?.map(TextColor::Rgb);
    }
    if let Some(b) = p::bool(r, "bold") {
        chr.bold = Some(b);
        chr.bold_cs = Some(b);
    }
    if let Some(b) = p::bool(r, "italic") {
        chr.italic = Some(b);
        chr.italic_cs = Some(b);
    }
    if let Some(sz) = p::f32(r, "size") {
        chr.size = Some(sz.clamp(1.0, 1638.0));
        chr.size_cs = chr.size;
    }
    if let Some(f) = p::str(r, "font").map(str::trim).filter(|f| !f.is_empty()) {
        chr.font = Some(f.to_string());
    }
    match r.get("borders") {
        Some(Value::Bool(true)) => {
            let width = p::f32(r, "borderWidth").unwrap_or(0.5).clamp(0.25, 6.0);
            *borders = Some(Borders::all(Border { style: BorderStyle::Single, width, color: color("borderColor")?, space: 0.0 }));
        }
        Some(Value::Bool(false)) => *borders = Some(Borders::all(Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 })),
        Some(Value::Null) => *borders = None,
        Some(_) => return Err(CmdError::Params("`borders` is true, false or null".into())),
        None => {}
    }
    Ok(())
}
