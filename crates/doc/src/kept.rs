//! Markup kept from the file a document was opened from, so that what isn't edited is written
//! back as it was: a paragraph keeps its source XML, which a writer uses as long as the
//! paragraph still is what the reader made of it (its [`fingerprint`] unchanged). Anything the
//! model has no place for (an unknown element, an attribute nobody reads) survives a save that
//! way. Never saved with the document (WordCraft's own format), and not part of its content
//! (`==` ignores it).

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::Paragraph;

/// Most kept markup per document, bytes (a hostile file can't make the kept copy grow without
/// bound: past it, paragraphs are written from the model as before).
pub const MAX_KEPT_BYTES: usize = 64 * 1024 * 1024;

/// The kept markup of a document (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct Kept(Option<Arc<KeptXml>>);

/// What a reader kept.
#[derive(Clone, Debug, Default)]
pub struct KeptXml {
    /// The format it came from (`"odt"`): only that format's writer uses it.
    pub format: String,
    /// Paragraph [`fingerprint`] → the paragraph's markup, in the order read (identical
    /// paragraphs keep the first's).
    pub paras: HashMap<u64, Arc<str>>,
    /// The same for the body's tables ([`table_fingerprint`]).
    pub tables: HashMap<u64, Arc<str>>,
    /// While reading: the markup of the body's `n`th table, until [`Kept::seal`].
    pub pending_tables: Vec<(usize, Arc<str>)>,
    /// Wrappers kept as read around paragraphs (sections): for a paragraph's fingerprint, the
    /// start tags opening before it and how many close after it. Only paragraphs that were the
    /// only one of their kind have them (a copy can't open a section twice).
    pub wraps: HashMap<u64, (Vec<Arc<str>>, u32)>,
    /// While reading: the same by paragraph `rev`, until [`Kept::seal`].
    pub pending_wraps: HashMap<u64, (Vec<Arc<str>>, u32)>,
    /// The same for paragraphs with twins (an empty paragraph has many): by fingerprint and
    /// [`Paragraph::rev`], which stays while the paragraph does (a writer opens each once).
    pub twin_wraps: HashMap<(u64, u64), (Vec<Arc<str>>, u32)>,
    /// The same wrappers around the body's tables, by [`table_fingerprint`] (tables that are
    /// the only one of their kind).
    pub table_wraps: HashMap<u64, (Vec<Arc<str>>, u32)>,
    /// While reading: those of the body's `n`th table, until `seal_kept`.
    pub pending_table_wraps: Vec<(usize, (Vec<Arc<str>>, u32))>,
    /// Indexes kept as read: by their first paragraph's fingerprint, how many top-level
    /// paragraphs they take, those paragraphs' fingerprint together ([`run_fingerprint`]),
    /// and the index's markup. Written as read while those paragraphs are all as read.
    pub indexes: HashMap<u64, (usize, u64, Arc<str>)>,
    /// While reading: by the first paragraph's `rev`, how many paragraphs, and the markup.
    pub pending_indexes: HashMap<u64, (usize, Arc<str>)>,
    /// The group of paragraphs written as read together (see `wordcraft_formats::model::Group`)
    /// each paragraph kept belongs to, by its fingerprint: the group's id and size.
    pub groups: HashMap<u64, (u32, u32)>,
    /// While reading: the same by paragraph `rev`, until `seal_kept`.
    pub pending_groups: HashMap<u64, (u32, u32)>,
    /// Each text box's, shape's or note's story's [`story_fingerprint`] when read: a paragraph
    /// showing one is written as read only while it is as read.
    pub story_fps: HashMap<u32, u64>,
    /// Each top-level comment's [`comments_fingerprint`] when read (it and its replies): a
    /// paragraph holding comments is written as read only while they are as read.
    pub comment_fps: HashMap<u32, u64>,
    /// Named definitions the kept markup refers to (styles, font faces…), by a key the format
    /// chooses (`"auto:P12"`, `"style:Heading_20_1"`), as read.
    pub defs: BTreeMap<String, Arc<str>>,
    /// While reading: paragraph [`Paragraph::rev`] → its markup, until [`Kept::seal`] turns
    /// them into fingerprints.
    pub pending: HashMap<u64, Arc<str>>,
    /// Bytes kept so far.
    pub bytes: usize,
    /// The keys of [`KeptXml::defs`] in the order the file had them.
    pub order: Vec<String>,
    /// Files of the package kept definitions refer to (a style's fill picture), by path.
    pub files: BTreeMap<String, Arc<Vec<u8>>>,
    /// The document's style definitions' [`styles_fingerprint`] when read: while it is the
    /// same, the file's own style definitions can be written as they were.
    pub styles_fp: Option<u64>,
    /// The document's page setup's [`pages_fingerprint`] when read: while it is the same, the
    /// file's own page layouts and master pages can be written as they were.
    pub pages_fp: Option<u64>,
    /// Each section's master page, as the file named it where its section starts (`None`: the
    /// file's default), for a writer naming the file's own master pages.
    pub section_masters: Vec<Option<String>>,
}

