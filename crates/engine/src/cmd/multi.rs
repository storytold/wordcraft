//! Multiple selection (#1031): with Ctrl/⌘ held, dragging over text, clicking a sentence or
//! double-clicking a word adds it to what is already selected, as in Word. The pieces are kept as
//! a [`ColumnBlock`] whose `ranges` are the pieces as they were made (the last is `Session::sel`)
//! and whose `segments` are their parts paragraph by paragraph, merged where they overlap, so the
//! selection is painted, counted, copied, deleted and character-formatted the way a column block
//! is (`column::dispatch`). Here only what differs: paragraph formatting and styles go piece by
//! piece; typing, pasting and other edits act on the piece added last and drop the others.

use serde_json::{Value, json};
use wordcraft_doc::Pos;

use super::caret::{Unit, unit_range};
use super::{parse_pos, pos_json};
use crate::{CmdError, CmdResult, ColumnBlock, CommandSpec, Selection, Session, p};

/// Most pieces one selection holds (hostile params: a script adding ranges forever).
const MAX_RANGES: usize = 10_000;
/// Most paragraph pieces the ranges cover together (hostile params: ranges over a huge document).
const MAX_SEGMENTS: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("select.add", "Add to Selection", "Editing › Selection", select_add)
            .params(
                r#"{"anchor": Pos, "focus": Pos, "replaceLast"?: bool} | {"pos": Pos, "unit"?: "word|sentence|paragraph" (default sentence), "replaceLast"?: bool} (Ctrl/⌘+drag, Ctrl/⌘+click, Ctrl/⌘+double-click: keep what is selected and add this; `replaceLast` swaps the piece added last, as a drag goes on)"#,
            )
            .pure(),
    ]
}

/// The pieces the selection is now: a multiple selection's own, a column block's or whole cells'
/// rows, or the selection itself (none for a caret).
fn current(s: &Session) -> Vec<Selection> {
    if let Some(c) = s.column.as_ref().filter(|c| c.sel == s.sel) {
        if !c.ranges.is_empty() {
            return c.ranges.clone();
        }
        if let Some(segs) = s.column_segments() {
            return segs.iter().filter(|(a, b)| a != b).map(|(a, b)| Selection { anchor: a.clone(), focus: b.clone() }).collect();
        }
    }
    if s.sel.is_collapsed() { Vec::new() } else { vec![s.sel.clone()] }
}

fn select_add(s: &mut Session, v: &Value) -> CmdResult {
    let piece = if let Some(pv) = v.get("pos") {
        let at = s.doc.clamp(&parse_pos(pv).ok_or_else(|| CmdError::Params("bad `pos`".into()))?);
        let unit = match p::str(v, "unit") {
            None => Unit::Sentence,
            Some(u @ ("word" | "sentence" | "paragraph")) => Unit::parse(u).unwrap_or(Unit::Sentence),
            Some(_) => return Err(CmdError::Params("`unit` must be word, sentence or paragraph".into())),
        };
        let (a, b) = unit_range(s, &at, unit);
        Selection { anchor: a, focus: b }
    } else {
        let get = |k: &str| v.get(k).and_then(parse_pos).ok_or_else(|| CmdError::Params(format!("bad `{k}`")));
        let (a, b) = (get("anchor")?, get("focus")?);
        Selection { anchor: s.doc.clamp(&a), focus: s.doc.clamp(&b) }
    };
    if piece.anchor.story != piece.focus.story {
        return Err(CmdError::Params("`anchor` and `focus` must be in the same story".into()));
    }
    let mut ranges = current(s);
    if p::bool(v, "replaceLast") == Some(true) {
        ranges.pop();
    }
    // One story at a time: a piece in another story (a header, a text box) starts again there.
    ranges.retain(|r| r.anchor.story == piece.anchor.story);
    let added = !piece.is_collapsed();
    if added {
        if ranges.len() >= MAX_RANGES {
            return Err(CmdError::Params(format!("a selection holds at most {MAX_RANGES} pieces")));
        }
        ranges.push(piece.clone());
    }
    set(s, ranges, piece.focus)?;
    Ok(result(s, added))
}

/// Make the selection `ranges` (the last one is the main selection); none leaves a caret at `caret`.
fn set(s: &mut Session, ranges: Vec<Selection>, caret: Pos) -> Result<(), CmdError> {
    let segments = segments(s, &ranges)?;
    s.column_mode = false;
    s.goal_x = None;
    s.pending = None;
    s.widened = None;
    match ranges.last() {
        None => {
            s.sel = Selection::caret(caret);
            s.column = None;
        }
        Some(last) if ranges.len() == 1 => {
            s.sel = last.clone();
            s.column = None;
        }
        Some(last) => {
            s.sel = last.clone();
            s.column = Some(ColumnBlock { sel: last.clone(), anchor_x: 0.0, focus_x: 0.0, segments, cells: None, ranges });
        }
    }
    Ok(())
}

