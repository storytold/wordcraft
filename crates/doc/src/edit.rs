//! Range editing on a [`Document`]: insert text, split paragraphs, delete ranges across
//! paragraphs and tables, apply character/paragraph formatting, copy a range to a fragment and
//! paste a fragment.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::para::InlineObject;
use crate::props::{CharProps, ParaProps};
use crate::{Block, Blocks, DocError, Document, Paragraph, Part, PartKind, Path, Pos, Result, para_block};

/// A copied piece of a document: blocks, where the first and last paragraphs may be partial.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Fragment {
    pub blocks: Vec<Block>,
    /// Copies of the text box stories its shapes show, by the source document's part id. Pasting
    /// gives each box a new story made from these.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parts: BTreeMap<u32, Part>,
}

impl Fragment {
    pub fn plain_text(&self) -> String {
        let mut d = Document::new();
        d.body = self.blocks.iter().cloned().map(Arc::new).collect();
        d.plain_text(crate::StoryRef::Body)
    }
    pub fn from_text(s: &str) -> Fragment {
        let lines: Vec<&str> = s.split('\n').collect();
        Fragment {
            blocks: lines.iter().map(|l| Block::Para(Paragraph::with_text(l.trim_end_matches('\r'), CharProps::default()))).collect(),
            ..Default::default()
        }
    }
}

fn order(a: &Pos, b: &Pos) -> (Pos, Pos) {
    if a <= b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) }
}

/// Nesting limit when following text boxes inside text boxes.
const MAX_BOX_DEPTH: usize = 8;
/// Most text box stories one paste may create (hostile documents can fan out).
const MAX_PASTED_BOXES: usize = 4096;

/// Call `f` on every paragraph of a block (tables descended, depth-limited).
pub(crate) fn each_para<'a>(b: &'a Block, depth: usize, f: &mut dyn FnMut(&'a Paragraph)) {
    match b {
        Block::Para(p) => f(p),
        Block::Table(t) if depth < 16 => {
            for cell in t.rows.iter().flat_map(|r| r.cells.iter()) {
                for cb in &cell.blocks {
                    each_para(cb, depth + 1, f);
                }
            }
        }
        Block::Table(_) => {}
    }
}

fn each_para_mut(b: &mut Block, depth: usize, f: &mut dyn FnMut(&mut Paragraph)) {
    match b {
        Block::Para(p) => f(p),
        Block::Table(t) if depth < 16 => {
            for cell in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                for cb in &mut cell.blocks {
                    each_para_mut(Arc::make_mut(cb), depth + 1, f);
                }
            }
        }
        Block::Table(_) => {}
    }
}

/// Whether a paragraph anywhere in `b` (also inside tables) uses paragraph or character style
/// `id` (depth-limited like `each_para`). Table styles are the engine's `table.deleteStyle`'s job.
fn uses_style(b: &Block, id: &str, depth: usize) -> bool {
    let is = |s: &Option<String>| s.as_deref() == Some(id);
    match b {
        Block::Para(p) => is(&p.props.style) || is(&p.mark.style) || p.runs.iter().any(|r| is(&r.props.style)),
        Block::Table(t) if depth < 16 => {
            t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).any(|cb| uses_style(cb, id, depth + 1))
        }
        Block::Table(_) => false,
    }
}

/// Point every paragraph and run in `b` that uses style `from` at `to`; returns how many
/// paragraphs changed.
fn restyle_block(b: &mut Block, from: &str, to: &Option<String>, depth: usize) -> usize {
    let fix = |s: &mut Option<String>| {
        if s.as_deref() == Some(from) {
            s.clone_from(to);
            true
        } else {
            false
        }
    };
    match b {
        Block::Para(p) => {
            let mut changed = fix(&mut p.props.style);
            changed |= fix(&mut p.mark.style);
            for r in &mut p.runs {
                changed |= fix(&mut r.props.style);
            }
            if changed {
                p.normalize();
                p.touch();
            }
            usize::from(changed)
        }
        Block::Table(t) => {
            let mut n = 0;
            if depth < 16 {
                for cell in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    for cb in &mut cell.blocks {
                        if uses_style(cb, from, depth + 1) {
                            n += restyle_block(Arc::make_mut(cb), from, to, depth + 1);
                        }
                    }
                }
            }
            n
        }
    }
}

