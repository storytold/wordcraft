//! Table Design › Table Styles: create, modify and delete table styles (whole table, header and
//! total rows, first and last columns, banded rows and columns), stored as table styles in the
//! document's style sheet (ECMA-376 §17.7.6).

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, ParaProps, Rgb, TextColor, Underline, VAlign};
use wordcraft_doc::styles::{PartFormat, Style, StyleKind, TablePart, TableStyleParts, TableStyleProps};
use wordcraft_doc::{Block, Table};

use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The parts a table style formats, by param name: the names the table style options follow
/// (`headerRow`, `bandedRows`…) and the ECMA-376 §17.7.6 part names (`firstRow`, `band2Horz`…).
pub const PARTS: [(&str, TablePart); 23] = [
    ("wholeTable", TablePart::WholeTable),
    ("headerRow", TablePart::FirstRow),
    ("firstRow", TablePart::FirstRow),
    ("lastRow", TablePart::LastRow),
    ("totalRow", TablePart::LastRow),
    ("firstColumn", TablePart::FirstCol),
    ("firstCol", TablePart::FirstCol),
    ("lastColumn", TablePart::LastCol),
    ("lastCol", TablePart::LastCol),
    ("bandedRows", TablePart::Band1Horz),
    ("band1Horz", TablePart::Band1Horz),
    ("band2Horz", TablePart::Band2Horz),
    ("bandedColumns", TablePart::Band1Vert),
    ("band1Vert", TablePart::Band1Vert),
    ("band2Vert", TablePart::Band2Vert),
    ("neCell", TablePart::NeCell),
    ("nwCell", TablePart::NwCell),
    ("seCell", TablePart::SeCell),
    ("swCell", TablePart::SwCell),
    ("topRightCell", TablePart::NeCell),
    ("topLeftCell", TablePart::NwCell),
    ("bottomRightCell", TablePart::SeCell),
    ("bottomLeftCell", TablePart::SwCell),
];

/// The part a param name means.
pub fn part_named(name: &str) -> Option<TablePart> {
    PARTS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, p)| *p)
}

