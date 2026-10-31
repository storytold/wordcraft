//! Caret movement and selection.

use serde_json::Value;
use wordcraft_doc::{Pos, StoryRef};

use super::{parse_pos, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

const EXTEND: &str = r#"{"extend"?: bool}"#;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("caret.left", "Left", "Navigation", |s, v| lr(s, v, true)).key("Left").params(EXTEND).pure(),
        CommandSpec::new("caret.right", "Right", "Navigation", |s, v| lr(s, v, false)).key("Right").params(EXTEND).pure(),
        CommandSpec::new("caret.up", "Up", "Navigation", |s, v| vert(s, v, -1)).key("Up").params(EXTEND).pure(),
        CommandSpec::new("caret.down", "Down", "Navigation", |s, v| vert(s, v, 1)).key("Down").params(EXTEND).pure(),
        // In a right-to-left paragraph the word to the left is the next one. On macOS ⌘←/⌘→ go
        // to the line's start and end, as in Word for Mac and every Mac text field (#954).
        CommandSpec::new("caret.wordLeft", "Word Left", "Navigation", |s, v| mv(s, v, if rtl_para(s) { word_right } else { word_left }))
            .key(super::mac_or("Alt+Left", "Mod+Left / Alt+Left"))
            .params(EXTEND)
            .pure(),
        CommandSpec::new("caret.wordRight", "Word Right", "Navigation", |s, v| mv(s, v, if rtl_para(s) { word_left } else { word_right }))
            .key(super::mac_or("Alt+Right", "Mod+Right / Alt+Right"))
            .params(EXTEND)
            .pure(),
        CommandSpec::new("caret.home", "Line Start", "Navigation", |s, v| line(s, v, true))
            .key(super::mac_or("Home / Mod+Left", "Home"))
            .params(EXTEND)
            .pure(),
        CommandSpec::new("caret.end", "Line End", "Navigation", |s, v| line(s, v, false))
            .key(super::mac_or("End / Mod+Right", "End"))
            .params(EXTEND)
            .pure(),
        CommandSpec::new("caret.paraUp", "Paragraph Up", "Navigation", |s, v| mv(s, v, para_up)).key("Mod+Up / Alt+Up").params(EXTEND).pure(),
        CommandSpec::new("caret.paraDown", "Paragraph Down", "Navigation", |s, v| mv(s, v, para_down))
            .key("Mod+Down / Alt+Down")
            .params(EXTEND)
            .pure(),
        CommandSpec::new("caret.docStart", "Document Start", "Navigation", |s, v| mv(s, v, |s, _| s.doc.start_of(s.sel.focus.story)))
            .key("Mod+Home")
            .params(EXTEND)
            .pure(),
        CommandSpec::new("caret.docEnd", "Document End", "Navigation", |s, v| mv(s, v, |s, _| s.doc.end_of(s.sel.focus.story)))
            .key("Mod+End")
            .params(EXTEND)
            .pure(),
        CommandSpec::new("caret.pageUp", "Page Up", "Navigation", |s, v| page(s, v, -1)).key("PageUp").params(EXTEND).pure(),
        CommandSpec::new("caret.pageDown", "Page Down", "Navigation", |s, v| page(s, v, 1)).key("PageDown").params(EXTEND).pure(),
        CommandSpec::new("caret.set", "Set Caret", "Navigation", set)
            .params(r#"{"pos": Pos, "extend"?: bool} | {"page": n, "x": pt, "y": pt}"#)
            .pure(),
        CommandSpec::new("select.all", "Select All", "Home › Editing › Select", select_all).key("Mod+A").pure(),
        CommandSpec::new("select.range", "Select Range", "Navigation", select_range).params(r#"{"anchor": Pos, "focus": Pos}"#).pure(),
        CommandSpec::new("select.word", "Select Word", "Navigation", select_word).pure(),
        CommandSpec::new("select.sentence", "Select Sentence", "Navigation", select_sentence).pure(),
        CommandSpec::new("select.paragraph", "Select Paragraph", "Navigation", select_paragraph).pure(),
        CommandSpec::new("select.dragUnit", "Extend Selection by Units", "Navigation", drag_unit)
            .params(r#"{"unit": "word|sentence|line|paragraph|document", "start": Pos, "end": Pos, "pos": Pos}"#)
            .pure(),
        CommandSpec::new("select.line", "Select Line", "Navigation", select_line).params(r#"{"pos"?: Pos}"#).pure(),
        CommandSpec::new("select.text", "Select Text", "Navigation", select_text).params(r#"{"text": string, "occurrence"?: n}"#).pure(),
        CommandSpec::new("select.collapse", "Collapse Selection", "Navigation", |s, v| {
            let to_end = p::bool(v, "end").unwrap_or(false);
            let (a, b) = s.sel.ordered();
            s.sel = Selection::caret(if to_end { b } else { a });
            sel_result(s)
        })
        .key("Escape")
        .pure(),
    ]
}

fn extend(v: &Value) -> bool {
    p::bool(v, "extend").unwrap_or(false)
}

fn apply(s: &mut Session, to: Pos, ext: bool) {
    if ext {
        s.sel.focus = to;
    } else {
        s.sel = Selection::caret(to);
    }
    s.pending = None;
}

fn mv(s: &mut Session, v: &Value, f: fn(&mut Session, &Pos) -> Pos) -> CmdResult {
    let ext = extend(v);
    s.goal_x = None;
    let from = s.sel.focus.clone();
    let to = f(s, &from);
    apply(s, to, ext);
    sel_result(s)
}

/// Does the caret's paragraph read right to left?
fn rtl_para(s: &Session) -> bool {
    s.doc.para_at(&s.sel.focus).is_some_and(|p| s.doc.styles.resolve_para(&p.props).bidi)
}

/// Is the caret's paragraph bidirectional (right to left, or with right-to-left text)? Arrow keys
/// move visually there.
fn bidi_para(s: &Session) -> bool {
    s.doc.para_at(&s.sel.focus).is_some_and(|p| wordcraft_doc::bidi::has_rtl(&p.text) || s.doc.styles.resolve_para(&p.props).bidi)
}

/// Left/Right: with a selection and no Shift, collapse to its edge.
fn lr(s: &mut Session, v: &Value, is_left: bool) -> CmdResult {
    if !extend(v) && !s.sel.is_collapsed() {
        let (a, b) = s.sel.ordered();
        // The left edge of a selection in right-to-left text is its logical end.
        let to_start = is_left != rtl_para(s);
        s.sel = Selection::caret(if to_start { a } else { b });
        s.goal_x = None;
        return sel_result(s);
    }
    if bidi_para(s) {
        return visual_lr(s, v, is_left);
    }
    mv(s, v, if is_left { left } else { right })
}

/// Left/Right in bidirectional text: one caret position to the left or right on the screen.
/// Past the line's visual end the caret goes to the neighbouring line in reading order (the
/// previous line when moving toward the paragraph's start edge).
fn visual_lr(s: &mut Session, v: &Value, is_left: bool) -> CmdResult {
    let ext = extend(v);
    s.goal_x = None;
    let from = s.sel.focus.clone();
    let l = s.layout();
    let to = match l.visual_step(&from, is_left, s.page_hint) {
        Some(wordcraft_layout::VisualStep::Moved(p)) => p,
        Some(wordcraft_layout::VisualStep::Edge { start, stop, last_line, rtl }) => {
            let at = |off| Pos { off, ..from.clone() };
            if is_left != rtl {
                match s.doc.para_at(&from) {
                    Some(para) if start > 0 => at(para.prev_boundary(start)),
                    _ => left(s, &at(0)),
                }
            } else if !last_line {
                at(stop)
            } else {
                let len = s.doc.para_at(&from).map_or(stop, |p| p.len());
                right(s, &at(len))
            }
        }
        None if is_left => left(s, &from),
        None => right(s, &from),
    };
    apply(s, to, ext);
    sel_result(s)
}

fn left(s: &mut Session, p: &Pos) -> Pos {
    if let Some(para) = s.doc.para_at(p)
        && p.off > 0
    {
        return Pos { off: para.prev_boundary(p.off), ..p.clone() };
    }
    match s.doc.prev_para(p.story, &p.path) {
        Some(q) => {
            let off = s.doc.para(p.story, &q).map(|x| x.len()).unwrap_or(0);
            Pos { story: p.story, path: q, off }
        }
        None => p.clone(),
    }
}

fn right(s: &mut Session, p: &Pos) -> Pos {
    if let Some(para) = s.doc.para_at(p)
        && p.off < para.len()
    {
        return Pos { off: para.next_boundary(p.off), ..p.clone() };
    }
    match s.doc.next_para(p.story, &p.path) {
        Some(q) => Pos { story: p.story, path: q, off: 0 },
        None => p.clone(),
    }
}

fn word_left(s: &mut Session, p: &Pos) -> Pos {
    match s.doc.para_at(p) {
        Some(para) if p.off > 0 => Pos { off: para.word_start(p.off), ..p.clone() },
        _ => left(s, p),
    }
}

fn word_right(s: &mut Session, p: &Pos) -> Pos {
    match s.doc.para_at(p) {
        Some(para) if p.off < para.len() => Pos { off: para.word_end(p.off), ..p.clone() },
        _ => right(s, p),
    }
}

fn para_up(s: &mut Session, p: &Pos) -> Pos {
    if p.off > 0 {
        return Pos { off: 0, ..p.clone() };
    }
    match s.doc.prev_para(p.story, &p.path) {
        Some(q) => Pos { story: p.story, path: q, off: 0 },
        None => p.clone(),
    }
}

fn para_down(s: &mut Session, p: &Pos) -> Pos {
    match s.doc.next_para(p.story, &p.path) {
        Some(q) => Pos { story: p.story, path: q, off: 0 },
        None => {
            let off = s.doc.para_at(p).map(|x| x.len()).unwrap_or(0);
            Pos { off, ..p.clone() }
        }
    }
}

fn vert(s: &mut Session, v: &Value, dir: i32) -> CmdResult {
    let ext = extend(v);
    let l = s.layout();
    let from = s.sel.focus.clone();
    let to = match l.vertical(&from, s.goal_x, dir, s.page_hint) {
        Some((p, gx)) => {
            s.goal_x = Some(gx);
            p
        }
        None => {
            // Top/bottom of the story: go to its start/end.
            if dir < 0 { s.doc.start_of(from.story) } else { s.doc.end_of(from.story) }
        }
    };
    if let Some(c) = l.caret_on(&to, s.page_hint) {
        s.page_hint = c.page;
    }
    let gx = s.goal_x;
    apply(s, to, ext);
    s.goal_x = gx;
    sel_result(s)
}

fn line(s: &mut Session, v: &Value, home: bool) -> CmdResult {
    let ext = extend(v);
    let l = s.layout();
    let from = s.sel.focus.clone();
    let to = match l.line_bounds(&from, s.page_hint) {
        Some((a, b)) => {
            if home {
                a
            } else {
                b
            }
        }
        None => from,
    };
    s.goal_x = None;
    apply(s, to, ext);
    sel_result(s)
}

fn page(s: &mut Session, v: &Value, dir: i32) -> CmdResult {
    let ext = extend(v);
    let l = s.layout();
    let from = s.sel.focus.clone();
    let Some(c) = l.caret_on(&from, s.page_hint) else { return sel_result(s) };
    let np = if dir < 0 { c.page.saturating_sub(1) } else { (c.page + 1).min(l.pages.len().saturating_sub(1)) };
    let to = l.hit(np, c.x, c.top + 1.0, from.story).unwrap_or(from);
    s.page_hint = np;
    apply(s, to, ext);
    sel_result(s)
}

fn set(s: &mut Session, v: &Value) -> CmdResult {
    let ext = extend(v);
    let to = if let Some(pv) = v.get("pos") {
        parse_pos(pv).ok_or_else(|| CmdError::Params("bad `pos`".into()))?
    } else if let (Some(pg), Some(x), Some(y)) = (p::u64(v, "page"), p::f32(v, "x"), p::f32(v, "y")) {
        let story = super::story_param(s, v);
        let l = s.layout();
        let page = usize::try_from(pg).unwrap_or(usize::MAX);
        s.page_hint = page;
        l.hit(page, x, y, story).ok_or_else(|| CmdError::Params("no text on that page".into()))?
    } else {
        return Err(CmdError::Params("`pos` or `page`/`x`/`y` required".into()));
    };
    let to = s.doc.clamp(&to);
    s.goal_x = None;
    apply(s, to, ext);
    sel_result(s)
}

fn select_all(s: &mut Session, v: &Value) -> CmdResult {
    if s.selects_whole_story() {
        s.column = None;
        s.column_mode = false;
        return sel_result(s);
    }
    if !p::bool(v, "whole").unwrap_or(false)
        && let Some(r) = super::table::grow_select_all(s)
    {
        return r;
    }
    let st = s.sel.focus.story;
    s.sel = Selection { anchor: s.doc.start_of(st), focus: s.doc.end_of(st) };
    s.column = None;
    s.column_mode = false;
    sel_result(s)
}

fn select_range(s: &mut Session, v: &Value) -> CmdResult {
    let a = v.get("anchor").and_then(parse_pos).ok_or_else(|| CmdError::Params("bad `anchor`".into()))?;
    let b = v.get("focus").and_then(parse_pos).ok_or_else(|| CmdError::Params("bad `focus`".into()))?;
    s.sel = Selection { anchor: s.doc.clamp(&a), focus: s.doc.clamp(&b) };
    sel_result(s)
}

/// A unit a selection grows by: what a double-click (word), Ctrl/⌘+click (sentence) or
/// triple-click (paragraph) selects, and what dragging after it extends by. A click in the
/// selection bar (the left margin) selects a line; triple-click there the whole document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Word,
    Sentence,
    Line,
    Paragraph,
    Document,
}

impl Unit {
    pub fn parse(s: &str) -> Option<Unit> {
        match s {
            "word" => Some(Unit::Word),
            "sentence" => Some(Unit::Sentence),
            "line" => Some(Unit::Line),
            "paragraph" => Some(Unit::Paragraph),
            "document" => Some(Unit::Document),
            _ => None,
        }
    }
}

/// The word, sentence, line, paragraph or story around `at`, as (start, end). A paragraph
/// includes its mark (it ends at the start of the next paragraph in the same container, when
/// there is one), and so does its last line.
pub fn unit_range(s: &mut Session, at: &Pos, unit: Unit) -> (Pos, Pos) {
    let Some(p) = s.doc.para_at(at) else { return (at.clone(), at.clone()) };
    let len = p.len();
    let mark_end = |s: &Session| match s.doc.next_para(at.story, &at.path) {
        Some(q) if q.parent() == at.path.parent() => Pos { story: at.story, path: q, off: 0 },
        _ => Pos { off: len, ..at.clone() },
    };
    let (a, b) = match unit {
        Unit::Word => p.word_at(at.off),
        Unit::Sentence => p.sentence_at(at.off),
        Unit::Paragraph => return (Pos { off: 0, ..at.clone() }, mark_end(s)),
        Unit::Document => return (s.doc.start_of(at.story), s.doc.end_of(at.story)),
        Unit::Line => {
            let hint = s.page_hint;
            return match s.layout().line_span(at, hint) {
                Some((a, _, true)) => (a, mark_end(s)),
                Some((a, b, false)) => (a, b),
                None => (at.clone(), at.clone()),
            };
        }
    };
    (Pos { off: a, ..at.clone() }, Pos { off: b, ..at.clone() })
}

fn select_unit(s: &mut Session, unit: Unit) -> CmdResult {
    let f = s.sel.focus.clone();
    if s.doc.para_at(&f).is_some() {
        let (a, b) = unit_range(s, &f, unit);
        s.sel = Selection { anchor: a, focus: b };
    }
    sel_result(s)
}

fn select_word(s: &mut Session, _: &Value) -> CmdResult {
    select_unit(s, Unit::Word)
}

fn select_sentence(s: &mut Session, _: &Value) -> CmdResult {
    select_unit(s, Unit::Sentence)
}

/// A triple-click (`cell: true`) on the last paragraph of a table cell selects the cell (Word
/// takes its end mark too, which shades it), so its borders then move on their own
/// (`table.resizeDrag`). Without `cell` (the ribbon's Select Paragraph) only the paragraph is selected.
fn select_paragraph(s: &mut Session, v: &Value) -> CmdResult {
    let r = select_unit(s, Unit::Paragraph);
    if v.get("cell").and_then(Value::as_bool) != Some(true) {
        return r;
    }
    let (a, b) = (s.sel.anchor.clone(), s.sel.focus.clone());
    let n = a.path.0.len();
    if a.story == b.story
        && a.path.0.len() == b.path.0.len()
        && let Some(tp) = a.path.0.get(..n.saturating_sub(3))
        && !tp.is_empty()
        && a.path.0.get(..n.saturating_sub(1)) == b.path.0.get(..n.saturating_sub(1))
        && let (Some(&row), Some(&cell)) = (a.path.0.get(n.saturating_sub(3)), a.path.0.get(n.saturating_sub(2)))
    {
        let tp = wordcraft_doc::Path(tp.to_vec());
        let (row, cell) = (row as usize, cell as usize);
        let last = s.doc.table(a.story, &tp).and_then(|t| t.rows.get(row)?.cells.get(cell)).map(|c| c.blocks.len().saturating_sub(1));
        let only = last.is_some_and(|l| a.path.0.last().is_some_and(|&i| i as usize == l));
        let grid = s.doc.table(a.story, &tp).map(|t| super::table::grid_cols(t, row, cell));
        if let (true, Some(cols)) = (only, grid) {
            return super::table::select_range(s, a.story, &tp, (row, row), cols);
        }
    }
    r
}

/// A click in the selection bar (#856): the whole line at `pos` (default: the caret's), its
/// trailing spaces and, on a paragraph's last line, the paragraph mark.
fn select_line(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(pos) = v.get("pos").and_then(parse_pos) {
        s.sel = Selection::caret(s.doc.clamp(&pos));
    }
    select_unit(s, Unit::Line)
}

/// Drag after a double-click (words), triple-click (paragraphs) or Ctrl/⌘+click (sentences):
/// the selection keeps the unit first selected (`start`..`end`) and grows by whole units to
/// the one under `pos`. Dragging forward anchors at the unit's start; dragging back before it
/// anchors at its end, as in Word.
fn drag_unit(s: &mut Session, v: &Value) -> CmdResult {
    let unit = p::str(v, "unit")
        .and_then(Unit::parse)
        .ok_or_else(|| CmdError::Params("`unit` must be word, sentence, line, paragraph or document".into()))?;
    let get = |k: &str| v.get(k).and_then(parse_pos).ok_or_else(|| CmdError::Params(format!("bad `{k}`")));
    let (at, start, end) = (s.doc.clamp(&get("pos")?), s.doc.clamp(&get("start")?), s.doc.clamp(&get("end")?));
    let (start, end) = if start <= end { (start, end) } else { (end, start) };
    // Only within one story (the pointer over another story keeps the selection).
    if at.story != start.story || end.story != start.story {
        return sel_result(s);
    }
    let (us, ue) = unit_range(s, &at, unit);
    s.sel = if unit == Unit::Document {
        Selection { anchor: us, focus: ue }
    } else if at >= start {
        Selection { focus: ue.max(end.clone()), anchor: start }
    } else {
        Selection { focus: us.min(start), anchor: end }
    };
    s.goal_x = None;
    s.pending = None;
    sel_result(s)
}

/// Select the n-th occurrence of `text` in the body (agents: "select the word X").
fn select_text(s: &mut Session, v: &Value) -> CmdResult {
    let needle = p::req_str(v, "text")?;
    let n = p::u64(v, "occurrence").unwrap_or(1).max(1) as usize;
    if needle.is_empty() {
        return Err(CmdError::Params("`text` is empty".into()));
    }
    let mut count = 0;
    let story = super::story_param(s, v);
    for path in s.doc.para_paths(story) {
        let Some(p) = s.doc.para(story, &path) else { continue };
        for (i, _) in p.text.match_indices(needle) {
            count += 1;
            if count == n {
                s.sel =
                    Selection { anchor: Pos { story, path: path.clone(), off: i }, focus: Pos { story, path: path.clone(), off: i + needle.len() } };
                return sel_result(s);
            }
        }
    }
    let _ = StoryRef::Body;
    Err(CmdError::Failed(format!("`{needle}` not found")))
}
