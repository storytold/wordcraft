//! `text.rewrite`: replace the text of a paragraph (or a range inside one) with new text by editing
//! only what differs, for agents. Replacing the whole old text marks all of it deleted and all the
//! new text inserted under Track Changes, so a reviewer sees a paragraph struck through and written
//! again when a few words changed, and the unchanged text loses its runs, comment anchors and
//! formatting. Here the old and new text are compared (words, or characters for Chinese and
//! Japanese text), and each difference is one small tracked deletion and insertion; text that
//! didn't change is never touched.

use serde_json::{Value, json};
use unicode_segmentation::UnicodeSegmentation;
use wordcraft_doc::Pos;
use wordcraft_doc::para::OBJ;

use super::tools::{Op, lcs_ops};
use super::{controls, delete_selection, paste_text};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// Cells the comparison table may take: more than this and the whole text is replaced.
const BUDGET: u64 = 10_000_000;

/// The share (percent) of the text two versions may differ in before it is replaced whole: past
/// it a word-by-word edit reads worse than a plain replacement.
const WHOLE_ABOVE_PERCENT: usize = 70;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("text.rewrite", "Rewrite Text", "Agents", rewrite).params(
            r#"{"text": string (the new text of the target, one paragraph), "paragraph"?: body paragraph index (default: the selection, else the caret's paragraph), "level"?: "word" (default; Chinese and Japanese by character) | "character"} → {"changes": n, "whole": bool}; with Track Changes on each difference is a small reviewable deletion and insertion, and text that didn't change keeps its formatting, comments and earlier revisions"#,
        ),
    ]
}

/// One difference: the bytes of the old text it replaces, and what replaces them.
#[derive(Debug, PartialEq)]
struct Edit {
    start: usize,
    end: usize,
    ins: String,
}

/// What kind of character a token is made of, for [`tokens`].
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Word,
    Space,
    Other,
}

fn is_ideograph(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{2FFFF}')
}

/// The start of each token of `text` and its end: words (letters and digits, with the apostrophe
/// inside a word), runs of spaces, other characters one by one, and each Chinese, Japanese or
/// Korean character on its own; or every grapheme with `characters`. Combining marks stay with
/// the character before.
fn tokens(text: &str, characters: bool) -> Vec<usize> {
    let mut starts: Vec<usize> = Vec::new();
    if characters {
        starts.extend(text.grapheme_indices(true).map(|(i, _)| i));
    } else {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut prev: Option<Kind> = None;
        for (k, (i, c)) in chars.iter().enumerate() {
            if wordcraft_doc::math_linear::is_combining(*c) && prev.is_some() {
                continue;
            }
            let inner_apostrophe = matches!(c, '\'' | '\u{2019}')
                && k > 0
                && chars.get(k - 1).is_some_and(|(_, p)| p.is_alphanumeric())
                && chars.get(k + 1).is_some_and(|(_, n)| n.is_alphanumeric());
            let kind = if is_ideograph(*c) {
                Kind::Other
            } else if c.is_alphanumeric() || *c == '_' || inner_apostrophe {
                Kind::Word
            } else if c.is_whitespace() {
                Kind::Space
            } else {
                Kind::Other
            };
            let continues = !is_ideograph(*c) && prev == Some(kind) && kind != Kind::Other;
            if !continues {
                starts.push(*i);
            }
            prev = Some(kind);
        }
    }
    starts.push(text.len());
    starts
}