/// Text box story ids of a paragraph's shapes.
fn box_ids(p: &Paragraph) -> impl Iterator<Item = u32> + '_ {
    p.objects.iter().filter_map(|o| if let InlineObject::Shape { story: Some(id), .. } = o { Some(*id) } else { None })
}

impl Document {
    /// Point every paragraph and run that uses paragraph or character style `from` at `to`
    /// (`None`: the default style), in every story, tables included. Returns how many paragraphs
    /// changed. (Tables' own table styles are retargeted by the engine's table style deletion.)
    pub fn restyle(&mut self, from: &str, to: Option<String>) -> usize {
        let stories: Vec<crate::StoryRef> =
            std::iter::once(crate::StoryRef::Body).chain(self.parts.keys().map(|k| crate::StoryRef::Part(*k))).collect();
        let mut n = 0;
        for s in stories {
            let Ok(bl) = self.story_mut(s) else { continue };
            for b in bl.iter_mut() {
                if uses_style(b, from, 0) {
                    n += restyle_block(Arc::make_mut(b), from, &to, 0);
                }
            }
        }
        n
    }

    /// Insert plain text at `pos` (`\n` inside `s` is a manual line break; use
    /// [`Document::split_paragraph`] for paragraph breaks). Returns the position after it.
    pub fn insert_text(&mut self, pos: &Pos, s: &str, props: &CharProps) -> Result<Pos> {
        let p = self.para_mut(pos.story, &pos.path)?;
        let n = p.insert_text(pos.off, s, props)?;
        Ok(Pos { off: pos.off + n, ..pos.clone() })
    }

    /// Insert an inline object at `pos`; returns the position after it.
    pub fn insert_object(&mut self, pos: &Pos, obj: InlineObject, props: &CharProps) -> Result<Pos> {
        let p = self.para_mut(pos.story, &pos.path)?;
        p.insert_object(pos.off, obj, props)?;
        Ok(Pos { off: pos.off + crate::para::OBJ.len_utf8(), ..pos.clone() })
    }

    /// Split the paragraph at `pos`; returns the start of the new (second) paragraph.
    pub fn split_paragraph(&mut self, pos: &Pos) -> Result<Pos> {
        let i = pos.path.last();
        let tail = self.para_mut(pos.story, &pos.path)?.split_off(pos.off)?;
        let bl = self.container_mut(pos.story, &pos.path)?;
        bl.insert((i + 1).min(bl.len()), para_block(tail));
        Ok(Pos { story: pos.story, path: pos.path.with_last(i + 1), off: 0 })
    }

    /// Insert a block before the block at `path` (same container).
    pub fn insert_block(&mut self, story: crate::StoryRef, path: &Path, block: Block) -> Result<()> {
        let i = path.last();
        let bl = self.container_mut(story, path)?;
        if i > bl.len() {
            return Err(DocError::BadPath(path.to_string()));
        }
        bl.insert(i, Arc::new(block));
        Ok(())
    }

    /// Remove the block at `path` (keeps at least one paragraph in the container).
    pub fn remove_block(&mut self, story: crate::StoryRef, path: &Path) -> Result<Block> {
        let i = path.last();
        let bl = self.container_mut(story, path)?;
        if i >= bl.len() {
            return Err(DocError::BadPath(path.to_string()));
        }
        let b = bl.remove(i);
        if bl.is_empty() || matches!(bl.last().map(|b| &**b), Some(Block::Table(_))) {
            bl.push(para_block(Paragraph::new()));
        }
        Ok(Arc::try_unwrap(b).unwrap_or_else(|a| (*a).clone()))
    }

    /// Paragraph paths between `a` and `b` inclusive (same story), document order.
    pub fn paths_between(&self, a: &Pos, b: &Pos) -> Vec<Path> {
        let (a, b) = order(a, b);
        if a.story != b.story {
            return Vec::new();
        }
        self.para_paths(a.story).into_iter().filter(|p| *p >= a.path && *p <= b.path).collect()
    }

