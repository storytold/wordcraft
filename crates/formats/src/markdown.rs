//! Markdown import and export via `craft-markdown`.

use std::collections::HashMap;

use craft_flow::CODE_STYLE;
use wordcraft_doc::{Block, Document, SectionProps};

pub use craft_markdown::{Refs, escape, parse, parse_inlines};

fn shared_section(page: &SectionProps) -> craft_wordmodel::SectionProps {
    let mut d = Document::new();
    d.last_section = page.clone();
    wordcraft_docx::to_shared_model(&d).map(|s| s.last_section).unwrap_or_default()
}

pub fn import(text: &str) -> Document {
    wordcraft_docx::from_shared_model(craft_markdown::import(text)).unwrap_or_default()
}

pub fn import_on(text: &str, page: &SectionProps) -> Document {
    wordcraft_docx::from_shared_model(craft_markdown::import_on(text, &shared_section(page))).unwrap_or_default()
}

/// Bytes of source that the searches for blocks' first words may read in all, as a multiple of the
/// text's length, plus [`SEARCH_BUDGET_MIN`]. Once they have read that much, each search reads at
/// most [`NEAR_LINES`] lines.
const SEARCH_BUDGET: usize = 64;
const SEARCH_BUDGET_MIN: usize = 1 << 20;
/// Lines one search reads once [`SEARCH_BUDGET`] is spent. Every block moves the next search
/// down at least one line, so each line is read by at most this many searches.
const NEAR_LINES: usize = 64;

/// [`import_on`] returning the document and the 0-based starting line of each top-level block.
pub fn import_on_with_lines(text: &str, page: &SectionProps) -> (Document, Vec<usize>) {
    let doc = import_on(text, page);
    let src_lines: Vec<&str> = text.strip_prefix('\u{FEFF}').unwrap_or(text).lines().collect();
    let mut out = Vec::with_capacity(doc.body.len());
    let mut cursor = 0usize;
    let mut budget = text.len().saturating_mul(SEARCH_BUDGET).saturating_add(SEARCH_BUDGET_MIN);
    // A word a search read to the end without finding, by the line that search started on.
    let mut absent: HashMap<&str, usize> = HashMap::new();
    // The first line from which no line has text outside a code fence.
    let mut no_text_from = usize::MAX;
    let code_blank = code_blanks(&src_lines);
    for block in &doc.body {
        let needle = match block.as_ref() {
            Block::Para(p) => p.text.split_whitespace().next().unwrap_or(""),
            Block::Table(t) => t
                .rows
                .first()
                .and_then(|r| r.cells.first())
                .and_then(|c| c.blocks.first())
                .and_then(|b| match b.as_ref() {
                    Block::Para(p) => p.text.split_whitespace().next(),
                    _ => None,
                })
                .unwrap_or(""),
        };
        let mut found = cursor;
        let hit = if needle.is_empty() || absent.get(needle).is_some_and(|&from| from <= cursor) {
            None
        } else {
            let end = if budget > 0 { src_lines.len() } else { cursor.saturating_add(NEAR_LINES).min(src_lines.len()) };
            let hit = src_lines.get(cursor..end).unwrap_or_default().iter().position(|l| {
                budget = budget.saturating_sub(l.len() + 1);
                l.contains(needle)
            });
            if hit.is_none() && end == src_lines.len() {
                absent.insert(needle, cursor);
            }
            hit
        };
        if let Some(rel) = hit {
            found = cursor + rel;
            cursor = found + 1;
            if matches!(block.as_ref(), Block::Table(_)) {
                while src_lines.get(cursor).is_some_and(|l| l.trim_start().starts_with('|')) {
                    cursor += 1;
                }
            }
        } else if cursor >= src_lines.len() {
            found = src_lines.len();
        } else if let Some(at) = blank_code_line(block, needle, &src_lines, &code_blank, cursor) {
            found = at;
            cursor = (at + 1).min(src_lines.len());
        } else if cursor < no_text_from {
            // A block whose first word is not in the source as written (an entity, an escape)
            // starts at the next line with text that is not a code fence, and the next search
            // starts after it.
            match src_lines[cursor..].iter().position(|l| !l.trim().is_empty() && !is_fence(l)) {
                Some(rel) => {
                    found = cursor + rel;
                    cursor = found + 1;
                }
                None => no_text_from = cursor,
            }
        }
        out.push(found.min(src_lines.len()));
    }
    (doc, out)
}

