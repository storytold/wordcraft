//! WordCraft document model.
//!
//! A [`Document`] holds the main story (`body`) and secondary stories (`parts`: headers,
//! footers, notes, comments, text boxes). A story is a list of [`Block`]s: paragraphs and tables;
//! table cells hold block lists of their own. Blocks are `Arc`-shared, so cloning a document is
//! cheap (undo snapshots, handing a frozen copy to the renderer) and edits copy only what they
//! touch (`Arc::make_mut`).
//!
//! Positions ([`Pos`]) name a story, a [`Path`] to a paragraph and a byte offset in it. Paths
//! compare in document order.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod edit;
pub mod encoding;
pub mod fields;
pub mod numbering;
pub mod para;
pub mod props;
pub mod resolve;
pub mod section;
pub mod styles;
pub mod table;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use fields::FieldRange;
pub use numbering::{ListKind, Numbering};
pub use para::{InlineObject, Paragraph, Run};
pub use props::{Align, CharProps, ParaProps, Rgb, TextColor};
pub use section::SectionProps;
pub use styles::{Style, StyleKind, StyleSheet};
pub use table::{Cell, Row, Table};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum DocError {
    #[error("offset {0} is not a character boundary in the paragraph")]
    BadOffset(usize),
    #[error("no paragraph at {0}")]
    BadPath(String),
    #[error("no story {0:?}")]
    BadStory(StoryRef),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, DocError>;

/// A block in a story. (Blocks are always behind an `Arc`, so the size difference is fine.)
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Block {
    Para(Paragraph),
    Table(Table),
}

impl Block {
    pub fn as_para(&self) -> Option<&Paragraph> {
        match self {
            Block::Para(p) => Some(p),
            Block::Table(_) => None,
        }
    }
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Block::Table(t) => Some(t),
            Block::Para(_) => None,
        }
    }
}

pub type Blocks = Vec<Arc<Block>>;

pub fn para_block(p: Paragraph) -> Arc<Block> {
    Arc::new(Block::Para(p))
}

/// Which story a position is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StoryRef {
    #[default]
    Body,
    Part(u32),
}

/// Path to a paragraph: `[block]` in a story, `[block, row, cell, block]` inside a table cell,
/// three more indexes per nesting level. Lexicographic order = document order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Path(pub Vec<u32>);

impl Path {
    pub fn top(i: usize) -> Path {
        Path(vec![i as u32])
    }
    /// The block index within its container.
    pub fn last(&self) -> usize {
        self.0.last().copied().unwrap_or(0) as usize
    }
    pub fn with_last(&self, i: usize) -> Path {
        let mut p = self.clone();
        if let Some(l) = p.0.last_mut() {
            *l = i as u32;
        }
        p
    }
    /// The container prefix (everything but the last index).
    pub fn parent(&self) -> &[u32] {
        self.0.split_last().map(|(_, r)| r).unwrap_or(&[])
    }
    /// Nesting depth (0 = top-level story block).
    pub fn depth(&self) -> usize {
        self.0.len().saturating_sub(1) / 3
    }
    /// The enclosing table cell (table block path, row, cell) if any.
    pub fn cell(&self) -> Option<(Path, usize, usize)> {
        if self.0.len() < 4 {
            return None;
        }
        let n = self.0.len();
        let t = Path(self.0.get(..n - 3)?.to_vec());
        Some((t, *self.0.get(n - 3)? as usize, *self.0.get(n - 2)? as usize))
    }
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s: Vec<String> = self.0.iter().map(|i| i.to_string()).collect();
        write!(f, "{}", s.join("/"))
    }
}

/// A position in a document.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Pos {
    pub story: StoryRef,
    pub path: Path,
    pub off: usize,
}