    /// Delete `a..b`. Returns the position where the deletion collapsed to.
    pub fn delete_range(&mut self, a: &Pos, b: &Pos) -> Result<Pos> {
        let (a, b) = order(a, b);
        if a == b {
            return Ok(a);
        }
        if a.story != b.story {
            return Err(DocError::Invalid("range spans stories".into()));
        }
        if a.path == b.path {
            self.para_mut(a.story, &a.path)?.delete(a.off, b.off)?;
            return Ok(a);
        }
        if a.path.parent() == b.path.parent() {
            // Same container: trim the ends, drop the blocks between, join.
            let (ia, ib) = (a.path.last(), b.path.last());
            {
                let pa = self.para_mut(a.story, &a.path)?;
                let end = pa.len();
                pa.delete(a.off, end)?;
            }
            let mut tail = {
                let pb = self.para_mut(b.story, &b.path)?;
                pb.delete(0, b.off)?;
                pb.clone()
            };
            let bl = self.container_mut(a.story, &a.path)?;
            if ib < bl.len() && ia < ib {
                bl.drain(ia + 1..=ib);
            }
            tail.props = ParaProps::default();
            let pa = self.para_mut(a.story, &a.path)?;
            let keep_props = pa.props.clone();
            pa.append(tail);
            pa.props = keep_props;
            return Ok(a);
        }
        // Different containers (range enters or leaves a table): clear text in every paragraph
        // inside; remove whole tables that are entirely covered when at the same level as `a`.
        let paths = self.paths_between(&a, &b);
        for p in paths.iter().rev() {
            let (from, to) = {
                let Some(para) = self.para(a.story, p) else { continue };
                let from = if *p == a.path { a.off } else { 0 };
                let to = if *p == b.path { b.off } else { para.len() };
                (from, to)
            };
            self.para_mut(a.story, p)?.delete(from, to)?;
        }
        // Drop top-level tables strictly between a and b at a's level.
        if a.path.depth() == 0 && b.path.depth() > 0 {
            let first_b = b.path.0.first().copied().unwrap_or(0) as usize;
            let ia = a.path.last();
            let bl = self.container_mut(a.story, &a.path)?;
            let mut i = first_b;
            while i > ia + 1 {
                i -= 1;
                if matches!(bl.get(i).map(|b| &**b), Some(Block::Table(_))) {
                    bl.remove(i);
                }
            }
        }
        Ok(a)
    }

    /// Apply `f` to the character formatting of `a..b`. A collapsed range changes nothing (the
    /// engine keeps "pending" formatting for the caret instead).
    pub fn format_range(&mut self, a: &Pos, b: &Pos, f: &dyn Fn(&mut CharProps)) -> Result<()> {
        let (a, b) = order(a, b);
        for p in self.paths_between(&a, &b) {
            let para = self.para_mut(a.story, &p)?;
            let from = if p == a.path { a.off } else { 0 };
            let to = if p == b.path { b.off } else { para.len() };
            para.format(from, to, f)?;
            // A fully selected paragraph also gets the mark formatted (Word does).
            if to == para.len() && (from == 0 || p != a.path) {
                f(&mut para.mark);
            }
        }
        Ok(())
    }

    /// Apply `f` to the paragraph properties of every paragraph touched by `a..b`.
    pub fn format_paragraphs(&mut self, a: &Pos, b: &Pos, f: &dyn Fn(&mut ParaProps)) -> Result<()> {
        for p in self.paths_between(a, b) {
            let para = self.para_mut(a.story, &p)?;
            f(&mut para.props);
            para.touch();
        }
        Ok(())
    }

    /// Copy `a..b` into a fragment (with copies of the text boxes in it).
    pub fn copy_range(&self, a: &Pos, b: &Pos) -> Fragment {
        let blocks = self.copy_blocks(a, b);
        let parts = self.text_box_parts(&blocks);
        Fragment { blocks, parts }
    }

    /// The whole body as a fragment (with its text boxes), to insert into another document.
    pub fn body_fragment(&self) -> Fragment {
        let blocks: Vec<Block> = self.body.iter().map(|b| (**b).clone()).collect();
        let parts = self.text_box_parts(&blocks);
        Fragment { blocks, parts }
    }

    /// Copies of the text box stories shapes in `blocks` show, nested boxes included.
    fn text_box_parts(&self, blocks: &[Block]) -> BTreeMap<u32, Part> {
        let mut out = BTreeMap::new();
        let mut todo = Vec::new();
        for b in blocks {
            each_para(b, 0, &mut |p| todo.extend(box_ids(p)));
        }
        while let Some(id) = todo.pop() {
            if out.contains_key(&id) {
                continue;
            }
            let Some(part) = self.parts.get(&id).filter(|p| p.kind == PartKind::TextBox) else { continue };
            for b in &part.blocks {
                each_para(b, 0, &mut |p| todo.extend(box_ids(p)));
            }
            out.insert(id, part.clone());
        }
        out
    }

