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
        } else if !web && pb.take_back_break_page() {
            // The mark stayed with the page before (parked in `place_para`), so the page the
            // break made is empty: this continuous section owns it, and when it asks for a
            // first-page header, that page shows it, like Word.
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
            // Its text (a group: its text boxes', each where the group's turn carries it; the
            // text itself stays upright), unless the box budget says no (a box inside itself,
            // too deep, too many).
            let centre = Point::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            let boxes: Vec<(u32, Rect)> = match obj {
                InlineObject::Group { .. } => group_members(obj, rect)
                    .into_iter()
                    .filter_map(|(r, c)| {
                        c.text_box()
                            .filter(|id| ctx.doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox))
                            .map(|id| (id, carried(r, float.spin(), centre)))
                    })
                    .collect(),
                _ => text_box.map(|id| (id, rect)).into_iter().collect(),
            };
            for (id, rect) in boxes {
                let items = textbox::box_items(ctx, id, rect, depth);
                let layer = if float.wrap == Wrap::BehindText { &mut behind } else { &mut front };
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
    if !pb.web
        && pb.after_break_mark
        && !pb.next_starts_page
        && p.section.is_some()
        && p.text.is_empty()
        && p.objects.is_empty()
        && !ctx.has_label(p)
        && let Some(page) = pb.pages.len().checked_sub(2)
    {
        // Word keeps an empty section mark with the preceding page break even when the section
        // that follows is continuous: the mark's (empty) line stays on the page before, so the
        // page the break made is empty for the continuous section start below to take over.
        let width = pb.col_w();
        let pl = ctx.para_labelled(p, width, None, &[], None);
        let prev = &mut pb.pages[page];
        let y = prev
            .items
            .iter()
            .filter(|it| matches!(it, Placed::Lines { .. } | Placed::Cell { .. }))
            .map(v_span)
            .map(|(_, bottom)| bottom)
            .fold(prev.body.y, f32::max);
        push_para(&mut prev.items, StoryRef::Body, &[block as u32], &pl, 0, pl.lines.len(), prev.body.x, y, width);
        return;
    }
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

#[cfg(test)]
mod tests;