/// The pieces' parts paragraph by paragraph, in document order, merged where they overlap or
/// touch (a column block's segments are each inside one paragraph).
fn segments(s: &Session, ranges: &[Selection]) -> Result<Vec<(Pos, Pos)>, CmdError> {
    let mut parts: Vec<(Pos, Pos)> = Vec::new();
    for r in ranges {
        let (a, b) = r.ordered();
        for path in s.doc.paths_between(&a, &b) {
            let len = s.doc.para(a.story, &path).map_or(0, |p| p.len());
            let start = if path == a.path { a.off.min(len) } else { 0 };
            let end = if path == b.path { b.off.min(len) } else { len };
            if start < end {
                parts.push((Pos { story: a.story, path: path.clone(), off: start }, Pos { story: a.story, path, off: end }));
            }
            if parts.len() > MAX_SEGMENTS {
                return Err(CmdError::Params(format!("a selection covers at most {MAX_SEGMENTS} paragraph pieces")));
            }
        }
    }
    parts.sort();
    let mut out: Vec<(Pos, Pos)> = Vec::with_capacity(parts.len());
    for (a, b) in parts {
        match out.last_mut() {
            Some((_, end)) if end.path == a.path && end.story == a.story && a.off <= end.off => {
                if b.off > end.off {
                    end.off = b.off;
                }
            }
            _ => out.push((a, b)),
        }
    }
    Ok(out)
}

/// The pieces' own spans, in document order, merged where one overlaps or touches the next
/// (unlike [`segments`], a span here can cross a paragraph break).
fn coalesced(ranges: &[Selection]) -> Vec<(Pos, Pos)> {
    let mut spans: Vec<(Pos, Pos)> = ranges.iter().map(Selection::ordered).collect();
    spans.sort();
    let mut out: Vec<(Pos, Pos)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some((_, end)) if a <= *end => {
                if b > *end {
                    *end = b;
                }
            }
            _ => out.push((a, b)),
        }
    }
    out
}

/// Delete every piece by its own span (bottom up, so earlier positions stay valid), so a piece
/// crossing a paragraph break joins the paragraphs like an ordinary continuous delete.
fn delete_pieces(s: &mut Session) -> CmdResult {
    let Some(block) = s.column.take() else { return super::sel_result(s) };
    let spans = coalesced(&block.ranges);
    for (a, b) in spans.iter().rev() {
        let (a, b) = (s.doc.clamp(a), s.doc.clamp(b));
        if a == b {
            continue;
        }
        if s.doc.settings.track_changes {
            super::track_delete(s, &a, &b)?;
        } else {
            s.doc.delete_range(&a, &b)?;
        }
    }
    let at = spans.first().map(|(a, _)| s.doc.clamp(a)).unwrap_or_else(|| s.sel.focus.clone());
    s.sel = Selection::caret(at);
    s.column_mode = false;
    s.goal_x = None;
    s.pending = None;
    super::sel_result(s)
}

/// Copy every piece, each in a paragraph of its own as a column block does, then delete them by
/// span so a piece crossing a paragraph break joins the paragraphs left behind.
fn cut_pieces(s: &mut Session) -> CmdResult {
    let segs = s.column_segments().map(<[(Pos, Pos)]>::to_vec).unwrap_or_default();
    let r = super::column::copy(s, &segs)?;
    delete_pieces(s)?;
    Ok(r)
}

/// The selection, its pieces with their text, and whether this call added one.
fn result(s: &Session, added: bool) -> Value {
    let ranges: Vec<Value> = current(s)
        .iter()
        .map(|r| {
            let (a, b) = r.ordered();
            json!({"anchor": pos_json(&r.anchor), "focus": pos_json(&r.focus), "text": s.doc.copy_range(&a, &b).plain_text()})
        })
        .collect();
    json!({"anchor": pos_json(&s.sel.anchor), "focus": pos_json(&s.sel.focus), "ranges": ranges, "added": added})
}

/// Commands that change paragraphs one at a time or move them: with several pieces they act on the
/// piece added last (the main selection), like the rest of the edits.
const LAST_PIECE_ONLY: &[&str] = &[
    "para.moveUp",
    "para.moveDown",
    "para.sort",
    "para.dialog",
    "para.restartNumbering",
    "para.setNumberingValue",
    "para.defineBullet",
    "para.defineNumber",
];