impl PartialEq for Kept {
    /// Not part of the document's content.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Kept {
    pub fn new(x: KeptXml) -> Kept {
        Kept(Some(Arc::new(x)))
    }
    pub fn get(&self) -> Option<&KeptXml> {
        self.0.as_deref()
    }
    /// What `format`'s writer may use, if anything was kept from such a file.
    pub fn of(&self, format: &str) -> Option<&KeptXml> {
        self.get().filter(|k| k.format == format)
    }
    pub fn get_mut(&mut self) -> Option<&mut KeptXml> {
        self.0.as_mut().map(Arc::make_mut)
    }
    pub fn clear(&mut self) {
        self.0 = None;
    }
    /// After reading: each paragraph still marked pending (by its `rev`, which any change after
    /// the reader made it would have moved on) gets its markup under its fingerprint.
    pub fn seal(&mut self, paras: &mut dyn Iterator<Item = &Paragraph>) {
        let Some(k) = self.get_mut() else { return };
        if k.pending.is_empty() {
            return;
        }
        for p in paras {
            if let Some(xml) = k.pending.remove(&p.rev) {
                k.paras.entry(fingerprint(p)).or_insert(xml);
            }
        }
        k.pending.clear();
    }
}

impl KeptXml {
    /// The markup kept for paragraph `p`, while it still is what was read.
    pub fn para(&self, p: &Paragraph) -> Option<&Arc<str>> {
        if self.paras.is_empty() {
            return None;
        }
        self.paras.get(&fingerprint(p))
    }
    /// The markup kept for table `t`, while it still is what was read.
    pub fn table(&self, t: &crate::Table) -> Option<&Arc<str>> {
        if self.tables.is_empty() {
            return None;
        }
        self.tables.get(&table_fingerprint(t))
    }
    /// Whether the comments paragraph `p` holds (those it begins, with their replies) are as
    /// read and are all the comments its kept markup `xml` holds: a reply written elsewhere
    /// in the file, or one added since, isn't in it.
    pub fn comments_as_read(&self, doc: &crate::Document, p: &Paragraph, xml: &str) -> bool {
        let in_markup = xml.matches("<office:annotation ").count() + xml.matches("<office:annotation>").count();
        let mut count = 0usize;
        for o in &p.objects {
            if let crate::para::InlineObject::CommentStart { id } = o {
                let (fp, n) = comments_fingerprint(doc, *id);
                if self.comment_fps.get(id) != Some(&fp) {
                    return false;
                }
                count = count.saturating_add(n);
            }
        }
        count == in_markup
    }
    /// Whether the stories paragraph `p` shows (its text boxes' and shapes' text, its notes') are
    /// as read.
    pub fn stories_as_read(&self, doc: &crate::Document, p: &Paragraph) -> bool {
        // A note's citation is its number in the document's numbering, which is the page setup's.
        let notes = p.objects.iter().any(|o| matches!(o, crate::para::InlineObject::NoteRef { .. }));
        if notes && !self.pages_as_read(doc) {
            return false;
        }
        p.objects
            .iter()
            .flat_map(|o| match o {
                crate::para::InlineObject::NoteRef { id, .. } => vec![*id],
                o => o.text_boxes(),
            })
            .all(|id| self.story_fps.get(&id).is_some_and(|fp| *fp == story_fingerprint(doc, id)))
    }
    /// The wrappers kept around table `t`, while it still is what was read.
    pub fn table_wrap(&self, t: &crate::Table) -> Option<&(Vec<Arc<str>>, u32)> {
        if self.table_wraps.is_empty() {
            return None;
        }
        self.table_wraps.get(&table_fingerprint(t))
    }
    /// The wrappers kept around paragraph `p`, while it still is what was read.
    pub fn wrap(&self, p: &Paragraph) -> Option<&(Vec<Arc<str>>, u32)> {
        if self.wraps.is_empty() && self.twin_wraps.is_empty() {
            return None;
        }
        let fp = fingerprint(p);
        self.wraps.get(&fp).or_else(|| self.twin_wraps.get(&(fp, p.rev)))
    }
    /// The index kept as read starting at paragraph `p`: how many paragraphs it takes, their
    /// [`run_fingerprint`] when read, its markup.
    pub fn index(&self, p: &Paragraph) -> Option<&(usize, u64, Arc<str>)> {
        if self.indexes.is_empty() {
            return None;
        }
        self.indexes.get(&fingerprint(p))
    }
    /// Keep an index's markup, starting at the paragraph whose `rev` is `rev` and taking `n`.
    pub fn keep_pending_index(&mut self, rev: u64, n: usize, xml: Arc<str>) {
        if self.bytes.saturating_add(xml.len()) <= MAX_KEPT_BYTES {
            self.bytes += xml.len();
            self.pending_indexes.insert(rev, (n, xml));
        }
    }
    /// The paragraph whose `rev` is `rev` belongs to group `group` (id, size).
    pub fn keep_pending_group(&mut self, rev: u64, group: (u32, u32)) {
        if self.pending_groups.len() < 100_000 {
            self.pending_groups.insert(rev, group);
        }
    }
    /// The group paragraph `p`'s markup kept as read belongs to (id, size).
    pub fn group(&self, p: &Paragraph) -> Option<(u32, u32)> {
        if self.groups.is_empty() {
            return None;
        }
        self.groups.get(&fingerprint(p)).copied()
    }
    /// Keep wrapper markup for the paragraph whose `rev` is `rev`, within the budget.
    pub fn keep_pending_wrap(&mut self, rev: u64, wrap: (Vec<Arc<str>>, u32)) {
        let n: usize = wrap.0.iter().map(|t| t.len()).sum();
        if self.bytes.saturating_add(n) <= MAX_KEPT_BYTES {
            self.bytes += n;
            self.pending_wraps.insert(rev, wrap);
        }
    }
    /// Keep wrapper markup for the body's `n`th table, within the budget.
    pub fn keep_pending_table_wrap(&mut self, n: usize, wrap: (Vec<Arc<str>>, u32)) {
        let size: usize = wrap.0.iter().map(|t| t.len()).sum();
        if self.bytes.saturating_add(size) <= MAX_KEPT_BYTES {
            self.bytes += size;
            self.pending_table_wraps.push((n, wrap));
        }
    }
    /// Keep `xml` for the body's `n`th table, within the budget.
    pub fn keep_pending_table(&mut self, n: usize, xml: Arc<str>) {
        if self.bytes.saturating_add(xml.len()) <= MAX_KEPT_BYTES {
            self.bytes += xml.len();
            self.pending_tables.push((n, xml));
        }
    }
    /// Keep `xml` for the paragraph whose `rev` is `rev`, within the budget.
    pub fn keep_pending(&mut self, rev: u64, xml: Arc<str>) {
        if self.bytes.saturating_add(xml.len()) <= MAX_KEPT_BYTES {
            self.bytes += xml.len();
            self.pending.insert(rev, xml);
        }
    }
    /// Keep a named definition, within the budget.
    pub fn keep_def(&mut self, key: String, xml: Arc<str>) {
        if self.bytes.saturating_add(xml.len()) <= MAX_KEPT_BYTES && !self.defs.contains_key(&key) {
            self.bytes += xml.len();
            self.order.push(key.clone());
            self.defs.insert(key, xml);
        }
    }
    /// Keep a file kept definitions refer to, within the budget.
    pub fn keep_file(&mut self, path: String, bytes: Vec<u8>) {
        if self.bytes.saturating_add(bytes.len()) <= MAX_KEPT_BYTES && !self.files.contains_key(&path) {
            self.bytes += bytes.len();
            self.files.insert(path, Arc::new(bytes));
        }
    }
    /// Whether `doc`'s page setup is still as it was read.
    pub fn pages_as_read(&self, doc: &crate::Document) -> bool {
        self.pages_fp.is_some_and(|fp| fp == pages_fingerprint(doc))
    }
    /// Whether `doc`'s style definitions are still as they were read.
    pub fn styles_as_read(&self, doc: &crate::Document) -> bool {
        self.styles_fp.is_some_and(|fp| fp == styles_fingerprint(doc))
    }
}