/// The formatting params both commands share; each part takes the same Part object.
macro_rules! style_params {
    ($head:literal) => {
        concat!(
            "{",
            $head,
            r#", "basedOn"?: string, "<part>"?: Part for each part name (wholeTable, headerRow|firstRow, lastRow|totalRow, firstColumn|firstCol, lastColumn|lastCol, bandedRows|band1Horz, band2Horz, bandedColumns|band1Vert, band2Vert, nwCell, neCell, swCell, seCell), "part"?: part name (the Part fields then sit at the top level), "parts"?: [Part & {"part": name}], "bandSize"?: n (rows per band), "para"?: ParaProps, "apply"?: bool}"#,
            r#" where Part = {"fill"?: "RRGGBB" | null, "font"?: string | null (null or "": inherited), "size"?: pt, "bold"?: bool, "italic"?: bool, "underline"?: bool | "single" | "double" | "thick" | "words" | "dotted" | "dash" | "wave" | "none", "color"?: "RRGGBB" | null, "align"?: "left" | "center" | "right" | "justify" | null, "vAlign"?: "top" | "center" | "bottom" | null, "cellAlign"?: "topLeft" | "topCenter" | "topRight" | "centerLeft" | "center" | "centerRight" | "bottomLeft" | "bottomCenter" | "bottomRight" | null (both alignments), "borders"?: bool | null (true: single lines on every edge, false: none, null: inherit) | Edges | [Edges], "borderColor"?: "RRGGBB", "borderWidth"?: pt}"#,
            r#" and Edges = {"edges"?: "all" | "none" | "outside" | "inside" | "top" | "bottom" | "left" | "right" | "insideH" | "insideV" (default all), "style"?: "single" | "double" | "dotted" | "dashed" | "dotDash" | "thick" | "triple" | "wave" | "none", "width"?: pt, "color"?: "RRGGBB" | null}"#
        )
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("table.newStyle", "New Table Style", "Table Design › Table Styles", new_style).params(style_params!(r#""name": string"#)),
        CommandSpec::new("table.modifyStyle", "Modify Table Style", "Table Design › Table Styles", modify_style)
            .params(style_params!(r#""style"?: string (default: the current table's), "name"?: string"#)),
        CommandSpec::new("table.deleteStyle", "Delete Table Style", "Table Design › Table Styles", delete_style)
            .params(r#"{"style"?: string (default: the current table's)}"#),
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

/// The style `table.newStyle` would add for `v`, validated.
fn build_new(s: &Session, v: &Value) -> Result<Style, CmdError> {
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
    let mut st = Style { id, name, kind: StyleKind::Table, based_on, priority: Some(99), ..Default::default() };
    // Bands of one row and one column unless the style it's based on sets them: Word, like us,
    // reads a style with no band size as having no bands (#3415).
    let base = st.based_on.as_deref().and_then(|b| s.doc.styles.table_style(b)).map(|t| t.parts).unwrap_or_default();
    if base.band_size.is_none() || base.col_band_size.is_none() {
        let parts = st.table.get_or_insert_with(TableStyleParts::default);
        parts.band_size = base.band_size.is_none().then_some(1);
        parts.col_band_size = base.col_band_size.is_none().then_some(1);
    }
    edit(&mut st, v)?;
    Ok(st)
}

fn new_style(s: &mut Session, v: &Value) -> CmdResult {
    let st = build_new(s, v)?;
    let id = st.id.clone();
    s.doc.styles.upsert(st);
    s.dirty = true;
    if p::bool(v, "apply").unwrap_or(true) {
        apply(s, &id)?;
    }
    Ok(json!({"id": id}))
}

/// The table style `v` names (`style`), else the current table's.
fn target(s: &Session, v: &Value) -> Result<String, CmdError> {
    match p::str(v, "style") {
        Some(n) => table_style_id(s, n).ok_or_else(|| CmdError::Params(format!("no table style `{n}`"))),
        None => current_table_style(s).ok_or_else(|| CmdError::Disabled("the cursor isn't in a table with a style".into())),
    }
}

fn modify_style(s: &mut Session, v: &Value) -> CmdResult {
    let st = build_modified(s, v)?;
    let id = st.id.clone();
    s.doc.styles.upsert(st);
    s.dirty = true;
    if p::bool(v, "apply").unwrap_or(false) {
        apply(s, &id)?;
    }
    Ok(json!({"id": id}))
}

/// The style `table.modifyStyle` would store for `v`, validated.
fn build_modified(s: &Session, v: &Value) -> Result<Style, CmdError> {
    let id = target(s, v)?;
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
    Ok(st)
}

/// How table style `v["style"]` (or, without it, a new style) would look with the formatting
/// params of `table.modifyStyle` / `table.newStyle` applied, for a dialog's live preview. Names
/// aren't checked; `None` when the params don't validate.
pub fn preview(s: &Session, v: &Value) -> Option<TableStyleProps> {
    if !v.is_object() {
        return None;
    }
    let mut v = v.clone();
    let st = if p::str(&v, "style").is_some() {
        if let Some(o) = v.as_object_mut() {
            o.remove("name");
        }
        build_modified(s, &v).ok()?
    } else {
        // A name no style has: the look doesn't depend on it.
        v["name"] = json!("\u{1}preview");
        build_new(s, &v).ok()?
    };
    let id = st.id.clone();
    let mut sheet = s.doc.styles.clone();
    sheet.upsert(st);
    sheet.table_style(&id)
}

/// Delete a custom table style. Styles based on it are re-based on its own base with its
/// formatting folded in, so they look the same; tables using it take Table Grid (the style new
/// tables get), or no style when the document has no Table Grid.
fn delete_style(s: &mut Session, v: &Value) -> CmdResult {
    let id = target(s, v)?;
    delete(s, &id)
}

/// Delete table style `id` (also what `styles.delete` runs for a table style): see
/// [`delete_style`].
pub fn delete(s: &mut Session, id: &str) -> CmdResult {
    let id = id.to_string();
    let st = s.doc.styles.get(&id).cloned().ok_or_else(|| CmdError::Params("no such style".into()))?;
    if st.kind != StyleKind::Table {
        return Err(CmdError::Params(format!("`{}` isn't a table style", st.name)));
    }
    if is_builtin_table_style(&st) {
        return Err(CmdError::Failed(format!("`{}` is a built-in table style and can't be deleted", st.name)));
    }
    let mut rebased = Vec::new();
    for child in s.doc.styles.styles.iter_mut().filter(|c| c.based_on.as_deref() == Some(id.as_str())) {
        let mut para = st.para.clone();
        para.overlay(&child.para);
        child.para = para;
        let mut chr = st.chr.clone();
        chr.overlay(&child.chr);
        child.chr = chr;
        if child.kind == StyleKind::Table {
            let mut parts = st.table.clone().unwrap_or_default();
            if let Some(t) = &child.table {
                parts.overlay(t);
            }
            child.table = (parts != TableStyleParts::default()).then_some(parts);
        }
        child.based_on = st.based_on.clone();
        rebased.push(child.id.clone());
    }
    s.doc.styles.styles.retain(|x| x.id != id);
    let fallback = table_style_id(s, "TableGrid");
    let mut tables = restyle(&mut s.doc.body, &id, fallback.as_deref(), 0);
    for part in s.doc.parts.values_mut() {
        tables += restyle(&mut part.blocks, &id, fallback.as_deref(), 0);
    }
    s.dirty = true;
    Ok(json!({"deleted": id, "tables": tables, "rebased": rebased}))
}

/// Whether `st` is one of Word's built-in table styles. A .docx can mark any style custom
/// (`w:customStyle`), so Table Grid and Normal Table are recognised by id and name too: deleting
/// them would leave new tables (and the fallback for deleted styles) without a style to take.
pub fn is_builtin_table_style(st: &Style) -> bool {
    const IDS: [&str; 2] = ["TableGrid", "TableNormal"];
    const NAMES: [&str; 3] = ["Table Grid", "Normal Table", "Table Normal"];
    st.builtin || IDS.iter().any(|x| st.id.eq_ignore_ascii_case(x)) || NAMES.iter().any(|x| st.name.eq_ignore_ascii_case(x))
}

/// Tables nest at most this deep for restyling.
const MAX_DEPTH: usize = 32;

/// Whether table `t`, or one nested in it, uses style `id`.
fn uses_style(t: &Table, id: &str, depth: usize) -> bool {
    t.props.style.as_deref() == Some(id)
        || (depth < MAX_DEPTH
            && t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks).any(|b| b.as_table().is_some_and(|t| uses_style(t, id, depth + 1))))
}

/// Give every table in `blocks` that uses style `from` style `to`; the number restyled.
fn restyle(blocks: &mut [Arc<Block>], from: &str, to: Option<&str>, depth: usize) -> usize {
    let mut n = 0;
    for b in blocks {
        if !b.as_table().is_some_and(|t| uses_style(t, from, depth)) {
            continue;
        }
        if let Block::Table(t) = Arc::make_mut(b) {
            if t.props.style.as_deref() == Some(from) {
                t.props.style = to.map(str::to_string);
                n += 1;
            }
            if depth < MAX_DEPTH {
                for c in t.rows.iter_mut().flat_map(|r| &mut r.cells) {
                    n += restyle(&mut c.blocks, from, to, depth + 1);
                }
            }
        }
    }
    n
}

/// Apply style `id` to the table at the caret, if any.
fn apply(s: &mut Session, id: &str) -> Result<(), CmdError> {
    let Some((tp, _, _)) = s.sel.focus.path.cell() else { return Ok(()) };
    s.doc.table_mut(s.sel.focus.story, &tp)?.props.style = Some(id.to_string());
    super::table::touch_cells(s, &tp)
}

/// Apply the formatting params to `st`.
fn edit(st: &mut Style, v: &Value) -> Result<(), CmdError> {
    for (key, part) in PARTS {
        if let Some(r) = v.get(key) {
            edit_part(st, part, r)?;
        }
    }
    match v.get("part") {
        None => {
            // Part fields at the top level only mean something with `part`; without it they'd be
            // silently dropped.
            if let Some(k) = PART_FIELDS.iter().find(|k| v.get(**k).is_some()) {
                return Err(CmdError::Params(format!("`{k}` is a part field: give `part`, or put it in a part (`wholeTable`, `headerRow`…)")));
            }
        }
        Some(Value::String(n)) => {
            let part = part_named(n).ok_or_else(|| CmdError::Params(format!("no table style part `{n}`")))?;
            edit_part(st, part, v)?;
        }
        Some(_) => return Err(CmdError::Params("`part` is a part name".into())),
    }
    match v.get("parts") {
        None => {}
        Some(Value::Array(list)) => {
            for r in list.iter().take(64) {
                let n = p::str(r, "part").ok_or_else(|| CmdError::Params("each of `parts` names its `part`".into()))?;
                let part = part_named(n).ok_or_else(|| CmdError::Params(format!("no table style part `{n}`")))?;
                edit_part(st, part, r)?;
            }
        }
        Some(_) => return Err(CmdError::Params("`parts` is a list".into())),
    }
    if let Some(n) = v.get("bandSize") {
        let mut parts = st.table.take().unwrap_or_default();
        parts.band_size = n.as_u64().map(|n| n.clamp(1, 1000) as u32);
        st.table = (parts != TableStyleParts::default()).then_some(parts);
    }
    if let Some(pp) = v.get("para") {
        let pp: ParaProps = serde_json::from_value(pp.clone()).map_err(|e| CmdError::Params(format!("para: {e}")))?;
        st.para.overlay(&pp);
        st.para.style = None;
        st.para.numbering = None;
    }
    Ok(())
}

/// The fields of a Part object.
const PART_FIELDS: [&str; 13] =
    ["fill", "font", "size", "bold", "italic", "underline", "color", "align", "vAlign", "cellAlign", "borders", "borderColor", "borderWidth"];

/// Apply one part's params `r` to part `part` of `st`.
fn edit_part(st: &mut Style, part: TablePart, r: &Value) -> Result<(), CmdError> {
    let mut f = st.table_part(part);
    region(r, &mut f)?;
    st.set_table_part(part, f);
    Ok(())
}

/// A colour param: `"RRGGBB"`, or null for none.
fn color_param(r: &Value, k: &str) -> Result<Option<Rgb>, CmdError> {
    match r.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(h)) => Rgb::parse(h).map(Some).ok_or_else(|| CmdError::Params(format!("`{k}`: bad colour `{h}`"))),
        Some(_) => Err(CmdError::Params(format!("`{k}` is \"RRGGBB\" or null"))),
    }
}

/// An explicit "no border" edge (`w:val="nil"`), which hides what a lower part draws.
const NO_LINE: Border = Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 };

/// The border styles by param name.
const LINE_STYLES: [(&str, BorderStyle); 9] = [
    ("none", BorderStyle::None),
    ("single", BorderStyle::Single),
    ("double", BorderStyle::Double),
    ("dotted", BorderStyle::Dotted),
    ("dashed", BorderStyle::Dashed),
    ("dotDash", BorderStyle::DotDash),
    ("thick", BorderStyle::Thick),
    ("triple", BorderStyle::Triple),
    ("wave", BorderStyle::Wave),
];

/// Set `edges` of `b` to `line`, as the border picker does.
pub fn set_edges(b: &mut Borders, edges: &str, line: Border) -> Result<(), CmdError> {
    let all = |b: &mut Borders, l: Border| *b = Borders::all(l);
    match edges {
        "all" => all(b, line),
        "none" => all(b, NO_LINE),
        "outside" | "box" => {
            (b.top, b.bottom, b.left, b.right) = (Some(line), Some(line), Some(line), Some(line));
        }
        "inside" => (b.between, b.inside_v) = (Some(line), Some(line)),
        "top" => b.top = Some(line),
        "bottom" => b.bottom = Some(line),
        "left" => b.left = Some(line),
        "right" => b.right = Some(line),
        "insideH" => b.between = Some(line),
        "insideV" => b.inside_v = Some(line),
        _ => return Err(CmdError::Params(format!("unknown border edges `{edges}`"))),
    }
    Ok(())
}

/// One `Edges` object of the `borders` param.
fn edges_param(e: &Value, b: &mut Borders) -> Result<(), CmdError> {
    if !e.is_object() {
        return Err(CmdError::Params("`borders` is true, false, null or {\"edges\", \"style\", \"width\", \"color\"}".into()));
    }
    let style = match e.get("style") {
        None => BorderStyle::Single,
        Some(Value::String(n)) => LINE_STYLES
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, s)| *s)
            .ok_or_else(|| CmdError::Params(format!("unknown border style `{n}`")))?,
        Some(_) => return Err(CmdError::Params("border `style` is a name".into())),
    };
    let width = p::f32(e, "width").unwrap_or(0.5).clamp(0.25, 6.0);
    let line = if style == BorderStyle::None { NO_LINE } else { Border { style, width, color: color_param(e, "color")?, space: 0.0 } };
    let edges = match e.get("edges") {
        None => "all",
        Some(Value::String(n)) => n.as_str(),
        Some(_) => return Err(CmdError::Params("border `edges` is a name".into())),
    };
    set_edges(b, edges, line)
}

/// Horizontal alignment by param name.
fn h_align(n: &str) -> Option<Align> {
    match n {
        "left" | "start" => Some(Align::Left),
        "center" => Some(Align::Center),
        "right" | "end" => Some(Align::Right),
        "justify" | "both" => Some(Align::Justify),
        _ => None,
    }
}

/// Vertical alignment by param name.
fn v_align(n: &str) -> Option<VAlign> {
    match n {
        "top" => Some(VAlign::Top),
        "center" | "middle" => Some(VAlign::Center),
        "bottom" => Some(VAlign::Bottom),
        _ => None,
    }
}

/// The nine cell alignments (`cellAlign`), vertical then horizontal.
pub const CELL_ALIGNS: [(&str, VAlign, Align); 9] = [
    ("topLeft", VAlign::Top, Align::Left),
    ("topCenter", VAlign::Top, Align::Center),
    ("topRight", VAlign::Top, Align::Right),
    ("centerLeft", VAlign::Center, Align::Left),
    ("center", VAlign::Center, Align::Center),
    ("centerRight", VAlign::Center, Align::Right),
    ("bottomLeft", VAlign::Bottom, Align::Left),
    ("bottomCenter", VAlign::Bottom, Align::Center),
    ("bottomRight", VAlign::Bottom, Align::Right),
];

/// One part's params: borders, fill, character formatting and cell alignment.
fn region(r: &Value, f: &mut PartFormat) -> Result<(), CmdError> {
    if !r.is_object() {
        return Err(CmdError::Params("a part is an object".into()));
    }
    let chr = &mut f.chr;
    if r.get("fill").is_some() {
        f.fill = color_param(r, "fill")?;
    }
    if r.get("color").is_some() {
        chr.color = color_param(r, "color")?.map(TextColor::Rgb);
    }
    if let Some(b) = p::bool(r, "bold") {
        chr.bold = Some(b);
        chr.bold_cs = Some(b);
    }
    if let Some(b) = p::bool(r, "italic") {
        chr.italic = Some(b);
        chr.italic_cs = Some(b);
    }
    match r.get("underline") {
        None => {}
        Some(Value::Bool(b)) => chr.underline = Some(if *b { Underline::Single } else { Underline::None }),
        Some(Value::Null) => chr.underline = None,
        Some(u @ Value::String(_)) => {
            chr.underline = Some(serde_json::from_value(u.clone()).map_err(|_| CmdError::Params(format!("unknown underline {u}")))?);
        }
        Some(_) => return Err(CmdError::Params("`underline` is a bool or a style name".into())),
    }
    if let Some(sz) = p::f32(r, "size") {
        chr.size = Some(sz.clamp(1.0, 1638.0));
        chr.size_cs = chr.size;
    }
    match r.get("font") {
        None => {}
        // Null or empty: the part's font is inherited again.
        Some(Value::Null) => chr.font = None,
        Some(Value::String(n)) => chr.font = Some(n.trim()).filter(|n| !n.is_empty()).map(|n| n.chars().take(256).collect()),
        Some(_) => return Err(CmdError::Params("`font` is a name or null".into())),
    }
    let name = |k: &str| -> Result<Option<Option<&str>>, CmdError> {
        match r.get(k) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(n)) => Ok(Some(Some(n.as_str()))),
            Some(_) => Err(CmdError::Params(format!("`{k}` is a name or null"))),
        }
    };
    if let Some(a) = name("align")? {
        f.align = a.map(|n| h_align(n).ok_or_else(|| CmdError::Params(format!("unknown alignment `{n}`")))).transpose()?;
    }
    if let Some(a) = name("vAlign")? {
        f.v_align = a.map(|n| v_align(n).ok_or_else(|| CmdError::Params(format!("unknown vertical alignment `{n}`")))).transpose()?;
    }
    if let Some(a) = name("cellAlign")? {
        match a {
            None => (f.align, f.v_align) = (None, None),
            Some(n) => {
                let (_, va, ha) = CELL_ALIGNS
                    .iter()
                    .find(|(k, _, _)| k.eq_ignore_ascii_case(n))
                    .ok_or_else(|| CmdError::Params(format!("unknown cell alignment `{n}`")))?;
                (f.align, f.v_align) = (Some(*ha), Some(*va));
            }
        }
    }
    match r.get("borders") {
        Some(Value::Bool(true)) => {
            let width = p::f32(r, "borderWidth").unwrap_or(0.5).clamp(0.25, 6.0);
            f.borders = Some(Borders::all(Border { style: BorderStyle::Single, width, color: color_param(r, "borderColor")?, space: 0.0 }));
        }
        Some(Value::Bool(false)) => f.borders = Some(Borders::all(NO_LINE)),
        Some(Value::Null) => f.borders = None,
        Some(Value::Array(list)) => {
            let mut b = f.borders.unwrap_or_default();
            for e in list.iter().take(64) {
                edges_param(e, &mut b)?;
            }
            f.borders = Some(b);
        }
        Some(e) => {
            let mut b = f.borders.unwrap_or_default();
            edges_param(e, &mut b)?;
            f.borders = Some(b);
        }
        None => {}
    }
    Ok(())
}
