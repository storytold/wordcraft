//! AutoFormat Now (Word's Format › AutoFormat, Alt+Ctrl+K): the AutoFormat-as-you-type
//! replacements applied to text already in the document — smart quotes, dashes and typed list
//! labels — over the paragraphs the selection touches, or the whole body (including table
//! cells) when it is collapsed. The command is one undo step.

use super::autoformat::{apply_list, list_kind, smart_quote};
use super::{CmdResult, ListKind, OBJ, Path, Pos, Selection, Session, StoryRef, Value, sel_result};
use wordcraft_doc::InlineObject;

/// The labels AutoFormat Now turns into lists: the bullets and the first numbers.
const LIST_LABELS: [&str; 5] = ["* ", "- ", "> ", "1. ", "1) "];

/// One replacement in a paragraph: bytes `start..end` become `with`.
struct Edit {
    start: usize,
    end: usize,
    with: &'static str,
}

/// `format.autoFormatNow`.
pub(super) fn autoformat_now(s: &mut Session, _: &Value) -> CmdResult {
    if !s.prefs.autocorrect.enabled {
        return sel_result(s);
    }
    let (a, b) = s.sel.ordered();
    let all = s.doc.para_paths(StoryRef::Body);
    let scope: Vec<Path> = if s.sel.is_collapsed() {
        all.clone()
    } else if a.story == StoryRef::Body && b.story == StoryRef::Body {
        s.doc.para_paths_in(&a, &b)
    } else {
        Vec::new()
    };
    let mut sel = s.sel.clone();
    // A field's result may span paragraphs, so its depth is followed through the whole body.
    let mut depth = 0usize;
    for path in &all {
        let Some(para) = s.doc.para(StoryRef::Body, path) else { continue };
        let (edits, after) = text_edits(s, para, depth);
        depth = after;
        if !scope.contains(path) {
            continue;
        }
        // Back to front, so earlier offsets stay valid.
        for e in edits.iter().rev() {
            let p = s.doc.para_mut(StoryRef::Body, path)?;
            let props = p.props_of_char(e.start).clone().without_symbol();
            p.delete(e.start, e.end)?;
            p.insert_text(e.start, e.with, &props)?;
            p.touch();
            shift(&mut sel, path, e.start, e.end, e.with.len());
        }
        if let Some((len, kind)) = list_label(s, path)
            && apply_list(s, StoryRef::Body, path, len, kind)?
        {
            shift(&mut sel, path, 0, len, 0);
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

/// The quote and dash replacements for `para`, which starts `depth` fields deep, and the field
/// depth at its end. Objects, field results and tracked deletions are left alone.
fn text_edits(s: &Session, para: &wordcraft_doc::Paragraph, mut depth: usize) -> (Vec<Edit>, usize) {
    let ac = &s.prefs.autocorrect;
    let chars: Vec<(usize, char)> = para.text.char_indices().collect();
    let mut free = Vec::with_capacity(chars.len());
    for &(i, c) in &chars {
        if c == OBJ {
            match para.object_at(i) {
                Some(InlineObject::FieldStart { .. }) => depth = depth.saturating_add(1),
                Some(InlineObject::FieldEnd) => depth = depth.saturating_sub(1),
                _ => {}
            }
            free.push(false);
        } else {
            free.push(depth == 0 && para.props_of_char(i).del.is_none());
        }
    }
    let ch = |k: usize| chars.get(k).map(|x| x.1);
    let ok = |k: usize| free.get(k).copied().unwrap_or(false);
    let word = |k: Option<usize>| k.and_then(ch).is_some_and(|c| !c.is_whitespace() && c != '-' && c != OBJ);
    let mut edits = Vec::new();
    // The previous character as it will read afterwards, for opening/closing quotes.
    let mut prev: Option<char> = None;
    let mut k = 0;
    while let Some(&(i, c)) = chars.get(k) {
        if ok(k)
            && ac.smart_quotes
            && let Some(q) = smart_quote(prev, c)
        {
            edits.push(Edit { start: i, end: i + 1, with: q });
            prev = q.chars().next();
            k += 1;
            continue;
        }
        if ok(k) && ac.dashes && c == '-' {
            let double = ch(k + 1) == Some('-') && ok(k + 1) && ch(k + 2) != Some('-');
            let spaced = |after: usize| k >= 2 && ch(k - 1) == Some(' ') && ch(after) == Some(' ') && word(Some(k - 2)) && word(Some(after + 1));
            // "word -- word" and "word - word" → en dash, the spaces kept.
            if double && spaced(k + 2) {
                edits.push(Edit { start: i, end: i + 2, with: "\u{2013}" });
                prev = Some('\u{2013}');
                k += 2;
                continue;
            }
            // "word--word" → em dash.
            if double && word(k.checked_sub(1)) && word(Some(k + 2)) {
                edits.push(Edit { start: i, end: i + 2, with: "\u{2014}" });
                prev = Some('\u{2014}');
                k += 2;
                continue;
            }
            if ch(k + 1) != Some('-') && spaced(k + 1) {
                edits.push(Edit { start: i, end: i + 1, with: "\u{2013}" });
                prev = Some('\u{2013}');
                k += 1;
                continue;
            }
        }
        prev = Some(c);
        k += 1;
    }
    (edits, depth)
}

/// The list label ("* ", "1. "…) the paragraph at `path` starts with, as typed text with more
/// after it, and the list it starts.
fn list_label(s: &Session, path: &Path) -> Option<(usize, ListKind)> {
    let para = s.doc.para(StoryRef::Body, path)?;
    let label = LIST_LABELS.into_iter().find(|l| para.text.starts_with(l))?;
    let rest = para.text.get(label.len()..)?;
    if rest.trim().is_empty() || para.props_of_char(0).del.is_some() {
        return None;
    }
    Some((label.len(), list_kind(label)?))
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
