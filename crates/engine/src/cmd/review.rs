//! Review tab: comments, track changes, word count, spelling (via the proofing hook).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Comment, Paragraph, PartKind, Pos, StoryRef, para_block};

use super::{fmt_revisions, now_iso, pos_json, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("review.newComment", "New Comment", "Review › Comments", new_comment).key(super::mac_or("Mod+Alt+A / Mod+Alt+M", "Mod+Alt+M")).params(r#"{"text": string}"#),
        CommandSpec::new("review.reply", "Reply", "Review › Comments", reply)
            .params(r#"{"id": non-negative JSON integer, "text": string}"#)
            .when(comments_change),
        CommandSpec::new("review.editComment", "Edit Comment", "Review › Comments", edit_comment)
            .params(r#"{"id": non-negative JSON integer, "text": string}"#)
            .when(comments_change),
        CommandSpec::new("review.deleteComment", "Delete", "Review › Comments", delete_comment).params(r#"{"id"?: non-negative JSON integer (omit to use the caret), "all"?: bool}"#),
        CommandSpec::new("review.resolveComment", "Resolve", "Review › Comments", |s, v| {
            validate_comment_id(v)?;
            let id = p::u64(v, "id").map(|x| x as u32).or_else(|| comment_at_caret(s)).ok_or_else(|| CmdError::Params("`id` required".into()))?;
            let c = s.doc.comments.get_mut(&id).ok_or_else(|| CmdError::Params("no such comment".into()))?;
            c.resolved = p::bool(v, "value").unwrap_or(!c.resolved);
            sel_result(s)
        })
        .params(r#"{"id"?: non-negative JSON integer (omit to use the caret), "value"?: bool (omit to toggle)}"#),
        CommandSpec::new("review.nextComment", "Next", "Review › Comments", |s, _| nav_comment(s, 1)).pure(),
        CommandSpec::new("review.previousComment", "Previous", "Review › Comments", |s, _| nav_comment(s, -1)).pure(),
        CommandSpec::new("review.comments", "Show Comments", "Review › Comments", list_comments).pure(),
        CommandSpec::new("review.trackChanges", "Track Changes", "Review › Tracking", |s, v| {
            s.doc.settings.track_changes = p::bool(v, "value").unwrap_or(!s.doc.settings.track_changes);
            Ok(json!({"value": s.doc.settings.track_changes}))
        })
        .params(r#"{"value"?: bool (omit to toggle)}"#)
        .key("Mod+Shift+E"),
        CommandSpec::new("review.acceptAll", "Accept All Changes", "Review › Changes", |s, v| resolve_all_then(s, v, true))
            .params(r#"{"stopTracking"?: bool (also turn Track Changes off)}"#),
        CommandSpec::new("review.rejectAll", "Reject All Changes", "Review › Changes", |s, v| resolve_all_then(s, v, false))
            .params(r#"{"stopTracking"?: bool (also turn Track Changes off)}"#),
        CommandSpec::new("review.accept", "Accept", "Review › Changes", |s, v| resolve_sel_then(s, v, true))
            .params(r#"{"next"?: bool (then select the next change)}"#),
        CommandSpec::new("review.reject", "Reject", "Review › Changes", |s, v| resolve_sel_then(s, v, false))
            .params(r#"{"next"?: bool (then select the next change)}"#),
        CommandSpec::new("review.nextChange", "Next Change", "Review › Changes", |s, _| nav_change(s, 1)).pure(),
        CommandSpec::new("review.previousChange", "Previous Change", "Review › Changes", |s, _| nav_change(s, -1)).pure(),
        CommandSpec::new("review.markup", "Display for Review", "Review › Tracking", |s, v| {
            s.view.show_markup = p::str(v, "value").map(|m| m != "noMarkup" && m != "original").unwrap_or(!s.view.show_markup);
            // Without markup, deletions leave the layout.
            s.relayout();
            Ok(json!({"showMarkup": s.view.show_markup}))
        })
        .pure(),
        CommandSpec::new("review.trackingOptions", "Track Changes Options", "Review › Tracking", tracking_options)
            .params(
                r#"{"comments"?: bool, "ink"?: bool, "insertionsDeletions"?: bool, "formatting"?: bool, "balloons"?: "revisions|inline|commentsAndFormatting", "commentDisplay"?: "margin|selected", "insertMark"?: "underline|doubleUnderline|bold|italic|strikethrough|colorOnly|none", "insertColor"?: "byAuthor" | "RRGGBB", "deleteMark"?: "strikethrough|doubleStrikethrough|hidden|caret|hash|underline|doubleUnderline|bold|italic|colorOnly|none", "deleteColor"?: "byAuthor" | "RRGGBB", "changedLines"?: "outside|left|right|none", "changedLinesColor"?: "auto" | "RRGGBB", "trackFormatting"?: bool, "moveFromMark"?: (as deleteMark; none = plain text, no colour), "moveFromColor"?: "byAuthor" | "RRGGBB", "moveToMark"?: (as insertMark; none = plain text, no colour), "moveToColor"?: "byAuthor" | "RRGGBB", "trackMoves"?: bool (the document's setting; off = a cut and paste is a deletion and an insertion, not a move)} → the options (omit all to read them)"#,
            )
            .pure(),
        CommandSpec::new("review.wordCount", "Word Count", "Review › Proofing", word_count)
            .params(r#"{"includeTextBoxes"?: bool}"#)
            // ⌘⇧G is Find Previous on macOS.
            .key(super::non_mac("Mod+Shift+G"))
            .pure(),
        CommandSpec::new("review.changes", "Reviewing Pane", "Review › Tracking", list_changes).pure(),
        CommandSpec::new("review.spelling", "Spelling & Grammar", "Review › Proofing", next_issue)
            .key(super::mac_or("F7 / Alt+F7 / Mod+Alt+L", "F7 / Alt+F7"))
            .pure(),
        CommandSpec::new("review.issues", "Proofing Issues", "Review › Proofing", all_issues).pure(),
        CommandSpec::new("review.suggestions", "Spelling Suggestions", "Review › Proofing", suggestions).params(r#"{"pos"?: Pos}"#).pure(),
        CommandSpec::new("review.addToDictionary", "Add to Dictionary", "Review › Proofing", |s, v| {
            let w = match p::str(v, "word") {
                Some(w) => w.to_string(),
                None => issue_at(s, &s.sel.focus.clone())
                    .map(|(a, b, _)| s.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off).map(str::to_string)).unwrap_or_default())
                    .unwrap_or_default(),
            };
            if w.is_empty() {
                return Err(CmdError::Params("no word".into()));
            }
            // The same rule as loading the dictionary file, so a word added is never dropped later.
            if wordcraft_proof::custom_word(&w).is_none() {
                let max = wordcraft_proof::MAX_CUSTOM_WORD_CHARS;
                return Err(CmdError::Params(format!("{w:?} isn't a single word of at most {max} characters")));
            }
            wordcraft_proof::add_word(w.trim());
            s.persist_user_dictionary(&[]);
            s.relayout();
            Ok(json!({"added": w}))
        })
        .params(r#"{"word"?: string}"#)
        .pure(),
        CommandSpec::new("review.ignoreAll", "Ignore All", "Review › Proofing", |s, v| {
            let w = match p::str(v, "word") {
                Some(w) => w.to_string(),
                None => issue_at(s, &s.sel.focus.clone())
                    .map(|(a, b, _)| s.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off).map(str::to_string)).unwrap_or_default())
                    .unwrap_or_default(),
            };
            // For this session only, as in Word: not added to (or saved with) the dictionary.
            if !w.trim().is_empty() {
                wordcraft_proof::ignore_word(w.trim());
            }
            s.relayout();
            Ok(json!({"ignored": w}))
        })
        .pure(),
        CommandSpec::new("review.applySuggestion", "Change", "Review › Proofing", |s, v| {
            let text = p::req_str(v, "text")?.to_string();
            let at = s.sel.focus.clone();
            if let Some((a, b, _)) = issue_at(s, &at) {
                s.sel = Selection { anchor: a, focus: b };
            }
            super::type_text(s, &text)?;
            sel_result(s)
        })
        .params(r#"{"text": string}"#),
        CommandSpec::new("review.proofing", "Check Spelling as You Type", "File › Options › Proofing", |s, v| {
            s.view.proofing = p::bool(v, "value").unwrap_or(!s.view.proofing);
            s.prefs.check_spelling = s.view.proofing;
            Ok(json!({"value": s.view.proofing}))
        })
        .pure(),
        CommandSpec::new("review.thesaurus", "Thesaurus", "Review › Proofing", thesaurus).key("Shift+F7").pure(),
    ]
}

fn new_comment(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    let (a, b) = s.sel.ordered();
    let (a, b) = if a == b {
        // Comment on the word at the caret.
        match s.doc.para_at(&a) {
            Some(para) if !para.is_empty() => {
                let (x, y) = para.word_at(a.off);
                let y = para.text.get(x..y).map(|w| x + w.trim_end().len()).unwrap_or(y);
                (Pos { off: x, ..a.clone() }, Pos { off: y, ..a })
            }
            _ => (a.clone(), a),
        }
    } else {
        (a, b)
    };
    let part = s.doc.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(&text, Default::default()))])?;
    let id = comment_id(s)?;
    let initials = s.user_initials();
    let now = now_iso();
    s.doc.comments.insert(
        id,
        Comment {
            author: s.author.clone(),
            initials,
            date: now.clone(),
            date_utc: Some(now),
            parent: None,
            resolved: false,
            part,
            ..Default::default()
        },
    );
    let props = Default::default();
    s.doc.insert_object(&b, InlineObject::CommentEnd { id }, &props)?;
    s.doc.insert_object(&a, InlineObject::CommentStart { id }, &props)?;
    s.view.comments_pane = true;
    Ok(json!({"id": id, "story": part}))
}

/// An omitted ID permits caret targeting; an explicit malformed value must not.
/// Unlike the general numeric parameter helper, comment IDs never coerce floats.
fn validate_comment_id(v: &Value) -> Result<(), CmdError> {
    if v.get("id").is_some_and(|id| id.as_u64().is_none_or(|n| u32::try_from(n).is_err())) {
        return Err(CmdError::Params("`id` must be a non-negative 32-bit JSON integer".into()));
    }
    Ok(())
}

fn reply(s: &mut Session, v: &Value) -> CmdResult {
    validate_comment_id(v)?;
    let parent = p::u64(v, "id").map(|x| x as u32).ok_or_else(|| CmdError::Params("`id` required".into()))?;
    if !s.doc.comments.contains_key(&parent) {
        return Err(CmdError::Params("no such comment".into()));
    }
    let text = p::req_str(v, "text")?;
    let part = s.doc.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(text, Default::default()))])?;
    let id = comment_id(s)?;
    let initials = s.user_initials();
    let now = now_iso();
    s.doc.comments.insert(
        id,
        Comment {
            author: s.author.clone(),
            initials,
            date: now.clone(),
            date_utc: Some(now),
            parent: Some(parent),
            resolved: false,
            part,
            ..Default::default()
        },
    );
    Ok(json!({"id": id}))
}

/// An id no comment has: one past the largest, or the lowest free one once a file used `u32::MAX`.
fn comment_id(s: &Session) -> Result<u32, CmdError> {
    let ids = &s.doc.comments;
    wordcraft_doc::free_id(ids.keys().next_back().copied(), ids.keys().copied(), 0).ok_or_else(|| CmdError::Failed("no free comment id".into()))
}

/// Comments can't be written or edited in a document protected as read only or for filling in
/// forms (the comment fields are read-only there, #1348).
fn comments_change(s: &Session) -> Option<&'static str> {
    match s.doc.settings.protection.as_deref() {
        Some("readOnly" | "forms") => Some("the document is protected"),
        _ => None,
    }
}

/// Replace a comment's (or reply's) text; each line becomes a paragraph.
fn edit_comment(s: &mut Session, v: &Value) -> CmdResult {
    validate_comment_id(v)?;
    let id = p::u64(v, "id").and_then(|x| u32::try_from(x).ok()).ok_or_else(|| CmdError::Params("`id` required".into()))?;
    let text = p::req_str(v, "text")?;
    let part = s.doc.comments.get(&id).map(|c| c.part).ok_or_else(|| CmdError::Params("no such comment".into()))?;
    let blocks = text.split('\n').take(10_000).map(|l| para_block(Paragraph::with_text(l.trim_end_matches('\r'), Default::default()))).collect();
    s.doc.set_story(StoryRef::Part(part), blocks)?;
    Ok(json!({"id": id}))
}

/// The innermost comment whose range holds the caret.
pub fn comment_range_at_caret(s: &Session) -> Option<u32> {
    let f = &s.sel.focus;
    let p = s.doc.para_at(f)?;
    let mut open: Vec<u32> = Vec::new();
    for off in p.object_offsets() {
        if off > f.off {
            break;
        }
        match p.object_at(off) {
            Some(InlineObject::CommentStart { id }) => open.push(*id),
            Some(InlineObject::CommentEnd { id }) => open.retain(|x| x != id),
            _ => {}
        }
    }
    open.last().copied()
}

/// Comment anchored around the caret, else the first one in its paragraph.
fn comment_at_caret(s: &Session) -> Option<u32> {
    let f = &s.sel.focus;
    let p = s.doc.para_at(f)?;
    comment_range_at_caret(s).or_else(|| {
        p.object_offsets().into_iter().find_map(|o| match p.object_at(o) {
            Some(InlineObject::CommentStart { id }) | Some(InlineObject::CommentEnd { id }) => Some(*id),
            _ => None,
        })
    })
}

/// Removes the comments' anchors from every story: the body, headers, footers, notes and text
/// boxes (#1345).
fn remove_anchors(s: &mut Session, ids: &BTreeSet<u32>) -> Result<(), CmdError> {
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for story in stories {
        for path in s.doc.para_paths(story) {
            let offs: Vec<usize> = s
                .doc
                .para(story, &path)
                .map(|p| {
                    p.object_offsets()
                        .into_iter()
                        .filter(|o| matches!(p.object_at(*o), Some(InlineObject::CommentStart { id }) | Some(InlineObject::CommentEnd { id }) if ids.contains(id)))
                        .collect()
                })
                .unwrap_or_default();
            if offs.is_empty() {
                continue;
            }
            let para = s.doc.para_mut(story, &path)?;
            for o in offs.into_iter().rev() {
                para.delete(o, o + wordcraft_doc::para::OBJ.len_utf8())?;
            }
        }
    }
    Ok(())
}

/// The comment and every reply below it, replies to replies included (#1345). A parent cycle
/// from a hostile file ends the walk instead of looping.
fn thread_of(s: &Session, id: u32) -> Vec<u32> {
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (k, c) in &s.doc.comments {
        if let Some(parent) = c.parent {
            children.entry(parent).or_default().push(*k);
        }
    }
    let mut seen = BTreeSet::from([id]);
    let mut ids = vec![id];
    let mut i = 0;
    while let Some(&at) = ids.get(i) {
        for k in children.get(&at).into_iter().flatten() {
            if seen.insert(*k) {
                ids.push(*k);
            }
        }
        i += 1;
    }
    ids
}

fn delete_comment(s: &mut Session, v: &Value) -> CmdResult {
    validate_comment_id(v)?;
    let ids: Vec<u32> = if p::bool(v, "all").unwrap_or(false) {
        s.doc.comments.keys().copied().collect()
    } else {
        let id = p::u64(v, "id").map(|x| x as u32).or_else(|| comment_at_caret(s)).ok_or_else(|| CmdError::Params("no comment here".into()))?;
        thread_of(s, id)
    };
    remove_anchors(s, &ids.iter().copied().collect())?;
    for id in &ids {
        if let Some(c) = s.doc.comments.remove(id) {
            s.doc.parts.remove(&c.part);
        }
    }
    s.clamp_selection();
    Ok(json!({"deleted": ids}))
}

/// Comments in reading order with their anchors: every story the reader sees, in
/// `Document::shown_stories` order (#2289). A comment anchored only in a story nothing shows
/// (a removed header) has no anchor.
pub fn comment_list(s: &Session) -> Vec<(u32, Option<Pos>)> {
    let mut found: Vec<(u32, Option<Pos>)> = Vec::new();
    // The balloons ask every frame; most documents have no comments.
    if s.doc.comments.is_empty() {
        return found;
    }
    for story in s.doc.shown_stories() {
        for path in s.doc.para_paths(story) {
            let Some(p) = s.doc.para(story, &path) else { continue };
            for o in p.object_offsets() {
                if let Some(InlineObject::CommentStart { id }) = p.object_at(o) {
                    found.push((*id, Some(Pos { story, path: path.clone(), off: o })));
                }
            }
        }
    }
    for id in s.doc.comments.keys() {
        if !found.iter().any(|(x, _)| x == id) {
            found.push((*id, None));
        }
    }
    found
}

fn list_comments(s: &mut Session, _: &Value) -> CmdResult {
    let list = comment_list(s);
    Ok(Value::Array(
        list.iter()
            .filter_map(|(id, pos)| {
                let c = s.doc.comments.get(id)?;
                Some(json!({
                    "id": id, "author": c.author, "date": c.date, "resolved": c.resolved, "parent": c.parent,
                    "text": s.doc.plain_text(StoryRef::Part(c.part)),
                    "anchor": pos.as_ref().map(pos_json),
                }))
            })
            .collect(),
    ))
}

fn nav_comment(s: &mut Session, dir: i32) -> CmdResult {
    let list: Vec<Pos> = comment_list(s).into_iter().filter_map(|(_, p)| p).collect();
    // Reading order: the story's place in `shown_stories`, then the place in the story. Part ids
    // follow creation, not reading, so `Pos` order won't do across stories.
    let rank: BTreeMap<StoryRef, usize> = s.doc.shown_stories().into_iter().enumerate().map(|(i, st)| (st, i)).collect();
    let key = |p: &Pos| rank.get(&p.story).map(|i| (*i, p.path.clone(), p.off));
    let caret = key(&s.sel.focus);
    let t = if dir > 0 {
        list.iter().find(|p| caret.is_some() && key(p) > caret).or(list.first())
    } else {
        list.iter().rev().find(|p| caret.is_some() && key(p) < caret).or(list.last())
    };
    if let Some(p) = t {
        s.sel = Selection::caret(p.clone());
    }
    sel_result(s)
}

/// Accept or reject the tracked formatting and text of `from..to` in one paragraph (not its mark).
fn resolve_runs(para: &mut wordcraft_doc::Paragraph, from: usize, to: usize, props: bool, accept: bool) -> Result<(), CmdError> {
    if props && (para.props.fmt_change.is_some() || para.props.num_change.is_some()) {
        fmt_revisions::resolve_para(&mut para.props, accept);
        para.touch();
    }
    let ranges: Vec<(usize, usize, bool, bool, bool)> = para
        .run_ranges()
        .filter(|(r, _)| r.end > from && r.start < to)
        .map(|(r, c)| (r.start.max(from), r.end.min(to), c.ins.is_some(), c.del.is_some(), c.fmt_change.is_some()))
        .collect();
    for (a, b, ins, del, fmt) in ranges.into_iter().rev() {
        if (del && accept) || (ins && !accept) {
            para.delete(a, b)?;
        } else if ins || del || fmt {
            para.format(a, b, &|c| {
                c.ins = None;
                c.del = None;
                fmt_revisions::resolve_char(c, accept);
            })?;
        }
    }
    Ok(())
}

/// Accept or reject a revision range in one paragraph; `mark` includes its paragraph mark (and
/// the section break on it), `props` the paragraph's own formatting change.
#[allow(clippy::too_many_arguments)]
fn resolve_para(
    s: &mut Session,
    story: StoryRef,
    path: &wordcraft_doc::Path,
    from: usize,
    to: usize,
    mark: bool,
    props: bool,
    accept: bool,
) -> Result<(), CmdError> {
    let para = s.doc.para_mut(story, path)?;
    resolve_runs(para, from, to, props, accept)?;
    if !mark {
        return Ok(());
    }
    if para.mark.fmt_change.is_some() || para.section.as_ref().is_some_and(|x| x.fmt_change.is_some()) {
        fmt_revisions::resolve_char(&mut para.mark, accept);
        if let Some(sec) = para.section.as_deref_mut() {
            fmt_revisions::resolve_section(sec, accept);
        }
        para.touch();
    }
    let (ins, del) = (para.mark.ins, para.mark.del);
    if ins.is_none() && del.is_none() {
        return Ok(());
    }
    // An inserted paragraph mark that is rejected, or a deleted one that is accepted, goes away:
    // the next paragraph joins this one (the split is undone). A paragraph left empty goes away
    // whole, so the next one keeps its own style and numbering, as in Word. The mark at the end
    // of a story or cell can't go, so it just stops being a revision.
    if ((del.is_some() && accept) || (ins.is_some() && !accept))
        && (super::remove_empty_para(s, story, path)? || super::join_next_para(s, story, path)?)
    {
        return Ok(());
    }
    let para = s.doc.para_mut(story, path)?;
    para.mark.ins = None;
    para.mark.del = None;
    para.touch();
    Ok(())
}

/// Accept or reject the changes of the table cell `path` is in: the table's, its row's and the
/// cell's own. Returns whether there were any.
fn resolve_cell(s: &mut Session, story: StoryRef, path: &wordcraft_doc::Path, accept: bool) -> Result<bool, CmdError> {
    let Some((tp, r, c)) = path.cell() else { return Ok(false) };
    let has = s.doc.table(story, &tp).is_some_and(|t| {
        let row = t.rows.get(r);
        t.props.fmt_change.is_some()
            || row.is_some_and(|x| x.props.fmt_change.is_some())
            || row.and_then(|x| x.cells.get(c)).is_some_and(|x| x.props.fmt_change.is_some())
    });
    if !has {
        return Ok(false);
    }
    let t = s.doc.table_mut(story, &tp)?;
    fmt_revisions::resolve_table(t, accept);
    if let Some(row) = t.rows.get_mut(r) {
        fmt_revisions::resolve_row(row, accept);
        if let Some(cell) = row.cells.get_mut(c) {
            fmt_revisions::resolve_cell(cell, accept);
        }
    }
    Ok(true)
}

/// A tracked change, for navigation and the Reviewing Pane: its range, kind (`insert`,
/// `delete`, `format`), revision, and for a formatting change the properties it set.
struct Change {
    a: Pos,
    b: Pos,
    kind: &'static str,
    rev: Option<u32>,
    props: Vec<Value>,
}

impl Change {
    fn format(a: Pos, b: Pos, rev: u32, props: Vec<Value>) -> Change {
        Change { a, b, kind: "format", rev: Some(rev), props }
    }
}

/// The tracked paragraph mark at the end of the paragraph at `path`, as a change range: from
/// the paragraph's end to the start of the next paragraph (or the end itself when none follows).
fn mark_change(s: &Session, story: StoryRef, path: &wordcraft_doc::Path) -> Option<Change> {
    let p = s.doc.para(story, path)?;
    let kind = run_kind(&s.doc, p.mark.ins, p.mark.del);
    if kind.is_empty() {
        return None;
    }
    let a = Pos { story, path: path.clone(), off: p.len() };
    let b = if super::has_next_para(s, story, path) { Pos { story, path: path.with_last(path.last().saturating_add(1)), off: 0 } } else { a.clone() };
    Some(Change { a, b, kind, rev: p.mark.ins.or(p.mark.del), props: Vec::new() })
}

/// The kind of an insertion / deletion change: `insert`, `delete`, or for one end of a move
/// `moveTo` / `moveFrom`; empty when neither.
fn run_kind(doc: &wordcraft_doc::Document, ins: Option<u32>, del: Option<u32>) -> &'static str {
    match (ins, del) {
        (Some(_), _) if doc.move_name(ins).is_some() => "moveTo",
        (Some(_), _) => "insert",
        (None, Some(_)) if doc.move_name(del).is_some() => "moveFrom",
        (None, Some(_)) => "delete",
        _ => "",
    }
}

/// The moves in the body, for balloons: where each end starts, its author, and whether it is
/// the source (`true`) or the destination.
pub fn move_changes(s: &Session) -> Vec<(Pos, String, bool)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for c in changes(s) {
        let from = match c.kind {
            "moveFrom" => true,
            "moveTo" => false,
            _ => continue,
        };
        let rev = c.rev.and_then(|r| s.doc.revisions.get(r as usize));
        let name = rev.and_then(|r| r.move_name.clone()).unwrap_or_default();
        // One balloon per end (an end runs on across paragraphs).
        if seen.insert((name, from)) {
            out.push((c.a, rev.map(|r| r.author.clone()).unwrap_or_default(), from));
        }
    }
    out
}

