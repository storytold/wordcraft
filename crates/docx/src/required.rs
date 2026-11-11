//! Conservative ownership markers for modeled paragraphs from a protected source package.
use std::collections::BTreeMap;
use std::sync::Arc;
use wordcraft_doc::{Block, Blocks, Document, NoteSeparators};

/// Distinguish isolated signature bytes from a structurally plausible ZIP record. A plausible
/// but broken archive is still refused by the bounded package reader, never treated as safe.
fn may_contain_zip(bytes: &[u8]) -> bool {
    for (at, signature) in bytes.windows(4).enumerate() {
        let (header_len, name_at, extra_at) = match signature {
            b"PK\x03\x04" => (30, 26, 28),
            b"PK\x01\x02" => (46, 28, 30),
            b"PK\x05\x06" => {
                if let Some(record) = bytes.get(at..at.saturating_add(22)) {
                    let comment = usize::from(u16::from_le_bytes([record[20], record[21]]));
                    if at.checked_add(22).and_then(|n| n.checked_add(comment)).is_some_and(|end| end <= bytes.len()) {
                        return true;
                    }
                }
                continue;
            }
            _ => continue,
        };
        if let Some(header) = bytes.get(at..at.saturating_add(header_len)) {
            let name = usize::from(u16::from_le_bytes([header[name_at], header[name_at + 1]]));
            let extra = usize::from(u16::from_le_bytes([header[extra_at], header[extra_at + 1]]));
            if name > 0
                && at
                    .checked_add(header_len)
                    .and_then(|n| n.checked_add(name))
                    .and_then(|n| n.checked_add(extra))
                    .is_some_and(|end| end <= bytes.len())
            {
                return true;
            }
        }
    }
    false
}

/// Replay of macro-free output cannot retain VBA that the normal writer would remove.
pub(crate) fn source_has_vba(source: &BTreeMap<String, Arc<Vec<u8>>>) -> Result<bool, crate::DocxError> {
    fn scan(source: &BTreeMap<String, Arc<Vec<u8>>>, budget: &mut u64, depth: usize) -> Result<bool, crate::DocxError> {
        for (name, bytes) in source {
            let lower = name.to_ascii_lowercase();
            if lower.rsplit('/').next() == Some("vbaproject.bin") {
                return Ok(true);
            }
            // ZIP permits a self-extracting prefix and trailing bytes. A plausible but
            // uninspectable package must not bypass the macro-free policy.
            if may_contain_zip(bytes) {
                if depth >= 4 {
                    return Err(crate::DocxError::Preservation("nested package exceeds macro inspection depth".into()));
                }
                let nested = crate::package::Package::open_limited(bytes, *budget, *budget)
                    .map_err(|_| crate::DocxError::Preservation("cannot inspect nested package for macros".into()))?;
                *budget = budget.saturating_sub(nested.size());
                if scan(&nested.shared_parts()?, budget, depth + 1)? {
                    return Ok(true);
                }
            }
            if !lower.ends_with(".rels") && lower != "[content_types].xml" {
                continue;
            }
            let root = crate::xml::parse_document(bytes)
                .map_err(|_| crate::DocxError::Preservation("cannot verify source relationships or content types for macros".into()))?;
            for e in root.els() {
                let value = if lower.ends_with(".rels") { e.attr("Type") } else { e.attr("ContentType") };
                if value.is_some_and(|v| v.to_ascii_lowercase().contains("vbaproject")) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    scan(source, &mut (wordcraft_doc::kept::MAX_KEPT_BYTES as u64), 0)
}

fn mark_blocks(blocks: &mut Blocks, depth: usize) {
    if depth > crate::xml::MAX_DEPTH {
        return;
    }
    for block in blocks {
        match Arc::make_mut(block) {
            Block::Para(p) => p.required_docx_source = true,
            Block::Table(t) => {
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        mark_blocks(&mut cell.blocks, depth + 1);
                    }
                }
            }
        }
    }
}
fn has_blocks(blocks: &Blocks, depth: usize) -> bool {
    if depth > crate::xml::MAX_DEPTH {
        return true;
    }
    blocks.iter().any(|block| match &**block {
        Block::Para(p) => p.required_docx_source,
        Block::Table(t) => t.rows.iter().any(|r| r.cells.iter().any(|c| has_blocks(&c.blocks, depth + 1))),
    })
}
fn separators_mut(s: &mut NoteSeparators) -> impl Iterator<Item = &mut Blocks> {
    [&mut s.separator, &mut s.continuation_separator, &mut s.continuation_notice].into_iter().flatten()
}
fn separators(s: &NoteSeparators) -> impl Iterator<Item = &Blocks> {
    [&s.separator, &s.continuation_separator, &s.continuation_notice].into_iter().flatten()
}
pub(crate) fn mark(doc: &mut Document) {
    mark_blocks(&mut doc.body, 0);
    for p in doc.parts.values_mut() {
        mark_blocks(&mut p.blocks, 0);
    }
    for b in &mut doc.glossary {
        mark_blocks(&mut b.blocks, 0);
    }
    for b in separators_mut(&mut doc.footnote_separators).chain(separators_mut(&mut doc.endnote_separators)) {
        mark_blocks(b, 0);
    }
}
/// Content equality alone cannot prove ownership: deleting and retyping the same text could
/// otherwise resurrect markup. An unchanged COW block retains its source identity.
pub(crate) fn same_owners(doc: &Document, source: &Document) -> bool {
    fn blocks(a: &Blocks, b: &Blocks) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
    }
    fn notes(a: &NoteSeparators, b: &NoteSeparators) -> bool {
        let a = [&a.separator, &a.continuation_separator, &a.continuation_notice];
        let b = [&b.separator, &b.continuation_separator, &b.continuation_notice];
        a.into_iter().zip(b).all(|(a, b)| match (a, b) {
            (Some(a), Some(b)) => blocks(a, b),
            (None, None) => true,
            _ => false,
        })
    }
    blocks(&doc.body, &source.body)
        && doc.parts.len() == source.parts.len()
        && doc.parts.iter().zip(&source.parts).all(|((id, a), (other, b))| id == other && blocks(&a.blocks, &b.blocks))
        && doc.glossary.len() == source.glossary.len()
        && doc.glossary.iter().zip(&source.glossary).all(|(a, b)| blocks(&a.blocks, &b.blocks))
        && notes(&doc.footnote_separators, &source.footnote_separators)
        && notes(&doc.endnote_separators, &source.endnote_separators)
}

pub(crate) fn has(doc: &Document) -> bool {
    has_blocks(&doc.body, 0)
        || doc.parts.values().any(|p| has_blocks(&p.blocks, 0))
        || doc.glossary.iter().any(|b| has_blocks(&b.blocks, 0))
        || separators(&doc.footnote_separators).chain(separators(&doc.endnote_separators)).any(|b| has_blocks(b, 0))
}
