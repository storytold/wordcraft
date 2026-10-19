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

pub mod bidi;
mod byte_store;
pub mod cfb_guard;
pub mod chart;
mod cjk_numbers;
pub mod cluster;
pub mod collate;
pub mod connector;
pub mod control;
pub mod edit;
pub mod effects;
pub mod encoding;
pub mod eq;
pub mod fields;
pub mod fill;
pub mod freeform;
pub mod graphic;
pub mod ime_text;
pub mod legacy_form;
pub mod line_dash;
pub mod link;
pub mod locale_number;
pub mod math;
pub mod math_edit;
pub mod math_latex;
pub mod math_linear;
pub mod math_symbols;
pub mod numbering;
pub mod para;
pub mod props;
pub mod resolve;
pub mod section;
pub mod smart_art;
pub mod styles;
pub mod symbol_fonts;
pub mod table;
pub mod wordart;
pub mod wrap;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use control::{ContentControl, ControlKind, ControlLock, ControlRange, ControlWrap};
pub use fields::FieldRange;
pub use numbering::{ListKind, Numbering};
pub use para::{InlineObject, Paragraph, Run};
pub use props::{Align, CharProps, ParaProps, Rgb, TextColor};
pub use section::SectionProps;
pub use styles::{Style, StyleKind, StyleSheet};
pub use table::{Cell, Row, Table};

mod review;

pub use review::{AUTO_TEXT_GALLERY, BuildingBlock, Comment, QUICK_PARTS_GALLERY, Revision, RevisionKind};

mod block;
mod compare;
mod document;
mod error;
mod parts;
mod position;
mod settings;
mod text_walk;

pub use block::{Block, Blocks, BoxBudget, MAX_TEXT_BOX_DEPTH};
pub use compare::para_block;
use document::MediaIndex;
pub use error::{DocError, Result};
pub use parts::{MAX_LINKED_BOXES, MAX_TEXT_BOX_INSET, NoteSeparators, Part, PartKind, PermRange, TEXT_BOX_INSET, TextBody, free_id};
pub use position::{Path, Pos, StoryRef};
pub use settings::{
    COMPAT_MODE_CURRENT, CoreProps, CustomProp, DEFAULT_GRID, DEFAULT_HYPHENATION_ZONE, LEGACY_COMPAT_MODE, PagePicture, Settings, Source,
    THEME_COLORS, Watermark, Zoom, ZoomFit,
};
pub use text_walk::count_words;
pub(crate) use text_walk::{PARA_WALK_DEPTH, walk_between};
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

/// Text boxes inside text boxes count (and show) this many levels deep.
pub const MAX_TEXT_BOX_DEPTH: usize = 4;

/// Bounds the work of expanding text boxes inside text boxes (layout, walks, saving), whatever a
/// document says: a box isn't expanded inside itself, nesting stops at [`MAX_TEXT_BOX_DEPTH`], and
/// an outermost box expands at most [`BoxBudget::MAX_NESTED`] boxes inside it. (A box whose
/// shapes all show that same box, or chains of boxes each showing the next many times, would
/// otherwise grow exponentially.) Outermost boxes aren't capped: their number is the document's
/// (or its pages', for headers), so the work stays linear and no box is left blank for budget.
#[derive(Debug, Default)]
pub struct BoxBudget {
    open: Vec<u32>,
    nested: usize,
    total: usize,
}

impl BoxBudget {
    /// Boxes expanded inside one outermost box.
    pub const MAX_NESTED: usize = 64;

    /// Start expanding text box story `id`; false if it mustn't be (pair a true with `leave`).
    pub fn enter(&mut self, id: u32) -> bool {
        if self.open.contains(&id) || self.open.len() >= MAX_TEXT_BOX_DEPTH {
            return false;
        }
        if self.open.is_empty() {
            self.nested = 0;
        } else if self.nested >= Self::MAX_NESTED {
            return false;
        } else {
            self.nested += 1;
        }
        self.total += 1;
        self.open.push(id);
        true
    }

