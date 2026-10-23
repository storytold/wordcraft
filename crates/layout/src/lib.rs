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
mod work;

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

mod boxes;
mod cache;
mod ctx;
mod floats;
mod flow;
mod headers;
mod notes;
mod page;
mod pass;
mod placed;
mod table_flow;

#[cfg(test)]
use boxes::{BOX_INSET_X, BOX_INSET_Y};
use boxes::{BoxInput, BoxLayout, Edges, fit_box, layout_box, push_para};
pub use cache::LayoutCache;
use cache::{MAX_TOTAL_PASSES, Totals};
#[cfg(test)]
use ctx::picture_outline;
use ctx::{Ctx, Pass};
use floats::{EarlyFloats, MAX_REWRAP_PASSES, merge_early};
pub use floats::{is_floating, shape_figure_alt};
#[cfg(test)]
use floats::{left_out_objects, wrap_area};
#[cfg(test)]
use headers::MAX_HEADER_ITEMS;
pub use notes::DECOR_STORY;
pub(crate) use notes::NOTE_SEP_W;
use notes::{MAX_NOTE_PASSES, footnote_pages};
#[cfg(test)]
use notes::{NOTE_SEP, note_numbers};
use page::item_bottom;
use pass::layout_pass;
pub use pass::now_ms;
pub use placed::{HfSlot, Placed, turn_point, turn_rect, unturn_point};

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
/// the saved file all see the original digits. Only ASCII digits are substituted: Arabic-Indic
/// (U+0660–U+0669) and Extended Arabic-Indic digits (U+06F0–U+06F9, Persian/Urdu ۰۱۲۳…) always
/// show as typed, so fonts that ligate them (e.g. the end-of-ayah mark U+06DD) keep working.
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
        if hindi { if c.is_ascii_digit() { char::from_u32(0x660 + (c as u32 - '0' as u32)).unwrap_or(c) } else { c } } else { c }
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
    /// Document file name for FILENAME fields ([`FieldCtx::filename`]).
    pub filename: Option<Arc<str>>,
    /// Document full path for FILENAME \p fields ([`FieldCtx::filepath`]).
    pub filepath: Option<Arc<str>>,
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
    /// The top-level body tables placed on this page as floating tables (`w:tblpPr`): a click
    /// outside one doesn't land in its cells ([`DocLayout::hit`]).
    pub float_tables: Vec<Path>,
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
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    opts.show_hidden.hash(&mut h);
    opts.hide_deleted.hash(&mut h);
    opts.hide_marked.hash(&mut h);
    opts.deletions_in_balloons.hash(&mut h);
    opts.proofing.hash(&mut h);
    opts.numeral.hash(&mut h);
    opts.field_codes.hash(&mut h);
    opts.proof.hash(&mut h);
    wordcraft_proof::dictionary_generation().hash(&mut h);
    hyphenation_key(doc).hash(&mut h);
    outline.hash(&mut h);
    opts.filename.hash(&mut h);
    opts.filepath.hash(&mut h);
    h.finish()
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

