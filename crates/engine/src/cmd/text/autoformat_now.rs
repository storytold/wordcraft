//! AutoFormat Now (Word's Format › AutoFormat, Alt+Ctrl+K): the AutoFormat-as-you-type
//! replacements applied to text already in the document — smart quotes, dashes and typed list
//! labels — over the paragraphs the selection touches, or the whole body (including table
//! cells) when it is collapsed. The command is one undo step.

use super::autoformat::{list_kind, make_list_item, smart_quote};
use super::{CmdResult, ListKind, OBJ, Path, Pos, Selection, Session, StoryRef, Value, sel_result};
use wordcraft_doc::{InlineObject, RevisionKind};

/// The labels AutoFormat Now turns into lists: the bullets and the first numbers.
const LIST_LABELS: [&str; 5] = ["* ", "- ", "> ", "1. ", "1) "];

/// One replacement in a paragraph: bytes `start..end` become `with`. A `tracked` replacement
/// keeps the old characters as a tracked deletion and inserts `with` after them.
struct Edit {
    start: usize,
    end: usize,
    with: &'static str,
    tracked: bool,
}

/// The content (start, end) of each content control in the body that can't be edited.
type Locked = Vec<(Pos, Pos)>;

/// `format.autoFormatNow`.
pub(super) fn autoformat_now(s: &mut Session, _: &Value) -> CmdResult {
    if !s.prefs.autocorrect.enabled {
        return sel_result(s);
    }
    let (a, b) = s.sel.ordered();
    let all = s.doc.para_paths(StoryRef::Body);
    // The paragraphs in scope, as an index range over `all`: the whole body for a caret, else
    // the selection's paragraphs, which are contiguous in `all`.
    let scope = if s.sel.is_collapsed() {
        0..all.len()
    } else if a.story == StoryRef::Body && b.story == StoryRef::Body {
        let picked = s.doc.para_paths_in(&a, &b);
        match picked.first().and_then(|first| all.iter().position(|p| p == first)) {
            Some(i) => i..i.saturating_add(picked.len()).min(all.len()),
            None => 0..0,
        }
    } else {
        0..0
    };
    let locked: Locked =
        s.doc.control_ranges(StoryRef::Body).into_iter().filter(|r| !r.control.lock.can_edit()).map(|r| (r.content_start(), r.end)).collect();
    let mut sel = s.sel.clone();
    // Under Track Changes the rewrite is a tracked deletion and insertion, as Change Case does
    // (#1329); the revisions are made on first use, one pair per run.
    let mut revs: Option<(u32, u32)> = None;
    // A field's result may span paragraphs, so its depth is followed through the whole body.
    let mut depth = 0usize;
    for (i, path) in all.iter().enumerate() {
        let Some(para) = s.doc.para(StoryRef::Body, path) else { continue };
        let at_start = depth;
        let in_scope = scope.contains(&i);
        let (edits, after) = text_edits(s, para, path, depth, in_scope, &locked);
        depth = after;
        if !in_scope {
            continue;
        }
        if edits.iter().any(|e| e.tracked) && revs.is_none() {
            revs = Some((crate::cmd::new_revision(s, RevisionKind::Delete), crate::cmd::new_revision(s, RevisionKind::Insert)));
        }
        if !edits.is_empty() {
            let p = s.doc.para_mut(StoryRef::Body, path)?;
            // Back to front, so earlier offsets stay valid.
            for e in edits.iter().rev() {
                match revs.filter(|_| e.tracked) {
                    Some((del, ins)) => {
                        let mut props = p.props_of_char(e.start).clone().without_symbol();
                        p.format(e.start, e.end, &|c| c.del = Some(del))?;
                        (props.ins, props.del, props.fmt_change) = (Some(ins), None, None);
                        p.insert_text(e.end, e.with, &props)?;
                        shift(&mut sel, path, e.start, e.end, (e.end - e.start).saturating_add(e.with.len()));
                    }
                    None => {
                        let props = p.props_of_char(e.start).clone().without_symbol();
                        p.delete(e.start, e.end)?;
                        p.insert_text(e.start, e.with, &props)?;
                        shift(&mut sel, path, e.start, e.end, e.with.len());
                    }
                }
            }
            p.touch();
        }
        if let Some((len, kind, tracked)) = list_label(s, path, at_start, &locked) {
            list_item(s, path, len, kind, tracked, &mut sel)?;
        }
    }
    for pos in [&mut sel.anchor, &mut sel.focus] {
        if let Some(p) = s.doc.para_at(pos) {
            pos.off = p.clamp(pos.off);
        }
    }
    s.sel = sel;
    sel_result(s)
}

