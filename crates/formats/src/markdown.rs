//! Markdown import and export via `craft-markdown`.

use std::collections::HashMap;

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
        } else if let Some(rel) = src_lines[cursor..].iter().position(|l| !l.trim().is_empty()) {
            // A block whose first word is not in the source as written (an entity, an escape)
            // starts at the next line with text, and the next search starts after it.
            found = cursor + rel;
            cursor = found + 1;
        }
        out.push(found.min(src_lines.len()));
    }
    (doc, out)
}

pub fn export(doc: &Document) -> String {
    match wordcraft_docx::to_shared_model(doc) {
        Ok(shared) => craft_markdown::export(&shared),
        Err(_) => String::new(),
    }
}
