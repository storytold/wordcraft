//! Outline view and the Outlining tab: outline levels (Heading 1–9 or body text), promote and
//! demote, moving paragraphs with what is under them, expand and collapse, Show Level, Show First
//! Line Only, Show Text Formatting. What's collapsed and shown is view state ([`OutlineState`]),
//! never saved in the document; level changes and moves are ordinary undoable edits.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::{Block, Document, Pos, StoryRef};
use wordcraft_layout::ViewMode;
use wordcraft_layout::outline::{self as ol, BODY, OutlineView};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// What Outline view shows (part of [`crate::ViewState`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OutlineState {
    /// Collapsed headings: body block index and a hash of the heading's text, so they are found
    /// again after edits move them (see [`after_command`]).
    pub collapsed: Vec<(usize, u64)>,
    /// Show Level: 1–9, or 0 for all levels.
    pub show_level: u8,
    /// Show First Line Only (body text).
    pub first_line_only: bool,
    /// Show Text Formatting turned off.
    pub hide_formatting: bool,
}

impl OutlineState {
    /// What the layout needs.
    pub fn view(&self) -> OutlineView {
        OutlineView {
            collapsed: self.collapsed.iter().map(|(i, _)| *i).collect(),
            show_level: self.show_level,
            first_line_only: self.first_line_only,
            plain: self.hide_formatting,
        }
    }
}

fn in_outline(s: &Session) -> Option<&'static str> {
    (s.view.mode != ViewMode::Outline).then_some("only in Outline view")
}

/// Runs another command as part of this one: it adds no undo checkpoint of its own, so one
/// outline command is one undo step.
fn run_joined(s: &mut Session, id: &str) -> CmdResult {
    s.join_next_undo();
    s.run(id, &json!({}))
}