    /// Done expanding the box last entered.
    pub fn leave(&mut self) {
        self.open.pop();
    }

    /// Whether a text box is being expanded now.
    pub fn inside(&self) -> bool {
        !self.open.is_empty()
    }

    /// Boxes expanded so far.
    pub fn total(&self) -> usize {
        self.total
    }
}

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

/// A document's own separators and continuation notice for its footnotes or its endnotes
/// (ECMA-376 §17.11: the `w:footnote`/`w:endnote` entries of type `separator`,
/// `continuationSeparator` and `continuationNotice`). `None` = the default: a short line above
/// the notes, a line across the column above notes continued from the page before, no notice.
/// The lines themselves are [`InlineObject::note_separator`] objects in the paragraphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteSeparators {
    /// Above the notes on a page.
    pub separator: Option<Blocks>,
    /// Above the rest of a note continued from the page before.
    pub continuation_separator: Option<Blocks>,
    /// Below the part of a note that continues on the next page.
    pub continuation_notice: Option<Blocks>,
}

impl NoteSeparators {
    /// No separator of the document's own: the defaults apply.
    pub fn is_empty(&self) -> bool {
        self.separator.is_none() && self.continuation_separator.is_none() && self.continuation_notice.is_none()
    }
}

/// A secondary story.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Part {
    pub kind: PartKind,
    pub blocks: Blocks,
    /// A text box's text direction, alignment and link (unused by other parts).
    #[serde(default, skip_serializing_if = "TextBody::is_default")]
    pub body: TextBody,
}

/// How a text box shows its text (Shape Format › Text): which way it runs, where it sits between
/// the box's top and bottom, and the box its overflow continues in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TextBody {
    pub vert: props::TextVert,
    pub anchor: props::VAlign,
    /// Create Link: the text box (another `TextBox` part, shown by its own shape) the text
    /// continues in when it doesn't fit this one. The linked box shows the rest of this box's
    /// story; its own blocks are unused. See [`Document::text_box_chains`].
    pub next: Option<u32>,
    /// Internal margins (left, top, right, bottom), EMU, when the file gives its own
    /// (`wps:bodyPr` `lIns`… ECMA-376 Part 1 §21.1.2.1.1, or VML `v:textbox` `inset`);
    /// `None` is Word's defaults.
    pub inset: Option<[u32; 4]>,
    /// Resize shape to fit text (`a:spAutoFit`, VML `mso-fit-shape-to-text`): Word grows the
    /// box to its text, so none of the text is cut.
    pub auto_fit: bool,
    /// The colour the shape's style gives its text (`wps:style/a:fontRef`): what automatic text
    /// colour means inside this box, instead of black. Text whose colour comes from its runs,
    /// styles or the document defaults keeps it (Word, observed with synthetic documents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_color: Option<props::Rgb>,
}

/// Word's default text box margins (left, top, right, bottom), EMU: 0.1" and 0.05".
pub const TEXT_BOX_INSET: [u32; 4] = [91_440, 45_720, 91_440, 45_720];
/// Widest text box margin kept, EMU (22", the largest page side).
pub const MAX_TEXT_BOX_INSET: u32 = 20_116_800;

impl TextBody {
    pub fn is_default(&self) -> bool {
        *self == TextBody::default()
    }

    /// The internal margins (left, top, right, bottom) in points.
    pub fn insets_pt(&self) -> [f32; 4] {
        self.inset.unwrap_or(TEXT_BOX_INSET).map(|v| v.min(MAX_TEXT_BOX_INSET) as f32 / 12_700.0)
    }
}