/// The line of a blank code line: the first blank line of a code block (see [`code_blanks`]) at
/// or after `cursor`, when only blank lines and fences come before it. `src_lines.len()` when the
/// text ends first, and `None` when a line with text comes first.
fn blank_code_line(block: &Block, needle: &str, src_lines: &[&str], code_blank: &[bool], cursor: usize) -> Option<usize> {
    let Block::Para(p) = block else { return None };
    if !needle.is_empty() || p.props.style.as_deref() != Some(CODE_STYLE) {
        return None;
    }
    for (i, l) in src_lines.iter().enumerate().skip(cursor) {
        if l.trim().is_empty() {
            if code_blank.get(i).copied().unwrap_or(false) {
                return Some(i);
            }
        } else if !is_fence(l) {
            return None;
        }
    }
    Some(src_lines.len())
}

/// The run of a code fence, as `craft-markdown` reads one: its character and length.
fn fence_run(line: &str) -> Option<(char, usize)> {
    if is_indented_code(line) {
        return None;
    }
    let t = line.trim_start_matches([' ', '\t']);
    let c = t.chars().next().filter(|&c| c == '`' || c == '~')?;
    let n = t.chars().take_while(|&x| x == c).count();
    if n < 3 || (c == '`' && t.get(n..).is_some_and(|info| info.contains('`'))) {
        return None;
    }
    Some((c, n))
}

fn is_fence(line: &str) -> bool {
    fence_run(line).is_some()
}

/// Whether `craft-markdown` reads a line with text as indented code: four columns or more of
/// leading spaces and tabs, with tab stops of 4.
fn is_indented_code(line: &str) -> bool {
    let mut col = 0;
    for c in line.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => break,
        }
        if col >= 4 {
            return true;
        }
    }
    false
}

/// For each line, whether it is a blank line of a code block: inside a fenced block, or between
/// two indented code lines with only blank lines between them.
fn code_blanks(src_lines: &[&str]) -> Vec<bool> {
    // Forward: whether a blank line is inside a fence, or follows an indented code line.
    let mut open: Option<(char, usize)> = None;
    let mut after_indented = false;
    let forward: Vec<(bool, bool)> = src_lines
        .iter()
        .map(|l| {
            let blank = l.trim().is_empty();
            match (open, fence_run(l)) {
                (None, Some(run)) => {
                    open = Some(run);
                    after_indented = false;
                    (false, false)
                }
                (Some((c, n)), Some((c2, n2))) if c2 == c && n2 >= n && l.trim_start_matches([' ', '\t']).trim_start_matches(c).trim().is_empty() => {
                    open = None;
                    (false, false)
                }
                (Some(_), _) => (blank, false),
                (None, None) => {
                    if !blank {
                        after_indented = is_indented_code(l);
                    }
                    (false, blank && after_indented)
                }
            }
        })
        .collect();
    // Backward: a blank line after an indented code line is a code line only when another
    // indented code line follows it.
    let mut out = vec![false; src_lines.len()];
    let mut before_indented = false;
    for (i, l) in src_lines.iter().enumerate().rev() {
        let (fenced, after) = forward[i];
        if !l.trim().is_empty() {
            before_indented = is_indented_code(l) && fence_run(l).is_none();
        }
        out[i] = fenced || (after && before_indented);
    }
    out
}

pub fn export(doc: &Document) -> String {
    match wordcraft_docx::to_shared_model(doc) {
        Ok(shared) => craft_markdown::export(&shared),
        Err(_) => String::new(),
    }
}
