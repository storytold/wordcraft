//! WordCraft layout: turns a [`Document`] into pages.
//!
//! - [`para`]: one paragraph → lines (shaping, first-fit line breaking, tabs, list labels,
//!   alignment). Results are memoised per paragraph revision in a [`LayoutCache`].
//! - [`layout`]: sections → pages and columns; tables; keep/widow rules; page breaks; headers
//!   and footers; page-number fields.
//! - [`display`]: a page → draw items (glyph runs, rules, fills, images) for the renderers.
//! - [`hit`]: point ↔ position, caret geometry, line navigation, selection rectangles.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod balloons;
mod contour;
pub mod display;
pub mod fields;
pub mod hit;
pub mod kinsoku;
pub mod math;
pub mod outline;
pub mod para;
pub mod styleref;
mod table;
mod textbox;
mod vertical;
pub mod wordart;

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use wordcraft_doc::numbering::{Counters, Level};
use wordcraft_doc::para::{Anchor, Float, FloatAlign, InlineObject, Wrap, WrapText};
use wordcraft_doc::props::{Border, CharProps, HeightRule, Rgb, TableFloat, TextDirection};
use wordcraft_doc::section::{HeaderSet, SectionProps, SectionStart};
use wordcraft_doc::{Block, Blocks, Document, Paragraph, Path, StoryRef};
use wordcraft_geom::{Point, Rect, Spin};

pub use fields::FieldCtx;
pub use hit::VisualStep;
pub use para::{LineEnd, ParaLayout};
pub use table::{autofit_widths, cell_margins, measure_table_columns};

/// How the document is viewed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewMode {
    #[default]
    Print,
    /// One page as wide as the window, no page breaks.
    Web,
    /// Draft: like web, page breaks shown as rules.
    Draft,
    /// Reading: reflow into viewport-height narrow pages, navigated horizontally.
    Read,
    /// Outline: like draft, the body as an outline of its headings ([`outline`]).
    Outline,
}

/// How digits look on screen (Word › File › Options › Advanced › Numeral).
///
/// Display only: the document text keeps the typed codepoints, so find, spelling, copying and
/// the saved file all see the original digits. Extended Arabic-Indic digits (U+06F0–U+06F9,
/// Persian/Urdu ۰۱۲۳…) always show as typed; only ASCII and Arabic-Indic (U+0660–U+0669) digits
/// are substituted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NumeralMode {
    /// Western digits everywhere (`0123456789`); the default, today's behaviour.
    #[default]
    Arabic,
    /// Eastern Arabic-Indic digits everywhere (`٠١٢٣٤٥٦٧٨٩`).
    Hindi,
    /// Hindi digits in and around right-to-left text, Western digits elsewhere (numbers take
    /// an even bidi level, so the surrounding text decides).
    Context,
    /// Hindi digits when the system locale is an Arabic-script language, else Western digits.
    /// Call [`NumeralMode::resolve`] before laying out.
    System,
}

impl NumeralMode {
    /// Short UI label (Word's option names).
    pub fn label(self) -> &'static str {
        match self {
            NumeralMode::Arabic => "Arabic",
            NumeralMode::Hindi => "Hindi",
            NumeralMode::Context => "Context",
            NumeralMode::System => "System",
        }
    }

    /// Parse a command value (`"arabic" | "hindi" | "context" | "system"`, any case).
    pub fn parse(s: &str) -> Option<NumeralMode> {
        match s.to_lowercase().as_str() {
            "arabic" => Some(NumeralMode::Arabic),
            "hindi" => Some(NumeralMode::Hindi),
            "context" => Some(NumeralMode::Context),
            "system" => Some(NumeralMode::System),
            _ => None,
        }
    }

    /// Resolve `System` against the system locale (a BCP 47 or POSIX tag such as `ar-EG` or
    /// `fa_IR.UTF-8`, found by the shell: layout never reads the environment). Hindi digits for
    /// Arabic-script languages, else Western; unknown locale = Western. Other modes pass through.
    pub fn resolve(self, system_locale: Option<&str>) -> NumeralMode {
        if self != NumeralMode::System {
            return self;
        }
        if system_locale.is_some_and(hindi_locale) { NumeralMode::Hindi } else { NumeralMode::Arabic }
    }

    /// The displayed form of `c` in a run shaped with direction `rtl`.
    ///
    /// One character maps to exactly one character, so shaped text stays codepoint-aligned
    /// with the document text (only byte offsets shift). A `System` mode that was not
    /// resolved falls back to Western digits.
    pub fn map(self, c: char, rtl: bool) -> char {
        let hindi = match self {
            NumeralMode::Arabic => false,
            NumeralMode::Hindi => true,
            NumeralMode::Context => rtl,
            NumeralMode::System => false,
        };
        if hindi {
            if c.is_ascii_digit() { char::from_u32(0x660 + (c as u32 - '0' as u32)).unwrap_or(c) } else { c }
        } else if ('\u{660}'..='\u{669}').contains(&c) {
            (b'0' + (c as u32 - 0x660) as u8) as char
        } else {
            c
        }
    }
}

/// Does locale `tag` use Hindi digits (Arabic, Persian, Urdu, Pashto, Sindhi, Kurdish)?
fn hindi_locale(tag: &str) -> bool {
    let lang = tag.split(['_', '-', '.', '@']).next().unwrap_or_default().to_ascii_lowercase();
    matches!(lang.as_str(), "ar" | "fa" | "ur" | "ps" | "sd" | "ku" | "ckb")
}

#[derive(Clone, Debug, Default)]
pub struct LayoutOptions {
    pub view: ViewMode,
    /// Width of the window in Web/Draft views, points.
    pub web_width: f32,
    /// Page height in Reading Mode (document points).
    pub read_height: f32,
    /// Transient read-only line-spacing override; ignored by print/export views.
    pub read_line_spacing: Option<f32>,
    pub show_hidden: bool,
    /// Final text ("No Markup"): tracked deletions take no space and draw nothing.
    pub hide_deleted: bool,
    /// Tracked deletions that take no space and draw nothing although markup is shown: those
    /// Track Changes Options mark Hidden (deletions, or moved-away text). `hide_deleted` and
    /// `deletions_in_balloons` leave every deletion out whatever this says.
    pub hide_marked: HideDeleted,
    /// Show revisions in balloons: tracked deletions in the body take no space and draw nothing
    /// (the editor shows them in balloons beside the page). Headers, footers, notes and text
    /// boxes get no balloons, so their deletions stay inline.
    pub deletions_in_balloons: bool,
    /// Check spelling as you type (squiggles).
    pub proofing: bool,
    /// How digits look on screen (display only; the text is untouched). Callers resolve
    /// `System` ([`NumeralMode::resolve`]); an unresolved `System` shows Western digits.
    pub numeral: NumeralMode,
    /// Print Markup (PDF, print, page images): widen each page by [`balloons::MARKUP_AREA`]
    /// and show the comments there in balloons joined to their anchors, as Word prints them.
    /// The editor draws its own interactive balloons and leaves this off.
    pub comment_balloons: bool,
    /// Show field codes (`{ PAGE }`, shaded) instead of field results (View › Field Codes, Alt+F9).
    /// A field whose own `code` flag is set shows the other one.
    pub field_codes: bool,
    /// Outline view: what's collapsed and shown (used when `view` is [`ViewMode::Outline`]).
    pub outline: Option<outline::OutlineView>,
    /// What the spelling check skips, and whether grammar is marked too (File › Options › Proofing).
    pub proof: ProofOptions,
    /// The word being typed, which gets no spelling squiggle until the caret leaves it (#1552).
    pub typing_word: Option<TypingWord>,
}

/// The word being typed ([`LayoutOptions::typing_word`]): the paragraph by its revision
/// ([`Paragraph::rev`]) and the word's byte range in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypingWord {
    pub rev: u64,
    pub start: usize,
    pub end: usize,
}

/// File › Options › Proofing choices that change the squiggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProofOptions {
    /// Mark grammar errors as you type.
    pub grammar: bool,
    pub spelling: wordcraft_proof::Options,
}

impl Default for ProofOptions {
    fn default() -> Self {
        ProofOptions { grammar: true, spelling: wordcraft_proof::Options::default() }
    }
}

impl LayoutOptions {
    /// Which tracked deletions in `story` take no space and draw nothing.
    pub fn hides_deleted(&self, story: StoryRef) -> HideDeleted {
        if self.hide_deleted || (self.deletions_in_balloons && story == StoryRef::Body) { HideDeleted::ALL } else { self.hide_marked }
    }
}

/// Which tracked deletions are left out of the layout: ordinary ones, and the text a tracked
/// move took away (the source of a move, `w:moveFrom`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HideDeleted {
    pub plain: bool,
    pub moved_from: bool,
}

impl HideDeleted {
    pub const ALL: HideDeleted = HideDeleted { plain: true, moved_from: true };
    /// Whether any deletion is left out.
    pub fn any(self) -> bool {
        self.plain || self.moved_from
    }
    /// Whether text deleted by revision `del` (`None`: not deleted) is left out.
    pub fn hides(self, doc: &Document, del: Option<u32>) -> bool {
        del.is_some() && if doc.move_name(del).is_some() { self.moved_from } else { self.plain }
    }
}

/// Something placed on a page (page coordinates, points, y down).
#[derive(Clone, Debug)]
pub enum Placed {
    /// Lines `l0..l1` of a paragraph; `y` is the top of line `l0`; `x` the column's left edge.
    /// Turned lines (a table cell's text direction) run as `turn` says from the page point
    /// (`x`, `y`): see [`turn_point`].
    Lines {
        story: StoryRef,
        path: Path,
        para: Arc<ParaLayout>,
        l0: usize,
        l1: usize,
        x: f32,
        y: f32,
        turn: TextDirection,
    },
    Fill {
        rect: Rect,
        color: Rgb,
    },
    Rule {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        border: Border,
    },
    /// A picture in `rect`, turned about the rect's centre by `spin` (as are shapes and charts).
    Image {
        rect: Rect,
        media: String,
        crop: [f32; 4],
        story: StoryRef,
        path: Path,
        off: usize,
        spin: Spin,
    },
    Shape {
        rect: Rect,
        kind: wordcraft_doc::para::ShapeKind,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
        stroke_width: f32,
        /// A freeform's paths (ink strokes among them).
        freeform: Option<Arc<wordcraft_doc::freeform::Freeform>>,
        effects: wordcraft_doc::effects::ShapeEffects,
        spin: Spin,
        /// Arrowheads and text warp (see [`wordcraft_doc::connector::ShapeExtra`]).
        extra: wordcraft_doc::connector::ShapeExtra,
        /// The floating object it draws (a Drawing Canvas's members: the canvas's), for leaving
        /// exactly one object out of a page (see [`crate::display::DisplayOptions::hide_object`]).
        object: Option<wordcraft_doc::Pos>,
    },
    /// A floating chart or diagram, drawn from its items inside `rect`. The object is the U+FFFC at
    /// byte `off` of paragraph `path` (its alt text).
    Graphic {
        rect: Rect,
        graphic: Arc<wordcraft_doc::graphic::Graphic>,
        story: StoryRef,
        path: Path,
        off: usize,
        spin: Spin,
    },
    /// `items` clipped to `rect`: a floating Drawing Canvas's members, kept inside its frame.
    /// `rect` is the unturned frame and `items` sit in it; `spin` turns the clipped whole about
    /// the frame's centre.
    Clip {
        rect: Rect,
        spin: Spin,
        items: Vec<Placed>,
    },
    /// A table cell's area (for hit testing and cell selection). A vertically merged cell is
    /// one `Cell` as tall as all its rows; its continuation rows get a `covered` `Cell` (their
    /// own row's height) so the rows keep a box each, but it isn't a cell of its own on screen.
    Cell {
        rect: Rect,
        covered: bool,
        table: Path,
        row: usize,
        cell: usize,
        story: StoryRef,
        /// Its row is exactly as tall as it says and the content is taller: what runs past the
        /// cell is cut off when drawn, as in Word.
        clip: bool,
    },
    /// The area of a picture, shape or text box, for hit testing, selection handles and dragging
    /// (drawn by `Image`/`Shape`/`Lines`). The object is the U+FFFC at byte `off` of paragraph
    /// `path` in `story`. `rect` is its unrotated frame; `spin` turns it about the centre.
    Object {
        rect: Rect,
        spin: Spin,
        story: StoryRef,
        path: Path,
        off: usize,
        /// The text box story it shows (`Document::parts` id).
        text_box: Option<u32>,
        wrap: Wrap,
        /// Its paragraph's column left and top: where offsets relative to the column and the
        /// paragraph (`Anchor::Column` / `Anchor::Paragraph`) start.
        origin: wordcraft_geom::Point,
    },
}

impl Placed {
    fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Placed::Lines { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Placed::Fill { rect, .. }
            | Placed::Image { rect, .. }
            | Placed::Shape { rect, .. }
            | Placed::Graphic { rect, .. }
            | Placed::Cell { rect, .. } => {
                rect.x += dx;
                rect.y += dy;
            }
            Placed::Object { rect, origin, .. } => {
                rect.x += dx;
                rect.y += dy;
                origin.x += dx;
                origin.y += dy;
            }
            Placed::Clip { rect, items, .. } => {
                rect.x += dx;
                rect.y += dy;
                for it in items {
                    it.translate(dx, dy);
                }
            }
            Placed::Rule { x0, y0, x1, y1, .. } => {
                *x0 += dx;
                *x1 += dx;
                *y0 += dy;
                *y1 += dy;
            }
        }
    }

    /// The page area of turned lines (`None` for anything else).
    pub fn turned_bounds(&self) -> Option<Rect> {
        let Placed::Lines { para, l0, l1, x, y, turn, .. } = self else { return None };
        if !turn.is_turned() {
            return None;
        }
        let first = para.lines.get(*l0)?;
        let last = para.lines.get(l1.checked_sub(1)?)?;
        let len = para.lines.get(*l0..*l1).unwrap_or(&[]).iter().map(|l| l.right).fold(0.0f32, f32::max);
        Some(turn_rect(*turn, *x, *y, Rect::new(0.0, 0.0, len, last.top + last.height - first.top)))
    }

    /// Move an item laid out in a turned frame (a table cell's text running `turn`) onto the
    /// page, the frame's origin landing on page point (`x`, `y`).
    fn turn(&mut self, turn: TextDirection, x: f32, y: f32) {
        if !turn.is_turned() {
            self.translate(x, y);
            return;
        }
        match self {
            Placed::Lines { x: lx, y: ly, turn: t, .. } => {
                (*lx, *ly) = turn_point(turn, x, y, *lx, *ly);
                // Turned text inside turned text keeps its own direction (no 180° text).
                if !t.is_turned() {
                    *t = turn;
                }
            }
            Placed::Fill { rect, .. }
            | Placed::Image { rect, .. }
            | Placed::Shape { rect, .. }
            | Placed::Graphic { rect, .. }
            | Placed::Cell { rect, .. } => {
                *rect = turn_rect(turn, x, y, *rect);
            }
            Placed::Object { rect, origin, .. } => {
                *rect = turn_rect(turn, x, y, *rect);
                (origin.x, origin.y) = turn_point(turn, x, y, origin.x, origin.y);
            }
            Placed::Clip { rect, items, .. } => {
                *rect = turn_rect(turn, x, y, *rect);
                for it in items {
                    it.turn(turn, x, y);
                }
            }
            Placed::Rule { x0, y0, x1, y1, .. } => {
                (*x0, *y0) = turn_point(turn, x, y, *x0, *y0);
                (*x1, *y1) = turn_point(turn, x, y, *x1, *y1);
            }
        }
    }
}

/// Where point (`u`, `v`) of a frame turned `turn` lands on the page, the frame's origin being
/// page point (`x`, `y`): `u` runs along the lines, `v` across them (down for horizontal text).
pub fn turn_point(turn: TextDirection, x: f32, y: f32, u: f32, v: f32) -> (f32, f32) {
    match turn {
        TextDirection::Horizontal => (x + u, y + v),
        TextDirection::Down => (x - v, y + u),
        TextDirection::Up => (x + v, y - u),
    }
}

/// The inverse of [`turn_point`]: the frame point at page point (`px`, `py`).
pub fn unturn_point(turn: TextDirection, x: f32, y: f32, px: f32, py: f32) -> (f32, f32) {
    match turn {
        TextDirection::Horizontal => (px - x, py - y),
        TextDirection::Down => (py - y, x - px),
        TextDirection::Up => (y - py, px - x),
    }
}

/// The page rectangle a frame rectangle covers (see [`turn_point`]).
pub fn turn_rect(turn: TextDirection, x: f32, y: f32, r: Rect) -> Rect {
    match turn {
        TextDirection::Horizontal => Rect::new(x + r.x, y + r.y, r.w, r.h),
        TextDirection::Down => Rect::new(x - r.y - r.h, y + r.x, r.h, r.w),
        TextDirection::Up => Rect::new(x + r.y, y - r.x - r.w, r.h, r.w),
    }
}

/// Which header/footer of its section a page shows ([`Page::hf_slot`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HfSlot {
    #[default]
    Default,
    /// The first page of a section with "Different First Page".
    First,
    /// An even page with "Different Odd & Even Pages".
    Even,
}

/// One laid-out page.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub w: f32,
    pub h: f32,
    /// Index into `Document::sections()`.
    pub section: usize,
    /// Page number as displayed (after section restarts).
    pub number: u32,
    pub items: Vec<Placed>,
    /// Header / footer items (drawn dimmed while editing the body).
    pub header: Vec<Placed>,
    pub footer: Vec<Placed>,
    /// The body text area (margins).
    pub body: Rect,
    pub header_story: Option<u32>,
    pub footer_story: Option<u32>,
    /// Which of its section's headers/footers this page shows (first page, even page or the
    /// default), whether or not the section has one: double-clicking an empty header area
    /// creates that one (#960).
    pub hf_slot: HfSlot,
    /// Top-level body block index at the start of the page.
    pub first_block: usize,
    /// Width of the markup area on the right of `w` that holds comment balloons (Print Markup,
    /// [`LayoutOptions::comment_balloons`]); 0 without one. The document page is `w - markup` wide.
    pub markup: f32,
}

/// The whole layout.
#[derive(Clone, Debug, Default)]
pub struct DocLayout {
    pub pages: Vec<Page>,
    /// Index: (story, paragraph path) → (page, item index) of each placed piece, in order.
    pub index: HashMap<(StoryRef, Path), Vec<(usize, usize)>>,
    /// Layout time, milliseconds.
    pub ms: f64,
    /// Text box stories laid out (each time a box's text was).
    pub text_boxes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    rev: u64,
    width: u32,
    label: Option<String>,
    page: Option<(u32, u32, u32, u32, u64, wordcraft_doc::section::NumFormat)>,
    /// The pages a split paragraph's fields are on after its first (`FieldCtx::page_from`).
    page_from: u64,
    table: u64,
    notes: u64,
    excl: u64,
    eq: u32,
    read_spacing: Option<u32>,
    grid: (u32, u32, u32, bool),
    fit: bool,
    /// Heights of the paragraph's inline boxes that resize to fit their text.
    fitted: u64,
    hide_deleted: HideDeleted,
    /// STYLEREF results in headers and footers ([`FieldCtx::styleref_key`]).
    sref: u64,
    /// The word being typed in this paragraph, left without a spelling squiggle.
    typing: Option<(usize, usize)>,
}

/// Memoised paragraph layouts.
#[derive(Default)]
pub struct LayoutCache {
    paras: HashMap<Key, Arc<ParaLayout>>,
    /// Narrowest and widest content of paragraphs measured for table autofit ([`Ctx::para_measured`]).
    measures: HashMap<Key, (f32, f32)>,
    /// Outlines of pictures' opaque pixels (Tight and Through wrapping), by media key, the
    /// media's bytes (address and length) and crop.
    outlines: HashMap<(String, usize, usize, [u32; 4]), Option<Arc<Vec<(f32, f32)>>>>,
    env: u64,
    used: HashSet<Key>,
    /// The STYLEREF style index of the last pass and what it was built from: the body blocks
    /// (shared, so an edited block is a different pointer) and `env` (styles and numbering). Kept
    /// while both are unchanged, so headers using STYLEREF don't rebuild it on every pass.
    styleref: Option<(Blocks, u64, Arc<styleref::StyleIndex>)>,
    pub hits: u64,
    pub misses: u64,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.paras.len()
    }
    pub fn is_empty(&self) -> bool {
        self.paras.is_empty()
    }
    pub fn clear(&mut self) {
        self.paras.clear();
        self.measures.clear();
        self.outlines.clear();
        self.styleref = None;
    }

    /// The STYLEREF style index for `doc`, rebuilt only when the body or the styles changed.
    fn style_index(&mut self, doc: &Document) -> Arc<styleref::StyleIndex> {
        if let Some((body, env, ix)) = &self.styleref
            && *env == self.env
            && body.len() == doc.body.len()
            && body.iter().zip(doc.body.iter()).all(|(a, b)| Arc::ptr_eq(a, b))
        {
            return ix.clone();
        }
        let ix = Arc::new(styleref::StyleIndex::build(doc));
        self.styleref = Some((doc.body.to_vec(), self.env, ix.clone()));
        ix
    }
}

fn hash_of<T: Hash>(t: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn env_hash(doc: &Document, opts: &LayoutOptions) -> u64 {
    let s = serde_json::to_string(&(&doc.styles, &doc.numbering, doc.settings.default_tab, &doc.settings.footnote_format)).unwrap_or_default();
    // Outline view lays paragraphs out changed (no indents; plain without text formatting).
    let outline = (opts.view == ViewMode::Outline).then(|| opts.outline.as_ref().is_some_and(|o| o.plain));
    hash_of(&(
        s,
        opts.show_hidden,
        opts.hide_deleted,
        opts.hide_marked,
        opts.deletions_in_balloons,
        opts.proofing,
        opts.numeral,
        opts.field_codes,
        opts.proof,
        wordcraft_proof::dictionary_generation(),
        hyphenation_key(doc),
        outline,
    ))
}

/// The hyphenation settings, hashable (the zone as its bits).
fn hyphenation_key(doc: &Document) -> (bool, u32, bool, u32) {
    let s = &doc.settings;
    (s.auto_hyphenation, s.hyphenation_zone.to_bits(), s.hyphenate_caps, s.consecutive_hyphen_limit)
}

fn has_page_fields(p: &Paragraph) -> bool {
    p.objects.iter().any(|o| match o {
        InlineObject::Field { instr, .. } => {
            matches!(fields::field_name(instr).as_str(), "PAGE" | "NUMPAGES" | "SECTIONPAGES" | "SECTION" | "PAGEREF")
        }
        _ => false,
    })
}

/// Does `p` show a PAGE field, whose result is the number of the page it lands on?
fn has_page_number_fields(p: &Paragraph) -> bool {
    p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if fields::field_name(instr) == "PAGE"))
}

/// Does `p` show a page total, which depends on how many pages the document has?
fn has_total_fields(p: &Paragraph) -> bool {
    p.objects.iter().any(|o| match o {
        InlineObject::Field { instr, .. } => matches!(fields::field_name(instr).as_str(), "NUMPAGES" | "SECTIONPAGES"),
        _ => false,
    })
}

/// Page totals: of the document, and of each section (by index).
#[derive(Clone, Debug, PartialEq)]
struct Totals {
    pages: u32,
    sections: Vec<u32>,
}

impl Totals {
    /// What a first pass assumes: one page.
    fn first_guess(sections: usize) -> Totals {
        Totals { pages: 1, sections: vec![1; sections] }
    }
    /// Count each section on every page containing its body, including a shared continuous
    /// start page. The page owner still selects headers and footers; empty sections count one.
    fn of(pages: &[Page], sections: &[(usize, &SectionProps)]) -> Totals {
        let mut per = vec![0u32; sections.len()];
        for page in pages {
            let mut present = HashSet::from([page.section]);
            for item in &page.items {
                let path = match item {
                    Placed::Lines { story: StoryRef::Body, path, .. }
                    | Placed::Image { story: StoryRef::Body, path, .. }
                    | Placed::Graphic { story: StoryRef::Body, path, .. }
                    | Placed::Object { story: StoryRef::Body, path, .. }
                    | Placed::Cell { story: StoryRef::Body, table: path, .. } => path,
                    _ => continue,
                };
                let Some(&block) = path.0.first() else { continue };
                present.insert(sections.partition_point(|(end, _)| *end < block as usize));
            }
            for section in present {
                if let Some(n) = per.get_mut(section) {
                    *n = n.saturating_add(1);
                }
            }
        }
        for n in &mut per {
            *n = (*n).max(1);
        }
        Totals { pages: u32::try_from(pages.len()).unwrap_or(u32::MAX).max(1), sections: per }
    }
    fn section(&self, si: usize) -> u32 {
        self.sections.get(si).copied().unwrap_or(1)
    }
}

/// The most extra layout passes for page totals in the body: a total that changes the page
/// count (its digits pushing text to another page) is tried again, this many times at most.
const MAX_TOTAL_PASSES: usize = 3;