/// Most boxes one chain of linked text boxes runs through (hostile files).
pub const MAX_LINKED_BOXES: usize = 256;

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
    /// Word's durable id for the comment (`w16cid:durableId`, hex), kept across saves so Word
    /// keeps tracking the same comment. `None` for comments made here: the writer assigns one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub durable_id: Option<String>,
    /// The comment's UTC timestamp from the file (`w16cex:dateUtc`); `date` is local time there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_utc: Option<String>,
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
    /// A move (ECMA-376 §17.13.5.21–28): the name linking the source (an `Delete` revision,
    /// `w:moveFrom`) with the destination (an `Insert` revision, `w:moveTo`). `None` = a plain
    /// insertion or deletion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_name: Option<String>,
}

impl Revision {
    pub fn new(kind: RevisionKind, author: impl Into<String>, date: impl Into<String>) -> Revision {
        Revision { kind, author: author.into(), date: date.into(), move_name: None }
    }
}

impl Document {
    /// The move name of revision `idx`, when it is one end of a move.
    pub fn move_name(&self, idx: Option<u32>) -> Option<&str> {
        idx.and_then(|i| self.revisions.get(i as usize)).and_then(|r| r.move_name.as_deref())
    }
}

/// A building block (Insert › Quick Parts, AutoText…) kept in the document's glossary
/// (ECMA-376 §17.12: `w:docPart`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BuildingBlock {
    /// The entry's name (`w:docPartPr/w:name`).
    pub name: String,
    /// Its gallery (`w:category/w:gallery`: `docParts` for Quick Parts, `autoTxt` for AutoText…).
    pub gallery: String,
    /// Its category within the gallery (`w:category/w:name`, e.g. "General").
    pub category: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The entry's identity (`w:guid`), kept as read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub guid: String,
    /// The paragraph style it's associated with (`w:style`), kept as read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub style: String,
    /// Its types (`w:types/w:type`: `normal`, `bbPlcHdr` for a content control's placeholder…).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    /// How it's inserted (`w:behaviors/w:behavior`: `content`, `p`, `pg`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub behaviors: Vec<String>,
    /// The content (`w:docPartBody`).
    pub blocks: Blocks,
}

/// The gallery Quick Parts saves to (`w:gallery w:val="docParts"`).
pub const QUICK_PARTS_GALLERY: &str = "docParts";
/// The AutoText gallery (`w:gallery w:val="autoTxt"`).
pub const AUTO_TEXT_GALLERY: &str = "autoTxt";