/// What `doc`'s style definitions are: its styles, lists and default tab interval.
pub fn styles_fingerprint(doc: &crate::Document) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match serde_json::to_vec(&(&doc.styles, &doc.numbering, doc.settings.default_tab)) {
        Ok(v) => v.hash(&mut h),
        Err(_) => u64::MAX.hash(&mut h),
    }
    h.finish()
}

/// What a block is, as the kept markup knows it: its paragraphs' fingerprints and
/// [`Paragraph::rev`]s in order, and a table's [`table_fingerprint`].
pub struct Identities {
    paras: Vec<(u64, u64)>,
    table: Option<u64>,
}

/// A block's [`Identities`] before and after a change that keeps what it is.
pub type Moved = (Identities, Identities);

/// The [`Identities`] of block `b`.
pub fn identities(b: &crate::Block) -> Identities {
    let mut paras_of = Vec::new();
    match b {
        crate::Block::Para(p) => paras_of.push(p),
        crate::Block::Table(t) => {
            for r in &t.rows {
                for c in &r.cells {
                    paras(&c.blocks, 1, &mut paras_of);
                }
            }
        }
    }
    Identities { paras: paras_of.iter().map(|p| (fingerprint(p), p.rev)).collect(), table: b.as_table().map(table_fingerprint) }
}