/// A layout pass: finished, or to be done again with the page totals it found.
enum Pass {
    Done(DocLayout),
    Again(Totals),
}

/// Walk state while paginating.
struct Ctx<'a> {
    doc: &'a Document,
    cache: &'a mut LayoutCache,
    opts: &'a LayoutOptions,
    counters: Counters,
    fields: FieldCtx,
    notes_hash: u64,
    numbers: HashMap<u32, Arc<ParaLayout>>,
    /// Automatic equation numbers so far.
    eq_count: u32,
    /// Bounds laying out text boxes inside text boxes, for the whole layout.
    boxes: wordcraft_doc::BoxBudget,
    /// Linked text boxes (Create Link).
    chains: textbox::Chains,
    /// The current section's document grid (body text only: none for headers and footers).
    grid: para::Grid,
    /// Whether tracked deletions are left out in the story being laid out
    /// ([`LayoutOptions::hides_deleted`]): the body's, or the story of the [`layout_box`] at work.
    hide_deleted: HideDeleted,
    /// Whether body text (or its notes) laid out so far shows a page total (`NUMPAGES`,
    /// `SECTIONPAGES`), which is only known once the whole body is paginated.
    uses_totals: bool,
    /// Whether to note `uses_totals` now (off while laying out headers and footers).
    track_totals: bool,
    /// The width a top-level table's percentage width is a share of, when it isn't the box's:
    /// an inherited header or footer's tables follow the section that defines it.
    pct_base: Option<f32>,
    /// Empty body paragraphs with hidden marks or continuous section breaks, in order:
    /// they take no room (see [`place_para`]).
    quiet_marks: Vec<usize>,
    /// Whether even pages are mirrored (mirrored margins, on pages): see [`mirror_even_pages`].
    mirror: bool,
    /// Cache-key hashes of the table-style text formatting cells were laid out with
    /// ([`Ctx::table_key`]).
    table_keys: Vec<(para::CellText, u64)>,
    /// While a header or footer with tables is laid out: its page, for floats in its cells placed
    /// on the page (see [`with_pin`]).
    pin: Option<Pin>,
    footer_wraps: HashMap<(usize, u32, bool), Vec<WrapArea>>,
    header_tables: HashMap<(usize, u32, bool), Vec<Rect>>,
    /// Heights of text boxes that resize to fit their text, by (story, width), worked out once
    /// per layout ([`textbox::fitted_height`]).
    fitted: HashMap<(u32, u32), Option<f32>>,
}

/// A header or footer (`story`) laid out for page `number` of a section (`sect`, `mirror`): its
/// table cells' floats placed on the page rather than in their cell ([`Float::outside_cell`],
/// Word's picture watermark in a header that starts with a table), drawn behind the header's
/// text (`behind`) or in front, in page coordinates. Once each (`seen`): a cell may be laid out
/// more than once.
struct Pin {
    story: StoryRef,
    sect: SectionProps,
    number: u32,
    mirror: bool,
    behind: Vec<Placed>,
    front: Vec<Placed>,
    seen: HashSet<(Vec<u32>, usize)>,
}

/// Whether a float's position on both axes is measured on the page (from its edges or margins),
/// not from its column, paragraph, character or line: one that can be placed on the page from
/// anywhere.
fn page_anchored(f: &Float) -> bool {
    use Anchor::*;
    matches!(f.h_rel, Page | Margin | LeftMargin | RightMargin | InsideMargin | OutsideMargin)
        && matches!(f.v_rel, Page | Margin | TopMargin | BottomMargin | InsideMargin | OutsideMargin)
}

/// Run `f`, a layout of header or footer `id` for page `number` of `sect`, with the floats in its
/// table cells that are placed on the page put there (see [`Pin`]). Returns `f`'s result and those
/// floats' items, in page coordinates: behind the header's text, and in front.
fn with_pin<T>(ctx: &mut Ctx, id: u32, sect: &SectionProps, number: u32, f: impl FnOnce(&mut Ctx) -> T) -> (T, Vec<Placed>, Vec<Placed>) {
    let tables = ctx.doc.parts.get(&id).is_some_and(|p| p.blocks.iter().any(|b| matches!(**b, Block::Table(_))));
    if !tables {
        return (f(ctx), Vec::new(), Vec::new());
    }
    let pin = Pin {
        story: StoryRef::Part(id),
        sect: sect.clone(),
        number,
        mirror: ctx.mirror,
        behind: Vec::new(),
        front: Vec::new(),
        seen: HashSet::new(),
    };
    let outer = ctx.pin.replace(pin);
    let out = f(ctx);
    let pin = std::mem::replace(&mut ctx.pin, outer);
    let (behind, front) = pin.map(|p| (p.behind, p.front)).unwrap_or_default();
    (out, behind, front)
}

impl Ctx<'_> {
    /// The outline Tight and Through wrapping follow around `o` (see [`contour::wrap_outline`]);
    /// pictures' are worked out once per picture and crop.
    fn wrap_outline(&mut self, o: &InlineObject) -> Option<Arc<Vec<(f32, f32)>>> {
        let (doc, cache) = (self.doc, &mut *self.cache);
        contour::wrap_outline(o, |media, crop| {
            let bytes = doc.media.get(media)?;
            let key = (media.to_string(), Arc::as_ptr(bytes) as usize, bytes.len(), crop.map(f32::to_bits));
            if let Some(o) = cache.outlines.get(&key) {
                return o.clone();
            }
            if cache.outlines.len() >= 256 {
                cache.outlines.clear();
            }
            let outline = contour::picture_outline(bytes, crop).map(Arc::new);
            cache.outlines.insert(key, outline.clone());
            outline
        })
    }

    /// A laid-out line number (cached).
    fn number_para(&mut self, n: u32) -> Arc<ParaLayout> {
        if let Some(p) = self.numbers.get(&n) {
            return p.clone();
        }
        let mut p = Paragraph::with_text(
            &n.to_string(),
            CharProps { size: Some(10.0), color: Some(wordcraft_doc::TextColor::Rgb(Rgb(0x60, 0x60, 0x60))), ..Default::default() },
        );
        p.props.space_after = Some(0.0);
        p.props.space_before = Some(0.0);
        let env = para::ParaEnv {
            doc: self.doc,
            width: 60.0,
            label: None,
            fields: &self.fields,
            show_hidden: false,
            field_codes: false,
            hide_deleted: Default::default(),
            table: None,
            proofing: None,
            typing_word: None,
            numeral: self.opts.numeral,
            exclusions: &[],
            eq_number: 0,
            read_line_spacing: None,
            grid: para::Grid::default(),
            fit_objects: false,
            fitted: &[],
        };
        let pl = Arc::new(para::layout_para(&p, &env));
        self.numbers.insert(n, pl.clone());
        pl
    }

    /// The list label of `p`, advancing the list counters: call it once per paragraph, in
    /// document order (only for paragraphs that will show).
    /// Whether `p` gets a list label ([`Ctx::next_label`] is `Some`), without counting it.
    fn has_label(&self, p: &Paragraph) -> bool {
        p.props
            .numbering
            .or_else(|| self.doc.styles.resolve_para(&p.props).numbering)
            .filter(|n| n.num != 0)
            .is_some_and(|n| self.doc.numbering.shows_label(n.num, n.level))
    }
    fn next_label(&mut self, p: &Paragraph) -> Option<(String, Level)> {
        let n = p.props.numbering.or_else(|| self.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0)?;
        self.counters.next_label(&self.doc.numbering, n.num, n.level)
    }

    /// Lay out `p` with its already counted `label`, so laying a paragraph out again (around
    /// floating objects, or after it moved) doesn't advance the list counters twice.
    fn para_labelled(
        &mut self,
        p: &Paragraph,
        width: f32,
        table: Option<&para::CellText>,
        exclusions: &[para::Exclusion],
        label: Option<(String, Level)>,
    ) -> Arc<ParaLayout> {
        let fitted = self.fitted_boxes(p);
        let (key, eq_number) = self.para_key(p, width, table, exclusions, label.as_ref(), &fitted);
        self.cache.used.insert(key.clone());
        if let Some(pl) = self.cache.paras.get(&key) {
            self.cache.hits += 1;
            return pl.clone();
        }
        self.cache.misses += 1;
        let typing = key.typing;
        let pl = Arc::new(para::layout_para(p, &self.para_env(width, label, table, exclusions, eq_number, typing, &fitted)));
        self.cache.paras.insert(key, pl.clone());
        pl
    }

    /// What `measure` reads from `p` laid out as [`Ctx::para_labelled`] would, remembering only
    /// the result: a table's columns measure every cell once at an unbounded width, and keeping
    /// those layouts would double what a long table holds (and walk them all again on each pass).
    fn para_measured(
        &mut self,
        p: &Paragraph,
        width: f32,
        table: Option<&para::CellText>,
        label: Option<(String, Level)>,
        measure: impl FnOnce(&ParaLayout) -> (f32, f32),
    ) -> (f32, f32) {
        let fitted = self.fitted_boxes(p);
        let (key, eq_number) = self.para_key(p, width, table, &[], label.as_ref(), &fitted);
        self.cache.used.insert(key.clone());
        if let Some(m) = self.cache.measures.get(&key) {
            self.cache.hits += 1;
            return *m;
        }
        self.cache.misses += 1;
        let typing = key.typing;
        let m = measure(&para::layout_para(p, &self.para_env(width, label, table, &[], eq_number, typing, &fitted)));
        self.cache.measures.insert(key, m);
        m
    }

    /// The heights of `p`'s inline text boxes that resize to fit their text, by object index
    /// ([`para::ParaEnv::fitted`]).
    fn fitted_boxes(&mut self, p: &Paragraph) -> Vec<(usize, f32)> {
        let mut out = Vec::new();
        for (k, o) in p.objects.iter().enumerate() {
            let InlineObject::Shape { story: Some(id), w, float, .. } = o else { continue };
            if float.wrap != Wrap::Inline || float.rot != 0.0 {
                continue;
            }
            let key = (*id, w.to_bits());
            let h = match self.fitted.get(&key) {
                Some(h) => *h,
                None => {
                    let h = textbox::fitted_height(self, *id, *w);
                    self.fitted.insert(key, h);
                    h
                }
            };
            if let Some(h) = h {
                out.push((k, h));
            }
        }
        out
    }

    /// The cache key of `p` laid out with these inputs, and its first automatic equation number
    /// (advancing the count past its equations).
    fn para_key(
        &mut self,
        p: &Paragraph,
        width: f32,
        table: Option<&para::CellText>,
        exclusions: &[para::Exclusion],
        label: Option<&(String, Level)>,
        fitted: &[(usize, f32)],
    ) -> (Key, u32) {
        let page = if has_page_fields(p) { Some(self.fields.page_key()) } else { None };
        if self.track_totals && !self.uses_totals && has_total_fields(p) {
            self.uses_totals = true;
        }
        // Automatic equation numbers count through the document.
        let eq_here: u32 = p
            .objects
            .iter()
            .map(|o| match o {
                InlineObject::Equation { math, display, .. } => math::auto_numbers(math, *display),
                _ => 0,
            })
            .sum();
        let eq_number = self.eq_count;
        self.eq_count = self.eq_count.saturating_add(eq_here);
        let key = Key {
            rev: p.rev,
            width: width.to_bits(),
            label: label.map(|(t, l)| format!("{t}|{}|{}|{:?}|{:?}|{:?}|{:?}", l.indent, l.hanging, l.suffix, l.align, l.tab, l.chr)),
            page_from: if page.is_some() && !self.fields.page_from.is_empty() { hash_of(&self.fields.page_from) } else { 0 },
            page,
            table: table.map(|t| self.table_key(t)).unwrap_or(0),
            notes: if p.objects.iter().any(|o| matches!(o, InlineObject::NoteRef { .. })) { self.notes_hash } else { 0 },
            excl: if exclusions.is_empty() { 0 } else { hash_of(&format!("{exclusions:?}")) },
            eq: if eq_here > 0 { eq_number } else { 0 },
            read_spacing: if self.opts.view == ViewMode::Read { self.opts.read_line_spacing.map(f32::to_bits) } else { None },
            grid: self.grid.key(),
            fit: self.boxes.inside(),
            fitted: if fitted.is_empty() { 0 } else { hash_of(&format!("{fitted:?}")) },
            hide_deleted: self.hide_deleted,
            sref: if self.fields.styleref.is_some() && styleref::has_styleref(p) { self.fields.styleref_key(p) } else { 0 },
            typing: self.opts.typing_word.filter(|t| t.rev == p.rev).map(|t| (t.start, t.end)),
        };
        (key, eq_number)
    }

    /// The cache-key hash of a cell's table-style text formatting. Hashing it means formatting it
    /// as text, which a long table would otherwise do for each of its paragraphs on every pass; a
    /// document's tables only give their text a few different formats.
    fn table_key(&mut self, t: &para::CellText) -> u64 {
        if let Some((_, h)) = self.table_keys.iter().find(|(c, _)| c == t) {
            return *h;
        }
        let h = hash_of(&format!("{t:?}"));
        if self.table_keys.len() >= 64 {
            self.table_keys.clear();
        }
        self.table_keys.push((t.clone(), h));
        h
    }

    fn para_env<'b>(
        &'b self,
        width: f32,
        label: Option<(String, Level)>,
        table: Option<&'b para::CellText>,
        exclusions: &'b [para::Exclusion],
        eq_number: u32,
        typing_word: Option<(usize, usize)>,
        fitted: &'b [(usize, f32)],
    ) -> para::ParaEnv<'b> {
        para::ParaEnv {
            doc: self.doc,
            width,
            label,
            fields: &self.fields,
            show_hidden: self.opts.show_hidden,
            field_codes: self.opts.field_codes,
            hide_deleted: self.hide_deleted,
            table,
            proofing: self.opts.proofing.then_some(self.opts.proof),
            typing_word,
            numeral: self.opts.numeral,
            exclusions,
            eq_number,
            read_line_spacing: if self.opts.view == ViewMode::Read { self.opts.read_line_spacing } else { None },
            grid: self.grid,
            fit_objects: self.boxes.inside(),
            fitted,
        }
    }
}

/// Which of `p`'s objects (by index) the layout leaves out ([`para::left_out`]): anchored in hidden
/// text unless `show_hidden`, in a tracked deletion with `hide_deleted`, or a hidden drawing
/// (`Float::hidden`, whatever is shown). `table_chr` is the
/// cell's table-style character formatting ([`para::CellText::chr`]), resolved as the paragraph is
/// laid out with (`StyleSheet::resolve_char_in`).
fn left_out_objects(
    doc: &Document,
    p: &Paragraph,
    table_chr: Option<&CharProps>,
    show_hidden: bool,
    hide_deleted: HideDeleted,
) -> impl Fn(usize) -> bool {
    let mut left = Vec::new();
    // No text left out with hidden text shown and markup on (hidden drawings still are, below).
    if (hide_deleted.any() || !show_hidden) && !p.objects.is_empty() {
        let style = p.props.style.as_deref();
        let is_left = |c: &CharProps| {
            let rc = doc.styles.resolve_char_in(style, table_chr, c);
            para::left_out(doc, &rc, show_hidden, hide_deleted)
        };
        // Runs and objects are both in order: one walk finds each object's run (past the last run,
        // the paragraph mark, as `props_of_char` gives), resolving a run's formatting only once.
        let mut runs = p.run_ranges().peekable();
        let mut last: Option<(usize, bool)> = None;
        for off in p.object_offsets() {
            while runs.next_if(|(r, _)| r.end <= off).is_some() {}
            let l = match runs.peek() {
                Some((r, c)) => match last {
                    Some((end, l)) if end == r.end => l,
                    _ => {
                        let l = is_left(c);
                        last = Some((r.end, l));
                        l
                    }
                },
                None => is_left(&p.mark),
            };
            left.push(l);
        }
    }
    // A hidden drawing is left out whatever is shown (Word shows it in no view).
    move |k| left.get(k).copied().unwrap_or(false) || p.objects.get(k).and_then(InlineObject::frame).is_some_and(|(_, _, f)| f.hidden)
}

/// Footnote and endnote numbers (each kind counted separately), in document order, as each
/// section's options say: starting at its "start at", numbered on, or starting again in each
/// section or (footnotes) on each page. `pages` gives the page each footnote was placed on by an
/// earlier pass (empty: numbered on). With `hide_deleted`, notes whose reference mark is a
/// tracked deletion are left out, as in the final text.
fn note_info(doc: &Document, hide_deleted: HideDeleted, pages: &HashMap<u32, usize>) -> HashMap<u32, fields::NoteNum> {
    use wordcraft_doc::section::NoteRestart;
    let sections = doc.sections();
    let opts: Vec<[wordcraft_doc::section::NoteOptions; 2]> =
        sections.iter().map(|(_, s)| [doc.note_options(s, false), doc.note_options(s, true)]).collect();
    let mut m = HashMap::new();
    // Per kind (footnotes, endnotes): the last number, and the section and page of the last note.
    let mut last: [Option<(u32, usize, Option<usize>)>; 2] = [None, None];
    let mut order = [0u32; 2];
    let mut si = 0usize;
    // Reading order, text boxes included where they are. A hidden reference mark still takes
    // its number, as in Word; a deleted one (or one in a deleted or hidden text box) does not,
    // and `para_notes` doesn't place its note either.
    let mut cur: Option<(usize, Vec<bool>)> = None;
    doc.objects_in_reading_order_by_block(&mut |block, p, k| {
        let key = p as *const Paragraph as usize;
        if cur.as_ref().is_none_or(|(at, _)| *at != key) {
            let gone = left_out_objects(doc, p, None, true, hide_deleted);
            cur = Some((key, (0..p.objects.len()).map(gone).collect()));
        }
        if cur.as_ref().is_some_and(|(_, gone)| gone.get(k).copied().unwrap_or(false)) {
            return false;
        }
        if let Some(InlineObject::NoteRef { kind, id, .. }) = p.objects.get(k) {
            while sections.get(si).is_some_and(|(end, _)| block > *end) && si + 1 < sections.len() {
                si += 1;
            }
            let ki = usize::from(*kind == wordcraft_doc::para::NoteKind::Endnote);
            let Some(o) = opts.get(si).and_then(|o| o.get(ki)) else { return true };
            let page = if ki == 0 { pages.get(id).copied() } else { None };
            let Some(slot) = last.get_mut(ki) else { return true };
            let n = match *slot {
                None => o.num_start,
                Some((prev, psect, ppage)) => {
                    let again = match o.num_restart {
                        NoteRestart::Continuous => false,
                        NoteRestart::EachSect => psect != si,
                        NoteRestart::EachPage => page.is_some() && page != ppage,
                    };
                    if again { o.num_start } else { prev.saturating_add(1) }
                }
            };
            *slot = Some((n, si, page));
            let ord = order.get_mut(ki).map(|c| {
                *c = c.saturating_add(1);
                *c
            });
            m.insert(*id, fields::NoteNum { n, fmt: o.num_fmt, section: si, order: ord.unwrap_or(0) });
        }
        true
    });
    m
}

/// Note part ids → numbers, numbered on (see [`note_info`]).
#[cfg(test)]
fn note_numbers(doc: &Document, hide_deleted: HideDeleted) -> HashMap<u32, u32> {
    note_info(doc, hide_deleted, &HashMap::new()).into_iter().map(|(id, n)| (id, n.n)).collect()
}

/// The page each footnote's text starts on.
fn footnote_pages(doc: &Document, pages: &[Page]) -> HashMap<u32, usize> {
    let mut m = HashMap::new();
    for (pi, p) in pages.iter().enumerate() {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Part(id), .. } | Placed::Cell { story: StoryRef::Part(id), .. } = it
                && doc.parts.get(id).is_some_and(|part| part.kind == wordcraft_doc::PartKind::Footnote)
            {
                m.entry(*id).or_insert(pi);
            }
        }
    }
    m
}

/// Cell identity is independent of optional table-style formatting.
enum BoxContext<'a> {
    Other,
    Cell(Option<&'a para::CellText>),
}

/// Lay out a block list into a free-standing box of `width` (no page breaks): table cells,
/// headers, footers, notes, text boxes. Returns items relative to (0, 0) and the height. `frame`
/// says where the box sits on its page, for floating objects positioned relative to the page or
/// its margins; without it (cells, notes, text boxes) they're positioned in the box.
#[allow(clippy::too_many_arguments)]
fn layout_box(
    ctx: &mut Ctx,
    story: StoryRef,
    blocks: &Blocks,
    prefix: &[u32],
    width: f32,
    context: BoxContext<'_>,
    depth: usize,
    frame: Option<PageFrame>,
) -> BoxLayout {
    // Deletions in balloons leave the body's text only: a header, note or text box keeps its own
    // inline (restored for the story around this box when it's done).
    let outer_hide = std::mem::replace(&mut ctx.hide_deleted, ctx.opts.hides_deleted(story));
    let empty_cell = para::CellText::default();
    let (table, in_cell) = match context {
        BoxContext::Other => (None, false),
        BoxContext::Cell(table) => (Some(table.unwrap_or(&empty_cell)), true),
    };
    // Word 16.113.3: the override applies in modes 11/12/14, but not mode 15.
    let allow_same_style_space =
        in_cell && ctx.doc.settings.allow_space_of_same_style_in_table && matches!(ctx.doc.settings.compat_mode, 11 | 12 | 14);
    let mut items = Vec::new();
    // Pictures behind the text go first; `behind` counts them.
    let mut behind = 0;
    // Wrap areas of the box's floating objects.
    let mut excl: Vec<WrapArea> = Vec::new();
    // Areas of the box's floating tables, which other floating tables may have to avoid.
    let mut float_tables: Vec<Rect> = Vec::new();
    let mut table_areas = Vec::new();
    let mut page_table_wraps = Vec::new();
    let mut y = 0.0f32;
    let mut prev_after = 0.0f32;
    let mut prev_after_auto = false;
    let mut prev_listed = false;
    let mut prev_style: Option<(String, bool)> = None;
    let mut last_top = 0.0f32;
    // Word has no frames in a text box: a framed paragraph there (its style's frame, usually)
    // runs with the box's text (#2357).
    let frames = !matches!(story, StoryRef::Part(id) if ctx.doc.parts.get(&id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox));
    for (i, b) in blocks.iter().enumerate() {
        last_top = y;
        let mut path = prefix.to_vec();
        path.push(i as u32);
        match &**b {
            Block::Para(p) => {
                let label = ctx.next_label(p);
                let listed = label.is_some();
                let mut pl = ctx.para_labelled(p, width, table, &[], label.clone());
                // A text frame (a footer's page number in a frame, say) is placed apart, like a
                // floating object, and the text after it wraps around it; it takes no room in
                // the flow.
                if let Some(fr) = pl.rp.frame.clone().filter(|_| frames) {
                    let fw = fr.w.unwrap_or_else(|| natural_width(&pl)).clamp(1.0, 1584.0);
                    let pf = if (fw - width).abs() > 0.01 { ctx.para_labelled(p, fw, table, &[], label) } else { pl };
                    let h = match fr.h {
                        Some(h) if fr.h_exact => h,
                        Some(h) => h.max(pf.height),
                        None => pf.height,
                    };
                    let float = frame_float(&fr);
                    let r = float_rect(frame, (0.0, width), y, fw, h, &float);
                    push_para(&mut items, story, &path, &pf, 0, pf.lines.len(), r.x, r.y, fw);
                    excl.extend(wrap_area(r, &float, None));
                    float_tables.push(r);
                    continue;
                }
                // A legacy table compatibility flag lets same-style paragraphs in a table keep their
                // spacing (`allow_same_style_space`).
                let ctxl = pl.rp.contextual_spacing && !allow_same_style_space;
                let mut before = pl.rp.space_before;
                if let Some((style, previous_contextual)) = &prev_style
                    && *style == pl.rp.style
                    && !allow_same_style_space
                {
                    if *previous_contextual {
                        y -= prev_after;
                        before = (before - prev_after).max(0.0);
                        prev_after = 0.0;
                    }
                    if ctxl {
                        before = 0.0;
                    }
                }
                // Automatic spacing: none between two list items (as in an HTML list).
                if i > 0 && prev_listed && listed {
                    if prev_after_auto {
                        y -= prev_after;
                        prev_after = 0.0;
                    }
                    if pl.rp.before_auto {
                        before = 0.0;
                    }
                }
                // Automatic spacing: none at the top of a cell or box, and it collapses with the
                // space after the paragraph above (the larger one counts).
                if pl.rp.before_auto && i == 0 {
                    before = 0.0;
                } else if i > 0 && before > 0.0 && prev_after > 0.0 && !ctx.doc.settings.sum_paragraph_spacing {
                    before = (before - prev_after).max(0.0);
                }
                y += if i == 0 { before } else { before.max(0.0) };
                // Floating objects anchored here: place them, then wrap the text around them.
                // One left out of the layout (hidden, or deleted in the final text) takes no room.
                let mut floats = HashMap::new();
                // A header or footer cell's floats placed on the page: no room taken in the cell.
                let mut pinned = HashMap::new();
                let left_out = left_out_objects(ctx.doc, p, table.map(|t| &t.chr), ctx.opts.show_hidden, ctx.hide_deleted);
                for (oi, o) in p.objects.iter().enumerate() {
                    let Some((w, h, float)) = floating(o) else { continue };
                    if left_out(oi) {
                        continue;
                    }
                    if let Some(pin) = ctx.pin.as_ref().filter(|pin| in_cell && pin.story == story && float.outside_cell && page_anchored(float)) {
                        let page = PageFrame { sect: &pin.sect, origin: (0.0, 0.0), number: pin.number, mirror: pin.mirror };
                        pinned.insert(oi, float_rect(Some(page), (0.0, width), y, w, h, float));
                        continue;
                    }
                    let r = float_rect(frame, (0.0, width), y, w, h, float);
                    let outline = ctx.wrap_outline(o);
                    excl.extend(wrap_area(r, float, outline.as_deref().map(|v| v.as_slice())));
                    floats.insert(oi, r);
                }
                let rel = rel_exclusions(&excl, 0.0, y);
                if !rel.is_empty() {
                    pl = ctx.para_labelled(p, width, table, &rel, label);
                }
                let lead = pl.lines.first().map_or(0.0, |l| l.top.max(0.0));
                push_para(&mut items, story, &path, &pl, 0, pl.lines.len(), 0.0, y + lead, width);
                // Pictures, shapes and text boxes: drawn, their areas and text boxes' text.
                if !p.objects.is_empty() {
                    let at = ObjFrame { page: frame, col: (0.0, width), para_y: y, table_chr: table.map(|t| &t.chr), pinned: Some(&pinned) };
                    let (back, front) = place_objects(ctx, story, &path, p, &pl, (0, pl.lines.len()), (0.0, y + lead), &at, &floats, depth);
                    let at = behind.min(items.len());
                    behind += back.len();
                    items.splice(at..at, back);
                    items.extend(front);
                }
                // Automatic space after: none after the last paragraph of a cell or box.
                let after = if pl.rp.after_auto && i + 1 == blocks.len() { 0.0 } else { pl.rp.space_after };
                y += pl.height + after;
                prev_after = after;
                prev_after_auto = pl.rp.after_auto;
                prev_listed = listed;
                prev_style = Some((pl.rp.style.clone(), ctxl));
            }
            Block::Table(t) => {
                if depth > 8 {
                    continue;
                }
                // Page tables do not contribute to the footer text's intrinsic height.
                if frame.is_none()
                    && !in_cell
                    && matches!(story, StoryRef::Part(id) if ctx.doc.parts.get(&id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::Footer))
                    && t.props.float.is_some_and(|f| f.v_rel == Anchor::Page)
                {
                    continue;
                }
                let tl = table::layout_table(ctx, story, t, &path, width, depth + 1);
                let (tx, ty) = match t.props.float {
                    // A floating table stands at its own position and takes no room; the text
                    // after it wraps around it.
                    Some(f) => {
                        let legacy = if ctx.doc.settings.compat_mode < 15 { table::first_cell_left_margin(ctx, t) } else { 0.0 };
                        let (r, area) = box_float_table_rect(frame, width, y, &tl, &f, legacy, &float_tables);
                        excl.push((area, false, None, WrapText::BothSides));
                        float_tables.push(area);
                        table_areas.push(area);
                        if f.v_rel == Anchor::Page {
                            page_table_wraps.push((area, false, None, WrapText::BothSides));
                        }
                        (r.x, r.y)
                    }
                    // A table in the text flow starts below the floating objects in its way.
                    None => (tl.x, below(&excl, y, tl.x, tl.x + tl.width)),
                };
                let mut row_y = ty;
                for row in &tl.rows {
                    for it in &row.items {
                        let mut it = it.clone();
                        it.translate(tx, row_y);
                        items.push(it);
                    }
                    row_y += row.height;
                }
                if t.props.float.is_none() {
                    y = row_y;
                    prev_after = 0.0;
                    prev_style = None;
                }
            }
        }
    }
    let floats_bottom = float_tables.iter().map(|r| r.bottom()).fold(0.0, f32::max);
    ctx.hide_deleted = outer_hide;
    BoxLayout { items, height: y.max(0.0), floats_bottom, last_top: last_top.max(0.0), table_areas, page_table_wraps }
}

