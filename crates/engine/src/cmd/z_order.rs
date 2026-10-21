//! Assign editor-created drawings a shared model rank before layout or export.

use std::sync::Arc;
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::para::{InlineObject, Wrap};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Pos};

use crate::{CmdError, Session};

// Bound both the scan and its rank vectors, including empty table cells and stories.
const MAX_ORDER_ITEMS: usize = 1_000_000;
const MAX_ORDER_DEPTH: usize = 64;

#[derive(Clone, Copy)]
pub(super) enum Intent {
    Fresh,
    Preserve,
}

fn limit() -> CmdError {
    CmdError::Failed("drawing order exceeds its allocation or traversal limit".into())
}

fn floating(obj: &InlineObject) -> bool {
    obj.frame().is_some_and(|(_, _, f)| f.wrap != Wrap::Inline)
}

struct Budget(usize);

impl Budget {
    fn take(&mut self) -> Result<(), CmdError> {
        self.0 = self.0.checked_sub(1).ok_or_else(limit)?;
        Ok(())
    }
}

fn scan_block(block: &Block, depth: usize, budget: &mut Budget, ranks: &mut Vec<u32>) -> Result<(), CmdError> {
    budget.take()?;
    if depth > MAX_ORDER_DEPTH {
        return Err(limit());
    }
    match block {
        Block::Para(p) => {
            for obj in &p.objects {
                budget.take()?;
                if let Some((_, _, f)) = obj.frame().filter(|_| floating(obj)) {
                    ranks.try_reserve(1).map_err(|_| limit())?;
                    ranks.push(f.z_order);
                }
            }
        }
        Block::Table(t) => {
            for row in &t.rows {
                budget.take()?;
                for cell in &row.cells {
                    budget.take()?;
                    for child in &cell.blocks {
                        scan_block(child, depth + 1, budget, ranks)?;
                    }
                }
            }
        }
    }
    // GroupChild.float is unused. Its child-vector paint order is not a rank namespace.
    Ok(())
}

fn document_ranks(doc: &Document) -> Result<Vec<u32>, CmdError> {
    let mut ranks = Vec::new();
    let mut budget = Budget(MAX_ORDER_ITEMS);
    for blocks in std::iter::once(&doc.body).chain(doc.parts.values().map(|p| &p.blocks)) {
        budget.take()?;
        for block in blocks {
            scan_block(block, 0, &mut budget, &mut ranks)?;
        }
    }
    Ok(ranks)
}

fn fragment_ranks(frag: &Fragment) -> Result<Vec<u32>, CmdError> {
    let mut ranks = Vec::new();
    let mut budget = Budget(MAX_ORDER_ITEMS);
    for block in &frag.blocks {
        scan_block(block, 0, &mut budget, &mut ranks)?;
    }
    for part in frag.parts.values() {
        budget.take()?;
        for block in &part.blocks {
            scan_block(block, 0, &mut budget, &mut ranks)?;
        }
    }
    Ok(ranks)
}

/// Reserve once per operation. Every checked allocation precedes normalization.
fn reserve(doc: &mut Document, incoming: &[u32]) -> Result<Vec<u32>, CmdError> {
    if incoming.is_empty() {
        return Ok(Vec::new());
    }
    if incoming.len() > MAX_ORDER_ITEMS {
        return Err(limit());
    }
    let mut sorted = Vec::new();
    sorted.try_reserve_exact(incoming.len()).map_err(|_| limit())?;
    sorted.extend(incoming.iter().copied().enumerate().map(|(index, z)| (z, index)));
    // The index makes ties deterministic without a sorting scratch allocation.
    sorted.sort_unstable();
    let mut count = 0u32;
    let mut previous = None;
    for (rank, _) in &sorted {
        if previous != Some(*rank) {
            count = count.checked_add(1).ok_or_else(limit)?;
            previous = Some(*rank);
        }
    }
    let mut old = document_ranks(doc)?;
    let max = old.iter().copied().max().unwrap_or(0);
    let normalize = max.checked_add(count).is_none();
    let start = if normalize {
        old.sort_unstable();
        old.dedup();
        u32::try_from(old.len()).map_err(|_| limit())?
    } else {
        max
    };
    start.checked_add(count).ok_or_else(limit)?;
    let mut assigned = Vec::new();
    assigned.try_reserve_exact(incoming.len()).map_err(|_| limit())?;
    assigned.resize(incoming.len(), 0);
    let mut rank = start;
    previous = None;
    for (old_rank, index) in sorted {
        if previous != Some(old_rank) {
            rank = rank.checked_add(1).ok_or_else(limit)?;
            previous = Some(old_rank);
        }
        *assigned.get_mut(index).ok_or_else(limit)? = rank;
    }
    if normalize {
        doc.objects_mut(&floating, &mut |obj| {
            if let Some(f) = obj.float_mut()
                && let Some(rank) = old.binary_search(&f.z_order).ok().and_then(|i| u32::try_from(i).ok()).and_then(|i| i.checked_add(1))
            {
                f.z_order = rank;
            }
        });
    }
    Ok(assigned)
}

pub(super) fn object(doc: &mut Document, obj: &mut InlineObject, intent: Intent) -> Result<(), CmdError> {
    if matches!(intent, Intent::Preserve) || !floating(obj) {
        return Ok(());
    }
    let rank = obj.frame().map_or(0, |(_, _, f)| f.z_order);
    let assigned = reserve(doc, &[rank])?;
    if let Some(f) = obj.float_mut() {
        f.z_order = assigned.first().copied().ok_or_else(limit)?;
    }
    Ok(())
}

pub(super) fn insert(s: &mut Session, at: &Pos, mut obj: InlineObject, props: &CharProps, intent: Intent) -> Result<Pos, CmdError> {
    object(&mut s.doc, &mut obj, intent)?;
    Ok(s.doc.insert_object(at, obj, props)?)
}

fn assign_block(block: &mut Block, ranks: &mut impl Iterator<Item = u32>) -> Result<(), CmdError> {
    match block {
        Block::Para(p) => {
            let mut changed = false;
            for obj in &mut p.objects {
                if floating(obj)
                    && let Some(f) = obj.float_mut()
                {
                    f.z_order = ranks.next().ok_or_else(limit)?;
                    changed = true;
                }
            }
            if changed {
                p.touch();
            }
        }
        Block::Table(t) => {
            for row in &mut t.rows {
                for cell in &mut row.cells {
                    for child in &mut cell.blocks {
                        assign_block(Arc::make_mut(child), ranks)?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn fragment(doc: &mut Document, frag: &mut Fragment, intent: Intent) -> Result<(), CmdError> {
    if matches!(intent, Intent::Preserve) {
        return Ok(());
    }
    let incoming = fragment_ranks(frag)?;
    if incoming.is_empty() {
        return Ok(());
    }
    let mut ranks = reserve(doc, &incoming)?.into_iter();
    for block in &mut frag.blocks {
        assign_block(block, &mut ranks)?;
    }
    for part in frag.parts.values_mut() {
        for block in &mut part.blocks {
            assign_block(Arc::make_mut(block), &mut ranks)?;
        }
    }
    if ranks.next().is_some() {
        return Err(limit());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
