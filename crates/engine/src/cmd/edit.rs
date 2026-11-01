//! Undo, clipboard, find & replace, Go To, Format Painter.

use serde_json::{Value, json};
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::{Path, Pos, StoryRef};

use super::{delete_selection, find_pattern, pos_json, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

fn align_of_str(s: &str) -> Option<wordcraft_doc::props::Align> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(wordcraft_doc::props::Align::Left),
        "center" => Some(wordcraft_doc::props::Align::Center),
        "right" | "end" => Some(wordcraft_doc::props::Align::Right),
        "justify" | "both" => Some(wordcraft_doc::props::Align::Justify),
        "distribute" => Some(wordcraft_doc::props::Align::Distribute),
        _ => None,
    }
}

/// Undo several steps at once: the Undo button's list (#701). `count` is validated here and
/// clamped to the undo depth, so hostile numbers never reach the stack.
fn undo_to(s: &mut Session, v: &Value) -> CmdResult {
    let n = p::u64(v, "count").ok_or_else(|| CmdError::Params("`count` (whole number, 1 or more) is required".into()))?;
    if n == 0 {
        return Err(CmdError::Params("`count` must be 1 or more".into()));
    }
    let n = usize::try_from(n).unwrap_or(usize::MAX).min(s.undo_depth());
    if s.step_lifts_protection(false) {
        return Err(CmdError::Disabled("edit.undoTo: use Stop Protection to lift the document's protection".into()));
    }
    let undone = s.undo_steps(n);
    let mut r = sel_result(s)?;
    if let Some(m) = r.as_object_mut() {
        m.insert("undone".into(), json!(undone));
        m.insert("depth".into(), json!(s.undo_depth()));
    }
    Ok(r)
}

/// Something the Spike can collect: text (it doesn't take empty cells).
fn has_text_sel(s: &Session) -> Option<&'static str> {
    if s.sel.is_collapsed() { Some("nothing is selected") } else { None }
}