/// The width a text frame without one takes: its widest line, with the paragraph's indents.
fn natural_width(pl: &ParaLayout) -> f32 {
    let widest = (0..pl.lines.len())
        .filter_map(|li| {
            let l = pl.lines.get(li)?;
            Some((pl.x_of(li, l.stop)? - pl.x_of(li, l.start)?).abs())
        })
        .fold(0.0f32, f32::max);
    widest + pl.rp.indent_left.max(0.0) + pl.rp.indent_right.max(0.0)
}

/// A text frame's placement as a floating object's (`w:framePr` → anchors, alignment, wrap).
fn frame_float(fr: &wordcraft_doc::props::TextFrame) -> Float {
    let anchor = |a: &str, text: Anchor| match a {
        "page" => Anchor::Page,
        "margin" => Anchor::Margin,
        _ => text,
    };
    let align = |a: Option<&str>| match a {
        Some("left" | "top") => Some(FloatAlign::Start),
        Some("center") => Some(FloatAlign::Center),
        Some("right" | "bottom") => Some(FloatAlign::End),
        Some("inside") => Some(FloatAlign::Inside),
        Some("outside") => Some(FloatAlign::Outside),
        _ => None,
    };
    Float {
        wrap: match fr.wrap.as_str() {
            "notBeside" => Wrap::TopAndBottom,
            "none" => Wrap::InFrontOfText,
            "tight" => Wrap::Tight,
            "through" => Wrap::Through,
            _ => Wrap::Square,
        },
        h_rel: anchor(&fr.h_anchor, Anchor::Column),
        v_rel: anchor(&fr.v_anchor, Anchor::Paragraph),
        x: fr.x,
        y: fr.y,
        h_align: align(fr.x_align.as_deref()),
        v_align: align(fr.y_align.as_deref()),
        dist: fr.h_space,
        dist_top: fr.v_space,
        dist_bottom: fr.v_space,
        ..Default::default()
    }
}

/// A laid-out box: items relative to its top-left, the height of its text flow, where its
/// floating tables end (a table cell grows to hold them) and where its last block starts (the
/// height without it).
struct BoxLayout {
    items: Vec<Placed>,
    table_areas: Vec<Rect>,
    page_table_wraps: Vec<WrapArea>,
    height: f32,
    floats_bottom: f32,
    last_top: f32,
}

/// Where floating table `tl` goes in a box `width` wide whose next paragraph starts at `y`, and
/// the area text keeps clear of. Positions relative to the page or its margins need the box's
/// `frame`; without one (a table cell) they're relative to the box.
fn box_float_table_rect(
    frame: Option<PageFrame>,
    width: f32,
    y: f32,
    tl: &table::TableLayout,
    f: &TableFloat,
    legacy: f32,
    float_tables: &[Rect],
) -> (Rect, Rect) {
    let h = tl.rows.iter().map(|r| r.height).sum::<f32>().clamp(0.0, 100_000.0);
    let w = tl.width.clamp(1.0, 100_000.0);
    // Across, as laid out before an even page is mirrored (see `float_rect`).
    let page_x = |fr: &PageFrame| {
        let (left, _, shift) = fr.sides();
        match f.h_rel {
            Anchor::Page => Some((-shift, fr.sect.page_w)),
            Anchor::Margin => Some((left - shift, fr.sect.text_width())),
            _ => None,
        }
    };
    let page_y = |s: &SectionProps| match f.v_rel {
        Anchor::Page => Some((0.0, s.page_h)),
        Anchor::Margin => Some((s.margin_top, s.text_height())),
        _ => None,
    };
    let h_area = frame.and_then(|fr| page_x(&fr).map(|(a, e)| (a - fr.origin.0, e))).unwrap_or((0.0, width));
    // In a cell an offset from the page or margin has nothing to measure from: the table stands
    // where the text is.
    let (v_area, y_off) = match frame.and_then(|fr| page_y(fr.sect).map(|(a, e)| (a - fr.origin.1, e))) {
        Some(area) => (area, f.y),
        None if matches!(f.v_rel, Anchor::Page | Anchor::Margin) => ((y, 0.0), 0.0),
        None => ((y, 0.0), f.y),
    };
    // Nor is there an area to align in: the table starts where the text is.
    let v_align = if frame.is_none() && matches!(f.v_rel, Anchor::Page | Anchor::Margin) { None } else { f.v_align };
    let x = align_in(h_area, f.x, w, PageFrame::h_align(frame.as_ref(), f.h_align)) - if f.h_align.is_none() { legacy } else { 0.0 };
    let r = clear_of_float_tables(Rect::new(x, align_in(v_area, y_off, h, v_align), w, h), f, legacy, h_area.0, float_tables);
    let [dl, dt, dr, db] = f.dist_from_text();
    (r, r.inset(-dl, -dt, -dr, -db))
}

/// Floating table `f` at `r`, moved below the floating tables in its way when it may not
/// overlap them. In Word 2013+ layout one placed by an offset keeps its distance from the text's
/// left edge (`text_left`).
fn clear_of_float_tables(mut r: Rect, f: &TableFloat, legacy: f32, text_left: f32, others: &[Rect]) -> Rect {
    let [dl, dt, dr, db] = f.dist_from_text();
    let keeps_text_distance = legacy == 0.0 && f.h_align.is_none() && f.x >= 0.0 && matches!(f.h_rel, Anchor::Margin | Anchor::Column);
    for _ in 0..64 {
        let area = r.inset(-dl, -dt, -dr, -db);
        let Some(below) = others.iter().filter(|o| !f.overlap && o.intersects(&area)).map(|o| o.bottom()).reduce(f32::max) else { break };
        r.y = below + dt;
        if keeps_text_distance {
            r.x = r.x.max(text_left + dl);
        }
    }
    r
}

/// A text box's default internal margins (Word's: 0.1" left/right, 0.05" top/bottom); a box's
/// own come from its [`wordcraft_doc::TextBody`].
#[cfg(test)]
const BOX_INSET_X: f32 = 7.2;
#[cfg(test)]
const BOX_INSET_Y: f32 = 3.6;

/// Keep what fits in a box `height` tall (items relative to its top): lines that don't fit are
/// dropped (whole lines; a line that only partly fits too) and so is anything starting below.
/// The first line always stays, so even a tiny box shows the start of its text.
fn fit_box(items: Vec<Placed>, height: f32) -> Vec<Placed> {
    let limit = height + 0.5;
    let mut out = Vec::with_capacity(items.len());
    let mut kept_line = false;
    for it in items {
        if let Some(r) = it.turned_bounds() {
            // Turned text (in a table cell) stays whole.
            if r.y < limit {
                out.push(it);
            }
            continue;
        }
        match it {
            Placed::Lines { story, path, para, l0, l1, x, y, turn } => {
                let Some(first) = para.lines.get(l0) else { continue };
                let mut end = l0;
                for k in l0..l1 {
                    let Some(l) = para.lines.get(k) else { break };
                    if y + (l.top - first.top) + l.height > limit && kept_line {
                        break;
                    }
                    end = k + 1;
                    kept_line = true;
                }
                if end > l0 {
                    out.push(Placed::Lines { story, path, para, l0, l1: end, x, y, turn });
                }
            }
            Placed::Fill { rect, color } if rect.y < limit => {
                out.push(Placed::Fill { rect: Rect::new(rect.x, rect.y, rect.w, rect.h.min(limit - rect.y)), color });
            }
            Placed::Rule { x0, y0, x1, y1, border } if y0.min(y1) < limit => {
                out.push(Placed::Rule { x0, y0: y0.min(limit), x1, y1: y1.min(limit), border });
            }
            Placed::Image { rect, .. }
            | Placed::Shape { rect, .. }
            | Placed::Graphic { rect, .. }
            | Placed::Clip { rect, .. }
            | Placed::Cell { rect, .. }
            | Placed::Object { rect, .. }
                if rect.y < limit =>
            {
                out.push(it);
            }
            _ => {}
        }
    }
    out
}

/// Paragraph lines plus their shading and borders.
fn push_para(items: &mut Vec<Placed>, story: StoryRef, path: &[u32], pl: &Arc<ParaLayout>, l0: usize, l1: usize, x: f32, y: f32, width: f32) {
    let (Some(first), Some(last)) = (pl.lines.get(l0), l1.checked_sub(1).and_then(|k| pl.lines.get(k))) else {
        return;
    };
    let h = last.top + last.height - first.top;
    // Indents are logical: a right-to-left paragraph's start indent (and hanging indent) is on the right.
    let start = pl.rp.indent_left.min(pl.rp.indent_left + pl.rp.indent_first);
    let (li, ri) = if pl.rp.bidi { (pl.rp.indent_right, start) } else { (start, pl.rp.indent_right) };
    let left = x + li;
    let right = x + width - ri;
    if let Some(c) = pl.rp.shading {
        let pad = pl.rp.borders.as_ref().map(|b| b.left.map(|l| l.space).unwrap_or(4.0)).unwrap_or(0.0);
        items.push(Placed::Fill { rect: Rect::new(left - pad, y, right - left + pad * 2.0, h), color: c });
    }
    items.push(Placed::Lines { story, path: Path(path.to_vec()), para: pl.clone(), l0, l1, x, y, turn: TextDirection::Horizontal });
    if let Some(b) = &pl.rp.borders {
        let sp = |o: &Option<Border>| o.map(|b| b.space).unwrap_or(0.0);
        let (lx, rx) = (left - sp(&b.left), right + sp(&b.right));
        let (ty, by) = (y - sp(&b.top), y + h + sp(&b.bottom));
        if let Some(t) = b.top.filter(|_| l0 == 0)
            && t.is_visible()
        {
            items.push(Placed::Rule { x0: lx, y0: ty, x1: rx, y1: ty, border: t });
        }
        if let Some(t) = b.bottom.filter(|_| l1 >= pl.lines.len())
            && t.is_visible()
        {
            items.push(Placed::Rule { x0: lx, y0: by, x1: rx, y1: by, border: t });
        }
        if let Some(t) = b.left.filter(Border::is_visible) {
            items.push(Placed::Rule { x0: lx, y0: ty, x1: lx, y1: by, border: t });
        }
        if let Some(t) = b.right.filter(Border::is_visible) {
            items.push(Placed::Rule { x0: rx, y0: ty, x1: rx, y1: by, border: t });
        }
    }
}

#[derive(Clone)]
struct PageBuilder<'a> {
    pages: Vec<Page>,
    sect: &'a SectionProps,
    sect_idx: usize,
    col: usize,
    cols: Vec<(f32, f32)>,
    y: f32,
    top: f32,
    bottom: f32,
    number: u32,
    web: bool,
    /// Footnotes waiting for the bottom of the current page (first the rest of notes continued
    /// from the page before).
    notes: Vec<NoteBox>,
    /// The parts of footnotes that didn't fit on their page, continued on the next one.
    carry: Vec<NoteBox>,
    /// Page bottom before footnotes took space.
    orig_bottom: f32,
    /// The paragraph above: its style, contextual spacing, space after, whether that is automatic
    /// and whether it is a list item.
    prev: Option<(String, bool, f32, bool, bool)>,
    /// The previous paragraph ended with a page break (its mark stayed with the break): the next
    /// paragraph, first on the new page, gets no space before (Word).
    after_break_mark: bool,
    /// The next section starts on a new page: this section's closing mark, when empty, needs no
    /// room (Word leaves it on a full page rather than starting a page for it).
    next_starts_page: bool,
    /// The number the next page takes instead of counting on: a section restarting its
    /// numbering, possibly at 0 (a cover page numbered 0).
    restart_at: Option<u32>,
    /// Wrap areas of floating objects on this page (see [`wrap_area`]).
    excl: Vec<WrapArea>,
    /// Wrap areas of the floating tables on this page, which others may not overlap.
    float_tables: Vec<Rect>,
    /// Line numbering counter.
    line_no: u32,
    /// Index of the first body item of the current page (vertical alignment shifts from here).
    page_items_start: usize,
    /// Where each top-level block after the first starts on the current page: its first item's
    /// index counted after the items behind the text (justified vertical alignment spreads the
    /// blocks apart there).
    block_marks: Vec<usize>,
    /// The vertical alignment of the section the current page started in: a page finished when
    /// the next section starts is aligned as its own section asks, not the next one.
    page_valign: wordcraft_doc::section::PageVAlign,
    /// The document's own footnote separators, laid out for the section's text width.
    seps: NoteSeps,
    /// Per page: how many items were inserted at the front of its list (behind the text), so a
    /// [`Snap`] can take them out again.
    behind: Vec<usize>,
    /// The bottom of the tallest column on the current page before the current one.
    max_y: f32,
    /// The bottom of the lowest line or table row placed on the current page (without the space
    /// after it).
    content_bottom: f32,
    /// Balancing columns: (page index, column height) — that page's columns end `height` below
    /// the top of the section on it.
    balance: Option<(usize, f32)>,
    /// The page with the last explicit column break.
    col_break_page: Option<usize>,
    /// The section's footnotes go right below its text, not at the page bottom ...
    sect_beneath: bool,
    /// ... and so do the current page's (set as the page starts).
    beneath: bool,
    /// Top of the footer of the pages being started: a footer reaching above the bottom margin
    /// pushes the body up (`f32::MAX`: no footer).
    foot_top: f32,
}

/// Gap above the footnote separator and its length.
const NOTE_SEP: f32 = 12.0;
/// Length of the default (short) note separator line.
pub(crate) const NOTE_SEP_W: f32 = 144.0;
/// The story of text drawn on a page that belongs to no story of the document (line numbers,
/// note separators and continuation notices): not hit by clicks.
pub const DECOR_STORY: StoryRef = StoryRef::Part(u32::MAX);

/// A note separator or continuation notice laid out from y = 0 (its items and height).
type SepBox = (Vec<Placed>, f32);

/// The document's own footnote or endnote separators and continuation notice, laid out (`None`:
/// the defaults).
#[derive(Clone, Default)]
struct NoteSeps {
    sep: Option<SepBox>,
    cont: Option<SepBox>,
    notice: Option<SepBox>,
}

impl NoteSeps {
    /// Lay out `seps` `width` wide.
    fn new(ctx: &mut Ctx, seps: &wordcraft_doc::NoteSeparators, width: f32) -> NoteSeps {
        let mut lay = |blocks: &Option<Blocks>| blocks.as_ref().map(|b| sep_box(ctx, b, width));
        NoteSeps { sep: lay(&seps.separator), cont: lay(&seps.continuation_separator), notice: lay(&seps.continuation_notice) }
    }
    /// Height of the separator above a page's notes (`cont`: the continuation separator, when the
    /// first is continued from the page before).
    fn sep_h(&self, cont: bool) -> f32 {
        let own = if cont { &self.cont } else { &self.sep };
        own.as_ref().map_or(NOTE_SEP, |s| s.1)
    }
    /// Height of the notice below a note that continues on the next page.
    fn notice_h(&self) -> f32 {
        self.notice.as_ref().map_or(0.0, |s| s.1)
    }
}

/// A note separator or continuation notice story (`blocks`) laid out `width` wide.
fn sep_box(ctx: &mut Ctx, blocks: &Blocks, width: f32) -> SepBox {
    let bl = layout_box(ctx, DECOR_STORY, blocks, &[], width, BoxContext::Other, 0, None);
    let mut items = bl.items;
    items.extend(separator_rules(blocks, &items));
    (items, bl.height)
}

/// The lines drawn by the note separators (see [`InlineObject::note_separator`]) in a separator
/// story's laid-out `items` (`blocks` its paragraphs): across each one's place in its line, half
/// way down the line.
fn separator_rules(blocks: &Blocks, items: &[Placed]) -> Vec<Placed> {
    let mut rules = Vec::new();
    for it in items {
        let Placed::Lines { para, path, l0, l1, x, y, turn: TextDirection::Horizontal, .. } = it else { continue };
        let [i] = path.0.as_slice() else { continue };
        let Some(Block::Para(p)) = blocks.get(*i as usize).map(|b| &**b) else { continue };
        let Some(first) = para.lines.get(*l0) else { continue };
        for line in para.lines.get(*l0..*l1).unwrap_or(&[]) {
            for k in line.c0..line.c1 {
                let Some(para::ClKind::Object(oi)) = para.clusters.get(k).map(|c| c.kind) else { continue };
                if p.objects.get(oi).and_then(InlineObject::as_note_separator).is_none() {
                    continue;
                }
                let (Some(a), Some(b)) = (line.cl_left(k), line.cl_right(k)) else { continue };
                let ry = y + line.top - first.top + line.height / 2.0;
                rules.push(Placed::Rule { x0: x + a.min(b), y0: ry, x1: x + a.max(b), y1: ry, border: Border::single(0.5) });
            }
        }
    }
    rules
}
/// The most pages added after the body for footnotes still continuing.
const MAX_NOTE_PAGES: usize = 10_000;

/// A footnote waiting for the bottom of a page: its items laid out from y = 0, and its height.
#[derive(Clone)]
struct NoteBox {
    id: u32,
    items: Vec<Placed>,
    h: f32,
    /// The rest of a note that didn't fit on an earlier page (drawn below the continuation
    /// separator).
    cont: bool,
}

/// The vertical extent of an item laid out in a horizontal frame.
fn v_span(it: &Placed) -> (f32, f32) {
    if let Some(r) = it.turned_bounds() {
        return (r.y, r.y + r.h);
    }
    match it {
        Placed::Lines { para, l0, l1, y, .. } => (*y, item_bottom(*y, para, *l0, *l1).unwrap_or(*y)),
        Placed::Fill { rect, .. }
        | Placed::Image { rect, .. }
        | Placed::Shape { rect, .. }
        | Placed::Graphic { rect, .. }
        | Placed::Clip { rect, .. }
        | Placed::Cell { rect, .. }
        | Placed::Object { rect, .. } => (rect.y, rect.y + rect.h),
        Placed::Rule { y0, y1, .. } => (y0.min(*y1), y0.max(*y1)),
    }
}

impl NoteBox {
    /// Lines of the note's own paragraphs, which may be split between pages (not those of its
    /// tables or text boxes).
    fn splittable(&self, it: &Placed) -> bool {
        matches!(it, Placed::Lines { story: StoryRef::Part(s), path, turn: TextDirection::Horizontal, .. } if *s == self.id && path.0.len() == 1)
    }