impl Pos {
    pub fn new(story: StoryRef, path: Path, off: usize) -> Pos {
        Pos { story, path, off }
    }
    pub fn body(block: usize, off: usize) -> Pos {
        Pos { story: StoryRef::Body, path: Path::top(block), off }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PartKind {
    #[default]
    Header,
    Footer,
    Footnote,
    Endnote,
    Comment,
    TextBox,
}

/// A secondary story.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Part {
    pub kind: PartKind,
    pub blocks: Blocks,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Comment {
    pub author: String,
    pub initials: String,
    /// ISO 8601.
    pub date: String,
    /// Reply to this comment id.
    pub parent: Option<u32>,
    pub resolved: bool,
    /// `Document::parts` id of the comment text.
    pub part: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RevisionKind {
    #[default]
    Insert,
    Delete,
    Format,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Revision {
    pub kind: RevisionKind,
    pub author: String,
    pub date: String,
}

/// A bibliography source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Source {
    /// Unique tag cited by CITATION fields.
    pub tag: String,
    /// book, article, website, report, film, other
    pub kind: String,
    /// "Last, First; Last, First"
    pub author: String,
    pub title: String,
    pub year: String,
    pub publisher: String,
    pub city: String,
    pub journal: String,
    pub volume: String,
    pub pages: String,
    pub url: String,
}

/// A custom document property (File › Info › Properties › Custom). Citation managers keep
/// their per-document preferences here (Zotero: `ZOTERO_PREF_1`, `ZOTERO_PREF_2`…).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomProp {
    pub name: String,
    /// OOXML variant type: `lpwstr`, `i4`, `r8`, `bool`, `filetime`…, or `raw` when `value` is
    /// the property's XML content kept verbatim (vectors, blobs).
    pub kind: String,
    pub value: String,
}

/// Document properties (File › Info).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CoreProps {
    pub title: String,
    pub subject: String,
    pub creator: String,
    pub keywords: String,
    pub description: String,
    pub category: String,
    pub last_modified_by: String,
    pub created: String,
    pub modified: String,
    pub revision: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub track_changes: bool,
    /// Default tab stop interval, points.
    pub default_tab: f32,
    pub even_odd_headers: bool,
    pub mirror_margins: bool,
    pub auto_hyphenation: bool,
    pub page_color: Option<Rgb>,
    pub watermark: Option<Watermark>,
    /// Theme fonts (major = headings, minor = body).
    pub major_font: String,
    pub minor_font: String,
    /// Theme colours: dk1 lt1 dk2 lt2 accent1..6 hlink folHlink.
    pub theme_colors: Vec<Rgb>,
    pub theme_name: String,
    pub footnote_format: section::NumFormat,
    pub endnote_format: section::NumFormat,
    pub protection: Option<String>,
    /// Word compatibility mode the document is laid out in (`compatibilityMode`): 15 for Word
    /// 2013 and later, which places a table's border at the margin rather than its text and lets
    /// justified lines shrink their spaces to fit more text.
    pub compat_mode: u32,
}

/// Word's compatibility mode for documents that don't state one.
pub const LEGACY_COMPAT_MODE: u32 = 12;

/// The compatibility mode of documents created by Word 2013 and later, and by WordCraft.
pub const COMPAT_MODE_CURRENT: u32 = 15;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watermark {
    pub text: String,
    pub font: String,
    pub color: Rgb,
    pub diagonal: bool,
    pub semitransparent: bool,
}

impl Default for Watermark {
    fn default() -> Self {
        Watermark { text: "DRAFT".into(), font: styles::BODY_FONT.into(), color: Rgb(0xC0, 0xC0, 0xC0), diagonal: true, semitransparent: true }
    }
}

/// Our default theme colours.
pub const THEME_COLORS: [Rgb; 12] = [
    Rgb(0x00, 0x00, 0x00),
    Rgb(0xFF, 0xFF, 0xFF),
    Rgb(0x0E, 0x28, 0x41),
    Rgb(0xE8, 0xE8, 0xE8),
    Rgb(0x15, 0x60, 0x82),
    Rgb(0xE9, 0x71, 0x32),
    Rgb(0x19, 0x6B, 0x24),
    Rgb(0x0F, 0x9E, 0xD5),
    Rgb(0xA0, 0x2B, 0x93),
    Rgb(0x4E, 0xA7, 0x2E),
    Rgb(0x46, 0x78, 0x86),
    Rgb(0x96, 0x60, 0x7D),
];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            track_changes: false,
            default_tab: 36.0,
            even_odd_headers: false,
            mirror_margins: false,
            auto_hyphenation: false,
            page_color: None,
            watermark: None,
            major_font: styles::HEADING_FONT.into(),
            minor_font: styles::BODY_FONT.into(),
            theme_colors: THEME_COLORS.to_vec(),
            theme_name: "Craft".into(),
            footnote_format: section::NumFormat::Decimal,
            endnote_format: section::NumFormat::LowerRoman,
            protection: None,
            compat_mode: COMPAT_MODE_CURRENT,
        }
    }
}

