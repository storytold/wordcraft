//! Undo, clipboard, find & replace, Go To, Format Painter.

use serde_json::{Value, json};
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::{Block, Pos, StoryRef};

use super::{delete_selection, pos_json, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

fn has_sel(s: &Session) -> Option<&'static str> {
    if s.sel.is_collapsed() { Some("nothing is selected") } else { None }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("edit.undo", "Undo", "Quick Access Toolbar", |s, _| {
            s.undo();
            sel_result(s)
        })
        .key("Mod+Z")
        .pure(),
        CommandSpec::new("edit.redo", "Redo", "Quick Access Toolbar", |s, _| {
            s.redo();
            sel_result(s)
        })
        .key("Mod+Y / Mod+Shift+Z")
        .pure(),
        CommandSpec::new("edit.cut", "Cut", "Home › Clipboard", cut).key("Mod+X").when(has_sel),
        CommandSpec::new("edit.copy", "Copy", "Home › Clipboard", copy).key("Mod+C").pure(),
        CommandSpec::new("edit.paste", "Paste", "Home › Clipboard", paste).key("Mod+V").params(r#"{"text"?: string}"#),
        CommandSpec::new("edit.pasteText", "Paste: Keep Text Only", "Home › Clipboard › Paste", paste_text)
            .params(r#"{"text"?: string}"#)
            .key("Mod+Shift+Alt+V"),
        CommandSpec::new("edit.pasteMerge", "Paste: Merge Formatting", "Home › Clipboard › Paste", paste_text).params(r#"{"text"?: string}"#),
        CommandSpec::new("edit.find", "Find", "Home › Editing", find)
            .key("Mod+F")
            .params(r#"{"text": string, "matchCase"?: bool, "wholeWord"?: bool, "regex"?: bool}"#)
            .pure(),
        CommandSpec::new("edit.findNext", "Find Next", "Home › Editing › Find", |s, _| step(s, 1)).key("Mod+G / F3").pure(),
        CommandSpec::new("edit.findPrevious", "Find Previous", "Home › Editing › Find", |s, _| step(s, -1)).key("Mod+Shift+G / Shift+F3").pure(),
        CommandSpec::new("edit.replace", "Replace", "Home › Editing", replace).key("Mod+H").params(r#"{"text": string, "with": string}"#),
        CommandSpec::new("edit.replaceAll", "Replace All", "Home › Editing › Replace", replace_all)
            .params(r#"{"text": string, "with": string, "matchCase"?: bool, "wholeWord"?: bool, "regex"?: bool}"#),
        CommandSpec::new("edit.goto", "Go To", "Home › Editing › Find", goto)
            .key("Mod+Alt+G / F5")
            .params(r#"{"page"?: n, "bookmark"?: string, "paragraph"?: n}"#)
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
    let (a, b) = s.sel.ordered();
    if a == b {
        return Ok(json!({"text": ""}));
    }
    let f = s.doc.copy_range(&a, &b);
    s.clipboard_text = f.plain_text();
    s.clipboard = Some(f);
    Ok(json!({"text": s.clipboard_text}))
}

fn cut(s: &mut Session, v: &Value) -> CmdResult {
    let r = copy(s, v)?;
    delete_selection(s)?;
    Ok(r)
}

fn paste(s: &mut Session, v: &Value) -> CmdResult {
    // An explicit text param (from the system clipboard) wins unless it matches our own copy.
    let frag = match (p::str(v, "text"), &s.clipboard) {
        (Some(t), Some(f)) if f.plain_text() == t.replace("\r\n", "\n") => f.clone(),
        (Some(t), _) => Fragment::from_text(&t.replace("\r\n", "\n")),
        (None, Some(f)) => f.clone(),
        (None, None) => return Err(CmdError::Failed("the clipboard is empty".into())),
    };
    if let Some(images) = v.get("image").and_then(Value::as_str) {
        let _ = images;
    }
    let at = delete_selection(s)?;
    let mut frag = frag;
    if s.doc.settings.track_changes {
        let rid = super::new_revision(s, wordcraft_doc::RevisionKind::Insert);
        for b in &mut frag.blocks {
            if let Block::Para(p) = b {
                for r in &mut p.runs {
                    r.props.ins = Some(rid);
                }
            }
        }
    }
    let end = s.doc.insert_fragment(&at, &frag)?;
    s.sel = Selection::caret(end);
    sel_result(s)
}

fn paste_text(s: &mut Session, v: &Value) -> CmdResult {
    let text = match p::str(v, "text") {
        Some(t) => t.to_string(),
        None => s.clipboard.as_ref().map(Fragment::plain_text).unwrap_or_default(),
    };
    super::type_text(s, &text.replace("\r\n", "\r").replace('\n', "\r"))?;
    sel_result(s)
}

/// All matches of the find state in the current story. Text marked deleted by
/// Track Changes is no longer part of the document, so the search reads each
/// paragraph without it: a match lies within one stretch of live text, and word
/// boundaries and anchors see the neighbouring live text, not the deleted text.
/// Otherwise Replace would find its own deleted text again.
fn search(s: &Session, story: StoryRef) -> Result<Vec<(Pos, Pos)>, CmdError> {
    use regex_automata::{Input, meta, util::syntax};
    let f = &s.find;
    if f.query.is_empty() {
        return Ok(Vec::new());
    }
    let pat = if f.regex { f.query.clone() } else { regex::escape(&f.query) };
    let pat = if f.whole_word { format!(r"\b{pat}\b") } else { pat };
    let re = meta::Regex::builder()
        .syntax(syntax::Config::new().case_insensitive(!f.match_case))
        .configure(meta::Config::new().nfa_size_limit(Some(1 << 20)))
        .build(&pat)
        .map_err(|e| CmdError::Params(format!("bad pattern: {e}")))?;
    let mut out = Vec::new();
    for path in s.doc.para_paths(story) {
        let Some(p) = s.doc.para(story, &path) else { continue };
        // The paragraph as it reads, and its stretches of live text as
        // (start in `live`, start in the paragraph, length).
        let mut live = String::new();
        let mut spans: Vec<(usize, usize, usize)> = Vec::new();
        for (r, c) in p.run_ranges() {
            let Some(t) = p.text.get(r.clone()).filter(|_| c.del.is_none()) else { continue };
            match spans.last_mut() {
                Some((_, raw, n)) if *raw + *n == r.start => *n += t.len(),
                _ => spans.push((live.len(), r.start, t.len())),
            }
            live.push_str(t);
        }
        for (at, raw, n) in spans {
            for m in re.find_iter(Input::new(&live).range(at..at + n)) {
                if m.is_empty() {
                    continue;
                }
                let (a, b) = (raw + m.start().saturating_sub(at), raw + m.end().saturating_sub(at));
                out.push((Pos { story, path: path.clone(), off: a }, Pos { story, path: path.clone(), off: b }));
                if out.len() >= 100_000 {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

fn read_opts(s: &mut Session, v: &Value) {
    if let Some(t) = p::str(v, "text") {
        s.find.query = t.to_string();
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
    if let Some(t) = p::str(v, "with") {
        s.find.replace = t.to_string();
    }
}

fn find(s: &mut Session, v: &Value) -> CmdResult {
    read_opts(s, v);
    if p::str(v, "text").is_none() {
        s.view.nav_pane = true;
        s.ui_requests.push(json!({"open": "find"}));
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

fn replace(s: &mut Session, v: &Value) -> CmdResult {
    read_opts(s, v);
    let story = s.sel.focus.story;
    let results = search(s, story)?;
    let (a, b) = s.sel.ordered();
    // Replace the current match if selected, then move to the next.
    if results.iter().any(|(x, y)| *x == a && *y == b) {
        let with = s.find.replace.clone();
        super::type_text(s, &with)?;
    }
    let _ = step(s, 1);
    Ok(json!({"remaining": search(s, story)?.len()}))
}

fn replace_all(s: &mut Session, v: &Value) -> CmdResult {
    read_opts(s, v);
    let story = s.sel.focus.story;
    let results = search(s, story)?;
    let with = s.find.replace.clone();
    let n = results.len();
    // Replace from the end so earlier positions stay valid.
    for (a, b) in results.into_iter().rev() {
        if s.doc.settings.track_changes {
            // Same path as a single replace, so the change is a reviewable revision.
            s.sel = Selection { anchor: a, focus: b };
            super::type_text(s, &with)?;
            continue;
        }
        let props = s.doc.para_at(&a).map(|p| p.props_of_char(a.off).clone()).unwrap_or_default();
        s.doc.delete_range(&a, &b)?;
        s.doc.insert_text(&a, &with, &props)?;
    }
    s.status = format!("All done. We made {n} replacements.");
    Ok(json!({"replaced": n}))
}

fn goto(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(pg) = p::u64(v, "page") {
        let l = s.layout();
        let idx = (pg.max(1) as usize - 1).min(l.pages.len().saturating_sub(1));
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
        s.ui_requests.push(json!({"scrollTo": "caret"}));
        return sel_result(s);
    }
    if let Some(name) = p::str(v, "bookmark") {
        let bm = s.doc.bookmarks().into_iter().find(|(n, _)| n == name).ok_or_else(|| CmdError::Failed(format!("no bookmark `{name}`")))?;
        s.sel = Selection::caret(bm.1);
        return sel_result(s);
    }
    if let Some(n) = p::u64(v, "paragraph") {
        let paths = s.doc.para_paths(StoryRef::Body);
        if let Some(path) = paths.get((n.max(1) - 1) as usize) {
            s.sel = Selection::caret(Pos { story: StoryRef::Body, path: path.clone(), off: 0 });
        }
        return sel_result(s);
    }
    s.ui_requests.push(json!({"open": "goto"}));
    sel_result(s)
}

fn painter_pick(s: &mut Session) {
    let f = s.sel.ordered().0;
    let chr = s.doc.para_at(&f).map(|p| p.props_of_char(f.off).clone()).unwrap_or_default();
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