    /// Where the note may be split, ascending: the bottoms of its lines that have more lines
    /// below them and don't cut through a picture, table or other unsplittable item.
    fn cuts(&self) -> Vec<f32> {
        let blocked: Vec<(f32, f32)> =
            self.items.iter().filter(|it| !self.splittable(it) && !matches!(it, Placed::Fill { .. } | Placed::Rule { .. })).map(v_span).collect();
        let mut last_top = f32::MIN;
        let mut cuts = Vec::new();
        for it in &self.items {
            let Placed::Lines { para, l0, l1, y, .. } = it else { continue };
            if !self.splittable(it) {
                continue;
            }
            let Some(first) = para.lines.get(*l0) else { continue };
            for l in para.lines.get(*l0..*l1).unwrap_or(&[]) {
                last_top = last_top.max(y + l.top - first.top);
                cuts.push(y + l.top - first.top + l.height);
            }
        }
        cuts.retain(|&b| b > 0.01 && b <= last_top + 0.01 && !blocked.iter().any(|&(t, e)| t < b - 0.01 && e > b + 0.01));
        cuts.sort_by(f32::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        cuts
    }

    /// The least of the note that must stay with its reference: its first line (all of it if it
    /// can't be split).
    fn first_piece(&self) -> f32 {
        self.cuts().first().copied().unwrap_or(self.h).min(self.h)
    }

    /// Split the note at line bottom `cut`: the part above (`cut` tall) and the rest, moved up to
    /// start at 0 and marked as a continuation.
    fn split(self, cut: f32) -> (NoteBox, NoteBox) {
        let (mut head, mut tail) = (Vec::new(), Vec::new());
        for it in self.items {
            let (top, bottom) = v_span(&it);
            if bottom <= cut + 0.01 {
                head.push(it);
                continue;
            }
            if top >= cut - 0.01 {
                tail.push(it);
                continue;
            }
            match it {
                Placed::Lines { story, path, para, l0, l1, x, y, turn } => {
                    let Some(first) = para.lines.get(l0) else { continue };
                    let ft = first.top;
                    // The first line that ends below the cut starts the tail.
                    let k = (l0..l1).find(|&k| para.lines.get(k).is_none_or(|l| y + l.top - ft + l.height > cut + 0.01)).unwrap_or(l1);
                    let ky = para.lines.get(k).map_or(y, |l| y + l.top - ft);
                    if k > l0 {
                        head.push(Placed::Lines { story, path: path.clone(), para: para.clone(), l0, l1: k, x, y, turn });
                    }
                    if k < l1 {
                        tail.push(Placed::Lines { story, path, para, l0: k, l1, x, y: ky, turn });
                    }
                }
                Placed::Fill { rect, color } => {
                    head.push(Placed::Fill { rect: Rect::new(rect.x, rect.y, rect.w, cut - rect.y), color });
                    tail.push(Placed::Fill { rect: Rect::new(rect.x, cut, rect.w, rect.y + rect.h - cut), color });
                }
                Placed::Rule { x0, y0, x1, y1, border } => {
                    head.push(Placed::Rule { x0, y0: y0.min(cut), x1, y1: y1.min(cut), border });
                    tail.push(Placed::Rule { x0, y0: y0.max(cut), x1, y1: y1.max(cut), border });
                }
                // Not split (cuts avoid them): stays above.
                other => head.push(other),
            }
        }
        // The rest starts at its first line (the space between paragraphs goes with the break).
        let start = tail.iter().filter(|it| matches!(it, Placed::Lines { .. })).map(|it| v_span(it).0).fold(f32::MAX, f32::min);
        let start = if start == f32::MAX { cut } else { start.max(cut) };
        for it in &mut tail {
            it.translate(0.0, -start);
            // Shading cut at the break starts at the rest's top.
            if let Placed::Fill { rect, .. } = it
                && rect.y < 0.0
            {
                rect.h = (rect.h + rect.y).max(0.0);
                rect.y = 0.0;
            }
        }
        let rest = (self.h - start).max(0.0);
        (NoteBox { id: self.id, items: head, h: cut, cont: self.cont }, NoteBox { id: self.id, items: tail, h: rest, cont: true })
    }
}

/// Page borders, measured from the page edge (Word's default), so they surround the header,
/// footer and line numbers. The first items of every printed page.
fn page_border_rules(s: &SectionProps) -> Vec<Placed> {
    let mut v = Vec::new();
    if let Some(b) = &s.page_borders {
        let sp = |e: &Option<Border>| e.map(|x| x.space).unwrap_or(24.0);
        let (x0, x1, y0, y1) = (sp(&b.left), s.page_w - sp(&b.right), sp(&b.top), s.page_h - sp(&b.bottom));
        for (e, a, bb) in [(b.top, (x0, y0), (x1, y0)), (b.bottom, (x0, y1), (x1, y1)), (b.left, (x0, y0), (x0, y1)), (b.right, (x1, y0), (x1, y1))] {
            if let Some(e) = e.filter(Border::is_visible) {
                v.push(Placed::Rule { x0: a.0, y0: a.1, x1: bb.0, y1: bb.1, border: e });
            }
        }
    }
    v
}

impl PageBuilder<'_> {
    /// Space footnotes take at the bottom of the page.
    fn notes_h(&self) -> f32 {
        match self.notes.first() {
            None => 0.0,
            Some(first) => self.seps.sep_h(first.cont) + self.notes.iter().map(|n| n.h).sum::<f32>(),
        }
    }
    /// The lowest bottom of the body text and tables on the current page.
    fn body_floor(&self) -> f32 {
        let Some(pg) = self.pages.last() else { return self.top };
        pg.items
            .iter()
            .skip(self.page_items_start)
            .filter(|it| matches!(it, Placed::Lines { story: StoryRef::Body, .. } | Placed::Cell { story: StoryRef::Body, .. }))
            .map(|it| v_span(it).1)
            .fold(self.top, f32::max)
    }
    /// Place the collected footnotes at the bottom of the current page. When they don't all fit
    /// below the body (`bottom`), the note that doesn't is split at a line and its rest, with
    /// the notes after it, continues on the next page.
    fn flush_notes(&mut self) {
        if self.notes.is_empty() {
            return;
        }
        let total = self.notes_h();
        let x = self.sect.margin_left + self.sect.gutter;
        // The note area starts below the text (a page of continued notes alone: at its top).
        let area = self.bottom.max(self.body_floor());
        let fits = total <= self.orig_bottom - area + 0.01;
        let top = match (fits, self.beneath) {
            (false, _) => area,
            // Below the page's last line of text.
            (true, true) => self.body_floor().min(self.orig_bottom - total),
            (true, false) => self.orig_bottom - total,
        };
        let notes = std::mem::take(&mut self.notes);
        // A page that starts with a continued note gets the continuation separator.
        let cont = notes.first().is_some_and(|n| n.cont);
        let mut y = top + self.seps.sep_h(cont);
        let mut placed = Vec::new();
        // Where the continuation notice goes, below a note that continues on the next page.
        let mut notice_y = None;
        let mut rest = notes.into_iter();
        while let Some(n) = rest.next() {
            let room = self.orig_bottom - y;
            if fits || n.h <= room + 0.01 {
                let ny = y;
                y += n.h;
                placed.push((n, ny));
                continue;
            }
            // Split at the last line that fits with the notice below it. The page's first note
            // always leaves something here (its first line, or all of it if it can't be split),
            // so continuing ends.
            let first = placed.is_empty();
            let cuts = n.cuts();
            let room = room - self.seps.notice_h();
            match cuts.iter().rev().find(|&&c| c <= room + 0.01).or(cuts.first().filter(|_| first)).copied() {
                Some(c) => {
                    let (head, tail) = n.split(c);
                    notice_y = Some(y + head.h);
                    placed.push((head, y));
                    self.carry.push(tail);
                }
                None if first => placed.push((n, y)),
                None => self.carry.push(n),
            }
            self.carry.extend(rest.by_ref());
            break;
        }
        if let Some(pg) = self.pages.last_mut() {
            match if cont { &self.seps.cont } else { &self.seps.sep } {
                Some((items, _)) => pg.items.extend(items.iter().cloned().map(|mut it| {
                    it.translate(x, top);
                    it
                })),
                None => {
                    let len = if cont { self.sect.text_width() } else { NOTE_SEP_W };
                    let sy = top + NOTE_SEP / 2.0;
                    pg.items.push(Placed::Rule { x0: x, y0: sy, x1: x + len, y1: sy, border: Border::single(0.5) });
                }
            }
            for (n, ny) in placed {
                for mut it in n.items {
                    it.translate(x, ny);
                    pg.items.push(it);
                }
            }
            if let (Some(ny), Some((items, _))) = (notice_y, &self.seps.notice) {
                pg.items.extend(items.iter().cloned().map(|mut it| {
                    it.translate(x, ny);
                    it
                }));
            }
        }
    }
    /// A top-level block starts here (see `block_marks`).
    fn mark_block(&mut self) {
        if let Some(pg) = self.pages.last() {
            let behind = self.behind.last().copied().unwrap_or(0);
            self.block_marks.push(pg.items.len().saturating_sub(behind));
        }
    }
    /// Justified vertical alignment: share the room left below the page's text out equally
    /// between the gaps of its blocks, so the last line sits on the bottom margin (above the
    /// footnotes). A page with one block, or several columns, stays at the top.
    fn justify_page(&mut self) {
        if self.web || self.cols.len() > 1 {
            return;
        }
        let behind = self.behind.last().copied().unwrap_or(0);
        let first = self.page_items_start;
        // The room is measured below the last paragraph's space after: Word for Mac 16.113.4 keeps
        // it inside the justified height (72 pt after leaves the last line 72 pt above the margin).
        let space = (self.orig_bottom - self.notes_h() - self.y).max(0.0);
        let Some(pg) = self.pages.last_mut() else { return };
        let front = pg.items.len().saturating_sub(behind);
        // Blocks that moved on to the next page whole leave a mark at the end: not a gap here.
        let mut cuts: Vec<usize> = self.block_marks.iter().copied().filter(|&m| m > first && m < front).collect();
        cuts.sort_unstable();
        cuts.dedup();
        if cuts.is_empty() || space <= 0.0 {
            return;
        }
        let gap = space / cuts.len() as f32;
        for (i, it) in pg.items.iter_mut().enumerate().skip(behind.saturating_add(first)) {
            let k = cuts.partition_point(|&c| c <= i.saturating_sub(behind));
            if k > 0 {
                it.translate(0.0, gap * k as f32);
            }
        }
    }
    /// Section vertical alignment: move the page's body down (centre/bottom), or spread its
    /// blocks out (justified).
    fn apply_valign(&mut self) {
        let k = match self.page_valign {
            wordcraft_doc::section::PageVAlign::Top => return,
            wordcraft_doc::section::PageVAlign::Justify => return self.justify_page(),
            wordcraft_doc::section::PageVAlign::Center => 0.5,
            wordcraft_doc::section::PageVAlign::Bottom => 1.0,
        };
        if self.web {
            return;
        }
        let space = (self.orig_bottom - self.notes_h() - self.y).max(0.0) * k;
        let start = self.page_items_start;
        if let Some(pg) = self.pages.last_mut() {
            for it in pg.items.iter_mut().skip(start) {
                it.translate(0.0, space);
            }
        }
    }
    /// Takes back the page a page break has just started while nothing is on it, so a page start
    /// that follows (a next-page section) doesn't leave it blank: Word keeps the break's mark on
    /// the page before, and starts one page.
    fn take_back_break_page(&mut self) -> bool {
        let empty = self.after_break_mark
            && self.at_top()
            && self.notes.is_empty()
            && self.pages.len() > 1
            && self.pages.last().is_some_and(|p| p.items.len() == self.page_items_start);
        if !empty {
            return false;
        }
        self.pages.pop();
        self.behind.pop();
        self.number = self.number.saturating_sub(1);
        self.after_break_mark = false;
        // The page before was aligned when it was left: nothing on it moves again.
        self.page_items_start = self.pages.last().map_or(0, |p| p.items.len());
        true
    }
    fn new_page(&mut self, first_block: usize, body_top: f32) {
        self.apply_valign();
        self.flush_notes();
        self.beneath = self.sect_beneath;
        self.excl.clear();
        self.float_tables.clear();
        if self.sect.line_numbers.as_ref().is_some_and(|l| l.restart == wordcraft_doc::section::LineNumberRestart::Page) {
            self.line_no = 0;
        }
        let s = self.sect;
        self.number = self.restart_at.take().unwrap_or_else(|| self.number.saturating_add(1));
        let (w, h) = if self.web { (s.page_w, f32::MAX / 4.0) } else { (s.page_w, s.page_h) };
        // A footer taller than the bottom margin pushes the body up (Word: no gap), leaving it at
        // least a little room.
        let bottom = (s.page_h - s.margin_bottom).min(self.foot_top.max(body_top + 36.0));
        let body = Rect::new(s.margin_left + s.gutter, body_top, s.text_width(), (bottom - body_top).max(36.0));
        let mut decor = Vec::new();
        if !self.web {
            decor = page_border_rules(s);
            // Lines between columns.
            if s.columns.separator && s.columns.count > 1 {
                let cols = s.column_boxes();
                for w in cols.windows(2) {
                    if let [(ax, aw), (bx, bw)] = w {
                        // Midway across the space between neighbours (leftwards in an RTL section).
                        let mid = if bx >= ax { (ax + aw + bx) / 2.0 } else { (bx + bw + ax) / 2.0 };
                        let x = s.margin_left + s.gutter + mid;
                        decor.push(Placed::Rule { x0: x, y0: body_top, x1: x, y1: bottom, border: Border::single(0.5) });
                    }
                }
            }
        }
        self.page_items_start = decor.len();
        self.block_marks.clear();
        self.page_valign = s.valign;
        self.pages.push(Page { w, h, section: self.sect_idx, number: self.number, body, first_block, items: decor, ..Default::default() });
        self.behind.push(0);
        self.col = 0;
        self.cols = s.column_boxes();
        self.top = body_top;
        self.bottom = if self.web { f32::MAX / 8.0 } else { bottom };
        self.orig_bottom = self.bottom;
        self.y = self.top;
        // Notes continued from the page before come first in this page's note area.
        // (`cap_columns` below keeps the columns above them.)
        if !self.carry.is_empty() && !self.web {
            self.notes = std::mem::take(&mut self.carry);
        }
        self.max_y = self.top;
        self.content_bottom = self.top;
        self.cap_columns();
    }
    /// Where the columns end on this page: the page bottom above its footnotes, or higher up
    /// when the columns are balanced.
    fn cap_columns(&mut self) {
        self.bottom = self.uncapped_bottom();
        if let Some((page, h)) = self.balance
            && page + 1 == self.pages.len()
        {
            self.bottom = self.bottom.min(self.top + h);
        }
    }
    /// The bottom of the columns without balancing: above the footnotes.
    fn uncapped_bottom(&self) -> f32 {
        self.orig_bottom - self.notes_h()
    }
    /// Below the text on this page: the tallest column's bottom.
    fn text_bottom(&self) -> f32 {
        self.max_y.max(self.y)
    }
    fn col_x(&self) -> f32 {
        self.sect.margin_left + self.sect.gutter + self.cols.get(self.col).map(|c| c.0).unwrap_or(0.0)
    }
    fn col_w(&self) -> f32 {
        self.cols.get(self.col).map(|c| c.1).unwrap_or_else(|| self.sect.text_width())
    }
    fn at_top(&self) -> bool {
        self.y <= self.top + 0.01
    }
    /// Move to the next column or page.
    fn advance(&mut self, block: usize, body_top: f32) {
        if self.col + 1 < self.cols.len() {
            self.max_y = self.max_y.max(self.y);
            self.col += 1;
            self.y = self.top;
        } else {
            self.new_page(block, body_top);
        }
    }
    fn page(&mut self) -> Option<&mut Page> {
        self.pages.last_mut()
    }
}

/// Passes the layout makes at most for footnotes numbered again on each page: their numbers
/// depend on the pages they land on, which (barely) depend on the numbers' widths.
const MAX_NOTE_PASSES: usize = 3;

/// Lay out the whole document.
pub fn layout(doc: &Document, cache: &mut LayoutCache, opts: &LayoutOptions) -> DocLayout {
    let t0 = now_ms();
    // Page totals shown in the body (`NUMPAGES`, `SECTIONPAGES`) are known only after
    // paginating: lay out again with the totals found until they hold (bounded). Footnotes
    // numbered again on each page likewise settle over a few more passes.
    let per_page = opts.view == ViewMode::Print
        && doc.sections().iter().any(|(_, s)| doc.note_options(s, false).num_restart == wordcraft_doc::section::NoteRestart::EachPage);
    let mut note_pages = HashMap::new();
    let mut note_passes = 1;
    let mut totals = Totals::first_guess(doc.sections().len());
    let mut passes = 0;
    let mut l = loop {
        match layout_pass(doc, cache, opts, &note_pages, &totals, passes < MAX_TOTAL_PASSES) {
            Pass::Again(t) => {
                totals = t;
                passes += 1;
            }
            Pass::Done(l) => {
                if per_page && note_passes < MAX_NOTE_PASSES {
                    // Number each page's footnotes from where the pass before placed them, until they stay.
                    let placed = footnote_pages(doc, &l.pages);
                    if placed != note_pages {
                        note_pages = placed;
                        note_passes += 1;
                        continue;
                    }
                }
                break l;
            }
        }
    };
    l.ms = now_ms() - t0;
    l
}

/// One layout pass, numbering footnotes restarted on each page by `note_pages` ([`note_info`]),
/// with `totals` assumed for the page totals in the body. With `may_repeat`, a pass whose body
/// shows totals that turn out different asks to be done again.
fn layout_pass(
    doc: &Document,
    cache: &mut LayoutCache,
    opts: &LayoutOptions,
    note_pages: &HashMap<u32, usize>,
    totals: &Totals,
    may_repeat: bool,
) -> Pass {
    let env = env_hash(doc, opts);
    if env != cache.env {
        cache.paras.clear();
        cache.measures.clear();
        cache.env = env;
    }
    cache.used.clear();
    // Note references are counted as the body shows them.
    let notes = note_info(doc, opts.hides_deleted(StoryRef::Body), note_pages);
    let notes_hash = hash_of(&{
        let mut v: Vec<_> = notes.iter().map(|(a, b)| (*a, b.n, b.fmt)).collect();
        v.sort_by_key(|(a, ..)| *a);
        v
    });
    let mut ctx = Ctx {
        doc,
        cache,
        opts,
        hide_deleted: opts.hides_deleted(StoryRef::Body),
        counters: Counters::default(),
        fields: FieldCtx {
            page: 1,
            pages: totals.pages,
            section_pages: totals.section(0),
            section: 1,
            notes: Arc::new(notes),
            title: doc.core.title.as_str().into(),
            author: doc.core.creator.as_str().into(),
            ..Default::default()
        },
        notes_hash,
        numbers: HashMap::new(),
        eq_count: 0,
        boxes: wordcraft_doc::BoxBudget::default(),
        chains: Default::default(),
        grid: para::Grid::default(),
        uses_totals: false,
        track_totals: true,
        pct_base: None,
        quiet_marks: Vec::new(),
        mirror: false,
        table_keys: Vec::new(),
        fitted: HashMap::new(),
        header_tables: HashMap::new(),
        footer_wraps: HashMap::new(),
        pin: None,
    };
    let sections = doc.sections();
    let web = matches!(opts.view, ViewMode::Web | ViewMode::Draft | ViewMode::Outline);
    let read = opts.view == ViewMode::Read;
    let reflow = web || read;
    ctx.mirror = doc.settings.mirror_margins && !reflow;
    ctx.quiet_marks = sections
        .windows(2)
        .filter(|w| w[1].1.start == SectionStart::Continuous)
        .map(|w| w[0].0)
        .filter(|end| doc.body.get(*end).and_then(|b| b.as_para()).is_some_and(|p| p.text.is_empty()))
        .collect();
    if !opts.show_hidden {
        ctx.quiet_marks.extend(
            doc.body
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.as_para().filter(|p| p.mark.hidden == Some(true) && p.plain_text().is_empty()).map(|_| i)),
        );
    }
    ctx.quiet_marks.sort_unstable();
    ctx.quiet_marks.dedup();
    let outline = (opts.view == ViewMode::Outline).then(|| OutlinePass::new(doc, opts.outline.clone().unwrap_or_default()));
    let default_sect = SectionProps::default();
    let mut web_sect;
    let first_sect = match sections.first() {
        Some((_, s)) => *s,
        None => &default_sect,
    };
    let sect_ref = if reflow {
        web_sect = first_sect.clone();
        web_sect.page_w = if opts.web_width.is_finite() { opts.web_width.clamp(144.0, 20_000.0) } else { 300.0 };
        if read {
            web_sect.page_h = if opts.read_height.is_finite() { opts.read_height.clamp(144.0, 20_000.0) } else { 500.0 };
            web_sect.margin_bottom = 18.0;
            web_sect.page_borders = None;
            web_sect.line_numbers = None;
        }
        web_sect.margin_left = 18.0;
        web_sect.margin_right = 18.0;
        web_sect.margin_top = 18.0;
        web_sect.gutter = 0.0;
        web_sect.columns = Default::default();
        web_sect.valign = wordcraft_doc::section::PageVAlign::Top;
        &web_sect
    } else {
        first_sect
    };
    let mut pb = PageBuilder {
        pages: Vec::new(),
        sect: sect_ref,
        sect_idx: 0,
        col: 0,
        cols: Vec::new(),
        y: 0.0,
        top: 0.0,
        bottom: 0.0,
        number: 0,
        web,
        notes: Vec::new(),
        carry: Vec::new(),
        orig_bottom: 0.0,
        prev: None,
        after_break_mark: false,
        next_starts_page: false,
        restart_at: None,
        excl: Vec::new(),
        float_tables: Vec::new(),
        line_no: 0,
        page_items_start: 0,
        block_marks: Vec::new(),
        page_valign: wordcraft_doc::section::PageVAlign::Top,
        seps: NoteSeps::default(),
        behind: Vec::new(),
        max_y: 0.0,
        content_bottom: 0.0,
        balance: None,
        col_break_page: None,
        sect_beneath: false,
        beneath: false,
        foot_top: f32::MAX,
    };
    // Endnotes in document order, with the section of their reference; placed at the end of a
    // section that keeps them there, else at the end of the document.
    let mut endnotes: std::collections::VecDeque<(u32, usize)> = {
        let mut ids: Vec<(u32, u32, usize)> = ctx
            .fields
            .notes
            .iter()
            .filter(|(id, _)| doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::Endnote))
            .map(|(id, n)| (n.order, *id, n.section))
            .collect();
        ids.sort();
        ids.into_iter().map(|(_, id, sect)| (id, sect)).collect()
    };
    // Text boxes laid out in balancing attempts that were thrown away.
    let mut boxes_discarded = 0usize;
    let mut block = 0usize;
    // Vertical sections are laid out on their pages turned on their side (see `vertical`).
    let turned: Vec<Option<SectionProps>> = sections
        .iter()
        .map(|(_, s)| {
            (!web && s.text_direction.is_turned()).then(|| {
                let top = body_top_for(&mut ctx, s, s.headers.default);
                vertical::unturned_sect(s, top)
            })
        })
        .collect();
    let mut prev_turn = TextDirection::Horizontal;
    for (si, (end, sect)) in sections.iter().enumerate() {
        let turn = if reflow { TextDirection::Horizontal } else { sect.text_direction };
        let sect: &SectionProps = match turned.get(si) {
            Some(Some(t)) => t,
            _ if reflow => sect_ref,
            _ => sect,
        };
        pb.sect = sect;
        pb.sect_idx = si;
        ctx.grid = para::Grid::default();
        let (foot, end_opts) = (doc.note_options(sect, false), doc.note_options(sect, true));
        pb.sect_beneath = foot.pos == wordcraft_doc::section::NotePos::BeneathText;
        if !web && !doc.footnote_separators.is_empty() {
            pb.seps = NoteSeps::new(&mut ctx, &doc.footnote_separators, sect.text_width());
        }
        ctx.fields.section = u32::try_from(si).unwrap_or(u32::MAX).saturating_add(1);
        ctx.fields.section_pages = totals.section(si);
        // Headers and footers left out are linked to the previous section's, and take its room.
        let (heads, feet) = (linked(&sections, si, |s| s.headers), linked(&sections, si, |s| s.footers));
        let hb = |i: Option<u32>| defining_width(&sections, si, |s| s.headers, i);
        let fb = |i: Option<u32>| defining_width(&sections, si, |s| s.footers, i);
        let body_top = if reflow || turn.is_turned() {
            sect.margin_top
        } else {
            with_pct_base(&mut ctx, hb(heads.default), |c| body_top_for(c, sect, heads.default))
        };
        let first_top = if reflow || turn.is_turned() || !sect.title_page {
            body_top
        } else {
            with_pct_base(&mut ctx, hb(heads.first), |c| body_top_for(c, sect, heads.first))
        };
        let foot_top =
            if reflow || turn.is_turned() { f32::MAX } else { with_pct_base(&mut ctx, fb(feet.default), |c| foot_top_for(c, sect, feet.default)) };
        let first_foot = if reflow || turn.is_turned() || !sect.title_page {
            foot_top
        } else {
            with_pct_base(&mut ctx, fb(feet.first), |c| foot_top_for(c, sect, feet.first))
        };
        ctx.grid = para::Grid::of(doc, sect);
        let restart = sect.page_num_start;
        if sect.line_numbers.as_ref().is_some_and(|l| l.restart == wordcraft_doc::section::LineNumberRestart::Section) {
            pb.line_no = 0;
        }
        let start = if si == 0 { SectionStart::NextPage } else { sect.start };
        if web && si > 0 {
            // one long page
        } else if pb.pages.is_empty() || start != SectionStart::Continuous || turn != prev_turn {
            // A page holds text running one way: a section turning the other way starts a page.
            if !web {
                pb.take_back_break_page();
            }
            let s = &doc.settings;
            let (filler, restart) = section_side(start, restart, pb.number, pb.pages.len(), s.even_odd_headers, s.mirror_margins);
            // The blank page that puts the section on the right side ends the previous section.
            if filler
                && !web
                && let Some((_, prev)) = si.checked_sub(1).and_then(|k| sections.get(k))
            {
                pb.sect = prev;
                pb.sect_idx = si - 1;
                let (ph, pf) = (linked(&sections, si - 1, |s| s.headers), linked(&sections, si - 1, |s| s.footers));
                let top = body_top_for(&mut ctx, prev, ph.default);
                pb.foot_top = foot_top_for(&mut ctx, prev, pf.default);
                pb.new_page(block, top);
                pb.sect = sect;
                pb.sect_idx = si;
            }
            if restart.is_some() {
                pb.restart_at = restart;
            }
            pb.foot_top = first_foot;
            pb.new_page(block, first_top);
            pb.foot_top = foot_top;
        } else {
            // Continuous: new column layout below the current text (its tallest column).
            pb.cols = sect.column_boxes();
            pb.col = 0;
            pb.top = pb.text_bottom();
            pb.y = pb.top;
            pb.max_y = pb.top;
        }
        prev_turn = turn;
        pb.foot_top = foot_top;
        let last = (*end).min(doc.body.len().saturating_sub(1));
        // Newspaper columns are balanced on the section's last page when the next section starts
        // on the same page (a continuous break), as in Word. That page is laid out again from the
        // start of the block that reaches it, with shorter columns (see `balance_columns`).
        let balance = !web
            && pb.cols.len() > 1
            && sect.valign == wordcraft_doc::section::PageVAlign::Top
            && sections.get(si + 1).is_some_and(|(_, s)| s.start == SectionStart::Continuous);
        pb.col_break_page = None;
        pb.next_starts_page = !web && sections.get(si + 1).is_some_and(|(_, s)| s.start != SectionStart::Continuous);
        let mut from = balance.then(|| (block, Snap::take(&mut pb, &ctx)));
        while block <= last {
            let Some(b) = doc.body.get(block) else { break };
            let before = balance.then(|| (pb.pages.len(), Snap::take(&mut pb, &ctx)));
            pb.mark_block();
            match (&**b, &outline) {
                (Block::Para(p), Some(o)) => place_outline_para(&mut ctx, &mut pb, p, block, o),
                (Block::Table(_), Some(o)) if o.hidden.get(block).copied().unwrap_or(false) => {}
                (Block::Para(p), None) => place_para(&mut ctx, &mut pb, p, block, body_top),
                (Block::Table(t), _) => place_table(&mut ctx, &mut pb, t, block, body_top),
            }
            if let Some((pages, snap)) = before
                && pb.pages.len() != pages
            {
                from = Some((block, snap));
            }
            block += 1;
        }
        if let Some((first, snap)) = from {
            boxes_discarded += balance_columns(&mut ctx, &mut pb, (first, block), &snap, body_top);
        }
        // Endnotes at the end of the section: those referenced so far.
        if end_opts.pos == wordcraft_doc::section::NotePos::SectEnd && !pb.pages.is_empty() {
            let mut here = Vec::new();
            while let Some((id, _)) = endnotes.front().filter(|(_, s)| *s <= si).copied() {
                endnotes.pop_front();
                here.push(id);
            }
            place_endnotes(&mut ctx, &mut pb, here, block.saturating_sub(1));
        }
    }
    if pb.pages.is_empty() {
        pb.new_page(0, sect_ref.margin_top);
    }
    // The rest of the endnotes after the last paragraph.
    let rest: Vec<u32> = endnotes.into_iter().map(|(id, _)| id).collect();
    place_endnotes(&mut ctx, &mut pb, rest, block);
    pb.apply_valign();
    pb.flush_notes();
    // Footnotes still continuing after the last page get pages of their own. Each page takes at
    // least a line of the first note, so this ends; the cap is a backstop.
    let last_block = doc.body.len().saturating_sub(1);
    for _ in 0..MAX_NOTE_PAGES {
        if pb.carry.is_empty() {
            break;
        }
        let top = pb.top;
        pb.new_page(last_block, top);
        pb.flush_notes();
    }
    pb.carry.clear();
    let mut pages = pb.pages;
    for p in &mut pages {
        if let Some((_, s)) = sections.get(p.section)
            && !web
        {
            vertical::turn_page(p, s.text_direction, s);
        }
    }
    if web && let Some(p) = pages.first_mut() {
        let bottom = p
            .items
            .iter()
            .filter_map(|it| if let Placed::Lines { y, para, l0, l1, .. } = it { item_bottom(*y, para, *l0, *l1) } else { None })
            .fold(0.0f32, f32::max);
        p.h = bottom + 36.0;
    }
    ctx.grid = para::Grid::default();
    if !reflow {
        let found = Totals::of(&pages, &sections);
        if ctx.uses_totals && may_repeat && found != *totals {
            return Pass::Again(found);
        }
        ctx.track_totals = false;
        // Page references in headers and footers are filled in after pagination, like their page
        // totals: they don't make the body lay out again.
        if headers_footers_refer_to_pages(doc) {
            let marks = bookmark_pages(doc, &pages, &sections);
            ctx.fields.bookmarks_key = marks_key(&marks);
            ctx.fields.bookmark_pages = Arc::new(marks);
        }
        headers_footers(&mut ctx, &mut pages, &sections, &found);
        if doc.settings.mirror_margins {
            mirror_even_pages(&mut pages, &sections);
        }
    }
    // Evict paragraph layouts that weren't used this pass if the cache grew large.
    if ctx.cache.paras.len() + ctx.cache.measures.len() > 4 * ctx.cache.used.len().max(1024) {
        let used = std::mem::take(&mut ctx.cache.used);
        ctx.cache.paras.retain(|k, _| used.contains(k));
        ctx.cache.measures.retain(|k, _| used.contains(k));
        ctx.cache.used = used;
    }
    let mut index: HashMap<(StoryRef, Path), Vec<(usize, usize)>> = HashMap::new();
    for (pi, p) in pages.iter().enumerate() {
        for (ii, it) in p.items.iter().enumerate() {
            if let Placed::Lines { story, path, .. } = it {
                index.entry((*story, path.clone())).or_default().push((pi, ii));
            }
        }
    }
    let mut lay = DocLayout { pages, index, ms: 0.0, text_boxes: 0 };
    if opts.comment_balloons && !web {
        balloons::add(&mut ctx, &mut lay);
    }
    lay.text_boxes = ctx.boxes.total().saturating_sub(boxes_discarded);
    Pass::Done(lay)
}

