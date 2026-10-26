//! Range editing on a [`Document`]: insert text, split paragraphs, delete ranges across
//! paragraphs and tables, apply character/paragraph formatting, copy a range to a fragment and
//! paste a fragment.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::para::{InlineObject, Run};
use crate::props::{CharProps, ParaProps};
use crate::{Block, Blocks, DocError, Document, Paragraph, Part, PartKind, Path, Pos, Result, para_block};
use crate::{Table, table::MAX_TABLE_DEPTH};

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
        self.to_document().plain_text(crate::StoryRef::Body)
    }
    /// The fragment as the body of a new document (default styles).
    pub fn to_document(&self) -> Document {
        let mut d = Document::new();
        d.body = self.blocks.iter().cloned().map(Arc::new).collect();
        d
    }
    /// Whether the fragment holds an equation (in tables too), which a copy also puts on the
    /// system clipboard as MathML (#816).
    pub fn has_equation(&self) -> bool {
        let mut found = false;
        for b in &self.blocks {
            each_para(b, 0, &mut |p| found |= p.objects.iter().any(|o| matches!(o, InlineObject::Equation { .. })));
        }
        found
    }
    /// The equation (its linear format and whether it is a display equation) when the fragment
    /// is that one equation and nothing else but white space and empty paragraphs: a copy of it
    /// also goes on the system clipboard in the MathML formats that equation editors read (#1297).
    pub fn sole_equation(&self) -> Option<(&str, bool)> {
        let mut found = None;
        for b in &self.blocks {
            let Block::Para(p) = b else { return None };
            for c in p.text.chars().filter(|c| !c.is_whitespace()) {
                if c != crate::para::OBJ || found.is_some() {
                    return None;
                }
                // The paragraph's only visible character, so its first object.
                match p.objects.first() {
                    Some(InlineObject::Equation { linear, display, .. }) => found = Some((linear.as_str(), *display)),
                    _ => return None,
                }
            }
        }
        found
    }
    /// The text a copy puts on the clipboard: [`Self::plain_text`] with symbol-font characters
    /// (Wingdings…) as their Unicode equivalents.
    pub fn clipboard_text(&self) -> String {
        self.to_document().shown_text(crate::StoryRef::Body)
    }
    /// Add `piece` at the end, followed by a paragraph mark unless it already ends with one: how
    /// Word's Spike collects what it cuts. The piece's text box stories get ids after this
    /// fragment's, so pieces copied from different documents (or after an id was freed and reused)
    /// never share one.
    pub fn append_piece(&mut self, mut piece: Fragment) {
        let Some(last) = piece.blocks.last() else { return };
        let ends_with_mark = piece.blocks.len() >= 2 && matches!(last, Block::Para(p) if p.is_empty());
        if !ends_with_mark {
            let mut mark = Paragraph::new();
            if let Block::Para(p) = last {
                mark.props = p.props.clone();
            }
            piece.blocks.push(Block::Para(mark));
        }
        let next = self.parts.keys().next_back().map_or(1, |k| k.saturating_add(1));
        let ids: BTreeMap<u32, u32> = piece.parts.keys().zip(next..=u32::MAX).map(|(old, new)| (*old, new)).collect();
        let renumber = |b: &mut Block| {
            each_para_mut(b, 0, &mut |p| {
                let mut changed = false;
                for o in p.objects.iter_mut() {
                    for slot in o.text_box_slots() {
                        if let Some(new) = slot.and_then(|old| ids.get(&old)) {
                            *slot = Some(*new);
                            changed = true;
                        }
                    }
                }
                if changed {
                    p.touch();
                }
            })
        };
        piece.blocks.iter_mut().for_each(renumber);
        for (old, mut part) in std::mem::take(&mut piece.parts) {
            part.blocks.iter_mut().map(Arc::make_mut).for_each(renumber);
            if let Some(new) = ids.get(&old) {
                self.parts.insert(*new, part);
            }
        }
        // The empty paragraph after the last mark is where the next piece starts.
        if self.blocks.len() >= 2 && matches!(self.blocks.last(), Some(Block::Para(p)) if p.is_empty()) {
            self.blocks.pop();
        }
        self.blocks.append(&mut piece.blocks);
    }
    pub fn from_text(s: &str) -> Fragment {
        let lines: Vec<&str> = s.split('\n').collect();
        Fragment {
            blocks: lines.iter().map(|l| Block::Para(Paragraph::with_text(l.trim_end_matches('\r'), CharProps::default()))).collect(),
            ..Default::default()
        }
    }
}

/// Whether block `b`, in a block list the paragraph walk reaches at `depth`, has cells the walk
/// skips ([`crate::PARA_WALK_DEPTH`]). Bounded by that depth.
fn deeper_than_walk(b: &Block, depth: usize) -> bool {
    match b {
        Block::Para(_) => false,
        Block::Table(t) => t
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .any(|c| depth + 1 > crate::PARA_WALK_DEPTH || c.blocks.iter().any(|x| deeper_than_walk(x, depth + 1))),
    }
}

fn order(a: &Pos, b: &Pos) -> (Pos, Pos) {
    if a <= b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) }
}

/// The blocks in `t`'s cells, row by row.
fn cell_blocks(t: &Table) -> impl DoubleEndedIterator<Item = &Block> {
    t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|b| &**b)
}

/// How many tables deep `t` goes, itself included: 1 when its cells hold no table (without
/// recursion: a pasted fragment may be deep).
fn table_height(t: &Table) -> usize {
    let mut most = 1;
    let mut stack: Vec<(&Block, usize)> = cell_blocks(t).map(|b| (b, 1)).collect();
    while let Some((b, d)) = stack.pop() {
        if let Block::Table(inner) = b {
            let d = d.saturating_add(1);
            most = most.max(d);
            stack.extend(cell_blocks(inner).map(|cb| (cb, d)));
        }
    }
    most
}

/// `b` as blocks nesting at most `room` tables deep: a table that would go deeper is replaced by
/// its paragraphs (tables in its cells too, document order), as the DOCX reader does.
fn fit_tables(b: Block, room: usize, out: &mut Vec<Block>) {
    match b {
        Block::Table(t) if room == 0 => {
            let mut stack: Vec<&Block> = cell_blocks(&t).rev().collect();
            while let Some(cb) = stack.pop() {
                match cb {
                    Block::Para(p) => out.push(Block::Para(p.clone())),
                    Block::Table(inner) => stack.extend(cell_blocks(inner).rev()),
                }
            }
        }
        Block::Table(mut t) if table_height(&t) > room => {
            for cell in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                let mut kept = Vec::new();
                for cb in std::mem::take(&mut cell.blocks) {
                    fit_tables(Arc::unwrap_or_clone(cb), room.saturating_sub(1), &mut kept);
                }
                cell.blocks = kept.into_iter().map(Arc::new).collect();
            }
            out.push(Block::Table(t));
        }
        b => out.push(b),
    }
}

/// Nesting limit when following text boxes inside text boxes.
const MAX_BOX_DEPTH: usize = 8;
/// Most text box stories one paste may create (hostile documents can fan out).
const MAX_PASTED_BOXES: usize = 4096;
/// Most text box content (estimated bytes, see [`block_cost`]) one paste may adopt: thousands of
/// boxes can show the same large story, and each needs a story of its own.
const MAX_PASTED_BOX_BYTES: usize = 32 << 20;