/// A layout pass: finished, or to be done again with the page totals it found, or with the
/// floats placed above their anchors it found ([`EarlyFloats`]).
enum Pass {
    Done(DocLayout),
    Again(Totals),
    Rewrap(EarlyFloats),
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
    /// Each body float's latest placement: on which page and with what wrap area, when it is
    /// above its anchor paragraph with text before it ([`EarlyFloats`]).
    floats_above: HashMap<(usize, usize), Option<(usize, WrapArea)>>,
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
    // Deepest a floating picture or shape that takes part in the text flow reaches, so the box
    // (a table cell especially) grows to hold it instead of letting it hang past the box's own
    // bottom edge (#3108). Behind-text and in-front-of-text objects don't: a page-anchored logo or
    // watermark behind a cell's text must not stretch its row (see `align_h` in table.rs).
    let mut float_objects_bottom = 0.0f32;
    let mut table_areas = Vec::new();
    let mut page_table_wraps = Vec::new();
    let mut y = 0.0f32;
    let mut prev_after = 0.0f32;
    let mut prev_after_auto = false;
    let mut prev_listed = false;
    let mut prev_style: Option<(String, bool)> = None;
    let mut last_top = 0.0f32;
    // The frame the paragraphs before this one are in, and where that frame's last one ended.
    let mut frame_run: Option<(FrameKey, f32)> = None;
    // Word has no frames in a text box: a framed paragraph there (its style's frame, usually)
    // runs with the box's text (#2357).
    let frames = !matches!(story, StoryRef::Part(id) if ctx.doc.parts.get(&id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox));
    // The bottom of the last paragraph's lines.
    let mut prev_end = 0.0f32;
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
                    // Paragraphs that share a frame's placement are one frame: Word stacks them
                    // inside it, one under the other (#2357). Each takes the frame's own place,
                    // so on their own they would all be drawn over each other; the first takes
                    // the frame's place and the rest follow it down.
                    let key = frame_key(&fr);
                    let at = match &frame_run {
                        Some((k, bottom)) if *k == key => *bottom,
                        _ => y,
                    };
                    let r = float_rect(frame, (0.0, width), at, fw, h, &float);
                    push_para(&mut items, story, &path, &pf, 0, pf.lines.len(), r.x, r.y, fw);
                    frame_run = Some((key, r.y + h));
                    excl.extend(wrap_area(r, &float, None));
                    float_tables.push(r);
                    continue;
                }
                // A paragraph outside the frame ends the run of paragraphs sharing it.
                frame_run = None;
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
                // Bordered paragraphs: room for their borders, those of a group joined in one box.
                let cell = table.map(|t| &t.para);
                let (above, below) = (joined(ctx.doc, blocks, i, i.checked_sub(1), cell), joined(ctx.doc, blocks, i, Some(i + 1), cell));
                let mut edges = Edges::own(&pl, 0, pl.lines.len());
                if above {
                    edges.top = 0.0;
                }
                if below {
                    edges.bottom = 0.0;
                }
                let lead = pl.lines.first().map_or(0.0, |l| l.top.max(0.0)) + edges.top;
                if above {
                    edges.up = (y + lead - prev_end).max(0.0);
                }
                prev_end = push_para(&mut items, story, &path, &pl, 0, pl.lines.len(), 0.0, y + lead, width, edges);
                // Pictures, shapes and text boxes: drawn, their areas and text boxes' text.
                if !p.objects.is_empty() {
                    let at = ObjFrame { page: frame, col: (0.0, width), para_y: y, table_chr: table.map(|t| &t.chr), pinned: Some(&pinned) };
                    let (back, front) = place_objects(ctx, story, &path, p, &pl, (0, pl.lines.len()), (0.0, y + lead), &at, &floats, depth);
                    for placed in back.iter().chain(front.iter()) {
                        if let Placed::Object { rect, spin, wrap, .. } = placed
                            && !matches!(wrap, Wrap::Inline | Wrap::BehindText | Wrap::InFrontOfText)
                        {
                            float_objects_bottom = float_objects_bottom.max(spin.bounds(*rect).bottom());
                        }
                    }
                    let at = behind.min(items.len());
                    behind += back.len();
                    items.splice(at..at, back);
                    items.extend(front);
                }
                // Automatic space after: none after the last paragraph of a cell or box.
                let after = if pl.rp.after_auto && i + 1 == blocks.len() { 0.0 } else { pl.rp.space_after };
                y += edges.top + pl.height + edges.bottom + after;
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
    let floats_bottom = float_tables.iter().map(|r| r.bottom()).fold(float_objects_bottom, f32::max);
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

/// What makes two paragraphs share a text frame: the frame's own placement and size (ECMA-376
/// §17.3.1.11). The space around the frame is not part of it, so a run of paragraphs whose
/// frames differ only in that is still one frame.
#[derive(Clone, PartialEq)]
struct FrameKey {
    x: f32,
    y: f32,
    w: Option<f32>,
    h: Option<f32>,
    h_exact: bool,
    x_align: Option<String>,
    y_align: Option<String>,
    h_anchor: String,
    v_anchor: String,
    wrap: String,
}