/// Make the paragraph at `path` a `kind` list item and remove its `len`-byte label: as a tracked
/// deletion with a tracked paragraph formatting change when `tracked`, so Reject All gives the
/// typed label back, else in place.
fn list_item(s: &mut Session, path: &Path, len: usize, kind: ListKind, tracked: bool, sel: &mut Selection) -> Result<(), crate::CmdError> {
    let Some(before) = s.doc.para(StoryRef::Body, path).map(|p| p.props.clone()) else { return Ok(()) };
    if !make_list_item(s, StoryRef::Body, path, kind)? {
        return Ok(());
    }
    if tracked {
        let del = crate::cmd::new_revision(s, RevisionKind::Delete);
        let p = s.doc.para_mut(StoryRef::Body, path)?;
        p.format(0, len, &|c| c.del = Some(del))?;
        p.touch();
    } else {
        s.doc.para_mut(StoryRef::Body, path)?.delete(0, len)?;
        shift(sel, path, 0, len, 0);
    }
    if s.doc.settings.track_changes {
        crate::cmd::fmt_revisions::record_para_props(s, StoryRef::Body, path, &before)?;
    }
    Ok(())
}

/// Whether bytes `start..end` of the paragraph at `path` touch the content of a control that
/// can't be edited (the geometry of `controls::check_edit`).
fn touches_locked(locked: &Locked, path: &Path, start: usize, end: usize) -> bool {
    locked.iter().any(|(cs, ce)| (path, start) < (&ce.path, ce.off) && (path, end) > (&cs.path, cs.off))
}

/// Whether the character at `i` was inserted, tracked, by the current author: such text is
/// rewritten in place even under Track Changes, so it stays one insertion.
fn own_insertion(s: &Session, para: &wordcraft_doc::Paragraph, i: usize) -> bool {
    para.props_of_char(i).ins.and_then(|r| s.doc.revisions.get(r as usize)).is_some_and(|r| r.author == s.author)
}