/// Accept or reject the tracked text and paragraph marks of the top-level paragraphs in a block
/// list that is not a story (a note separator).
fn resolve_loose_blocks(blocks: &mut wordcraft_doc::Blocks, accept: bool) -> Result<(), CmdError> {
    for b in blocks.iter_mut() {
        let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) else { continue };
        let len = p.len();
        resolve_runs(p, 0, len, true, accept)?;
        if p.mark.fmt_change.is_some() || p.section.as_ref().is_some_and(|x| x.fmt_change.is_some()) {
            fmt_revisions::resolve_char(&mut p.mark, accept);
            if let Some(sec) = p.section.as_deref_mut() {
                fmt_revisions::resolve_section(sec, accept);
            }
        }
        // A mark can't be removed here (no neighbour to join), so it just stops being a revision.
        p.mark.ins = None;
        p.mark.del = None;
        p.touch();
    }
    fmt_revisions::resolve_containers(blocks, accept, 0);
    Ok(())
}

fn resolve_all(s: &mut Session, accept: bool) -> CmdResult {
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for st in stories {
        for path in s.doc.para_paths(st).into_iter().rev() {
            let len = s.doc.para(st, &path).map(|p| p.len()).unwrap_or(0);
            resolve_para(s, st, &path, 0, len, true, true, accept)?;
        }
    }
    // Tables, rows, cells and sections.
    fmt_revisions::resolve_containers(&mut s.doc.body, accept, 0);
    for part in s.doc.parts.values_mut() {
        fmt_revisions::resolve_containers(&mut part.blocks, accept, 0);
    }
    fmt_revisions::resolve_section(&mut s.doc.last_section, accept);
    // The note separators are block lists of their own, not stories; they must not keep ids of
    // revisions that are about to be dropped.
    for sep in [&mut s.doc.footnote_separators, &mut s.doc.endnote_separators] {
        for blocks in [&mut sep.separator, &mut sep.continuation_separator, &mut sep.continuation_notice].into_iter().flatten() {
            resolve_loose_blocks(blocks, accept)?;
        }
    }
    s.doc.revisions.clear();
    s.clamp_selection();
    sel_result(s)
}

/// Accept or Reject All; with `stopTracking`, Track Changes is turned off as well.
fn resolve_all_then(s: &mut Session, v: &Value, accept: bool) -> CmdResult {
    let out = resolve_all(s, accept)?;
    if p::bool(v, "stopTracking") == Some(true) {
        s.doc.settings.track_changes = false;
    }
    Ok(out)
}

/// Accept or Reject at the selection; with `next`, the next change is selected afterwards.
fn resolve_sel_then(s: &mut Session, v: &Value, accept: bool) -> CmdResult {
    resolve_sel(s, accept, p::bool(v, "next") == Some(true))
}

