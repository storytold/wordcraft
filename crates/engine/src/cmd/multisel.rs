//! Separate pieces of text selected at once (#788): Ctrl/⌘+drag or Ctrl/⌘+double-click with text
//! selected adds another piece, as in Word. The pieces are a [`ColumnBlock`] with `pieces` set,
//! so they behave like a column selection (`column.rs`): Copy and Cut take one paragraph per
//! piece (a piece across paragraphs one per paragraph), Delete / Backspace remove every piece,
//! typing or pasting replaces them, character and paragraph formatting apply to each, and any
//! caret move or ordinary selection drops them.

use serde_json::{Value, json};
use wordcraft_doc::Pos;

use super::caret::{Unit, unit_range};
use super::{parse_pos, pos_json};
use crate::{CmdError, CmdResult, ColumnBlock, CommandSpec, Selection, Session};

/// Most pieces a selection holds (hostile params: endless additions).
const MAX_PIECES: usize = 10_000;
/// Most one-paragraph segments the pieces split into (a piece over a huge document).
const MAX_SEGMENTS: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("select.add", "Add to Selection", "Navigation", select_add)
            .params(
                r#"{"anchor": Pos, "focus": Pos} | {"from": {"page": n, "x": pt, "y": pt}, "to": {"page": n, "x": pt, "y": pt}, "story"?: StoryRef} | {"word": Pos} (adds the range, or the word at `word`, to the selection as a separate piece)"#,
            )
            .pure(),
    ]
}

/// Result: the selection and each piece's text, in document order.
fn result(s: &Session) -> CmdResult {
    let pieces: Vec<String> = match s.selection_pieces() {
        Some(p) => p.iter().map(|(a, b)| s.doc.copy_range(a, b).plain_text()).collect(),
        None => {
            let (a, b) = s.sel.ordered();
            if a == b { Vec::new() } else { vec![s.doc.copy_range(&a, &b).plain_text()] }
        }
    };
    Ok(json!({"anchor": pos_json(&s.sel.anchor), "focus": pos_json(&s.sel.focus), "pieces": pieces}))
}

fn select_add(s: &mut Session, v: &Value) -> CmdResult {
    let (a, b) = if let (Some(av), Some(fv)) = (v.get("anchor"), v.get("focus")) {
        let a = parse_pos(av).ok_or_else(|| CmdError::Params("bad `anchor`".into()))?;
        let b = parse_pos(fv).ok_or_else(|| CmdError::Params("bad `focus`".into()))?;
        (s.doc.clamp(&a), s.doc.clamp(&b))
    } else if let (Some(fv), Some(tv)) = (v.get("from"), v.get("to")) {
        let (fp, fx, fy) = super::column::point(fv, "from")?;
        let (tp, tx, ty) = super::column::point(tv, "to")?;
        let story = super::story_param(s, v);
        let l = s.layout();
        let a = l.hit(fp, fx, fy, story).ok_or_else(|| CmdError::Params("no text at `from`".into()))?;
        let b = l.hit(tp, tx, ty, story).ok_or_else(|| CmdError::Params("no text at `to`".into()))?;
        s.page_hint = tp;
        (a, b)
    } else if let Some(wv) = v.get("word") {
        let at = parse_pos(wv).ok_or_else(|| CmdError::Params("bad `word`".into()))?;
        let at = s.doc.clamp(&at);
        unit_range(s, &at, Unit::Word)
    } else {
        return Err(CmdError::Params("needs `anchor` and `focus`, `from` and `to`, or `word`".into()));
    };
    if a.story != b.story {
        return Err(CmdError::Params("a piece must be in one story".into()));
    }
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    // The pieces so far: the current ones, or the selected text as the first.
    let mut pieces = match s.selection_pieces() {
        Some(p) => p.to_vec(),
        None => {
            let (x, y) = s.sel.ordered();
            if x != y { vec![(x, y)] } else { Vec::new() }
        }
    };
    // All pieces are in one story; adding in another starts again.
    pieces.retain(|(x, _)| x.story == a.story);
    if a == b {
        return result(s);
    }
    if pieces.len() >= MAX_PIECES {
        return Err(CmdError::Params(format!("a selection holds at most {MAX_PIECES} pieces")));
    }
    pieces.push((a.clone(), b.clone()));
    let pieces = normalise(pieces);
    s.column_mode = false;
    s.goal_x = None;
    s.pending = None;
    match pieces.as_slice() {
        // Pieces that merged into one are an ordinary selection.
        [(x, y)] => {
            s.sel = Selection { anchor: x.clone(), focus: y.clone() };
            s.column = None;
        }
        _ => {
            let segments = segments(s, &pieces);
            s.sel = Selection { anchor: a, focus: b };
            s.column = Some(ColumnBlock { sel: s.sel.clone(), anchor_x: 0.0, focus_x: 0.0, segments, cells: None, pieces: Some(pieces) });
        }
    }
    result(s)
}

/// Sort ordered pieces and merge those that overlap or touch.
fn normalise(mut pieces: Vec<(Pos, Pos)>) -> Vec<(Pos, Pos)> {
    pieces.sort();
    let mut out: Vec<(Pos, Pos)> = Vec::with_capacity(pieces.len());
    for (a, b) in pieces {
        match out.last_mut() {
            Some(last) if a <= last.1 => {
                if b > last.1 {
                    last.1 = b;
                }
            }
            _ => out.push((a, b)),
        }
    }
    out
}

/// Each piece split at paragraph ends: one `(start, end)` per paragraph it covers.
fn segments(s: &Session, pieces: &[(Pos, Pos)]) -> Vec<(Pos, Pos)> {
    let mut out = Vec::new();
    for (a, b) in pieces {
        if a.path == b.path {
            out.push((a.clone(), b.clone()));
        } else {
            for path in s.doc.para_paths_in(a, b) {
                let start = if path == a.path { a.off } else { 0 };
                let end = if path == b.path { b.off } else { s.doc.para(a.story, &path).map_or(0, |p| p.len()) };
                out.push((Pos { story: a.story, path: path.clone(), off: start }, Pos { story: a.story, path, off: end }));
                if out.len() >= MAX_SEGMENTS {
                    return out;
                }
            }
        }
        if out.len() >= MAX_SEGMENTS {
            break;
        }
    }
    out
}
