//! Bringing a revised document's pictures, lists, styles and tracked changes into the original
//! for Compare / Combine (#1317). The content is pointed at the copies by [`Imported`].

use std::collections::BTreeMap;
use std::sync::Arc;

use wordcraft_doc::{Block, Document, Paragraph};

use crate::cmd::paste::each_para;

const MAX_DEPTH: usize = 64;

/// Point a tracked formatting change at its renumbered revision (dropped when it has none).
fn remap_change<T>(c: &mut Option<Box<wordcraft_doc::props::PropChange<T>>>, rev: &dyn Fn(&mut Option<u32>)) {
    let mut r = c.as_ref().map(|c| c.rev);
    rev(&mut r);
    match (r, c.as_mut()) {
        (Some(v), Some(x)) => x.rev = v,
        _ => *c = None,
    }
}

/// [`remap_change`] for the table, row and cell formatting changes in `b` (bounded depth).
fn remap_table_changes(b: &mut Block, depth: usize, rev: &dyn Fn(&mut Option<u32>)) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Block::Table(t) = b {
        remap_change(&mut t.props.fmt_change, rev);
        for row in &mut t.rows {
            rev(&mut row.props.ins);
            rev(&mut row.props.del);
            remap_change(&mut row.props.fmt_change, rev);
            for cell in &mut row.cells {
                remap_change(&mut cell.props.fmt_change, rev);
                for cb in &mut cell.blocks {
                    remap_table_changes(Arc::make_mut(cb), depth + 1, rev);
                }
            }
        }
    }
}

/// How content from another document is pointed at the resources [`import_resources`] brought
/// into this one: media keys, list ids and revision numbers.
pub(super) struct Imported {
    media: BTreeMap<String, String>,
    nums: BTreeMap<u32, u32>,
    rev_base: u32,
    src_revs: usize,
}

impl Imported {
    /// The source's revision `i` as ours (`None` when the source has no such revision).
    fn rev(&self, i: &mut Option<u32>) {
        *i = i.filter(|v| (*v as usize) < self.src_revs).and_then(|v| self.rev_base.checked_add(v));
    }

    fn remap_object(&self, o: &mut wordcraft_doc::InlineObject, depth: usize) {
        match o {
            wordcraft_doc::InlineObject::Image { media: key, .. } => {
                if let Some(k) = self.media.get(key.as_str()) {
                    *key = k.clone();
                }
            }
            wordcraft_doc::InlineObject::Group { children, .. } if depth < MAX_DEPTH => {
                for c in children {
                    self.remap_object(&mut c.obj, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// Point a paragraph from the source at our copies of its lists, revisions and pictures.
    pub(super) fn remap_para(&self, p: &mut Paragraph) {
        let rev = |i: &mut Option<u32>| self.rev(i);
        if let Some(n) = p.props.numbering.as_mut()
            && n.num != 0
        {
            n.num = self.nums.get(&n.num).copied().unwrap_or(0);
        }
        for r in &mut p.runs {
            rev(&mut r.props.ins);
            rev(&mut r.props.del);
            remap_change(&mut r.props.fmt_change, &rev);
        }
        rev(&mut p.mark.ins);
        rev(&mut p.mark.del);
        remap_change(&mut p.mark.fmt_change, &rev);
        remap_change(&mut p.props.fmt_change, &rev);
        let mut n = p.props.num_change.as_ref().map(|c| c.rev);
        rev(&mut n);
        match (n, p.props.num_change.as_mut()) {
            (Some(v), Some(c)) => c.rev = v,
            _ => p.props.num_change = None,
        }
        for o in &mut p.objects {
            self.remap_object(o, 0);
        }
    }

    /// [`Self::remap_para`] for every paragraph in a block, and its tables' tracked changes.
    pub(super) fn remap_block(&self, b: &mut Block) {
        each_para(b, 0, &mut |p| self.remap_para(p));
        remap_table_changes(b, 0, &|r| self.rev(r));
    }
}

/// Bring the media, lists, styles and revisions of `src` into `doc`, renumbered so they can't
/// collide with `doc`'s own; styles `doc` already has keep its definition. With `reuse`, a list
/// `doc` has with the same id and definition is shared instead of copied (compared documents
/// usually share their lists). Pictures with the same bytes always share one copy.
pub(super) fn import_resources(doc: &mut Document, src: &Document, reuse: bool) -> Imported {
    let mut media = BTreeMap::new();
    for (key, data) in &src.media {
        let ext = key.rsplit_once('.').map(|(_, e)| e).unwrap_or("png");
        media.insert(key.clone(), doc.add_media(data.to_vec(), ext));
    }
    let mut abs_map = BTreeMap::new();
    let mut next_abs = doc.numbering.abstracts.iter().map(|a| a.id.saturating_add(1)).max().unwrap_or(0);
    for a in &src.numbering.abstracts {
        if reuse && doc.numbering.abstracts.contains(a) {
            abs_map.insert(a.id, a.id);
            continue;
        }
        let id = next_abs;
        next_abs = next_abs.saturating_add(1);
        abs_map.insert(a.id, id);
        Arc::make_mut(&mut doc.numbering).abstracts.push(wordcraft_doc::numbering::AbstractNum { id, ..a.clone() });
    }
    let mut nums = BTreeMap::new();
    let mut next_num = doc.numbering.nums.iter().map(|n| n.id.saturating_add(1)).max().unwrap_or(1).max(1);
    for n in &src.numbering.nums {
        let abstract_id = abs_map.get(&n.abstract_id).copied().unwrap_or(n.abstract_id);
        let num = wordcraft_doc::numbering::Num { abstract_id, ..n.clone() };
        if reuse && doc.numbering.nums.contains(&num) {
            nums.insert(n.id, n.id);
            continue;
        }
        let id = next_num;
        next_num = next_num.saturating_add(1);
        nums.insert(n.id, id);
        Arc::make_mut(&mut doc.numbering).nums.push(wordcraft_doc::numbering::Num { id, ..num });
    }
    for st in &src.styles.styles {
        if doc.styles.get(&st.id).is_none() {
            Arc::make_mut(&mut doc.styles).upsert(st.clone());
        }
    }
    // Tracked changes the source carries point at its own revisions: add them after ours and
    // renumber.
    let rev_base = u32::try_from(doc.revisions.len()).unwrap_or(u32::MAX);
    doc.revisions.extend(src.revisions.iter().cloned());
    Imported { media, nums, rev_base, src_revs: src.revisions.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1317 review: tracked row insertions and deletions follow the revisions' new indices.
    #[test]
    fn row_revision_marks_are_remapped_on_import() {
        let mut t = wordcraft_doc::Table::new(2, 1, 100.0);
        t.rows[0].props.ins = Some(0);
        t.rows[1].props.del = Some(1);
        let mut b = Block::Table(t);
        remap_table_changes(&mut b, 0, &|r| *r = r.map(|v| v + 5));
        let Block::Table(t) = b else { panic!("table") };
        assert_eq!(t.rows[0].props.ins, Some(5));
        assert_eq!(t.rows[1].props.del, Some(6));
    }
}