/// A bibliography source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Source {
    /// Unique tag cited by CITATION fields.
    pub tag: String,
    /// book, bookSection, article (journal), periodical, conference, report, website, film, other
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
    /// More bibliography fields (Create Source › Show All Bibliography Fields).
    pub editor: String,
    pub edition: String,
    pub issue: String,
    pub month: String,
    pub day: String,
    /// Date accessed (websites).
    pub accessed: String,
    pub doi: String,
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
    /// Record a tracked cut and paste of a sentence or paragraph as a move (Track moves); off
    /// when the file has `w:doNotTrackMoves` (ECMA-376 §17.15.1.29), and then it is an
    /// ordinary deletion and insertion.
    pub track_moves: bool,
    /// Default tab stop interval, points.
    pub default_tab: f32,
    pub even_odd_headers: bool,
    pub mirror_margins: bool,
    pub auto_hyphenation: bool,
    /// Hyphenation zone (`w:hyphenationZone`), points: a line is hyphenated only when breaking
    /// it at a word boundary would leave more than this empty at its end.
    pub hyphenation_zone: f32,
    /// Words in capitals may be hyphenated (`w:doNotHyphenateCaps` turns it off).
    pub hyphenate_caps: bool,
    /// Most consecutive lines that may end with a hyphen (`w:consecutiveHyphenLimit`); 0 = no limit.
    pub consecutive_hyphen_limit: u32,
    pub page_color: Option<Rgb>,
    /// Picture or texture page background (Design › Page Color › Fill Effects), drawn over the colour.
    pub page_picture: Option<PagePicture>,
    pub watermark: Option<Watermark>,
    /// Theme fonts (major = headings, minor = body).
    pub major_font: String,
    pub minor_font: String,
    /// Theme colours: dk1 lt1 dk2 lt2 accent1..6 hlink folHlink.
    pub theme_colors: Vec<Rgb>,
    pub theme_name: String,
    pub footnote_format: section::NumFormat,
    pub endnote_format: section::NumFormat,
    /// The document's footnote and endnote placement, start and restart (`w:footnotePr` /
    /// `w:endnotePr` in the settings); sections may override them.
    #[serde(skip_serializing_if = "section::NoteProps::is_empty")]
    pub footnote_pr: section::NoteProps,
    #[serde(skip_serializing_if = "section::NoteProps::is_empty")]
    pub endnote_pr: section::NoteProps,
    /// Restrict Editing, when enforced: `readOnly`, `comments`, `trackedChanges`, `forms`, or
    /// `none` when only formatting is restricted (`w:documentProtection w:edit`).
    pub protection: Option<String>,
    /// Restrict Editing limits formatting to the styles that aren't locked, and forbids direct
    /// formatting (`w:formatting="1"`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub protect_formatting: bool,
    /// The protection password's hash, as the file's `w:documentProtection` attributes
    /// (`w:algorithmName`, `w:hashValue`, `w:saltValue`, `w:spinCount`, or the older
    /// `w:cryptAlgorithmSid`, `w:hash`, `w:salt`…). Empty: no password.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub protect_hash: Vec<(String, String)>,
    /// Word compatibility mode the document is laid out in (`compatibilityMode`): 15 for Word
    /// 2013 and later, which places a table's border at the margin rather than its text and lets
    /// justified lines shrink their spaces to fit more text.
    pub compat_mode: u32,
    /// Fonts the document's font table classes as roman (serif, `w:family w:val="roman"`): Word
    /// draws one that isn't installed in a serif face.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roman_fonts: Vec<String>,
    /// Alternate names the document's font table gives for its fonts (`w:font/w:altName`,
    /// ECMA-376 §17.8.3.1), requested name → alternate: Word draws a font that isn't installed
    /// in its alternate when that is available.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alt_fonts: Vec<(String, String)>,
    /// `w:doNotUseHTMLParagraphAutoSpacing`: the space between two paragraphs is the first's space
    /// after plus the second's space before, not the larger of the two.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sum_paragraph_spacing: bool,
    /// Keep contextual paragraph spacing inside legacy table cells (`allowSpaceOfSameStyleInTable`).
    /// Word ignores the layout override in compatibility mode 15, but the flag still round-trips.
    pub allow_space_of_same_style_in_table: bool,
    /// Drawing grid spacing in points, across and down (View › Gridlines); ECMA-376's default is 1/8 inch.
    pub grid_h: f32,
    pub grid_v: f32,
    /// Equation options (`m:mathPr`), when the document has them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub math: Option<math::MathProps>,
    /// Citation and bibliography style (References › Style) by name, when the document
    /// records one (OOXML's `b:Sources/@StyleName`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bib_style: Option<String>,
    /// The template the document is attached to (Tools › Templates and Add-ins): the target of
    /// `w:attachedTemplate`'s relationship, a path or `file:` URL as the file gives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached_template: Option<String>,
    /// Update the document's styles from the attached template when it is opened (`w:linkStyles`).
    pub link_styles: bool,
    /// The zoom the document was saved at (`w:zoom`), restored when it is opened; `None` for 100%
    /// without a fit mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom: Option<Zoom>,
    /// Remove personal information when saving (`w:removePersonalInformation`, ECMA-376
    /// §17.15.1.70): the saved file names comment and revision authors "Author" and leaves out
    /// the document's creator and last-modified-by. The open document keeps its real values.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remove_personal_info: bool,
    /// Remove date and time from annotations when saving (`w:removeDateAndTime`, §17.15.1.69):
    /// comments and tracked changes are saved without `w:date`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remove_date_time: bool,
}

