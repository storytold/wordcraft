use super::block::{Block, Blocks, BoxBudget};
use super::compare::{para_block, same_blocks, same_bytes, same_glossary, same_parts};
use super::error::{DocError, Result};
use super::parts::{MAX_LINKED_BOXES, NoteSeparators, Part, PartKind, PermRange, TextBody, free_id};
use super::position::{Path, Pos, StoryRef};
use super::settings::{CoreProps, CustomProp, Settings};
use super::text_walk::{TextMode, notes_and_boxes, story_word_count_of, text_of, walk};
use std::sync::Arc;

use super::{BTreeMap, CharProps, Document, InlineObject, Numbering, Paragraph, SectionProps, StyleSheet, Table, edit, numbering, para, section};

/// [`Document::media`] by content, so adding media is linear in the bytes added rather than a
/// scan of every earlier image (CWE-407). Rebuilt whenever `media` changed size behind its back;
/// a hit is always checked against `media`, so a stale entry can at worst miss a duplicate.
#[derive(Clone, Default)]
pub(crate) struct MediaIndex {
    by_bytes: std::collections::HashMap<Arc<Vec<u8>>, String>,
    /// `media.len()` when the index last matched it.
    len: usize,
    /// The number after the last generated `image{n}` key, so taken names are probed once.
    next: usize,
}

impl MediaIndex {
    fn rebuild(&mut self, media: &BTreeMap<String, Arc<Vec<u8>>>) {
        self.by_bytes.clear();
        // The first key (in key order) of equal bytes wins, as a scan would find it.
        for (k, v) in media.iter().rev() {
            self.by_bytes.insert(v.clone(), k.clone());
        }
        self.len = media.len();
        self.next = 0;
    }

    fn find(&self, media: &BTreeMap<String, Arc<Vec<u8>>>, bytes: &Vec<u8>) -> Option<String> {
        self.by_bytes.get(bytes).filter(|k| media.get(*k).is_some_and(|v| **v == *bytes)).cloned()
    }
}

impl PartialEq for MediaIndex {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for MediaIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MediaIndex")
    }
}

impl Default for Document {
    fn default() -> Self {
        Document::new()
    }
}

impl Document {
    /// A blank document: one empty Normal paragraph, Letter, built-in styles.
    pub fn new() -> Self {
        Document {
            body: vec![para_block(Paragraph::new())],
            last_section: SectionProps::default(),
            parts: BTreeMap::new(),
            styles: StyleSheet::builtin(),
            numbering: Numbering::default(),
            comments: BTreeMap::new(),
            revisions: Vec::new(),
            settings: Settings::default(),
            core: CoreProps::default(),
            sources: Vec::new(),
            custom_props: Vec::new(),
            footnote_separators: NoteSeparators::default(),
            endnote_separators: NoteSeparators::default(),
            media: BTreeMap::new(),
            passthrough: BTreeMap::new(),
            glossary: Vec::new(),
            media_index: MediaIndex::default(),
        }
    }

    /// A document from plain text, one paragraph per line.
    pub fn from_text(text: &str) -> Self {
        let mut d = Document::new();
        d.body = text.split('\n').map(|l| para_block(Paragraph::with_text(l.trim_end_matches('\r'), CharProps::default()))).collect();
        d.ensure_nonempty();
        d
    }

    fn fix_cell_blocks(bl: &mut Blocks) {
        if bl.is_empty() || matches!(bl.last().map(|b| &**b), Some(Block::Table(_))) {
            bl.push(para_block(Paragraph::new()));
        }
    }

    fn fix_story_blocks(bl: &mut Blocks) {
        if !bl.iter().any(|b| matches!(**b, Block::Para(_))) || matches!(bl.last().map(|b| &**b), Some(Block::Table(_))) {
            bl.push(para_block(Paragraph::new()));
        }
        for b in bl.iter_mut() {
            if let Block::Table(t) = &**b {
                let needs = t
                    .rows
                    .iter()
                    .any(|r| r.cells.iter().any(|c| c.blocks.is_empty() || matches!(c.blocks.last().map(|b| &**b), Some(Block::Table(_)))));
                if needs && let Block::Table(t) = Arc::make_mut(b) {
                    for r in &mut t.rows {
                        for c in &mut r.cells {
                            Self::fix_cell_blocks(&mut c.blocks);
                        }
                    }
                }
            }
        }
    }