/// What a paste may still spend on text box stories.
struct BoxBudget {
    stories: usize,
    bytes: usize,
    /// The source stories being copied, innermost last: a box inside one of them showing it
    /// again is a cycle.
    active: Vec<u32>,
    /// Each source story's cost ([`block_cost`]), worked out once per paste: a story too big for
    /// what is left is not walked again for every box that shows it.
    costs: BTreeMap<u32, usize>,
    /// Boxes left without a story because the paste's budget couldn't hold theirs.
    skipped: usize,
}

impl BoxBudget {
    fn new() -> BoxBudget {
        BoxBudget { stories: MAX_PASTED_BOXES, bytes: MAX_PASTED_BOX_BYTES, active: Vec::new(), costs: BTreeMap::new(), skipped: 0 }
    }
}

/// Rough size of a block's content, and whether it holds text boxes (and so must be copied).
fn block_cost(b: &Block) -> (usize, bool) {
    let (mut cost, mut boxes) = (std::mem::size_of::<Block>(), false);
    each_para(b, 0, &mut |p| {
        let items = p
            .runs
            .len()
            .saturating_mul(std::mem::size_of::<Run>())
            .saturating_add(p.objects.len().saturating_mul(std::mem::size_of::<InlineObject>()));
        let objects = p.objects.iter().fold(0usize, |n, o| n.saturating_add(object_cost(o)));
        cost = cost.saturating_add(p.text.len()).saturating_add(items).saturating_add(objects);
        boxes |= box_ids(p).next().is_some();
    });
    (cost, boxes)
}

/// Rough heap bytes a clone of `o` copies beyond its own storage: its strings and lists, and its
/// group members however deep. What it shares through `Arc` (pictures' embedded objects, charts,
/// freeforms, wrap polygons, gradients) is not copied, so not counted.
fn object_cost(o: &InlineObject) -> usize {
    let float = |f: &crate::para::Float| f.alt.text.len().saturating_add(f.alt.title.len());
    let mut cost = 0usize;
    let mut todo = vec![o];
    while let Some(o) = todo.pop() {
        let add = match o {
            InlineObject::Image { media, alt, float: f, .. } => media.len().saturating_add(alt.len()).saturating_add(float(f)),
            InlineObject::Graphic { alt, float: f, .. } => alt.len().saturating_add(float(f)),
            InlineObject::Shape { float: f, extra, .. } => {
                let warp = extra.warp.as_ref().map_or(0, |w| {
                    let adj = w.adj.iter().map(|(name, _)| std::mem::size_of::<(String, i64)>().saturating_add(name.len()));
                    adj.fold(w.preset.len(), usize::saturating_add)
                });
                float(f).saturating_add(warp)
            }
            InlineObject::Group { float: f, children, .. } => {
                todo.extend(children.iter().map(|c| &c.obj));
                children.len().saturating_mul(std::mem::size_of::<crate::para::GroupChild>()).saturating_add(float(f))
            }
            InlineObject::Field { instr, result, form, .. } => instr.len().saturating_add(result.len()).saturating_add(form.len()),
            InlineObject::FieldStart { instr, .. } => instr.len(),
            InlineObject::ControlStart { control } => std::mem::size_of::<crate::control::ContentControl>().saturating_add(serialized_len(control)),
            InlineObject::NoteRef { custom, .. } => custom.len(),
            InlineObject::BookmarkStart { name } | InlineObject::BookmarkEnd { name } => name.len(),
            InlineObject::PermStart { group, editor, .. } => group.len().saturating_add(editor.len()),
            InlineObject::Equation { linear, math, .. } => linear.len().saturating_add(math.heap_bytes()),
            InlineObject::Ruby { base, ruby, lang, props, .. } => {
                [base.len(), ruby.len(), lang.len(), std::mem::size_of::<CharProps>(), serialized_len(props)]
                    .into_iter()
                    .fold(0, usize::saturating_add)
            }
            InlineObject::Opaque { format, xml, text } => format.len().saturating_add(xml.len()).saturating_add(text.len()),
            InlineObject::FieldEnd
            | InlineObject::ControlEnd
            | InlineObject::CommentStart { .. }
            | InlineObject::CommentEnd { .. }
            | InlineObject::PermEnd { .. }
            | InlineObject::PositionalTab { .. } => 0,
        };
        cost = cost.saturating_add(add);
    }
    cost
}

/// Length of `v` written as JSON: at least the bytes of every string and list it owns. For the
/// nested formatting and control types, which share nothing through `Arc`.
fn serialized_len<T: Serialize>(v: &T) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(b.len());
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut n = Count(0);
    let _ = serde_json::to_writer(&mut n, v);
    n.0
}

/// Call `f` on every paragraph of a block, in document order, however deep its tables nest. The
/// walk keeps its own stack instead of recursing, so it is complete: a text box it doesn't reach
/// is shown nowhere, which pruning relies on (#1339). `_depth` is unused (callers keep passing it).
pub(crate) fn each_para<'a>(b: &'a Block, _depth: usize, f: &mut dyn FnMut(&'a Paragraph)) {
    // Most blocks are paragraphs, which need no stack.
    if let Block::Para(p) = b {
        return f(p);
    }
    let mut todo = vec![b];
    while let Some(b) = todo.pop() {
        match b {
            Block::Para(p) => f(p),
            Block::Table(t) => {
                // Last first, so they come off the stack in document order.
                let cells = t.rows.iter().rev().flat_map(|r| r.cells.iter().rev());
                todo.extend(cells.flat_map(|c| c.blocks.iter().rev()).map(|cb| &**cb));
            }
        }
    }
}

/// [`each_para`], changing the paragraphs.
pub(crate) fn each_para_mut(b: &mut Block, _depth: usize, f: &mut dyn FnMut(&mut Paragraph)) {
    if let Block::Para(p) = b {
        return f(p);
    }
    let mut todo = vec![b];
    while let Some(b) = todo.pop() {
        match b {
            Block::Para(p) => f(p),
            Block::Table(t) => {
                let cells = t.rows.iter_mut().rev().flat_map(|r| r.cells.iter_mut().rev());
                todo.extend(cells.flat_map(|c| c.blocks.iter_mut().rev()).map(Arc::make_mut));
            }
        }
    }
}

/// Whether a paragraph anywhere in `b` (also inside tables) uses paragraph or character style
/// `id` (tables to 16 levels deep). Table styles are the engine's `table.deleteStyle`'s job.
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
    p.objects.iter().flat_map(InlineObject::text_boxes)
}

