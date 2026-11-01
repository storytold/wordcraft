//! Advanced paragraph tools (#5043): converting and cleaning up paragraphs, like WPS Office's
//! paragraph tools. Each command works on the paragraphs the selection covers, or on the whole
//! document with `scope: "all"`, and runs as one undo step. Deletions respect Track Changes.

use serde_json::{Value, json};

use wordcraft_doc::para::{LINE_BREAK, PAGE_BREAK};
use wordcraft_doc::{Path, Pos, RevisionKind, StoryRef};

use super::{CmdError, CmdResult, Session, p, sel_result};
use crate::cmd::{has_next_para, new_revision, split_para, track_delete_as};

/// The maximum number of paragraphs one call may touch: the work, and the blank paragraphs a
/// command can add, stay bounded whatever the document says.
const MAX_TOOL_PARAS: usize = 10_000;

/// Leading spaces one paragraph may gain from an indent, whatever the file's indent says.
const MAX_INDENT_SPACES: usize = 1_000;

/// The scope of a tools command: `{"scope": "selection|all"}`, `selection` by default.
/// `all` selects the whole document first, so the command, its undo step and the result all
/// follow the full selection like any other command.
fn scope_all(s: &mut Session, v: &Value) -> Result<(), CmdError> {
    match p::str(v, "scope").unwrap_or("selection") {
        "selection" => Ok(()),
        "all" => {
            s.run("select.all", &json!({}))?;
            Ok(())
        }
        x => Err(CmdError::Params(format!("unknown scope `{x}` (selection|all)"))),
    }
}

/// The paragraphs a command runs over, in document order, and the story they are in.
fn targets(s: &Session) -> Result<(StoryRef, Vec<Path>), CmdError> {
    let (a, b) = s.sel.ordered();
    let paths = s.doc.para_paths_in(&a, &b);
    if paths.is_empty() {
        return Err(CmdError::Failed("no paragraphs selected".into()));
    }
    if paths.len() > MAX_TOOL_PARAS {
        return Err(CmdError::Params(format!("select at most {MAX_TOOL_PARAS} paragraphs to process them in one command")));
    }
    Ok((a.story, paths))
}

/// Deletes `a..b` inside one paragraph, recording a tracked deletion when Track Changes is on.
/// Offsets past the paragraph's end are clamped, so the document is never half-edited.
fn delete_in_para(s: &mut Session, story: StoryRef, path: &Path, a: usize, b: usize) -> Result<(), CmdError> {
    let len = s.doc.para(story, path).ok_or_else(|| CmdError::Failed("no paragraph here".into()))?.len();
    let (a, b) = (a.min(b).min(len), b.min(len));
    if a >= b {
        return Ok(());
    }
    let at = |off: usize| Pos { story, path: path.clone(), off };
    if s.doc.settings.track_changes {
        let rid = new_revision(s, RevisionKind::Delete);
        track_delete_as(s, &at(a), &at(b), rid)?;
    } else {
        s.doc.delete_range(&at(a), &at(b))?;
    }
    Ok(())
}