    /// Every story must hold at least one paragraph, and every cell too.
    pub fn ensure_nonempty(&mut self) {
        Self::fix_story_blocks(&mut self.body);
        for p in self.parts.values_mut() {
            if p.blocks.is_empty() {
                p.blocks.push(para_block(Paragraph::new()));
            }
        }
    }

    /// Whether the document holds the same content as `other`: the same blocks, the same
    /// styles, lists, settings, comments and the rest.
    ///
    /// The blocks are compared by address first: the document is copy-on-write, so a block
    /// that changed was copied and has another address. Comparing them by value would walk
    /// the whole document, and this is asked after every command.
    ///
    /// Every field is listed, so a new one does not build until someone decides whether a
    /// change to it counts (the same rule the session's edit snapshot follows).
    pub fn same_content(&self, other: &Document) -> bool {
        let Document {
            body,
            last_section,
            parts,
            styles,
            numbering,
            comments,
            revisions,
            settings,
            core,
            sources,
            custom_props,
            footnote_separators,
            endnote_separators,
            media,
            passthrough,
            glossary,
            media_index: _,
        } = self;
        let Document {
            body: other_body,
            last_section: other_section,
            parts: other_parts,
            styles: other_styles,
            numbering: other_numbering,
            comments: other_comments,
            revisions: other_revisions,
            settings: other_settings,
            core: other_core,
            sources: other_sources,
            custom_props: other_props,
            footnote_separators: other_footnotes,
            endnote_separators: other_endnotes,
            media: other_media,
            passthrough: other_passthrough,
            glossary: other_glossary,
            media_index: _,
        } = other;
        same_blocks(body, other_body)
            && same_parts(parts, other_parts)
            && last_section == other_section
            && styles == other_styles
            && numbering == other_numbering
            && comments == other_comments
            && revisions == other_revisions
            && settings == other_settings
            && core == other_core
            && sources == other_sources
            && custom_props == other_props
            && footnote_separators == other_footnotes
            && endnote_separators == other_endnotes
            && same_bytes(media, other_media)
            && same_bytes(passthrough, other_passthrough)
            && same_glossary(glossary, other_glossary)
    }

    pub fn story(&self, s: StoryRef) -> Option<&Blocks> {
        match s {
            StoryRef::Body => Some(&self.body),
            StoryRef::Part(id) => self.parts.get(&id).map(|p| &p.blocks),
        }
    }
    pub fn story_mut(&mut self, s: StoryRef) -> Result<&mut Blocks> {
        match s {
            StoryRef::Body => Ok(&mut self.body),
            StoryRef::Part(id) => self.parts.get_mut(&id).map(|p| &mut p.blocks).ok_or(DocError::BadStory(s)),
        }
    }

    /// The block list that contains the block at `path`.
    pub fn container(&self, s: StoryRef, path: &Path) -> Option<&Blocks> {
        let mut bl = self.story(s)?;
        let parent = path.parent();
        if !parent.len().is_multiple_of(3) {
            return None;
        }
        for ch in parent.chunks(3) {
            let [b, r, c] = ch else { return None };
            let Block::Table(t) = &**bl.get(*b as usize)? else { return None };
            bl = &t.rows.get(*r as usize)?.cells.get(*c as usize)?.blocks;
        }
        Some(bl)
    }

    pub fn container_mut(&mut self, s: StoryRef, path: &Path) -> Result<&mut Blocks> {
        let bad = || DocError::BadPath(path.to_string());
        let parent: Vec<u32> = path.parent().to_vec();
        if !parent.len().is_multiple_of(3) {
            return Err(bad());
        }
        let mut bl = self.story_mut(s)?;
        for ch in parent.chunks(3) {
            let [b, r, c] = ch else { return Err(bad()) };
            let blk = bl.get_mut(*b as usize).ok_or_else(bad)?;
            let Block::Table(t) = Arc::make_mut(blk) else { return Err(bad()) };
            bl = &mut t.rows.get_mut(*r as usize).ok_or_else(bad)?.cells.get_mut(*c as usize).ok_or_else(bad)?.blocks;
        }
        Ok(bl)
    }

    pub fn block(&self, s: StoryRef, path: &Path) -> Option<&Block> {
        self.container(s, path)?.get(path.last()).map(|b| &**b)
    }

    pub fn para(&self, s: StoryRef, path: &Path) -> Option<&Paragraph> {
        self.block(s, path)?.as_para()
    }
    pub fn para_at(&self, p: &Pos) -> Option<&Paragraph> {
        self.para(p.story, &p.path)
    }