/// The key of a text frame's placement.
fn frame_key(fr: &wordcraft_doc::props::TextFrame) -> FrameKey {
    FrameKey {
        x: fr.x,
        y: fr.y,
        w: fr.w,
        h: fr.h,
        h_exact: fr.h_exact,
        x_align: fr.x_align.clone(),
        y_align: fr.y_align.clone(),
        h_anchor: fr.h_anchor.clone(),
        v_anchor: fr.v_anchor.clone(),
        wrap: fr.wrap.clone(),
    }
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

/// Word joins neighbouring paragraphs with the same borders and indents into one box: the top
/// border goes above the first of them only, the bottom one below the last, and the shading and
/// side borders run on through the space between them (`table`: the cell's paragraph properties).
fn border_group(doc: &Document, a: &Paragraph, b: &Paragraph, table: Option<&wordcraft_doc::props::ParaProps>) -> bool {
    let (a, b) = (doc.styles.resolve_para_in(&a.props, table), doc.styles.resolve_para_in(&b.props, table));
    a.borders.is_some() && a.borders == b.borders && a.indent_left == b.indent_left && a.indent_right == b.indent_right
}

/// Whether paragraph `i` of `blocks` and the one at `j` are of one border group.
fn joined(doc: &Document, blocks: &Blocks, i: usize, j: Option<usize>, table: Option<&wordcraft_doc::props::ParaProps>) -> bool {
    match (blocks.get(i).map(|b| &**b), j.and_then(|j| blocks.get(j)).map(|b| &**b)) {
        (Some(Block::Para(a)), Some(Block::Para(b))) => border_group(doc, a, b, table),
        _ => false,
    }
}

/// The room a paragraph's top or bottom border takes in the text flow: its distance from the
/// text plus its width (Word).
fn border_room(b: Option<Border>) -> f32 {
    b.filter(Border::is_visible).map_or(0.0, |b| b.space + b.width)
}

/// How a piece of a paragraph meets the paragraphs around it: the room taken by its top and
/// bottom borders (0: none drawn there — the paragraph goes on from or into one of its border
/// group, or it is split there) and how far its shading and side borders reach up to join the
/// paragraph above in its group.
#[derive(Clone, Copy, Default)]
struct Edges {
    top: f32,
    bottom: f32,
    up: f32,
}

impl Edges {
    /// A paragraph on its own: its top and bottom borders at its ends.
    fn own(pl: &ParaLayout, l0: usize, l1: usize) -> Edges {
        let b = pl.rp.borders.unwrap_or_default();
        Edges { top: if l0 == 0 { border_room(b.top) } else { 0.0 }, bottom: if l1 >= pl.lines.len() { border_room(b.bottom) } else { 0.0 }, up: 0.0 }
    }
}

/// Paragraph lines plus their shading and borders; returns the bottom of the lines. The
/// shading fills the box out to the borders, the room they take included (Word).
#[allow(clippy::too_many_arguments)]
fn push_para(
    items: &mut Vec<Placed>,
    story: StoryRef,
    path: &[u32],
    pl: &Arc<ParaLayout>,
    l0: usize,
    l1: usize,
    x: f32,
    y: f32,
    width: f32,
    e: Edges,
) -> f32 {
    let (Some(first), Some(last)) = (pl.lines.get(l0), l1.checked_sub(1).and_then(|k| pl.lines.get(k))) else {
        return y;
    };
    let h = last.top + last.height - first.top;
    // Indents are logical: a right-to-left paragraph's start indent (and hanging indent) is on the right.
    let start = pl.rp.indent_left.min(pl.rp.indent_left + pl.rp.indent_first);
    let (li, ri) = if pl.rp.bidi { (pl.rp.indent_right, start) } else { (start, pl.rp.indent_right) };
    let left = x + li;
    let right = x + width - ri;
    let (top, bottom) = (y - e.top - e.up, y + h + e.bottom);
    if let Some(c) = pl.rp.shading {
        let pad = pl.rp.borders.as_ref().map(|b| b.left.map(|l| l.space).unwrap_or(4.0)).unwrap_or(0.0);
        items.push(Placed::Fill { rect: Rect::new(left - pad, top, right - left + pad * 2.0, bottom - top), color: c });
    }
    items.push(Placed::Lines { story, path: Path(path.to_vec()), para: pl.clone(), l0, l1, x, y, turn: TextDirection::Horizontal });
    if let Some(b) = &pl.rp.borders {
        let sp = |o: &Option<Border>| o.map(|b| b.space).unwrap_or(0.0);
        let (lx, rx) = (left - sp(&b.left), right + sp(&b.right));
        // The top and bottom lines lie beyond their distance from the text.
        if let Some(t) = b.top.filter(|t| e.top > 0.0 && t.is_visible()) {
            let ty = y - t.space - t.width / 2.0;
            items.push(Placed::Rule { x0: lx, y0: ty, x1: rx, y1: ty, border: t });
        }
        if let Some(t) = b.bottom.filter(|t| e.bottom > 0.0 && t.is_visible()) {
            let by = y + h + t.space + t.width / 2.0;
            items.push(Placed::Rule { x0: lx, y0: by, x1: rx, y1: by, border: t });
        }
        if let Some(t) = b.left.filter(Border::is_visible) {
            items.push(Placed::Rule { x0: lx, y0: top, x1: lx, y1: bottom, border: t });
        }
        if let Some(t) = b.right.filter(Border::is_visible) {
            items.push(Placed::Rule { x0: rx, y0: top, x1: rx, y1: bottom, border: t });
        }
    }
    y + h
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
    /// Where the last paragraph's lines ended: page count, column and y.
    para_end: Option<(usize, usize, f32)>,
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
    /// The wrap areas pages start with ([`EarlyFloats`]) ...
    early: &'a EarlyFloats,
    /// ... and how many of the current page's `excl` they are (its first ones).
    seeded: usize,
    /// No layout pass follows for floats found above their anchors: each is kept clear of the
    /// text laid out before it ([`settle_early`]).
    settle: bool,
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
        // Floats placed above their anchors further down this page are known from its start.
        let early = self.early;
        self.excl.extend(early.get(&self.pages.len()).into_iter().flatten().map(|(_, _, a)| a.clone()));
        self.seeded = self.excl.len();
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
    let (mut early, mut rewraps) = (EarlyFloats::new(), 0);
    let mut l = loop {
        match layout_pass(doc, cache, opts, &note_pages, &totals, passes < MAX_TOTAL_PASSES, (&early, rewraps < MAX_REWRAP_PASSES)) {
            Pass::Again(t) => {
                totals = t;
                passes += 1;
            }
            Pass::Rewrap(found) => {
                merge_early(&mut early, found);
                rewraps += 1;
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

#[cfg(test)]
thread_local! {
    /// How many wrap relays (#1374) the layouts on this thread made: tests check none happen
    /// without something new to wrap around.
    static WRAP_RELAYS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
