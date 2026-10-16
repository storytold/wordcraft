//! Range fields: a [`InlineObject::FieldStart`] … [`InlineObject::FieldEnd`] pair whose result is
//! the ordinary content between the two markers, possibly spanning paragraphs (a Zotero
//! bibliography). Pairs nest like brackets and never cross a block list: a pair opened in a
//! table cell closes in that cell.
//!
//! Editing can leave a marker without its partner (deleting a selection that holds only one
//! end). Such orphans are harmless in the model and in layout; writers call
//! [`Document::balance_field_ranges`] so a saved file always has whole fields.

use crate::para::OBJ;
use crate::{Block, Blocks, Document, InlineObject, Path, Pos, StoryRef};

/// Deepest table nesting scanned (matches `walk` in the crate root).
const MAX_DEPTH: usize = 16;

/// A whole range field.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldRange {
    pub instr: String,
    pub locked: bool,
    pub dirty: bool,
    /// Where the `FieldStart` marker is.
    pub start: Pos,
    /// Where the `FieldEnd` marker is (the result is everything between the two).
    pub end: Pos,
    /// Number of enclosing range fields.
    pub depth: usize,
}

#[derive(Default)]
struct Scan {
    ranges: Vec<FieldRange>,
    orphans: Vec<(Path, usize)>,
}

fn scan(story: StoryRef, bl: &Blocks, prefix: &mut Vec<u32>, depth: usize, out: &mut Scan) {
    if depth > MAX_DEPTH {
        return;
    }
    // Open starts: (index into out.ranges once closed is unknown, so keep the data here).
    let mut open: Vec<(String, bool, bool, Pos)> = Vec::new();
    for (i, b) in bl.iter().enumerate() {
        prefix.push(u32::try_from(i).unwrap_or(u32::MAX));
        match &**b {
            Block::Para(p) => {
                let path = Path(prefix.clone());
                for off in p.object_offsets() {
                    match p.object_at(off) {
                        Some(InlineObject::FieldStart { instr, locked, dirty }) => {
                            open.push((instr.clone(), *locked, *dirty, Pos::new(story, path.clone(), off)));
                        }
                        Some(InlineObject::FieldEnd) => match open.pop() {
                            Some((instr, locked, dirty, start)) => {
                                let depth = open.len();
                                out.ranges.push(FieldRange { instr, locked, dirty, start, end: Pos::new(story, path.clone(), off), depth });
                            }
                            None => out.orphans.push((path.clone(), off)),
                        },
                        _ => {}
                    }
                }
            }
            Block::Table(t) => {
                for (r, row) in t.rows.iter().enumerate() {
                    for (c, cell) in row.cells.iter().enumerate() {
                        prefix.push(u32::try_from(r).unwrap_or(u32::MAX));
                        prefix.push(u32::try_from(c).unwrap_or(u32::MAX));
                        scan(story, &cell.blocks, prefix, depth + 1, out);
                        prefix.pop();
                        prefix.pop();
                    }
                }
            }
        }
        prefix.pop();
    }
    out.orphans.extend(open.into_iter().map(|(_, _, _, p)| (p.path, p.off)));
}

fn scan_story(story: StoryRef, bl: &Blocks) -> Scan {
    let mut out = Scan::default();
    scan(story, bl, &mut Vec::new(), 0, &mut out);
    out.ranges.sort_by(|a, b| a.start.path.cmp(&b.start.path).then(a.start.off.cmp(&b.start.off)));
    out
}

impl Document {
    fn story_refs(&self) -> Vec<StoryRef> {
        std::iter::once(StoryRef::Body).chain(self.parts.keys().map(|k| StoryRef::Part(*k))).collect()
    }

    /// The whole range fields of a story, in document order of their starts.
    pub fn field_ranges(&self, story: StoryRef) -> Vec<FieldRange> {
        self.story(story).map(|bl| scan_story(story, bl).ranges).unwrap_or_default()
    }

    /// `p` moved past the range-field ends right after it. A click just after a field's result
    /// lands before its end marker (the marker takes no room); typing or inserting there belongs
    /// after the field, as in Word, not in its result.
    pub fn past_field_ends(&self, p: &Pos) -> Pos {
        let Some(para) = self.para_at(p) else { return p.clone() };
        let mut off = p.off;
        while matches!(para.object_at(off), Some(InlineObject::FieldEnd)) {
            off += OBJ.len_utf8();
        }
        Pos { off, ..p.clone() }
    }

    /// Whether any story holds a range-field marker without its partner.
    pub fn has_unbalanced_field_ranges(&self) -> bool {
        self.story_refs().into_iter().any(|s| self.story(s).is_some_and(|bl| !scan_story(s, bl).orphans.is_empty()))
    }

    /// Remove range-field markers that have no partner. Returns how many were removed.
    pub fn balance_field_ranges(&mut self) -> usize {
        let mut removed = 0;
        for s in self.story_refs() {
            let Some(bl) = self.story(s) else { continue };
            let mut orphans = scan_story(s, bl).orphans;
            // Last first, so earlier offsets in the same paragraph stay valid.
            orphans.sort();
            for (path, off) in orphans.into_iter().rev() {
                if let Ok(p) = self.para_mut(s, &path)
                    && p.delete(off, off + OBJ.len_utf8()).is_ok()
                {
                    removed += 1;
                }
            }
        }
        removed
    }
}