    pub fn para_mut(&mut self, s: StoryRef, path: &Path) -> Result<&mut Paragraph> {
        let i = path.last();
        let bl = self.container_mut(s, path)?;
        match bl.get_mut(i).map(Arc::make_mut) {
            Some(Block::Para(p)) => Ok(p),
            _ => Err(DocError::BadPath(path.to_string())),
        }
    }

    pub fn table(&self, s: StoryRef, path: &Path) -> Option<&Table> {
        self.block(s, path)?.as_table()
    }
    pub fn table_mut(&mut self, s: StoryRef, path: &Path) -> Result<&mut Table> {
        let i = path.last();
        let bl = self.container_mut(s, path)?;
        match bl.get_mut(i).map(Arc::make_mut) {
            Some(Block::Table(t)) => Ok(t),
            _ => Err(DocError::BadPath(path.to_string())),
        }
    }

    /// Paths of every paragraph in a story, in document order (tables descended, depth-limited).
    pub fn para_paths(&self, s: StoryRef) -> Vec<Path> {
        let mut out = Vec::new();
        if let Some(bl) = self.story(s) {
            walk(bl, &mut Vec::new(), &mut out, 0);
        }
        out
    }

    /// First and last positions of a story.
    pub fn start_of(&self, s: StoryRef) -> Pos {
        let path = self.para_paths(s).into_iter().next().unwrap_or_else(|| Path::top(0));
        Pos { story: s, path, off: 0 }
    }
    pub fn end_of(&self, s: StoryRef) -> Pos {
        let path = self.para_paths(s).pop().unwrap_or_else(|| Path::top(0));
        let off = self.para(s, &path).map(|p| p.len()).unwrap_or(0);
        Pos { story: s, path, off }
    }

    /// The position just after the shape that owns text box story `part` (body first, then the
    /// other parts), for leaving the text box.
    pub fn text_box_anchor(&self, part: u32) -> Option<Pos> {
        let stories = std::iter::once(StoryRef::Body).chain(self.parts.keys().filter(|k| **k != part).map(|k| StoryRef::Part(*k)));
        for story in stories {
            for path in self.para_paths(story) {
                let Some(p) = self.para(story, &path) else { continue };
                for (off, o) in p.objects_with_offsets() {
                    if o.text_boxes().contains(&part) {
                        return Some(Pos { story, path, off: off + para::OBJ.len_utf8() });
                    }
                }
            }
        }
        None
    }

    /// The next / previous paragraph path in document order.
    pub fn next_para(&self, s: StoryRef, path: &Path) -> Option<Path> {
        let all = self.para_paths(s);
        let i = all.binary_search(path).ok()?;
        all.get(i + 1).cloned()
    }
    pub fn prev_para(&self, s: StoryRef, path: &Path) -> Option<Path> {
        let all = self.para_paths(s);
        let i = all.binary_search(path).ok()?;
        i.checked_sub(1).and_then(|j| all.get(j).cloned())
    }

    /// Clamp a position into the document (valid path, char boundary).
    pub fn clamp(&self, p: &Pos) -> Pos {
        if let Some(para) = self.para_at(p) {
            return Pos { story: p.story, path: p.path.clone(), off: para.clamp(p.off) };
        }
        let story = if self.story(p.story).is_some() { p.story } else { StoryRef::Body };
        let all = self.para_paths(story);
        let path = all.iter().rev().find(|q| **q <= p.path).or(all.first()).cloned().unwrap_or_else(|| Path::top(0));
        let off = self.para(story, &path).map(|q| q.clamp(p.off)).unwrap_or(0);
        Pos { story, path, off }
    }

    /// The whole text of a story, paragraphs separated by `\n` (tables: cells by `\t`).
    pub fn plain_text(&self, s: StoryRef) -> String {
        self.story_text(s, TextMode::Plain)
    }

    /// [`Self::plain_text`] as it reads on screen: symbol-font characters (Wingdings…) as their
    /// Unicode equivalents ([`Paragraph::shown_text`]).
    pub fn shown_text(&self, s: StoryRef) -> String {
        self.story_text(s, TextMode::Shown)
    }

    /// [`Self::plain_text`] without tracked-deleted runs. A deleted paragraph mark still yields `\n`.
    pub fn final_text(&self, s: StoryRef) -> String {
        self.story_text(s, TextMode::Final)
    }