/// A saved zoom: a percentage, or a fit mode (the percentage it last came to).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Zoom {
    /// 10–500.
    pub percent: u32,
    pub fit: Option<ZoomFit>,
}

/// View › Zoom's fit modes a document can be saved in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ZoomFit {
    OnePage,
    PageWidth,
    TextWidth,
}

/// Word's compatibility mode for documents that don't state one.
pub const LEGACY_COMPAT_MODE: u32 = 12;

/// The compatibility mode of documents created by Word 2013 and later, and by WordCraft.
pub const COMPAT_MODE_CURRENT: u32 = 15;

/// Word's hyphenation zone when a document doesn't set one (0.25"), points.
pub const DEFAULT_HYPHENATION_ZONE: f32 = 18.0;

/// ECMA-376 §17.15.1.44/45: 180 twentieths of a point when a document doesn't say.
pub const DEFAULT_GRID: f32 = 9.0;

/// A picture fill for the page background (`v:background/v:fill` with an image).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PagePicture {
    /// Key into [`Document::media`].
    pub media: String,
    /// `v:fill/@type` (`tile`, `frame`, `tile`...); tiled when `tile`, stretched to the page otherwise.
    pub fill_type: String,
    /// `o:title` of the fill (texture name), kept for round trip.
    pub title: String,
}

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
            track_moves: true,
            default_tab: 36.0,
            even_odd_headers: false,
            mirror_margins: false,
            auto_hyphenation: false,
            hyphenation_zone: DEFAULT_HYPHENATION_ZONE,
            hyphenate_caps: true,
            consecutive_hyphen_limit: 0,
            page_color: None,
            page_picture: None,
            watermark: None,
            major_font: styles::HEADING_FONT.into(),
            minor_font: styles::BODY_FONT.into(),
            theme_colors: THEME_COLORS.to_vec(),
            theme_name: "Craft".into(),
            footnote_format: section::NumFormat::Decimal,
            endnote_format: section::NumFormat::LowerRoman,
            footnote_pr: section::NoteProps::default(),
            endnote_pr: section::NoteProps::default(),
            protection: None,
            protect_formatting: false,
            protect_hash: Vec::new(),
            compat_mode: COMPAT_MODE_CURRENT,
            roman_fonts: Vec::new(),
            alt_fonts: Vec::new(),
            sum_paragraph_spacing: false,
            allow_space_of_same_style_in_table: false,
            grid_h: DEFAULT_GRID,
            grid_v: DEFAULT_GRID,
            math: None,
            bib_style: None,
            attached_template: None,
            link_styles: false,
            zoom: None,
            remove_personal_info: false,
            remove_date_time: false,
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
    /// The document's own footnote separators and continuation notice.
    #[serde(skip_serializing_if = "NoteSeparators::is_empty")]
    pub footnote_separators: NoteSeparators,
    /// The document's own endnote separators and continuation notice.
    #[serde(skip_serializing_if = "NoteSeparators::is_empty")]
    pub endnote_separators: NoteSeparators,
    /// Embedded media (images) by key. Native JSON writes each as base64 (`byte_store.rs`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", with = "byte_store")]
    pub media: BTreeMap<String, Arc<Vec<u8>>>,
    /// Source-format parts we keep verbatim for round-trip (e.g. custom XML), by part name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", with = "byte_store")]
    pub passthrough: BTreeMap<String, Arc<Vec<u8>>>,
    /// The document's own building blocks (its glossary document), in file order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<BuildingBlock>,
    /// Lookup cache for [`Document::add_media`]; not part of the document (never saved, ignored
    /// by `==`).
    #[serde(skip)]
    media_index: MediaIndex,
}

#[cfg(test)]
mod test_log;
#[cfg(test)]
mod tests;