/// Run a command on a multiple selection (`column::dispatch` sends here while one is current;
/// copying, cutting, deleting and character formatting stay with the column block's handling).
/// `None`: not ours, the column block's handling applies.
pub(crate) fn dispatch(s: &mut Session, id: &str, mutates: bool, run: fn(&mut Session, &Value) -> CmdResult, v: &Value) -> Option<CmdResult> {
    if !s.column.as_ref().is_some_and(|c| c.sel == s.sel && !c.ranges.is_empty()) {
        return None;
    }
    if (id.starts_with("para.") && !LAST_PIECE_ONLY.contains(&id)) || id == "styles.apply" {
        return Some(per_piece(s, run, v));
    }
    // Deleting takes each piece by its own span, not the per-paragraph segments a column block
    // uses: a piece crossing a paragraph break must merge the paragraphs, as an ordinary
    // continuous delete does (a column block's segments never merge paragraphs, by design).
    if matches!(id, "text.delete" | "text.backspace" | "text.deleteWordBack" | "text.deleteWordForward") {
        return Some(delete_pieces(s));
    }
    if id == "edit.cut" {
        return Some(cut_pieces(s));
    }
    let column_handles = matches!(id, "edit.copy" | "format.changeCase" | "format.clear" | "format.resetChar") || id.starts_with("format.");
    if column_handles {
        return None;
    }
    // Typing, pasting and every other edit replace or act on the piece added last; the others
    // are let go (Word has one caret to type at).
    if mutates {
        s.column = None;
    }
    Some(run(s, v))
}