/// Pagination state saved before a block, to lay the blocks from there out again (balancing
/// columns): the builder without its pages, how far its last page had got, and the walk state.
struct Snap<'a> {
    pb: PageBuilder<'a>,
    pages: usize,
    items: usize,
    behind: usize,
    counters: Counters,
    eq_count: u32,
    page_field: u32,
    boxes: usize,
}

impl<'a> Snap<'a> {
    fn take(pb: &mut PageBuilder<'a>, ctx: &Ctx) -> Self {
        // The pages stay out of the copy: it would cost the whole document per block.
        let (pages, behind) = (std::mem::take(&mut pb.pages), std::mem::take(&mut pb.behind));
        let copy = pb.clone();
        (pb.pages, pb.behind) = (pages, behind);
        Snap {
            pb: copy,
            pages: pb.pages.len(),
            items: pb.pages.last().map_or(0, |p| p.items.len()),
            behind: pb.behind.last().copied().unwrap_or(0),
            counters: ctx.counters.clone(),
            eq_count: ctx.eq_count,
            page_field: ctx.fields.page,
            boxes: ctx.boxes.total(),
        }
    }

    /// Go back to this state: drop the pages added since and what was added to the last one
    /// (appended, or inserted at the front behind the text).
    fn restore(&self, pb: &mut PageBuilder<'a>, ctx: &mut Ctx) {
        let (mut pages, mut behind) = (std::mem::take(&mut pb.pages), std::mem::take(&mut pb.behind));
        pages.truncate(self.pages);
        behind.truncate(self.pages);
        if let Some(pg) = pages.last_mut() {
            let k = behind.last().copied().unwrap_or(0).saturating_sub(self.behind).min(pg.items.len());
            pg.items.drain(..k);
            pg.items.truncate(self.items);
        }
        if let Some(b) = behind.last_mut() {
            *b = self.behind;
        }
        *pb = self.pb.clone();
        (pb.pages, pb.behind) = (pages, behind);
        ctx.counters = self.counters.clone();
        ctx.eq_count = self.eq_count;
        ctx.fields.page = self.page_field;
    }
}

/// One balancing pass: blocks `first..end` laid out again from `snap` with the columns on `page`
/// ending `h` below `top`; true if the page's text fits them.
fn balance_pass<'a>(
    ctx: &mut Ctx,
    pb: &mut PageBuilder<'a>,
    snap: &Snap<'a>,
    (first, end): (usize, usize),
    (page, top, h): (usize, f32, f32),
    body_top: f32,
) -> bool {
    snap.restore(pb, ctx);
    pb.balance = Some((page, h));
    pb.cap_columns();
    for block in first..end {
        match ctx.doc.body.get(block).map(|b| &**b) {
            Some(Block::Para(p)) => place_para(ctx, pb, p, block, body_top),
            Some(Block::Table(t)) => place_table(ctx, pb, t, block, body_top),
            None => break,
        }
        // Spilled onto another page: too short (and no need to lay out the rest).
        if pb.pages.len() > page + 1 {
            break;
        }
    }
    pb.balance = None;
    pb.cap_columns();
    pb.pages.len() == page + 1 && pb.content_bottom <= top + h + 0.5
}

/// Balancing lays out at most this many pages again per pass (a block reaching the last page
/// from further back is left unbalanced), so its cost stays a small multiple of one page.
const BALANCE_MAX_PAGES: usize = 16;
/// Passes of the binary search on the column height (enough for well under a point).
const BALANCE_PASSES: usize = 12;

/// Balance the columns on the last page of a section followed by a continuous break, as Word
/// does: lay blocks `first..end` out again from `snap` (taken before `first`) with the shortest
/// columns that still hold that page's text — a binary search on the column height, where a
/// height fits when nothing moves to another page and nothing (an unsplittable row, a line
/// taller than the column) runs past it. Keep-together, keep-with-next and widow/orphan control
/// apply as usual, so the columns may come out a little uneven. Not balanced: a page with an
/// explicit column break, which says where the columns end. Returns the text boxes laid out in the
/// passes thrown away.
fn balance_columns<'a>(ctx: &mut Ctx, pb: &mut PageBuilder<'a>, blocks: (usize, usize), snap: &Snap<'a>, body_top: f32) -> usize {
    let page = pb.pages.len().saturating_sub(1);
    let ncols = pb.cols.len().max(1) as f32;
    let top = pb.top;
    let full = pb.uncapped_bottom() - top;
    let used = pb.content_bottom.min(pb.uncapped_bottom()) - top;
    if pb.col_break_page == Some(page)
        || snap.pages == 0
        || (page + 1).saturating_sub(snap.pages) > BALANCE_MAX_PAGES
        || used <= 1.0
        || full <= 1.0
        || pb.cols.len() < 2
    {
        return 0;
    }
    let pass = |ctx: &mut Ctx, pb: &mut PageBuilder<'a>, h: f32| balance_pass(ctx, pb, snap, blocks, (page, top, h), body_top);
    // The text on the page fills `ncols` columns at least `used / ncols` high; the original layout
    // (`full`) fits, and so, nearly always, do columns as high as its tallest one.
    let mut lo = (used / ncols - 1.0).max(0.0);
    let mut hi = full;
    let mut last = None;
    let discard = |ctx: &Ctx| ctx.boxes.total().saturating_sub(snap.boxes);
    let mut thrown = discard(ctx);
    if used < full - 0.5 {
        let ok = pass(ctx, pb, used);
        if ok {
            hi = used;
        }
        last = Some((used, ok));
    }
    for _ in 0..BALANCE_PASSES {
        if hi - lo <= 1.0 {
            break;
        }
        let mid = (lo + hi) / 2.0;
        thrown = discard(ctx);
        let ok = pass(ctx, pb, mid);
        if ok {
            hi = mid;
        } else {
            lo = mid;
        }
        last = Some((mid, ok));
    }
    // End on the best height (the full column, as laid out first, when none was shorter).
    if last != Some((hi, true)) {
        thrown = discard(ctx);
        if !pass(ctx, pb, hi) && hi < full {
            thrown = discard(ctx);
            pass(ctx, pb, full);
        }
    }
    thrown
}

/// Place endnotes `ids` below the text so far: a separator (the document's own, else a short
/// line), then each note, moving to the next column or page when one doesn't fit.
fn place_endnotes(ctx: &mut Ctx, pb: &mut PageBuilder, ids: Vec<u32>, block: usize) {
    if ids.is_empty() || pb.web {
        return;
    }
    // The document's own endnote separator, else a short line.
    let width = pb.col_w();
    if let Some((items, h)) = ctx.doc.endnote_separators.separator.as_ref().map(|b| sep_box(ctx, b, width)) {
        let (x, y) = (pb.col_x(), pb.y);
        if let Some(pg) = pb.page() {
            pg.items.extend(items.into_iter().map(|mut it| {
                it.translate(x, y);
                it
            }));
        }
        pb.y += h;
    } else {
        pb.y += 12.0;
        let (x, ry) = (pb.col_x(), pb.y);
        if let Some(pg) = pb.page() {
            pg.items.push(Placed::Rule { x0: x, y0: ry, x1: x + NOTE_SEP_W, y1: ry, border: Border::single(0.5) });
        }
        pb.y += 8.0;
    }
    let mut added = 0usize;
    for id in ids {
        let Some(part) = ctx.doc.parts.get(&id) else { continue };
        let blocks = part.blocks.clone();
        let BoxLayout { items, height: h, .. } = layout_box(ctx, StoryRef::Part(id), &blocks, &[], pb.col_w(), BoxContext::Other, 0, None);
        // A note longer than the room left continues at a line break in the next column or on
        // a new page (Word). Each page takes at least a line, so this ends; the page cap is a
        // backstop (the rest then runs off the last page).
        let mut note = NoteBox { id, items, h, cont: false };
        loop {
            let room = pb.bottom - pb.y;
            let cut = if note.h <= room + 0.01 || added >= MAX_NOTE_PAGES {
                None
            } else {
                let cuts = note.cuts();
                match cuts.iter().rev().find(|&&c| c <= room + 0.01).or(cuts.first().filter(|_| pb.at_top())) {
                    Some(&c) => Some(c),
                    None if pb.at_top() => None,
                    None => {
                        pb.advance(block, pb.top);
                        added += 1;
                        continue;
                    }
                }
            };
            let (head, rest) = match cut {
                Some(c) => {
                    let (head, rest) = note.split(c);
                    (head, Some(rest))
                }
                None => (note, None),
            };
            let (x, y) = (pb.col_x(), pb.y);
            if let Some(pg) = pb.page() {
                for mut it in head.items {
                    it.translate(x, y);
                    pg.items.push(it);
                }
            }
            pb.y += head.h;
            let Some(rest) = rest else { break };
            note = rest;
            pb.advance(block, pb.top);
            added += 1;
        }
    }
}

fn item_bottom(y: f32, para: &ParaLayout, l0: usize, l1: usize) -> Option<f32> {
    let f = para.lines.get(l0)?;
    let l = para.lines.get(l1.checked_sub(1)?)?;
    Some(y + l.top + l.height - f.top)
}

/// Whether a section that starts on a new page needs a blank page first so it lands on the right
/// side, and the page number it restarts at. `last` is the number of the page before it and
/// `pages` how many pages there are so far.
/// - An odd- or even-page break counts on from the previous section's numbers. A section that
///   restarts its numbering gets no blank page unless the document prints both sides (different
///   odd and even headers, or mirror margins); a restart at the wrong parity takes the next number.
/// - With different odd and even headers, a next-page break that restarts the numbers at an odd
///   (even) number starts on an odd (even) sheet.
///
/// (Word's output for docxide-pdf's fixtures: bush_fires_act_comparison has a blank page 2.)
fn section_side(start: SectionStart, restart: Option<u32>, last: u32, pages: usize, even_odd: bool, mirror: bool) -> (bool, Option<u32>) {
    match start {
        SectionStart::OddPage | SectionStart::EvenPage => {
            let want_odd = start == SectionStart::OddPage;
            let continuing_odd = last.saturating_add(1) % 2 == 1;
            let filler = pages > 0 && continuing_odd != want_odd && (restart.is_none() || even_odd || mirror);
            let restart = restart.map(|n| if (n % 2 == 1) == want_odd { n } else { n.saturating_add(1) });
            (filler, restart)
        }
        SectionStart::NextPage => {
            let filler = even_odd && pages > 0 && restart.is_some_and(|n| (n % 2 == 1) != ((pages + 1) % 2 == 1));
            (filler, restart)
        }
        SectionStart::Continuous | SectionStart::NextColumn => (false, restart),
    }
}

/// The text width of the section that defines header or footer `id`, when section `si` inherits
/// it rather than naming it (`None` when it names it). Measured in Word: an inherited part's
/// percentage-width tables are sized by that section, though it's laid out on `si`'s pages.
fn defining_width(sections: &[(usize, &SectionProps)], si: usize, get: fn(&SectionProps) -> HeaderSet, id: Option<u32>) -> Option<f32> {
    let id = id?;
    let names = |s: &SectionProps| {
        let h = get(s);
        [h.default, h.first, h.even].contains(&Some(id))
    };
    let own = sections.get(si)?.1;
    if names(own) {
        return None;
    }
    sections.iter().take(si).rev().find(|(_, s)| names(s)).map(|(_, s)| s.text_width()).filter(|w| (w - own.text_width()).abs() > 0.01)
}

/// Lay out with percentage-width tables sized by `base` (see [`defining_width`]).
fn with_pct_base<T>(ctx: &mut Ctx, base: Option<f32>, f: impl FnOnce(&mut Ctx) -> T) -> T {
    let before = std::mem::replace(&mut ctx.pct_base, base);
    let out = f(ctx);
    ctx.pct_base = before;
    out
}

/// Where body text starts: the top margin, or below the header if it's taller.
fn body_top_for(ctx: &mut Ctx, sect: &SectionProps, header: Option<u32>) -> f32 {
    let Some(id) = header else { return sect.margin_top };
    let Some(part) = ctx.doc.parts.get(&id) else { return sect.margin_top };
    let blocks = part.blocks.clone();
    // Placed as `headers_footers` draws it, so page-relative floats wrap the same way.
    let frame = PageFrame { sect, origin: (sect.margin_left + sect.gutter, sect.header), number: 1, mirror: ctx.mirror };
    let track = std::mem::replace(&mut ctx.track_totals, false);
    let (h, ..) = with_pin(ctx, id, sect, 1, |ctx| {
        layout_box(ctx, StoryRef::Part(id), &blocks, &[], sect.text_width(), BoxContext::Other, 0, Some(frame)).height
    });
    ctx.track_totals = track;
    // Word starts the body right below a header that reaches past the top margin, no gap.
    sect.margin_top.max(sect.header + h)
}

/// Where the footer starts (as `headers_footers` places it), `f32::MAX` without one.
fn foot_top_for(ctx: &mut Ctx, sect: &SectionProps, footer: Option<u32>) -> f32 {
    let Some(part) = footer.and_then(|id| ctx.doc.parts.get(&id).map(|p| (id, p))) else { return f32::MAX };
    let (id, blocks) = (part.0, part.1.blocks.clone());
    // A page total in the footer is filled in after pagination, like the header's: it doesn't
    // make the body lay out again.
    let track = std::mem::replace(&mut ctx.track_totals, false);
    let (laid, ..) =
        with_pin(ctx, id, sect, 1, |ctx| layout_box(ctx, StoryRef::Part(id), &blocks, &[], sect.text_width(), BoxContext::Other, 0, None));
    ctx.track_totals = track;
    let top = sect.page_h - sect.footer - laid.height;
    // Flow-table strokes extend above their cell boxes. Floating decorations keep their wrap rules.
    let flow_cells: Vec<_> = laid
        .items
        .iter()
        .filter_map(|item| match item {
            Placed::Cell { rect, table, story: own, .. }
                if *own == StoryRef::Part(id)
                    && table.0.len() == 1
                    && table
                        .0
                        .first()
                        .and_then(|i| blocks.get(*i as usize))
                        .is_some_and(|b| matches!(&**b, Block::Table(t) if t.props.float.is_none())) =>
            {
                Some(rect)
            }
            _ => None,
        })
        .collect();
    laid.items
        .iter()
        .filter_map(|item| match item {
            Placed::Rule { x0, x1, y0, y1, border }
                if border.is_visible()
                    && (y0 - y1).abs() < 0.01
                    && flow_cells.iter().any(|r| (r.y - y0).abs() < 0.01 && *x0 <= r.x + 0.01 && *x1 >= r.right() - 0.01) =>
            {
                Some(top + y0 - border.width / 2.0)
            }
            _ => None,
        })
        .fold(top, f32::min)
}

/// Where a box of laid-out items sits on its page, for floating objects positioned relative to
/// the page or its margins.
#[derive(Clone, Copy)]
struct PageFrame<'a> {
    sect: &'a SectionProps,
    /// Page position of the box's (0, 0); (0, 0) for body text.
    origin: (f32, f32),
    /// The page's number: inside and outside are left and right on odd pages, the other way
    /// round on even ones.
    number: u32,
    /// Whether an even page is mirrored ([`mirror_even_pages`]).
    mirror: bool,
}

impl PageFrame<'_> {
    fn even(&self) -> bool {
        self.number.is_multiple_of(2)
    }

    /// The widths of the page's left and right margins (the gutter is on the inside), and how
    /// far [`mirror_even_pages`] moves what is laid out on it.
    fn sides(&self) -> (f32, f32, f32) {
        let (inside, outside) = (self.sect.margin_left + self.sect.gutter, self.sect.margin_right);
        let shift = outside - inside;
        if self.mirror && self.even() && shift.is_finite() { (outside, inside, shift) } else { (inside, outside, 0.0) }
    }

    /// Alignment `a` across the page: inside and outside become right and left on an even page.
    fn h_align(frame: Option<&Self>, a: Option<FloatAlign>) -> Option<FloatAlign> {
        match a {
            Some(FloatAlign::Inside) if frame.is_some_and(|f| f.even()) => Some(FloatAlign::End),
            Some(FloatAlign::Outside) if frame.is_some_and(|f| f.even()) => Some(FloatAlign::Start),
            a => a,
        }
    }
}

/// A floating (not inline) picture, chart or shape: its size and placement.
fn floating(o: &InlineObject) -> Option<(f32, f32, &Float)> {
    o.frame().filter(|(_, _, float)| float.wrap != Wrap::Inline)
}

/// Where a floating object goes, in the coordinates of the box `frame` places on the page.
/// `col` is the column (or cell) left edge and width, `y0` the anchor paragraph's top. Without a
/// frame (table cells, text boxes, notes) positions relative to the page or margins fall back
/// to the box itself.
fn float_rect(frame: Option<PageFrame>, col: (f32, f32), y0: f32, w: f32, h: f32, float: &Float) -> Rect {
    let (w, h) = (w.clamp(1.0, 4000.0), h.clamp(1.0, 4000.0));
    // Each axis: the reference area's start and extent, in page coordinates. Across, as laid out
    // before an even page is mirrored (less the shift it then gets).
    let page_x = |f: &PageFrame| {
        let (s, even) = (f.sect, f.even());
        let (left, right, shift) = f.sides();
        let (start, extent) = match float.h_rel {
            Anchor::Page => (0.0, s.page_w),
            Anchor::Margin => (left, s.text_width()),
            Anchor::LeftMargin => (0.0, left),
            Anchor::RightMargin => (s.page_w - right, right),
            Anchor::InsideMargin if even => (s.page_w - right, right),
            Anchor::OutsideMargin if !even => (s.page_w - right, right),
            Anchor::InsideMargin | Anchor::OutsideMargin => (0.0, left),
            _ => return None,
        };
        Some((start - shift, extent))
    };
    let page_y = |s: &SectionProps| match float.v_rel {
        Anchor::Page => Some((0.0, s.page_h)),
        Anchor::Margin => Some((s.margin_top, s.page_h - s.margin_top - s.margin_bottom)),
        Anchor::TopMargin | Anchor::InsideMargin => Some((0.0, s.margin_top)),
        Anchor::BottomMargin | Anchor::OutsideMargin => Some((s.page_h - s.margin_bottom, s.margin_bottom)),
        _ => None,
    };
    let hx = frame.and_then(|f| page_x(&f).map(|(a, e)| (a - f.origin.0, e))).unwrap_or(col);
    let vy = frame.and_then(|f| page_y(f.sect).map(|(a, e)| (a - f.origin.1, e))).unwrap_or((y0, 0.0));
    let h_align = PageFrame::h_align(frame.as_ref(), float.h_align);
    Rect::new(align_in(hx, float.x, w, h_align), align_in(vy, float.y, h, float.v_align), w, h)
}

/// Where an object of `size` starts on one axis of `area` (start, extent): at `offset` from the
/// area's start, or aligned in it when `align` is set.
fn align_in((start, extent): (f32, f32), offset: f32, size: f32, align: Option<FloatAlign>) -> f32 {
    match align {
        None => start + if offset.is_finite() { offset.clamp(-31_680.0, 31_680.0) } else { 0.0 },
        Some(FloatAlign::Start | FloatAlign::Inside) => start,
        Some(FloatAlign::Center) => start + (extent - size) / 2.0,
        Some(FloatAlign::End | FloatAlign::Outside) => start + extent - size,
    }
}

/// An area text keeps clear of: its bounds, whether text may only go above and below it, and
/// (Tight and Through wrapping) the outline text follows inside it, and the sides text may go on.
type WrapArea = (Rect, bool, Option<Arc<contour::Contour>>, WrapText);