    fn story_text(&self, s: StoryRef, mode: TextMode) -> String {
        let mut out = String::new();
        if let Some(bl) = self.story(s) {
            text_of(bl, &mut out, 0, mode);
        }
        if out.ends_with('\n') {
            out.pop();
        }
        out
    }

    /// Section properties in effect for each top-level body block (index → section number), and
    /// the list of sections.
    pub fn sections(&self) -> Vec<(usize, &SectionProps)> {
        let mut v = Vec::new();
        for (i, b) in self.body.iter().enumerate() {
            if let Block::Para(p) = &**b
                && let Some(s) = &p.section
            {
                v.push((i, &**s));
            }
        }
        v.push((self.body.len().saturating_sub(1), &self.last_section));
        v
    }

    /// The footnote (`endnote` false) or endnote options in effect in section `sect`: its own,
    /// else the document's, else Word's defaults (footnotes at the page bottom in 1, 2, 3;
    /// endnotes at the end of the document in i, ii, iii; numbered on through the document).
    /// Placements that don't apply to the kind fall back to its default.
    pub fn note_options(&self, sect: &SectionProps, endnote: bool) -> section::NoteOptions {
        use section::NotePos;
        let (own, doc, fmt) = if endnote {
            (&sect.endnote_pr, &self.settings.endnote_pr, self.settings.endnote_format)
        } else {
            (&sect.footnote_pr, &self.settings.footnote_pr, self.settings.footnote_format)
        };
        let pos = own.pos.or(doc.pos);
        let pos = if endnote {
            pos.filter(|p| matches!(p, NotePos::SectEnd | NotePos::DocEnd)).unwrap_or(NotePos::DocEnd)
        } else {
            pos.filter(|p| matches!(p, NotePos::PageBottom | NotePos::BeneathText)).unwrap_or(NotePos::PageBottom)
        };
        section::NoteOptions {
            pos,
            num_fmt: own.num_fmt.unwrap_or(fmt),
            num_start: own.num_start.or(doc.num_start).unwrap_or(1).clamp(1, section::MAX_NOTE_START),
            num_restart: own.num_restart.or(doc.num_restart).unwrap_or_default(),
        }
    }

    /// The section that contains top-level body block `block` (index into `sections()`).
    pub fn section_index_of(&self, block: usize) -> usize {
        self.sections().iter().position(|(end, _)| block <= *end).unwrap_or(0)
    }

    /// Mutable section properties for the section containing body block `block`.
    pub fn section_mut(&mut self, block: usize) -> &mut SectionProps {
        let ends: Vec<usize> = self.sections().iter().map(|(e, _)| *e).collect();
        let idx = ends.iter().position(|e| block <= *e).unwrap_or(ends.len().saturating_sub(1));
        if idx + 1 < ends.len()
            && let Some(e) = ends.get(idx)
            && let Some(Block::Para(p)) = self.body.get_mut(*e).map(Arc::make_mut)
            && let Some(s) = p.section.as_mut()
        {
            return s;
        }
        &mut self.last_section
    }

    /// Remove the section break on the mark of top-level body paragraph `block`: its section
    /// joins the following one ([`SectionProps::absorb`]). False when there is no break there.
    pub fn remove_section_break(&mut self, block: usize) -> bool {
        if !matches!(self.body.get(block).map(|b| &**b), Some(Block::Para(p)) if p.section.is_some()) {
            return false;
        }
        let Some(Block::Para(p)) = self.body.get_mut(block).map(Arc::make_mut) else { return false };
        let Some(before) = p.section.take() else { return false };
        self.section_mut(block).absorb(&before);
        true
    }

    /// Allocate a new part id and insert the part. Part ids run out only past `u32::MAX` parts, more
    /// than memory holds; then nothing is added and the id is 0, which [`free_id`] never gives a part.
    pub fn add_part(&mut self, kind: PartKind, blocks: Blocks) -> u32 {
        self.try_add_part(kind, blocks).unwrap_or(0)
    }

    /// [`Document::add_part`], or an error, adding nothing, when every part id is taken.
    pub fn try_add_part(&mut self, kind: PartKind, blocks: Blocks) -> Result<u32> {
        let id = free_id(self.parts.keys().next_back().copied(), self.parts.keys().copied(), 1)
            .ok_or_else(|| DocError::Invalid("no free part id".into()))?;
        let mut part = Part { kind, blocks, body: TextBody::default() };
        if part.blocks.is_empty() {
            part.blocks.push(para_block(Paragraph::new()));
        }
        self.parts.insert(id, part);
        Ok(id)
    }

