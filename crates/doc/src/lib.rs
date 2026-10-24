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

/// A free id, `min` or more, for something new (a part, a list, a comment): one past `largest`, the
/// largest id in use, so ids keep growing; once that would pass `u32::MAX` (a file can use any id),
/// the lowest one `used` (every id in use) doesn't hold. `None` when every id from `min` up is taken.
pub fn free_id(largest: Option<u32>, used: impl Iterator<Item = u32>, min: u32) -> Option<u32> {
    let Some(largest) = largest else { return Some(min) };
    if let Some(next) = largest.checked_add(1) {
        return Some(next.max(min));
    }
    let mut ids: Vec<u32> = used.filter(|&i| i >= min).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut want = min;
    for i in ids {
        if i > want {
            break;
        }
        want = want.checked_add(1)?;
    }
    Some(want)
}

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
        let s: Vec<String> = self.0.iter().map(ToString::to_string).collect();
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
    /// Two pages share one sheet, its long side halved (`w:printTwoOnOne`).
    pub two_per_sheet: bool,
    /// Pages are printed as a booklet, folded in the middle of the sheet (`w:bookFoldPrinting`).
    pub book_fold: bool,
    /// Book fold printed right to left (`w:bookFoldRevPrinting`).
    pub book_fold_reverse: bool,
    /// Sheets per booklet (`w:bookFoldPrintingSheets`); 0 = all in one booklet.
    pub book_fold_sheets: u16,
    /// Page Setup's Gutter position is Top (`w:gutterAtTop`). Only kept for Normal pages: the
    /// other Multiple pages modes fix the gutter's position (and 2 pages per sheet on a portrait
    /// sheet writes it, as Word does). Not laid out yet.
    pub gutter_at_top: bool,
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
    /// The shape's box (its `style` width and height, points), which the text is stretched to
    /// fill (`v:textpath fitshape`); `None`: the box Word's Design › Watermark gives
    /// ([`Watermark::frame`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 2]>,
}

impl Default for Watermark {
    fn default() -> Self {
        Watermark {
            text: "DRAFT".into(),
            font: styles::BODY_FONT.into(),
            color: Rgb(0xC0, 0xC0, 0xC0),
            diagonal: true,
            semitransparent: true,
            size: None,
        }
    }
}