/// The area text keeps clear of around a floating object at `r` (its rectangle grown by the
/// distances from text; Tight and Through: around `outline`, unit coordinates across `r`, when
/// it has one), and whether text may only go above and below it. `None` when text flows over
/// or under the object.
fn wrap_area(r: Rect, float: &Float, outline: Option<&[(f32, f32)]>) -> Option<WrapArea> {
    if matches!(float.wrap, Wrap::Inline | Wrap::BehindText | Wrap::InFrontOfText) {
        return None;
    }
    if matches!(float.wrap, Wrap::Tight | Wrap::Through)
        && let Some((area, c)) = outline.and_then(|o| contour::contour_area(r, float, o))
    {
        return Some((area, false, Some(Arc::new(c)), float.wrap_text));
    }
    let d = |v: f32| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 };
    let (side, top, bottom) = (d(float.dist), d(float.dist_top), d(float.dist_bottom));
    // A rotated object keeps text clear of its rotated bounds.
    let r = float.spin().bounds(r);
    Some((Rect::new(r.x - side, r.y - top, r.w + side * 2.0, r.h + top + bottom), float.wrap == Wrap::TopAndBottom, None, float.wrap_text))
}

/// Wrap areas relative to a paragraph whose column starts at `x0` and whose top is `y0`.
fn rel_exclusions(excl: &[WrapArea], x0: f32, y0: f32) -> Vec<para::Exclusion> {
    excl.iter()
        .map(|(r, tb, c, side)| para::Exclusion {
            top: r.y - y0,
            bottom: r.bottom() - y0,
            left: r.x - x0,
            right: r.right() - x0,
            top_bottom: *tb,
            contour: c.clone(),
            side: *side,
        })
        .filter(|e| e.bottom > 0.0)
        .collect()
}

/// The items drawing floating object `o` at `rect` (a group: its members), turned by its own
/// spin inside `outer` (its group's, about `outer`'s centre point).
fn float_items(o: &InlineObject, rect: Rect, outer: (Spin, Point), story: StoryRef, path: &[u32], off: usize) -> Vec<Placed> {
    let own = o.frame().map(|(_, _, f)| f.spin()).unwrap_or_default();
    let (ospin, oc) = outer;
    let rect = carried(rect, ospin, oc);
    let spin = own.within(ospin);
    match o {
        InlineObject::Image { media, crop, .. } => {
            vec![Placed::Image { rect, media: media.clone(), crop: *crop, story, path: Path(path.to_vec()), off, spin }]
        }
        InlineObject::Shape { kind, fill, stroke, stroke_width, effects, freeform, extra, .. } => {
            let object = wordcraft_doc::Pos { story, path: Path(path.to_vec()), off };
            vec![Placed::Shape {
                rect,
                kind: *kind,
                fill: *fill,
                stroke: *stroke,
                stroke_width: *stroke_width,
                effects: *effects,
                freeform: freeform.clone(),
                spin,
                extra: extra.clone(),
                object: Some(object),
            }]
        }
        InlineObject::Graphic { graphic, .. } => {
            vec![Placed::Graphic { rect, graphic: graphic.clone(), story, path: Path(path.to_vec()), off, spin }]
        }
        InlineObject::Group { .. } => {
            let centre = Point::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            match o.canvas_style() {
                // A Drawing Canvas: its background and outline, then its members clipped to its
                // frame, all turned together: the members sit in the unturned frame and the clip
                // carries the turn (drawn as the turn around the clip).
                Some(c) => {
                    let members = group_members(o, rect)
                        .into_iter()
                        .flat_map(|(r, m)| float_items(m, r, (Spin::default(), centre), story, path, off))
                        .collect();
                    vec![
                        Placed::Shape {
                            rect,
                            kind: wordcraft_doc::para::ShapeKind::Rectangle,
                            fill: c.fill,
                            stroke: c.stroke,
                            stroke_width: c.stroke_width,
                            effects: Default::default(),
                            freeform: None,
                            spin,
                            extra: Default::default(),
                            object: Some(wordcraft_doc::Pos { story, path: Path(path.to_vec()), off }),
                        },
                        Placed::Clip { rect, spin, items: members },
                    ]
                }
                None => group_members(o, rect).into_iter().flat_map(|(r, c)| float_items(c, r, (spin, centre), story, path, off)).collect(),
            }
        }
        _ => Vec::new(),
    }
}

/// A group's members, each with its rectangle when the group is at `rect`.
/// `rect` moved (not turned) to where `spin` about `centre` takes its centre: a group member's
/// frame inside its turned group.
fn carried(rect: Rect, spin: Spin, centre: Point) -> Rect {
    let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let (nx, ny) = spin.apply(centre.x, centre.y, cx, cy);
    Rect::new(rect.x + nx - cx, rect.y + ny - cy, rect.w, rect.h)
}

fn group_members(o: &InlineObject, rect: Rect) -> Vec<(Rect, &InlineObject)> {
    // Hidden members aren't placed (nor their text boxes laid out).
    o.group_rects(rect.x, rect.y, rect.w, rect.h)
        .into_iter()
        .filter(|(_, c)| !c.frame().is_some_and(|(_, _, f)| f.hidden))
        .map(|([x, y, w, h], c)| (Rect::new(x, y, w, h), c))
        .collect()
}

/// Position the floating objects anchored in `p` for a paragraph whose text starts at `y0` in
/// the current column, add their wrap areas to the page (replacing any from an earlier attempt,
/// from `excl_mark` on) and lay the paragraph out around them, with its list `label`.
fn anchor_floats(
    ctx: &mut Ctx,
    pb: &mut PageBuilder,
    p: &Paragraph,
    y0: f32,
    before: f32,
    excl_mark: usize,
    label: &Option<(String, Level)>,
) -> (HashMap<usize, Rect>, Arc<ParaLayout>, Vec<para::Exclusion>) {
    pb.excl.truncate(excl_mark);
    ctx.fields.page = pb.number;
    let (col_x, width) = (pb.col_x(), pb.col_w());
    let mut rects = HashMap::new();
    if !pb.web {
        let frame = PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror };
        // An object left out of the layout (hidden, or deleted in the final text) is not placed, so
        // it takes no room either.
        let left_out = left_out_objects(ctx.doc, p, None, ctx.opts.show_hidden, ctx.hide_deleted);
        for (oi, o) in p.objects.iter().enumerate() {
            let Some((w, h, float)) = floating(o) else { continue };
            if left_out(oi) {
                continue;
            }
            // Positions relative to the paragraph start above its space before (Word for Windows).
            let r = float_rect(Some(frame), (col_x, width), y0 - before, w, h, float);
            rects.insert(oi, r);
            let outline = ctx.wrap_outline(o);
            if let Some(area) = wrap_area(r, float, outline.as_deref().map(|v| v.as_slice())) {
                pb.excl.push(area);
            }
        }
    }
    let rel = rel_exclusions(&body_wraps(ctx, pb), col_x, y0);
    let pl = ctx.para_labelled(p, width, None, &rel, label.clone());
    (rects, pl, rel)
}

/// Footer page tables wrap only the intersecting body lines, which resume below them.
fn footer_table_wraps(ctx: &mut Ctx, pb: &PageBuilder) -> Vec<WrapArea> {
    if pb.web || ctx.opts.view == ViewMode::Read {
        return Vec::new();
    }
    let Some(page) = pb.pages.last() else { return Vec::new() };
    let first = pb.pages.iter().rev().nth(1).is_none_or(|prev| prev.section != page.section);
    let key = (page.section, page.number, first);
    if let Some(areas) = ctx.footer_wraps.get(&key) {
        return areas.clone();
    }
    ctx.footer_wraps.insert(key, Vec::new());
    let doc = ctx.doc;
    let sections = doc.sections();
    let Some((_, sect)) = sections.get(page.section) else { return Vec::new() };
    if sect.text_direction.is_turned() {
        return Vec::new();
    }
    let set = linked(&sections, page.section, |s| s.footers);
    let id = if sect.title_page && first {
        set.first
    } else if doc.settings.even_odd_headers && page.number.is_multiple_of(2) {
        set.even.or(set.default)
    } else {
        set.default
    };
    let Some(id) = id else { return Vec::new() };
    let Some(part) = doc.parts.get(&id) else { return Vec::new() };
    let blocks = part.blocks.clone();
    let x = sect.margin_left + sect.gutter;
    let base = defining_width(&sections, page.section, |s| s.footers, Some(id));
    let counters = std::mem::take(&mut ctx.counters);
    let boxes = std::mem::take(&mut ctx.boxes);
    let chains = std::mem::take(&mut ctx.chains);
    let fields = ctx.fields.clone();
    let grid = std::mem::take(&mut ctx.grid);
    let track = std::mem::replace(&mut ctx.track_totals, false);
    ctx.fields.page = page.number;
    let areas = again(ctx, 0, |ctx| {
        with_pct_base(ctx, base, |ctx| {
            let story = StoryRef::Part(id);
            let h = layout_box(ctx, story, &blocks, &[], sect.text_width(), BoxContext::Other, 0, None).height;
            let y = sect.page_h - sect.footer - h;
            let frame = PageFrame { sect, origin: (x, y), number: page.number, mirror: ctx.mirror };
            let mut areas = layout_box(ctx, story, &blocks, &[], sect.text_width(), BoxContext::Other, 0, Some(frame)).page_table_wraps;
            for (rect, ..) in &mut areas {
                rect.x += x;
                rect.y += y;
            }
            areas
        })
    });
    ctx.counters = counters;
    ctx.boxes = boxes;
    ctx.chains = chains;
    ctx.fields = fields;
    ctx.grid = grid;
    ctx.track_totals = track;
    ctx.footer_wraps.insert(key, areas.clone());
    areas
}

fn body_wraps(ctx: &mut Ctx, pb: &PageBuilder) -> Vec<WrapArea> {
    let mut areas = footer_table_wraps(ctx, pb);
    areas.extend_from_slice(&pb.excl);
    areas
}

/// Where a paragraph's floating objects are positioned from, when they aren't placed already:
/// the box on its page, the column (left, width) and the paragraph's top. Also where offsets
/// relative to the column and paragraph start, for dragging.
#[derive(Clone, Copy)]
struct ObjFrame<'a> {
    page: Option<PageFrame<'a>>,
    col: (f32, f32),
    para_y: f32,
    /// A table's character formatting under the paragraph's (see `left_out_objects`).
    table_chr: Option<&'a CharProps>,
    /// Floats placed on the page from a header or footer cell (by object index, in page
    /// coordinates): their items go to [`Ctx::pin`].
    pinned: Option<&'a HashMap<usize, Rect>>,
}

/// The pictures, shapes and text boxes in lines `l0..l1` of paragraph `p` (story `story`, at
/// `path`), whose lines start at (x, y): floating ones drawn, every one's area (hit testing,
/// selection) and text boxes' text. Floating rects come from `floats` (by object index) or are
/// worked out from `at`. Returns what goes behind the text, and what goes in front.
#[allow(clippy::too_many_arguments)]
fn place_objects(
    ctx: &mut Ctx,
    story: StoryRef,
    path: &[u32],
    p: &Paragraph,
    pl: &ParaLayout,
    (l0, l1): (usize, usize),
    (x, y): (f32, f32),
    at: &ObjFrame,
    floats: &HashMap<usize, Rect>,
    depth: usize,
) -> (Vec<Placed>, Vec<Placed>) {
    let (mut behind, mut front) = (Vec::new(), Vec::new());
    let Some(fl) = pl.lines.get(l0) else { return (behind, front) };
    // Objects left out of the layout (hidden, or deleted in the final text) aren't placed.
    let left_out = left_out_objects(ctx.doc, p, at.table_chr, ctx.opts.show_hidden, ctx.hide_deleted);
    // A floating object belongs to its paragraph, not to its line: Word puts it on the page the
    // paragraph starts on (a cover page is a paragraph holding a page break and then the text
    // box with the title; the box stays on the cover). So the paragraph's first piece places
    // every floating object, whichever of its lines holds the anchor, and later pieces none;
    // inline objects go with their own line.
    let first = l0 == 0;
    for li in 0..pl.lines.len() {
        let in_piece = (l0..l1).contains(&li);
        if !in_piece && !first {
            continue;
        }
        let Some(line) = pl.lines.get(li) else { continue };
        for k in line.c0..line.c1 {
            let Some(c) = pl.clusters.get(k) else { continue };
            let para::ClKind::Object(oi) = c.kind else { continue };
            if left_out(oi) {
                continue;
            }
            let Some(obj) = p.objects.get(oi) else { continue };
            let Some((w, h, float)) = obj.frame() else { continue };
            let floating = float.wrap != Wrap::Inline;
            if if floating { !first } else { !in_piece } {
                continue;
            }
            let pinned = at.pinned.and_then(|m| m.get(&oi)).copied().filter(|_| floating);
            let marks = (behind.len(), front.len());
            let rect = if let Some(r) = pinned {
                r
            } else if floating {
                floats.get(&oi).copied().unwrap_or_else(|| float_rect(at.page, at.col, at.para_y, w, h, float))
            } else {
                let cx = x + line.cl_left(k).unwrap_or(0.0);
                display::inline_rect(Some(obj), cx, y + (line.baseline - fl.top), c.adv, c.obj_h)
            };
            let layer = if float.wrap == Wrap::BehindText { &mut behind } else { &mut front };
            // Floating ones are drawn here; inline ones with their line.
            if floating {
                layer.extend(float_items(obj, rect, (Spin::default(), Point::default()), story, path, c.start));
            }
            let text_box = match obj {
                InlineObject::Shape { story: Some(id), .. } if ctx.doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox) => {
                    Some(*id)
                }
                _ => None,
            };
            // A box linked from another shows (and edits) that box's story.
            let shown = text_box.map(|id| textbox::shown_story(ctx, id));
            front.push(Placed::Object {
                rect,
                spin: float.spin(),
                story,
                path: Path(path.to_vec()),
                off: c.start,
                text_box: shown,
                wrap: float.wrap,
                origin: Point::new(at.col.0, at.para_y),
            });
            // A group's turn carries each box; group and member spins also turn its text.
            // Upright text stays upright, and shape flips don't mirror the lettering.
            let centre = Point::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            let boxes: Vec<(u32, Rect, Spin)> = match obj {
                InlineObject::Group { .. } => group_members(obj, rect)
                    .into_iter()
                    .filter_map(|(r, c)| {
                        c.text_box().filter(|id| ctx.doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox)).map(|id| {
                            (id, carried(r, float.spin(), centre), c.frame().map_or(Spin::default(), |(_, _, f)| f.spin()).within(float.spin()))
                        })
                    })
                    .collect(),
                _ => text_box.map(|id| (id, rect, float.spin())).into_iter().collect(),
            };
            for (id, rect, spin) in boxes {
                let mut items = textbox::box_items(ctx, id, rect, depth);
                let layer = if float.wrap == Wrap::BehindText { &mut behind } else { &mut front };
                let upright = ctx.doc.parts.get(&id).is_some_and(|part| part.body.upright);
                let turn = if (spin.deg - 90.0).abs() < 0.01 {
                    TextDirection::Down
                } else if (spin.deg - 270.0).abs() < 0.01 {
                    TextDirection::Up
                } else {
                    TextDirection::Horizontal
                };
                if !upright && turn.is_turned() {
                    let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
                    let (ox, oy) = if turn == TextDirection::Down { (cx + cy, cy - cx) } else { (cx - cy, cy + cx) };
                    for item in &mut items {
                        item.turn(turn, ox, oy);
                    }
                }
                layer.extend(items);
            }
            // Placed on the page from a header or footer cell: drawn with the header, once.
            if pinned.is_some() {
                let (b, f) = (behind.split_off(marks.0.min(behind.len())), front.split_off(marks.1.min(front.len())));
                if let Some(pin) = ctx.pin.as_mut()
                    && pin.seen.insert((path.to_vec(), c.start))
                {
                    pin.behind.extend(b);
                    pin.front.extend(f);
                }
            }
        }
    }
    (behind, front)
}

/// Footnotes referenced in a paragraph laid out as `pl`: in its text, and inside its text boxes
/// (on the box's cluster), as (cluster, note id).
fn para_notes(doc: &Document, p: &Paragraph, pl: &ParaLayout) -> Vec<(usize, u32)> {
    let mut notes = pl.notes.clone();
    if p.objects.iter().any(|o| !o.text_boxes().is_empty()) {
        for (ci, c) in pl.clusters.iter().enumerate() {
            if let para::ClKind::Object(oi) = c.kind
                && let Some(o) = p.objects.get(oi).filter(|o| !o.frame().is_some_and(|(_, _, f)| f.hidden))
            {
                for id in o.text_boxes() {
                    notes.extend(doc.notes_in_text_box(id).into_iter().map(|n| (ci, n)));
                }
            }
        }
    }
    notes
}

/// A closing mark before a continuous section takes no room after content. An otherwise empty
/// section starting a new page retains its paragraph line.
fn place_para(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, body_top: f32) {
    if !pb.web && ctx.quiet_marks.binary_search(&block).is_ok() {
        let follows_section = block.checked_sub(1).and_then(|i| ctx.doc.body.get(i)).and_then(|b| b.as_para()).is_some_and(|p| p.section.is_some());
        let section = pb.sect_idx;
        let own_page = pb.page().is_some_and(|page| page.section == section);
        // Word keeps the line when a new page's section contains only this mark.
        if p.section.is_some()
            && p.objects.is_empty()
            && (ctx.opts.show_hidden || !ctx.doc.styles.resolve_char(p.props.style.as_deref(), &p.mark).hidden)
            && !ctx.has_label(p)
            && (block == 0 || follows_section)
            && pb.at_top()
            && own_page
        {
            place_para_in_flow(ctx, pb, p, block, body_top);
            return;
        }
        // Its (empty) line stays where the text ended, even at the bottom of a full column.
        let (y, prev, bottom, content_bottom, at) = (pb.y, pb.prev.clone(), pb.bottom, pb.content_bottom, (pb.pages.len(), pb.col));
        pb.bottom = f32::MAX / 4.0;
        place_para_in_flow(ctx, pb, p, block, body_top);
        pb.bottom = bottom;
        if (pb.pages.len(), pb.col) == at {
            (pb.y, pb.prev, pb.content_bottom) = (y, prev, content_bottom);
        }
        return;
    }
    place_para_in_flow(ctx, pb, p, block, body_top);
}

/// How many times one body paragraph is laid out again for the page numbers of the later pages
/// it runs onto (#1352).
const MAX_PAGE_RELAYS: usize = 64;

/// Run `f`, a layout of a paragraph again, with the automatic equation numbers it counted from
/// (`eq_start`) the first time, and leave the count where it was: laying it out again doesn't
/// count its numbered equations twice.
fn again<T>(ctx: &mut Ctx, eq_start: u32, f: impl FnOnce(&mut Ctx) -> T) -> T {
    let now = std::mem::replace(&mut ctx.eq_count, eq_start);
    let out = f(ctx);
    ctx.eq_count = now;
    out
}

/// How many times one body paragraph's lines are broken again for the wrap areas of the later
/// columns and pages it runs onto (#1374).
const MAX_WRAP_RELAYS: usize = 64;