fn resolve_sel(s: &mut Session, accept: bool, next: bool) -> CmdResult {
    let (a, b) = s.sel.ordered();
    // A caret right before a tracked paragraph mark (and no tracked text around it): that mark.
    let mut caret_mark = None;
    // A caret in a paragraph whose own formatting changed (and no tracked text around it).
    let mut caret_props = None;
    let ranged = a != b;
    let (a, b) = if a == b {
        // The change at the caret: the run around it, else the paragraph mark right after it,
        // else the paragraph's formatting, else the table cell's.
        let found = s.doc.para_at(&a).and_then(|p| {
            let runs: Vec<_> = p.run_ranges().collect();
            let i = runs
                .iter()
                .position(|(r, c)| r.start <= a.off && a.off <= r.end && (c.ins.is_some() || c.del.is_some() || c.fmt_change.is_some()))?;
            let (mut start, mut end) = (runs[i].0.start, runs[i].0.end);
            let key = (runs[i].1.ins, runs[i].1.del);
            if key == (None, None) {
                return Some(start..end);
            }
            // Comment anchors split a revision's text into several runs; the revision goes on past them.
            let is_anchor =
                |k: usize| matches!(p.object_at(runs[k].0.start), Some(InlineObject::CommentStart { .. } | InlineObject::CommentEnd { .. }));
            let same = |k: usize| (runs[k].1.ins, runs[k].1.del) == key;
            let mut k = i;
            while k > 0 {
                if same(k - 1) {
                    k -= 1;
                } else if is_anchor(k - 1) && k > 1 && same(k - 2) {
                    k -= 2;
                } else {
                    break;
                }
                start = runs[k].0.start;
            }
            let mut k = i;
            while k + 1 < runs.len() {
                if same(k + 1) {
                    k += 1;
                } else if is_anchor(k + 1) && k + 2 < runs.len() && same(k + 2) {
                    k += 2;
                } else {
                    break;
                }
                end = runs[k].0.end;
            }
            Some(start..end)
        });
        let para_fmt = s.doc.para_at(&a).is_some_and(|p| p.props.fmt_change.is_some() || p.props.num_change.is_some());
        let mark_fmt = s.doc.para_at(&a).is_some_and(|p| a.off == p.len() && p.mark.fmt_change.is_some());
        match found {
            Some(r) => (Pos { off: r.start, ..a.clone() }, Pos { off: r.end, ..a }),
            None if mark_fmt || mark_change(s, a.story, &a.path).is_some_and(|m| m.a == a) => {
                caret_mark = Some(a.path.clone());
                (a.clone(), a)
            }
            None if para_fmt => {
                caret_props = Some(a.path.clone());
                (a.clone(), a)
            }
            None => {
                if resolve_cell(s, a.story, &a.path, accept)? {
                    return if next { select_next_unresolved(s) } else { sel_result(s) };
                }
                return nav_change(s, 1);
            }
        }
    } else {
        (a, b)
    };
    // Moves with an end in the range: their other ends go with them.
    let moves = super::moves::names_in(s, &a, &b, caret_mark.as_ref());
    for path in s.doc.paths_between(&a, &b).into_iter().rev() {
        let len = s.doc.para(a.story, &path).map(|p| p.len()).unwrap_or(0);
        let from = if path == a.path { a.off } else { 0 };
        let to = if path == b.path { b.off } else { len };
        // A paragraph's mark is in the selection when the selection goes on past it.
        let mark = path != b.path || caret_mark.as_ref() == Some(&path);
        let props = (ranged && (path != b.path || to > from)) || caret_props.as_ref() == Some(&path);
        resolve_para(s, a.story, &path, from, to, mark, props, accept)?;
        if ranged {
            resolve_cell(s, a.story, &path, accept)?;
        }
    }
    super::moves::resolve(s, &moves, accept)?;
    s.sel = Selection::caret(a);
    s.clamp_selection();
    if next { select_next_unresolved(s) } else { sel_result(s) }
}

fn changes(s: &Session) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    let mk = |path: &wordcraft_doc::Path, off| Pos { story: StoryRef::Body, path: path.clone(), off };
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        // A table's, row's and cell's changes sit at the start of the cell's first paragraph.
        if path.last() == 0
            && let Some((tp, r, c)) = path.cell()
            && let Some(t) = s.doc.table(StoryRef::Body, &tp)
        {
            let row = t.rows.get(r);
            let cell = row.and_then(|x| x.cells.get(c));
            let found = [
                (r == 0 && c == 0).then(|| t.props.fmt_change.as_ref().map(|ch| (ch.rev, fmt_revisions::table_diff(&t.props)))).flatten(),
                (c == 0).then(|| row.and_then(|x| x.props.fmt_change.as_ref().map(|ch| (ch.rev, fmt_revisions::row_diff(&x.props))))).flatten(),
                cell.and_then(|x| x.props.fmt_change.as_ref().map(|ch| (ch.rev, fmt_revisions::cell_diff(&x.props)))),
            ];
            for (rev, props) in found.into_iter().flatten() {
                out.push(Change::format(mk(&path, 0), mk(&path, 0), rev, props));
            }
        }
        if let Some(ch) = &p.props.fmt_change {
            out.push(Change::format(mk(&path, 0), mk(&path, p.len()), ch.rev, fmt_revisions::para_diff(&p.props)));
        }
        for (r, c) in p.run_ranges() {
            let kind = run_kind(&s.doc, c.ins, c.del);
            if !kind.is_empty() {
                out.push(Change { a: mk(&path, r.start), b: mk(&path, r.end), kind, rev: c.ins.or(c.del), props: Vec::new() });
            }
            let Some(ch) = &c.fmt_change else { continue };
            let props = fmt_revisions::char_diff(c);
            // Neighbouring runs changed the same way by the same revision are one change.
            if let Some(last) =
                out.last_mut().filter(|l| l.kind == "format" && l.rev == Some(ch.rev) && l.b == mk(&path, r.start) && l.props == props)
            {
                last.b = mk(&path, r.end);
            } else {
                out.push(Change::format(mk(&path, r.start), mk(&path, r.end), ch.rev, props));
            }
        }
        if let Some(ch) = &p.mark.fmt_change {
            let end = mk(&path, p.len());
            out.push(Change::format(end.clone(), end, ch.rev, fmt_revisions::char_diff(&p.mark)));
        }
        out.extend(mark_change(s, StoryRef::Body, &path));
        if let Some(ch) = p.section.as_ref().and_then(|x| x.fmt_change.as_ref().map(|ch| (ch.rev, x))) {
            let end = mk(&path, p.len());
            out.push(Change::format(end.clone(), end, ch.0, fmt_revisions::section_diff(ch.1)));
        }
    }
    if let Some(ch) = &s.doc.last_section.fmt_change {
        let end = s.doc.end_of(StoryRef::Body);
        out.push(Change::format(end.clone(), end, ch.rev, fmt_revisions::section_diff(&s.doc.last_section)));
    }
    out
}

/// The formatting changes in the body, for balloons: (position, author, properties set).
pub fn format_changes(s: &Session) -> Vec<(Pos, String, Vec<Value>)> {
    changes(s)
        .into_iter()
        .filter(|c| c.kind == "format")
        .map(|c| {
            let author = c.rev.and_then(|r| s.doc.revisions.get(r as usize)).map(|r| r.author.clone()).unwrap_or_default();
            (c.a, author, c.props)
        })
        .collect()
}

/// Whether the document has any tracked formatting change (cheap: stops at the first).
pub fn has_format_changes(doc: &wordcraft_doc::Document) -> bool {
    if doc.last_section.fmt_change.is_some() {
        return true;
    }
    doc.para_paths(StoryRef::Body).iter().any(|path| {
        doc.para(StoryRef::Body, path).is_some_and(|p| {
            p.props.fmt_change.is_some()
                || p.mark.fmt_change.is_some()
                || p.runs.iter().any(|r| r.props.fmt_change.is_some())
                || p.section.as_ref().is_some_and(|x| x.fmt_change.is_some())
                || path.cell().and_then(|(tp, r, c)| doc.table(StoryRef::Body, &tp).map(|t| (t, r, c))).is_some_and(|(t, r, c)| {
                    t.props.fmt_change.is_some()
                        || t.rows.get(r).is_some_and(|x| x.props.fmt_change.is_some() || x.cells.get(c).is_some_and(|x| x.props.fmt_change.is_some()))
                })
        })
    })
}

/// A tracked deletion in the body shown in a balloon (Show revisions in balloons).
#[derive(Clone, Debug, PartialEq)]
pub struct DeletionBalloon {
    /// Where the deleted text was: the balloon's anchor.
    pub at: Pos,
    pub author: String,
    /// The deleted text, a deleted paragraph mark as ¶, shortened to [`DELETION_TEXT_CHARS`].
    pub text: String,
}

/// At most this many characters of deleted text show in a balloon (a deleted page shows its start).
pub const DELETION_TEXT_CHARS: usize = 200;
/// At most this many deletion balloons (a document of many small deletions can't flood the area).
const MAX_DELETION_BALLOONS: usize = 500;

/// The tracked deletions in the body, for balloons, in document order. Deleted text running on
/// across runs, revisions and paragraph marks by the same author is one balloon. The source of a
/// move is left out: it has its "Moved from" balloon ([`move_changes`]).
pub fn deletion_balloons(s: &Session) -> Vec<DeletionBalloon> {
    let doc = &s.doc;
    let author = |rev: u32| doc.revisions.get(rev as usize).map(|r| r.author.clone()).unwrap_or_default();
    let mk = |path: &wordcraft_doc::Path, off| Pos { story: StoryRef::Body, path: path.clone(), off };
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for (r, c) in p.run_ranges() {
            // Inserted and then deleted text is a deletion (it's drawn and left out as one).
            let Some(rev) = plain_deletion(doc, c.del) else { continue };
            push_deletion(&mut out, mk(&path, r.start), mk(&path, r.end), author(rev), p.text.get(r).unwrap_or(""));
        }
        if let Some(rev) = plain_deletion(doc, p.mark.del) {
            let at = mk(&path, p.len());
            let next = path.with_last(path.last().saturating_add(1));
            let end = if super::has_next_para(s, StoryRef::Body, &path) { mk(&next, 0) } else { at.clone() };
            push_deletion(&mut out, at, end, author(rev), "¶");
        }
    }
    out.into_iter().map(|(d, _, _)| d).collect()
}