    fn copy_blocks(&self, a: &Pos, b: &Pos) -> Vec<Block> {
        let (a, b) = order(a, b);
        let mut out = Vec::new();
        if a.story != b.story {
            return out;
        }
        if a.path.parent() == b.path.parent() {
            let Some(bl) = self.container(a.story, &a.path) else { return out };
            for i in a.path.last()..=b.path.last() {
                let Some(blk) = bl.get(i) else { break };
                match &**blk {
                    Block::Para(p) => {
                        let mut p = p.clone();
                        let to = if i == b.path.last() { b.off } else { p.len() };
                        let from = if i == a.path.last() { a.off } else { 0 };
                        let _ = p.delete(to.min(p.len()), p.len());
                        let _ = p.delete(0, from.min(p.len()));
                        if i == b.path.last() && to < self.para(a.story, &b.path).map(Paragraph::len).unwrap_or(0) {
                            p.section = None;
                        }
                        out.push(Block::Para(p));
                    }
                    Block::Table(t) => out.push(Block::Table(t.clone())),
                }
            }
            return out;
        }
        for p in self.paths_between(&a, &b) {
            if let Some(para) = self.para(a.story, &p) {
                let mut q = para.clone();
                let to = if p == b.path { b.off } else { q.len() };
                let from = if p == a.path { a.off } else { 0 };
                let _ = q.delete(to.min(q.len()), q.len());
                let _ = q.delete(0, from.min(q.len()));
                q.section = None;
                out.push(Block::Para(q));
            }
        }
        out
    }

    /// Give every text box in `blocks` a new story of its own, copied from `parts` (or, for a
    /// fragment without them, from this document's text box of that id). A box whose story can't
    /// be found keeps no story rather than showing an unrelated one.
    fn adopt_text_boxes(&mut self, blocks: &mut [Block], parts: &BTreeMap<u32, Part>, depth: usize, budget: &mut usize) {
        let mut any = false;
        for b in blocks.iter() {
            each_para(b, 0, &mut |p| any |= box_ids(p).next().is_some());
        }
        if !any {
            return;
        }
        for b in blocks.iter_mut() {
            each_para_mut(b, 0, &mut |p| {
                let mut changed = false;
                for o in p.objects.iter_mut() {
                    if let InlineObject::Shape { story, .. } = o
                        && let Some(old) = *story
                    {
                        *story = self.adopt_text_box(old, parts, depth, budget);
                        changed = true;
                    }
                }
                if changed {
                    p.touch();
                }
            });
        }
    }

    /// A new text box story copied from `old`; its own boxes get new stories too.
    fn adopt_text_box(&mut self, old: u32, parts: &BTreeMap<u32, Part>, depth: usize, budget: &mut usize) -> Option<u32> {
        if depth >= MAX_BOX_DEPTH || *budget == 0 {
            return None;
        }
        let src = parts.get(&old).or_else(|| self.parts.get(&old).filter(|p| p.kind == PartKind::TextBox))?;
        let mut blocks: Vec<Block> = src.blocks.iter().map(|b| (**b).clone()).collect();
        *budget -= 1;
        self.adopt_text_boxes(&mut blocks, parts, depth + 1, budget);
        Some(self.add_part(PartKind::TextBox, blocks.into_iter().map(Arc::new).collect()))
    }