/// Paragraph formatting and styles: once per piece (skipping a piece whose paragraphs an earlier
/// one already did, so a toggle isn't undone), then the pieces are selected again.
fn per_piece(s: &mut Session, run: fn(&mut Session, &Value) -> CmdResult, v: &Value) -> CmdResult {
    let Some(block) = s.column.take() else { return run(s, v) };
    let mut done: Vec<wordcraft_doc::Path> = Vec::new();
    let mut out = Ok(Value::Null);
    for r in &block.ranges {
        let (a, b) = (s.doc.clamp(&r.anchor), s.doc.clamp(&r.focus));
        let paths = s.doc.para_paths_in(&a, &b);
        if paths.iter().all(|p| done.contains(p)) {
            continue;
        }
        done.extend(paths);
        s.sel = Selection { anchor: a, focus: b };
        out = run(s, v);
        if out.is_err() {
            break;
        }
    }
    s.sel = block.sel.clone();
    s.column = Some(block);
    out.map(|_| result(s, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::Document;

    fn session(paras: &[&str]) -> Session {
        Session::new(Document::from_text(&paras.join("\n")))
    }

    fn range(s: &mut Session, a: (usize, usize), b: (usize, usize)) {
        s.run("select.range", &json!({"anchor": pos_json(&Pos::body(a.0, a.1)), "focus": pos_json(&Pos::body(b.0, b.1))})).unwrap();
    }

    fn add(s: &mut Session, a: (usize, usize), b: (usize, usize), replace: bool) -> Value {
        s.run("select.add", &json!({"anchor": pos_json(&Pos::body(a.0, a.1)), "focus": pos_json(&Pos::body(b.0, b.1)), "replaceLast": replace}))
            .unwrap()
    }

    fn texts(v: &Value) -> Vec<String> {
        v["ranges"].as_array().unwrap().iter().map(|r| r["text"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn ctrl_drag_adds_a_piece_and_keeps_the_first() {
        let mut s = session(&["alpha beta gamma delta"]);
        range(&mut s, (0, 0), (0, 5));
        let r = add(&mut s, (0, 11), (0, 16), false);
        assert_eq!(texts(&r), ["alpha", "gamma"]);
        assert_eq!(s.selected_text(), "alpha\ngamma");
        assert_eq!(s.column_segments().unwrap().len(), 2);
    }

    #[test]
    fn a_drag_in_progress_swaps_its_own_piece() {
        let mut s = session(&["alpha beta gamma delta"]);
        range(&mut s, (0, 0), (0, 5));
        add(&mut s, (0, 11), (0, 12), false);
        add(&mut s, (0, 11), (0, 14), true);
        let r = add(&mut s, (0, 11), (0, 16), true);
        assert_eq!(texts(&r), ["alpha", "gamma"]);
        // Back where the drag started: only the first piece is left, and nothing was added.
        let r = add(&mut s, (0, 11), (0, 11), true);
        assert_eq!(texts(&r), ["alpha"]);
        assert_eq!(r["added"], false);
        assert!(s.column.is_none(), "one piece is an ordinary selection");
        assert_eq!(s.selected_text(), "alpha");
    }

    #[test]
    fn ctrl_click_adds_the_sentence_and_double_click_a_word() {
        let mut s = session(&["One two. Three four. Five six."]);
        range(&mut s, (0, 0), (0, 3));
        let at = pos_json(&Pos::body(0, 11));
        let r = s.run("select.add", &json!({"pos": at})).unwrap();
        assert_eq!(texts(&r), ["One", "Three four. "]);
        let r = s.run("select.add", &json!({"pos": at, "unit": "word", "replaceLast": true})).unwrap();
        assert_eq!(texts(&r), ["One", "Three "]);
        assert!(s.run("select.add", &json!({"pos": at, "unit": "document"})).is_err());
    }

    #[test]
    fn pieces_across_paragraphs_are_split_and_overlaps_merged() {
        let mut s = session(&["first para", "second para", "third"]);
        range(&mut s, (0, 6), (1, 6));
        add(&mut s, (1, 3), (2, 2), false);
        let segs = s.column_segments().unwrap().to_vec();
        assert_eq!(segs, vec![(Pos::body(0, 6), Pos::body(0, 10)), (Pos::body(1, 0), Pos::body(1, 11)), (Pos::body(2, 0), Pos::body(2, 2))]);
    }

    #[test]
    fn bold_formats_every_piece() {
        let mut s = session(&["alpha beta gamma"]);
        range(&mut s, (0, 0), (0, 5));
        add(&mut s, (0, 11), (0, 16), false);
        s.run("format.bold", &json!({})).unwrap();
        let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
        let bold = |off: usize| p.props_at(off).bold == Some(true);
        assert!(bold(1) && bold(12) && !bold(7));
        // Still both pieces, so the next command takes them too.
        assert_eq!(s.column_segments().unwrap().len(), 2);
        s.undo();
        assert!(!s.doc.para_at(&Pos::body(0, 0)).unwrap().props_at(1).bold.unwrap_or(false));
    }

    #[test]
    fn delete_removes_every_piece_in_one_undo_step() {
        let mut s = session(&["alpha beta gamma"]);
        range(&mut s, (0, 0), (0, 6));
        add(&mut s, (0, 10), (0, 16), false);
        s.run("text.delete", &json!({})).unwrap();
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "beta");
        s.undo();
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "alpha beta gamma");
    }

    #[test]
    fn delete_a_piece_crossing_a_paragraph_break_joins_the_paragraphs() {
        let mut s = session(&["one two", "three four"]);
        range(&mut s, (0, 0), (0, 3));
        add(&mut s, (0, 4), (1, 5), false);
        s.run("text.delete", &json!({})).unwrap();
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "  four");
        assert!(s.doc.para_at(&Pos::body(1, 0)).is_none(), "the paragraph break was removed too");
        s.undo();
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "one two");
        assert_eq!(s.doc.para_at(&Pos::body(1, 0)).unwrap().text, "three four");
    }

    #[test]
    fn backspace_removes_every_piece_like_delete() {
        let mut s = session(&["alpha beta gamma"]);
        range(&mut s, (0, 0), (0, 6));
        add(&mut s, (0, 10), (0, 16), false);
        s.run("text.backspace", &json!({})).unwrap();
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "beta");
        s.undo();
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "alpha beta gamma");
    }

    #[test]
    fn copy_takes_every_piece() {
        let mut s = session(&["alpha beta gamma"]);
        range(&mut s, (0, 0), (0, 5));
        add(&mut s, (0, 11), (0, 16), false);
        s.run("edit.copy", &json!({})).unwrap();
        assert_eq!(s.clipboard_text, "alpha\ngamma");
    }

    #[test]
    fn typing_replaces_the_piece_added_last() {
        let mut s = session(&["alpha beta gamma"]);
        range(&mut s, (0, 0), (0, 5));
        add(&mut s, (0, 11), (0, 16), false);
        s.run("text.insert", &json!({"text": "X"})).unwrap();
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "alpha beta X");
        assert!(s.column_segments().is_none());
    }

    #[test]
    fn paragraph_formatting_takes_every_pieces_paragraph() {
        let mut s = session(&["one", "two", "three"]);
        range(&mut s, (0, 0), (0, 2));
        add(&mut s, (2, 0), (2, 2), false);
        s.run("para.alignCenter", &json!({})).unwrap();
        let centred = |i: usize| s.doc.styles.resolve_para(&s.doc.para_at(&Pos::body(i, 0)).unwrap().props).align;
        assert_eq!(centred(0), centred(2));
        assert_ne!(centred(0), centred(1));
        assert_eq!(s.column_segments().unwrap().len(), 2);
    }

    #[test]
    fn moving_the_caret_drops_the_pieces() {
        let mut s = session(&["alpha beta gamma"]);
        range(&mut s, (0, 0), (0, 5));
        add(&mut s, (0, 11), (0, 16), false);
        s.run("caret.right", &json!({})).unwrap();
        assert!(s.column_segments().is_none());
    }
}