/// Something to cut: text, or whole table cells (an empty cell's selection has no length, #1744).
fn has_sel(s: &Session) -> Option<&'static str> {
    if s.sel.is_collapsed() && s.cell_range().is_none() { Some("nothing is selected") } else { None }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("edit.undo", "Undo", "Quick Access Toolbar", |s, _| {
            if s.step_lifts_protection(false) {
                return Err(CmdError::Disabled("edit.undo: use Stop Protection to lift the document's protection".into()));
            }
            s.undo();
            sel_result(s)
        })
        .key("Mod+Z")
        .pure(),
        CommandSpec::new("edit.undoTo", "Undo Actions", "Quick Access Toolbar", undo_to)
            .params(r#"{"count": n (1 = the newest step, like edit.undo; n undoes the newest n steps at once; at most the undo depth)}"#)
            .when(|s| if s.can_undo() { None } else { Some("there is nothing to undo") })
            .pure(),
        CommandSpec::new("edit.undoHistory", "Undo History", "Quick Access Toolbar", |s, _| {
            Ok(json!({"undo": s.undo_history(), "depth": s.undo_depth(), "redo": s.redo_label()}))
        })
        .pure(),
        CommandSpec::new("edit.redo", "Redo", "Quick Access Toolbar", |s, _| {
            if s.step_lifts_protection(true) {
                return Err(CmdError::Disabled("edit.redo: use Stop Protection to lift the document's protection".into()));
            }
            s.redo();
            sel_result(s)
        })
        .key("Mod+Y / Mod+Shift+Z")
        .pure(),
        CommandSpec::new("edit.cut", "Cut", "Home › Clipboard", cut).key("Mod+X").when(has_sel),
        CommandSpec::new("edit.copy", "Copy", "Home › Clipboard", copy).key("Mod+C / Mod+Insert").pure(),
        // Word's Spike (Ctrl+F3, Ctrl+Shift+F3; ⌘ on the Mac): the AutoText entry "Spike", so Quick
        // Parts and AutoText insert it without emptying it, as in Word.
        CommandSpec::new("edit.spike", "Spike", "Editing", spike).key("Mod+F3").when(has_text_sel),
        CommandSpec::new("edit.insertSpike", "Insert Spike", "Editing", insert_spike).key("Mod+Shift+F3").when(|s| {
            if s.math.is_some() {
                Some("leave the equation first")
            } else if s.building_blocks.contains_key(SPIKE) {
                None
            } else {
                Some("the Spike is empty")
            }
        }),
        CommandSpec::new("edit.paste", "Paste", "Home › Clipboard", paste).key("Mod+V / Shift+Insert").params(r#"{"text"?: string, "html"?: string, "rtf"?: string}"#),
        CommandSpec::new("edit.pasteText", "Paste: Keep Text Only", "Home › Clipboard › Paste", paste_text)
            .params(r#"{"text"?: string}"#)
            .key("Mod+Shift+Alt+V"),
        CommandSpec::new("edit.pasteMerge", "Paste: Merge Formatting", "Home › Clipboard › Paste", paste_merge)
            .params(r#"{"text"?: string, "html"?: string, "rtf"?: string}"#),
        CommandSpec::new("edit.clipboardPane", "Clipboard Pane", "Home › Clipboard", |s, v| {
            let on = p::bool(v, "value").unwrap_or(!s.view.clipboard_pane);
            s.view.clipboard_pane = on;
            Ok(json!({"value": on, "count": s.clip_history.len()}))
        })
        .params(r#"{"value"?: bool}"#)
        .pure(),
        CommandSpec::new("edit.clipboardItems", "Clipboard Items", "Home › Clipboard › Clipboard Pane", |s, _| Ok(s.clip_history.describe())).pure(),
        CommandSpec::new("edit.pasteClipboardItem", "Paste Clipboard Item", "Home › Clipboard › Clipboard Pane", paste_clip_item)
            .params(r#"{"index": n (0 = newest)}"#)
            .when(has_clips),
        CommandSpec::new("edit.pasteAllClipboard", "Paste All", "Home › Clipboard › Clipboard Pane", paste_all_clips).when(has_clips),
        CommandSpec::new("edit.deleteClipboardItem", "Delete Clipboard Item", "Home › Clipboard › Clipboard Pane", |s, v| {
            let i = clip_index(s, v)?;
            s.clip_history.items.remove(i);
            Ok(s.clip_history.describe())
        })
        .params(r#"{"index": n (0 = newest)}"#)
        .when(has_clips)
        .pure(),
        CommandSpec::new("edit.clearClipboard", "Clear All", "Home › Clipboard › Clipboard Pane", |s, _| {
            s.clip_history.items.clear();
            Ok(s.clip_history.describe())
        })
        .pure(),
        CommandSpec::new("edit.find", "Find", "Home › Editing", find)
            .key("Mod+F")
            .params(r#"{"text": string, "matchCase"?: bool, "wholeWord"?: bool, "matchPrefix"?: bool, "matchSuffix"?: bool, "ignorePunct"?: bool, "ignoreSpace"?: bool, "wildcards"?: bool, "special"?: bool (^p ^t ^# … codes, default true), "regex"?: bool}"#)
            .pure(),
        CommandSpec::new("edit.advancedFind", "Advanced Find", "Home › Editing › Find", advanced_find)
            .params(r#"{"text"?: string, …options of edit.find, "in"?: "main|selection", "highlight"?: bool}"#)
            .pure(),
        CommandSpec::new("edit.findNext", "Find Next", "Home › Editing › Find", |s, v| {
            read_opts(s, v);
            step(s, 1)
        })
        .key("Mod+G / F3 / Shift+F4 / Mod+PageDown")
        .params(r#"{"text"?: string, "matchCase"?: bool, "wholeWord"?: bool, "regex"?: bool}"#)
        .pure(),
        CommandSpec::new("edit.findPrevious", "Find Previous", "Home › Editing › Find", |s, v| {
            read_opts(s, v);
            step(s, -1)
        })
        // Elsewhere Ctrl+Shift+G is Word Count; Ctrl+Page Up/Down browse the find results.
        .key(super::mac_or("Mod+Shift+G / Mod+PageUp", "Mod+PageUp"))
        .params(r#"{"text"?: string, "matchCase"?: bool, "wholeWord"?: bool, "regex"?: bool}"#)
        .pure(),
        CommandSpec::new("edit.replace", "Replace", "Home › Editing", replace).params(r#"{"text": string, "with": string, …options of edit.replaceAll}"#),
        // Ctrl+H, Home › Replace: the Find and Replace dialog on its Replace tab (#712). Opening
        // it changes nothing, so it is its own command and never marks the document edited.
        CommandSpec::new("edit.findReplace", "Find and Replace", "Home › Editing", |s, v| {
            let tab = match p::str(v, "tab").unwrap_or("replace") {
                t @ ("find" | "replace" | "goto") => t,
                other => return Err(CmdError::Params(format!("`tab` is find, replace or goto, not `{other}`"))),
            };
            s.push_ui_request(json!({"open": tab}));
            Ok(json!({"open": tab}))
        })
        // Word for Mac's own key is ⌃H.
        .key(super::mac_or("Mod+H / Ctrl+H", "Mod+H"))
        .params(r#"{"tab"?: "find|replace|goto" (default replace)}"#)
        .pure(),
        CommandSpec::new("edit.replaceAll", "Replace All", "Home › Editing › Replace", replace_all)
            .params(r#"{"text": string, "with": string (^p ^t ^& ^c …; \\1 with wildcards; with regex also $1 ${n} ${name} $0 $$ \\1 \\n (paragraph mark) \\t), …options of edit.find}"#),
        CommandSpec::new("edit.goto", "Go To", "Home › Editing › Find", goto)
            .key("Mod+Alt+G / F5")
            .params(r#"{"page"?: n, "pageBy"?: int (pages forward, negative back, from the caret's page), "bookmark"?: string (selects its text), "paragraph"?: n}; none opens Find and Replace on its Go To tab"#)
            .pure(),
        CommandSpec::new("edit.goBack", "Go Back", "Navigation", |s, _| {
            let p = s.go_back().ok_or_else(|| CmdError::Failed("no edits to go back to yet".into()))?;
            s.sel = Selection::caret(p);
            s.goal_x = None;
            sel_result(s)
        })
        .key("Shift+F5 / Mod+Alt+Z")
        .pure(),
        CommandSpec::new("edit.formatPainter", "Format Painter", "Home › Clipboard", painter).params(r#"{"sticky"?: bool}"#).pure(),
        CommandSpec::new("edit.copyFormat", "Copy Formatting", "Home › Clipboard", |s, _| {
            painter_pick(s);
            sel_result(s)
        })
        .key("Mod+Shift+C")
        .pure(),
        CommandSpec::new("edit.pasteFormat", "Paste Formatting", "Home › Clipboard", paste_format).key("Mod+Shift+V"),
    ]
}

fn copy(s: &mut Session, _: &Value) -> CmdResult {
    let whole = super::table::selected_table(s);
    copy_with(s, whole.as_ref())
}

/// Copy, `whole` being the table the selection covers whole, if it does.
fn copy_with(s: &mut Session, whole: Option<&Path>) -> CmdResult {
    let (a, b) = s.sel.ordered();
    // Whole table cells selected copy as a table, even empty ones (their selection has no
    // length): the whole table, or a table of just the selected rows or cells (#1744).
    let table = match (whole, s.cell_range()) {
        (Some(tp), _) => s.doc.copy_block(a.story, tp),
        (None, Some(r)) => s.doc.copy_table_cells(r.story, &r.table, r.rows, r.cols),
        (None, None) => None,
    };
    let f = match table {
        Some(f) => f,
        None if a == b => return Ok(json!({"text": ""})),
        None => s.doc.copy_range(&a, &b),
    };
    s.clipboard_text = f.clipboard_text();
    s.clip_history.push(&f, &s.clipboard_text);
    s.clipboard = Some(f);
    Ok(copy_result(s))
}

/// What Copy and Cut return: the clipboard's text, and the copy as HTML (equations as MathML,
/// #816) for the host to put on the system clipboard too, so other programs keep the formatting;
/// and for a copy of one equation alone, its MathML (#1297).
pub(crate) fn copy_result(s: &mut Session) -> Value {
    s.clipboard_html = match &s.clipboard {
        Some(f) => wordcraft_formats::html::export_fragment(&f.to_document()),
        None => String::new(),
    };
    let mut r = json!({"text": s.clipboard_text});
    if !s.clipboard_html.is_empty() {
        r["html"] = json!(s.clipboard_html);
    }
    // A copy of one equation alone, as MathML for the clipboard formats equation editors read
    // ("MathML" on Windows, `public.mathml.presentation` on macOS; #1297). Word looks for them
    // before anything else when it pastes; a copy with text around the equation doesn't offer
    // them, so pasting it keeps the text.
    if let Some((linear, display)) = s.clipboard.as_ref().and_then(|f| f.sole_equation()) {
        r["mathml"] = json!(wordcraft_formats::mathml::linear_to_mathml(linear, display));
    }
    r
}

fn cut(s: &mut Session, _: &Value) -> CmdResult {
    let whole = super::table::selected_table(s);
    let r = copy_with(s, whole.as_ref())?;
    // A whole table selected: Cut removes the table, as Delete Table does (#1744). With Track
    // Changes on, the cut below marks its text deleted (a tracked cut may become a move).
    if !s.doc.settings.track_changes
        && let Some(tp) = &whole
    {
        super::table::delete_table_at(s, tp)?;
        super::controls::restore_placeholder(s)?;
        return Ok(r);
    }
    // Whole rows or some cells: their text goes, the rows and cells stay, as in Word.
    if whole.is_none()
        && let Some(range) = s.cell_range().cloned()
    {
        super::table::clear_range_cells(s, &range)?;
        super::controls::restore_placeholder(s)?;
        return Ok(r);
    }
    // A tracked cut of a paragraph or sentence may become a move when pasted.
    if !super::moves::cut_tracked(s)? {
        delete_selection(s)?;
    }
    super::controls::restore_placeholder(s)?;
    Ok(r)
}

/// The AutoText entry the Spike collects into.
const SPIKE: &str = "Spike";

/// Ctrl+F3: cut the selection to the end of the Spike. The clipboard is left alone.
fn spike(s: &mut Session, _: &Value) -> CmdResult {
    let (a, b) = s.sel.ordered();
    // Column selection collapses the selection before the command runs.
    if a == b {
        return Err(CmdError::Disabled("edit.spike: nothing is selected".into()));
    }
    let piece = s.doc.copy_range(&a, &b);
    if piece.blocks.is_empty() {
        return Err(CmdError::Disabled("edit.spike: this selection can't be collected".into()));
    }
    delete_selection(s)?;
    super::controls::restore_placeholder(s)?;
    s.building_blocks.entry(SPIKE.to_string()).or_default().append_piece(piece);
    sel_result(s)
}

/// Ctrl+Shift+F3: insert everything the Spike collected, in order, and empty it.
fn insert_spike(s: &mut Session, _: &Value) -> CmdResult {
    let frag = s.building_blocks.get(SPIKE).cloned().ok_or_else(|| CmdError::Disabled("edit.insertSpike: the Spike is empty".into()))?;
    super::paste::insert(s, frag)?;
    s.building_blocks.remove(SPIKE);
    sel_result(s)
}

/// Paste keeps the source formatting: our own copy while the system clipboard still holds it,
/// else another program's HTML or RTF (#687). The system clipboard's text has none, so it pastes
/// as Keep Text Only does.
fn paste(s: &mut Session, v: &Value) -> CmdResult {
    match super::paste::paste_content(s, v)? {
        super::paste::Pasted::Fragment(frag) => super::paste::insert(s, frag)?,
        super::paste::Pasted::Text(text) => super::paste::insert_text(s, &text)?,
    }
    sel_result(s)
}

/// Merge Formatting: the pasted text takes the destination's formatting, keeping its emphasis.
fn paste_merge(s: &mut Session, v: &Value) -> CmdResult {
    let mut frag = super::paste::paste_fragment(s, v)?;
    super::paste::merge_formatting(s, &mut frag);
    super::paste::insert(s, frag)?;
    sel_result(s)
}

/// Most items the Clipboard pane keeps.
pub const CLIP_MAX_ITEMS: usize = 24;
/// A copy bigger than this (estimated) is not collected (it still reaches the clipboard).
const CLIP_ITEM_BYTES: usize = 4 << 20;
/// All collected items together stay under this; the oldest go first.
const CLIP_TOTAL_BYTES: usize = 24 << 20;
/// Characters of preview per item.
const CLIP_PREVIEW_CHARS: usize = 120;

/// One item collected by Copy or Cut.
#[derive(Clone, Debug)]
pub struct ClipItem {
    pub fragment: Fragment,
    /// Its plain text.
    pub text: String,
    /// Rough memory use.
    bytes: usize,
}

impl ClipItem {
    /// One line of the item's text for a list: whitespace runs become one space, objects drop out.
    pub fn preview(&self) -> String {
        let words = self.text.split(|c: char| c.is_whitespace() || c == wordcraft_doc::para::OBJ).filter(|w| !w.is_empty());
        let mut out = String::new();
        let mut n = 0;
        for w in words {
            if !out.is_empty() {
                out.push(' ');
                n += 1;
            }
            for c in w.chars() {
                if n >= CLIP_PREVIEW_CHARS {
                    out.push('…');
                    return out;
                }
                out.push(c);
                n += 1;
            }
        }
        out
    }
}

/// Items collected by Copy and Cut during the session, newest first: the Clipboard pane
/// (Home › Clipboard). At most [`CLIP_MAX_ITEMS`]; oversized copies are skipped.
#[derive(Clone, Debug, Default)]
pub struct ClipHistory {
    items: Vec<ClipItem>,
}

impl ClipHistory {
    pub fn items(&self) -> &[ClipItem] {
        &self.items
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn push(&mut self, f: &Fragment, text: &str) {
        // Text plus per-paragraph overhead; pictures are references into the document.
        let paras = text.matches('\n').count().saturating_add(f.blocks.len()).saturating_add(f.parts.len()).saturating_add(1);
        let bytes = text.len().saturating_mul(2).saturating_add(paras.saturating_mul(256));
        if bytes > CLIP_ITEM_BYTES || f.blocks.is_empty() {
            return;
        }
        // Copying the same thing again doesn't add a second item.
        if self.items.first().is_some_and(|i| i.text == text && i.fragment == *f) {
            return;
        }
        self.items.insert(0, ClipItem { fragment: f.clone(), text: text.to_string(), bytes });
        self.items.truncate(CLIP_MAX_ITEMS);
        while self.items.len() > 1 && self.items.iter().map(|i| i.bytes).sum::<usize>() > CLIP_TOTAL_BYTES {
            self.items.pop();
        }
    }

    /// The items for agents and the pane, newest first.
    fn describe(&self) -> Value {
        Value::Array(
            self.items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    json!({
                        "index": i,
                        "preview": it.preview(),
                        "chars": it.text.chars().count(),
                        "objects": it.text.matches(wordcraft_doc::para::OBJ).count(),
                    })
                })
                .collect(),
        )
    }
}

fn has_clips(s: &Session) -> Option<&'static str> {
    if s.clip_history.is_empty() { Some("the Clipboard pane is empty") } else { None }
}

fn clip_index(s: &Session, v: &Value) -> Result<usize, CmdError> {
    let i = p::u64(v, "index").ok_or_else(|| CmdError::Params("`index` (number, 0 = newest) is required".into()))?;
    usize::try_from(i)
        .ok()
        .filter(|i| *i < s.clip_history.len())
        .ok_or_else(|| CmdError::Params(format!("no clipboard item {i} (there are {})", s.clip_history.len())))
}

/// Paste a collected item through the normal paste path. The clipboard itself stays as it was.
fn paste_clip(s: &mut Session, f: Fragment) -> CmdResult {
    let saved = s.clipboard.replace(f);
    let r = paste(s, &json!({}));
    s.clipboard = saved;
    r
}

fn paste_clip_item(s: &mut Session, v: &Value) -> CmdResult {
    let i = clip_index(s, v)?;
    let f = s.clip_history.items.get(i).map(|x| x.fragment.clone()).ok_or_else(|| CmdError::Params("no such clipboard item".into()))?;
    paste_clip(s, f)
}

/// Every collected item, in the order they were copied (one undo step).
fn paste_all_clips(s: &mut Session, _: &Value) -> CmdResult {
    let all: Vec<Fragment> = s.clip_history.items.iter().rev().map(|x| x.fragment.clone()).collect();
    let mut r = sel_result(s)?;
    for f in all {
        r = paste_clip(s, f)?;
    }
    Ok(r)
}

fn paste_text(s: &mut Session, v: &Value) -> CmdResult {
    let text = match p::str(v, "text") {
        Some(t) => t.to_string(),
        None => s.clipboard.as_ref().map(Fragment::plain_text).unwrap_or_default(),
    };
    if text.is_empty() {
        return Err(CmdError::Failed("the clipboard is empty".into()));
    }
    super::paste_text(s, &text.replace("\r\n", "\r").replace('\n', "\r"))?;
    sel_result(s)
}

/// One match: its range and, when asked for, the Replace with text filled in for it.
pub(crate) struct Found {
    pub a: Pos,
    pub b: Pos,
    pub rep: Option<String>,
}

/// A stretch of searched text: `len` bytes from `hay` are paragraph `para`'s bytes from `raw`.
/// A paragraph mark is a one-byte stretch at the paragraph's end (`mark`).
struct Seg {
    hay: usize,
    para: usize,
    raw: usize,
    len: usize,
    mark: bool,
}

/// All matches of the find state in the current story. Text marked deleted by
/// Track Changes is no longer part of the document, so the search reads each
/// paragraph without it: a match lies within one stretch of live text, and word
/// boundaries and anchors see the neighbouring live text, not the deleted text.
/// Otherwise Replace would find its own deleted text again.
pub(crate) fn search(s: &Session, story: StoryRef) -> Result<Vec<(Pos, Pos)>, CmdError> {
    Ok(search_full(s, story, false)?.into_iter().map(|f| (f.a, f.b)).collect())
}

fn find_opts(f: &crate::session::FindState) -> find_pattern::Opts {
    find_pattern::Opts {
        match_case: f.match_case,
        whole_word: f.whole_word,
        prefix: f.match_prefix,
        suffix: f.match_suffix,
        ignore_punct: f.ignore_punct,
        ignore_space: f.ignore_space,
        wildcards: f.wildcards,
        regex: f.regex,
        literal: f.literal,
    }
}

fn search_format_only(s: &Session, story: StoryRef, with_rep: bool) -> Vec<Found> {
    let f = &s.find;
    let paths = s.doc.para_paths(story);
    let mut out = Vec::new();
    for path in paths {
        let Some(p) = s.doc.para(story, &path) else { continue };
        if let Some(st) = &f.find_style {
            let is_char = s.doc.styles.find(st).is_some_and(|t| t.kind == wordcraft_doc::StyleKind::Character);
            if !is_char && !para_matches_style(&s.doc.styles, p, st) {
                continue;
            }
        }
        if let Some(al) = &f.find_align {
            let cur_align = p.props.align.unwrap_or(wordcraft_doc::props::Align::Left);
            if cur_align != *al {
                continue;
            }
        }
        if let Some(pp) = &f.find_para
            && !para_props_match(p, pp)
        {
            continue;
        }
        for (r, ch) in p.run_ranges() {
            let Some(t) = p.text.get(r.clone()).filter(|_| ch.del.is_none()) else { continue };
            if t.is_empty() {
                continue;
            }
            if let Some(target) = &f.find_format
                && !char_props_match(ch, target)
            {
                continue;
            }
            if let Some(st) = &f.find_style {
                let is_char = s.doc.styles.find(st).is_some_and(|t| t.kind == wordcraft_doc::StyleKind::Character);
                if is_char && !run_matches_style(&s.doc.styles, ch, st) {
                    continue;
                }
            }
            let a = Pos { story, path: path.clone(), off: r.start };
            let b = Pos { story, path: path.clone(), off: r.end };
            let rep = with_rep.then(|| if f.replace.is_empty() { String::new() } else { f.replace.clone() });
            out.push(Found { a, b, rep });
            if out.len() >= 100_000 {
                return out;
            }
        }
    }
    out
}

/// [`search`], with the Replace with text of each match when `with_rep` (special codes, `^&`,
/// wildcard groups). A pattern that can match a paragraph mark (`^p`) reads runs of sibling
/// paragraphs as one text with a mark between them; otherwise each paragraph is read alone.
pub(crate) fn search_full(s: &Session, story: StoryRef, with_rep: bool) -> Result<Vec<Found>, CmdError> {
    use regex_automata::{Input, meta, util::syntax};
    let f = &s.find;
    if f.query.is_empty() {
        let has_fmt = f.find_format.is_some() || f.find_style.is_some() || f.find_align.is_some() || f.find_para.is_some();
        if has_fmt {
            return Ok(search_format_only(s, story, with_rep));
        }
        return Ok(Vec::new());
    }
    let opts = find_opts(f);
    let c = find_pattern::compile(&f.query, &opts).map_err(CmdError::Params)?;
    let re = meta::Regex::builder()
        .syntax(syntax::Config::new().case_insensitive(c.case_insensitive))
        .configure(meta::Config::new().nfa_size_limit(Some(1 << 20)))
        .build(&c.pattern)
        .map_err(|e| CmdError::Params(format!("bad pattern: {e}")))?;
    let pieces = with_rep.then(|| {
        let index = |name: &str| re.group_info().to_index(regex_automata::PatternID::ZERO, name);
        find_pattern::resolve_names(find_pattern::parse_replace(&f.replace, &opts), &index)
    });
    let paths = s.doc.para_paths(story);
    // Paragraphs read as one text: consecutive siblings when the pattern spans paragraphs.
    let mut chunks: Vec<std::ops::Range<usize>> = Vec::new();
    for i in 0..paths.len() {
        let joins =
            c.spans_paras && i > 0 && paths.get(i - 1).zip(paths.get(i)).is_some_and(|(x, y)| x.parent() == y.parent() && x.last() + 1 == y.last());
        match chunks.last_mut() {
            Some(r) if joins => r.end = i + 1,
            _ => chunks.push(i..i + 1),
        }
    }
    let mut out = Vec::new();
    let mut caps = re.create_captures();
    for chunk in chunks {
        let mut hay = String::new();
        let mut segs: Vec<Seg> = Vec::new();
        // Where the live text breaks (tracked-deleted text, or a paragraph edge when not spanning).
        let mut cuts: Vec<usize> = Vec::new();
        let last = chunk.end.saturating_sub(1);
        for i in chunk {
            let Some(p) = paths.get(i).and_then(|path| s.doc.para(story, path)) else {
                cuts.push(hay.len());
                continue;
            };
            let mut expect = 0;
            for (r, ch) in p.run_ranges() {
                let Some(t) = p.text.get(r.clone()).filter(|_| ch.del.is_none()) else { continue };
                if t.is_empty() {
                    continue;
                }
                if r.start != expect {
                    cuts.push(hay.len());
                }
                match segs.last_mut() {
                    Some(sg) if !sg.mark && sg.para == i && sg.raw + sg.len == r.start => sg.len += t.len(),
                    _ => segs.push(Seg { hay: hay.len(), para: i, raw: r.start, len: t.len(), mark: false }),
                }
                hay.push_str(t);
                expect = r.end;
            }
            if i < last {
                if expect != p.text.len() {
                    cuts.push(hay.len());
                }
                segs.push(Seg { hay: hay.len(), para: i, raw: p.text.len(), len: 1, mark: true });
                hay.push(if p.section.is_some() { find_pattern::SECT_MARK } else { find_pattern::PARA_MARK });
            }
        }
        let pos_of = |x: usize, end: bool| -> Option<Pos> {
            let k = segs.partition_point(|sg| if end { sg.hay + sg.len < x } else { sg.hay + sg.len <= x });
            let sg = segs.get(k)?;
            if (end && sg.hay >= x) || (!end && sg.hay > x) {
                return None;
            }
            if sg.mark && x > sg.hay {
                let path = paths.get(sg.para)?;
                return Some(Pos { story, path: path.with_last(path.last() + 1), off: 0 });
            }
            Some(Pos { story, path: paths.get(sg.para)?.clone(), off: sg.raw + (x - sg.hay) })
        };
        cuts.push(hay.len());
        let mut from = 0;
        for cut in cuts {
            if cut <= from {
                continue;
            }
            let mut input = Input::new(&hay).range(from..cut);
            from = cut;
            loop {
                re.search_captures(&input, &mut caps);
                let Some(m) = caps.get_match() else { break };
                // Step past an empty match so the search moves on.
                let next = if m.is_empty() {
                    hay.get(m.end()..).and_then(|t| t.chars().next()).map_or(m.end() + 1, |c| m.end() + c.len_utf8())
                } else {
                    m.end()
                };
                if next > input.end() {
                    break;
                }
                input.set_start(next);
                if m.is_empty() {
                    continue;
                }
                let (Some(a), Some(b)) = (pos_of(m.start(), false), pos_of(m.end(), true)) else { continue };
                if let Some(fmt) = &f.find_format
                    && !matches_format(&s.doc, &a, &b, fmt)
                {
                    continue;
                }
                if let Some(st) = &f.find_style {
                    let p = s.doc.para_at(&a);
                    let is_char = s.doc.styles.find(st).is_some_and(|t| t.kind == wordcraft_doc::StyleKind::Character);
                    if is_char {
                        let ch_owned = p.map(|p| p.props_of_char(a.off).clone()).unwrap_or_default();
                        if !run_matches_style(&s.doc.styles, &ch_owned, st) {
                            continue;
                        }
                    } else {
                        let Some(p) = p else { continue };
                        if !para_matches_style(&s.doc.styles, p, st) {
                            continue;
                        }
                    }
                }
                if let Some(al) = &f.find_align {
                    let p = s.doc.para_at(&a);
                    let cur_align = p.and_then(|p| p.props.align).unwrap_or(wordcraft_doc::props::Align::Left);
                    if cur_align != *al {
                        continue;
                    }
                }
                if let Some(pp) = &f.find_para {
                    let Some(p) = s.doc.para_at(&a) else { continue };
                    if !para_props_match(p, pp) {
                        continue;
                    }
                }
                let rep = pieces.as_ref().map(|pc| {
                    let group = |n: usize| caps.get_group(n).and_then(|g| hay.get(g.range())).map(str::to_string);
                    find_pattern::expand(pc, hay.get(m.range()).unwrap_or(""), &group, &s.clipboard_text)
                });
                out.push(Found { a, b, rep });
                if out.len() >= 100_000 {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

fn read_find_formats(f: &mut crate::doc_state::FindState, v: &Value) {
    if let Some(val) = v.get("findFormat").or_else(|| v.get("format")) {
        if val.is_null() {
            f.find_format = None;
        } else if let Ok(cp) = serde_json::from_value::<wordcraft_doc::props::CharProps>(val.clone()) {
            f.find_format = Some(cp);
        }
    }
    if let Some(val) = v.get("replaceFormat").or_else(|| v.get("withFormat")) {
        if val.is_null() {
            f.replace_format = None;
        } else if let Ok(cp) = serde_json::from_value::<wordcraft_doc::props::CharProps>(val.clone()) {
            f.replace_format = Some(cp);
        }
    }
    if p::bool(v, "clearFindFormat").unwrap_or(false) {
        f.find_format = None;
    }
    if p::bool(v, "clearReplaceFormat").unwrap_or(false) {
        f.replace_format = None;
    }
    if p::bool(v, "clearFindStyle").unwrap_or(false) {
        f.find_style = None;
    }
    if p::bool(v, "clearReplaceStyle").unwrap_or(false) {
        f.replace_style = None;
    }
    if p::bool(v, "clearFindAlign").unwrap_or(false) {
        f.find_align = None;
    }
    if p::bool(v, "clearReplaceAlign").unwrap_or(false) {
        f.replace_align = None;
    }
    if let Some(st) = p::str(v, "findStyle").or_else(|| p::str(v, "style")) {
        f.find_style = if st.is_empty() { None } else { Some(st.to_string()) };
    }
    if let Some(st) = p::str(v, "replaceStyle").or_else(|| p::str(v, "withStyle")) {
        f.replace_style = if st.is_empty() { None } else { Some(st.to_string()) };
    }
    if let Some(al) = p::str(v, "findAlign").or_else(|| p::str(v, "align")).and_then(align_of_str) {
        f.find_align = Some(al);
    }
    if let Some(al) = p::str(v, "replaceAlign").or_else(|| p::str(v, "withAlign")).and_then(align_of_str) {
        f.replace_align = Some(al);
    }
    if let Some(mut val) = v.get("findPara").cloned() {
        if val.is_null() {
            f.find_para = None;
        } else {
            if let Some(al_str) = val.get("align").and_then(serde_json::Value::as_str)
                && let Some(al) = align_of_str(al_str)
                && let Ok(al_val) = serde_json::to_value(al)
            {
                val["align"] = al_val;
            }
            if let Ok(pp) = serde_json::from_value::<wordcraft_doc::props::ParaProps>(val) {
                f.find_para = Some(pp);
            }
        }
    }
    if let Some(mut val) = v.get("replacePara").cloned() {
        if val.is_null() {
            f.replace_para = None;
        } else {
            if let Some(al_str) = val.get("align").and_then(serde_json::Value::as_str)
                && let Some(al) = align_of_str(al_str)
                && let Ok(al_val) = serde_json::to_value(al)
            {
                val["align"] = al_val;
            }
            if let Ok(pp) = serde_json::from_value::<wordcraft_doc::props::ParaProps>(val) {
                f.replace_para = Some(pp);
            }
        }
    }
    if p::bool(v, "clearFindPara").unwrap_or(false) {
        f.find_para = None;
    }
    if p::bool(v, "clearReplacePara").unwrap_or(false) {
        f.replace_para = None;
    }
    if p::bool(v, "clearFormatting").unwrap_or(false) {
        f.find_format = None;
        f.replace_format = None;
        f.find_style = None;
        f.replace_style = None;
        f.find_align = None;
        f.replace_align = None;
        f.find_para = None;
        f.replace_para = None;
    }
}

fn read_opts(s: &mut Session, v: &Value) {
    // A new search changes what Reading Highlight marks.
    s.find.highlights = None;
    if let Some(t) = p::str(v, "text") {
        s.find.query = t.to_string();
    } else if v.get("text").is_some_and(serde_json::Value::is_null)
        || ((v.get("findStyle").is_some() || v.get("findFormat").is_some() || v.get("findPara").is_some() || v.get("findAlign").is_some())
            && v.get("text").is_none())
    {
        s.find.query.clear();
    }
    if let Some(b) = p::bool(v, "matchCase") {
        s.find.match_case = b;
    }
    if let Some(b) = p::bool(v, "wholeWord") {
        s.find.whole_word = b;
    }
    if let Some(b) = p::bool(v, "regex") {
        s.find.regex = b;
    }
    if let Some(b) = p::bool(v, "wildcards") {
        s.find.wildcards = b;
    }
    if let Some(b) = p::bool(v, "special") {
        s.find.literal = !b;
    }
    if let Some(b) = p::bool(v, "matchPrefix") {
        s.find.match_prefix = b;
    }
    if let Some(b) = p::bool(v, "matchSuffix") {
        s.find.match_suffix = b;
    }
    if let Some(b) = p::bool(v, "ignorePunct") {
        s.find.ignore_punct = b;
    }
    if let Some(b) = p::bool(v, "ignoreSpace") {
        s.find.ignore_space = b;
    }
    if let Some(t) = p::str(v, "with") {
        s.find.replace = t.to_string();
    }
    read_find_formats(&mut s.find, v);
}

fn find(s: &mut Session, v: &Value) -> CmdResult {
    read_opts(s, v);
    let has_fmt = s.find.find_format.is_some() || s.find.find_style.is_some() || s.find.find_align.is_some() || s.find.find_para.is_some();
    if p::str(v, "text").is_none() && !has_fmt {
        // Word's Ctrl+F: the Navigation pane with its search box ready to type in (#712); the
        // Find and Replace dialog is Advanced Find.
        s.view.nav_pane = true;
        s.push_ui_request(json!({"focus": "navSearch"}));
        return Ok(json!({"count": s.find.results.len()}));
    }
    let story = s.sel.focus.story;
    s.find.results = search(s, story)?;
    // Select the first match after the caret.
    let caret = s.sel.focus.clone();
    let idx = s.find.results.iter().position(|(a, _)| *a >= caret).unwrap_or(0);
    s.find.current = idx;
    if let Some((a, b)) = s.find.results.get(idx).cloned() {
        s.sel = Selection { anchor: a, focus: b };
    }
    Ok(json!({
        "count": s.find.results.len(),
        "matches": s.find.results.iter().take(200).map(|(a, b)| json!({"start": pos_json(a), "end": pos_json(b)})).collect::<Vec<_>>(),
    }))
}

/// Find in the classic dialog: every match in the body or the selection, optionally highlighted while you read.
fn advanced_find(s: &mut Session, v: &Value) -> CmdResult {
    read_opts(s, v);
    let has_fmt = s.find.find_format.is_some() || s.find.find_style.is_some() || s.find.find_align.is_some() || s.find.find_para.is_some();
    if p::str(v, "text").is_none() && v.get("highlight").is_none() && !has_fmt {
        s.push_ui_request(json!({"open": "find"}));
        return Ok(json!({"count": s.find.results.len()}));
    }
    let within = match p::str(v, "in").unwrap_or("main") {
        "main" => None,
        "selection" if !s.sel.is_collapsed() => Some(s.sel.ordered()),
        "selection" => return Err(CmdError::Params("`in: selection` needs a selection".into())),
        other => return Err(CmdError::Params(format!("`in` is main or selection, not `{other}`"))),
    };
    let story = within.as_ref().map(|(a, _)| a.story).unwrap_or(StoryRef::Body);
    let mut found = search(s, story)?;
    if let Some((a, b)) = &within {
        found.retain(|(x, y)| x >= a && y <= b);
    }
    if let Some(h) = p::bool(v, "highlight") {
        s.find.highlight = h;
        s.find.highlights = None;
    }
    let n = found.len();
    let matches: Vec<Value> = found.iter().take(200).map(|(a, b)| json!({"start": pos_json(a), "end": pos_json(b)})).collect();
    s.find.current = 0;
    s.find.results = found;
    Ok(json!({"count": n, "matches": matches, "highlight": s.find.highlight}))
}

fn step(s: &mut Session, dir: i64) -> CmdResult {
    let story = s.sel.focus.story;
    s.find.results = search(s, story)?;
    let n = s.find.results.len();
    if n == 0 {
        return Err(CmdError::Failed("no matches".into()));
    }
    let caret = if dir > 0 { s.sel.ordered().1 } else { s.sel.ordered().0 };
    let idx = if dir > 0 {
        s.find.results.iter().position(|(a, _)| *a >= caret).unwrap_or(0)
    } else {
        s.find.results.iter().rposition(|(_, b)| *b <= caret).unwrap_or(n - 1)
    };
    s.find.current = idx;
    if let Some((a, b)) = s.find.results.get(idx).cloned() {
        s.sel = Selection { anchor: a, focus: b };
    }
    Ok(json!({"index": idx, "count": n}))
}

/// Replace needs something to look for: the `text` given, or the last search's (Find then Replace
/// with just a `with`). Neither is a call that would silently replace nothing (a misspelt `text`).
fn need_query(s: &Session, v: &Value) -> Result<(), CmdError> {
    let has_fmt = s.find.find_format.is_some()
        || s.find.find_para.is_some()
        || v.get("findPara").is_some_and(|f| !f.is_null())
        || s.find.find_style.is_some()
        || s.find.find_align.is_some()
        || v.get("findFormat").is_some_and(|f| !f.is_null())
        || v.get("format").is_some_and(|f| !f.is_null())
        || v.get("findStyle").is_some_and(|f| !f.is_null())
        || v.get("style").is_some_and(|f| !f.is_null())
        || v.get("findAlign").is_some_and(|f| !f.is_null())
        || v.get("align").is_some_and(|f| !f.is_null());
    if p::str(v, "text").is_none() && s.find.query.is_empty() && !has_fmt {
        return Err(CmdError::Params("`text` (string) is required".into()));
    }
    Ok(())
}

fn para_matches_style(styles: &wordcraft_doc::StyleSheet, p: &wordcraft_doc::Paragraph, st: &str) -> bool {
    let target = styles.find(st);
    let target_id = target.map(|t| t.id.as_str()).unwrap_or(st);
    let target_name = target.map(|t| t.name.as_str()).unwrap_or(st);

    let cur_id = p.props.style.as_deref().unwrap_or("Normal");
    let cur_name = styles.get(cur_id).map(|t| t.name.as_str()).unwrap_or(cur_id);

    cur_id.eq_ignore_ascii_case(target_id)
        || cur_id.eq_ignore_ascii_case(target_name)
        || cur_name.eq_ignore_ascii_case(target_id)
        || cur_name.eq_ignore_ascii_case(target_name)
        || cur_id.replace(' ', "").eq_ignore_ascii_case(&st.replace(' ', ""))
}

fn run_matches_style(styles: &wordcraft_doc::StyleSheet, ch: &wordcraft_doc::CharProps, st: &str) -> bool {
    let Some(target) = styles.find(st) else { return false };
    if target.kind != wordcraft_doc::StyleKind::Character {
        return false;
    }
    let target_id = &target.id;
    let target_name = &target.name;
    let r_style = ch.style.as_deref().unwrap_or_default();
    r_style.eq_ignore_ascii_case(target_id) || r_style.eq_ignore_ascii_case(target_name)
}

fn apply_replace_style(s: &mut Session, a: &Pos, b: &Pos) -> Result<(), CmdError> {
    let Some(st) = s.find.replace_style.clone() else {
        return Ok(());
    };
    let target = s.doc.styles.find(&st).cloned();
    let style_id = target.as_ref().map(|t| t.id.clone()).unwrap_or_else(|| st.clone());
    let is_char_style = target.as_ref().is_some_and(|t| t.kind == wordcraft_doc::StyleKind::Character);

    if is_char_style {
        s.doc.format_range(a, b, &|cp| cp.style = Some(style_id.clone()))?;
        s.touch();
    } else {
        if let Ok(p) = s.doc.para_mut(a.story, &a.path) {
            p.props.style = Some(style_id.clone());
            if style_id.starts_with("Heading") || style_id == "Title" || style_id == "Normal" {
                if p.props.numbering.is_some_and(|n| n.num != 0) {
                    p.props.numbering = None;
                }
                let len = p.len();
                let _ = p.format(0, len, &|c| {
                    c.font = None;
                    c.size = None;
                    c.color = None;
                });
            }
            p.touch();
            s.touch();
        }
    }
    Ok(())
}

fn apply_replace_para_props(s: &mut Session, story: StoryRef, path: &Path) {
    if s.find.replace_align.is_none() && s.find.replace_para.is_none() {
        return;
    }
    if let Ok(p) = s.doc.para_mut(story, path) {
        if let Some(al) = &s.find.replace_align {
            p.props.align = Some(*al);
        }
        if let Some(rp) = &s.find.replace_para {
            p.props.overlay(rp);
        }
        p.touch();
        s.touch();
    }
}

fn replace(s: &mut Session, v: &Value) -> CmdResult {
    need_query(s, v)?;
    read_opts(s, v);
    let story = s.sel.focus.story;
    let results = search_full(s, story, true)?;
    let (a, b) = s.sel.ordered();
    // Replace the current match if selected, then move to the next.
    if let Some(f) = results.into_iter().find(|f| f.a == a && f.b == b) {
        let rep_text = f.rep.unwrap_or_default();
        let has_replace_fmt =
            s.find.replace_format.is_some() || s.find.replace_style.is_some() || s.find.replace_align.is_some() || s.find.replace_para.is_some();
        if rep_text.is_empty() && has_replace_fmt {
            if let Some(rf) = &s.find.replace_format {
                s.doc.format_range(&a, &b, &|cp| cp.overlay(rf))?;
            }
            apply_replace_style(s, &a, &b)?;
            apply_replace_para_props(s, a.story, &a.path);
        } else {
            let mut props = s.doc.para_at(&a).map(|p| p.props_of_char(a.off).clone().without_symbol()).unwrap_or_default();
            if let Some(rf) = &s.find.replace_format {
                props.overlay(rf);
            }
            if let Some(st) = &s.find.replace_style
                && let Some(target) = s.doc.styles.find(st)
                && target.kind == wordcraft_doc::StyleKind::Character
            {
                props.style = Some(target.id.clone());
            }
            let at = s.doc.delete_range(&a, &b)?;
            let end_at = s.doc.insert_text(&at, &rep_text, &props)?;
            apply_replace_style(s, &at, &end_at)?;
            apply_replace_para_props(s, at.story, &at.path);
            s.sel = crate::Selection { anchor: at, focus: end_at };
        }
    }
    let _ = step(s, 1);
    Ok(json!({"remaining": search(s, story)?.len()}))
}

/// Every match replaced, last first so earlier positions stay valid (one undo step). A `\r` in
/// the replacement (`^p`) splits the paragraph.
fn replace_all(s: &mut Session, v: &Value) -> CmdResult {
    need_query(s, v)?;
    read_opts(s, v);
    let story = s.sel.focus.story;
    let results = search_full(s, story, true)?;
    let n = results.len();
    super::controls::check_deletions(s, story, results.iter().map(|f| (&f.a, &f.b)))?;
    let has_replace_fmt =
        s.find.replace_format.is_some() || s.find.replace_style.is_some() || s.find.replace_align.is_some() || s.find.replace_para.is_some();
    for Found { a, b, rep } in results.into_iter().rev() {
        let with = rep.unwrap_or_default();
        if with.is_empty() && has_replace_fmt {
            if let Some(rf) = &s.find.replace_format {
                s.doc.format_range(&a, &b, &|cp| cp.overlay(rf))?;
            }
            apply_replace_style(s, &a, &b)?;
            apply_replace_para_props(s, a.story, &a.path);
            continue;
        }
        if s.doc.settings.track_changes {
            // Same path as a single replace, so the change is a reviewable revision.
            s.sel = Selection { anchor: a, focus: b };
            super::paste_text(s, &with)?;
            let sel_a = s.sel.anchor.clone();
            let sel_b = s.sel.focus.clone();
            apply_replace_style(s, &sel_a, &sel_b)?;
            apply_replace_para_props(s, sel_a.story, &sel_a.path);
            continue;
        }
        // Replacing a symbol gives the new text the symbol's formatting, not its own font.
        let mut props = s.doc.para_at(&a).map(|p| p.props_of_char(a.off).clone().without_symbol()).unwrap_or_default();
        if let Some(rf) = &s.find.replace_format {
            props.overlay(rf);
        }
        if let Some(st) = &s.find.replace_style
            && let Some(target) = s.doc.styles.find(st)
            && target.kind == wordcraft_doc::StyleKind::Character
        {
            props.style = Some(target.id.clone());
        }
        let mut at = s.doc.delete_range(&a, &b)?;
        let start_pos = at.clone();
        for (k, part) in with.split(['\r', '\u{2029}']).enumerate() {
            if k > 0 {
                at = s.doc.split_paragraph(&at)?;
            }
            if !part.is_empty() {
                at = s.doc.insert_text(&at, part, &props)?;
            }
        }
        let end_pos = at;
        apply_replace_style(s, &start_pos, &end_pos)?;
        apply_replace_para_props(s, start_pos.story, &start_pos.path);
    }
    s.status = format!("All done. We made {n} replacements.");
    Ok(json!({"replaced": n}))
}

fn goto(s: &mut Session, v: &Value) -> CmdResult {
    // Relative pages, Go To's "+2" / "-1": from the page the caret is on.
    let by = v.get("pageBy").and_then(Value::as_i64);
    let rel = by.map(|by| {
        let here = s.layout().caret_on(&s.sel.focus, s.page_hint).map_or(0, |c| c.page) as i64;
        (here + 1).saturating_add(by).max(1) as u64
    });
    if let Some(pg) = rel.or_else(|| p::u64(v, "page")) {
        let l = s.layout();
        let idx = crate::p::page_index(pg).min(l.pages.len().saturating_sub(1));
        let target = l.pages.get(idx).and_then(|page| {
            page.items.iter().find_map(|it| match it {
                wordcraft_layout::Placed::Lines { story, path, para, l0, .. } if *story == StoryRef::Body => {
                    para.lines.get(*l0).map(|ln| Pos { story: *story, path: path.clone(), off: ln.start })
                }
                _ => None,
            })
        });
        if let Some(t) = target {
            s.sel = Selection::caret(t);
            s.page_hint = idx;
        }
        s.push_ui_request(json!({"scrollTo": "caret"}));
        return sel_result(s);
    }
    if let Some(name) = p::str(v, "bookmark") {
        // Word's built-in bookmark: the top of the document.
        if name == "_top" {
            s.sel = Selection::caret(Pos::body(0, 0));
            return sel_result(s);
        }
        // As in Word, the bookmark's text is selected (a caret for an empty bookmark).
        let (from, to) = super::insert::bookmark_range(&s.doc, name).ok_or_else(|| CmdError::Failed(format!("no bookmark `{name}`")))?;
        s.sel = Selection { anchor: from, focus: to };
        return sel_result(s);
    }
    if let Some(n) = p::u64(v, "paragraph") {
        let paths = s.doc.para_paths(StoryRef::Body);
        if let Some(path) = paths.get((n.max(1) - 1) as usize) {
            s.sel = Selection::caret(Pos { story: StoryRef::Body, path: path.clone(), off: 0 });
        }
        return sel_result(s);
    }
    s.push_ui_request(json!({"open": "goto"}));
    sel_result(s)
}

fn painter_pick(s: &mut Session) {
    let f = s.sel.ordered().0;
    // A symbol's own font isn't formatting to paint onto text.
    let chr = s.doc.para_at(&f).map(|p| p.props_of_char(f.off).clone().without_symbol()).unwrap_or_default();
    let para = s.doc.para_at(&f).map(|p| p.props.clone()).unwrap_or_default();
    s.painter = Some((chr, para, false));
}

fn painter(s: &mut Session, v: &Value) -> CmdResult {
    if s.painter.is_some() {
        s.painter = None;
        return Ok(json!({"active": false}));
    }
    painter_pick(s);
    if let Some((_, _, sticky)) = s.painter.as_mut() {
        *sticky = p::bool(v, "sticky").unwrap_or(false);
    }
    Ok(json!({"active": true}))
}

/// Apply the format painter's formatting to the selection.
fn paste_format(s: &mut Session, _: &Value) -> CmdResult {
    let Some((chr, para, sticky)) = s.painter.clone() else { return Err(CmdError::Failed("no formatting copied".into())) };
    let (a, b) = s.sel.ordered();
    if a == b {
        // No selection: apply paragraph formatting (and word formatting like Word).
        let f = s.sel.focus.clone();
        if let Some(pp) = s.doc.para_at(&f) {
            let (x, y) = pp.word_at(f.off);
            s.doc.format_range(&Pos { off: x, ..f.clone() }, &Pos { off: y, ..f.clone() }, &|c| *c = chr.cleared().overlaid(&chr))?;
        }
        s.doc.format_paragraphs(&a, &b, &|p| *p = para.clone())?;
    } else {
        s.doc.format_range(&a, &b, &|c| {
            let link = c.link.clone();
            *c = c.cleared().overlaid(&chr);
            c.link = link;
        })?;
        if a.path != b.path || a.off == 0 {
            s.doc.format_paragraphs(&a, &b, &|p| *p = para.clone())?;
        }
    }
    if !sticky {
        s.painter = None;
    }
    sel_result(s)
}

fn char_props_effects_match(actual: &wordcraft_doc::props::CharProps, target: &wordcraft_doc::props::CharProps) -> bool {
    if let Some(ds) = target.double_strike
        && actual.double_strike.unwrap_or(false) != ds
    {
        return false;
    }
    if let Some(va) = target.vert_align
        && actual.vert_align.unwrap_or_default() != va
    {
        return false;
    }
    if let Some(sc) = target.small_caps
        && actual.small_caps.unwrap_or(false) != sc
    {
        return false;
    }
    if let Some(c) = target.caps
        && actual.caps.unwrap_or(false) != c
    {
        return false;
    }
    if let Some(h) = target.hidden
        && actual.hidden.unwrap_or(false) != h
    {
        return false;
    }
    if let Some(sp) = target.spacing
        && (actual.spacing.unwrap_or(0.0) - sp).abs() > 0.1
    {
        return false;
    }
    if let Some(pos) = target.position
        && (actual.position.unwrap_or(0.0) - pos).abs() > 0.1
    {
        return false;
    }
    if let Some(k) = target.kern
        && (actual.kern.unwrap_or(0.0) - k).abs() > 0.1
    {
        return false;
    }
    true
}

fn para_props_indents_match(actual: &wordcraft_doc::props::ParaProps, target: &wordcraft_doc::props::ParaProps) -> bool {
    if let Some(l) = target.indent_left
        && (actual.indent_left.unwrap_or(0.0) - l).abs() > 0.1
    {
        return false;
    }
    if let Some(r) = target.indent_right
        && (actual.indent_right.unwrap_or(0.0) - r).abs() > 0.1
    {
        return false;
    }
    if let Some(f) = target.indent_first
        && (actual.indent_first.unwrap_or(0.0) - f).abs() > 0.1
    {
        return false;
    }
    if let Some(b) = target.space_before
        && (actual.space_before.unwrap_or(0.0) - b).abs() > 0.1
    {
        return false;
    }
    if let Some(a) = target.space_after
        && (actual.space_after.unwrap_or(0.0) - a).abs() > 0.1
    {
        return false;
    }
    true
}

fn para_props_match(p: &wordcraft_doc::Paragraph, target: &wordcraft_doc::props::ParaProps) -> bool {
    if let Some(st) = &target.style {
        let cur = p.props.style.as_deref().unwrap_or("Normal");
        if !cur.eq_ignore_ascii_case(st) {
            return false;
        }
    }
    if let Some(al) = target.align
        && p.props.align.unwrap_or(wordcraft_doc::props::Align::Left) != al
    {
        return false;
    }
    if !para_props_indents_match(&p.props, target) {
        return false;
    }
    if let Some(ls) = target.line_spacing
        && p.props.line_spacing.unwrap_or_default() != ls
    {
        return false;
    }
    if let Some(w) = target.widow_control
        && p.props.widow_control.unwrap_or(true) != w
    {
        return false;
    }
    if let Some(kn) = target.keep_next
        && p.props.keep_next.unwrap_or(false) != kn
    {
        return false;
    }
    if let Some(kl) = target.keep_lines
        && p.props.keep_lines.unwrap_or(false) != kl
    {
        return false;
    }
    if let Some(pb) = target.page_break_before
        && p.props.page_break_before.unwrap_or(false) != pb
    {
        return false;
    }
    true
}

fn char_props_match(actual: &wordcraft_doc::props::CharProps, target: &wordcraft_doc::props::CharProps) -> bool {
    if let Some(b) = target.bold
        && actual.bold.unwrap_or(false) != b
    {
        return false;
    }
    if let Some(i) = target.italic
        && actual.italic.unwrap_or(false) != i
    {
        return false;
    }
    if let Some(u) = target.underline
        && actual.underline.unwrap_or_default() != u
    {
        return false;
    }
    if let Some(s) = target.strike
        && actual.strike.unwrap_or(false) != s
    {
        return false;
    }
    if let Some(sz) = target.size
        && (actual.size.unwrap_or(12.0) - sz).abs() > 0.1
    {
        return false;
    }
    if let Some(font) = &target.font
        && actual.font.as_deref().unwrap_or("") != font.as_str()
    {
        return false;
    }
    if let Some(color) = &target.color
        && actual.color.as_ref() != Some(color)
    {
        return false;
    }
    if let Some(hl) = &target.highlight
        && actual.highlight.as_ref() != Some(hl)
    {
        return false;
    }
    char_props_effects_match(actual, target)
}

fn matches_format(doc: &wordcraft_doc::Document, a: &Pos, b: &Pos, target: &wordcraft_doc::props::CharProps) -> bool {
    if a.story != b.story || a.path != b.path {
        let Some(p) = doc.para_at(a) else { return false };
        return char_props_match(p.props_of_char(a.off), target);
    }
    let Some(p) = doc.para_at(a) else { return false };
    if a.off == b.off {
        return char_props_match(p.props_of_char(a.off), target);
    }
    for (r, ch) in p.run_ranges() {
        if r.end <= a.off || r.start >= b.off {
            continue;
        }
        if !char_props_match(ch, target) {
            return false;
        }
    }
    true
}