    /// Insert a fragment at `pos`; returns the position after the pasted content.
    pub fn insert_fragment(&mut self, pos: &Pos, frag: &Fragment) -> Result<Pos> {
        let mut blocks = frag.blocks.clone();
        if blocks.is_empty() {
            return Ok(pos.clone());
        }
        if self.para_at(pos).is_none() {
            return Err(DocError::BadPath(pos.path.to_string()));
        }
        // Pasted text boxes get stories of their own (never shared with the copy's source).
        let mut budget = MAX_PASTED_BOXES;
        self.adopt_text_boxes(&mut blocks, &frag.parts, 0, &mut budget);
        // Single paragraph: inline insert keeping runs.
        if blocks.len() == 1
            && let Some(Block::Para(src)) = blocks.first()
        {
            let src = src.clone();
            let target = self.para_mut(pos.story, &pos.path)?;
            let tail = target.split_off(pos.off)?;
            let added = src.len();
            let keep = target.props.clone();
            let sect = tail.section.clone();
            let mut src = src;
            src.section = None;
            target.append(src);
            target.append(tail);
            target.props = keep;
            target.section = sect;
            return Ok(Pos { off: pos.off + added, ..pos.clone() });
        }
        // Multi-block: split, merge first into left part, last into right part.
        let after = self.split_paragraph(pos)?;
        let left = pos.path.clone();
        let first = blocks.remove(0);
        let last = blocks.pop();
        let mut idx = left.last() + 1;
        match first {
            Block::Para(p) => {
                let l = self.para_mut(pos.story, &left)?;
                let keep = l.props.clone();
                let mut p2 = p;
                let pprops = p2.props.clone();
                p2.section = None;
                l.append(p2);
                l.props = if keep.is_empty() { pprops } else { keep };
            }
            t @ Block::Table(_) => {
                self.insert_block(pos.story, &left.with_last(idx), t)?;
                idx += 1;
            }
        }
        for b in blocks {
            self.insert_block(pos.story, &left.with_last(idx), b)?;
            idx += 1;
        }
        let right = after.path.with_last(idx);
        match last {
            Some(Block::Para(p)) => {
                let r = self.para_mut(pos.story, &right)?;
                let tail = std::mem::replace(r, p.clone());
                let off = r.len();
                let sect = tail.section.clone();
                r.append(tail);
                r.section = sect;
                Ok(Pos { story: pos.story, path: right, off })
            }
            Some(t @ Block::Table(_)) => {
                self.insert_block(pos.story, &right, t)?;
                Ok(Pos { story: pos.story, path: right.with_last(idx + 1), off: 0 })
            }
            None => Ok(Pos { story: pos.story, path: right, off: 0 }),
        }
    }

    /// Replace every block of a story.
    pub fn set_story(&mut self, story: crate::StoryRef, blocks: Blocks) -> Result<()> {
        let s = self.story_mut(story)?;
        *s = blocks;
        if s.is_empty() {
            s.push(para_block(Paragraph::new()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StoryRef, Table};

    fn doc(lines: &[&str]) -> Document {
        Document::from_text(&lines.join("\n"))
    }

    #[test]
    fn split_and_join() {
        let mut d = doc(&["Hello world"]);
        let p = d.split_paragraph(&Pos::body(0, 5)).unwrap();
        assert_eq!(p, Pos::body(1, 0));
        assert_eq!(d.plain_text(StoryRef::Body), "Hello\n world");
        let at = d.delete_range(&Pos::body(0, 5), &Pos::body(1, 0)).unwrap();
        assert_eq!(at, Pos::body(0, 5));
        assert_eq!(d.plain_text(StoryRef::Body), "Hello world");
    }

    #[test]
    fn delete_multi() {
        let mut d = doc(&["one", "two", "three"]);
        d.delete_range(&Pos::body(2, 2), &Pos::body(0, 1)).unwrap();
        assert_eq!(d.plain_text(StoryRef::Body), "oree");
        assert_eq!(d.body.len(), 1);
    }

    #[test]
    fn format_across() {
        let mut d = doc(&["abc", "def"]);
        d.format_range(&Pos::body(0, 1), &Pos::body(1, 2), &|c| c.bold = Some(true)).unwrap();
        let p0 = d.para(StoryRef::Body, &Path::top(0)).unwrap();
        assert_eq!(p0.props_of_char(0).bold, None);
        assert_eq!(p0.props_of_char(1).bold, Some(true));
        let p1 = d.para(StoryRef::Body, &Path::top(1)).unwrap();
        assert_eq!(p1.props_of_char(1).bold, Some(true));
        assert_eq!(p1.props_of_char(2).bold, None);
    }

    #[test]
    fn copy_paste_round_trip() {
        let mut d = doc(&["alpha", "beta", "gamma"]);
        let f = d.copy_range(&Pos::body(0, 2), &Pos::body(2, 3));
        assert_eq!(f.plain_text(), "pha\nbeta\ngam");
        let end = d.insert_fragment(&Pos::body(2, 5), &f).unwrap();
        assert_eq!(d.plain_text(StoryRef::Body), "alpha\nbeta\ngammapha\nbeta\ngam");
        assert_eq!(end, Pos::body(4, 3));
        let one = Fragment::from_text("XY");
        let e = d.insert_fragment(&Pos::body(0, 0), &one).unwrap();
        assert_eq!(e, Pos::body(0, 2));
        assert!(d.plain_text(StoryRef::Body).starts_with("XYalpha"));
    }

    fn text_box(d: &mut Document, pos: &Pos, text: &str) -> u32 {
        let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(text, CharProps::default()))]);
        d.insert_object(pos, box_shape(Some(id)), &CharProps::default()).unwrap();
        id
    }