impl KeptXml {
    /// The markup kept for a block's paragraphs and table follows them from `before` to `after`
    /// (what they were and are: see [`crate::Document::fill_objects`]).
    pub fn follow(&mut self, before: &Identities, after: &Identities) {
        for ((fp, rev), (nfp, nrev)) in before.paras.iter().zip(&after.paras) {
            if fp == nfp && rev == nrev {
                continue;
            }
            if let Some(x) = self.paras.remove(fp) {
                self.paras.entry(*nfp).or_insert(x);
            }
            if let Some(w) = self.wraps.remove(fp) {
                self.wraps.entry(*nfp).or_insert(w);
            }
            if let Some(w) = self.twin_wraps.remove(&(*fp, *rev)) {
                self.twin_wraps.entry((*nfp, *nrev)).or_insert(w);
            }
            // Its group too: all a group's paragraphs are written as read, or none.
            if let Some(g) = self.groups.remove(fp) {
                self.groups.entry(*nfp).or_insert(g);
            }
        }
        if let (Some(t), Some(nt)) = (before.table, after.table)
            && t != nt
        {
            if let Some(x) = self.tables.remove(&t) {
                self.tables.entry(nt).or_insert(x);
            }
            if let Some(w) = self.table_wraps.remove(&t) {
                self.table_wraps.entry(nt).or_insert(w);
            }
        }
    }
}

/// How deep [`story_fingerprint`] follows stories in stories (text boxes in text boxes, notes).
const MAX_STORY_DEPTH: usize = 8;

/// What story `id` (a text box's or a shape's text) is: its blocks, and the stories and notes
/// they hold in turn.
pub fn story_fingerprint(doc: &crate::Document, id: u32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut todo = vec![(id, 0usize)];
    let mut seen = std::collections::HashSet::new();
    while let Some((id, depth)) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let blocks = doc.parts.get(&id).map(|p| &p.blocks);
        match serde_json::to_vec(&(id, blocks)) {
            Ok(v) => v.hash(&mut h),
            Err(_) => u64::MAX.hash(&mut h),
        }
        if depth >= MAX_STORY_DEPTH {
            continue;
        }
        let Some(blocks) = blocks else { continue };
        let mut held = Vec::new();
        paras(blocks, 0, &mut held);
        for p in held {
            for o in &p.objects {
                todo.extend(o.text_boxes().into_iter().map(|s| (s, depth + 1)));
                if let crate::para::InlineObject::NoteRef { id, .. } = o {
                    todo.push((*id, depth + 1));
                }
            }
        }
    }
    h.finish()
}

/// Most replies a comment's fingerprint follows (hostile input).
const MAX_REPLIES: usize = 10_000;