impl Watermark {
    /// The box the text fills on a page `page_w` × `page_h` (points): its own size, else half the
    /// page diagonal across (half the page width when not diagonal), as high as the text's length
    /// makes it. Centred on the margins and, when diagonal, turned 45° counterclockwise (Word's
    /// `rotation:315`).
    pub fn frame(&self, page_w: f32, page_h: f32) -> [f32; 2] {
        if let Some([w, h]) = self.size.filter(|[w, h]| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0) {
            return [w.min(4000.0), h.min(4000.0)];
        }
        let (pw, ph) = (wordcraft_geom::finite(page_w).clamp(72.0, 4000.0), wordcraft_geom::finite(page_h).clamp(72.0, 4000.0));
        let across = if self.diagonal { (pw * pw + ph * ph).sqrt() } else { pw };
        let width = (across * 0.5).clamp(36.0, 2000.0);
        let chars = self.text.chars().take(200).count().max(1) as f32;
        [width, (width / (0.55 * chars)).clamp(12.0, 400.0)]
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

/// Most sheets per booklet Word offers (`w:bookFoldPrintingSheets`).
pub const MAX_BOOKLET_SHEETS: u16 = 40;

impl Settings {
    /// Page Setup's "Multiple pages" choice these settings amount to (book fold wins over two
    /// per sheet, which wins over mirrored margins).
    pub fn multiple_pages(&self) -> section::MultiplePages {
        use section::MultiplePages as M;
        if self.book_fold {
            M::BookFold
        } else if self.two_per_sheet {
            M::TwoPerSheet
        } else if self.mirror_margins {
            M::Mirror
        } else {
            M::Normal
        }
    }

    /// Choose the "Multiple pages" mode; the booklet's sheet count and direction are kept.
    pub fn set_multiple_pages(&mut self, mode: section::MultiplePages) {
        use section::MultiplePages as M;
        self.mirror_margins = mode == M::Mirror;
        self.two_per_sheet = mode == M::TwoPerSheet;
        self.book_fold = mode == M::BookFold;
    }

    /// Margins mirror on facing pages (inside/outside): mirrored margins, or a booklet.
    pub fn mirrors(&self) -> bool {
        self.mirror_margins || self.book_fold
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            track_changes: false,
            track_moves: true,
            default_tab: 36.0,
            even_odd_headers: false,
            mirror_margins: false,
            two_per_sheet: false,
            book_fold: false,
            book_fold_reverse: false,
            book_fold_sheets: 0,
            gutter_at_top: false,
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
                for off in p.object_offsets() {
                    if p.object_at(off).is_some_and(|o| o.text_boxes().contains(&part)) {
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

    /// [`Document::sections`] as laid out: under 2 pages per sheet or Book fold each section's
    /// paper becomes the half-sheet page (see [`SectionProps::for_multiple_pages`]). Use this to
    /// size content to the page (#1836).
    pub fn laid_out_sections(&self) -> Vec<(usize, SectionProps)> {
        let mode = self.settings.multiple_pages();
        self.sections().into_iter().map(|(e, s)| (e, s.for_multiple_pages(mode))).collect()
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

    /// Ranges that stay editable while the document is protected (Restrict Editing exceptions),
    /// in document order across every story: the [`InlineObject::PermStart`]'s id, group and
    /// editor, its start (just after the marker) and its end (at the matching
    /// [`InlineObject::PermEnd`], or the end of the story when it has none).
    pub fn perm_ranges(&self) -> Vec<PermRange> {
        let mut out: Vec<PermRange> = Vec::new();
        let stories = std::iter::once(StoryRef::Body).chain(self.parts.keys().map(|k| StoryRef::Part(*k)));
        for story in stories {
            let mut open: Vec<usize> = Vec::new();
            for path in self.para_paths(story) {
                let Some(p) = self.para(story, &path) else { continue };
                for off in p.object_offsets() {
                    match p.object_at(off) {
                        Some(InlineObject::PermStart { id, group, editor }) => {
                            let start = Pos { story, path: path.clone(), off: off + para::OBJ.len_utf8() };
                            open.push(out.len());
                            out.push(PermRange { id: *id, group: group.clone(), editor: editor.clone(), start: start.clone(), end: start });
                        }
                        Some(InlineObject::PermEnd { id }) => {
                            if let Some(k) = open.iter().rposition(|i| out.get(*i).is_some_and(|r| r.id == *id)) {
                                let i = open.remove(k);
                                if let Some(r) = out.get_mut(i) {
                                    r.end = Pos { story, path: path.clone(), off };
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // A range left open runs to the end of its story.
            let end = self.end_of(story);
            for i in open {
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

/// A range that stays editable while the document is protected; see [`Document::perm_ranges`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermRange {
    pub id: u32,
    /// Editing group (`everyone`, `editors`…); empty when the range names an editor.
    pub group: String,
    /// The one editor the range is for; empty for a group.
    pub editor: String,
    pub start: Pos,
    pub end: Pos,
}

impl PermRange {
    /// Can everyone edit it (rather than one person or group)?
    pub fn everyone(&self) -> bool {
        self.group.eq_ignore_ascii_case("everyone")
    }
}

/// Words in `s` (tokens separated by whitespace that contain at least one alphanumeric char).
pub fn count_words(s: &str) -> usize {
    s.split(|c: char| c.is_whitespace() || c == para::OBJ).filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

/// How deep in nested tables [`Document::para_paths`] goes: cells below this are skipped.
pub(crate) const PARA_WALK_DEPTH: usize = 16;

/// The note and text box stories an object shows, for `Document::reachable`.
fn notes_and_boxes(o: &InlineObject) -> Vec<(u32, &'static [PartKind])> {
    match o {
        InlineObject::NoteRef { id, .. } => vec![(*id, &[PartKind::Footnote, PartKind::Endnote][..])],
        o => o.text_boxes().into_iter().map(|id| (id, &[PartKind::TextBox][..])).collect(),
    }
}

fn walk(bl: &Blocks, prefix: &mut Vec<u32>, out: &mut Vec<Path>, depth: usize) {
    if depth > PARA_WALK_DEPTH {
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

/// [`walk`] for the paragraphs from path `a` to `b` inclusive, skipping the blocks outside.
pub(crate) fn walk_between(bl: &Blocks, prefix: &mut Vec<u32>, out: &mut Vec<Path>, depth: usize, (a, b): (&[u32], &[u32])) {
    use std::cmp::Ordering::{Greater, Less};
    if depth > PARA_WALK_DEPTH {
        return;
    }
    // Where the paths starting with `q` lie against `x`: all before, all after, or either.
    let side = |q: &[u32], x: &[u32]| {
        let m = q.len().min(x.len());
        q[..m].cmp(&x[..m])
    };
    for (i, blk) in bl.iter().enumerate() {
        prefix.push(i as u32);
        if side(prefix, b) == Greater {
            prefix.pop();
            break;
        }
        if side(prefix, a) != Less {
            match &**blk {
                Block::Para(_) => {
                    if prefix.as_slice() >= a && prefix.as_slice() <= b {
                        out.push(Path(prefix.clone()));
                    }
                }
                Block::Table(t) => {
                    for (r, row) in t.rows.iter().enumerate() {
                        for (c, cell) in row.cells.iter().enumerate() {
                            prefix.push(r as u32);
                            prefix.push(c as u32);
                            if side(prefix, a) != Less && side(prefix, b) != Greater {
                                walk_between(&cell.blocks, prefix, out, depth + 1, (a, b));
                            }
                            prefix.pop();
                            prefix.pop();
                        }
                    }
                }
            }
        }
        prefix.pop();
    }
}

/// Which text of a paragraph [`text_of`] collects.
#[derive(Clone, Copy)]
enum TextMode {
    /// Every run, tracked deletions included ([`Paragraph::plain_text`]).
    Plain,
    /// As shown on screen ([`Paragraph::shown_text`]).
    Shown,
    /// Without tracked deletions ([`Paragraph::final_text`]).
    Final,
}

fn text_of(bl: &Blocks, out: &mut String, depth: usize, mode: TextMode) {
    if depth > 16 {
        return;
    }
    for b in bl {
        match &**b {
            Block::Para(p) => {
                out.push_str(&match mode {
                    TextMode::Plain => p.plain_text(),
                    TextMode::Shown => p.shown_text(),
                    TextMode::Final => p.final_text(),
                });
                out.push('\n');
            }
            Block::Table(t) => {
                for row in &t.rows {
                    let mut cells = Vec::new();
                    for c in &row.cells {
                        let mut s = String::new();
                        text_of(&c.blocks, &mut s, depth + 1, mode);
                        cells.push(s.trim_end_matches('\n').replace('\n', " "));
                    }
                    out.push_str(&cells.join("\t"));
                    out.push('\n');
                }
            }
        }
    }
}

fn story_word_count_of(bl: &Blocks, numbering: &Numbering, styles: &styles::StyleSheet, counters: &mut numbering::Counters, depth: usize) -> usize {
    if depth > 16 {
        return 0;
    }
    let mut total = 0usize;
    for b in bl {
        match &**b {
            Block::Para(p) => {
                let n = p.props.numbering.or_else(|| styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0);
                if let Some(n) = n
                    && let Some((label, def)) = counters.next_label(numbering, n.num, n.level)
                    && !matches!(def.format, section::NumFormat::Bullet | section::NumFormat::None)
                {
                    total = total.saturating_add(count_words(&label));
                }
                total = total.saturating_add(count_words(&p.plain_text()));
            }
            Block::Table(t) => {
                for row in &t.rows {
                    for c in &row.cells {
                        total = total.saturating_add(story_word_count_of(&c.blocks, numbering, styles, counters, depth + 1));
                    }
                }
            }
        }
    }
    total
}

#[cfg(test)]
mod tests;