    fn box_shape(story: Option<u32>) -> InlineObject {
        InlineObject::Shape {
            kind: crate::para::ShapeKind::TextBox,
            w: 144.0,
            h: 72.0,
            fill: None,
            stroke: None,
            stroke_width: 0.75,
            float: Default::default(),
            story,
        }
    }

    /// Story ids of the boxes in a body paragraph, in order.
    fn boxes_in(d: &Document, path: &Path) -> Vec<Option<u32>> {
        let p = d.para(StoryRef::Body, path).unwrap();
        p.objects.iter().filter_map(|o| if let InlineObject::Shape { story, .. } = o { Some(*story) } else { None }).collect()
    }

    #[test]
    fn pasted_text_box_gets_its_own_story() {
        let mut d = doc(&["ab", "cd"]);
        let id = text_box(&mut d, &Pos::body(0, 1), "Inside");
        let f = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5));
        assert_eq!(f.parts.keys().copied().collect::<Vec<_>>(), vec![id]);
        // Edit the original after copying: the paste keeps the copied text.
        d.insert_text(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 6 }, " later", &CharProps::default()).unwrap();
        d.insert_fragment(&Pos::body(1, 2), &f).unwrap();
        let pasted = boxes_in(&d, &Path::top(1));
        let [Some(new)] = pasted[..] else { panic!("{pasted:?}") };
        assert_ne!(new, id);
        assert_eq!(d.parts.get(&new).map(|p| p.kind), Some(PartKind::TextBox));
        assert_eq!(d.plain_text(StoryRef::Part(new)), "Inside");
        assert_eq!(d.plain_text(StoryRef::Part(id)), "Inside later");
        // Editing the copy leaves the original alone; a second paste is a third story.
        d.insert_text(&Pos { story: StoryRef::Part(new), path: Path::top(0), off: 0 }, "Copy: ", &CharProps::default()).unwrap();
        assert_eq!(d.plain_text(StoryRef::Part(id)), "Inside later");
        d.insert_fragment(&Pos::body(1, 0), &f).unwrap();
        let again = boxes_in(&d, &Path::top(1));
        assert_eq!(again.len(), 2);
        assert!(again.iter().all(|s| s.is_some_and(|s| s != id)) && again[0] != again[1], "{again:?}");
        // Multi-paragraph pastes too.
        let f2 = d.copy_range(&Pos::body(0, 0), &Pos::body(1, 1));
        let before = d.parts.len();
        d.insert_fragment(&Pos::body(1, 0), &f2).unwrap();
        assert_eq!(d.parts.len(), before + 1);
    }

    #[test]
    fn text_box_pasted_into_another_document_brings_its_text() {
        let mut src = doc(&["x"]);
        let id = text_box(&mut src, &Pos::body(0, 1), "From the source");
        let f = src.copy_range(&Pos::body(0, 0), &Pos::body(0, 1 + 3));
        // The target already has a header with the same part id.
        let mut dst = doc(&["target"]);
        let hdr = dst.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Header text", CharProps::default()))]);
        assert_eq!(hdr, id);
        dst.insert_fragment(&Pos::body(0, 6), &f).unwrap();
        let [Some(new)] = boxes_in(&dst, &Path::top(0))[..] else { panic!() };
        assert_ne!(new, hdr);
        assert_eq!(dst.plain_text(StoryRef::Part(new)), "From the source");
        assert_eq!(dst.plain_text(StoryRef::Part(hdr)), "Header text");
        // The whole body of a document, as Insert › Text from File uses it.
        let mut dst2 = doc(&["t"]);
        dst2.insert_fragment(&Pos::body(0, 1), &src.body_fragment()).unwrap();
        let [Some(n2)] = boxes_in(&dst2, &Path::top(0))[..] else { panic!() };
        assert_eq!(dst2.plain_text(StoryRef::Part(n2)), "From the source");
    }

    #[test]
    fn text_box_in_a_copied_table_gets_its_own_story() {
        let mut d = doc(&["before", "after"]);
        d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(Table::new(1, 1, 200.0))).unwrap();
        let cell = Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 };
        let id = text_box(&mut d, &cell, "In a cell");
        let f = d.copy_range(&Pos::body(0, 0), &Pos::body(2, 5));
        assert!(f.parts.contains_key(&id));
        d.insert_fragment(&Pos::body(2, 5), &f).unwrap();
        let ids: Vec<u32> = d
            .para_paths(StoryRef::Body)
            .iter()
            .filter_map(|p| d.para(StoryRef::Body, p))
            .flat_map(|p| p.objects.iter())
            .filter_map(|o| if let InlineObject::Shape { story, .. } = o { *story } else { None })
            .collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert_ne!(ids[0], ids[1]);
        assert!(ids.iter().all(|i| d.plain_text(StoryRef::Part(*i)) == "In a cell"));
    }

    #[test]
    fn fragment_without_parts_never_shows_an_unrelated_story() {
        let mut d = doc(&["x"]);
        let hdr = d.add_part(PartKind::Header, Vec::new());
        let own = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Mine", CharProps::default()))]);
        let mut p = Paragraph::new();
        p.insert_object(0, box_shape(Some(hdr)), &CharProps::default()).unwrap();
        p.insert_object(0, box_shape(Some(own)), &CharProps::default()).unwrap();
        p.insert_object(0, box_shape(Some(999)), &CharProps::default()).unwrap();
        let f = Fragment { blocks: vec![Block::Para(p)], ..Default::default() };
        d.insert_fragment(&Pos::body(0, 1), &f).unwrap();
        let got = boxes_in(&d, &Path::top(0));
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], None, "missing story");
        let Some(copy) = got[1] else { panic!("{got:?}") };
        assert_ne!(copy, own);
        assert_eq!(d.plain_text(StoryRef::Part(copy)), "Mine");
        assert_eq!(got[2], None, "a header isn't a text box");
    }

    #[test]
    fn self_nested_text_box_paste_is_bounded() {
        let mut d = doc(&["x"]);
        let id = text_box(&mut d, &Pos::body(0, 1), "loop");
        // The box contains a box showing itself.
        d.insert_object(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 }, box_shape(Some(id)), &CharProps::default()).unwrap();
        let f = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 1 + 3));
        let before = d.parts.len();
        d.insert_fragment(&Pos::body(0, 0), &f).unwrap();
        let added = d.parts.len() - before;
        assert!((1..=MAX_BOX_DEPTH).contains(&added), "{added}");
    }

    #[test]
    fn tables_in_paths() {
        let mut d = doc(&["before", "after"]);
        d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(Table::new(2, 2, 200.0))).unwrap();
        let paths = d.para_paths(StoryRef::Body);
        assert_eq!(paths.len(), 6);
        assert_eq!(paths[1], Path(vec![1, 0, 0, 0]));
        let cell = Pos { story: StoryRef::Body, path: Path(vec![1, 0, 1, 0]), off: 0 };
        let e = d.insert_text(&cell, "x", &CharProps::default()).unwrap();
        assert_eq!(e.off, 1);
        assert_eq!(d.next_para(StoryRef::Body, &Path(vec![1, 0, 1, 0])), Some(Path(vec![1, 1, 0, 0])));
        assert!(d.plain_text(StoryRef::Body).contains("\tx"));
        // Delete from before the table into a cell: text cleared, table kept (partially covered).
        d.delete_range(&Pos::body(0, 2), &cell).unwrap();
        assert_eq!(d.para(StoryRef::Body, &Path::top(0)).unwrap().text, "be");
        assert!(d.container(StoryRef::Body, &Path(vec![1, 0, 0, 0])).is_some());
        assert_eq!(Path(vec![1, 0, 1, 0]).cell(), Some((Path::top(1), 0, 1)));
    }

    #[test]
    fn bad_positions_error() {
        let mut d = doc(&["é"]);
        assert!(d.insert_text(&Pos::body(0, 1), "x", &CharProps::default()).is_err());
        assert!(d.insert_text(&Pos::body(5, 0), "x", &CharProps::default()).is_err());
        assert!(d.split_paragraph(&Pos::body(0, 99)).is_err());
        let c = d.clamp(&Pos::body(9, 9));
        assert_eq!(c, Pos::body(0, 2));
        assert!(d.delete_range(&Pos { story: StoryRef::Part(3), path: Path::top(0), off: 0 }, &Pos::body(0, 0)).is_err());
    }
}