/// A document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Document {
    pub body: Blocks,
    /// Properties of the last section (OOXML's body-level `w:sectPr`).
    pub last_section: SectionProps,
    pub parts: BTreeMap<u32, Part>,
    pub styles: StyleSheet,
    pub numbering: Numbering,
    pub comments: BTreeMap<u32, Comment>,
    pub revisions: Vec<Revision>,
    pub settings: Settings,
    pub core: CoreProps,
    /// Bibliography sources (References › Manage Sources).
    pub sources: Vec<Source>,
    /// Custom document properties, in file order.
    pub custom_props: Vec<CustomProp>,
    /// Embedded media (images) by key.
    #[serde(skip)]
    pub media: BTreeMap<String, Arc<Vec<u8>>>,
    /// Source-format parts we keep verbatim for round-trip (e.g. custom XML), by part name.
    #[serde(skip)]
    pub passthrough: BTreeMap<String, Arc<Vec<u8>>>,
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
            media: BTreeMap::new(),
            passthrough: BTreeMap::new(),
        }
    }

    /// A document from plain text, one paragraph per line.
    pub fn from_text(text: &str) -> Self {
        let mut d = Document::new();
        d.body = text.split('\n').map(|l| para_block(Paragraph::with_text(l.trim_end_matches('\r'), CharProps::default()))).collect();
        d.ensure_nonempty();
        d
    }

    /// Every story must hold at least one paragraph, and every cell too.
    pub fn ensure_nonempty(&mut self) {
        fn fix(bl: &mut Blocks) {
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
                                fix_cell(&mut c.blocks);
                            }
                        }
                    }
                }
            }
        }
        fn fix_cell(bl: &mut Blocks) {
            if bl.is_empty() || matches!(bl.last().map(|b| &**b), Some(Block::Table(_))) {
                bl.push(para_block(Paragraph::new()));
            }
        }
        fix(&mut self.body);
        for p in self.parts.values_mut() {
            if p.blocks.is_empty() {
                p.blocks.push(para_block(Paragraph::new()));
            }
        }
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
        let mut out = String::new();
        if let Some(bl) = self.story(s) {
            text_of(bl, &mut out, 0);
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

    /// Allocate a new part id and insert the part.
    pub fn add_part(&mut self, kind: PartKind, blocks: Blocks) -> u32 {
        let id = self.parts.keys().next_back().map(|k| k + 1).unwrap_or(1);
        let mut part = Part { kind, blocks };
        if part.blocks.is_empty() {
            part.blocks.push(para_block(Paragraph::new()));
        }
        self.parts.insert(id, part);
        id
    }

    /// Count words like Word's status bar (whitespace-separated tokens containing a letter or digit).
    pub fn word_count(&self) -> usize {
        count_words(&self.plain_text(StoryRef::Body))
    }

    /// Number of paragraphs in the body (all depths).
    pub fn paragraph_count(&self) -> usize {
        self.para_paths(StoryRef::Body).len()
    }

    /// Add a media item; returns its key (deduplicated by content).
    pub fn add_media(&mut self, bytes: Vec<u8>, ext: &str) -> String {
        if let Some((k, _)) = self.media.iter().find(|(_, v)| v.as_slice() == bytes.as_slice()) {
            return k.clone();
        }
        let n = self.media.len() + 1;
        let mut key = format!("image{n}.{ext}");
        let mut i = n;
        while self.media.contains_key(&key) {
            i += 1;
            key = format!("image{i}.{ext}");
        }
        self.media.insert(key.clone(), Arc::new(bytes));
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
            None => self.custom_props.push(CustomProp { name: name.to_string(), kind: "lpwstr".into(), value: value.to_string() }),
        }
    }

    /// Remove a custom property. Returns whether it existed.
    pub fn remove_custom_prop(&mut self, name: &str) -> bool {
        let n = self.custom_props.len();
        self.custom_props.retain(|p| !p.name.eq_ignore_ascii_case(name));
        self.custom_props.len() != n
    }

    /// Bookmark names in the body, in order.
    pub fn bookmarks(&self) -> Vec<(String, Pos)> {
        let mut out = Vec::new();
        for path in self.para_paths(StoryRef::Body) {
            if let Some(p) = self.para(StoryRef::Body, &path) {
                for off in p.object_offsets() {
                    if let Some(InlineObject::BookmarkStart { name }) = p.object_at(off) {
                        out.push((name.clone(), Pos { story: StoryRef::Body, path: path.clone(), off }));
                    }
                }
            }
        }
        out
    }
}

/// Words in `s` (tokens separated by whitespace that contain at least one alphanumeric char).
pub fn count_words(s: &str) -> usize {
    s.split(|c: char| c.is_whitespace() || c == para::OBJ).filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

fn walk(bl: &Blocks, prefix: &mut Vec<u32>, out: &mut Vec<Path>, depth: usize) {
    if depth > 16 {
        return;
    }
    for (i, b) in bl.iter().enumerate() {
        prefix.push(i as u32);
        match &**b {
            Block::Para(_) => out.push(Path(prefix.clone())),
            Block::Table(t) => {
                for (r, row) in t.rows.iter().enumerate() {
                    for (c, cell) in row.cells.iter().enumerate() {
                        prefix.push(r as u32);
                        prefix.push(c as u32);
                        walk(&cell.blocks, prefix, out, depth + 1);
                        prefix.pop();
                        prefix.pop();
                    }
                }
            }
        }
        prefix.pop();
    }
}

fn text_of(bl: &Blocks, out: &mut String, depth: usize) {
    if depth > 16 {
        return;
    }
    for b in bl {
        match &**b {
            Block::Para(p) => {
                out.push_str(&p.plain_text());
                out.push('\n');
            }
            Block::Table(t) => {
                for row in &t.rows {
                    let mut cells = Vec::new();
                    for c in &row.cells {
                        let mut s = String::new();
                        text_of(&c.blocks, &mut s, depth + 1);
                        cells.push(s.trim_end_matches('\n').replace('\n', " "));
                    }
                    out.push_str(&cells.join("\t"));
                    out.push('\n');
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