#[cfg(test)]
thread_local! {
    /// How many wrap relays (#1374) the layouts on this thread made: tests check none happen
    /// without something new to wrap around.
    static WRAP_RELAYS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The wrap areas for a paragraph laid out against `rel` (relative to its top) whose lines from the
/// one at `t` (relative to its top) on go into the column starting at `x0`, that line at `y` on
/// the page, which has the wrap areas `excl` (#1374). The areas the lines before were broken
/// around end at `t`; the column's start there (one reaching above `t` with an outline keeps its
/// whole outline, so it may change a line before, and the paragraph is then not broken again).
/// Areas well clear of the column (outside `-width..2 * width`, room for negative indents) can't
/// touch its lines and are left out.
fn continued_exclusions(rel: &[para::Exclusion], excl: &[WrapArea], x0: f32, y: f32, t: f32, width: f32) -> Vec<para::Exclusion> {
    let before = rel.iter().filter(|e| e.top < t).map(|e| para::Exclusion { bottom: e.bottom.min(t), ..e.clone() });
    let here = rel_exclusions(excl, x0, y - t)
        .into_iter()
        .filter(|e| e.bottom > t && e.right > -width && e.left < 2.0 * width)
        .map(|e| if e.top < t && e.contour.is_none() { para::Exclusion { top: t, ..e } } else { e });
    before.chain(here).collect()
}

/// Run `f` with `map` as the field context's page map (`FieldCtx::page_from`), then take it back.
fn with_page_map<T>(ctx: &mut Ctx, map: &mut Vec<(usize, u32)>, f: impl FnOnce(&mut Ctx) -> T) -> T {
    ctx.fields.page_from = std::mem::take(map);
    let out = f(ctx);
    *map = std::mem::take(&mut ctx.fields.page_from);
    out
}

fn place_para_in_flow(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, body_top: f32) {
    let width = pb.col_w();
    // Floating objects anchored here join the page's wrap areas before the text is laid out; if
    // the paragraph then starts somewhere else (another column or page, or contextual spacing
    // drops its space before), they are placed again there.
    let space_before = ctx.doc.styles.resolve_para(&p.props).space_before;
    let excl_mark = pb.excl.len();
    let guess = pb.y + space_before;
    let label = ctx.next_label(p);
    // The pages this paragraph's fields are on after its first (`FieldCtx::page_from`), kept here
    // and lent to the field context only while this paragraph itself is laid out, so footnotes,
    // text boxes and headers laid out in between never see it (#1352).
    let mut page_map: Vec<(usize, u32)> = Vec::new();
    // Where its automatic equation numbers start: each layout of it again counts from here.
    let eq_start = ctx.eq_count;
    let (mut float_rects, mut pl, mut rel) = anchor_floats(ctx, pb, p, guess, space_before, excl_mark, &label);
    // The page its fields were laid out for (and the width), and how many times it was laid out again for a later
    // page (bounded: a paragraph over hundreds of pages keeps the last number it got).
    let (mut laid_page, mut laid_width) = (pb.number, pb.col_w());
    let mut page_relays = 0usize;
    // How many times its lines were broken again for the wrap areas of a later column (#1374).
    let mut wrap_relays = 0usize;
    // The anchor paragraph's top (above its space before) the floats were placed from.
    let mut anchor_y = pb.y;
    let spot = |pb: &PageBuilder| (pb.pages.len(), pb.col);
    let placed_at = spot(pb);
    let broke_before = pl.rp.page_break_before && !pb.at_top() && !pb.web;
    if broke_before {
        pb.new_page(block, body_top);
    }
    // Same-style contextual spacing removes this paragraph's before-space, or subtracts the
    // previous paragraph's after-space from the merged gap (ISO/IEC 29500-1 §17.3.1.9).
    let mut before = pl.rp.space_before;
    let prev_para = pb.prev.clone();
    // At the top of a page, measured in Word for Windows: no space before after a paragraph that
    // ended with a page break, nor (from Word 2013's layout, compatibility mode 15) for a page
    // break before; a section break or, in older modes, a page break before keeps it.
    let after_break_mark = std::mem::take(&mut pb.after_break_mark);
    if pb.at_top() && (after_break_mark || (broke_before && ctx.doc.settings.compat_mode >= 15)) {
        before = 0.0;
    } else if pb.at_top()
        && !ctx.doc.settings.sum_paragraph_spacing
        && let Some((_, _, after, _, _)) = &prev_para
    {
        // After a page break before or a section break the space between the paragraphs is
        // still counted as one: the previous paragraph's space after (left on its page) comes
        // off the space before here (Word: 24 pt before after 10 pt after leaves 14 pt).
        before = (before - after.max(0.0)).max(0.0);
    }
    if let Some((style, ctxl, mut after, after_auto, listed)) = pb.prev.take()
        && !pb.at_top()
    {
        // Contextual spacing suppresses each paragraph's own side of the gap.
        if style == pl.rp.style {
            if ctxl {
                pb.y -= after;
                before = (before - after).max(0.0);
                after = 0.0;
            }
            if pl.rp.contextual_spacing {
                before = 0.0;
            }
        }
        // Automatic spacing: none between two list items (as in an HTML list; measured in Word:
        // any two items, of one list or two, at any level).
        if listed && label.is_some() {
            if after_auto {
                pb.y -= after;
                after = 0.0;
            }
            if pl.rp.before_auto {
                before = 0.0;
            }
        }
        // The space between two paragraphs is the larger of the first's space after and the
        // second's space before, not their sum.
        if before > 0.0 && after > 0.0 && !ctx.doc.settings.sum_paragraph_spacing {
            before = (before - after).max(0.0);
        }
    }
    // Automatic spacing: none at the top of the document.
    if pl.rp.before_auto && block == 0 {
        before = 0.0;
    }
    pb.y += before;
    let mut applied_before = before;
    // Keep lines together: if it doesn't fit but would on an empty column, move it. Keep with
    // next alone moves a paragraph only when it would fit here by itself (Word for Windows): one
    // that doesn't breaks as usual, its last lines going over with the next paragraph.
    if pl.rp.keep_lines || (pl.rp.keep_next && pb.y + pl.height <= pb.bottom + 0.01) {
        let need = pl.height + if pl.rp.keep_next { next_first_line(ctx, block, width, 0) } else { 0.0 };
        // (Against the whole column: balanced columns grow to hold it.)
        if pb.y + need > pb.bottom && !pb.at_top() && need <= pb.uncapped_bottom() - pb.top {
            pb.advance(block, body_top);
            pb.y += pl.rp.space_before.min(0.0);
            applied_before = pl.rp.space_before.min(0.0);
        }
    }
    // A different top only matters when there is something to wrap around.
    if spot(pb) != placed_at || ((pb.y - guess).abs() > 0.01 && !pb.excl.is_empty()) {
        let y0 = pb.y;
        (float_rects, pl, rel) = again(ctx, eq_start, |ctx| anchor_floats(ctx, pb, p, y0, applied_before, excl_mark, &label));
        (laid_page, laid_width) = (pb.number, pb.col_w());
        anchor_y = y0 - applied_before;
    }
    let mut n = pl.lines.len();
    // Worked out only for a last line on a page with notes.
    let mut after_on_page: Option<f32> = None;
    let mut l0 = 0;
    while l0 < n {
        // The lines going into a new column (or page) were broken around the wrap areas where the
        // paragraph started: break them again around this column's (the lines before stay as they
        // were, and so do the paragraph's own floating objects), when they differ (#1374).
        if l0 > 0
            && !pb.web
            && wrap_relays < MAX_WRAP_RELAYS
            && let Some((resume, t)) = pl.lines.get(l0).map(|l| (l.start, l.top))
        {
            let here = continued_exclusions(&rel, &body_wraps(ctx, pb), pb.col_x(), pb.y, t, laid_width);
            if here != rel {
                wrap_relays += 1;
                ctx.fields.page = laid_page;
                #[cfg(test)]
                WRAP_RELAYS.with(|c| c.set(c.get() + 1));
                let relaid =
                    with_page_map(ctx, &mut page_map, |ctx| again(ctx, eq_start, |ctx| ctx.para_labelled(p, laid_width, None, &here, label.clone())));
                // Only if the rest still starts at the same line, at the same height (an outline
                // reaching above it could move the lines before down, and the column's areas with them).
                if let Some(k) = relaid.lines.iter().position(|l| l.start == resume)
                    && relaid.lines.get(k).is_some_and(|l| (l.top - t).abs() < 0.01)
                {
                    (pl, rel) = (relaid, here);
                    n = pl.lines.len();
                    l0 = k;
                }
            }
        }
        // Page fields in the lines going onto a new page show that page's number: lay the
        // paragraph out again with the fields from there on on this page (the lines before stay as
        // they were, so the line the rest starts with is still there), breaking the rest again for
        // the new results' widths (#1352). Only a paragraph with page fields pays for this.
        if l0 > 0
            && pl.has_page_fields
            && !pb.web
            && page_relays < MAX_PAGE_RELAYS
            && let Some(resume) = pl.lines.get(l0).map(|l| l.start)
            && page_map.last().map_or(laid_page, |e| e.1) != pb.number
            && has_page_number_fields(p)
        {
            page_relays += 1;
            page_map.push((resume, pb.number));
            ctx.fields.page = laid_page;
            let relaid =
                with_page_map(ctx, &mut page_map, |ctx| again(ctx, eq_start, |ctx| ctx.para_labelled(p, laid_width, None, &rel, label.clone())));
            match relaid.lines.iter().position(|l| l.start == resume) {
                Some(k) => {
                    pl = relaid;
                    n = pl.lines.len();
                    l0 = k;
                }
                None => {
                    page_map.pop();
                }
            }
        }
        // What is laid out for these lines from here (their footnotes) is on this page.
        ctx.fields.page = pb.number;
        // How many lines fit?
        let Some(first) = pl.lines.get(l0) else { break };
        // A paragraph's first line may start below its top (pushed down by a floating object).
        let lead = if l0 == 0 { first.top.max(0.0) } else { 0.0 };
        let mut l1 = l0;
        let mut forced_break = None;
        // A page break followed only by the paragraph mark: the mark's (empty) line stays on this
        // page with the break, so nothing of the paragraph goes to the next page (Word).
        let mut mark_with_break = false;
        let mut new_notes: Vec<NoteBox> = Vec::new();
        let notes = para_notes(ctx.doc, p, &pl);
        while l1 < n {
            let Some(l) = pl.lines.get(l1) else { break };
            // Footnotes referenced on this line go to the bottom of this page.
            let mut line_notes = Vec::new();
            if !pb.web {
                for (ci, id) in &notes {
                    if *ci >= l.c0
                        && *ci < l.c1
                        && !pb.notes.iter().chain(new_notes.iter()).any(|x| x.id == *id)
                        && let Some(part) = ctx.doc.parts.get(id).filter(|p| p.kind == wordcraft_doc::PartKind::Footnote)
                    {
                        let blocks = part.blocks.clone();
                        let BoxLayout { items, height: h, .. } =
                            layout_box(ctx, StoryRef::Part(*id), &blocks, &[], pb.sect.text_width(), BoxContext::Other, 0, None);
                        line_notes.push(NoteBox { id: *id, items, h, cont: false });
                    }
                }
            }
            let extra: f32 = line_notes.iter().map(|n| n.h).sum::<f32>()
                + if pb.notes.is_empty() && new_notes.is_empty() && !line_notes.is_empty() { pb.seps.sep_h(false) } else { 0.0 };
            let limit = pb.bottom
                - new_notes.iter().map(|n| n.h).sum::<f32>()
                - if pb.notes.is_empty() && !new_notes.is_empty() { pb.seps.sep_h(false) } else { 0.0 }
                - extra;
            let bottom = pb.y + lead + (l.top + l.height - first.top);
            // On a page with footnotes the paragraph's last line keeps its space after clear of
            // the separator (Word): if the space doesn't fit, the line goes to the next page (and,
            // with widow control, one more with it). Without notes the space after may run past
            // the bottom margin as usual.
            let has_notes = !pb.notes.is_empty() || !new_notes.is_empty() || !line_notes.is_empty();
            let tail = if has_notes && l1 + 1 == n {
                *after_on_page.get_or_insert_with(|| space_after_kept(ctx, block, &pl.rp, label.is_some()))
            } else {
                0.0
            };
            let over = bottom + tail > limit + 0.01;
            // Notes too long for the rest of the page continue on the next one: the line still
            // fits if the notes before its last one, and that one's first line, fit below it.
            // The notes then take the rest of the page.
            let split = bottom > limit + 0.01
                && line_notes.split_last().is_some_and(|(last, others)| {
                    let before: f32 = pb.notes.iter().chain(&new_notes).chain(others).map(|n| n.h).sum();
                    let sep = pb.seps.sep_h(pb.notes.first().is_some_and(|n| n.cont));
                    pb.body_floor().max(bottom) + sep + before + last.first_piece() + pb.seps.notice_h() <= pb.orig_bottom + 0.01
                });
            if over && l1 > l0 && !split {
                break;
            }
            // A line holding nothing but a page or column break stays at the bottom of the page
            // even when it doesn't fit: it only ends the page, and moving it would leave a blank
            // page behind it (Word does the same).
            let only_break = matches!(l.end, LineEnd::PageBreak | LineEnd::ColumnBreak)
                && p.text
                    .get(l.start..l.stop)
                    .is_some_and(|t| t.chars().all(|c| matches!(c, wordcraft_doc::para::PAGE_BREAK | wordcraft_doc::para::COLUMN_BREAK)));
            // An empty paragraph closing a section before a new page: measured in Word, it stays
            // on a full page instead of starting one of its own.
            let closing_mark = pb.next_starts_page && p.section.is_some() && p.text.is_empty();
            if over && l1 == l0 && !pb.at_top() && !split && !only_break && !closing_mark {
                break;
            }
            new_notes.extend(line_notes);
            l1 += 1;
            if matches!(l.end, LineEnd::PageBreak | LineEnd::ColumnBreak) && !pb.web {
                forced_break = Some(l.end);
                if l.end == LineEnd::PageBreak && l1 + 1 == n && pl.lines.get(l1).is_some_and(|m| only_markers(p, m.start..m.stop)) {
                    l1 += 1;
                    mark_with_break = true;
                }
                break;
            }
            if split {
                break;
            }
        }
        // Widow/orphan control (2-line minimum at either end).
        if forced_break.is_none() && l1 < n && pl.rp.widow_control && n >= 2 {
            if l1 - l0 == 1 && l0 == 0 && !pb.at_top() {
                l1 = l0; // orphan: move the first line to the next page
            } else if n - l1 == 1 && (l1 - l0 >= 3 || (l0 > 0 && l1 - l0 >= 2)) {
                l1 -= 1; // widow: take one more line along
            } else if n - l1 == 1 && l0 == 0 && l1 - l0 == 2 && !pb.at_top() {
                // Neither split of a three-line paragraph is allowed (two and a widow, or an
                // orphan and two): it moves whole, as in Word.
                l1 = l0;
            }
        }
        // A row (text both sides of a floating object) stays together.
        while l1 > l0 && l1 < n && pl.lines.get(l1).is_some_and(|l| l.beside) {
            l1 -= 1;
        }
        if l1 == l0 {
            if pb.at_top() {
                l1 = l0 + 1; // can't fit even one line on an empty page: overflow
            } else {
                pb.advance(block, body_top);
                if l0 == 0 {
                    let y0 = pb.y;
                    (float_rects, pl, rel) = again(ctx, eq_start, |ctx| anchor_floats(ctx, pb, p, y0, 0.0, excl_mark, &label));
                    (laid_page, laid_width) = (pb.number, pb.col_w());
                    anchor_y = y0;
                    n = pl.lines.len();
                }
                continue;
            }
        }
        // Keep only the notes of the lines that are placed.
        let placed_end = pl.lines.get(l1.saturating_sub(1)).map(|l| l.c1).unwrap_or(0);
        new_notes.retain(|n| notes.iter().any(|(ci, nid)| *nid == n.id && *ci < placed_end));
        let before = pb.notes_h();
        pb.notes.extend(new_notes);
        if pb.balance.is_some() {
            // Balanced columns end where the balance puts them unless the notes reach higher.
            pb.cap_columns();
        } else {
            // Relative, so a bottom moved down for notes continuing on the next page stays put.
            pb.bottom -= pb.notes_h() - before;
        }
        // Mirror indents: on an even (left-hand) page the inside indent is on the right, so the
        // paragraph's lines (broken for the same width) move over by the indents' difference.
        let x = pb.col_x() + mirror_shift(&pl.rp, pb.number, pb.web);
        let y = pb.y + lead;
        // Notes that no longer fit below the text (a long one, or the rest of one continued from
        // the page before) get what's left under it and continue on the next page.
        if !pb.notes.is_empty()
            && !pb.web
            && let Some(placed_bottom) = item_bottom(y, &pl, l0, l1)
            && placed_bottom > pb.bottom + 0.01
        {
            pb.bottom = pb.body_floor().max(placed_bottom).min(pb.orig_bottom);
        }
        let mut items = Vec::new();
        push_para(&mut items, StoryRef::Body, &[block as u32], &pl, l0, l1, x, y, width);
        // Pictures, shapes and text boxes in these lines (web view: floats placed by this piece).
        let at = ObjFrame {
            page: Some(PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror }),
            col: (x, width),
            para_y: if pb.web { y } else { anchor_y },
            table_chr: None,
            pinned: None,
        };
        let (behind, front) = place_objects(ctx, StoryRef::Body, &[block as u32], p, &pl, (l0, l1), (x, y), &at, &float_rects, 0);
        items.extend(front);
        // Line numbers in the left margin.
        if let Some(ln) = pb.sect.line_numbers.clone().filter(|_| !pb.web && pl.rp.style != "Header" && !pl.rp.suppress_line_numbers)
            && let Some(first) = pl.lines.get(l0)
        {
            for li in l0..l1 {
                let Some(line) = pl.lines.get(li).filter(|l| !l.beside) else { continue };
                pb.line_no = pb.line_no.saturating_add(1);
                let n = pb.line_no.saturating_add(ln.start.saturating_sub(1));
                if ln.count_by > 1 && !n.is_multiple_of(ln.count_by) {
                    continue;
                }
                let num = ctx.number_para(n);
                let w = num.lines.first().and_then(|l| l.xs.last().copied()).unwrap_or(10.0);
                let dist = if ln.distance > 0.0 { ln.distance } else { 18.0 };
                let ly = y + (line.top - first.top) + (line.baseline - line.top) - num.lines.first().map(|l| l.baseline).unwrap_or(10.0);
                items.push(Placed::Lines {
                    story: DECOR_STORY,
                    path: Path(vec![]),
                    para: num.clone(),
                    l0: 0,
                    l1: 1,
                    x: pb.sect.margin_left + pb.sect.gutter - dist - w,
                    y: ly,
                    turn: TextDirection::Horizontal,
                });
            }
        }
        let n_behind = behind.len();
        if let Some(pg) = pb.pages.last_mut() {
            for (i, it) in behind.into_iter().enumerate() {
                pg.items.insert(i.min(pg.items.len()), it);
            }
            pg.items.extend(items);
        }
        if let Some(b) = pb.behind.last_mut() {
            *b += n_behind;
        }
        if let (Some(f), Some(l)) = (pl.lines.get(l0), pl.lines.get(l1 - 1)) {
            pb.y = y + (l.top + l.height - f.top);
            pb.content_bottom = pb.content_bottom.max(pb.y);
        }
        l0 = l1;
        // A paragraph that starts with a page break starts on the new page, its space before
        // there (Word).
        let starts_with_break = l0 == 1
            && pl.lines.first().is_some_and(|l| {
                p.text.get(l.start..l.stop).is_some_and(|t| !t.is_empty() && t.chars().all(|c| c == wordcraft_doc::para::PAGE_BREAK))
            });
        match forced_break {
            Some(LineEnd::PageBreak) if mark_with_break => {
                pb.new_page(block, body_top);
                pb.after_break_mark = true;
            }
            Some(LineEnd::PageBreak) if starts_with_break && !pb.web => {
                pb.new_page(block, body_top);
                pb.y += pl.rp.space_before.max(0.0);
            }
            Some(LineEnd::PageBreak) => pb.new_page(block, body_top),
            Some(LineEnd::ColumnBreak) => {
                pb.col_break_page = Some(pb.pages.len().saturating_sub(1));
                pb.advance(block, body_top);
                // Unequal columns: the text after the break is broken for its own column's width
                // (Word), not the width of the column it started in.
                if (pb.col_w() - width).abs() > 0.5
                    && let Some(resume) = pl.lines.get(l0).map(|l| l.start)
                {
                    let y0 = pb.y;
                    let (rects, relaid, relaid_rel) =
                        with_page_map(ctx, &mut page_map, |ctx| again(ctx, eq_start, |ctx| anchor_floats(ctx, pb, p, y0, 0.0, excl_mark, &label)));
                    if let Some(k) = relaid.lines.iter().position(|l| l.start == resume) {
                        (float_rects, pl, rel) = (rects, relaid, relaid_rel);
                        (laid_page, laid_width) = (pb.number, pb.col_w());
                        anchor_y = y0;
                        n = pl.lines.len();
                        l0 = k;
                    }
                }
            }
            _ if l0 < n => pb.advance(block, body_top),
            _ => {}
        }
    }
    // A mark that stayed with its page break keeps its space after on that page too: the next page
    // starts with the next paragraph (Word, compatibility modes 12 to 15).
    let space_after = if pb.after_break_mark { 0.0 } else { pl.rp.space_after };
    pb.y += space_after;
    pb.prev = Some((pl.rp.style.clone(), pl.rp.contextual_spacing, space_after, pl.rp.after_auto, label.is_some()));
}

/// Whether `range` of `p`'s text holds nothing but zero-width markers (bookmarks, field and
/// control boundaries…), or nothing at all: a line of them takes no room after a page break.
fn only_markers(p: &Paragraph, range: std::ops::Range<usize>) -> bool {
    let Some(text) = p.text.get(range.clone()) else { return false };
    text.char_indices().all(|(i, c)| c == wordcraft_doc::para::OBJ && p.object_at(range.start + i).is_some_and(InlineObject::is_marker))
}

/// The outline of the body for an Outline view pass.
struct OutlinePass {
    view: outline::OutlineView,
    levels: Vec<u8>,
    hidden: Vec<bool>,
}

impl OutlinePass {
    fn new(doc: &Document, view: outline::OutlineView) -> Self {
        let levels = outline::levels(doc);
        let hidden = outline::hidden(&levels, &view);
        OutlinePass { view, levels, hidden }
    }
}

/// A body paragraph in Outline view (one long sheet: no page or column breaks, no floats
/// wrapping): indented by its level behind its symbol; hidden when collapsed or deeper than Show
/// Level; body text cut to its first line with Show First Line Only.
fn place_outline_para(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, o: &OutlinePass) {
    // List numbers count the hidden paragraphs too.
    let label = ctx.next_label(p);
    if o.hidden.get(block).copied().unwrap_or(false) {
        return;
    }
    let level = o.levels.get(block).copied().unwrap_or(outline::BODY);
    let indent = outline::indent(&o.levels, block);
    let x = pb.col_x() + indent + outline::SYMBOL_W;
    let width = (pb.col_w() - indent - outline::SYMBOL_W).max(72.0);
    let q = outline::outline_para(ctx.doc, p, o.view.plain);
    ctx.fields.page = pb.number;
    let pl = ctx.para_labelled(&q, width, None, &[], label);
    pb.y += pl.rp.space_before.max(0.0);
    let n = pl.lines.len();
    let l1 = if o.view.first_line_only && level >= outline::BODY { n.min(1) } else { n };
    let y = pb.y + pl.lines.first().map(|f| f.top.max(0.0)).unwrap_or(0.0);
    let mut items = Vec::new();
    push_para(&mut items, StoryRef::Body, &[block as u32], &pl, 0, l1, x, y, width);
    let at = ObjFrame {
        page: Some(PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror }),
        col: (x, width),
        para_y: y,
        table_chr: None,
        pinned: None,
    };
    let (behind, front) = place_objects(ctx, StoryRef::Body, &[block as u32], &q, &pl, (0, l1), (x, y), &at, &HashMap::new(), 0);
    items.extend(front);
    let children = level < outline::BODY && outline::has_children(&o.levels, block);
    // Collapsed, or with something under it hidden by Show Level: the wavy line says so.
    let collapsed = children && o.hidden.get(block + 1).copied().unwrap_or(false);
    items.extend(outline::symbol_items(&pl, x, y, level, children, collapsed));
    if let Some(pg) = pb.page() {
        for (i, it) in behind.into_iter().enumerate() {
            pg.items.insert(i.min(pg.items.len()), it);
        }
        pg.items.extend(items);
    }
    if let (Some(f), Some(l)) = (pl.lines.first(), l1.checked_sub(1).and_then(|k| pl.lines.get(k))) {
        pb.y = y + (l.top + l.height - f.top);
    }
    pb.y += pl.rp.space_after.max(0.0);
}

/// How far a paragraph with mirrored indents (`w:mirrorIndents`) moves right on page `number`:
/// its left and right indents are the inside and outside ones, which swap sides on even pages.
fn mirror_shift(rp: &wordcraft_doc::resolve::ResolvedPara, number: u32, web: bool) -> f32 {
    if !rp.mirror_indents || web || !number.is_multiple_of(2) {
        return 0.0;
    }
    let (left, right) = rp.visual_indents();
    right - left
}

/// The space after body paragraph `block` (resolved as `rp`) that is really left below it: none
/// when contextual spacing or automatic spacing between list items drops it before the next
/// paragraph. Same tests as the spacing between paragraphs in `place_para_in_flow`: contextual
/// spacing removes this paragraph's space after only when this one asks for it, and the next
/// counts as a list item when it gets a label.
fn space_after_kept(ctx: &Ctx, block: usize, rp: &wordcraft_doc::resolve::ResolvedPara, listed: bool) -> f32 {
    let after = rp.space_after.max(0.0);
    if after <= 0.0 {
        return 0.0;
    }
    if let Some(Block::Para(next)) = ctx.doc.body.get(block + 1).map(|b| &**b) {
        let nrp = ctx.doc.styles.resolve_para(&next.props);
        if nrp.style == rp.style && rp.contextual_spacing {
            return 0.0;
        }
        if listed && rp.after_auto && ctx.has_label(next) {
            return 0.0;
        }
    }
    after
}

/// The room the paragraphs after `block` need on the page for keep with next: enough of the
/// next one to start there — its first line, two of its lines with widow/orphan control (it
/// doesn't leave a single line at the bottom of a page; it moves over whole instead), or all
/// of it under keep lines together — or all of it and what it keeps with in turn when it is
/// kept with the next too (a chain moves together in Word). Word carries a kept heading over
/// when the paragraph after it cannot start on the page (#966).
fn next_first_line(ctx: &mut Ctx, block: usize, width: f32, depth: usize) -> f32 {
    match ctx.doc.body.get(block + 1).map(|b| &**b) {
        Some(Block::Para(p)) => {
            // Don't advance list counters for a lookahead: lay out without the label.
            let rp = ctx.doc.styles.resolve_para(&p.props);
            let env = para::ParaEnv {
                doc: ctx.doc,
                width,
                label: None,
                fields: &ctx.fields,
                show_hidden: ctx.opts.show_hidden,
                field_codes: ctx.opts.field_codes,
                hide_deleted: ctx.opts.hides_deleted(StoryRef::Body),
                table: None,
                proofing: None,
                typing_word: None,
                numeral: ctx.opts.numeral,
                exclusions: &[],
                eq_number: ctx.eq_count,
                read_line_spacing: if ctx.opts.view == ViewMode::Read { ctx.opts.read_line_spacing } else { None },
                grid: ctx.grid,
                fit_objects: false,
                fitted: &[],
            };
            let _ = rp;
            let pl = para::layout_para(p, &env);
            if pl.rp.keep_next && depth < 8 {
                pl.rp.space_before + pl.height + pl.rp.space_after + next_first_line(ctx, block + 1, width, depth + 1)
            } else {
                // The room this one needs to start on the page with the kept paragraph before
                // it (#966).
                let to = |k: usize| match (pl.lines.first(), pl.lines.get(k)) {
                    (Some(first), Some(last)) => last.top + last.height - first.top,
                    _ => 0.0,
                };
                pl.rp.space_before
                    + if pl.rp.keep_lines {
                        pl.height
                    } else if pl.rp.widow_control && pl.lines.len() >= 2 {
                        to(1)
                    } else {
                        to(0)
                    }
            }
        }
        Some(Block::Table(_)) => 24.0,
        None => 0.0,
    }
}

fn row_keeps_next(row: &table::RowLayout) -> bool {
    let Some(items) = row.cells.first().and_then(|cell| row.items.get(cell.items.clone())) else { return false };
    let Some((depth, story)) = items
        .iter()
        .filter_map(|item| match item {
            Placed::Cell { table, story, .. } => Some((table.0.len() + 3, *story)),
            _ => None,
        })
        .min_by_key(|(depth, _)| *depth)
    else {
        return false;
    };
    // Word takes the row's keep flag from the first paragraph of its first cell.
    items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { path, story: own, para, .. } if path.0.len() == depth && path.0.last() == Some(&0) && *own == story => {
                Some(para.rp.keep_next)
            }
            _ => None,
        })
        .unwrap_or(false)
}

fn row_can_start(row: &wordcraft_doc::Row, laid: &table::RowLayout, room: f32, page_height: f32) -> bool {
    if laid.height <= room + 0.01 {
        return true;
    }
    if row.props.header || row.props.cant_split || (row.props.height_rule == HeightRule::Exact && row.props.height.is_some_and(|h| h > 0.0)) {
        return false;
    }
    if row.props.height_rule == HeightRule::AtLeast && row.props.height.is_some_and(|h| h > room + 0.01 && h <= page_height + 0.01) {
        return false;
    }
    table::split_row(laid, room).is_some()
}