/// The differences between `old` and `new` as edits of `old`, back to front order not applied
/// yet. `whole` when the two differ too much (or are too long to compare) and one edit replaces
/// everything.
fn plan(old: &str, new: &str, characters: bool) -> (Vec<Edit>, bool) {
    if old == new {
        return (Vec::new(), false);
    }
    let whole = || (vec![Edit { start: 0, end: old.len(), ins: new.to_string() }], true);
    let (os, ns) = (tokens(old, characters), tokens(new, characters));
    let pairs = |starts: &[usize]| -> Vec<(usize, usize)> { starts.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))).collect() };
    let (ot, nt) = (pairs(&os), pairs(&ns));
    let text_of = |text: &'_ str, (a, b): (usize, usize)| text.get(a..b).unwrap_or("").to_string();
    let (oa, na): (Vec<String>, Vec<String>) = (ot.iter().map(|t| text_of(old, *t)).collect(), nt.iter().map(|t| text_of(new, *t)).collect());
    let mut budget = BUDGET;
    let ops = lcs_ops(&oa, &na, &mut budget);
    // Runs of edits as token ranges: (old start, old end, new start, new end).
    let mut runs: Vec<(usize, usize, usize, usize)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut open: Option<(usize, usize)> = None;
    for op in &ops {
        match op {
            Op::Same => {
                if let Some((si, sj)) = open.take() {
                    runs.push((si, i, sj, j));
                }
                i += 1;
                j += 1;
            }
            Op::Del => {
                open.get_or_insert((i, j));
                i += 1;
            }
            Op::Ins => {
                open.get_or_insert((i, j));
                j += 1;
            }
        }
    }
    if let Some((si, sj)) = open.take() {
        runs.push((si, i, sj, j));
    }
    // A single space or punctuation mark between two edits joins them into one replacement: a
    // changed phrase reads as one change, not as words with the old spaces left between them.
    let mut merged: Vec<(usize, usize, usize, usize)> = Vec::new();
    for r in runs {
        if let Some(last) = merged.last_mut()
            && r.0 == last.1 + 1
            && oa.get(last.1).is_some_and(|t| t.chars().all(|c| !c.is_alphanumeric()))
        {
            last.1 = r.1;
            last.3 = r.3;
            continue;
        }
        merged.push(r);
    }
    // Where the tokens `a..b` of a text of `len` bytes start and end.
    let span = |t: &[(usize, usize)], a: usize, b: usize, len: usize| -> (usize, usize) {
        let start = t.get(a).map_or(len, |x| x.0);
        let end = if b > a { t.get(b - 1).map_or(len, |x| x.1) } else { start };
        (start, end)
    };
    let mut edits: Vec<Edit> = Vec::new();
    for (a0, a1, b0, b1) in merged {
        let (start, end) = span(&ot, a0, a1, old.len());
        let (ns0, ne0) = span(&nt, b0, b1, new.len());
        edits.push(Edit { start, end, ins: new.get(ns0..ne0).unwrap_or("").to_string() });
    }
    let changed: usize = edits.iter().map(|e| (e.end - e.start) + e.ins.len()).sum();
    if changed * 100 > (old.len() + new.len()) * WHOLE_ABOVE_PERCENT {
        return whole();
    }
    (edits, false)
}

fn rewrite(s: &mut Session, v: &Value) -> CmdResult {
    let new = p::req_str(v, "text")?.to_string();
    if new.contains(['\n', '\r', '\u{2029}']) {
        return Err(CmdError::Params("`text` is one paragraph: it holds a line or paragraph break".into()));
    }
    let characters = match p::str(v, "level") {
        None | Some("word") => false,
        Some("character" | "char") => true,
        Some(other) => return Err(CmdError::Params(format!("`level` is \"word\" or \"character\", not \"{other}\""))),
    };
    // The target: a body paragraph by index, else the selection, else the caret's paragraph.
    let (story, path, from, to) = if let Some(n) = p::u64(v, "paragraph") {
        let n = usize::try_from(n).map_err(|_| CmdError::Params("`paragraph` is out of range".into()))?;
        let at = Pos::body(n, 0);
        let para = s.doc.para_at(&at).ok_or_else(|| CmdError::Params(format!("body block {n} is not a paragraph")))?;
        (at.story, at.path.clone(), 0, para.text.len())
    } else {
        let (a, b) = s.sel.ordered();
        if a != b {
            if a.story != b.story || a.path != b.path {
                return Err(CmdError::Params("the selection spans several paragraphs: rewrite works inside one".into()));
            }
            (a.story, a.path.clone(), a.off, b.off)
        } else {
            let para = s.doc.para_at(&a).ok_or_else(|| CmdError::Params("the caret is not in a paragraph".into()))?;
            (a.story, a.path.clone(), 0, para.text.len())
        }
    };
    let at = |off: usize| Pos { story, path: path.clone(), off };
    let old = {
        let para = s.doc.para_at(&at(0)).ok_or_else(|| CmdError::Params("the target is not a paragraph".into()))?;
        let text = para.text.get(from..to).ok_or_else(|| CmdError::Params("the target range is not on character boundaries".into()))?;
        if text.contains(OBJ) {
            return Err(CmdError::Params("the target holds objects (pictures, fields, equations): rewrite works on plain text".into()));
        }
        if text.char_indices().any(|(i, _)| para.props_of_char(from + i).del.is_some()) {
            return Err(CmdError::Params("the target holds tracked deletions: accept or reject them first".into()));
        }
        text.to_string()
    };
    let (edits, whole) = plan(&old, &new, characters);
    if edits.is_empty() {
        return Ok(json!({"changes": 0, "whole": false}));
    }
    let spans: Vec<(Pos, Pos)> = edits.iter().filter(|e| e.end > e.start).map(|e| (at(from + e.start), at(from + e.end))).collect();
    controls::check_deletions(s, story, spans.iter().map(|(a, b)| (a, b)))?;
    // Last edit first, so earlier positions stay valid; the command is one undo step.
    for e in edits.iter().rev() {
        let (a, b) = (at(from + e.start), at(from + e.end));
        s.sel = Selection { anchor: a, focus: b };
        if e.ins.is_empty() {
            delete_selection(s)?;
        } else {
            paste_text(s, &e.ins)?;
        }
    }
    Ok(json!({"changes": edits.len(), "whole": whole}))
}

#[cfg(test)]
mod tests;