impl Document {
    /// Convert notes between footnotes and endnotes: footnotes (`foot`) become endnotes and
    /// endnotes (`end`) footnotes, both at once to swap them. Each note's kind changes, as do its
    /// reference marks wherever they are (the body, text boxes, the note's own mark) and the
    /// note styles (Footnote Text and Footnote Reference to their endnote twins and back).
    /// Returns how many notes changed.
    pub fn convert_notes(&mut self, foot: bool, end: bool) -> usize {
        use crate::para::NoteKind;
        let flips: std::collections::HashSet<u32> = self
            .parts
            .iter()
            .filter(|(_, p)| (foot && p.kind == PartKind::Footnote) || (end && p.kind == PartKind::Endnote))
            .map(|(id, _)| *id)
            .collect();
        if flips.is_empty() {
            return 0;
        }
        let swap = |s: &mut Option<String>| {
            let to = match s.as_deref() {
                Some("FootnoteText") => "EndnoteText",
                Some("EndnoteText") => "FootnoteText",
                Some("FootnoteReference") => "EndnoteReference",
                Some("EndnoteReference") => "FootnoteReference",
                _ => return,
            };
            *s = Some(to.to_string());
        };
        let refers = |p: &Paragraph| p.objects.iter().any(|o| matches!(o, InlineObject::NoteRef { id, .. } if flips.contains(id)));
        // In a paragraph: flip the converted notes' marks and their runs' reference style; in a
        // converted note's own story also the paragraph style.
        let convert = |p: &mut Paragraph, in_note: bool| {
            if !in_note && !refers(p) {
                return;
            }
            let offs = p.object_offsets();
            let mut at = Vec::new();
            for (k, o) in p.objects.iter_mut().enumerate() {
                if let InlineObject::NoteRef { kind, id, .. } = o
                    && flips.contains(id)
                {
                    *kind = if *kind == NoteKind::Footnote { NoteKind::Endnote } else { NoteKind::Footnote };
                    at.extend(offs.get(k).copied());
                }
            }
            let mut start = 0usize;
            for r in &mut p.runs {
                let end = start.saturating_add(r.len);
                if at.iter().any(|&o| o >= start && o < end) {
                    swap(&mut r.props.style);
                }
                start = end;
            }
            if in_note {
                swap(&mut p.props.style);
                swap(&mut p.mark.style);
            }
            p.touch();
        };
        let touches = |b: &Block| {
            let mut any = false;
            each_para(b, 0, &mut |p| any |= refers(p));
            any
        };
        for b in self.body.iter_mut() {
            if touches(b) {
                each_para_mut(Arc::make_mut(b), 0, &mut |p| convert(p, false));
            }
        }
        for (id, part) in self.parts.iter_mut() {
            let own = flips.contains(id);
            if own {
                part.kind = if part.kind == PartKind::Footnote { PartKind::Endnote } else { PartKind::Footnote };
            }
            for b in part.blocks.iter_mut() {
                if own || touches(b) {
                    each_para_mut(Arc::make_mut(b), 0, &mut |p| convert(p, own));
                }
            }
        }
        flips.len()
    }

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

    /// Insert a block before the block at `path` (same container). A table that would nest
    /// deeper than [`MAX_TABLE_DEPTH`] is refused.
    pub fn insert_block(&mut self, story: crate::StoryRef, path: &Path, block: Block) -> Result<()> {
        if let Block::Table(t) = &block
            && path.depth().saturating_add(table_height(t)) > MAX_TABLE_DEPTH
        {
            return Err(DocError::Invalid(format!("tables can't nest more than {MAX_TABLE_DEPTH} deep")));
        }
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
        // Only the blocks the range reaches are walked (not every paragraph of the story).
        let mut out = Vec::new();
        if let Some(bl) = self.story(a.story) {
            crate::walk_between(bl, &mut Vec::new(), &mut out, 0, (&a.path.0, &b.path.0));
        }
        out
    }