/// What comment `root` is with its replies (and theirs): each one's author, date, state and
/// text, and how many they are.
pub fn comments_fingerprint(doc: &crate::Document, root: u32) -> (u64, usize) {
    family_fingerprint(doc, root, &replies(doc))
}

/// Each comment's replies, by the comment's id.
fn replies(doc: &crate::Document) -> HashMap<u32, Vec<u32>> {
    let mut out: HashMap<u32, Vec<u32>> = HashMap::new();
    for (id, c) in &doc.comments {
        if let Some(p) = c.parent {
            out.entry(p).or_default().push(*id);
        }
    }
    out
}

/// [`comments_fingerprint`], with each comment's `replies`.
fn family_fingerprint(doc: &crate::Document, root: u32, replies: &HashMap<u32, Vec<u32>>) -> (u64, usize) {
    let mut ids = vec![root];
    let mut seen = std::collections::HashSet::from([root]);
    let mut i = 0;
    while let Some(&id) = ids.get(i) {
        for r in replies.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
            // (A reply naming itself, or a circle of them, is followed once.)
            if ids.len() < MAX_REPLIES && seen.insert(*r) {
                ids.push(*r);
            }
        }
        i += 1;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for id in &ids {
        let c = doc.comments.get(id);
        let text = c.and_then(|c| doc.parts.get(&c.part)).map(|p| &p.blocks);
        match serde_json::to_vec(&(c, text)) {
            Ok(v) => v.hash(&mut h),
            Err(_) => u64::MAX.hash(&mut h),
        }
    }
    (h.finish(), ids.len())
}

/// What `doc`'s page setup is: each section's setup (where it starts, its size, margins,
/// columns, headers and footers), the headers' and footers' content, the page colour and
/// picture, and the notes' numbering.
pub fn pages_fingerprint(doc: &crate::Document) -> u64 {
    let mut sections: Vec<&crate::SectionProps> = doc.body.iter().filter_map(|b| b.as_para()).filter_map(|p| p.section.as_deref()).collect();
    sections.push(&doc.last_section);
    let parts: Vec<(&u32, &crate::Part)> =
        doc.parts.iter().filter(|(_, p)| matches!(p.kind, crate::PartKind::Header | crate::PartKind::Footer)).collect();
    let s = &doc.settings;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match serde_json::to_vec(&(
        sections,
        parts,
        &s.page_color,
        &s.page_picture,
        (&s.footnote_format, &s.endnote_format, &s.footnote_pr, &s.endnote_pr),
        (&doc.footnote_separators, &doc.endnote_separators),
    )) {
        Ok(v) => v.hash(&mut h),
        Err(_) => u64::MAX.hash(&mut h),
    }
    h.finish()
}

/// What a run of paragraphs is in the model, together.
pub fn run_fingerprint<'a>(paras: impl Iterator<Item = &'a Paragraph>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in paras {
        fingerprint(p).hash(&mut h);
    }
    h.finish()
}

/// What table `t` is in the model.
pub fn table_fingerprint(t: &crate::Table) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match serde_json::to_vec(t) {
        Ok(v) => v.hash(&mut h),
        Err(_) => u64::MAX.hash(&mut h),
    }
    h.finish()
}

/// What paragraph `p` is in the model (its text, runs, objects and properties, not its `rev`):
/// equal for a paragraph nobody changed.
pub fn fingerprint(p: &Paragraph) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match serde_json::to_vec(p) {
        Ok(v) => v.hash(&mut h),
        // Unserializable (no paragraph is): a value no paragraph's markup is kept under.
        Err(_) => u64::MAX.hash(&mut h),
    }
    h.finish()
}

/// Deepest table nesting [`crate::Document::seal_kept`] looks into.
const MAX_SEAL_DEPTH: usize = 16;

fn paras<'a>(bl: &'a crate::Blocks, depth: usize, out: &mut Vec<&'a Paragraph>) {
    for b in bl {
        match &**b {
            crate::Block::Para(p) => out.push(p),
            crate::Block::Table(t) if depth < MAX_SEAL_DEPTH => {
                for r in &t.rows {
                    for c in &r.cells {
                        paras(&c.blocks, depth + 1, out);
                    }
                }
            }
            crate::Block::Table(_) => {}
        }
    }
}

