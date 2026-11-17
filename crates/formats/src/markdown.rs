//! Markdown import and export via `craft-markdown`.

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

/// [`import_on`] returning the document and the 0-based starting line of each top-level block.
pub fn import_on_with_lines(text: &str, page: &SectionProps) -> (Document, Vec<usize>) {
    let doc = import_on(text, page);
    let src_lines: Vec<&str> = text.strip_prefix('\u{FEFF}').unwrap_or(text).lines().collect();
    let mut out = Vec::with_capacity(doc.body.len());
    let mut cursor = 0usize;
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
        if !needle.is_empty() {
            if let Some(rel) = src_lines[cursor..].iter().position(|l| l.contains(needle)) {
                found = cursor + rel;
                cursor = found + 1;
                if matches!(block.as_ref(), Block::Table(_)) {
                    while src_lines.get(cursor).is_some_and(|l| l.trim_start().starts_with('|')) {
                        cursor += 1;
                    }
                }
            }
        } else if cursor >= src_lines.len() {
            found = src_lines.len();
        } else if let Some(rel) = src_lines[cursor..].iter().position(|l| !l.trim().is_empty()) {
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