    /// Delete `a..b`. Returns the position where the deletion collapsed to. On error the document
    /// is unchanged: both ends are checked before anything is deleted.
    pub fn delete_range(&mut self, a: &Pos, b: &Pos) -> Result<Pos> {
        let (a, b) = order(a, b);
        if a == b {
            return Ok(a);
        }
        if a.story != b.story {
            return Err(DocError::Invalid("range spans stories".into()));
        }
        // Check both ends before the first edit, so an error never leaves the document half-deleted.
        for p in [&a, &b] {
            match self.para(p.story, &p.path) {
                Some(para) if para.clamp(p.off) == p.off => {}
                Some(_) => return Err(DocError::BadOffset(p.off)),
                None => return Err(DocError::BadPath(p.path.to_string())),
            }
        }
        // Content control markers whose partner is outside the range stay (the control is
        // emptied, not broken).
        let mut kept = self.kept_control_markers(&a, &b);
        // The walk below groups `kept` by paragraph (`chunk_by`) and looks offsets up
        // (`binary_search`), so it must be sorted; `kept_control_markers` returns it so, and this
        // keeps a change there from quietly losing markers.
        if !kept.is_sorted() {
            kept.sort();
        }
        let kept_in = |path: &Path| -> Vec<usize> { kept.iter().filter(|(p, _)| p == path).map(|(_, o)| *o).collect() };
        if a.path == b.path {
            self.para_mut(a.story, &a.path)?.delete_except(a.off, b.off, &kept_in(&a.path))?;
            return Ok(a);
        }
        if a.path.parent() == b.path.parent() {
            // Same container: trim the ends, drop the blocks between, join.
            let (ia, ib) = (a.path.last(), b.path.last());
            // Kept markers of the paragraphs in between move to the join.
            let mut moved: Vec<(InlineObject, CharProps)> = Vec::new();
            // `kept` is sorted: one walk of each paragraph's objects and runs (#3614).
            for group in kept.chunk_by(|x, y| x.0 == y.0) {
                let Some((path, _)) = group.first() else { continue };
                if *path == a.path || *path == b.path {
                    continue;
                }
                let Some(p) = self.para(a.story, path) else { continue };
                let mut runs = p.run_ranges().peekable();
                for (off, o) in p.objects_with_offsets() {
                    if group.binary_search_by_key(&off, |(_, k)| *k).is_err() {
                        continue;
                    }
                    while runs.next_if(|(r, _)| r.end <= off).is_some() {}
                    let props = runs.peek().filter(|(r, _)| r.contains(&off)).map_or(&p.mark, |(_, c)| *c);
                    moved.push((o.clone(), props.clone()));
                }
            }
            {
                let pa = self.para_mut(a.story, &a.path)?;
                let end = pa.len();
                pa.delete_except(a.off, end, &kept_in(&a.path))?;
                for (o, props) in moved {
                    let at = pa.len();
                    pa.insert_object(at, o, &props)?;
                }
            }
            let mut tail = {
                let pb = self.para_mut(b.story, &b.path)?;
                pb.delete_except(0, b.off, &kept_in(&b.path))?;
                pb.clone()
            };
            let bl = self.container_mut(a.story, &a.path)?;
            // Section breaks on the marks in between go with their paragraphs.
            let between: Vec<_> = bl
                .get(ia + 1..ib)
                .unwrap_or_default()
                .iter()
                .filter_map(|b| match &**b {
                    Block::Para(p) => p.section.clone(),
                    Block::Table(_) => None,
                })
                .collect();
            if ib < bl.len() && ia < ib {
                bl.drain(ia + 1..=ib);
            }
            tail.props = ParaProps::default();
            // The deleted marks include `a`'s, so a section break there goes too: the joined
            // paragraph ends where `b`'s mark did, and its text takes the following section's
            // setup, as in Word (#2127).
            let sect = tail.section.take();
            let pa = self.para_mut(a.story, &a.path)?;
            let keep_props = pa.props.clone();
            pa.append(tail);
            pa.props = keep_props;
            let before = std::mem::replace(&mut pa.section, sect);
            if a.story == crate::StoryRef::Body && a.path.0.len() == 1 {
                // Nearest first, as "link to previous" resolved them before the delete.
                let following = self.section_mut(a.path.last());
                for s in between.iter().rev().chain(before.iter()) {
                    following.absorb(s);
                }
            }
            return Ok(a);
        }
        // Different containers (range enters or leaves a table): clear text in every paragraph
        // inside, then remove the tables it covers whole (below).
        // Whether `b` is at its paragraph's end, before that paragraph is trimmed.
        let b_at_end = self.para(b.story, &b.path).is_some_and(|p| p.len() == b.off);
        let paths = self.paths_between(&a, &b);
        for p in paths.iter().rev() {
            let (from, to) = {
                let Some(para) = self.para(a.story, p) else { continue };
                let from = if *p == a.path { a.off } else { 0 };
                let to = if *p == b.path { b.off } else { para.len() };
                (from, to)
            };
            self.para_mut(a.story, p)?.delete_except(from, to, &kept_in(p))?;
        }
        // Tables the range covers whole, in the innermost container holding both ends, go (#1198,
        // #1844): those strictly between the ends, the one `a` is in when it starts at that
        // table's very start, and the one `b` is in when it ends at that table's very end. A range
        // that is one table and nothing else only empties it (Word's Delete keeps the table).
        let common = a.path.0.iter().zip(&b.path.0).take_while(|(x, y)| x == y).count();
        let level = common - common % 3;
        let (Some(&ia), Some(&ib)) = (a.path.0.get(level), b.path.0.get(level)) else { return Ok(a) };
        let (ia, ib) = (ia as usize, ib as usize);
        if ia >= ib {
            return Ok(a);
        }
        let prefix = a.path.0.get(..level).unwrap_or_default().to_vec();
        let block_path = |i: usize| {
            let mut v = prefix.clone();
            v.push(i as u32);
            Path(v)
        };
        // Paragraphs inside the block at index `i` of the container, in document order.
        let paths = self.para_paths(a.story);
        let inside = |i: usize| -> Vec<&Path> {
            let pre = block_path(i);
            paths.iter().filter(|q| q.0.len() > pre.0.len() && q.0.starts_with(&pre.0)).collect()
        };
        // A table with cells deeper than the paragraph walk goes has paragraphs `inside` never
        // lists, so where it starts or ends isn't known: it is never taken as covered.
        let walked = |i: usize| self.container(a.story, &block_path(i)).and_then(|bl| bl.get(i)).is_some_and(|b| !deeper_than_walk(b, level / 3));
        let a_covers = a.path.0.len() > level + 1 && a.off == 0 && walked(ia) && inside(ia).first() == Some(&&a.path);
        let b_covers = b.path.0.len() > level + 1 && walked(ib) && inside(ib).last() == Some(&&b.path) && b_at_end;
        let first = if a_covers { ia } else { ia + 1 };
        let last = if b_covers { ib } else { ib - 1 };
        let mut removed = 0;
        {
            let bl = self.container_mut(a.story, &block_path(ia))?;
            for i in (first..=last.min(bl.len().saturating_sub(1))).rev() {
                if matches!(bl.get(i).map(|b| &**b), Some(Block::Table(_))) {
                    bl.remove(i);
                    removed += 1;
                }
            }
            if removed == 0 {
                return Ok(a);
            }
            // A story or cell always ends with a paragraph.
            if bl.is_empty() || matches!(bl.last().map(|b| &**b), Some(Block::Table(_))) {
                bl.push(para_block(Paragraph::new()));
            }
        }
        // The paragraphs left between the ends join, as in a range within one container.
        let at = if a_covers { Pos { story: a.story, path: block_path(ia), off: 0 } } else { a.clone() };
        let end = if b_covers {
            Pos { story: a.story, path: block_path((ib + 1).saturating_sub(removed)), off: 0 }
        } else {
            Pos { story: a.story, path: block_path(ib.saturating_sub(removed)), off: 0 }
        };
        let paras = |p: &Pos| p.path.0.len() == level + 1 && self.para(p.story, &p.path).is_some();
        if paras(&at) && paras(&end) && at.path <= end.path {
            return self.delete_range(&at, &end);
        }
        if self.para(at.story, &at.path).is_some() {
            return Ok(at);
        }
        Ok(self.clamp(&at))
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
            // A fully selected paragraph also gets the mark formatted (Word does), but a hyperlink
            // ends before the mark: a linked mark would carry the link into every paragraph Enter
            // makes from it (issue #413).
            if to == para.len() && (from == 0 || p != a.path) {
                let style = para.mark.style.clone();
                f(&mut para.mark);
                if para.mark.link.take().is_some() {
                    para.mark.style = style.filter(|s| s != "Hyperlink");
                }
            }
        }
        Ok(())
    }

    /// The paragraphs paragraph formatting of `a..b` applies to: [`Self::paths_between`] without
    /// the last paragraph when a non-empty range ends at its start (Select Paragraph ends there
    /// to include the paragraph mark). A caret applies to its own paragraph (Word, #418).
    pub fn para_paths_in(&self, a: &Pos, b: &Pos) -> Vec<Path> {
        let (a, b) = order(a, b);
        let mut paths = self.paths_between(&a, &b);
        if b.off == 0 && a.path != b.path && paths.len() > 1 && paths.last() == Some(&b.path) {
            paths.pop();
        }
        paths
    }

    /// Apply `f` to the paragraph properties of every paragraph `a..b` covers ([`Self::para_paths_in`]).
    pub fn format_paragraphs(&mut self, a: &Pos, b: &Pos, f: &dyn Fn(&mut ParaProps)) -> Result<()> {
        for p in self.para_paths_in(a, b) {
            let para = self.para_mut(a.story, &p)?;
            f(&mut para.props);
            para.touch();
        }
        Ok(())
    }

    /// Copy `a..b` into a fragment (with copies of the text boxes in it).
    pub fn copy_range(&self, a: &Pos, b: &Pos) -> Fragment {
        let mut blocks = self.copy_blocks(a, b);
        // Half a content control (one marker without the other) isn't copied.
        crate::control::balance_blocks(&mut blocks);
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
    fn adopt_text_boxes(&mut self, blocks: &mut [Block], parts: &BTreeMap<u32, Part>, depth: usize, budget: &mut BoxBudget) {
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
                    for story in o.text_box_slots() {
                        if let Some(old) = *story {
                            *story = self.adopt_text_box(old, parts, depth, budget);
                            changed = true;
                        }
                    }
                }
                if changed {
                    p.touch();
                }
            });
        }
    }

    /// A new text box story copied from `old`; its own boxes get new stories too. Blocks without
    /// text boxes are shared with the source (copy-on-write), the others copied. A box showing a
    /// story it is inside, or whose story the paste's budget can't hold, gets none (CWE-400);
    /// only that box: a later box with a smaller story is still copied while the budget lasts.
    fn adopt_text_box(&mut self, old: u32, parts: &BTreeMap<u32, Part>, depth: usize, budget: &mut BoxBudget) -> Option<u32> {
        if depth >= MAX_BOX_DEPTH || budget.active.contains(&old) {
            return None;
        }
        if budget.stories == 0 {
            budget.skipped = budget.skipped.saturating_add(1);
            return None;
        }
        let src = parts.get(&old).or_else(|| self.parts.get(&old).filter(|p| p.kind == PartKind::TextBox))?;
        // Charge the whole story before copying any of it. Its cost is worked out once per paste
        // (that walks its payloads), however many boxes show it.
        let cost = *budget.costs.entry(old).or_insert_with(|| src.blocks.iter().fold(0usize, |a, b| a.saturating_add(block_cost(b).0)));
        let Some(left) = budget.bytes.checked_sub(cost) else {
            budget.skipped = budget.skipped.saturating_add(1);
            return None;
        };
        budget.bytes = left;
        budget.stories -= 1;
        let src_blocks = src.blocks.clone();
        // The copy keeps its text direction and alignment, not its link (a copy stands alone).
        let body = crate::TextBody { next: None, ..src.body };
        budget.active.push(old);
        let mut blocks: Blocks = Vec::with_capacity(src_blocks.len());
        for b in src_blocks {
            let mut boxes = false;
            each_para(&b, 0, &mut |p| boxes |= box_ids(p).next().is_some());
            if boxes {
                let mut copy = (*b).clone();
                self.adopt_text_boxes(std::slice::from_mut(&mut copy), parts, depth + 1, budget);
                blocks.push(Arc::new(copy));
            } else {
                blocks.push(b);
            }
        }
        budget.active.pop();
        let id = self.try_add_part(PartKind::TextBox, blocks).ok()?;
        if let Some(p) = self.parts.get_mut(&id) {
            p.body = body;
        }
        Some(id)
    }

    /// Insert a fragment at `pos`; returns the position after the pasted content.
    pub fn insert_fragment(&mut self, pos: &Pos, frag: &Fragment) -> Result<Pos> {
        // Tables that would nest deeper than the limit here become their paragraphs (#1596).
        let room = MAX_TABLE_DEPTH.saturating_sub(pos.path.depth());
        let mut blocks = Vec::with_capacity(frag.blocks.len());
        for b in frag.blocks.iter().cloned() {
            fit_tables(b, room, &mut blocks);
        }
        if blocks.is_empty() {
            return Ok(pos.clone());
        }
        if self.para_at(pos).is_none() {
            return Err(DocError::BadPath(pos.path.to_string()));
        }
        // Pasted text boxes get stories of their own (never shared with the copy's source).
        let mut budget = BoxBudget::new();
        self.adopt_text_boxes(&mut blocks, &frag.parts, 0, &mut budget);
        if budget.skipped > 0 {
            // Once per paste, not once per box (review of #3874).
            log::warn!(
                "paste: {} text boxes are left empty; their stories are past what one paste copies ({MAX_PASTED_BOXES} stories, {MAX_PASTED_BOX_BYTES} bytes)",
                budget.skipped
            );
        }
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

    /// Paths between two positions are those of all the story's paragraphs between them.
    #[test]
    fn paths_between_matches_every_path_filtered() {
        let mut d = doc(&["a", "b", "c", "d", "e"]);
        let mut inner = Table::new(2, 2, 100.0);
        inner.rows[1].cells[0].blocks.push(crate::para_block(crate::Paragraph::new()));
        let mut t = Table::new(3, 2, 200.0);
        t.rows[1].cells[1].blocks.insert(0, Arc::new(Block::Table(inner)));
        d.insert_block(StoryRef::Body, &Path::top(2), Block::Table(t)).unwrap();
        d.insert_block(StoryRef::Body, &Path::top(4), Block::Table(Table::new(1, 3, 200.0))).unwrap();
        let all = d.para_paths(StoryRef::Body);
        for a in &all {
            for b in &all {
                let (pa, pb) = (Pos { story: StoryRef::Body, path: a.clone(), off: 0 }, Pos { story: StoryRef::Body, path: b.clone(), off: 0 });
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                let want: Vec<Path> = all.iter().filter(|p| *p >= lo && *p <= hi).cloned().collect();
                assert_eq!(d.paths_between(&pa, &pb), want, "{a:?} {b:?}");
            }
        }
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
    fn delete_range_with_bad_end_leaves_document_unchanged() {
        let mut d = doc(&["one", "two", "three"]);
        let before = d.plain_text(StoryRef::Body);
        // Missing paragraph as the second end.
        assert!(d.delete_range(&Pos::body(0, 1), &Pos::body(9, 0)).is_err());
        assert_eq!(d.plain_text(StoryRef::Body), before);
        // Offset past the end of the second paragraph.
        assert!(d.delete_range(&Pos::body(0, 1), &Pos::body(1, 99)).is_err());
        assert_eq!(d.plain_text(StoryRef::Body), before);
        // Offset inside a multi-byte character.
        let mut d = doc(&["abc", "\u{e9}x"]);
        assert!(d.delete_range(&Pos::body(0, 1), &Pos::body(1, 1)).is_err());
        assert_eq!(d.plain_text(StoryRef::Body), "abc\n\u{e9}x");
        // Range entering a table with a bad offset at its start: the cell text stays.
        let mut d = doc(&["before", "after"]);
        d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(Table::new(2, 1, 200.0))).unwrap();
        d.insert_text(&Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 }, "x", &CharProps::default()).unwrap();
        let before = d.plain_text(StoryRef::Body);
        let cell = Pos { story: StoryRef::Body, path: Path(vec![1, 1, 0, 0]), off: 0 };
        assert!(d.delete_range(&Pos::body(0, 99), &cell).is_err());
        assert_eq!(d.plain_text(StoryRef::Body), before);
    }

    #[test]
    fn delete_multi() {
        let mut d = doc(&["one", "two", "three"]);
        d.delete_range(&Pos::body(2, 2), &Pos::body(0, 1)).unwrap();
        assert_eq!(d.plain_text(StoryRef::Body), "oree");
        assert_eq!(d.body.len(), 1);
    }

    /// #2127: a range over two section breaks joins three sections into the last; the headers
    /// and footers it showed by linking to earlier ones stay, each from the nearest section that
    /// named it, as they resolved before the delete.
    #[test]
    fn delete_over_two_section_breaks_keeps_the_linked_headers() {
        let mut d = doc(&["A", "B", "C"]);
        let set = |default| crate::section::HeaderSet { default: Some(default), ..Default::default() };
        let s1 = crate::section::SectionProps { headers: set(1), footers: set(5), ..Default::default() };
        let s2 = crate::section::SectionProps { headers: set(2), ..Default::default() };
        let s3 = crate::section::SectionProps { margin_left: 99.0, ..Default::default() };
        for (i, s) in [(0, s1), (1, s2), (2, s3.clone())] {
            if let Some(Block::Para(p)) = d.body.get_mut(i).map(Arc::make_mut) {
                p.section = Some(Box::new(s));
            }
        }
        assert_eq!(d.sections().len(), 4);
        d.delete_range(&Pos::body(0, 1), &Pos::body(2, 0)).unwrap();
        assert_eq!(d.plain_text(StoryRef::Body), "AC");
        let sections = d.sections();
        assert_eq!(sections.len(), 2);
        assert_eq!(*sections[0].1, crate::section::SectionProps { headers: set(2), footers: set(5), ..s3 });
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

    /// The first paragraph of the deepest cell, following the first table at each level.
    fn deepest(d: &Document) -> Path {
        let (mut path, mut blocks) = (Vec::new(), &d.body);
        while let Some((i, t)) = blocks.iter().enumerate().find_map(|(i, b)| if let Block::Table(t) = &**b { Some((i, t)) } else { None }) {
            path.extend([i as u32, 0, 0]);
            blocks = &t.rows[0].cells[0].blocks;
        }
        path.push(0);
        Path(path)
    }

    fn body_height(d: &Document) -> usize {
        d.body.iter().filter_map(|b| if let Block::Table(t) = &**b { Some(table_height(t)) } else { None }).max().unwrap_or(0)
    }

    /// How many times `c` is in `blocks`, the deepest cells too.
    fn count(blocks: &[Arc<Block>], c: char) -> usize {
        blocks
            .iter()
            .map(|b| match &**b {
                Block::Para(p) => p.text.matches(c).count(),
                Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).map(|cell| count(&cell.blocks, c)).sum(),
            })
            .sum()
    }

    #[test]
    fn tables_nest_at_most_max_table_depth() {
        let mut d = doc(&["x", ""]);
        for _ in 0..MAX_TABLE_DEPTH {
            let at = deepest(&d);
            d.insert_block(StoryRef::Body, &at, Block::Table(Table::new(1, 1, 100.0))).unwrap();
        }
        assert_eq!(body_height(&d), MAX_TABLE_DEPTH);
        // One more in the deepest cell is refused and changes nothing; a paragraph is fine.
        let (at, before) = (deepest(&d), d.clone());
        assert_eq!(at.depth(), MAX_TABLE_DEPTH);
        assert!(d.insert_block(StoryRef::Body, &at, Block::Table(Table::new(1, 1, 100.0))).is_err());
        assert_eq!(d, before);
        d.insert_block(StoryRef::Body, &at, Block::Para(Paragraph::new())).unwrap();
        // One level up, a table fits but a table holding a table doesn't.
        let up = Path(at.0[..at.0.len() - 3].to_vec());
        let mut two = Table::new(1, 1, 100.0);
        two.rows[0].cells[0].blocks.insert(0, Arc::new(Block::Table(Table::new(1, 1, 100.0))));
        assert!(d.insert_block(StoryRef::Body, &up, Block::Table(two)).is_err());
        d.insert_block(StoryRef::Body, &up, Block::Table(Table::new(1, 1, 100.0))).unwrap();
        assert_eq!(body_height(&d), MAX_TABLE_DEPTH);
    }

    #[test]
    fn pasting_a_table_into_its_own_cell_stops_at_max_table_depth() {
        // #1596: copy everything and paste it into the deepest cell, again and again: each round
        // doubled the nesting, until saving, copying or closing the document overflowed the stack.
        let mut d = doc(&["x", ""]);
        d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(Table::new(1, 1, 100.0))).unwrap();
        d.insert_text(&Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 }, "a", &CharProps::default()).unwrap();
        for _ in 0..8 {
            let all = d.copy_range(&Pos::body(0, 0), &Pos::body(d.body.len() - 1, 0));
            let at = deepest(&d);
            let off = d.para(StoryRef::Body, &at).map_or(0, Paragraph::len);
            d.insert_fragment(&Pos { story: StoryRef::Body, path: at, off }, &all).unwrap();
        }
        assert_eq!(body_height(&d), MAX_TABLE_DEPTH);
        // Nothing is lost: every copy's text is there, below the limit as paragraphs.
        assert_eq!((count(&d.body, 'x'), count(&d.body, 'a')), (256, 256));
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
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
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
        // The copy's inner box would show the story being copied: a cycle, so it gets none.
        assert_eq!(added, 1);
    }

    /// A fragment whose body is one paragraph of `refs` text boxes all showing story 1.
    fn fan_out(story: Vec<Arc<Block>>, refs: usize) -> Fragment {
        let mut p = Paragraph::new();
        for _ in 0..refs {
            p.insert_object(0, box_shape(Some(1)), &CharProps::default()).unwrap();
        }
        let mut parts = BTreeMap::new();
        parts.insert(1, crate::Part { kind: PartKind::TextBox, blocks: story, body: Default::default() });
        Fragment { blocks: vec![Block::Para(p)], parts }
    }

    #[test]
    fn pasted_boxes_share_the_blocks_they_need_not_copy() {
        let big = para_block(Paragraph::with_text(&"x".repeat(100_000), CharProps::default()));
        let f = fan_out(vec![big.clone()], 3);
        let mut d = doc(&["x"]);
        d.insert_fragment(&Pos::body(0, 1), &f).unwrap();
        let got = boxes_in(&d, &Path::top(0));
        assert_eq!(got.len(), 3);
        let ids: Vec<u32> = got.into_iter().flatten().collect();
        assert_eq!(ids.len(), 3, "each box has a story");
        assert!(ids.windows(2).all(|w| w[0] != w[1]), "stories of their own: {ids:?}");
        for id in ids {
            let b = d.parts[&id].blocks.first().unwrap();
            assert!(Arc::ptr_eq(b, &big), "no text box inside: shared, not copied");
        }
    }

    #[test]
    fn a_box_over_the_budget_leaves_later_smaller_boxes_their_stories() {
        // Review of #3874: once one story didn't fit, every later box in the paste was left
        // empty, however small its story. Only the box that doesn't fit is.
        let big = para_block(Paragraph::with_text(&"x".repeat(100_000), CharProps::default()));
        let small = para_block(Paragraph::with_text("small", CharProps::default()));
        let mut p = Paragraph::new();
        for story in [2, 1, 2, 1] {
            p.insert_object(0, box_shape(Some(story)), &CharProps::default()).unwrap();
        }
        let mut parts = BTreeMap::new();
        parts.insert(1, crate::Part { kind: PartKind::TextBox, blocks: vec![big], body: Default::default() });
        parts.insert(2, crate::Part { kind: PartKind::TextBox, blocks: vec![small], body: Default::default() });
        let mut blocks = vec![Block::Para(p)];
        let mut d = doc(&["x"]);
        let mut budget = BoxBudget { bytes: 50_000, ..BoxBudget::new() };
        d.adopt_text_boxes(&mut blocks, &parts, 0, &mut budget);
        let Some(Block::Para(p)) = blocks.first() else { panic!() };
        let got: Vec<Option<u32>> = p.objects.iter().map(|o| if let InlineObject::Shape { story, .. } = o { *story } else { None }).collect();
        assert_eq!(got.iter().map(Option::is_some).collect::<Vec<_>>(), [false, true, false, true], "{got:?}");
        assert_eq!(budget.skipped, 2);
        assert_eq!(budget.costs.len(), 2, "each story costed once");
    }

    #[test]
    fn pasted_box_copies_are_capped_in_bytes() {
        // Story 1 is a 1 MiB paragraph that holds a text box itself, so each box showing it needs
        // its own copy; 64 of them used to copy 64 MiB.
        let mut p = Paragraph::with_text(&"x".repeat(1 << 20), CharProps::default());
        p.insert_object(0, box_shape(Some(999)), &CharProps::default()).unwrap();
        let f = fan_out(vec![para_block(p)], 64);
        let mut d = doc(&["x"]);
        let before = d.parts.len();
        let (_, warned) = crate::test_log::warnings(|| d.insert_fragment(&Pos::body(0, 1), &f).unwrap());
        let added = d.parts.len() - before;
        // The boxes left empty are said to be, once for the paste (review of #3874).
        let empty: Vec<_> = warned.iter().filter(|w| w.contains("text boxes are left empty")).collect();
        assert_eq!(empty.len(), 1, "{warned:?}");
        assert!(empty[0].starts_with(&format!("paste: {} text boxes", 64 - added)), "{empty:?}");
        let copied: usize = d.parts.values().flat_map(|p| p.blocks.iter()).map(|b| if let Block::Para(p) = &**b { p.text.len() } else { 0 }).sum();
        assert!((1..64).contains(&added), "{added}");
        assert!(copied <= MAX_PASTED_BOX_BYTES, "{copied}");
        // The boxes past the cap keep no story rather than a shared one.
        let got = boxes_in(&d, &Path::top(0));
        assert_eq!(got.iter().flatten().count(), added);
    }

    #[test]
    fn pasted_box_copies_count_the_objects_they_copy() {
        // #3614 review: a story holding a text box and a 1 MiB kept-for-round-trip object costs
        // the object's XML too, not only its slot: 64 copies used to copy 64 MiB of it.
        let mut p = Paragraph::with_text("x", CharProps::default());
        p.insert_object(0, box_shape(Some(999)), &CharProps::default()).unwrap();
        let opaque = InlineObject::Opaque { format: "docx".into(), xml: "x".repeat(1 << 20), text: String::new() };
        p.insert_object(0, opaque, &CharProps::default()).unwrap();
        let f = fan_out(vec![para_block(p)], 64);
        let mut d = doc(&["x"]);
        let before = d.parts.len();
        d.insert_fragment(&Pos::body(0, 1), &f).unwrap();
        let added = d.parts.len() - before;
        let xml = |o: &InlineObject| if let InlineObject::Opaque { xml, .. } = o { xml.len() } else { 0 };
        let copied: usize =
            d.parts.values().flat_map(|p| p.blocks.iter()).map(|b| if let Block::Para(p) = &**b { p.objects.iter().map(xml).sum() } else { 0 }).sum();
        assert!((1..64).contains(&added), "{added}");
        assert!(copied <= MAX_PASTED_BOX_BYTES, "{copied}");
        // Nested group members and equations count too.
        let member = |obj| crate::para::GroupChild { x: 0.0, y: 0.0, obj };
        let deep = InlineObject::Opaque { format: String::new(), xml: "y".repeat(5000), text: String::new() };
        let inner =
            InlineObject::Group { w: 1.0, h: 1.0, float: Default::default(), ch_w: 1.0, ch_h: 1.0, children: vec![member(deep)], canvas: None };
        let outer =
            InlineObject::Group { w: 1.0, h: 1.0, float: Default::default(), ch_w: 1.0, ch_h: 1.0, children: vec![member(inner)], canvas: None };
        assert!(object_cost(&outer) >= 5000);
        let eq = InlineObject::Equation {
            linear: String::new(),
            display: false,
            math: crate::math::Math { nodes: crate::math::parse_linear(&"z".repeat(5000)), ..Default::default() },
        };
        assert!(object_cost(&eq) >= 5000);
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

    fn para(text: &str) -> Block {
        Block::Para(Paragraph::with_text(text, CharProps::default()))
    }

    #[test]
    fn appended_pieces_are_each_followed_by_one_paragraph_mark() {
        let mut spike = Fragment::default();
        spike.append_piece(Fragment { blocks: vec![para("a")], ..Default::default() });
        spike.append_piece(Fragment { blocks: vec![para("b")], ..Default::default() });
        assert_eq!(spike.plain_text(), "a\nb\n");
        // A piece that already ends with its paragraph mark gets no second one.
        let mut spike = Fragment::default();
        spike.append_piece(Fragment { blocks: vec![para("a"), para("")], ..Default::default() });
        assert_eq!(spike.plain_text(), "a\n");
        spike.append_piece(Fragment { blocks: vec![para("b"), para("c")], ..Default::default() });
        assert_eq!(spike.plain_text(), "a\nb\nc\n");
    }

    #[test]
    fn appended_pieces_keep_their_own_text_boxes() {
        // Two documents give their first text box the same id.
        let mut one = doc(&["x"]);
        let id1 = text_box(&mut one, &Pos::body(0, 1), "One");
        let mut two = doc(&["y"]);
        let id2 = text_box(&mut two, &Pos::body(0, 1), "Two");
        assert_eq!(id1, id2);
        let mut spike = Fragment::default();
        spike.append_piece(one.copy_range(&Pos::body(0, 0), &Pos::body(0, 1 + 3)));
        spike.append_piece(two.copy_range(&Pos::body(0, 0), &Pos::body(0, 1 + 3)));
        let mut dst = doc(&[""]);
        dst.insert_fragment(&Pos::body(0, 0), &spike).unwrap();
        let texts: Vec<String> =
            [0, 1].iter().flat_map(|i| boxes_in(&dst, &Path::top(*i))).map(|b| dst.plain_text(StoryRef::Part(b.unwrap()))).collect();
        assert_eq!(texts, ["One", "Two"]);
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

    // delete_range over whole tables (#1844), in the body, a header and a footnote.

    /// A document whose story (the body, or a header or footnote part) holds `lines`.
    fn story_doc(kind: Option<PartKind>, lines: &[&str]) -> (Document, StoryRef) {
        match kind {
            None => (doc(lines), StoryRef::Body),
            Some(k) => {
                let mut d = doc(&["body"]);
                let blocks = lines.iter().map(|l| para_block(Paragraph::with_text(l, CharProps::default()))).collect();
                let id = d.add_part(k, blocks);
                (d, StoryRef::Part(id))
            }
        }
    }

    const STORIES: [Option<PartKind>; 3] = [None, Some(PartKind::Header), Some(PartKind::Footnote)];

    fn at(story: StoryRef, path: &[u32], off: usize) -> Pos {
        Pos { story, path: Path(path.to_vec()), off }
    }

    /// Insert an `rows`×`cols` table at `path` whose cells hold `texts`, row by row.
    fn put_table(d: &mut Document, story: StoryRef, path: &[u32], rows: usize, cols: usize, texts: &[&str]) {
        d.insert_block(story, &Path(path.to_vec()), Block::Table(Table::new(rows, cols, 200.0))).unwrap();
        for (k, t) in texts.iter().enumerate() {
            let mut p = path.to_vec();
            p.extend([(k / cols) as u32, (k % cols) as u32, 0]);
            d.insert_text(&at(story, &p, 0), t, &CharProps::default()).unwrap();
        }
    }

    /// The story's top-level blocks: a paragraph's text, or `[table]`.
    fn shape(d: &Document, story: StoryRef) -> Vec<String> {
        d.story(story).unwrap().iter().map(|b| b.as_para().map_or("[table]".to_string(), |p| p.text.clone())).collect()
    }

    fn cell_text(d: &Document, story: StoryRef, table: &[u32], r: u32, c: u32) -> String {
        let mut p = table.to_vec();
        p.extend([r, c, 0]);
        d.para(story, &Path(p)).map(|p| p.text.clone()).unwrap_or_else(|| "<none>".into())
    }

    #[test]
    fn a_range_ending_at_the_end_of_a_cell_that_isnt_the_last_keeps_the_table() {
        for kind in STORIES {
            for (r, c) in [(0, 1), (1, 0)] {
                let (mut d, st) = story_doc(kind, &["before", "after"]);
                put_table(&mut d, st, &[1], 2, 2, &["a", "b", "c", "d"]);
                d.delete_range(&at(st, &[0], 0), &at(st, &[1, r, c, 0], 1)).unwrap();
                assert_eq!(shape(&d, st), ["", "[table]", "after"], "{kind:?} ({r},{c})");
                assert_eq!(cell_text(&d, st, &[1], 1, 1), "d", "{kind:?} ({r},{c})");
            }
        }
    }

    #[test]
    fn a_range_from_the_start_of_one_table_into_the_middle_of_the_next_removes_only_the_first() {
        for kind in STORIES {
            let (mut d, st) = story_doc(kind, &["x", "mid", "y"]);
            put_table(&mut d, st, &[2], 2, 2, &["e", "f", "g", "h"]);
            put_table(&mut d, st, &[1], 2, 2, &["a", "b", "c", "d"]);
            assert_eq!(shape(&d, st), ["x", "[table]", "mid", "[table]", "y"]);
            let to = d.delete_range(&at(st, &[1, 0, 0, 0], 0), &at(st, &[3, 0, 1, 0], 1)).unwrap();
            assert_eq!(shape(&d, st), ["x", "", "[table]", "y"], "{kind:?}");
            assert_eq!([cell_text(&d, st, &[2], 0, 0), cell_text(&d, st, &[2], 0, 1), cell_text(&d, st, &[2], 1, 0)], ["", "", "g"], "{kind:?}");
            assert!(d.para(st, &to.path).is_some(), "{kind:?}: {to:?}");
        }
    }

    #[test]
    fn a_range_from_a_cell_paragraph_to_the_end_of_a_table_nested_in_that_cell_removes_only_the_nested_table() {
        for kind in STORIES {
            let (mut d, st) = story_doc(kind, &["before", "after"]);
            put_table(&mut d, st, &[1], 1, 1, &["p"]);
            put_table(&mut d, st, &[1, 0, 0, 1], 1, 1, &["n"]);
            d.insert_block(st, &Path(vec![1, 0, 0, 2]), Block::Para(Paragraph::with_text("q", CharProps::default()))).unwrap();
            let to = d.delete_range(&at(st, &[1, 0, 0, 0], 1), &at(st, &[1, 0, 0, 1, 0, 0, 0], 1)).unwrap();
            assert_eq!(shape(&d, st), ["before", "[table]", "after"], "{kind:?}");
            let cell = d.container(st, &Path(vec![1, 0, 0, 0])).unwrap();
            let cell: Vec<String> = cell.iter().map(|b| b.as_para().map_or("[table]".to_string(), |p| p.text.clone())).collect();
            assert_eq!(cell, ["pq"], "{kind:?}");
            assert_eq!(to, at(st, &[1, 0, 0, 0], 1), "{kind:?}");
        }
    }

    #[test]
    fn two_whole_tables_at_the_end_of_a_story_leave_one_empty_paragraph() {
        for kind in STORIES {
            for trailing in [true, false] {
                let (mut d, st) = story_doc(kind, &[""]);
                put_table(&mut d, st, &[0], 1, 2, &["a", "b"]);
                put_table(&mut d, st, &[1], 2, 1, &["c", "d"]);
                if !trailing {
                    // A story that ends in a table (a hostile file).
                    d.story_mut(st).unwrap().pop();
                }
                let to = d.delete_range(&at(st, &[0, 0, 0, 0], 0), &at(st, &[1, 1, 0, 0], 1)).unwrap();
                assert_eq!(shape(&d, st), [""], "{kind:?} trailing {trailing}");
                assert_eq!(to, at(st, &[0], 0), "{kind:?} trailing {trailing}");
            }
        }
    }

    /// Content controls pair their markers within one block list (a story, or a cell), so a
    /// marker in a table never has its partner outside it: a control inside a table removed whole
    /// goes with it, and one the range only enters keeps both markers (the control is emptied).
    #[test]
    fn removing_a_whole_table_keeps_a_control_the_range_only_partly_covers() {
        use crate::control::ContentControl;
        let start = || InlineObject::ControlStart { control: Box::new(ContentControl::default()) };
        for kind in STORIES {
            let (mut d, st) = story_doc(kind, &["before", "after"]);
            put_table(&mut d, st, &[1], 1, 1, &["c"]);
            // One control around "efo" in "before", one around the cell's text.
            d.insert_object(&at(st, &[0], 4), InlineObject::ControlEnd, &CharProps::default()).unwrap();
            let in_before = d.insert_object(&at(st, &[0], 1), start(), &CharProps::default()).unwrap();
            d.insert_object(&at(st, &[1, 0, 0, 0], 1), InlineObject::ControlEnd, &CharProps::default()).unwrap();
            d.insert_object(&at(st, &[1, 0, 0, 0], 0), start(), &CharProps::default()).unwrap();
            assert_eq!(d.control_ranges(st).len(), 2, "setup");
            let end = d.para(st, &Path(vec![1, 0, 0, 0])).unwrap().len();
            // From inside the first control to the end of the table.
            d.delete_range(&Pos { off: in_before.off + 1, ..in_before }, &at(st, &[1, 0, 0, 0], end)).unwrap();
            assert_eq!(shape(&d, st), ["b\u{FFFC}e\u{FFFC}after"], "{kind:?}: the table goes, the paragraphs join");
            let ranges = d.control_ranges(st);
            assert_eq!(ranges.len(), 1, "{kind:?}: {ranges:?}");
            assert!(ranges.iter().all(|r| r.start.path == Path::top(0) && r.end.path == Path::top(0)), "{kind:?}");
            let objects = d.para(st, &Path::top(0)).map_or(0, |p| p.objects.len());
            assert_eq!(objects, 2, "{kind:?}: no orphan markers left");
        }
    }

    #[test]
    fn a_table_nested_deeper_than_the_paragraph_walk_is_never_removed_whole() {
        // 20 tables, each in the first cell of the one around it, ahead of that cell's paragraph:
        // the innermost paragraphs come first in the document but the paragraph walk stops short.
        let mut t = Table::new(1, 1, 200.0);
        if let Some(cell) = t.rows.get_mut(0).and_then(|r| r.cells.get_mut(0)) {
            cell.blocks = vec![para_block(Paragraph::with_text("deep", CharProps::default()))];
        }
        for _ in 0..20 {
            let mut outer = Table::new(1, 1, 200.0);
            if let Some(cell) = outer.rows.get_mut(0).and_then(|r| r.cells.get_mut(0)) {
                cell.blocks.insert(0, Arc::new(Block::Table(t)));
            }
            t = outer;
        }
        let mut d = doc(&["before", "after"]);
        d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
        let first = d.para_paths(StoryRef::Body).into_iter().find(|p| p.0.first() == Some(&1)).unwrap();
        d.delete_range(&Pos { story: StoryRef::Body, path: first, off: 0 }, &Pos::body(2, 0)).unwrap();
        assert!(d.body.get(1).is_some_and(|b| b.as_table().is_some()), "a table with paragraphs the walk never saw was removed");
    }
}