/// Add `author`'s deleted `text` (from `at` to `end`) to `out`: to the last balloon when that
/// deletion ends at `at` and is theirs, else as a new one. Each balloon carries where its deletion
/// ends and how many characters its text has.
fn push_deletion(out: &mut Vec<(DeletionBalloon, Pos, usize)>, at: Pos, end: Pos, author: String, text: &str) {
    let joins = out.last().is_some_and(|(d, e, _)| *e == at && d.author == author);
    if !joins {
        if out.len() >= MAX_DELETION_BALLOONS {
            return;
        }
        out.push((DeletionBalloon { at, author, text: String::new() }, end.clone(), 0));
    }
    let Some((d, e, chars)) = out.last_mut() else { return };
    *e = end;
    for c in text.chars() {
        if *chars > DELETION_TEXT_CHARS {
            break;
        }
        let c = match c {
            // Pictures, fields and other objects show no text of their own.
            wordcraft_doc::para::OBJ => continue,
            '\t' | wordcraft_doc::para::LINE_BREAK | wordcraft_doc::para::PAGE_BREAK | wordcraft_doc::para::COLUMN_BREAK => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        *chars += 1;
        d.text.push(if *chars > DELETION_TEXT_CHARS { '…' } else { c });
    }
}

/// A deletion revision that is not the source of a move (`w:moveFrom` is a deletion revision
/// with a move name).
fn plain_deletion(doc: &wordcraft_doc::Document, del: Option<u32>) -> Option<u32> {
    del.filter(|_| doc.move_name(del).is_none())
}

/// Whether the body has any tracked deletion other than a move's source (cheap: stops at the
/// first).
pub fn has_deletions(doc: &wordcraft_doc::Document) -> bool {
    doc.para_paths(StoryRef::Body).iter().any(|path| {
        doc.para(StoryRef::Body, path)
            .is_some_and(|p| plain_deletion(doc, p.mark.del).is_some() || p.runs.iter().any(|r| plain_deletion(doc, r.props.del).is_some()))
    })
}

fn nav_change(s: &mut Session, dir: i32) -> CmdResult {
    let list = changes(s);
    let caret = s.sel.ordered();
    let t = if dir > 0 {
        list.iter().find(|c| c.a >= caret.1 && (c.a > caret.0 || c.b > caret.1)).or(list.first())
    } else {
        list.iter().rev().find(|c| c.b <= caret.0 && (c.b < caret.1 || c.a < caret.0)).or(list.last())
    };
    if let Some(c) = t {
        s.sel = Selection { anchor: c.a.clone(), focus: c.b.clone() };
    }
    sel_result(s)
}

/// After a change at the caret is resolved: select the first change from the caret on, including
/// one that starts and ends there (an empty paragraph's, a paragraph mark's or a table's
/// formatting), else the first change in the document.
fn select_next_unresolved(s: &mut Session) -> CmdResult {
    let list = changes(s);
    let caret = s.sel.ordered().0;
    if let Some(c) = list.iter().find(|c| c.a >= caret).or(list.first()) {
        s.sel = Selection { anchor: c.a.clone(), focus: c.b.clone() };
    }
    sel_result(s)
}

fn list_changes(s: &mut Session, _: &Value) -> CmdResult {
    let list = changes(s);
    Ok(Value::Array(
        list.iter()
            .map(|c| {
                let rev = c.rev.and_then(|r| s.doc.revisions.get(r as usize));
                let mut v = json!({
                    "kind": c.kind, "start": pos_json(&c.a), "end": pos_json(&c.b),
                    "text": s.doc.copy_range(&c.a, &c.b).plain_text(),
                    "author": rev.map(|r| r.author.clone()), "date": rev.map(|r| r.date.clone()),
                });
                if c.kind == "format" {
                    v["description"] = json!(fmt_revisions::describe(&c.props));
                    v["props"] = json!(c.props);
                }
                // One end of a move: its name links it with the other end.
                if let Some(name) = rev.and_then(|r| r.move_name.as_deref()).filter(|_| c.kind.starts_with("move")) {
                    v["move"] = json!(name);
                    v["description"] = json!(if c.kind == "moveFrom" { "Moved from" } else { "Moved to" });
                }
                v
            })
            .collect(),
    ))
}

fn word_count(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(b) = p::bool(v, "includeTextBoxes") {
        s.prefs.count_notes = b;
    }
    let stories = if s.prefs.count_notes { s.doc.counted_stories() } else { vec![StoryRef::Body] };
    // A selected picture, shape or text box isn't a text selection: count the document.
    let text = if s.sel.is_collapsed() || super::objects::object_selection(s).is_some() {
        stories.iter().map(|st| s.doc.plain_text(*st)).collect::<Vec<_>>().join("\n")
    } else {
        s.selected_text()
    };
    let words = if s.sel.is_collapsed() || super::objects::object_selection(s).is_some() {
        stories.iter().map(|st| s.doc.story_word_count(*st)).sum()
    } else {
        wordcraft_doc::count_words(&text)
    };
    let chars = text.chars().filter(|c| *c != '\n' && *c != '\r').count();
    let chars_no_spaces = text.chars().filter(|c| !c.is_whitespace()).count();
    let paragraphs = text.split('\n').filter(|l| !l.trim().is_empty()).count();
    let lines: usize = {
        let l = s.layout();
        l.pages
            .iter()
            .flat_map(|p| p.items.iter())
            .map(|it| {
                if let wordcraft_layout::Placed::Lines { l0, l1, story, .. } = it
                    && stories.contains(story)
                {
                    l1 - l0
                } else {
                    0
                }
            })
            .sum()
    };
    let pages = s.layout().pages.len();
    Ok(json!({
        "pages": pages, "words": words, "characters": chars_no_spaces, "charactersWithSpaces": chars, "paragraphs": paragraphs, "lines": lines,
        "includeTextBoxes": s.prefs.count_notes,
    }))
}

/// Issues (spelling + grammar) in one paragraph as positions, each in its run's language and
/// skipping text left unchecked — the same issues the layout underlines.
fn para_issues(s: &Session, story: StoryRef, path: &wordcraft_doc::Path) -> Vec<(Pos, Pos, wordcraft_proof::Issue)> {
    let Some(p) = s.doc.para(story, path) else { return Vec::new() };
    let runs = wordcraft_layout::para::proof_runs(&s.doc, p, None);
    wordcraft_layout::para::paragraph_issues(p, &runs, &s.prefs.spelling(), true, wordcraft_layout::para::is_list_item(&s.doc, p))
        .into_iter()
        .map(|i| (Pos { story, path: path.clone(), off: i.start }, Pos { story, path: path.clone(), off: i.end }, i))
        .collect()
}

/// The issue under a position.
fn issue_at(s: &Session, at: &Pos) -> Option<(Pos, Pos, wordcraft_proof::Issue)> {
    para_issues(s, at.story, &at.path).into_iter().find(|(a, b, _)| a.off <= at.off && at.off <= b.off)
}

fn issue_json(s: &Session, a: &Pos, b: &Pos, i: &wordcraft_proof::Issue) -> Value {
    let word = s.doc.para_at(a).and_then(|p| p.text.get(a.off..b.off)).unwrap_or("").to_string();
    let sugg = if i.kind == wordcraft_proof::IssueKind::Spelling { wordcraft_proof::suggest_in(&word, 6, i.lang) } else { i.suggestions.clone() };
    json!({
        "start": pos_json(a), "end": pos_json(b), "text": word,
        "kind": if i.kind == wordcraft_proof::IssueKind::Spelling { "spelling" } else { "grammar" },
        "message": i.message, "suggestions": sugg, "lang": i.lang.code(),
    })
}

/// F7: select the next issue after the caret and return it with suggestions.
fn next_issue(s: &mut Session, _: &Value) -> CmdResult {
    let caret = s.sel.ordered().1;
    let story = caret.story;
    let paths = s.doc.para_paths(story);
    let start = paths.iter().position(|p| *p == caret.path).unwrap_or(0);
    for k in 0..paths.len() {
        let Some(path) = paths.get((start + k) % paths.len().max(1)) else { break };
        for (a, b, i) in para_issues(s, story, path) {
            if k == 0 && a.off < caret.off && start + k < paths.len() && paths.len() > 1 {
                continue;
            }
            let r = issue_json(s, &a, &b, &i);
            s.sel = Selection { anchor: a, focus: b };
            return Ok(r);
        }
    }
    s.status = "The spelling and grammar check is complete.".into();
    Ok(json!({"done": true}))
}

fn all_issues(s: &mut Session, _: &Value) -> CmdResult {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        for (a, b, i) in para_issues(s, StoryRef::Body, &path) {
            out.push(issue_json(s, &a, &b, &i));
            if out.len() >= 500 {
                return Ok(Value::Array(out));
            }
        }
    }
    Ok(Value::Array(out))
}

fn suggestions(s: &mut Session, v: &Value) -> CmdResult {
    let at = v.get("pos").and_then(super::parse_pos).unwrap_or_else(|| s.sel.focus.clone());
    match issue_at(s, &at) {
        Some((a, b, i)) => Ok(issue_json(s, &a, &b, &i)),
        None => Ok(Value::Null),
    }
}

/// A tiny built-in thesaurus of common words (our own list).
fn thesaurus(s: &mut Session, v: &Value) -> CmdResult {
    const T: &[(&str, &[&str])] = &[
        ("good", &["fine", "excellent", "decent", "solid", "worthy"]),
        ("bad", &["poor", "inferior", "faulty", "unpleasant"]),
        ("big", &["large", "huge", "sizable", "vast", "major"]),
        ("small", &["little", "tiny", "compact", "minor", "modest"]),
        ("fast", &["quick", "rapid", "swift", "speedy"]),
        ("slow", &["unhurried", "gradual", "leisurely", "sluggish"]),
        ("happy", &["glad", "cheerful", "content", "joyful", "pleased"]),
        ("sad", &["unhappy", "sorrowful", "downcast", "gloomy"]),
        ("important", &["significant", "key", "vital", "essential", "major"]),
        ("show", &["display", "reveal", "present", "demonstrate"]),
        ("make", &["create", "build", "produce", "craft", "form"]),
        ("use", &["employ", "apply", "utilize", "operate"]),
        ("help", &["assist", "aid", "support", "serve"]),
        ("start", &["begin", "launch", "open", "initiate"]),
        ("end", &["finish", "close", "conclude", "stop"]),
        ("say", &["state", "tell", "remark", "mention"]),
        ("think", &["believe", "consider", "reckon", "suppose"]),
        ("new", &["fresh", "novel", "recent", "modern"]),
        ("old", &["aged", "former", "ancient", "vintage"]),
        ("beautiful", &["lovely", "attractive", "gorgeous", "stunning"]),
        ("easy", &["simple", "effortless", "straightforward"]),
        ("hard", &["difficult", "tough", "demanding", "firm"]),
        ("work", &["labor", "effort", "job", "task", "operate"]),
        ("idea", &["concept", "notion", "thought", "plan"]),
        ("change", &["alter", "modify", "adjust", "shift"]),
        ("great", &["excellent", "superb", "terrific", "considerable"]),
        ("very", &["extremely", "highly", "truly", "really"]),
        ("shared", &["common", "joint", "communal", "mutual"]),
        ("community", &["group", "society", "collective", "circle"]),
    ];
    let word = match p::str(v, "word") {
        Some(w) => w.to_lowercase(),
        None => {
            let f = s.sel.focus.clone();
            let p = s.doc.para_at(&f).ok_or_else(|| CmdError::Failed("no word".into()))?;
            let (a, b) = p.word_at(f.off);
            p.text.get(a..b).unwrap_or("").trim().to_lowercase()
        }
    };
    let syn = T.iter().find(|(w, _)| *w == word).map(|(_, l)| l.to_vec()).unwrap_or_default();
    Ok(json!({"word": word, "synonyms": syn}))
}

/// Track Changes Options: change the fields given, return them all. A per-user preference.
fn tracking_options(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_doc::Rgb;
    use wordcraft_layout::display::{BalloonMode, ChangeBar, CommentDisplay, DeleteMark, InsertMark};
    let mut m = s.prefs.markup.clone();
    let color = |k: &str, auto: &str, cur: Option<Rgb>| -> Result<Option<Rgb>, CmdError> {
        match p::str(v, k) {
            None => Ok(cur),
            Some(c) if c == auto => Ok(None),
            Some(c) => Rgb::parse(c).map(Some).ok_or_else(|| CmdError::Params(format!("`{k}`: \"{auto}\" or RRGGBB"))),
        }
    };
    let pick = |k: &str| p::str(v, k);
    for (k, f) in [
        ("comments", &mut m.comments),
        ("ink", &mut m.ink),
        ("insertionsDeletions", &mut m.insertions_deletions),
        ("formatting", &mut m.formatting),
        ("trackFormatting", &mut m.track_formatting),
    ] {
        if let Some(b) = p::bool(v, k) {
            *f = b;
        }
    }
    if let Some(x) = pick("balloons") {
        m.balloons = BalloonMode::parse(x).ok_or_else(|| CmdError::Params("`balloons`: revisions|inline|commentsAndFormatting".into()))?;
    }
    if let Some(x) = pick("commentDisplay") {
        m.comment_display = CommentDisplay::parse(x).ok_or_else(|| CmdError::Params("`commentDisplay`: margin|selected".into()))?;
    }
    if let Some(x) = pick("insertMark") {
        m.insert_mark = InsertMark::parse(x).ok_or_else(|| CmdError::Params("unknown `insertMark`".into()))?;
    }
    if let Some(x) = pick("deleteMark") {
        m.delete_mark = DeleteMark::parse(x).ok_or_else(|| CmdError::Params("unknown `deleteMark`".into()))?;
    }
    if let Some(x) = pick("moveFromMark") {
        m.move_from_mark = DeleteMark::parse(x).ok_or_else(|| CmdError::Params("unknown `moveFromMark`".into()))?;
    }
    if let Some(x) = pick("moveToMark") {
        m.move_to_mark = InsertMark::parse(x).ok_or_else(|| CmdError::Params("unknown `moveToMark`".into()))?;
    }
    if let Some(x) = pick("changedLines") {
        m.changed_lines = ChangeBar::parse(x).ok_or_else(|| CmdError::Params("unknown `changedLines`".into()))?;
    }
    m.insert_color = color("insertColor", "byAuthor", m.insert_color)?;
    m.delete_color = color("deleteColor", "byAuthor", m.delete_color)?;
    m.changed_lines_color = color("changedLinesColor", "auto", m.changed_lines_color)?;
    m.move_from_color = color("moveFromColor", "byAuthor", m.move_from_color)?;
    m.move_to_color = color("moveToColor", "byAuthor", m.move_to_color)?;
    // Track moves is the document's (`w:doNotTrackMoves`), so changing it is an undo step; the
    // command is pure, so it records the step itself, once every parameter has been accepted.
    // The other options are the user's own, so the command stays available under Restrict
    // Editing; this one edits the document, so it passes the same check as an edit and isn't
    // limited to an exception.
    if let Some(b) = p::bool(v, "trackMoves")
        && b != s.doc.settings.track_moves
    {
        if super::protect::check_before(s, "review.trackingOptions", &json!({}))? {
            return Err(CmdError::Disabled(
                "review.trackingOptions: the document is protected; stop protection to change whether moves are tracked".into(),
            ));
        }
        s.checkpoint("Track Changes Options");
        s.doc.settings.track_moves = b;
        s.touch();
    }
    if m != s.prefs.markup {
        s.prefs.markup = m;
        // Hidden deletions leave the layout.
        s.relayout();
    }
    let mut out = tracking_json(&s.prefs.markup);
    out["trackMoves"] = json!(s.doc.settings.track_moves);
    Ok(out)
}