/// The quote and dash replacements for `para` (at `path`), which starts `depth` fields deep,
/// and the field depth at its end; with `collect` off only the depth is followed. Objects, field
/// results, tracked deletions and locked content controls are left alone.
fn text_edits(s: &Session, para: &wordcraft_doc::Paragraph, path: &Path, mut depth: usize, collect: bool, locked: &Locked) -> (Vec<Edit>, usize) {
    let ac = &s.prefs.autocorrect;
    if !collect || !(ac.smart_quotes || ac.dashes) {
        for (i, c) in para.text.char_indices() {
            if c == OBJ {
                match para.object_at(i) {
                    Some(InlineObject::FieldStart { .. }) => depth = depth.saturating_add(1),
                    Some(InlineObject::FieldEnd) => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
        }
        return (Vec::new(), depth);
    }
    let chars: Vec<(usize, char)> = para.text.char_indices().collect();
    let mut free = Vec::with_capacity(chars.len());
    // Characters that don't show: tracked deletions and the markers of controls, bookmarks,
    // comments and permissions. The quote rule reads past them to the visible character.
    let mut hidden = Vec::with_capacity(chars.len());
    for &(i, c) in &chars {
        if c == OBJ {
            let marker = match para.object_at(i) {
                Some(InlineObject::FieldStart { .. }) => {
                    depth = depth.saturating_add(1);
                    false
                }
                Some(InlineObject::FieldEnd) => {
                    depth = depth.saturating_sub(1);
                    false
                }
                Some(
                    InlineObject::ControlStart { .. }
                    | InlineObject::ControlEnd
                    | InlineObject::BookmarkStart { .. }
                    | InlineObject::BookmarkEnd { .. }
                    | InlineObject::CommentStart { .. }
                    | InlineObject::CommentEnd { .. }
                    | InlineObject::PermStart { .. }
                    | InlineObject::PermEnd { .. },
                ) => true,
                _ => false,
            };
            free.push(false);
            hidden.push(marker || para.props_of_char(i).del.is_some());
        } else {
            let deleted = para.props_of_char(i).del.is_some();
            free.push(depth == 0 && !deleted);
            hidden.push(deleted);
        }
    }
    let tracking = s.doc.settings.track_changes;
    let edit = |start: usize, end: usize, with: &'static str| Edit { start, end, with, tracked: tracking && !own_insertion(s, para, start) };
    let ch = |k: usize| chars.get(k).map(|x| x.1);
    let ok = |k: usize| free.get(k).copied().unwrap_or(false);
    let word = |k: Option<usize>| k.and_then(ch).is_some_and(|c| !c.is_whitespace() && c != '-' && c != OBJ);
    let mut edits = Vec::new();
    // The previous visible character as it will read afterwards, for opening/closing quotes.
    let mut prev: Option<char> = None;
    let mut k = 0;
    while let Some(&(i, c)) = chars.get(k) {
        if ok(k)
            && ac.smart_quotes
            && let Some(q) = smart_quote(prev, c)
        {
            edits.push(edit(i, i + 1, q));
            prev = q.chars().next();
            k += 1;
            continue;
        }
        if ok(k) && ac.dashes && c == '-' {
            let double = ch(k + 1) == Some('-') && ok(k + 1) && ch(k + 2) != Some('-');
            let spaced = |after: usize| k >= 2 && ch(k - 1) == Some(' ') && ch(after) == Some(' ') && word(Some(k - 2)) && word(Some(after + 1));
            // "word -- word" and "word - word" → en dash, the spaces kept.
            if double && spaced(k + 2) {
                edits.push(edit(i, i + 2, "\u{2013}"));
                prev = Some('\u{2013}');
                k += 2;
                continue;
            }
            // "word--word" → em dash.
            if double && word(k.checked_sub(1)) && word(Some(k + 2)) {
                edits.push(edit(i, i + 2, "\u{2014}"));
                prev = Some('\u{2014}');
                k += 2;
                continue;
            }
            if ch(k + 1) != Some('-') && spaced(k + 1) {
                edits.push(edit(i, i + 1, "\u{2013}"));
                prev = Some('\u{2013}');
                k += 1;
                continue;
            }
        }
        if !hidden.get(k).copied().unwrap_or(false) {
            prev = Some(c);
        }
        k += 1;
    }
    // Text in a control locked against editing stays as it is.
    edits.retain(|e| !touches_locked(locked, path, e.start, e.end));
    (edits, depth)
}

/// The list label ("* ", "1. "…) the paragraph at `path` starts with, as typed text with more
/// after it, the list it starts, and whether its removal is tracked. A paragraph that starts
/// inside a field result (`depth` > 0), a label with a tracked deletion in it, and a label in a
/// locked content control are not labels.
fn list_label(s: &Session, path: &Path, depth: usize, locked: &Locked) -> Option<(usize, ListKind, bool)> {
    if depth > 0 {
        return None;
    }
    let para = s.doc.para(StoryRef::Body, path)?;
    let label = LIST_LABELS.into_iter().find(|l| para.text.starts_with(l))?;
    let rest = para.text.get(label.len()..)?;
    // Labels are ASCII, so each byte is a character.
    if rest.trim().is_empty() || (0..label.len()).any(|i| para.props_of_char(i).del.is_some()) || touches_locked(locked, path, 0, label.len()) {
        return None;
    }
    let tracked = s.doc.settings.track_changes && !own_insertion(s, para, 0);
    Some((label.len(), list_kind(label)?, tracked))
}

/// Move a selection end in `path` past the replacement of `start..end` by `len` bytes.
fn shift(sel: &mut Selection, path: &Path, start: usize, end: usize, len: usize) {
    for pos in [&mut sel.anchor, &mut sel.focus] {
        let pos: &mut Pos = pos;
        if pos.story != StoryRef::Body || &pos.path != path || pos.off <= start {
            continue;
        }
        pos.off = if pos.off >= end { pos.off - (end - start) + len } else { start };
    }
}
