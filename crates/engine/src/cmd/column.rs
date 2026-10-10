//! Column (block) selection: Alt+drag, or Ctrl+Shift+F8 then the arrow keys, selects the same
//! horizontal range on a run of lines. Copy and Cut take one line per block row, Delete /
//! Backspace remove just the block, character formatting applies to each row's piece, paragraph
//! formatting to the paragraphs it touches. Typing or pasting first deletes the block; any other
//! edit collapses it to its top-left corner. Any ordinary caret move or selection drops it.

use serde_json::{Value, json};
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Paragraph, Pos};

use super::{parse_pos, pos_json};
use crate::{CmdError, CmdResult, ColumnBlock, CommandSpec, Selection, Session, p};

/// Most lines a block covers (hostile params: a block over a huge document).
const MAX_LINES: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("select.column", "Column Selection", "Editing › Selection", select_column)
            .key("Mod+Shift+F8")
            .params(
                r#"{"anchor": Pos, "focus": Pos} | {"from": {"page": n, "x": pt, "y": pt}, "to": {"page": n, "x": pt, "y": pt}, "story"?: StoryRef} | {"on"?: bool} (no corners: toggle column selection mode, where the arrow keys extend the block)"#,
            )
            .pure(),
    ]
}

/// Result: the corners, each block row's text and whether column mode is on.
fn result(s: &Session) -> CmdResult {
    let rows: Vec<String> = s.column_segments().map(|segs| segs.iter().map(|(a, b)| row_text(s, a, b)).collect()).unwrap_or_default();
    Ok(json!({"anchor": pos_json(&s.sel.anchor), "focus": pos_json(&s.sel.focus), "rows": rows, "columnMode": s.column_mode}))
}

fn row_text(s: &Session, a: &Pos, b: &Pos) -> String {
    let (a, b) = (s.doc.clamp(a), s.doc.clamp(b));
    if a.path != b.path || a.story != b.story {
        return String::new();
    }
    s.doc.para_at(&a).and_then(|p| p.text.get(a.off.min(b.off)..b.off.max(a.off))).map(str::to_string).unwrap_or_default()
}

/// `{"page": n, "x": pt, "y": pt}`.
fn point(v: &Value, name: &str) -> Result<(usize, f32, f32), CmdError> {
    match (p::u64(v, "page"), p::f32(v, "x"), p::f32(v, "y")) {
        (Some(pg), Some(x), Some(y)) => Ok((usize::try_from(pg).unwrap_or(usize::MAX), x, y)),
        _ => Err(CmdError::Params(format!("`{name}` needs `page`, `x` and `y`"))),
    }
}

fn select_column(s: &mut Session, v: &Value) -> CmdResult {
    if let (Some(av), Some(fv)) = (v.get("anchor"), v.get("focus")) {
        let a = parse_pos(av).ok_or_else(|| CmdError::Params("bad `anchor`".into()))?;
        let b = parse_pos(fv).ok_or_else(|| CmdError::Params("bad `focus`".into()))?;
        let (a, b) = (s.doc.clamp(&a), s.doc.clamp(&b));
        if a.story != b.story {
            return Err(CmdError::Params("`anchor` and `focus` must be in the same story".into()));
        }
        let l = s.layout();
        let xa = l.caret_on(&a, s.page_hint).map(|c| c.x).ok_or_else(|| CmdError::Failed("`anchor` isn't laid out".into()))?;
        let xb = l.caret_on(&b, s.page_hint).map(|c| c.x).ok_or_else(|| CmdError::Failed("`focus` isn't laid out".into()))?;
        make(s, a, b, xa, xb);
        return result(s);
    }
    if let (Some(fv), Some(tv)) = (v.get("from"), v.get("to")) {
        let (fp, fx, fy) = point(fv, "from")?;
        let (tp, tx, ty) = point(tv, "to")?;
        let story = super::story_param(s, v);
        let l = s.layout();
        let a = l.hit(fp, fx, fy, story).ok_or_else(|| CmdError::Params("no text at `from`".into()))?;
        let b = l.hit(tp, tx, ty, story).ok_or_else(|| CmdError::Params("no text at `to`".into()))?;
        s.page_hint = tp;
        make(s, a, b, fx, tx);
        return result(s);
    }
    // Ctrl+Shift+F8: toggle the mode; the block starts at the caret.
    let on = p::bool(v, "on").unwrap_or(!s.column_mode);
    s.column_mode = on;
    if on {
        let f = s.sel.focus.clone();
        let x = s.layout().caret_on(&f, s.page_hint).map(|c| c.x).unwrap_or(0.0);
        make(s, f.clone(), f, x, x);
        s.status = "Column selection: the arrow keys extend the block, Esc ends it".into();
    } else {
        s.status.clear();
    }
    result(s)
}

/// Make the block between corners `a` and `b` whose page x are `xa` and `xb`.
fn make(s: &mut Session, a: Pos, b: Pos, xa: f32, xb: f32) {
    let (xa, xb) = (if xa.is_finite() { xa } else { 0.0 }, if xb.is_finite() { xb } else { 0.0 });
    let (left, right) = (xa.min(xb), xa.max(xb));
    let l = s.layout();
    let segments = l.column_segments(&s.doc, &a, &b, left, right, s.page_hint, MAX_LINES);
    s.sel = Selection { anchor: a, focus: b };
    s.column = Some(ColumnBlock { sel: s.sel.clone(), anchor_x: xa, focus_x: xb, segments });
    s.pending = None;
}