    /// Count words like Word's status bar (whitespace-separated tokens containing a letter or digit,
    /// including generated numbered-list labels).
    pub fn word_count(&self) -> usize {
        self.story_word_count(StoryRef::Body)
    }

    /// Word's default count: the body plus its text boxes, footnotes and endnotes.
    pub fn word_count_including_notes(&self) -> usize {
        self.counted_stories().iter().map(|s| self.story_word_count(*s)).sum()
    }

    /// Word count of one story, including generated numbered-list labels.
    pub fn story_word_count(&self, s: StoryRef) -> usize {
        let Some(bl) = self.story(s) else { return 0 };
        let mut counters = numbering::Counters::default();
        story_word_count_of(bl, &self.numbering, &self.styles, &mut counters, 0)
    }

    /// The body, then the text boxes, footnotes and endnotes it shows (nested ones too), in
    /// document order. Headers, footers, comments and stories nothing points at aren't included.
    pub fn counted_stories(&self) -> Vec<StoryRef> {
        self.reachable(vec![StoryRef::Body], &notes_and_boxes)
    }

    /// The stories a reader sees, in reading order: the body, then the headers and footers in
    /// use, each followed by the notes and text boxes it shows. Hidden stories (a removed header,
    /// a first-page or even-page header whose option is off, a linked text box's own text) and
    /// comment text aren't included.
    pub fn shown_stories(&self) -> Vec<StoryRef> {
        let followers: std::collections::BTreeSet<u32> = self.text_box_chains().into_iter().flat_map(|c| c.into_iter().skip(1)).collect();
        let follow = |o: &InlineObject| {
            let mut v = notes_and_boxes(o);
            v.retain(|(id, _)| !followers.contains(id));
            v
        };
        let mut out = self.reachable(vec![StoryRef::Body], &follow);
        let mut seen: std::collections::BTreeSet<StoryRef> = out.iter().copied().collect();
        let mut roots = Vec::new();
        let mut root_seen = std::collections::BTreeSet::new();
        let (mut headers, mut footers) = (section::HeaderSet::default(), section::HeaderSet::default());
        for (_, sec) in self.sections() {
            // Unset versions are inherited from the previous section.
            for (set, own) in [(&mut headers, &sec.headers), (&mut footers, &sec.footers)] {
                set.default = own.default.or(set.default);
                set.first = own.first.or(set.first);
                set.even = own.even.or(set.even);
                let first = set.first.filter(|_| sec.title_page);
                let even = set.even.filter(|_| self.settings.even_odd_headers);
                // By the first page each one shows on: page 1, then page 2 (even), then page 3.
                let order = if sec.title_page { [first, even, set.default] } else { [set.default, even, None] };
                for id in order.into_iter().flatten() {
                    if self.parts.get(&id).is_some_and(|p| matches!(p.kind, PartKind::Header | PartKind::Footer)) && root_seen.insert(id) {
                        roots.push(StoryRef::Part(id));
                    }
                }
            }
        }
        for st in self.reachable(roots, &follow) {
            if seen.insert(st) {
                out.push(st);
            }
        }
        out
    }

    /// Drop the text box stories no box shows any more (a deleted or cut box's text), nested
    /// ones too. Undo snapshots and the clipboard keep their own copies. Free for a document
    /// without text boxes; otherwise one pass over its paragraphs. Returns how many went.
    pub fn prune_text_boxes(&mut self) -> usize {
        if !self.parts.values().any(|p| p.kind == PartKind::TextBox) {
            return 0;
        }
        let roots = std::iter::once(StoryRef::Body)
            .chain(self.parts.iter().filter(|(_, p)| p.kind != PartKind::TextBox).map(|(id, _)| StoryRef::Part(*id)))
            .collect();
        let live = self.reachable(roots, &|o| o.text_boxes().into_iter().map(|id| (id, &[PartKind::TextBox][..])).collect());
        let before = self.parts.len();
        self.parts.retain(|id, p| p.kind != PartKind::TextBox || live.contains(&StoryRef::Part(*id)));
        let gone = before - self.parts.len();
        if gone > 0 {
            // A link to a box that went ends the chain there (its id may be reused).
            let ids: std::collections::BTreeSet<u32> = self.parts.keys().copied().collect();
            for p in self.parts.values_mut() {
                if p.body.next.is_some_and(|n| !ids.contains(&n)) {
                    p.body.next = None;
                }
            }
        }
        gone
    }

