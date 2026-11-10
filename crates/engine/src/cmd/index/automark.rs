//! References › Insert Index › AutoMark: index entries from a concordance file, and the matcher
//! Mark All shares with it.
//!
//! A concordance file is a document whose two-column table pairs the text to find (first column)
//! with the index entry to mark it with (second column; `Main:Sub` for a subentry, empty for the
//! text itself). A document without a table is read a line at a time, the text and the entry
//! separated by a tab. As in Word, each row marks the first exact (same case, whole word)
//! occurrence of its text in every paragraph, and a paragraph that already holds the entry's XE
//! field is left alone.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Pos, StoryRef};

use super::super::references::quoted;
use super::{ENTRY_STYLE, clean};
use crate::{CmdError, CmdResult, Session};

/// Most concordance rows one AutoMark uses (Word's own limit on a table's rows); the rest are
/// left out.
const MAX_TERMS: usize = wordcraft_doc::table::MAX_ROWS;

/// Longest index entry kept, as Mark Entry keeps it.
const MAX_ENTRY: usize = 200;

/// Longest text looked for, in characters: what Word's Find box takes. A longer row is skipped.
const MAX_TEXT: usize = 255;

/// Largest concordance file read: 32,767 rows of the longest text and entry fit many times over,
/// and a document's worth of anything bigger is not a word list.
const MAX_FILE: u64 = 32 << 20;

/// One text to find, with the XE field that marks it.
pub(super) struct Term {
    pub text: String,
    pub instr: String,
}

/// The XE field for index entry `entry` (`Main:Sub` for a subentry), cleaned as Mark Entry cleans
/// it; `None` when nothing is left. A backslash is written doubled, as quoted field text needs
/// it, so the index shows the entry as the concordance has it.
fn xe_for(entry: &str) -> Option<String> {
    let entry = clean(entry, MAX_ENTRY);
    (!entry.is_empty()).then(|| format!("XE {}", quoted(&entry)))
}

/// A term for a concordance row: the text found and its entry (the text itself when the entry
/// is empty). `None` for a row with nothing to find, or more than [`MAX_TEXT`] characters of it.
fn term(text: &str, entry: &str) -> Option<Term> {
    let text = text.trim();
    if text.is_empty() || text.contains(OBJ) || text.chars().nth(MAX_TEXT).is_some() {
        return None;
    }
    let instr = xe_for(entry).or_else(|| xe_for(text))?;
    Some(Term { text: text.to_string(), instr })
}

/// The text of a table cell: its paragraphs, one space between them (nested tables left out).
fn cell_text(cell: &wordcraft_doc::table::Cell) -> String {
    let parts: Vec<String> = cell.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text().trim().to_string()).collect();
    parts.into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The concordance rows of `doc`, as (text to find, index entry), at most [`MAX_TERMS`]: every
/// row of the tables in its body, in order; in a body without a table, every paragraph, its text
/// split at the first tab. A row without a second column has an empty entry.
pub(super) fn concordance(doc: &Document) -> Vec<(String, String)> {
    let tables: Vec<&wordcraft_doc::table::Table> = doc.body.iter().filter_map(|b| b.as_table()).collect();
    if tables.is_empty() {
        let lines = doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text());
        return lines
            .map(|l| match l.split_once('\t') {
                Some((text, entry)) => (text.to_string(), entry.to_string()),
                None => (l, String::new()),
            })
            .take(MAX_TERMS)
            .collect();
    }
    tables
        .iter()
        .flat_map(|t| t.rows.iter())
        .take(MAX_TERMS)
        .map(|r| {
            let mut cells = r.cells.iter().map(cell_text);
            (cells.next().unwrap_or_default(), cells.next().unwrap_or_default())
        })
        .collect()
}

/// The concordance rows of the file at `path` (see [`concordance`]): any document WordCraft
/// opens, at most 32 MB, read inside the automation read root. For the app, which reads the file
/// the person picked and passes its rows as `references.autoMark`'s `entries`.
pub fn read_concordance(path: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    crate::io::open_path_up_to(path, MAX_FILE).map(|doc| concordance(&doc))
}