/// Column mode: the caret moved to `to`; grow the block to it (keeping the goal column on
/// Up/Down).
fn extend_to(s: &mut Session, to: Pos) {
    let Some(block) = s.column.clone() else { return };
    let x = match s.goal_x {
        Some(gx) => gx,
        None => s.layout().caret_on(&to, s.page_hint).map(|c| c.x).unwrap_or(block.focus_x),
    };
    make(s, block.sel.anchor, to, block.anchor_x, x);
}

/// Run a command with the column block in mind (called by `Session::run` for every command).
pub(crate) fn dispatch(s: &mut Session, id: &str, mutates: bool, run: fn(&mut Session, &Value) -> CmdResult, v: &Value) -> CmdResult {
    // A block the selection has moved away from is gone.
    if s.column.as_ref().is_some_and(|c| c.sel != s.sel) {
        s.column = None;
        s.column_mode = false;
    }
    if s.column_mode && id.starts_with("caret.") && id != "caret.set" {
        let mut params = v.clone();
        match params.as_object_mut() {
            Some(o) => {
                o.insert("extend".into(), Value::Bool(true));
            }
            None => params = json!({"extend": true}),
        }
        run(s, &params)?;
        let to = s.sel.focus.clone();
        extend_to(s, to);
        return result(s);
    }
    if matches!(id, "caret.set" | "select.collapse") || (mutates && !id.starts_with("format.") && !id.starts_with("para.")) {
        s.column_mode = false;
    }
    let Some(segs) = s.column_segments().map(<[(Pos, Pos)]>::to_vec) else { return run(s, v) };
    match id {
        "edit.copy" => copy(s, &segs),
        "edit.cut" => {
            let r = copy(s, &segs)?;
            delete(s, &segs)?;
            Ok(r)
        }
        "text.delete" | "text.backspace" | "text.deleteWordBack" | "text.deleteWordForward" => {
            delete(s, &segs)?;
            super::sel_result(s)
        }
        "format.changeCase" | "format.clear" => per_row(s, run, v, &segs),
        _ if id.starts_with("format.") || id.starts_with("para.") || id == "select.column" => run(s, v),
        _ if mutates && (id.starts_with("text.") || id.starts_with("edit.paste")) => {
            delete(s, &segs)?;
            run(s, v)
        }
        _ if mutates => {
            let at = segs.first().map(|(a, _)| s.doc.clamp(a)).unwrap_or_else(|| s.sel.focus.clone());
            s.sel = Selection::caret(at);
            s.column = None;
            run(s, v)
        }
        _ => run(s, v),
    }
}

/// Copy: one paragraph per block row (formatting kept); the plain text joins rows with newlines.
fn copy(s: &mut Session, segs: &[(Pos, Pos)]) -> CmdResult {
    let mut blocks = Vec::new();
    let mut parts = std::collections::BTreeMap::new();
    let mut rows = Vec::new();
    for (a, b) in segs {
        let (a, b) = (s.doc.clamp(a), s.doc.clamp(b));
        let frag = if a.path == b.path && a.story == b.story && a != b { s.doc.copy_range(&a, &b) } else { Fragment::default() };
        // Text boxes in a row keep their stories (keyed by this document's part ids).
        parts.extend(frag.parts);
        let mut para = frag
            .blocks
            .into_iter()
            .find_map(|bl| match bl {
                Block::Para(p) => Some(p),
                Block::Table(_) => None,
            })
            .unwrap_or_else(|| Paragraph::with_text("", CharProps::default()));
        para.section = None;
        rows.push(row_text(s, &a, &b));
        blocks.push(Block::Para(para));
    }
    s.clipboard_text = rows.join("\n");
    s.clipboard = Some(Fragment { blocks, parts });
    Ok(json!({"text": s.clipboard_text}))
}

/// Delete each row's piece (bottom up, so earlier positions stay valid) and leave the caret at
/// the block's top-left corner.
fn delete(s: &mut Session, segs: &[(Pos, Pos)]) -> Result<(), CmdError> {
    for (a, b) in segs.iter().rev() {
        let (a, b) = (s.doc.clamp(a), s.doc.clamp(b));
        if a == b || a.path != b.path || a.story != b.story {
            continue;
        }
        if s.doc.settings.track_changes {
            super::track_delete(s, &a, &b)?;
        } else {
            s.doc.delete_range(&a, &b)?;
        }
    }
    let at = segs.first().map(|(a, _)| s.doc.clamp(a)).unwrap_or_else(|| s.sel.focus.clone());
    s.sel = Selection::caret(at);
    s.column = None;
    s.column_mode = false;
    s.goal_x = None;
    s.pending = None;
    Ok(())
}

/// Run a selection command once per non-empty row, then restore the block.
fn per_row(s: &mut Session, run: fn(&mut Session, &Value) -> CmdResult, v: &Value, segs: &[(Pos, Pos)]) -> CmdResult {
    let block = s.column.take();
    for (a, b) in segs.iter().rev() {
        let (a, b) = (s.doc.clamp(a), s.doc.clamp(b));
        if a == b || a.path != b.path {
            continue;
        }
        s.sel = Selection { anchor: a, focus: b };
        if let Err(e) = run(s, v) {
            s.column = block;
            return Err(e);
        }
    }
    if let Some(c) = &block {
        s.sel = c.sel.clone();
    }
    s.column = block;
    result(s)
}