/// The deletions to make in one paragraph, as byte ranges, merged where they touch and applied
/// last first so the earlier offsets stay valid.
fn apply_deletions(s: &mut Session, story: StoryRef, path: &Path, mut ranges: Vec<(usize, usize)>) -> Result<(), CmdError> {
    ranges.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in ranges {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    for (a, b) in merged.into_iter().rev() {
        delete_in_para(s, story, path, a, b)?;
    }
    Ok(())
}

/// The byte ranges of every manual line break in the paragraph.
fn break_ranges(para: &wordcraft_doc::Paragraph) -> Vec<(usize, usize)> {
    para.text.char_indices().filter(|(_, c)| *c == LINE_BREAK).map(|(i, _)| (i, i + 1)).collect()
}

/// `para.convertBreaks`: every manual line break (Shift+Enter) in the selection becomes a
/// paragraph break. Paragraphs are handled last first: a split only shifts the paths after it.
pub(super) fn convert_breaks(s: &mut Session, v: &Value) -> CmdResult {
    scope_all(s, v)?;
    let (story, paths) = targets(s)?;
    for path in paths.iter().rev() {
        let Some(para) = s.doc.para(story, path) else { continue };
        for (i, _) in break_ranges(para).into_iter().rev() {
            let new = split_para(s, &Pos { story, path: path.clone(), off: i })?;
            // The break stays at the head of the new paragraph; the paragraph break replaces it.
            delete_in_para(s, story, &new.path, 0, 1)?;
        }
    }
    sel_result(s)
}

/// `para.blankBetween`: insert one empty paragraph between the selected paragraphs; `after`
/// adds one blank paragraph after the last selected paragraph too.
pub(super) fn blank_between(s: &mut Session, v: &Value) -> CmdResult {
    let after = p::bool(v, "after").unwrap_or(false);
    scope_all(s, v)?;
    let (story, paths) = targets(s)?;
    let count = paths.len().saturating_sub(1) + usize::from(after);
    if count > MAX_TOOL_PARAS {
        return Err(CmdError::Params(format!("insert at most {MAX_TOOL_PARAS} blank paragraphs at once")));
    }
    // Last first, each split at a paragraph's end: a split only shifts the paths after it.
    if after {
        let last = paths.last().ok_or_else(|| CmdError::Failed("no paragraph selected".into()))?;
        let len = s.doc.para(story, last).map(|p| p.len()).unwrap_or(0);
        split_para(s, &Pos { story, path: last.clone(), off: len })?;
    }
    for path in paths[..paths.len().saturating_sub(1)].iter().rev() {
        let len = s.doc.para(story, path).map(|p| p.len()).unwrap_or(0);
        split_para(s, &Pos { story, path: path.clone(), off: len })?;
    }
    sel_result(s)
}

/// `para.separateParagraphs`: every selected paragraph becomes a paragraph of its own with a
/// blank line between them.
pub(super) fn separate_paragraphs(s: &mut Session, v: &Value) -> CmdResult {
    blank_between(s, &json!({"scope": p::str(v, "scope").unwrap_or("selection"), "after": true}))
}

/// `para.removeBlank`: delete every empty paragraph in the selection.
pub(super) fn remove_blank(s: &mut Session, v: &Value) -> CmdResult {
    scope_all(s, v)?;
    let (story, paths) = targets(s)?;
    // Last first: removing a block leaves the paths before it as they are.
    for path in paths.iter().rev() {
        let Some(para) = s.doc.para(story, path) else { continue };
        // A paragraph must be empty, carry no section break and have a paragraph after it in
        // its container: a story's, part's or cell's last mark can't be removed, and a paragraph
        // before a table can't be joined across it.
        if !para.is_empty() || para.section.is_some() || !has_next_para(s, story, path) {
            continue;
        }
        if s.doc.settings.track_changes {
            let next = path.with_last(path.last() + 1);
            let rid = new_revision(s, RevisionKind::Delete);
            track_delete_as(s, &Pos { story, path: path.clone(), off: 0 }, &Pos { story, path: next, off: 0 }, rid)?;
        } else {
            s.doc.remove_block(story, path)?;
        }
    }
    sel_result(s)
}

/// The number of leading spaces in the paragraph's text.
fn leading_spaces(para: &wordcraft_doc::Paragraph) -> usize {
    para.text.chars().take_while(|c| *c == ' ').count()
}

/// `para.trimLeadingSpaces`: delete the spaces at the start of each selected paragraph.
pub(super) fn trim_leading(s: &mut Session, v: &Value) -> CmdResult {
    scope_all(s, v)?;
    let (story, paths) = targets(s)?;
    for path in paths {
        let Some(para) = s.doc.para(story, &path) else { continue };
        let n = leading_spaces(para);
        apply_deletions(s, story, &path, vec![(0, n)])?;
    }
    sel_result(s)
}

/// `para.removeExtraSpaces`: every run of spaces outside the paragraph's start becomes one
/// space; tabs and page breaks count as spaces, so spaces around a tab collapse to it.
/// Leading spaces stay (that is `para.trimLeadingSpaces`).
pub(super) fn remove_extra_spaces(s: &mut Session, v: &Value) -> CmdResult {
    scope_all(s, v)?;
    let (story, paths) = targets(s)?;
    for path in paths {
        let Some(para) = s.doc.para(story, &path) else { continue };
        let mut runs = Vec::new();
        let mut start = None;
        for (i, c) in para.text.char_indices() {
            if matches!(c, ' ' | '\t' | PAGE_BREAK) {
                start.get_or_insert(i);
            } else if let Some(a) = start.take() {
                // A run at the paragraph's start stays; a run inside the text collapses to one space.
                if a > 0 && i - a > 1 {
                    runs.push((a, i - 1));
                }
            }
        }
        // A run that reaches the paragraph's end collapses to one space too.
        if let Some(a) = start
            && a > 0
            && para.len() - a > 1
        {
            runs.push((a + 1, para.len()));
        }
        apply_deletions(s, story, &path, runs)?;
    }
    sel_result(s)
}

/// `para.spacesToIndent`: each paragraph's leading spaces become a first-line indent of 72 pt
/// per space (Word's point-based indent from typing spaces), and are deleted.
pub(super) fn spaces_to_indent(s: &mut Session, v: &Value) -> CmdResult {
    scope_all(s, v)?;
    let (story, paths) = targets(s)?;
    for path in paths {
        let Some(para) = s.doc.para(story, &path) else { continue };
        let spaces = leading_spaces(para);
        if spaces == 0 {
            continue;
        }
        {
            let p = s.doc.para_mut(story, &path)?;
            p.props.indent_first = Some((p.props.indent_first.unwrap_or(0.0) + spaces as f32 * 72.0).clamp(-1584.0, 1584.0));
            p.props.cancel_char_indents(false, false, true);
            p.touch();
        }
        // A space is one byte, so the byte range is the count.
        apply_deletions(s, story, &path, vec![(0, spaces)])?;
    }
    sel_result(s)
}

/// `para.indentToSpaces`: each paragraph's first-line and left indents become leading spaces
/// (72 pt per space, as typing spaces indents), and the indents go to zero.
pub(super) fn indent_to_spaces(s: &mut Session, v: &Value) -> CmdResult {
    scope_all(s, v)?;
    let (story, paths) = targets(s)?;
    for path in paths {
        let Some(para) = s.doc.para(story, &path) else { continue };
        let pts = para.props.indent_left.unwrap_or(0.0) + para.props.indent_first.unwrap_or(0.0);
        let n = (pts / 72.0).round().clamp(0.0, MAX_INDENT_SPACES as f32) as usize;
        if n == 0 {
            continue;
        }
        let mut props = s.doc.para(story, &path).map(|p| p.props_at(0)).cloned().unwrap_or_default();
        // The revision first: `new_revision` edits `s.doc`, so it runs before the paragraph borrow.
        // The spaces are marked as a tracked insertion through `props.ins`, as typing is.
        if s.doc.settings.track_changes {
            props.ins = Some(new_revision(s, RevisionKind::Insert));
        }
        let p = s.doc.para_mut(story, &path)?;
        p.insert_text(0, &" ".repeat(n), &props)?;
        p.props.indent_first = Some(0.0);
        p.props.indent_left = Some(0.0);
        p.props.cancel_char_indents(true, false, true);
        p.touch();
    }
    sel_result(s)
}

/// `para.removeLineBreaks`: delete every manual line break in the selection, so the lines
/// join into one paragraph.
pub(super) fn remove_line_breaks(s: &mut Session, v: &Value) -> CmdResult {
    scope_all(s, v)?;
    let (story, paths) = targets(s)?;
    for path in paths {
        let Some(para) = s.doc.para(story, &path) else { continue };
        apply_deletions(s, story, &path, break_ranges(para))?;
    }
    sel_result(s)
}