/// `references.autoMark`'s rows: the concordance file at `path`, or `entries` (each a text, or
/// `[text, entry]`). A network path is refused before anything looks at it: opening
/// `\\host\share\…` would make Windows sign in to that host with the user's credentials.
fn rows(v: &Value) -> Result<Vec<(String, String)>, CmdError> {
    if let Some(path) = v.get("path").filter(|p| !p.is_null()) {
        let Some(path) = path.as_str() else { return Err(CmdError::Params("`path` is a file path".into())) };
        if super::super::templates::is_network_path(path) {
            return Err(CmdError::Params("the concordance file can't be on another computer".into()));
        }
        return read_concordance(std::path::Path::new(path)).map_err(CmdError::Failed);
    }
    let Some(list) = v.get("entries").and_then(Value::as_array) else {
        return Err(CmdError::Params("give `path` (a concordance file) or `entries`".into()));
    };
    list.iter()
        .take(MAX_TERMS)
        .map(|e| match e {
            Value::String(t) => Ok((t.clone(), String::new())),
            Value::Array(pair) if (1..=2).contains(&pair.len()) => {
                let text = pair.first().and_then(Value::as_str);
                let entry = pair.get(1).map_or(Some(""), Value::as_str);
                match (text, entry) {
                    (Some(t), Some(en)) => Ok((t.to_string(), en.to_string())),
                    _ => Err(CmdError::Params("each entry is a text or [text, entry]".into())),
                }
            }
            _ => Err(CmdError::Params("each entry is a text or [text, entry]".into())),
        })
        .collect()
}

/// `references.autoMark`: mark every row's text with its entry.
pub(super) fn auto_mark(s: &mut Session, v: &Value) -> CmdResult {
    let terms: Vec<Term> = rows(v)?.iter().filter_map(|(t, e)| term(t, e)).collect();
    let marked = mark_terms(s, &terms)?;
    s.clamp_selection();
    Ok(json!({"marked": marked, "terms": terms.len()}))
}

/// The run of letters and digits `t` starts with (empty when it starts with anything else).
fn leading_word(t: &str) -> &str {
    let end = t.char_indices().find(|(_, c)| !c.is_alphanumeric()).map_or(t.len(), |(i, _)| i);
    t.get(..end).unwrap_or_default()
}

/// Insert each term's XE field, hidden, after its first whole-word, same-case match in every body
/// paragraph that doesn't hold that field yet (index entries and tables of contents are not
/// searched). Terms marking the same entry count as one: the earliest term with a match in a
/// paragraph marks it, at that term's first match. Returns how many fields went in.
///
/// Each paragraph is read once, whatever the number of terms: a term can only match where a
/// word starts, and only where that word is the one the term starts with.
pub(super) fn mark_terms(s: &mut Session, terms: &[Term]) -> Result<u32, CmdError> {
    let mut by_word: HashMap<&str, Vec<usize>> = HashMap::new();
    for (k, t) in terms.iter().enumerate() {
        by_word.entry(leading_word(&t.text)).or_default().push(k);
    }
    if by_word.is_empty() {
        return Ok(0);
    }
    let hidden = CharProps { hidden: Some(true), ..Default::default() };
    let mut marked = 0u32;
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(para) = s.doc.para(StoryRef::Body, &path) else { continue };
        if para.props.style.as_deref().is_some_and(|st| st == ENTRY_STYLE || st.starts_with("TOC")) {
            continue;
        }
        // Each term's first match in this paragraph: where it ends.
        let mut first: Vec<(usize, usize)> = Vec::new();
        let mut seen: HashSet<usize> = HashSet::new();
        let text = para.text.as_str();
        let mut word_start = true;
        for (i, c) in text.char_indices() {
            wordcraft_doc::work::steps(1);
            if word_start {
                let rest = text.get(i..).unwrap_or_default();
                for &k in by_word.get(leading_word(rest)).into_iter().flatten() {
                    wordcraft_doc::work::steps(1);
                    let Some(t) = terms.get(k) else { continue };
                    if seen.contains(&k) || !rest.starts_with(t.text.as_str()) {
                        continue;
                    }
                    let end = i.saturating_add(t.text.len());
                    let whole = text.get(end..).and_then(|r| r.chars().next()).is_none_or(|n| !n.is_alphanumeric());
                    if whole {
                        seen.insert(k);
                        first.push((k, end));
                    }
                }
            }
            word_start = !c.is_alphanumeric();
        }
        if first.is_empty() {
            continue;
        }
        // Earlier terms first; one field per entry, none where the paragraph has it already.
        let mut have: HashSet<&str> = para
            .objects
            .iter()
            .filter_map(|o| match o {
                InlineObject::Field { instr, .. } => Some(instr.as_str()),
                _ => None,
            })
            .collect();
        first.sort_unstable();
        let mut at: Vec<(usize, usize)> = Vec::new();
        for (k, end) in first {
            if let Some(t) = terms.get(k)
                && have.insert(t.instr.as_str())
            {
                at.push((end, k));
            }
        }
        // From the end back, so earlier offsets stay put; fields at one place keep the terms' order.
        at.sort_unstable_by(|a, b| b.cmp(a));
        for (end, k) in at {
            let Some(t) = terms.get(k) else { continue };
            let field =
                InlineObject::Field { instr: t.instr.clone(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() };
            s.doc.insert_object(&Pos { story: StoryRef::Body, path: path.clone(), off: end }, field, &hidden)?;
            marked = marked.saturating_add(1);
        }
    }
    Ok(marked)
}

#[cfg(test)]
mod tests;