    /// The chains of linked text boxes (Create Link), each in order from the box holding the
    /// text: every chain has two or more boxes. A link to a part that isn't a text box, a box
    /// already linked from another, or links running in a circle are ignored (those boxes show
    /// their own text); a chain stops after [`MAX_LINKED_BOXES`].
    pub fn text_box_chains(&self) -> Vec<Vec<u32>> {
        let is_box = |id: u32| self.parts.get(&id).is_some_and(|p| p.kind == PartKind::TextBox);
        // Who links to whom: the first box (by id) to link to a box wins.
        let mut prev: BTreeMap<u32, u32> = BTreeMap::new();
        for (id, p) in &self.parts {
            if p.kind != PartKind::TextBox {
                continue;
            }
            if let Some(n) = p.body.next.filter(|n| *n != *id && is_box(*n)) {
                prev.entry(n).or_insert(*id);
            }
        }
        let next = |id: u32| self.parts.get(&id).and_then(|p| p.body.next).filter(|n| prev.get(n) == Some(&id));
        let mut out = Vec::new();
        for (id, p) in &self.parts {
            if p.kind != PartKind::TextBox || prev.contains_key(id) || next(*id).is_none() {
                continue;
            }
            let mut chain = vec![*id];
            let mut at = *id;
            while let Some(n) = next(at) {
                if chain.len() >= MAX_LINKED_BOXES || chain.contains(&n) {
                    break;
                }
                chain.push(n);
                at = n;
            }
            out.push(chain);
        }
        out
    }

    /// The chain (see [`Document::text_box_chains`]) text box `part` belongs to, if linked.
    pub fn text_box_chain(&self, part: u32) -> Option<Vec<u32>> {
        self.text_box_chains().into_iter().find(|c| c.contains(&part))
    }