fn place_table(ctx: &mut Ctx, pb: &mut PageBuilder, t: &wordcraft_doc::Table, block: usize, body_top: f32) {
    pb.prev = None;
    let width = pb.col_w();
    let mut tl = table::layout_table(ctx, StoryRef::Body, t, &[block as u32], width, 1);
    if let Some(f) = t.props.float.filter(|_| !pb.web) {
        // Before Word 2013 layout (compatibility mode 15) an offset places the first cell's text,
        // so the edge sits a cell margin further out.
        let legacy = if ctx.doc.settings.compat_mode < 15 { table::first_cell_left_margin(ctx, t) } else { 0.0 };
        let h: f32 = tl.rows.iter().map(|r| r.height).sum();
        if h <= pb.uncapped_bottom() - pb.top + 0.01 {
            place_floating_table(ctx, pb, &tl, &f, legacy, block, body_top);
            return;
        }
        // Too tall for a page: Word lets it run across pages like a table in the text, from its
        // own left edge.
        tl.x = float_table_x(pb, tl.width, &f, legacy, ctx.mirror) - pb.col_x();
    }
    // A table in the text flow starts below the floating objects in its way.
    below_floats(ctx, pb, pb.col_x() + tl.x, pb.col_x() + tl.x + tl.width);
    let header_rows: Vec<usize> = (0..t.rows.len()).take_while(|r| t.rows.get(*r).is_some_and(|row| row.props.header)).collect();
    // Header rows are re-placed on every page a table spills onto, so a table with many header
    // rows that each spill onto their own page is O(header rows × pages) placed items — a few KB
    // of document can otherwise fill memory. Once this many header rows have been repeated across
    // the whole table, later pages carry none; pagination still finishes. Far above any real
    // table (e.g. 3 header rows over 30 000 pages).
    const MAX_HEADER_REPEATS: usize = 200_000;
    let mut header_repeats = 0usize;
    // Resolve kept chains once; nested paragraphs cannot keep their enclosing row.
    let mut kept: Vec<(usize, f32)> = (0..tl.rows.len()).map(|i| (i, 0.0)).collect();
    for i in (0..tl.rows.len().saturating_sub(1)).rev() {
        let Some(row) = tl.rows.get(i) else { continue };
        if i < header_rows.len() || !row_keeps_next(row) {
            continue;
        }
        let Some((end, height)) = kept.get(i + 1).copied() else { continue };
        if let Some(group) = kept.get_mut(i) {
            *group = (end, row.height + height);
        }
    }
    let repeated_height: f32 = header_rows.iter().filter_map(|i| tl.rows.get(*i)).map(|r| r.height).sum();
    let place = |pb: &mut PageBuilder, row: &table::RowLayout| {
        let (x, y) = (pb.col_x() + tl.x, pb.y);
        if let Some(pg) = pb.page() {
            for it in &row.items {
                let mut it = it.clone();
                it.translate(x, y);
                pg.items.push(it);
            }
        }
        pb.y += row.height;
        pb.content_bottom = pb.content_bottom.max(pb.y);
    };
    // Measured in Word: an at-least row's minimum height is reserved, not split. Where less room
    // than that is left on a page that already holds something, the row starts the next page
    // whole; a row taller than a page then splits from the top of that page.
    let short_of_min = |ri: usize, room: f32| {
        t.rows.get(ri).is_some_and(|r| r.props.height_rule == HeightRule::AtLeast && r.props.height.is_some_and(|h| h > room + 0.01))
    };
    // Like Word, repeated header rows don't stay alone at the bottom of a page: when the first row
    // after them can't start under them there, the table starts on the next page.
    let first = header_rows.len();
    if !header_rows.is_empty()
        && !pb.at_top()
        && let Some(row) = tl.rows.get(first)
    {
        let room = pb.bottom - pb.y - tl.rows.iter().take(first).map(|r| r.height).sum::<f32>();
        let splittable = !t.rows.get(first).is_some_and(|r| r.props.cant_split);
        if row.height > room + 0.01 && (short_of_min(first, room) || !(splittable && table::split_row(row, room).is_some())) {
            pb.advance(block, body_top);
        }
    }
    // For the move-on rule, the table's own header rows at the top of a page leave it empty.
    let table_at_top = pb.at_top();
    for (ri, row) in tl.rows.iter().enumerate() {
        // Word never splits a row of an exact height: it moves on whole.
        let splittable = !header_rows.contains(&ri)
            && !t
                .rows
                .get(ri)
                .is_some_and(|r| r.props.cant_split || (r.props.height_rule == HeightRule::Exact && r.props.height.is_some_and(|h| h > 0.0)));
        let mut rest: Option<table::RowLayout> = None;
        // On a new page holding only the repeated header rows.
        let mut fresh = false;
        // Nothing but this table's header rows above it on a page it started at the top of.
        let under_top_headers = ri == first && table_at_top;
        // Each pass places the part of the row that fits, then breaks the page.
        for _ in 0..1000 {
            let cur = rest.as_ref().unwrap_or(row);
            // A fitting kept row follows the next row when that row cannot start below it.
            // Move only the prefix's start; an oversized follower cannot separate its kept rows.
            let defer_kept = pb.y + cur.height <= pb.bottom + 0.01
                && !pb.web
                && !pb.at_top()
                && !fresh
                && rest.is_none()
                && !ri.checked_sub(1).and_then(|prev| kept.get(prev)).is_some_and(|&(end, _)| end >= ri)
                && kept.get(ri).is_some_and(|&(end, height)| {
                    if end == ri {
                        return false;
                    }
                    let (Some(next), Some(laid)) = (t.rows.get(end), tl.rows.get(end)) else { return false };
                    let page_height = pb.bottom - body_top - repeated_height;
                    height <= page_height + 0.01 && !row_can_start(next, laid, pb.bottom - pb.y - height, page_height)
                });
            if pb.y + cur.height <= pb.bottom + 0.01 && !defer_kept {
                break;
            }
            // An at-least row with less room than its minimum moves on whole, unless nothing
            // is above it on this page (moving on would gain nothing).
            let room = pb.bottom - pb.y;
            let moves_on = rest.is_none() && !pb.at_top() && !fresh && !under_top_headers && short_of_min(ri, room);
            let split = if splittable && !moves_on && !defer_kept { table::split_row(cur, room) } else { None };
            match split {
                Some((a, b)) => {
                    place(pb, &a);
                    table_notes(ctx, pb, &a.items);
                    rest = Some(b);
                }
                // Like Word, a row that does not fit on an empty page, or under the header rows
                // repeated there, is placed anyway and runs into the bottom margin.
                None if pb.at_top() || fresh => break,
                None => {}
            }
            pb.advance(block, body_top);
            // Repeat header rows, up to this table's cap.
            if !header_rows.contains(&ri) && header_repeats < MAX_HEADER_REPEATS {
                for hr in &header_rows {
                    if header_repeats >= MAX_HEADER_REPEATS {
                        break;
                    }
                    if let Some(h) = tl.rows.get(*hr) {
                        place(pb, h);
                        header_repeats += 1;
                    }
                }
            }
            fresh = true;
        }
        let last = rest.as_ref().unwrap_or(row);
        place(pb, last);
        table_notes(ctx, pb, &last.items);
    }
}

/// Footnotes referenced in the cell lines of a placed table row (or part of one) go to the
/// bottom of the page, like those of body paragraphs; notes that don't fit there continue on
/// the next page (see [`PageBuilder::flush_notes`]).
fn table_notes(ctx: &mut Ctx, pb: &mut PageBuilder, items: &[Placed]) {
    if pb.web {
        return;
    }
    let mut ids: Vec<u32> = Vec::new();
    for it in items {
        let Placed::Lines { story: StoryRef::Body, para, l0, l1, .. } = it else { continue };
        let (Some(a), Some(b)) = (para.lines.get(*l0), para.lines.get(l1.saturating_sub(1))) else { continue };
        for (ci, id) in &para.notes {
            if *ci >= a.c0 && *ci < b.c1 && !ids.contains(id) && !pb.notes.iter().any(|n| n.id == *id) {
                ids.push(*id);
            }
        }
    }
    let before = pb.notes_h();
    for id in ids {
        let Some(part) = ctx.doc.parts.get(&id).filter(|p| p.kind == wordcraft_doc::PartKind::Footnote) else { continue };
        let blocks = part.blocks.clone();
        let BoxLayout { items, height: h, .. } = layout_box(ctx, StoryRef::Part(id), &blocks, &[], pb.sect.text_width(), BoxContext::Other, 0, None);
        pb.notes.push(NoteBox { id, items, h, cont: false });
    }
    pb.bottom -= pb.notes_h() - before;
}

/// Header tables can displace a non-overlapping body table. Page-positioned drawings keep
/// their own wrapping rules, so only table areas are retained from the header layout.
fn header_table_areas(ctx: &mut Ctx, pb: &PageBuilder) -> Vec<Rect> {
    if pb.web || ctx.opts.view == ViewMode::Read {
        return Vec::new();
    }
    let Some(page) = pb.pages.last() else { return Vec::new() };
    let first = pb.pages.iter().rev().nth(1).is_none_or(|prev| prev.section != page.section);
    let key = (page.section, page.number, first);
    if let Some(areas) = ctx.header_tables.get(&key) {
        return areas.clone();
    }
    let sections = ctx.doc.sections();
    let Some((_, sect)) = sections.get(page.section) else { return Vec::new() };
    if sect.text_direction.is_turned() {
        return Vec::new();
    }
    let set = linked(&sections, page.section, |s| s.headers);
    let id = if sect.title_page && first {
        set.first
    } else if ctx.doc.settings.even_odd_headers && page.number.is_multiple_of(2) {
        set.even.or(set.default)
    } else {
        set.default
    };
    let Some(id) = id else { return Vec::new() };
    let Some(part) = ctx.doc.parts.get(&id) else { return Vec::new() };
    let blocks = part.blocks.clone();
    let x = sect.margin_left + sect.gutter;
    let frame = PageFrame { sect, origin: (x, sect.header), number: page.number, mirror: ctx.mirror };
    let base = defining_width(&sections, page.section, |s| s.headers, Some(id));
    let counters = std::mem::take(&mut ctx.counters);
    let boxes = std::mem::take(&mut ctx.boxes);
    let chains = std::mem::take(&mut ctx.chains);
    let fields = ctx.fields.clone();
    let grid = std::mem::take(&mut ctx.grid);
    let track = std::mem::replace(&mut ctx.track_totals, false);
    ctx.fields.page = page.number;
    let mut areas = again(ctx, 0, |ctx| {
        with_pct_base(ctx, base, |ctx| layout_box(ctx, StoryRef::Part(id), &blocks, &[], sect.text_width(), BoxContext::Other, 0, Some(frame)))
            .table_areas
    });
    ctx.counters = counters;
    ctx.boxes = boxes;
    ctx.chains = chains;
    ctx.fields = fields;
    ctx.grid = grid;
    ctx.track_totals = track;
    for r in &mut areas {
        r.x += x;
        r.y += sect.header;
    }
    ctx.header_tables.insert(key, areas.clone());
    areas
}

/// Place floating table `tl` (`w:tblpPr`): at its own position, which takes no room in the text
/// flow; the text after it wraps around it.
fn place_floating_table(ctx: &mut Ctx, pb: &mut PageBuilder, tl: &table::TableLayout, f: &TableFloat, legacy: f32, block: usize, body_top: f32) {
    let h: f32 = tl.rows.iter().map(|r| r.height).sum::<f32>().clamp(0.0, 100_000.0);
    let w = tl.width.clamp(1.0, 100_000.0);
    let [dl, dt, dr, db] = f.dist_from_text();
    for attempt in 0..2 {
        let s = pb.sect;
        let h_area = float_table_h_area(pb, f, ctx.mirror);
        // The text a table anchored to it stands in: where the next paragraph starts.
        let v_area = match f.v_rel {
            Anchor::Page => (0.0, s.page_h),
            Anchor::Margin => (s.margin_top, s.text_height()),
            _ => (pb.y, 0.0),
        };
        let x = float_table_x(pb, w, f, legacy, ctx.mirror);
        let mut obstacles = if f.overlap { Vec::new() } else { header_table_areas(ctx, pb) };
        obstacles.extend_from_slice(&pb.float_tables);
        let r = clear_of_float_tables(Rect::new(x, align_in(v_area, f.y, h, f.v_align), w, h), f, legacy, h_area.0, &obstacles);
        // Like Word, a table that doesn't fit below the text before it moves to the next page with
        // its anchor: one placed relative to the paragraph when it would run off the page, one
        // placed on the page or margin when it would also cover the text already on the page.
        let moves = if f.v_rel == Anchor::Paragraph {
            r.bottom() > pb.bottom + 0.01
        } else {
            let covers_text = pb.page().is_some_and(|page| page.items.iter().any(|item| placed_intersects(item, &r)));
            covers_text && pb.y + h > pb.bottom + 0.01
        };
        if attempt == 0 && moves && !pb.at_top() {
            pb.advance(block, body_top);
            continue;
        }
        if let Some(pg) = pb.page() {
            let mut y = r.y;
            for row in &tl.rows {
                for it in &row.items {
                    let mut it = it.clone();
                    it.translate(r.x, y);
                    pg.items.push(it);
                }
                y += row.height;
            }
        }
        let area = r.inset(-dl, -dt, -dr, -db);
        pb.excl.push((area, false, None, WrapText::BothSides));
        pb.float_tables.push(area);
        return;
    }
}

fn placed_intersects(item: &Placed, rect: &Rect) -> bool {
    match item {
        Placed::Lines { para, l0, l1, x, y, .. } => para.lines.get(*l0..*l1).is_some_and(|lines| {
            lines.iter().enumerate().any(|(offset, line)| {
                if para.label.is_some() && *l0 + offset == 0 {
                    return Rect::new(x + line.left, y + line.top, line.right - line.left, line.height).intersects(rect);
                }
                para.clusters.get(line.c0..line.c1).is_some_and(|clusters| {
                    clusters.iter().enumerate().any(|(index, cluster)| {
                        if !matches!(
                            cluster.kind,
                            crate::para::ClKind::Text | crate::para::ClKind::Space | crate::para::ClKind::Tab | crate::para::ClKind::Object(_)
                        ) {
                            return false;
                        }
                        let ci = line.c0 + index;
                        let Some(left) = line.cl_left(ci) else { return false };
                        let Some(right) = line.cl_right(ci) else { return false };
                        Rect::new(x + left, y + line.top, right - left, line.height).intersects(rect)
                    })
                })
            })
        }),
        Placed::Fill { rect: bounds, .. }
        | Placed::Image { rect: bounds, .. }
        | Placed::Shape { rect: bounds, .. }
        | Placed::Graphic { rect: bounds, .. }
        | Placed::Cell { rect: bounds, .. }
        | Placed::Object { rect: bounds, .. }
        | Placed::Clip { rect: bounds, .. } => bounds.intersects(rect),
        Placed::Rule { x0, y0, x1, y1, .. } => Rect::new(x0.min(*x1), y0.min(*y1), (x1 - x0).abs(), (y1 - y0).abs()).intersects(rect),
    }
}

/// The page `pb` is filling, for placing floating objects on it (`mirror`: [`Ctx::mirror`]).
fn page_frame<'a>(pb: &PageBuilder<'a>, mirror: bool) -> PageFrame<'a> {
    PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror }
}

/// The area a floating table's `x` is measured in: its start and width on the page, as laid out
/// before an even page is mirrored (see [`float_rect`]).
fn float_table_h_area(pb: &PageBuilder, f: &TableFloat, mirror: bool) -> (f32, f32) {
    let s = pb.sect;
    let (left, _, shift) = page_frame(pb, mirror).sides();
    match f.h_rel {
        Anchor::Page => (-shift, s.page_w),
        Anchor::Margin => (left - shift, s.text_width()),
        _ => (pb.col_x(), pb.col_w()),
    }
}

/// A floating table's left edge on the page (`legacy`: the first cell's left margin before Word
/// 2013 layout, when an offset places the cell's text).
fn float_table_x(pb: &PageBuilder, w: f32, f: &TableFloat, legacy: f32, mirror: bool) -> f32 {
    let align = PageFrame::h_align(Some(&page_frame(pb, mirror)), f.h_align);
    let x = align_in(float_table_h_area(pb, f, mirror), f.x, w, align);
    if f.h_align.is_none() { x - legacy } else { x }
}

/// Move `pb.y` below the floating objects in the way of something spanning `x0..x1` that
/// starts there (a table in the text flow doesn't wrap around them).
fn below_floats(ctx: &mut Ctx, pb: &mut PageBuilder, x0: f32, x1: f32) {
    pb.y = below(&body_wraps(ctx, pb), pb.y, x0, x1);
}

/// `y`, moved below the wrap areas in `excl` in the way of something spanning `x0..x1` that
/// starts there.
fn below(excl: &[WrapArea], mut y: f32, x0: f32, x1: f32) -> f32 {
    for _ in 0..64 {
        let in_the_way = excl.iter().filter(|(r, ..)| r.y <= y + 0.01 && r.bottom() > y && r.x < x1 && r.right() > x0);
        let Some(bottom) = in_the_way.map(|(r, ..)| r.bottom()).reduce(f32::max) else { break };
        y = bottom;
    }
    y
}

/// Section `si`'s headers (or footers, as `get` picks), with those it leaves out inherited from
/// the sections before it (Word's "link to previous").
fn linked(sections: &[(usize, &SectionProps)], si: usize, get: fn(&SectionProps) -> HeaderSet) -> HeaderSet {
    let mut set = sections.get(si).map(|(_, s)| get(s)).unwrap_or_default();
    for (_, s) in sections.iter().take(si).rev() {
        if set.default.is_some() && set.first.is_some() && set.even.is_some() {
            break;
        }
        let o = get(s);
        set.default = set.default.or(o.default);
        set.first = set.first.or(o.first);
        set.even = set.even.or(o.even);
    }
    set
}

/// Whether a header, a footer or a text box shows a `PAGEREF` field, which they resolve from the
/// paginated body (see [`bookmark_pages`]).
fn headers_footers_refer_to_pages(doc: &Document) -> bool {
    use wordcraft_doc::PartKind;
    doc.parts.iter().filter(|(_, part)| matches!(part.kind, PartKind::Header | PartKind::Footer | PartKind::TextBox)).any(|(id, _)| {
        let story = StoryRef::Part(*id);
        doc.para_paths(story)
            .iter()
            .filter_map(|path| doc.para(story, path))
            .any(|p| p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if fields::field_name(instr) == "PAGEREF")))
    })
}

/// Where each bookmark in the body starts: the number and number format of the page it is placed
/// on (with two of the same name, the first in the document).
fn bookmark_pages(doc: &Document, pages: &[Page], sections: &[(usize, &SectionProps)]) -> fields::BookmarkPages {
    // Each paragraph's bookmarks: name, offset, and its line in the paragraph layout last looked
    // up (the pieces of a paragraph usually share one).
    let mut wanted: HashMap<Path, Vec<(String, usize, Option<(usize, usize)>)>> = HashMap::new();
    let mut seen = HashSet::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        if !p.objects.iter().any(|o| matches!(o, InlineObject::BookmarkStart { .. })) {
            continue;
        }
        for (off, o) in p.object_offsets().into_iter().zip(&p.objects) {
            if let InlineObject::BookmarkStart { name } = o
                && seen.insert(name.as_str())
            {
                wanted.entry(path.clone()).or_default().push((name.clone(), off, None));
            }
        }
    }
    let mut out = fields::BookmarkPages::new();
    for page in pages {
        if wanted.is_empty() {
            break;
        }
        let format = sections.get(page.section).map(|(_, s)| s.page_num_format).unwrap_or_default();
        for it in &page.items {
            let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } = it else { continue };
            let Some(marks) = wanted.get_mut(path) else { continue };
            let layout = Arc::as_ptr(para) as usize;
            marks.retain_mut(|(name, off, line)| {
                let li = match *line {
                    Some((k, li)) if k == layout => li,
                    _ => {
                        let li = para.line_of(*off);
                        *line = Some((layout, li));
                        li
                    }
                };
                if !(*l0..*l1).contains(&li) {
                    return true;
                }
                out.insert(std::mem::take(name), (page.number, format));
                false
            });
            if marks.is_empty() {
                wanted.remove(path);
            }
        }
    }
    out
}

/// A key for caches standing for `marks` (0 for none).
fn marks_key(marks: &fields::BookmarkPages) -> u64 {
    if marks.is_empty() {
        return 0;
    }
    let mut v: Vec<_> = marks.iter().collect();
    v.sort_by(|a, b| a.0.cmp(b.0));
    hash_of(&v)
}

fn headers_footers(ctx: &mut Ctx, pages: &mut [Page], sections: &[(usize, &SectionProps)], totals: &Totals) {
    let total = pages.len() as u32;
    let even_odd = ctx.doc.settings.even_odd_headers;
    let styles = styleref::headers_use_styleref(ctx.doc).then(|| ctx.cache.style_index(ctx.doc));
    let mut first_of_section: HashSet<usize> = HashSet::new();
    let mut seen = HashSet::new();
    for (i, p) in pages.iter().enumerate() {
        if seen.insert(p.section) {
            first_of_section.insert(i);
        }
    }
    for (i, page) in pages.iter_mut().enumerate() {
        let Some((_, sect)) = sections.get(page.section) else { continue };
        // Inherit header/footer references from earlier sections (Word's "link to previous").
        let pick = |get: fn(&SectionProps) -> HeaderSet| -> Option<u32> {
            let set = linked(sections, page.section, get);
            if sect.title_page && first_of_section.contains(&i) {
                return set.first;
            }
            if even_odd && page.number % 2 == 0 {
                return set.even.or(set.default);
            }
            set.default
        };
        let hid = pick(|s| s.headers);
        let fid = pick(|s| s.footers);
        page.hf_slot = if sect.title_page && first_of_section.contains(&i) {
            HfSlot::First
        } else if even_odd && page.number % 2 == 0 {
            HfSlot::Even
        } else {
            HfSlot::Default
        };
        ctx.fields.page = page.number;
        ctx.fields.page_format = sect.page_num_format;
        ctx.fields.pages = total;
        ctx.fields.section_pages = totals.section(page.section);
        ctx.fields.section = page.section as u32 + 1;
        // STYLEREF: the body paragraphs on this page.
        ctx.fields.styleref = styles.as_ref().map(|ix| {
            let paths = page.items.iter().filter_map(|it| match it {
                Placed::Lines { story: StoryRef::Body, path, .. } => Some(path),
                _ => None,
            });
            (ix.clone(), ix.page_at(paths, page.first_block))
        });
        let w = sect.text_width();
        let x = sect.margin_left + sect.gutter;
        if let Some(id) = hid
            && let Some(part) = ctx.doc.parts.get(&id)
        {
            let blocks = part.blocks.clone();
            let frame = PageFrame { sect, origin: (x, sect.header), number: page.number, mirror: ctx.mirror };
            let base = defining_width(sections, page.section, |s| s.headers, hid);
            let (mut items, behind, front) = with_pin(ctx, id, sect, page.number, |ctx| {
                with_pct_base(ctx, base, |c| layout_box(c, StoryRef::Part(id), &blocks, &[], w, BoxContext::Other, 0, Some(frame)).items)
            });
            for it in &mut items {
                it.translate(x, sect.header);
            }
            page.header = behind.into_iter().chain(items).chain(front).collect();
            page.header_story = Some(id);
        }
        if let Some(id) = fid
            && let Some(part) = ctx.doc.parts.get(&id)
        {
            let blocks = part.blocks.clone();
            let base = defining_width(sections, page.section, |s| s.footers, fid);
            // The footer's height fixes where it sits, which page-relative floats need.
            let (h, ..) = with_pin(ctx, id, sect, page.number, |ctx| {
                with_pct_base(ctx, base, |c| layout_box(c, StoryRef::Part(id), &blocks, &[], w, BoxContext::Other, 0, None).height)
            });
            let frame = PageFrame { sect, origin: (x, sect.page_h - sect.footer - h), number: page.number, mirror: ctx.mirror };
            let (mut items, behind, front) = with_pin(ctx, id, sect, page.number, |ctx| {
                with_pct_base(ctx, base, |c| layout_box(c, StoryRef::Part(id), &blocks, &[], w, BoxContext::Other, 0, Some(frame)).items)
            });
            for it in &mut items {
                it.translate(x, sect.page_h - sect.footer - h);
            }
            page.footer = behind.into_iter().chain(items).chain(front).collect();
            page.footer_story = Some(id);
        }
    }
    ctx.fields.styleref = None;
}

/// Mirrored margins: even pages have the inside margin (and gutter) on the right, so their
/// header, text, notes and footer move to start at the outside margin. Page borders, measured
/// from the page edge, stay.
fn mirror_even_pages(pages: &mut [Page], sections: &[(usize, &SectionProps)]) {
    for page in pages.iter_mut().filter(|p| p.number % 2 == 0) {
        let Some((_, s)) = sections.get(page.section) else { continue };
        let dx = s.margin_right - s.margin_left - s.gutter;
        if dx.abs() < 0.001 || !dx.is_finite() {
            continue;
        }
        let borders = page_border_rules(s).len();
        for it in page.items.iter_mut().skip(borders).chain(page.header.iter_mut()).chain(page.footer.iter_mut()) {
            it.translate(dx, 0.0);
        }
        page.body.x += dx;
    }
}

/// Is an object floating (not laid out inline)?
pub fn is_floating(o: &InlineObject) -> bool {
    o.is_floating()
}

/// Milliseconds since an arbitrary epoch (monotonic on native; the browser's clock on the web).
pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }
    // `Instant::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(target_arch = "wasm32")]
    {
        let ms = js_sys::Date::now();
        if ms.is_finite() { ms } else { 0.0 }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_autospace;
#[cfg(test)]
mod tests_bidi;
#[cfg(test)]
mod tests_complex;
#[cfg(test)]
mod tests_grid;
#[cfg(test)]
mod tests_ligature;
#[cfg(test)]
mod tests_numeral;
#[cfg(test)]
mod tests_pages;
#[cfg(test)]
mod tests_styleref;
#[cfg(test)]
mod tests_textbox;
#[cfg(test)]
mod tests_vertical;