impl crate::Document {
    /// After reading: the markup kept for paragraphs still as the reader made them is filed
    /// under their fingerprints (see [`Kept::seal`]); the rest is dropped.
    pub fn seal_kept(&mut self) {
        // The body's tables kept, by their place among its tables; its indexes, by their run of
        // top-level paragraphs.
        if let Some(k) = self.kept.get_mut() {
            let pending = std::mem::take(&mut k.pending_indexes);
            for (i, b) in self.body.iter().enumerate() {
                let Some(p) = b.as_para() else { continue };
                let Some((n, xml)) = pending.get(&p.rev) else { continue };
                let run: Option<Vec<&Paragraph>> = self.body.get(i..i + n).map(|r| r.iter().filter_map(|b| b.as_para()).collect());
                if let Some(run) = run.filter(|r| r.len() == *n) {
                    k.indexes.entry(fingerprint(p)).or_insert((*n, run_fingerprint(run.into_iter()), xml.clone()));
                }
            }
            let pending = std::mem::take(&mut k.pending_tables);
            let tables: Vec<&crate::Table> = self.body.iter().filter_map(|b| b.as_table()).collect();
            for (n, xml) in pending {
                if let Some(t) = tables.get(n) {
                    k.tables.entry(table_fingerprint(t)).or_insert(xml);
                }
            }
            // Wrappers, for tables that are the only one of their kind (a copy can't open a
            // section twice).
            let pending = std::mem::take(&mut k.pending_table_wraps);
            if !pending.is_empty() {
                let fps: Vec<u64> = tables.iter().map(|t| table_fingerprint(t)).collect();
                for (n, w) in pending {
                    if let Some(fp) = fps.get(n)
                        && fps.iter().filter(|f| *f == fp).count() == 1
                    {
                        k.table_wraps.insert(*fp, w);
                    }
                }
            }
        }
        // The text boxes' and shapes' stories as read, for paragraphs showing them.
        if self.kept.get().is_some_and(|k| !k.pending.is_empty()) {
            let fps: HashMap<u32, u64> = self
                .parts
                .iter()
                .filter(|(_, p)| matches!(p.kind, crate::PartKind::TextBox | crate::PartKind::Footnote | crate::PartKind::Endnote))
                .map(|(id, _)| (*id, story_fingerprint(self, *id)))
                .collect();
            if let Some(k) = self.kept.get_mut() {
                k.story_fps = fps;
            }
        }
        // The comments as read, for paragraphs holding them.
        if self.kept.get().is_some_and(|k| !k.pending.is_empty()) && !self.comments.is_empty() {
            let replies = replies(self);
            let fps: HashMap<u32, u64> =
                self.comments.iter().filter(|(_, c)| c.parent.is_none()).map(|(id, _)| (*id, family_fingerprint(self, *id, &replies).0)).collect();
            if let Some(k) = self.kept.get_mut() {
                k.comment_fps = fps;
            }
        }
        if self.kept.get().is_none_or(|k| k.pending.is_empty() && k.pending_wraps.is_empty()) {
            return;
        }
        let mut all = Vec::new();
        paras(&self.body, 0, &mut all);
        for p in self.parts.values() {
            paras(&p.blocks, 0, &mut all);
        }
        let mut kept = std::mem::take(&mut self.kept);
        // Groups, by fingerprint; a group with a paragraph that has twins (which a writer can't
        // tell apart) isn't kept.
        if let Some(k) = kept.get_mut()
            && !k.pending_groups.is_empty()
        {
            let mut count: HashMap<u64, usize> = HashMap::new();
            let fps: Vec<(u64, u64)> = all.iter().map(|p| (p.rev, fingerprint(p))).collect();
            for (_, fp) in &fps {
                *count.entry(*fp).or_default() += 1;
            }
            let pending = std::mem::take(&mut k.pending_groups);
            let mut broken = std::collections::HashSet::new();
            let mut members: Vec<(u64, u64, (u32, u32))> = Vec::new();
            for (rev, fp) in &fps {
                if let Some(g) = pending.get(rev) {
                    if count.get(fp) != Some(&1) {
                        broken.insert(g.0);
                    }
                    members.push((*rev, *fp, *g));
                }
            }
            for (rev, fp, g) in members {
                if broken.contains(&g.0) {
                    k.pending.remove(&rev);
                } else {
                    k.groups.insert(fp, g);
                }
            }
        }
        // Wrappers, for paragraphs that are the only one of their kind.
        if let Some(k) = kept.get_mut()
            && !k.pending_wraps.is_empty()
        {
            let mut count: HashMap<u64, usize> = HashMap::new();
            let fps: Vec<(u64, u64)> = all.iter().map(|p| (p.rev, fingerprint(p))).collect();
            for (_, fp) in &fps {
                *count.entry(*fp).or_default() += 1;
            }
            for (rev, fp) in fps {
                if let Some(w) = k.pending_wraps.remove(&rev) {
                    if count.get(&fp) == Some(&1) {
                        k.wraps.insert(fp, w);
                    } else {
                        k.twin_wraps.insert((fp, rev), w);
                    }
                }
            }
            k.pending_wraps.clear();
        }
        kept.seal(&mut all.into_iter());
        self.kept = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_keeps_its_markup_until_it_changes() {
        let mut p = Paragraph::with_text("Hello", Default::default());
        let mut kept = Kept::new(KeptXml { format: "odt".into(), ..Default::default() });
        if let Some(k) = kept.get_mut() {
            k.keep_pending(p.rev, Arc::from("<text:p>Hello</text:p>"));
        }
        kept.seal(&mut std::iter::once(&p));
        let k = kept.of("odt").unwrap();
        assert_eq!(k.para(&p).map(|x| &**x), Some("<text:p>Hello</text:p>"));
        assert!(k.pending.is_empty());
        // The same paragraph again (a copy, its rev moved on): still what was read.
        p.touch();
        assert!(k.para(&p).is_some());
        // Changed: written from the model.
        p.text.push('!');
        assert!(k.para(&p).is_none());
        assert!(kept.of("docx").is_none());
    }

    #[test]
    fn a_paragraph_changed_while_reading_keeps_nothing() {
        let mut p = Paragraph::with_text("Hi", Default::default());
        let mut kept = Kept::new(KeptXml { format: "odt".into(), ..Default::default() });
        if let Some(k) = kept.get_mut() {
            k.keep_pending(p.rev, Arc::from("<text:p>Hi</text:p>"));
        }
        p.touch();
        kept.seal(&mut std::iter::once(&p));
        assert!(kept.of("odt").unwrap().paras.is_empty());
    }

    #[test]
    fn kept_markup_is_bounded() {
        let mut k = KeptXml::default();
        let big: Arc<str> = Arc::from("x".repeat(MAX_KEPT_BYTES / 2 + 1));
        k.keep_pending(1, big.clone());
        k.keep_pending(2, big);
        assert_eq!(k.pending.len(), 1);
    }

    /// Filling in what objects show (`Document::fill_objects`, a chart drawn from its data)
    /// doesn't change the document: a paragraph's group and a text box's story stay as read.
    #[test]
    fn filling_objects_keeps_groups_and_stories_as_read() {
        use crate::para::InlineObject;
        let field =
            || InlineObject::Field { instr: "X".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() };
        let mut doc = crate::Document::from_text("Body");
        let mut boxed = Paragraph::with_text("In the box", Default::default());
        boxed.insert_object(0, field(), &Default::default()).unwrap();
        let id = doc.add_part(crate::PartKind::TextBox, vec![crate::para_block(boxed)]);
        doc.para_mut(crate::StoryRef::Body, &crate::Path::top(0)).unwrap().insert_object(0, field(), &Default::default()).unwrap();
        let fp = fingerprint(doc.body[0].as_para().unwrap());
        let mut k = KeptXml { format: "odt".into(), ..Default::default() };
        k.paras.insert(fp, Arc::from("<text:p>Body</text:p>"));
        k.groups.insert(fp, (1, 1));
        k.story_fps.insert(id, story_fingerprint(&doc, id));
        doc.kept = Kept::new(k);
        doc.fill_objects(&|o| matches!(o, InlineObject::Field { result, .. } if result.is_empty()), &mut |o| {
            if let InlineObject::Field { result, .. } = o {
                *result = "filled".into();
            }
        });
        let p = doc.body[0].as_para().unwrap();
        assert_ne!(fingerprint(p), fp, "the paragraph's fingerprint moved");
        let k = doc.kept.get().unwrap();
        assert!(k.para(p).is_some());
        assert_eq!(k.group(p), Some((1, 1)));
        assert_eq!(k.story_fps.get(&id), Some(&story_fingerprint(&doc, id)));
    }
}