    /// The size (width, height) of the shapes showing text boxes `ids`, in any story (a group
    /// member's as the group stretches it).
    pub fn text_box_sizes(&self, ids: &std::collections::BTreeSet<u32>) -> BTreeMap<u32, (f32, f32)> {
        let mut out = BTreeMap::new();
        let stories = std::iter::once(&self.body).chain(self.parts.values().map(|p| &p.blocks));
        for blocks in stories {
            for b in blocks {
                edit::each_para(b, 0, &mut |p| {
                    for o in &p.objects {
                        match o {
                            InlineObject::Shape { story: Some(id), w, h, .. } if ids.contains(id) => {
                                out.entry(*id).or_insert((*w, *h));
                            }
                            InlineObject::Group { w, h, .. } => {
                                for (r, c) in o.group_rects(0.0, 0.0, *w, *h) {
                                    if let Some(id) = c.text_box().filter(|id| ids.contains(id)) {
                                        out.entry(id).or_insert((r[2], r[3]));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
        }
        out
    }

    /// Every inline object in reading order: the body's, with each text box's objects where the
    /// box is (nested boxes too, within a [`BoxBudget`]).
    pub fn objects_in_reading_order(&self, f: &mut dyn FnMut(&InlineObject)) {
        self.walk_objects(&self.body, &mut BoxBudget::default(), &mut |p, k| {
            if let Some(o) = p.objects.get(k) {
                f(o);
            }
            true
        });
    }

    /// Like [`Document::objects_in_reading_order`], with each object's paragraph and index;
    /// `f` returns whether to go into the object's text box (false: the box is left out).
    pub fn objects_in_reading_order_at(&self, f: &mut dyn FnMut(&Paragraph, usize) -> bool) {
        self.walk_objects(&self.body, &mut BoxBudget::default(), f);
    }

    /// Like [`Document::objects_in_reading_order_at`], also giving the top-level body block each
    /// object is in (for objects in text boxes, the block of the box's anchor).
    pub fn objects_in_reading_order_by_block(&self, f: &mut dyn FnMut(usize, &Paragraph, usize) -> bool) {
        let mut budget = BoxBudget::default();
        for (i, b) in self.body.iter().enumerate() {
            self.walk_objects(std::slice::from_ref(b), &mut budget, &mut |p, k| f(i, p, k));
        }
    }

    /// The footnotes and endnotes referenced inside text box story `part` (nested boxes too), in
    /// reading order.
    pub fn notes_in_text_box(&self, part: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut budget = BoxBudget::default();
        if let Some(p) = self.parts.get(&part).filter(|p| p.kind == PartKind::TextBox)
            && budget.enter(part)
        {
            self.walk_objects(&p.blocks, &mut budget, &mut |p, k| {
                if let Some(InlineObject::NoteRef { id, .. }) = p.objects.get(k) {
                    out.push(*id);
                }
                true
            });
            budget.leave();
        }
        out
    }

    fn walk_objects(&self, blocks: &[Arc<Block>], budget: &mut BoxBudget, f: &mut dyn FnMut(&Paragraph, usize) -> bool) {
        for b in blocks {
            edit::each_para(b, 0, &mut |p| {
                for (k, o) in p.objects.iter().enumerate() {
                    if !f(p, k) {
                        continue;
                    }
                    for id in o.text_boxes() {
                        if let Some(part) = self.parts.get(&id).filter(|p| p.kind == PartKind::TextBox)
                            && budget.enter(id)
                        {
                            self.walk_objects(&part.blocks, budget, f);
                            budget.leave();
                        }
                    }
                }
            });
        }
    }

    /// `roots`, then every story their objects lead to (`follow`: an object's story id and the
    /// part kinds that count), transitively, in the order found.
    fn reachable(&self, roots: Vec<StoryRef>, follow: &dyn Fn(&InlineObject) -> Vec<(u32, &'static [PartKind])>) -> Vec<StoryRef> {
        let mut out = roots;
        let mut seen: std::collections::BTreeSet<StoryRef> = out.iter().copied().collect();
        let mut i = 0;
        while let Some(s) = out.get(i).copied() {
            i += 1;
            let mut found = Vec::new();
            for b in self.story(s).into_iter().flatten() {
                edit::each_para(b, 0, &mut |p| {
                    found.extend(p.objects.iter().flat_map(follow));
                });
            }
            for (id, kinds) in found {
                if self.parts.get(&id).is_some_and(|p| kinds.contains(&p.kind)) && seen.insert(StoryRef::Part(id)) {
                    out.push(StoryRef::Part(id));
                }
            }
        }
        out
    }

    /// A drawing id no shape uses yet (for gluing a connector to a shape that has none).
    pub fn next_shape_id(&self) -> u32 {
        let mut max = 0u32;
        let mut see = |o: &InlineObject| match o {
            InlineObject::Shape { extra, .. } => max = max.max(extra.id),
            InlineObject::Group { children, .. } => {
                for c in children {
                    if let InlineObject::Shape { extra, .. } = &c.obj {
                        max = max.max(extra.id);
                    }
                }
            }
            _ => {}
        };
        for blocks in std::iter::once(&self.body).chain(self.parts.values().map(|p| &p.blocks)) {
            for b in blocks {
                edit::each_para(b, 0, &mut |p| p.objects.iter().for_each(&mut see));
            }
        }
        max.saturating_add(1).max(1)
    }

    /// Change every inline object in every story (paragraphs inside tables too), copying only
    /// the blocks whose paragraphs `wants` says have objects to change.
    pub fn objects_mut(&mut self, wants: &dyn Fn(&InlineObject) -> bool, f: &mut dyn FnMut(&mut InlineObject)) {
        let mut stories: Vec<&mut Blocks> = vec![&mut self.body];
        stories.extend(self.parts.values_mut().map(|p| &mut p.blocks));
        for blocks in stories {
            for b in blocks.iter_mut() {
                let mut any = false;
                edit::each_para(b, 0, &mut |p| any |= p.objects.iter().any(wants));
                if any {
                    edit::each_para_mut(Arc::make_mut(b), 0, &mut |p| {
                        if p.objects.iter().any(wants) {
                            p.objects.iter_mut().filter(|o| wants(o)).for_each(&mut *f);
                            p.touch();
                        }
                    });
                }
            }
        }
    }

    /// Repair every paragraph's runs (see [`Paragraph::normalize`]); for models from untrusted
    /// JSON, whose run lengths may overflow, fall short or split a character.
    pub fn normalize_runs(&mut self) {
        let mut stories: Vec<&mut Blocks> = vec![&mut self.body];
        stories.extend(self.parts.values_mut().map(|p| &mut p.blocks));
        for blocks in stories {
            for b in blocks.iter_mut() {
                edit::each_para_mut(Arc::make_mut(b), 0, &mut Paragraph::normalize);
            }
        }
    }

    /// Number of paragraphs in the body (all depths).
    pub fn paragraph_count(&self) -> usize {
        self.para_paths(StoryRef::Body).len()
    }

    /// Add a media item; returns its key (deduplicated by content).
    pub fn add_media(&mut self, bytes: Vec<u8>, ext: &str) -> String {
        let ix = &mut self.media_index;
        if ix.len != self.media.len() {
            ix.rebuild(&self.media);
        }
        if let Some(k) = ix.find(&self.media, &bytes) {
            return k;
        }
        if ix.by_bytes.contains_key(&bytes) {
            // Stale (`media` was edited directly without changing its size): look again.
            ix.rebuild(&self.media);
            if let Some(k) = ix.find(&self.media, &bytes) {
                return k;
            }
        }
        let n = self.media.len().saturating_add(1);
        let mut i = n.max(ix.next);
        let mut key = format!("image{i}.{ext}");
        while self.media.contains_key(&key) {
            i = i.saturating_add(1);
            key = format!("image{i}.{ext}");
        }
        let bytes = Arc::new(bytes);
        self.media.insert(key.clone(), bytes.clone());
        ix.by_bytes.entry(bytes).or_insert_with(|| key.clone());
        ix.len = self.media.len();
        ix.next = i.saturating_add(1);
        key
    }

    /// The value of a custom property (names compare case-insensitively, as in Word).
    pub fn custom_prop(&self, name: &str) -> Option<&str> {
        self.custom_props.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| p.value.as_str())
    }

    /// Set a custom text property, replacing one of the same name in place.
    pub fn set_custom_prop(&mut self, name: &str, value: &str) {
        match self.custom_props.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) {
            Some(p) => {
                p.kind = "lpwstr".into();
                p.value = value.to_string();
            }
            None => {
                self.custom_props.push(CustomProp { name: name.to_string(), kind: "lpwstr".into(), value: value.to_string(), ..Default::default() })
            }
        }
    }

    /// Remove a custom property. Returns whether it existed.
    pub fn remove_custom_prop(&mut self, name: &str) -> bool {
        let n = self.custom_props.len();
        self.custom_props.retain(|p| !p.name.eq_ignore_ascii_case(name));
        self.custom_props.len() != n
    }

    /// Ranges that stay editable while the document is protected (Restrict Editing exceptions),
    /// in document order across every story: the [`InlineObject::PermStart`]'s id, group and
    /// editor, its start (just after the marker) and its end (at the matching
    /// [`InlineObject::PermEnd`], or the end of the story when it has none).
    pub fn perm_ranges(&self) -> Vec<PermRange> {
        let mut out: Vec<PermRange> = Vec::new();
        let stories = std::iter::once(StoryRef::Body).chain(self.parts.keys().map(|k| StoryRef::Part(*k)));
        for story in stories {
            // Open ranges by id, latest last (an end closes the latest open range of its id).
            let mut open: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
            for path in self.para_paths(story) {
                let Some(p) = self.para(story, &path) else { continue };
                for (off, o) in p.objects_with_offsets() {
                    match o {
                        InlineObject::PermStart { id, group, editor } => {
                            let start = Pos { story, path: path.clone(), off: off + para::OBJ.len_utf8() };
                            open.entry(*id).or_default().push(out.len());
                            out.push(PermRange { id: *id, group: group.clone(), editor: editor.clone(), start: start.clone(), end: start });
                        }
                        InlineObject::PermEnd { id } => {
                            if let Some(i) = open.get_mut(id).and_then(Vec::pop)
                                && let Some(r) = out.get_mut(i)
                            {
                                r.end = Pos { story, path: path.clone(), off };
                            }
                        }
                        _ => {}
                    }
                }
            }
            // A range left open runs to the end of its story.
            let end = self.end_of(story);
            for i in open.into_values().flatten() {
                if let Some(r) = out.get_mut(i) {
                    r.end = end.clone();
                }
            }
        }
        out
    }

    /// Bookmark names in the body, in order.
    pub fn bookmarks(&self) -> Vec<(String, Pos)> {
        let mut out = Vec::new();
        for path in self.para_paths(StoryRef::Body) {
            if let Some(p) = self.para(StoryRef::Body, &path) {
                for (off, o) in p.objects_with_offsets() {
                    if let InlineObject::BookmarkStart { name } = o {
                        out.push((name.clone(), Pos { story: StoryRef::Body, path: path.clone(), off }));
                    }
                }
            }
        }
        out
    }
}