const LEVEL: &str = r#"{"level": 1-9 | "body"}"#;
const BLOCK: &str = r#"{"block"?: body paragraph index (default: the selection)}"#;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("outline.promoteToHeading1", "Promote to Heading 1", "Outlining › Outline Tools", |s, _| set_levels(s, Some(1))),
        CommandSpec::new("outline.promote", "Promote", "Outlining › Outline Tools", |s, _| run_joined(s, "para.promote")),
        CommandSpec::new("outline.demote", "Demote", "Outlining › Outline Tools", |s, _| run_joined(s, "para.demote")),
        CommandSpec::new("outline.demoteToBody", "Demote to Body Text", "Outlining › Outline Tools", |s, _| set_levels(s, None)),
        CommandSpec::new("outline.level", "Outline Level", "Outlining › Outline Tools", |s, v| {
            let level = match v.get("level") {
                Some(Value::String(b)) if b.eq_ignore_ascii_case("body") => None,
                Some(x) => match x.as_u64() {
                    Some(n @ 1..=9) => Some(n as u8),
                    _ => return Err(CmdError::Params(format!("`level` must be 1–9 or \"body\": {LEVEL}"))),
                },
                None => return Err(CmdError::Params(format!("`level` is required: {LEVEL}"))),
            };
            set_levels(s, level)
        })
        .params(LEVEL),
        CommandSpec::new("outline.moveUp", "Move Up", "Outlining › Outline Tools", |s, _| run_joined(s, "para.moveUp")),
        CommandSpec::new("outline.moveDown", "Move Down", "Outlining › Outline Tools", |s, _| run_joined(s, "para.moveDown")),
        CommandSpec::new("outline.expand", "Expand", "Outlining › Outline Tools", |s, v| expand(s, v, false))
            .key("Alt+Shift+=")
            .params(BLOCK)
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.collapse", "Collapse", "Outlining › Outline Tools", |s, v| collapse(s, v, false))
            .key("Alt+Shift+-")
            .params(BLOCK)
            .when(in_outline)
            .pure(),
        // Double-clicking a heading's symbol: all of it collapsed, or all of it expanded.
        CommandSpec::new("outline.toggle", "Expand or Collapse Heading", "Outlining › Outline Tools", |s, v| {
            let collapsed = targets(s, v).first().is_some_and(|h| s.view.outline.collapsed.iter().any(|(i, _)| i == h));
            if collapsed { expand(s, v, true) } else { collapse(s, v, true) }
        })
        .params(BLOCK)
        .when(in_outline)
        .pure(),
        // Clicking a symbol: the paragraph and everything under it.
        CommandSpec::new("outline.selectSubtree", "Select Heading and Content", "Outlining › Outline Tools", select_subtree)
            .params(BLOCK)
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel", "Show Level", "Outlining › Outline Tools", |s, v| {
            let level = match v.get("level") {
                Some(Value::String(a)) if a.eq_ignore_ascii_case("all") => 0,
                Some(x) => match x.as_u64() {
                    Some(n @ 1..=9) => n as u8,
                    _ => return Err(CmdError::Params(r#"`level` must be 1–9 or "all""#.into())),
                },
                None => return Err(CmdError::Params(r#"`level` is required: 1–9 or "all""#.into())),
            };
            show_level(s, level)
        })
        .params(r#"{"level": 1-9 | "all"}"#)
        .when(in_outline)
        .pure(),
        CommandSpec::new("outline.showLevel1", "Show Level 1", "Outlining › Outline Tools", |s, _| show_level(s, 1))
            .key("Alt+Shift+1")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel2", "Show Level 2", "Outlining › Outline Tools", |s, _| show_level(s, 2))
            .key("Alt+Shift+2")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel3", "Show Level 3", "Outlining › Outline Tools", |s, _| show_level(s, 3))
            .key("Alt+Shift+3")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel4", "Show Level 4", "Outlining › Outline Tools", |s, _| show_level(s, 4))
            .key("Alt+Shift+4")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel5", "Show Level 5", "Outlining › Outline Tools", |s, _| show_level(s, 5))
            .key("Alt+Shift+5")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel6", "Show Level 6", "Outlining › Outline Tools", |s, _| show_level(s, 6))
            .key("Alt+Shift+6")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel7", "Show Level 7", "Outlining › Outline Tools", |s, _| show_level(s, 7))
            .key("Alt+Shift+7")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel8", "Show Level 8", "Outlining › Outline Tools", |s, _| show_level(s, 8))
            .key("Alt+Shift+8")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showLevel9", "Show Level 9", "Outlining › Outline Tools", |s, _| show_level(s, 9))
            .key("Alt+Shift+9")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.showAll", "Show All Levels", "Outlining › Outline Tools", |s, _| show_level(s, 0))
            .key("Alt+Shift+A")
            .when(in_outline)
            .pure(),
        CommandSpec::new("outline.firstLineOnly", "Show First Line Only", "Outlining › Outline Tools", |s, v| {
            s.view.outline.first_line_only = p::bool(v, "value").unwrap_or(!s.view.outline.first_line_only);
            Ok(json!({"value": s.view.outline.first_line_only}))
        })
        .key(super::non_mac("Alt+Shift+L"))
        .params(r#"{"value"?: bool}"#)
        .when(in_outline)
        .pure(),
        CommandSpec::new("outline.showFormatting", "Show Text Formatting", "Outlining › Outline Tools", |s, v| {
            let on = p::bool(v, "value").unwrap_or(s.view.outline.hide_formatting);
            s.view.outline.hide_formatting = !on;
            Ok(json!({"value": on}))
        })
        .params(r#"{"value"?: bool}"#)
        .when(in_outline)
        .pure(),
        CommandSpec::new("outline.close", "Close Outline View", "Outlining › Close", |s, _| s.run("view.printLayout", &json!({})))
            .when(in_outline)
            .pure(),
        // Collapsed headings in the other views (#1745) are the stored "Collapsed by default"
        // flag (Word for Mac's only control), so a fold is undoable and saved with the file.
        CommandSpec::new("outline.expandHeading", "Expand Heading", "Home › Paragraph", |s, v| fold_heading(s, v, Some(false))).params(BLOCK),
        CommandSpec::new("outline.collapseHeading", "Collapse Heading", "Home › Paragraph", |s, v| fold_heading(s, v, Some(true))).params(BLOCK),
        // The fold arrow beside a heading in the left margin.
        CommandSpec::new("outline.toggleHeading", "Expand or Collapse Heading", "Home › Paragraph", |s, v| fold_heading(s, v, None)).params(BLOCK),
        CommandSpec::new("outline.expandAllHeadings", "Expand All Headings", "Home › Paragraph", |s, _| fold_all(s, false)),
        CommandSpec::new("outline.collapseAllHeadings", "Collapse All Headings", "Home › Paragraph", |s, _| fold_all(s, true)),
        CommandSpec::new("outline.state", "Outline State", "Outlining", |s, _| {
            let levels = ol::levels(&s.doc);
            let hidden = ol::hidden(&levels, &s.view.outline.view());
            let shown: Vec<usize> = hidden.iter().enumerate().filter(|(_, h)| !**h).map(|(i, _)| i).collect();
            Ok(json!({
                "levels": levels,
                "shown": shown,
                "collapsed": s.view.outline.view().collapsed,
                "showLevel": s.view.outline.show_level,
                "firstLineOnly": s.view.outline.first_line_only,
                "showFormatting": !s.view.outline.hide_formatting,
            }))
        })
        .pure(),
    ]
}

/// What "Collapsed by default" headings hide in the body (#1745), by top-level block: nothing
/// in Outline view, which collapses headings its own way (as the layout does).
pub(crate) fn folded(s: &Session) -> Vec<bool> {
    if s.view.mode == ViewMode::Outline && !s.view.read_mode {
        return Vec::new();
    }
    ol::folded(&s.doc)
}

/// Whether body block `i` is a heading its stored flag collapses.
pub(crate) fn folds(s: &Session, i: usize) -> bool {
    let Some(Block::Para(p)) = s.doc.body.get(i).map(|b| &**b) else { return false };
    let rp = s.doc.styles.resolve_para(&p.props);
    rp.collapsed && rp.outline_level.is_some()
}

/// Collapse or expand body heading `i` through its stored flag, with as little direct formatting
/// as it takes: none when its style already says so, `false` over a style that collapses.
fn set_fold(s: &mut Session, i: usize, on: bool) -> Result<(), CmdError> {
    let Some(Block::Para(p)) = s.doc.body.get(i).map(|b| &**b) else { return Ok(()) };
    let mut inherited = p.props.clone();
    inherited.collapsed = None;
    let style_on = s.doc.styles.resolve_para(&inherited).collapsed;
    let want = if on { (!style_on).then_some(true) } else { style_on.then_some(false) };
    if p.props.collapsed != want {
        let para = s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(i))?;
        para.props.collapsed = want;
        para.touch();
    }
    Ok(())
}

/// Expand body heading `i` when it is collapsed (Enter in a collapsed heading).
pub(crate) fn expand_heading(s: &mut Session, i: usize) {
    if folds(s, i) {
        let _ = set_fold(s, i, false);
    }
}

/// Expand (`Some(false)`), collapse or toggle (`None`) the headings [`targets`] names.
fn fold_heading(s: &mut Session, v: &Value, on: Option<bool>) -> CmdResult {
    let targets = targets(s, v);
    if targets.is_empty() {
        return Err(CmdError::Failed("no heading to expand or collapse".into()));
    }
    for h in targets {
        let on = on.unwrap_or_else(|| !folds(s, h));
        set_fold(s, h, on)?;
    }
    keep_caret_shown(s);
    sel_result(s)
}

/// Expand every heading in the body, or collapse every one with something under it.
fn fold_all(s: &mut Session, on: bool) -> CmdResult {
    let levels = ol::levels(&s.doc);
    for (i, l) in levels.iter().enumerate() {
        if *l < BODY && (!on || ol::has_children(&levels, i)) {
            set_fold(s, i, on)?;
        }
    }
    keep_caret_shown(s);
    sel_result(s)
}

/// After a fold, a selection it hides (either end) becomes a caret on the heading (#1745).
pub(crate) fn keep_caret_shown(s: &mut Session) {
    let (focus, anchor) = (s.sel.focus.clone(), s.sel.anchor.clone());
    let to = super::caret::shown_pos(s, &focus, focus.clone(), false);
    if to != focus || super::caret::shown_pos(s, &anchor, anchor.clone(), false) != anchor {
        s.sel = Selection::caret(to);
    }
}

/// The top-level body block `p` is in, when that block is hidden by a collapsed heading.
fn hidden_block(folded: &[bool], p: &Pos) -> Option<usize> {
    let top = (p.story == StoryRef::Body).then(|| p.path.0.first().map(|b| *b as usize)).flatten()?;
    folded.get(top).copied().unwrap_or(false).then_some(top)
}

/// Expand every collapsed heading whose fold hides body block `top` (nested folds too), so it
/// shows. Returns whether anything changed.
pub(crate) fn show_block(s: &mut Session, top: usize) -> bool {
    let levels = ol::levels(&s.doc);
    let mut changed = false;
    for i in (0..top.min(levels.len())).rev() {
        if folds(s, i) && ol::subtree_end(&levels, i) > top {
            changed |= set_fold(s, i, false).is_ok();
        }
    }
    changed
}

/// Whether either end of the selection is in text a collapsed heading hides.
pub(crate) fn selection_hidden(s: &Session) -> bool {
    let folded = folded(s);
    !folded.is_empty() && (hidden_block(&folded, &s.sel.focus).is_some() || hidden_block(&folded, &s.sel.anchor).is_some())
}

/// Open the folds hiding the selection's ends (Find landing in hidden text, Word for Mac; Go To,
/// `select.text`, `caret.set`, paste).
pub(crate) fn show_selection(s: &mut Session) -> bool {
    let folded = folded(s);
    let ends = [hidden_block(&folded, &s.sel.focus), hidden_block(&folded, &s.sel.anchor)];
    let mut changed = false;
    for top in ends.into_iter().flatten() {
        changed |= show_block(s, top);
    }
    changed
}

/// Before several paragraphs are inserted at `at`: a collapsed heading there opens, so they don't
/// land in its fold out of sight (#1745).
pub(crate) fn open_for_insert(s: &mut Session, at: &Pos, blocks: usize) {
    if blocks > 1 && at.story == StoryRef::Body && at.path.0.len() == 1 {
        expand_heading(s, at.path.last());
    }
}

/// The top-level body blocks the selection touches (none outside the body text or in a table).
fn selected_blocks(s: &Session) -> Option<(usize, usize)> {
    let (a, b) = s.sel.ordered();
    if a.story != StoryRef::Body || b.story != StoryRef::Body {
        return None;
    }
    let i0 = *a.path.0.first()? as usize;
    let mut i1 = *b.path.0.first()? as usize;
    // A selection ending at the very start of a paragraph doesn't touch it.
    if i1 > i0 && b.off == 0 && b.path.0.len() == 1 {
        i1 -= 1;
    }
    Some((i0, i1))
}

/// The headings a command acts on: `block` when given, else the selected headings, else the
/// heading the caret is under.
fn targets(s: &Session, v: &Value) -> Vec<usize> {
    let levels = ol::levels(&s.doc);
    let heading = |i: usize| levels.get(i).is_some_and(|l| *l < BODY);
    if let Some(b) = v.get("block").and_then(Value::as_u64) {
        let b = usize::try_from(b).unwrap_or(usize::MAX);
        return if heading(b) { vec![b] } else { Vec::new() };
    }
    let Some((i0, i1)) = selected_blocks(s) else { return Vec::new() };
    let sel: Vec<usize> = (i0..=i1.min(levels.len().saturating_sub(1))).filter(|i| heading(*i)).collect();
    if !sel.is_empty() {
        return sel;
    }
    (0..=i0.min(levels.len().saturating_sub(1))).rev().find(|i| heading(*i)).into_iter().collect()
}

fn text_hash(doc: &Document, i: usize) -> Option<u64> {
    let Some(Block::Para(p)) = doc.body.get(i).map(|b| &**b) else { return None };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.text.hash(&mut h);
    Some(h.finish())
}

fn set_collapsed(s: &mut Session, set: &BTreeSet<usize>) {
    s.view.outline.collapsed = set.iter().filter_map(|i| text_hash(&s.doc, *i).map(|h| (*i, h))).collect();
}

/// Show Level turned into collapsed headings, so single headings can then be expanded or
/// collapsed: the headings at the shown level with something under them are collapsed and every
/// level is shown.
fn settle_show_level(s: &mut Session) -> BTreeSet<usize> {
    let mut set = s.view.outline.view().collapsed;
    let n = s.view.outline.show_level;
    if (1..=9).contains(&n) {
        let levels = ol::levels(&s.doc);
        let hidden = ol::hidden(&levels, &s.view.outline.view());
        for (i, l) in levels.iter().enumerate() {
            if *l < BODY && *l <= n && !hidden.get(i).copied().unwrap_or(true) && ol::has_children(&levels, i) {
                // A heading whose next shown paragraph is deeper keeps that open.
                let deeper_shown = (i + 1..ol::subtree_end(&levels, i)).any(|j| !hidden.get(j).copied().unwrap_or(true));
                if !deeper_shown {
                    set.insert(i);
                }
            }
        }
        s.view.outline.show_level = 0;
    }
    set
}

/// Expand: a collapsed heading opens (what was collapsed under it stays so); an open one shows
/// the shallowest collapsed level under it. `all`: the heading and everything under it opens.
fn expand(s: &mut Session, v: &Value, all: bool) -> CmdResult {
    let targets = targets(s, v);
    let mut set = settle_show_level(s);
    let levels = ol::levels(&s.doc);
    for h in targets {
        let end = ol::subtree_end(&levels, h);
        if all {
            set.retain(|i| *i < h || *i >= end);
            continue;
        }
        if set.remove(&h) {
            continue;
        }
        let hidden = ol::hidden(&levels, &OutlineView { collapsed: set.clone(), ..Default::default() });
        let open: Vec<usize> = (h + 1..end).filter(|j| set.contains(j) && !hidden.get(*j).copied().unwrap_or(true)).collect();
        if let Some(top) = open.iter().filter_map(|j| levels.get(*j)).min().copied() {
            for j in open {
                if levels.get(j) == Some(&top) {
                    set.remove(&j);
                }
            }
        }
    }
    set_collapsed(s, &set);
    sel_result(s)
}

/// Collapse: the deepest open level under a heading closes first; a heading with nothing open
/// under it closes itself. `all`: the heading closes at once.
fn collapse(s: &mut Session, v: &Value, all: bool) -> CmdResult {
    let targets = targets(s, v);
    let mut set = settle_show_level(s);
    let levels = ol::levels(&s.doc);
    for h in targets {
        if !ol::has_children(&levels, h) || set.contains(&h) {
            continue;
        }
        if all {
            set.insert(h);
            continue;
        }
        let hidden = ol::hidden(&levels, &OutlineView { collapsed: set.clone(), ..Default::default() });
        let open: Vec<usize> = (h + 1..ol::subtree_end(&levels, h))
            .filter(|j| {
                levels.get(*j).is_some_and(|l| *l < BODY)
                    && ol::has_children(&levels, *j)
                    && !set.contains(j)
                    && !hidden.get(*j).copied().unwrap_or(true)
            })
            .collect();
        match open.iter().filter_map(|j| levels.get(*j)).max().copied() {
            Some(deepest) => {
                for j in open {
                    if levels.get(j) == Some(&deepest) {
                        set.insert(j);
                    }
                }
            }
            None => {
                set.insert(h);
            }
        }
    }
    set_collapsed(s, &set);
    sel_result(s)
}

fn show_level(s: &mut Session, level: u8) -> CmdResult {
    s.view.outline.show_level = level;
    s.view.outline.collapsed.clear();
    Ok(json!({"showLevel": level}))
}

/// The end of block `i` as a selection end: the end of its text, or for a table the start of what
/// follows it.
fn block_end(doc: &Document, i: usize) -> Pos {
    match doc.body.get(i).map(|b| &**b) {
        Some(Block::Para(p)) => Pos::body(i, p.text.len()),
        _ => doc.clamp(&Pos::body(i.saturating_add(1), 0)),
    }
}

fn select_subtree(s: &mut Session, v: &Value) -> CmdResult {
    let levels = ol::levels(&s.doc);
    let start = match v.get("block").and_then(Value::as_u64) {
        Some(b) => usize::try_from(b).unwrap_or(usize::MAX),
        None => selected_blocks(s).map(|(i, _)| i).ok_or_else(|| CmdError::Failed("put the caret in the body text".into()))?,
    };
    if start >= levels.len() {
        return Err(CmdError::Params("`block` is past the end of the document".into()));
    }
    let end = ol::subtree_end(&levels, start).max(start + 1);
    let anchor = s.doc.clamp(&Pos::body(start, 0));
    let focus = block_end(&s.doc, end - 1);
    s.sel = Selection { anchor, focus };
    s.goal_x = None;
    sel_result(s)
}

/// Apply Heading `level` (or body text, `None`) to the selected paragraphs.
fn set_levels(s: &mut Session, level: Option<u8>) -> CmdResult {
    let (a, b) = s.sel.ordered();
    let paths = s.doc.paths_between(&a, &b);
    if paths.is_empty() {
        return Err(CmdError::Failed("select paragraphs to change their level".into()));
    }
    for path in paths {
        super::para::set_heading(s, a.story, &path, level)?;
    }
    sel_result(s)
}

/// The headings hidden under collapsed headings in the selection, with their levels: promoting
/// or demoting a collapsed heading carries them along (Outline view only).
pub(crate) fn hidden_subheadings(s: &Session) -> Vec<(usize, u8)> {
    if s.view.mode != ViewMode::Outline || s.view.outline.collapsed.is_empty() {
        return Vec::new();
    }
    let Some((i0, i1)) = selected_blocks(s) else { return Vec::new() };
    let levels = ol::levels(&s.doc);
    let hidden = ol::hidden(&levels, &s.view.outline.view());
    let mut out = Vec::new();
    for &(h, _) in &s.view.outline.collapsed {
        if h < i0 || h > i1 {
            continue;
        }
        for j in h + 1..ol::subtree_end(&levels, h) {
            if let Some(&l) = levels.get(j).filter(|l| **l < BODY)
                && hidden.get(j).copied().unwrap_or(false)
                && !out.iter().any(|(k, _)| *k == j)
            {
                out.push((j, l));
            }
        }
    }
    out
}

/// Move Up / Move Down in Outline view: the selected paragraphs and what is hidden under them
/// pass the next shown paragraph and what is hidden under it. `None` when the selection isn't in
/// top-level body paragraphs (the ordinary move then applies).
pub(crate) fn move_shown(s: &mut Session, up: bool) -> Option<CmdResult> {
    if s.view.mode != ViewMode::Outline {
        return None;
    }
    let (a, b) = s.sel.ordered();
    if a.story != StoryRef::Body || b.story != StoryRef::Body || a.path.0.len() != 1 || b.path.0.len() != 1 {
        return None;
    }
    let (i0, i1) = selected_blocks(s)?;
    let levels = ol::levels(&s.doc);
    let hidden = ol::hidden(&levels, &s.view.outline.view());
    let is_hidden = |i: usize| hidden.get(i).copied().unwrap_or(false);
    let mut r1 = i1;
    while is_hidden(r1 + 1) {
        r1 += 1;
    }
    let len = s.doc.body.len();
    let (lo, hi, shift): (usize, usize, isize) = if up {
        let Some(v) = (0..i0).rev().find(|j| !is_hidden(*j)) else { return Some(Err(CmdError::Failed("already at the top".into()))) };
        (v, r1, -((i0 - v) as isize))
    } else {
        let w = r1 + 1;
        if w >= len {
            return Some(Err(CmdError::Failed("already at the bottom".into())));
        }
        let mut e = w;
        while is_hidden(e + 1) {
            e += 1;
        }
        (i0, e, (e - r1) as isize)
    };
    let blocks = match s.doc.container_mut(StoryRef::Body, &a.path) {
        Ok(b) => b,
        Err(e) => return Some(Err(e.into())),
    };
    let Some(slice) = blocks.get_mut(lo..=hi) else { return Some(Err(CmdError::Failed("bad range".into()))) };
    if slice.iter().any(|b| b.as_para().is_some_and(|p| p.section.is_some())) {
        return Some(Err(CmdError::Failed("paragraphs don't move across a section break".into())));
    }
    if up {
        slice.rotate_left(i0 - lo);
    } else {
        slice.rotate_right(hi - r1);
    }
    // The selection moves with the paragraphs; an end just past them stays just past them.
    let new_r1 = r1.saturating_add_signed(shift);
    let map = |p: &Pos| -> Pos {
        let i = p.path.last();
        if (i0..=r1).contains(&i) {
            Pos { path: p.path.with_last(i.saturating_add_signed(shift)), ..p.clone() }
        } else if new_r1 + 1 < len {
            Pos::body(new_r1 + 1, 0)
        } else {
            block_end(&s.doc, new_r1)
        }
    };
    let (anchor, focus) = (map(&s.sel.anchor), map(&s.sel.focus));
    s.sel = Selection { anchor: s.doc.clamp(&anchor), focus: s.doc.clamp(&focus) };
    s.goal_x = None;
    Some(sel_result(s))
}

/// After every command: collapsed headings are found again where edits moved them (by their
/// text; one whose text changed in place stays collapsed), and in Outline view a caret the
/// outline doesn't show (in collapsed text, or past a body paragraph's first line) moves on to
/// the nearest place it does, in the direction it was going.
pub(crate) fn after_command(s: &mut Session, before: &Selection) {
    if !s.view.outline.collapsed.is_empty() {
        let hashes: Vec<Option<u64>> = (0..s.doc.body.len()).map(|i| text_hash(&s.doc, i)).collect();
        let levels = ol::levels(&s.doc);
        let mut used = BTreeSet::new();
        let mut out = Vec::new();
        for (i, h) in std::mem::take(&mut s.view.outline.collapsed) {
            let free = |j: usize, used: &BTreeSet<usize>| levels.get(j).is_some_and(|l| *l < BODY) && !used.contains(&j);
            let same = |j: usize| hashes.get(j).copied().flatten() == Some(h);
            let found = if free(i, &used) && same(i) {
                Some(i)
            } else {
                (0..levels.len()).filter(|j| free(*j, &used) && same(*j)).min_by_key(|j| j.abs_diff(i)).or_else(|| free(i, &used).then_some(i))
            };
            if let Some(j) = found {
                used.insert(j);
                out.push((j, hashes.get(j).copied().flatten().unwrap_or(h)));
            }
        }
        out.sort_unstable();
        s.view.outline.collapsed = out;
    }
    if s.view.mode != ViewMode::Outline || !s.sel.is_collapsed() || s.sel.focus.story != StoryRef::Body {
        return;
    }
    if s.layout().caret(&s.sel.focus).is_some() {
        return;
    }
    let Some(&block) = s.sel.focus.path.0.first() else { return };
    let block = block as usize;
    let levels = ol::levels(&s.doc);
    let hidden = ol::hidden(&levels, &s.view.outline.view());
    let layout = s.layout();
    let shown_para = |j: usize| !hidden.get(j).copied().unwrap_or(true) && matches!(s.doc.body.get(j).map(|b| &**b), Some(Block::Para(_)));
    let forward = before.focus <= s.sel.focus;
    let next = (block + 1..levels.len()).find(|j| shown_para(*j)).map(|j| Pos::body(j, 0));
    let prev = || {
        if shown_para(block) {
            return Some(Pos::body(block, 0));
        }
        // The end of the paragraph before, if it shows (not past a first line).
        let j = (0..block).rev().find(|j| shown_para(*j))?;
        let end = block_end(&s.doc, j);
        Some(if layout.caret(&end).is_some() { end } else { Pos::body(j, 0) })
    };
    let target = if forward { next.or_else(prev) } else { prev().or(next) };
    if let Some(t) = target {
        let t = s.doc.clamp(&t);
        s.sel = Selection::caret(t);
    }
}