/// The Track Changes Options as `review.trackingOptions` takes them.
pub fn tracking_json(m: &wordcraft_layout::display::MarkupOptions) -> Value {
    let hex = |c: Option<wordcraft_doc::Rgb>, auto: &str| c.map(|c| c.hex()).unwrap_or_else(|| auto.to_string());
    json!({
        "comments": m.comments,
        "ink": m.ink,
        "insertionsDeletions": m.insertions_deletions,
        "formatting": m.formatting,
        "balloons": m.balloons.name(),
        "commentDisplay": m.comment_display.name(),
        "insertMark": m.insert_mark.name(),
        "insertColor": hex(m.insert_color, "byAuthor"),
        "deleteMark": m.delete_mark.name(),
        "deleteColor": hex(m.delete_color, "byAuthor"),
        "changedLines": m.changed_lines.name(),
        "changedLinesColor": hex(m.changed_lines_color, "auto"),
        "trackFormatting": m.track_formatting,
        "moveFromMark": m.move_from_mark.name(),
        "moveFromColor": hex(m.move_from_color, "byAuthor"),
        "moveToMark": m.move_to_mark.name(),
        "moveToColor": hex(m.move_to_color, "byAuthor"),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::Document;
    use wordcraft_layout::display::{DisplayOptions, Draw, page_display};

    use crate::Session;

    /// What the first page draws with the session's Track Changes Options.
    fn draws(s: &mut Session) -> (Vec<Draw>, f32) {
        let l = s.layout();
        let page = &l.pages[0];
        let opts = DisplayOptions { revisions: s.prefs.markup.clone(), ..Default::default() };
        (page_display(&s.doc, page, &opts), page.body.x)
    }

    fn glyph_colors(d: &[Draw]) -> Vec<(wordcraft_doc::Rgb, bool)> {
        d.iter().filter_map(|x| if let Draw::Glyphs { color, synth_bold, .. } = x { Some((*color, *synth_bold)) } else { None }).collect()
    }

    fn lines(d: &[Draw]) -> Vec<(f32, f32, f32, f32)> {
        d.iter().filter_map(|x| if let Draw::Line { x0, y0, x1, y1, .. } = x { Some((*x0, *y0, *x1, *y1)) } else { None }).collect()
    }

    /// #328: Track Changes Options change how an insertion and a deletion are drawn, the bar
    /// beside changed lines, and whether deletions take room.
    #[test]
    fn tracking_options_change_how_revisions_are_drawn() {
        let mut s = Session::new(Document::new());
        // No proofing squiggles among the lines.
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "keep gone"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("caret.docStart", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "new "})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 9}, "focus": {"block": 0, "off": 13}})).unwrap();
        s.run("text.delete", &json!({})).unwrap();

        // Defaults: underlined insertion and struck deletion in the author's colour, a bar left of
        // the text.
        let (d, body_x) = draws(&mut s);
        let bars: Vec<_> = lines(&d).into_iter().filter(|(x0, _, x1, _)| x0 == x1 && *x0 < body_x).collect();
        assert_eq!(bars.len(), 1, "one changed line, one bar in the left margin");
        let horizontal = lines(&d).into_iter().filter(|(_, y0, _, y1)| y0 == y1).count();
        assert_eq!(horizontal, 2, "an underline and a strikethrough: {:?}", lines(&d));
        let author = glyph_colors(&d).iter().filter(|(c, _)| *c != wordcraft_doc::Rgb::BLACK).count();
        assert!(author >= 2, "insertion and deletion in the author's colour");

        // Bold, fixed-colour insertions; hidden deletions; no bars.
        let r = s
            .run(
                "review.trackingOptions",
                &json!({"insertMark": "bold", "insertColor": "00AA00", "deleteMark": "hidden", "changedLines": "none", "balloons": "inline"}),
            )
            .unwrap();
        assert_eq!((r["insertMark"].as_str(), r["deleteColor"].as_str(), r["balloons"].as_str()), (Some("bold"), Some("byAuthor"), Some("inline")));
        let (d, _) = draws(&mut s);
        assert!(lines(&d).is_empty(), "no underline, no strikethrough, no bar: {:?}", lines(&d));
        assert!(glyph_colors(&d).contains(&(wordcraft_doc::Rgb(0, 0xAA, 0), true)), "a bold green insertion");
        let text: String = d.iter().filter_map(|x| if let Draw::Glyphs { text, .. } = x { Some(text.as_str()) } else { None }).collect();
        assert!(!text.contains("gone"), "the deletion takes no room: {text:?}");

        // Unknown choices are refused; reading changes nothing.
        assert!(s.run("review.trackingOptions", &json!({"deleteMark": "sparkles"})).is_err());
        assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["insertColor"], "00AA00");
    }

    /// #1055: comments show in the margin as in Word unless the option says otherwise.
    #[test]
    fn comment_display_defaults_to_margin_and_is_validated() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["commentDisplay"], "margin");
        assert_eq!(s.run("review.trackingOptions", &json!({"commentDisplay": "selected"})).unwrap()["commentDisplay"], "selected");
        assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["commentDisplay"], "selected");
        assert!(s.run("review.trackingOptions", &json!({"commentDisplay": "never"})).is_err());
        assert_eq!(s.run("review.trackingOptions", &json!({"commentDisplay": "margin"})).unwrap()["commentDisplay"], "margin");
    }

    /// Gap #5 (Track Changes Options › Moves): moved text is drawn with the Moved from / Moved
    /// to marks and colours, by default green with a double strikethrough where it was and a
    /// double underline where it went.
    #[test]
    fn move_marks_and_colours_change_how_moved_text_is_drawn() {
        use wordcraft_doc::Rgb;
        use wordcraft_layout::display::{MOVE_COLOR, Stroke};
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "First one.\nSecond one.\nThird one."})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}})).unwrap();
        s.run("edit.cut", &json!({})).unwrap();
        s.run("caret.set", &json!({"pos": {"block": 0, "off": 0}})).unwrap();
        s.run("edit.paste", &json!({})).unwrap();
        // Horizontal lines (underlines, strikethroughs) with their colour and stroke.
        let strokes = |d: &[Draw]| -> Vec<(Rgb, Stroke)> {
            d.iter()
                .filter_map(|x| if let Draw::Line { y0, y1, color, stroke, .. } = x { (y0 == y1).then_some((*color, *stroke)) } else { None })
                .collect()
        };

        let r = s.run("review.trackingOptions", &json!({})).unwrap();
        assert_eq!(
            (r["moveFromMark"].as_str(), r["moveFromColor"].as_str(), r["moveToMark"].as_str(), r["moveToColor"].as_str()),
            (Some("doubleStrikethrough"), Some(MOVE_COLOR.hex().as_str()), Some("doubleUnderline"), Some(MOVE_COLOR.hex().as_str())),
        );
        let (d, _) = draws(&mut s);
        let st = strokes(&d);
        assert_eq!(
            st.iter().filter(|(c, k)| *c == MOVE_COLOR && *k == Stroke::Double).count(),
            2,
            "green double underline and strikethrough: {st:?}"
        );
        assert!(glyph_colors(&d).contains(&(MOVE_COLOR, false)));

        // Red single strikethrough where it was; bold blue where it went.
        let r = s
            .run(
                "review.trackingOptions",
                &json!({"moveFromMark": "strikethrough", "moveFromColor": "FF0000", "moveToMark": "bold", "moveToColor": "0000FF"}),
            )
            .unwrap();
        assert_eq!((r["moveFromMark"].as_str(), r["moveToColor"].as_str()), (Some("strikethrough"), Some("0000FF")));
        let (d, _) = draws(&mut s);
        let st = strokes(&d);
        assert_eq!(st, vec![(Rgb(0xFF, 0, 0), Stroke::Solid)], "only a red strikethrough is left");
        let g = glyph_colors(&d);
        assert!(g.contains(&(Rgb(0, 0, 0xFF), true)) && g.contains(&(Rgb(0xFF, 0, 0), false)), "{g:?}");
        assert!(!g.iter().any(|(c, _)| *c == MOVE_COLOR), "{g:?}");

        // By author: the insertion and deletion colour of the author.
        s.run("review.trackingOptions", &json!({"moveToColor": "byAuthor"})).unwrap();
        let (d, _) = draws(&mut s);
        let author = wordcraft_layout::display::revision_color(0);
        assert!(glyph_colors(&d).contains(&(author, true)), "{:?}", glyph_colors(&d));

        // Unknown choices are refused.
        assert!(s.run("review.trackingOptions", &json!({"moveToMark": "sparkles"})).is_err());
        assert!(s.run("review.trackingOptions", &json!({"moveFromColor": "green"})).is_err());
    }

    /// Three paragraphs; with Track Changes on, the second cut and pasted before the first, then
    /// the Track Changes Options `opts` applied.
    fn moved_with(opts: serde_json::Value) -> Session {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "First one.\nSecond one.\nThird one."})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}})).unwrap();
        s.run("edit.cut", &json!({})).unwrap();
        s.run("caret.set", &json!({"pos": {"block": 0, "off": 0}})).unwrap();
        s.run("edit.paste", &json!({})).unwrap();
        s.run("review.trackingOptions", &opts).unwrap();
        s
    }

    /// The caret at body paragraph `block`, offset `off`: (x, top).
    fn caret_at(s: &mut Session, block: usize, off: usize) -> (f32, f32) {
        let c = s.layout().caret(&wordcraft_doc::Pos::body(block, off)).unwrap();
        (c.x, c.top)
    }

    /// Review of gap #5: Moved from = Hidden takes the moved-away paragraph out of the layout,
    /// as Hidden does for deletions: the text below closes up as if the move were accepted, the
    /// caret can't land inside it, and nothing of it is drawn, not even its ¶.
    #[test]
    fn hidden_moved_from_text_takes_no_room() {
        let mut accepted = moved_with(json!({}));
        accepted.run("review.acceptAll", &json!({})).unwrap();
        let third_accepted = caret_at(&mut accepted, 2, 0).1;

        let mut s = moved_with(json!({"moveFromMark": "hidden"}));
        assert_eq!(s.doc.body.len(), 4, "the moved-away paragraph is still in the document");
        assert!((caret_at(&mut s, 3, 0).1 - third_accepted).abs() < 0.01, "no blank line where it was");
        // Inside it is the same place as its start and as the paragraph after it.
        assert_eq!(caret_at(&mut s, 2, 5), caret_at(&mut s, 2, 0));
        assert_eq!(caret_at(&mut s, 2, 0), caret_at(&mut s, 3, 0));
        let (d, _) = draws(&mut s);
        assert_eq!(glyph_text(&d).matches("Second").count(), 1, "{:?}", glyph_text(&d));
        // Formatting marks shown: one ¶ per visible paragraph.
        let l = s.layout();
        let opts = DisplayOptions { marks: true, revisions: s.prefs.markup.clone(), ..Default::default() };
        let d = page_display(&s.doc, &l.pages[0], &opts);
        assert_eq!(d.iter().filter(|x| matches!(x, Draw::Mark { ch: '¶', .. })).count(), 3);
    }

    /// Each "Second" run drawn, in page order (the moved-to paragraph first, then the moved-from
    /// one): (colour, bold, italic).
    fn seconds(d: &[Draw]) -> Vec<(wordcraft_doc::Rgb, bool, bool)> {
        d.iter()
            .filter_map(|x| match x {
                Draw::Glyphs { text, color, synth_bold, synth_italic, .. } if text.starts_with("Second") => {
                    Some((*color, *synth_bold, *synth_italic))
                }
                _ => None,
            })
            .collect()
    }

    /// Horizontal lines (underlines and strikethroughs): their strokes.
    fn h_strokes(d: &[Draw]) -> Vec<wordcraft_layout::display::Stroke> {
        d.iter().filter_map(|x| if let Draw::Line { y0, y1, stroke, .. } = x { (y0 == y1).then_some(*stroke) } else { None }).collect()
    }

    /// Change bars: vertical lines left of the body.
    fn bars(d: &[Draw], body_x: f32) -> usize {
        lines(d).into_iter().filter(|(x0, _, x1, _)| x0 == x1 && *x0 < body_x).count()
    }

    /// Gap #5, Word for Mac 16.113.4 (2026-10-15): Moved to = (none) draws the moved text as
    /// plain text, no mark and no colour, although the colour stays set; the change bar stays.
    #[test]
    fn moved_to_none_is_plain_text_with_its_change_bar() {
        use wordcraft_layout::display::{MOVE_COLOR, Stroke};
        let mut s = moved_with(json!({}));
        let (d, body_x) = draws(&mut s);
        let default_bars = bars(&d, body_x);
        assert!(default_bars > 0);
        let mut s = moved_with(json!({"moveToMark": "none"}));
        assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["moveToColor"], MOVE_COLOR.hex());
        let (d, body_x) = draws(&mut s);
        let sec = seconds(&d);
        assert_eq!(sec.first().map(|c| c.0), Some(wordcraft_doc::Rgb::BLACK), "{sec:?}");
        assert_eq!(sec.get(1).map(|c| c.0), Some(MOVE_COLOR), "the moved-from end keeps its colour");
        assert_eq!(h_strokes(&d), vec![Stroke::Double], "only the moved-from strikethrough");
        assert_eq!(bars(&d, body_x), default_bars, "the change bars stay");
    }

    /// Gap #5, Word for Mac 16.113.4: Deletions and Moved from offer (none), Bold, Italic and
    /// Double underline too. Moved from (none) is plain text, as Moved to (none); an ordinary
    /// deletion marked (none) keeps its colour.
    #[test]
    fn deletion_marks_none_bold_italic_and_double_underline() {
        use wordcraft_layout::display::{MOVE_COLOR, Stroke};
        let plain = json!({"moveToMark": "none", "moveToColor": "byAuthor"});
        let with = |extra: serde_json::Value| {
            let mut o = plain.clone();
            o.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            moved_with(o)
        };
        let mut s = with(json!({"moveFromMark": "none"}));
        let (d, _) = draws(&mut s);
        assert_eq!(seconds(&d).get(1).map(|c| c.0), Some(wordcraft_doc::Rgb::BLACK), "{:?}", seconds(&d));
        assert!(h_strokes(&d).is_empty(), "{:?}", h_strokes(&d));
        let mut s = with(json!({"moveFromMark": "bold"}));
        let (d, _) = draws(&mut s);
        assert_eq!(seconds(&d).get(1), Some(&(MOVE_COLOR, true, false)));
        assert!(h_strokes(&d).is_empty());
        let mut s = with(json!({"moveFromMark": "italic"}));
        let (d, _) = draws(&mut s);
        assert_eq!(seconds(&d).get(1), Some(&(MOVE_COLOR, false, true)));
        let mut s = with(json!({"moveFromMark": "doubleUnderline"}));
        let (d, _) = draws(&mut s);
        assert_eq!(h_strokes(&d), vec![Stroke::Double]);
        let r = s.run("review.trackingOptions", &json!({"deleteMark": "none"})).unwrap();
        assert_eq!((r["deleteMark"].as_str(), r["moveFromMark"].as_str()), (Some("none"), Some("doubleUnderline")));

        // An ordinary deletion marked (none): no line, the author's colour.
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "keep gone"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 9}})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        s.run("review.trackingOptions", &json!({"deleteMark": "none"})).unwrap();
        let (d, _) = draws(&mut s);
        assert!(h_strokes(&d).is_empty());
        let gone = d.iter().find_map(|x| if let Draw::Glyphs { text, color, .. } = x { text.contains("gone").then_some(*color) } else { None });
        assert_eq!(gone, Some(wordcraft_layout::display::revision_color(0)));
    }

    /// Review of gap #5: Deletions = Hidden hides deletions, not moved-away text, which follows
    /// Moved from (left at its default: shown, green, double strikethrough) and keeps its room,
    /// as in Word for Mac 16.113.4 (user-checked 2026-10-15).
    #[test]
    fn hidden_deletions_leave_moved_from_text_shown() {
        use wordcraft_layout::display::{MOVE_COLOR, Stroke};
        let mut accepted = moved_with(json!({}));
        accepted.run("review.acceptAll", &json!({})).unwrap();
        let third_accepted = caret_at(&mut accepted, 2, 0).1;
        let mut s = moved_with(json!({"deleteMark": "hidden"}));
        assert!(caret_at(&mut s, 3, 0).1 > third_accepted + 1.0, "the moved-away paragraph keeps its line");
        let (d, _) = draws(&mut s);
        assert_eq!(glyph_text(&d).matches("Second").count(), 2, "{:?}", glyph_text(&d));
        let struck = d.iter().any(|x| matches!(x, Draw::Line { color, stroke: Stroke::Double, y0, y1, .. } if *color == MOVE_COLOR && y0 == y1));
        assert!(struck, "green double lines");
    }

    /// Gap #5: Track moves is a document setting (`w:doNotTrackMoves`): turning it off is one
    /// undo step, and setting it to what it is adds none.
    #[test]
    fn track_moves_is_a_document_setting_with_one_undo_step() {
        let mut s = Session::new(Document::new());
        assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["trackMoves"], true);
        assert!(s.doc.settings.track_moves);
        assert!(!s.can_undo());
        assert_eq!(s.run("review.trackingOptions", &json!({"trackMoves": true})).unwrap()["trackMoves"], true);
        assert!(!s.can_undo(), "no change, no undo step");
        assert_eq!(s.run("review.trackingOptions", &json!({"trackMoves": false})).unwrap()["trackMoves"], false);
        assert!(!s.doc.settings.track_moves && s.dirty);
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.settings.track_moves);
        assert!(!s.can_undo());
        // A refused parameter changes nothing.
        assert!(s.run("review.trackingOptions", &json!({"trackMoves": false, "moveToMark": "sparkles"})).is_err());
        assert!(s.doc.settings.track_moves);
        // Documents saved before the setting existed keep tracking moves.
        let old: wordcraft_doc::Settings = serde_json::from_value(json!({"trackChanges": true})).unwrap();
        assert!(old.track_moves);
    }

    /// #1055: the caret's own comment is the one whose range holds it, not any in its paragraph.
    #[test]
    fn comment_range_at_caret_needs_the_caret_inside_the_range() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("one two three"));
        s.run("select.text", &json!({"text": "two"})).unwrap();
        let id = s.run("review.newComment", &json!({"text": "note"})).unwrap()["id"].as_u64().unwrap() as u32;
        s.run("select.text", &json!({"text": "two"})).unwrap();
        s.run("caret.left", &json!({})).unwrap();
        assert_eq!(super::comment_range_at_caret(&s), Some(id), "inside the commented text");
        s.run("select.text", &json!({"text": "three"})).unwrap();
        assert_eq!(super::comment_range_at_caret(&s), None, "elsewhere in the paragraph");
        assert_eq!(super::comment_at_caret(&s), Some(id), "the older lookup still falls back to the paragraph's comment");
    }

    fn glyph_text(d: &[Draw]) -> String {
        d.iter().filter_map(|x| if let Draw::Glyphs { text, .. } = x { Some(text.as_str()) } else { None }).collect()
    }

    /// Show revisions in balloons: a tracked deletion leaves the text for a balloon at its place
    /// and keeps its change bar; it comes back inline in the other modes and wherever no balloons
    /// are shown (here the Comments pane, which changes the view without a relayout call).
    #[test]
    fn revisions_in_balloons_take_deletions_out_of_the_text() {
        use super::{DeletionBalloon, deletion_balloons};
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.author = "Ana".into();
        s.run("document.setText", &json!({"text": "keep gone stay"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 10}})).unwrap();
        s.run("text.delete", &json!({})).unwrap();

        // The default (only comments and formatting in balloons): struck through inline.
        assert!(!s.revisions_in_balloons());
        let (d, _) = draws(&mut s);
        assert!(glyph_text(&d).contains("gone"), "{:?}", glyph_text(&d));

        s.run("review.trackingOptions", &json!({"balloons": "revisions"})).unwrap();
        assert!(s.revisions_in_balloons());
        let (d, body_x) = draws(&mut s);
        assert!(!glyph_text(&d).contains("gone"), "the deletion left the text: {:?}", glyph_text(&d));
        let horizontal = lines(&d).into_iter().filter(|(_, y0, _, y1)| y0 == y1).count();
        assert_eq!(horizontal, 0, "no strikethrough left behind: {:?}", lines(&d));
        assert_eq!(lines(&d).into_iter().filter(|(x0, _, x1, _)| x0 == x1 && *x0 < body_x).count(), 1, "the changed line keeps its bar");
        let at = wordcraft_doc::Pos::body(0, 5);
        assert_eq!(deletion_balloons(&s), vec![DeletionBalloon { at: at.clone(), author: "Ana".into(), text: "gone ".into() }]);
        // The balloon's anchor is where the deletion was: between "keep " and "stay".
        let l = s.layout();
        let (anchor, stay) = (l.caret(&at).unwrap().x, l.caret(&wordcraft_doc::Pos::body(0, 10)).unwrap().x);
        assert!((anchor - stay).abs() < 0.01, "the deletion takes no room: {anchor} vs {stay}");

        // No balloons beside the page while the Comments pane lists comments: inline again.
        s.run("view.commentsPane", &json!({"value": true})).unwrap();
        assert!(!s.revisions_in_balloons());
        assert!(glyph_text(&draws(&mut s).0).contains("gone"));
        s.run("view.commentsPane", &json!({"value": false})).unwrap();
        assert!(!glyph_text(&draws(&mut s).0).contains("gone"));
        s.run("review.trackingOptions", &json!({"balloons": "inline"})).unwrap();
        assert!(!s.revisions_in_balloons());
        assert!(glyph_text(&draws(&mut s).0).contains("gone"));
    }

    /// Deleted text running on across runs, separate deletions and a paragraph mark is one balloon
    /// per author; a long deletion shows its start.
    #[test]
    fn deletion_balloons_join_runs_and_paragraph_marks_and_stay_short() {
        use super::{DELETION_TEXT_CHARS, deletion_balloons};
        let mut s = Session::new(Document::new());
        s.author = "Ana".into();
        s.run("document.setText", &json!({"text": "one two\nthree four"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        // "two", then the paragraph mark and "three" in a second deletion: one balloon.
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 4}, "focus": {"block": 0, "off": 7}})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 7}, "focus": {"block": 1, "off": 5}})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        // Bo's deletion right after Ana's is a balloon of its own.
        s.author = "Bo".into();
        s.run("select.range", &json!({"anchor": {"block": 1, "off": 5}, "focus": {"block": 1, "off": 10}})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        let b = deletion_balloons(&s);
        let got: Vec<(&str, &str)> = b.iter().map(|d| (d.author.as_str(), d.text.as_str())).collect();
        assert_eq!(got, vec![("Ana", "two¶three"), ("Bo", " four")]);
        assert_eq!(b[0].at, wordcraft_doc::Pos::body(0, 4));

        // A deletion far longer than a balloon shows: its first characters and an ellipsis.
        let mut s = Session::new(Document::new());
        let long = "word ".repeat(400);
        s.run("document.setText", &json!({"text": long})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        let b = deletion_balloons(&s);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text.chars().count(), DELETION_TEXT_CHARS + 1);
        assert!(b[0].text.starts_with("word word") && b[0].text.ends_with('…'));
    }

    /// #404: the source of a move is a deletion revision with a move name. With revisions in
    /// balloons it leaves the text like a deletion but has only its "Moved from" balloon, not a
    /// "Deleted: …" one as well; a plain deletion right after it is a balloon of its own.
    #[test]
    fn a_moves_source_gets_no_deletion_balloon() {
        use super::{DeletionBalloon, deletion_balloons, has_deletions, move_changes};
        use wordcraft_doc::Pos;
        let mut s = Session::new(Document::new());
        s.run("document.setText", &json!({"text": "First one.\nSecond one.\nThird one."})).unwrap();
        s.run("file.setAuthor", &json!({"name": "Ana"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        // The second paragraph, with its mark, cut and pasted before the first: a move.
        s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}})).unwrap();
        s.run("edit.cut", &json!({})).unwrap();
        s.run("caret.set", &json!({"pos": {"block": 0, "off": 0}})).unwrap();
        s.run("edit.paste", &json!({})).unwrap();
        s.run("review.trackingOptions", &json!({"balloons": "revisions"})).unwrap();
        assert!(s.revisions_in_balloons());

        let ends: Vec<(Pos, bool)> = move_changes(&s).into_iter().map(|(at, _, from)| (at, from)).collect();
        assert_eq!(ends, vec![(Pos::body(0, 0), false), (Pos::body(2, 0), true)]);
        assert!(deletion_balloons(&s).is_empty(), "the move's source has its \"Moved from\" balloon only: {:?}", deletion_balloons(&s));
        assert!(!has_deletions(&s.doc), "a move alone needs no deletion balloons");
        assert_eq!(glyph_text(&draws(&mut s).0), "Second one.First one.Third one.", "the moved-away text left the text");

        // "Third " deleted right where the moved-away paragraph ends: not joined to it.
        s.run("select.range", &json!({"anchor": {"block": 3, "off": 0}, "focus": {"block": 3, "off": 6}})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        assert!(has_deletions(&s.doc));
        assert_eq!(deletion_balloons(&s), vec![DeletionBalloon { at: Pos::body(3, 0), author: "Ana".into(), text: "Third ".into() }]);
        assert_eq!(move_changes(&s).len(), 2, "the move keeps both its balloons");
    }

    /// Revisions in balloons take deletions out of the body only, where their balloons are:
    /// headers, footers, footnotes, endnotes and text boxes get no balloons, so their deletions
    /// stay in the text, struck through, as in the other modes.
    #[test]
    fn deletions_outside_the_body_stay_inline_when_revisions_go_in_balloons() {
        use super::{DeletionBalloon, deletion_balloons};
        use wordcraft_doc::{Path, Pos, StoryRef};
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.author = "Ana".into();
        s.run("document.setText", &json!({"text": "Body zeta keep"})).unwrap();
        let part = |v: serde_json::Value, key: &str| StoryRef::Part(v[key].as_u64().unwrap() as u32);
        let header = part(s.run("insert.header", &json!({"text": "Head alpha"})).unwrap(), "story");
        s.run("insert.closeHeader", &json!({})).unwrap();
        let footer = part(s.run("insert.footer", &json!({"text": "Foot beta"})).unwrap(), "story");
        s.run("insert.closeHeader", &json!({})).unwrap();
        let in_body = json!({"pos": {"block": 0, "off": 4}});
        s.run("caret.set", &in_body).unwrap();
        let footnote = part(s.run("references.footnote", &json!({"text": "Note gamma"})).unwrap(), "id");
        s.run("caret.set", &in_body).unwrap();
        let endnote = part(s.run("references.endnote", &json!({"text": "End delta"})).unwrap(), "id");
        s.run("caret.set", &in_body).unwrap();
        let text_box = part(s.run("insert.textBox", &json!({"text": "Box epsilon"})).unwrap(), "story");

        // With Track Changes on, a word deleted in each story.
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        let words = [(StoryRef::Body, "zeta"), (header, "alpha"), (footer, "beta"), (footnote, "gamma"), (endnote, "delta"), (text_box, "epsilon")];
        for (story, word) in words {
            let at = s.doc.para(story, &Path::top(0)).and_then(|p| p.text.find(word)).unwrap();
            let pos = |off: usize| json!({"story": story, "path": [0], "off": off});
            s.run("select.range", &json!({"anchor": pos(at), "focus": pos(at + word.len())})).unwrap();
            s.run("text.delete", &json!({})).unwrap();
            let p = s.doc.para(story, &Path::top(0)).unwrap();
            assert!(p.run_ranges().any(|(r, c)| c.del.is_some() && p.text.get(r) == Some(word)), "{word} is a tracked deletion");
        }
        let struck = |d: &[Draw]| lines(d).into_iter().filter(|(_, y0, _, y1)| y0 == y1).count();

        // Comments and formatting in balloons: every deletion struck through where it was.
        let (d, _) = draws(&mut s);
        let text = glyph_text(&d);
        for (_, word) in words {
            assert!(text.contains(word), "{word} in {text:?}");
        }
        let all_struck = struck(&d);

        // Revisions in balloons: only the body's deletion leaves the text, for its balloon.
        s.run("review.trackingOptions", &json!({"balloons": "revisions"})).unwrap();
        assert!(s.revisions_in_balloons());
        let (d, _) = draws(&mut s);
        let text = glyph_text(&d);
        assert!(!text.contains("zeta"), "the body's deletion left the text: {text:?}");
        for (_, word) in &words[1..] {
            assert!(text.contains(word), "{word} stays inline: {text:?}");
        }
        assert_eq!(struck(&d), all_struck - 1, "the other deletions stay struck through");
        // Its balloon is the only one: the notes' and text box's objects sit before "zeta".
        let at = s.doc.para(StoryRef::Body, &Path::top(0)).and_then(|p| p.text.find("zeta")).unwrap();
        assert_eq!(deletion_balloons(&s), vec![DeletionBalloon { at: Pos::body(0, at), author: "Ana".into(), text: "zeta".into() }]);
    }

    fn body_para(s: &Session) -> &wordcraft_doc::Paragraph {
        s.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap()
    }

    /// A comment inside a tracked insertion splits it into several runs; Accept and Reject at the
    /// caret resolve the whole insertion, not just the run around the caret.
    #[test]
    fn accept_and_reject_cover_a_change_split_by_a_comment() {
        for accept in [true, false] {
            let mut s = Session::new(Document::new());
            s.view.proofing = false;
            s.author = "Ana".into();
            s.run("review.trackChanges", &json!({"value": true})).unwrap();
            s.run("text.insert", &json!({"text": "hello world"})).unwrap();
            s.run("select.range", &json!({"anchor": {"block": 0, "off": 3}, "focus": {"block": 0, "off": 8}})).unwrap();
            s.run("review.newComment", &json!({"text": "note"})).unwrap();
            assert!(body_para(&s).runs.len() > 3, "the comment anchors split the insertion");
            s.run("select.range", &json!({"anchor": {"block": 0, "off": 1}, "focus": {"block": 0, "off": 1}})).unwrap();
            s.run(if accept { "review.accept" } else { "review.reject" }, &json!({})).unwrap();
            let p = body_para(&s);
            assert!(p.runs.iter().all(|r| r.props.ins.is_none()), "no tracked insertion is left");
            // The comment's anchors are objects in the text; read past them.
            let shown: String = p.text.chars().filter(|c| *c != wordcraft_doc::para::OBJ).collect();
            assert_eq!(shown.contains("hello"), accept, "{:?}", p.text);
        }
    }

    /// "one two three" with "new " typed at the start, "two " deleted and " end" typed at the end,
    /// under Track Changes: three changes.
    fn three_changes() -> Session {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.author = "Ana".into();
        s.run("document.setText", &json!({"text": "one two three"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("caret.docStart", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "new "})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 8}, "focus": {"block": 0, "off": 12}})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": " end"})).unwrap();
        s
    }

    fn change_spans(s: &mut Session) -> Vec<(String, u64, u64)> {
        let list = s.run("review.changes", &json!({})).unwrap();
        list.as_array()
            .unwrap()
            .iter()
            .map(|c| (c["kind"].as_str().unwrap().to_string(), c["start"]["off"].as_u64().unwrap(), c["end"]["off"].as_u64().unwrap()))
            .collect()
    }

    fn selected(s: &Session) -> (usize, usize) {
        let (a, b) = s.sel.ordered();
        (a.off, b.off)
    }

    /// Accept and Move to Next, Reject and Move to Next: the change at the caret is resolved and
    /// the next one is selected; without `next` the caret stays where the change was.
    #[test]
    fn accept_and_reject_can_move_to_the_next_change() {
        for (cmd, accept) in [("review.accept", true), ("review.reject", false)] {
            let mut s = three_changes();
            let before = change_spans(&mut s);
            assert_eq!(before.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["insert", "delete", "insert"], "{cmd}");
            s.run("caret.docStart", &json!({})).unwrap();
            s.run(cmd, &json!({"next": true})).unwrap();
            let after = change_spans(&mut s);
            assert_eq!(after.len(), 2, "{cmd}: the first insertion is resolved");
            assert_eq!(after[0].0, "delete", "{cmd}");
            let deletion = (after[0].1 as usize, after[0].2 as usize);
            assert_eq!(selected(&s), deletion, "{cmd}: the deletion is selected next");
            assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).starts_with("new "), accept, "{cmd}");

            // Without `next`, the caret stays at the resolved change.
            let mut s = three_changes();
            s.run("caret.docStart", &json!({})).unwrap();
            s.run(cmd, &json!({})).unwrap();
            assert_eq!(selected(&s), (0, 0), "{cmd}");
        }
    }

    /// With the caret on no change, Accept and Move to Next selects the next change without
    /// resolving it, as Accept and Next Change do.
    #[test]
    fn accept_and_move_to_next_off_a_change_selects_it_unresolved() {
        let mut s = three_changes();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 5}})).unwrap();
        s.run("review.accept", &json!({"next": true})).unwrap();
        let after = change_spans(&mut s);
        assert_eq!(after.len(), 3, "nothing is resolved");
        assert_eq!(selected(&s), (after[1].1 as usize, after[1].2 as usize), "the deletion after the caret is selected");
    }

    /// Reject and Move to Next on the only text of a centered paragraph: the paragraph's formatting
    /// change, now empty at the caret, is selected next, not the insertion after it.
    #[test]
    fn move_to_next_selects_a_change_left_at_the_caret() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.author = "Ana".into();
        s.run("document.setText", &json!({"text": "one\n\nthree"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 1, "off": 0}})).unwrap();
        s.run("para.align", &json!({"value": "center"})).unwrap();
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "!"})).unwrap();
        let before = change_spans(&mut s);
        assert_eq!(before.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["format", "insert", "insert"], "{before:?}");
        s.run("select.range", &json!({"anchor": {"block": 1, "off": 1}, "focus": {"block": 1, "off": 1}})).unwrap();
        s.run("review.reject", &json!({"next": true})).unwrap();
        let after = change_spans(&mut s);
        assert_eq!(after.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["format", "insert"], "{after:?}");
        let (a, b) = s.sel.ordered();
        let at = (wordcraft_doc::Path::top(1), 0);
        assert_eq!(((a.path.clone(), a.off), (b.path.clone(), b.off)), (at.clone(), at), "the centering is selected");
    }

    /// Accept All Changes and Stop Tracking, Reject All Changes and Stop Tracking: every change is
    /// resolved and Track Changes is off; plain Accept All and Reject All leave it on.
    #[test]
    fn accept_and_reject_all_can_stop_tracking() {
        for cmd in ["review.acceptAll", "review.rejectAll"] {
            let mut s = three_changes();
            s.run(cmd, &json!({"stopTracking": true})).unwrap();
            assert!(change_spans(&mut s).is_empty(), "{cmd}");
            assert!(!s.doc.settings.track_changes, "{cmd}: Track Changes is off");
            s.run("edit.undo", &json!({})).unwrap();
            assert!(s.doc.settings.track_changes, "{cmd}: one undo step brings tracking and the changes back");
            assert_eq!(change_spans(&mut s).len(), 3, "{cmd}");

            let mut s = three_changes();
            s.run(cmd, &json!({})).unwrap();
            assert!(change_spans(&mut s).is_empty(), "{cmd}");
            assert!(s.doc.settings.track_changes, "{cmd}: Track Changes stays on");
        }
    }

    /// "plain words here" with "words" made bold, then 14 pt, and the paragraph centred, under
    /// Track Changes by Ana.
    fn formatted_while_tracking() -> Session {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.author = "Ana".into();
        s.run("document.setText", &json!({"text": "plain words here"})).unwrap();
        // Untracked formatting is no revision.
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 12}, "focus": {"block": 0, "off": 16}})).unwrap();
        s.run("format.italic", &json!({})).unwrap();
        assert!(body_para(&s).runs.iter().all(|r| r.props.fmt_change.is_none()));
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 11}})).unwrap();
        s.run("format.bold", &json!({})).unwrap();
        s.run("format.size", &json!({"size": 14})).unwrap();
        s.run("para.alignCenter", &json!({})).unwrap();
        s
    }

    /// #41: formatting with Track Changes on records the formatting before the author's first
    /// change; it's listed in the Reviewing Pane and marked by a bar beside the line.
    #[test]
    fn formatting_is_tracked_while_tracking_changes() {
        let mut s = formatted_while_tracking();
        let p = body_para(&s);
        let ch = p.props_of_char(6).fmt_change.as_deref().expect("a formatting change on \"words\"");
        assert_eq!((ch.old.bold, ch.old.size), (None, None), "the formatting before the first change is kept");
        let rev = &s.doc.revisions[ch.rev as usize];
        assert_eq!((rev.kind, rev.author.as_str()), (wordcraft_doc::RevisionKind::Format, "Ana"));
        assert!(p.props_of_char(0).fmt_change.is_none() && p.props_of_char(13).fmt_change.is_none());
        assert_eq!(p.props.fmt_change.as_deref().map(|c| c.old.align), Some(None));

        let list = s.run("review.changes", &json!({})).unwrap();
        let desc: Vec<&str> = list.as_array().unwrap().iter().filter(|c| c["kind"] == "format").filter_map(|c| c["description"].as_str()).collect();
        assert_eq!(desc, ["Formatted: Align: center", "Formatted: Bold, Bold cs, Size: 14, Size cs: 14"], "{list:#}");

        let (d, body_x) = draws(&mut s);
        let bars = lines(&d).into_iter().filter(|(x0, _, x1, _)| x0 == x1 && *x0 < body_x).count();
        assert_eq!(bars, 1, "one changed line, one bar");

        // Formatting changed back is no change any more.
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 11}})).unwrap();
        s.run("format.set", &json!({"props": {"bold": false}})).unwrap();
        s.run("format.size", &json!({"size": 11})).unwrap();
        assert!(body_para(&s).props_of_char(6).fmt_change.is_some(), "explicit Not Bold and 11 pt differ from inherited");
    }

    /// #41: rejecting restores the formatting before the change; accepting keeps the new
    /// formatting and drops the record. Both find formatting changes at the caret too.
    #[test]
    fn formatting_changes_are_accepted_and_rejected() {
        let mut s = formatted_while_tracking();
        s.run("review.rejectAll", &json!({})).unwrap();
        let p = body_para(&s);
        let c = p.props_of_char(6);
        assert_eq!((c.bold, c.size, c.fmt_change.is_none()), (None, None, true));
        assert_eq!((p.props.align, p.props.fmt_change.is_none()), (None, true));
        assert_eq!(p.props_of_char(13).italic, Some(true), "untracked formatting stays");

        s.run("edit.undo", &json!({})).unwrap();
        // Accept at the caret: just the run's change.
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 8}, "focus": {"block": 0, "off": 8}})).unwrap();
        s.run("review.accept", &json!({})).unwrap();
        let p = body_para(&s);
        assert_eq!((p.props_of_char(6).bold, p.props_of_char(6).fmt_change.is_none()), (Some(true), true));
        assert!(p.props.fmt_change.is_some(), "the paragraph's change is still there");
        s.run("review.acceptAll", &json!({})).unwrap();
        let p = body_para(&s);
        assert_eq!((p.props.align, p.props.fmt_change.is_none()), (Some(wordcraft_doc::props::Align::Center), true));
        assert!(s.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty());
    }

    /// Clearing formatting in the Style Inspector keeps pending formatting revisions.
    #[test]
    fn inspector_clear_keeps_formatting_changes() {
        let mut s = formatted_while_tracking();
        s.run("review.trackChanges", &json!({"value": false})).unwrap();
        s.run("styles.inspectorClear", &json!({"level": "paragraphFormatting"})).unwrap();
        assert!(body_para(&s).props.fmt_change.is_some(), "paragraph formatting change survives");

        let mut s = formatted_while_tracking();
        s.run("review.trackChanges", &json!({"value": false})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 11}})).unwrap();
        s.run("styles.inspectorClear", &json!({"level": "characterFormatting"})).unwrap();
        assert!(body_para(&s).props_of_char(6).fmt_change.is_some(), "character formatting change survives");
    }

    /// Comment anchors left in any story (the body, headers, footers, notes and text boxes), with
    /// the story each sits in.
    fn anchors(s: &Session) -> Vec<(wordcraft_doc::StoryRef, u32)> {
        use wordcraft_doc::StoryRef;
        use wordcraft_doc::para::InlineObject;
        let stories = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k)));
        let mut ids = Vec::new();
        for story in stories {
            for path in s.doc.para_paths(story) {
                let p = s.doc.para(story, &path).unwrap();
                for o in p.object_offsets() {
                    if let Some(InlineObject::CommentStart { id } | InlineObject::CommentEnd { id }) = p.object_at(o) {
                        ids.push((story, *id));
                    }
                }
            }
        }
        ids
    }

    /// #1345: deleting a comment anchored in a header removes its anchors there, not just in the
    /// body; Undo brings the comment and its anchors back.
    #[test]
    fn deleting_a_header_comment_removes_its_anchors() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "Body text"})).unwrap();
        let hdr = s.run("insert.header", &json!({"text": "Header text"})).unwrap()["story"].as_u64().unwrap() as u32;
        s.run("select.all", &json!({})).unwrap();
        let id = s.run("review.newComment", &json!({"text": "in the header"})).unwrap()["id"].as_u64().unwrap() as u32;
        let in_header = vec![(wordcraft_doc::StoryRef::Part(hdr), id); 2];
        assert_eq!(anchors(&s), in_header, "both anchors sit in the header");
        s.run("review.deleteComment", &json!({"id": id})).unwrap();
        assert!(anchors(&s).is_empty(), "no anchor points at the deleted comment");
        assert!(s.doc.comments.is_empty());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(anchors(&s), in_header);
        assert!(s.doc.comments.contains_key(&id));
    }

    /// #1345: deleting a comment deletes the whole thread below it, replies to replies included,
    /// with their text parts; Undo restores all of them.
    #[test]
    fn deleting_a_comment_deletes_replies_to_replies() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "hello world"})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}})).unwrap();
        let root = s.run("review.newComment", &json!({"text": "root"})).unwrap()["id"].as_u64().unwrap();
        let reply = s.run("review.reply", &json!({"id": root, "text": "reply"})).unwrap()["id"].as_u64().unwrap();
        let nested = s.run("review.reply", &json!({"id": reply, "text": "reply to reply"})).unwrap()["id"].as_u64().unwrap();
        let parts = |s: &Session| s.doc.comments.values().map(|c| c.part).collect::<Vec<_>>();
        let before = parts(&s);
        let deleted = s.run("review.deleteComment", &json!({"id": root})).unwrap();
        assert_eq!(deleted["deleted"], json!([root, reply, nested]));
        assert!(s.doc.comments.is_empty(), "no reply is left without its parent: {:?}", s.doc.comments);
        assert!(before.iter().all(|p| !s.doc.parts.contains_key(p)), "the comments' text parts are gone");
        assert!(anchors(&s).is_empty());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.comments.keys().map(|k| u64::from(*k)).collect::<Vec<_>>(), vec![root, reply, nested]);
        assert_eq!(parts(&s), before);
        assert_eq!(anchors(&s).len(), 2);
    }

    /// A parent cycle (possible in a hostile file) doesn't hang Delete; both comments go.
    #[test]
    fn deleting_a_comment_in_a_parent_cycle_ends() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "hello"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        let a = s.run("review.newComment", &json!({"text": "a"})).unwrap()["id"].as_u64().unwrap() as u32;
        let b = s.run("review.reply", &json!({"id": a, "text": "b"})).unwrap()["id"].as_u64().unwrap() as u32;
        s.doc.comments.get_mut(&a).unwrap().parent = Some(b);
        s.run("review.deleteComment", &json!({"id": a})).unwrap();
        assert!(s.doc.comments.is_empty());
    }

    /// A very long reply chain (as a hostile file could hold) is deleted in one pass over the
    /// comments, not one pass per reply: 50,000 replies would otherwise freeze the app.
    #[test]
    fn deleting_a_long_reply_chain_stays_fast() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "hello"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        let root = s.run("review.newComment", &json!({"text": "root"})).unwrap()["id"].as_u64().unwrap() as u32;
        let template = s.doc.comments.get(&root).unwrap().clone();
        for k in 1..=50_000u32 {
            s.doc.comments.insert(root + k, wordcraft_doc::Comment { parent: Some(root + k - 1), ..template.clone() });
        }
        let started = std::time::Instant::now();
        s.run("review.deleteComment", &json!({"id": root})).unwrap();
        assert!(s.doc.comments.is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    /// A comment on the whole of the story the caret is in; its id and that story.
    fn comment_here(s: &mut Session, text: &str) -> (u64, wordcraft_doc::StoryRef) {
        let story = s.sel.focus.story;
        s.run("select.all", &json!({})).unwrap();
        (s.run("review.newComment", &json!({"text": text})).unwrap()["id"].as_u64().unwrap(), story)
    }

    /// Comments in a header, a footer, a text box and a footnote, made in that order, then one
    /// on a word of the body. Returns each comment's id and story in reading order, which isn't
    /// the order they were made: the body, the text box and footnote it shows, then the header
    /// and the footer.
    fn comments_in_every_story(s: &mut Session) -> Vec<(u64, wordcraft_doc::StoryRef)> {
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "Body text with a comment on it."})).unwrap();
        s.run("insert.header", &json!({"text": "header"})).unwrap();
        let header = comment_here(s, "header");
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.run("insert.footer", &json!({"text": "footer"})).unwrap();
        let footer = comment_here(s, "footer");
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.textBox", &json!({"text": "boxed"})).unwrap();
        let text_box = comment_here(s, "text box");
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("references.footnote", &json!({"text": "noted"})).unwrap();
        let note = comment_here(s, "footnote");
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.run("select.text", &json!({"text": "comment"})).unwrap();
        let body = (s.run("review.newComment", &json!({"text": "body"})).unwrap()["id"].as_u64().unwrap(), wordcraft_doc::StoryRef::Body);
        s.run("caret.docStart", &json!({})).unwrap();
        vec![body, text_box, note, header, footer]
    }

    /// `review.comments` as (id, the anchor's story).
    fn listed(s: &mut Session) -> Vec<(u64, Option<wordcraft_doc::StoryRef>)> {
        let list = s.run("review.comments", &json!({})).unwrap();
        list.as_array().unwrap().iter().map(|c| (c["id"].as_u64().unwrap(), serde_json::from_value(c["anchor"]["story"].clone()).ok())).collect()
    }

    /// The stories Next Comment visits, `n` presses from the caret.
    fn next_stories(s: &mut Session, n: usize) -> Vec<wordcraft_doc::StoryRef> {
        (0..n)
            .map(|_| {
                s.run("review.nextComment", &json!({})).unwrap();
                s.sel.focus.story
            })
            .collect()
    }

    /// #2289: comments in headers, footers, text boxes and footnotes are listed with their
    /// anchors, in reading order rather than the order their stories were made.
    #[test]
    fn comments_outside_the_body_are_listed_with_their_anchors() {
        let mut s = Session::new(Document::new());
        let want: Vec<_> = comments_in_every_story(&mut s).into_iter().map(|(id, st)| (id, Some(st))).collect();
        assert_eq!(listed(&mut s), want);
    }

    /// #2289: Next and Previous Comment go through every story's comments in reading order and
    /// wrap around.
    #[test]
    fn next_and_previous_comment_reach_comments_outside_the_body() {
        let mut s = Session::new(Document::new());
        let want: Vec<_> = comments_in_every_story(&mut s).into_iter().map(|(_, st)| st).collect();
        let mut wrapped = want.clone();
        wrapped.push(want[0]);
        assert_eq!(next_stories(&mut s, want.len() + 1), wrapped, "Next visits each comment, then wraps to the first");
        let back: Vec<_> = (0..want.len())
            .map(|_| {
                s.run("review.previousComment", &json!({})).unwrap();
                s.sel.focus.story
            })
            .collect();
        let mut rev = want.clone();
        rev.reverse();
        assert_eq!(back, rev, "Previous from the first comment wraps to the last");
    }

    /// #2289: a header comes before the footer whichever was made first.
    #[test]
    fn next_comment_reaches_the_header_before_the_footer() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "Body"})).unwrap();
        s.run("insert.footer", &json!({"text": "footer"})).unwrap();
        let footer = comment_here(&mut s, "footer").1;
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.run("insert.header", &json!({"text": "header"})).unwrap();
        let header = comment_here(&mut s, "header").1;
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert_eq!(next_stories(&mut s, 2), vec![header, footer]);
    }

    /// #2289: a comment left in a story nothing shows any more (a removed header, a deleted
    /// footnote's text) has no anchor, and Next never takes the caret into that hidden story.
    #[test]
    fn comments_in_hidden_stories_are_not_navigated_to() {
        let mut s = Session::new(Document::new());
        let all = comments_in_every_story(&mut s);
        let (note, header) = (all[2], all[3]);
        s.run("insert.removeHeader", &json!({})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("text.backspace", &json!({})).unwrap();
        assert!(!s.doc.shown_stories().contains(&note.1), "the footnote's reference is gone");
        let list = listed(&mut s);
        for hidden in [note, header] {
            assert!(list.contains(&(hidden.0, None)), "comment {} sits in a hidden story: {list:?}", hidden.0);
        }
        let shown = [all[0].1, all[1].1, all[4].1];
        for st in next_stories(&mut s, all.len()) {
            assert!(shown.contains(&st), "Next went to hidden {st:?}");
        }
    }

    /// A body comment, then, with `option` on, a comment in the header of page `page`; `option`
    /// is turned off again. Returns the header comment's id and story.
    fn comment_in_header_version(s: &mut Session, option: &str, page: usize) -> (u64, wordcraft_doc::StoryRef) {
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "Body text with a comment on it."})).unwrap();
        s.run("select.text", &json!({"text": "comment"})).unwrap();
        s.run("review.newComment", &json!({"text": "body"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.pageBreak", &json!({})).unwrap();
        s.run(option, &json!({"value": true})).unwrap();
        s.run("insert.editHeader", &json!({"page": page})).unwrap();
        s.run("text.insert", &json!({"text": "header"})).unwrap();
        let header = comment_here(s, "header");
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.run(option, &json!({"value": false})).unwrap();
        s.run("caret.docStart", &json!({})).unwrap();
        header
    }

    /// A first-page or even-page header is hidden while its option is off: its comments have no
    /// anchor and Next skips them.
    #[test]
    fn comments_in_header_versions_whose_option_is_off_are_not_navigated_to() {
        for (option, page, slot) in [("layout.differentFirstPage", 0, "first"), ("layout.differentOddEven", 1, "even")] {
            let mut s = Session::new(Document::new());
            let (id, story) = comment_in_header_version(&mut s, option, page);
            let headers = s.doc.sections().first().map(|(_, sec)| sec.headers).unwrap();
            let want = if slot == "first" { headers.first } else { headers.even };
            assert_eq!(want.map(wordcraft_doc::StoryRef::Part), Some(story), "{option}: the comment is in the {slot} header");
            assert!(listed(&mut s).contains(&(id, None)), "{option} off: no anchor");
            for st in next_stories(&mut s, 3) {
                assert_eq!(st, wordcraft_doc::StoryRef::Body, "{option} off: Next went to the hidden {slot} header");
            }
            s.run(option, &json!({"value": true})).unwrap();
            assert!(listed(&mut s).contains(&(id, Some(story))), "{option} on: anchored again");
        }
    }

    /// With Different First Page on, the first page's header comes before the later pages'
    /// header, in the list and for Next.
    #[test]
    fn first_page_header_comes_before_the_default_header() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "Page one"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.pageBreak", &json!({})).unwrap();
        s.run("layout.differentFirstPage", &json!({"value": true})).unwrap();
        let mut header = |page: usize| {
            s.run("insert.editHeader", &json!({"page": page})).unwrap();
            s.run("text.insert", &json!({"text": "header"})).unwrap();
            let c = comment_here(&mut s, "header");
            s.run("insert.closeHeader", &json!({})).unwrap();
            c
        };
        let (first, default) = (header(0), header(1));
        assert_ne!(first.1, default.1);
        s.run("caret.docStart", &json!({})).unwrap();
        assert_eq!(listed(&mut s), vec![(first.0, Some(first.1)), (default.0, Some(default.1))]);
        assert_eq!(next_stories(&mut s, 2), vec![first.1, default.1]);
    }

    /// A linked text box shows the first box's text: comments in its own text have no anchor and
    /// Next skips them.
    #[test]
    fn comments_in_a_linked_text_box_own_text_are_not_navigated_to() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "Body text with a comment on it."})).unwrap();
        s.run("select.text", &json!({"text": "comment"})).unwrap();
        s.run("review.newComment", &json!({"text": "body"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.textBox", &json!({"text": "first box"})).unwrap();
        let first = s.sel.focus.story;
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        // Only a blank box can be linked to.
        s.run("insert.textBox", &json!({"text": " "})).unwrap();
        let (id, follower) = comment_here(&mut s, "follower");
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.run("caret.set", &json!({"pos": {"story": first, "path": [0], "off": 0}})).unwrap();
        s.run("shape.link", &json!({"target": 1})).unwrap();
        assert_eq!(s.doc.text_box_chains().first().and_then(|c| c.get(1)).copied().map(wordcraft_doc::StoryRef::Part), Some(follower));
        s.run("caret.docStart", &json!({})).unwrap();
        assert!(listed(&mut s).contains(&(id, None)), "linked: no anchor");
        for st in next_stories(&mut s, 3) {
            assert_ne!(st, follower, "Next went into the linked box's own text");
        }
        s.run("caret.set", &json!({"pos": {"story": first, "path": [0], "off": 0}})).unwrap();
        s.run("shape.breakLink", &json!({})).unwrap();
        assert!(listed(&mut s).contains(&(id, Some(follower))), "unlinked: anchored again");
    }

    /// Text inside a comment isn't an anchor: a comment marker in a comment's own story is not
    /// listed as that comment's place in the document.
    #[test]
    fn comment_list_skips_comment_text() {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.run("document.setText", &json!({"text": "hello"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        let id = s.run("review.newComment", &json!({"text": "note"})).unwrap()["id"].as_u64().unwrap() as u32;
        let part = s.doc.comments.get(&id).unwrap().part;
        let at = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Part(part), path: wordcraft_doc::Path::top(0), off: 0 };
        s.doc.insert_object(&at, wordcraft_doc::InlineObject::CommentStart { id }, &Default::default()).unwrap();
        let list = super::comment_list(&s);
        assert_eq!(list.iter().filter(|(x, _)| *x == id).count(), 1);
        assert_eq!(list.first().and_then(|(_, p)| p.as_ref()).map(|p| p.story), Some(wordcraft_doc::StoryRef::Body));
    }

    #[test]
    fn rtf_tracked_changes_accept_and_reject_after_opening_and_round_trip() {
        // Gap 19: an RTF file's tracked insertions and deletions open as tracked changes, also
        // after a WordCraft → RTF → WordCraft round trip.
        let rtf: &[u8] = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}}{\*\revtbl {Unknown;}{Alice;}{Bob;}}
\pard Keep {\revised\revauth1\revdttm-1480361314 added }text{\deleted\revauthdel2\revdttmdel-2016341499  gone} end{\revised\revauth1\revdttm-1480361314\deleted\revauthdel2\revdttmdel-2016341499  both}.\par}";
        let opened = wordcraft_formats::rtf::import(rtf).unwrap();
        let again = wordcraft_formats::rtf::import(wordcraft_formats::rtf::export(&opened).as_bytes()).unwrap();
        for doc in [opened, again] {
            for (cmd, want) in [("review.acceptAll", "Keep added text end."), ("review.rejectAll", "Keep text gone end.")] {
                let mut s = Session::new(doc.clone());
                assert!(!s.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty(), "{cmd}");
                s.run(cmd, &json!({})).unwrap();
                assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), want, "{cmd}");
                assert!(s.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty(), "{cmd}");
            }
        }
    }
}
