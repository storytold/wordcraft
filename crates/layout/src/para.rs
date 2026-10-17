//! Paragraph layout: resolve runs, shape them into clusters, break lines (first-fit, as word
//! processors do), place tabs, list labels and alignment.

use std::collections::VecDeque;
use std::sync::Arc;

use wordcraft_doc::math::MathJc;
use wordcraft_doc::numbering::{Level, LevelSuffix};
use wordcraft_doc::para::{COLUMN_BREAK, InlineObject, LINE_BREAK, NoteKind, OBJ, PAGE_BREAK, RubyAlign, SOFT_HYPHEN, WrapText};
use wordcraft_doc::props::{Align, CharProps, LineSpacing, ParaProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::resolve::{ResolvedChar, ResolvedPara};
use wordcraft_doc::{Document, Paragraph};
use wordcraft_fonts::FaceRef;

use crate::NumeralMode;
use crate::fields::{FIELD_SHADING, FieldCtx, field_code_text, field_text_on};

/// How a run set in one of Word's symbol fonts (Symbol, Wingdings, Wingdings 2/3, Webdings)
/// shows its characters. Those fonts are proprietary and usually missing: a substitute has other
/// glyphs, or none, at their codes, so each character is drawn as its Unicode equivalent
/// ([`wordcraft_doc::symbol_fonts`]) from a face that has it. The text itself is unchanged.
#[derive(Clone, Copy)]
struct SymbolRun<'a> {
    family: &'a str,
    /// The symbol font itself is installed (`face` is it).
    installed: bool,
    face: FaceRef,
}

impl<'a> SymbolRun<'a> {
    fn of(family: &'a str, r: &wordcraft_fonts::word::Resolved) -> Option<Self> {
        wordcraft_doc::symbol_fonts::is_symbol_font(family).then_some(SymbolRun { family, installed: !r.substituted, face: r.face })
    }

    /// The character drawn for `c`.
    fn show(&self, c: char) -> char {
        if self.installed {
            if self.face.covers(c) {
                return c;
            }
            // A plain 8-bit code in the installed font: its glyph sits at the private-use code.
            if let Some(p) = (0x20..=0xFF).contains(&u32::from(c)).then(|| char::from_u32(0xF000 + u32::from(c))).flatten()
                && self.face.covers(p)
            {
                return p;
            }
        }
        let Some(u) = wordcraft_doc::symbol_fonts::symbol_char(self.family, c) else { return c };
        // A few list bullets have common look-alikes in far more fonts: use one when no face
        // has the exact character.
        let alt = match u {
            '\u{26AB}' => Some('\u{25CF}'),
            '\u{25FC}' => Some('\u{25A0}'),
            '\u{2B9A}' => Some('\u{27A2}'),
            _ => None,
        };
        let has = |ch: char| self.face.covers(ch) || wordcraft_fonts::FontDb::global().fallback_for(ch, self.face.id()).is_some();
        match alt {
            Some(a) if !has(u) && has(a) => a,
            _ => u,
        }
    }
}

/// A face at a size with the formatting needed to draw it.
#[derive(Clone, Debug)]
pub struct StyleRun {
    pub face: FaceRef,
    pub size: f32,
    pub synth_bold: bool,
    pub synth_italic: bool,
    pub rc: Arc<ResolvedChar>,
    /// Line ascent/descent at this size, points.
    pub ascent: f32,
    pub descent: f32,
    /// Baseline shift (superscript etc.), points, positive up.
    pub shift: f32,
}

/// A positioned glyph inside its cluster.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub gid: u32,
    /// Offset from the cluster's x, points.
    pub dx: f32,
    pub dy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClKind {
    Text,
    Space,
    Tab,
    LineBreak,
    PageBreak,
    ColumnBreak,
    /// Inline object with its index in `Paragraph::objects`.
    Object(usize),
    /// Zero-width marker (bookmark, comment anchor) or hidden text.
    Marker,
}

/// The smallest unit of layout: a grapheme cluster (or object, tab, break).
#[derive(Clone, Debug)]
pub struct Cluster {
    pub start: usize,
    pub end: usize,
    pub adv: f32,
    pub kind: ClKind,
    /// Index into `ParaLayout::styles`.
    pub style: u32,
    /// Glyph range in `ParaLayout::glyphs`.
    pub g0: u32,
    pub g1: u32,
    /// A line may break after this cluster.
    pub break_after: bool,
    /// Height above the baseline for objects (images), points.
    pub obj_h: f32,
    /// Depth below the baseline for objects (equations), points.
    pub obj_d: f32,
    /// The cluster is a decimal separator (decimal tabs align on it).
    pub dot: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnd {
    /// Wrapped (or last line).
    Wrap,
    LineBreak,
    PageBreak,
    ColumnBreak,
    /// End of paragraph.
    Para,
}

/// A laid-out line. Coordinates are relative to the paragraph's content box: x from the
/// column's left edge, y from the top of the first line.
#[derive(Clone, Debug)]
pub struct Line {
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
    /// Cluster index range.
    pub c0: usize,
    pub c1: usize,
    /// x of each cluster in the line, plus the end x (len = c1 - c0 + 1).
    pub xs: Vec<f32>,
    /// For tab clusters: the leader to draw across them.
    pub leaders: Vec<(usize, TabLeader)>,
    pub end: LineEnd,
    /// Byte range of the paragraph covered by the line.
    pub start: usize,
    pub stop: usize,
    /// Left and right edges available to the line.
    pub left: f32,
    pub right: f32,
    /// The line ends at a hyphenation point: (style, glyph, advance) of the hyphen drawn there.
    pub hyphen: Option<(u32, u32, f32)>,
    /// The text continues the previous line's row on the far side of a floating object (same top
    /// and height): one row, two lines. Row counts (line numbers, keeping rows on one page) skip it.
    pub beside: bool,
    /// Bidirectional lines (right-to-left text, or any line of a right-to-left paragraph): each
    /// cluster's visual box, in cluster order (len = c1 - c0), after the Unicode Bidirectional
    /// Algorithm reordered the line. Empty for plain left-to-right lines, whose clusters sit at
    /// `xs`. Read cluster geometry through [`Line::cl_left`] and friends, which handle both.
    pub vis: Vec<VisCl>,
    /// The paragraph reads right to left (its start edge is on the right).
    pub rtl: bool,
}

/// Where a cluster of a bidirectional line is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisCl {
    /// Left edge and width, points (paragraph coordinates, like `Line::xs`).
    pub x: f32,
    pub w: f32,
    /// Embedding level after rule L1 of UAX #9 (odd = right to left).
    pub level: u8,
}

impl VisCl {
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }
    /// The edge a caret before this cluster sits at (right edge of right-to-left text).
    pub fn leading(&self) -> f32 {
        if self.is_rtl() { self.x + self.w } else { self.x }
    }
    /// The edge a caret after this cluster sits at.
    pub fn trailing(&self) -> f32 {
        if self.is_rtl() { self.x } else { self.x + self.w }
    }
}

/// Width of a paragraph mark's box, points: what a selection running past the paragraph's end
/// highlights, and how much room the mark takes for assistive technology.
pub const PARA_MARK_WIDTH: f32 = 5.0;

impl Line {
    /// Left edge of cluster `k` (a paragraph cluster index on this line).
    pub fn cl_left(&self, k: usize) -> Option<f32> {
        let i = k.checked_sub(self.c0)?;
        if self.vis.is_empty() { self.xs.get(i).copied() } else { self.vis.get(i).map(|v| v.x) }
    }
    /// Right edge of cluster `k`.
    pub fn cl_right(&self, k: usize) -> Option<f32> {
        let i = k.checked_sub(self.c0)?;
        if self.vis.is_empty() { self.xs.get(i + 1).copied() } else { self.vis.get(i).map(|v| v.x + v.w) }
    }
    /// Is cluster `k` right-to-left text?
    pub fn cl_rtl(&self, k: usize) -> bool {
        k.checked_sub(self.c0).and_then(|i| self.vis.get(i)).is_some_and(VisCl::is_rtl)
    }
    /// Caret x after the line's last cluster (its logical end), hyphen included.
    pub fn end_x(&self) -> f32 {
        let h = self.hyphen.map_or(0.0, |h| h.2);
        match self.vis.last() {
            Some(v) if v.is_rtl() => v.x - h,
            Some(v) => v.x + v.w + h,
            None => self.xs.last().copied().unwrap_or(self.left),
        }
    }
    /// x of the line's visual end for the paragraph's direction, where its paragraph mark goes:
    /// the right edge of the rightmost cluster in a left-to-right paragraph, the left edge of the
    /// leftmost in a right-to-left one. Differs from [`Line::end_x`] when the logically last text
    /// runs the other way (Hebrew ending a left-to-right paragraph, Latin ending a right-to-left one).
    pub fn visual_end_x(&self) -> f32 {
        let end = self.end_x();
        if self.rtl { self.vis.iter().map(|v| v.x).fold(end, f32::min) } else { self.vis.iter().map(|v| v.x + v.w).fold(end, f32::max) }
    }
    /// The box of the mark ending the line (its paragraph mark or line break) as its left edge and
    /// width: [`PARA_MARK_WIDTH`] at the line's visual end, past the text in a left-to-right
    /// paragraph and before it in a right-to-left one.
    pub fn mark_box(&self) -> (f32, f32) {
        let x = self.visual_end_x();
        (if self.rtl { x - PARA_MARK_WIDTH } else { x }, PARA_MARK_WIDTH)
    }
    /// x where the hyphen of a hyphenated line (advance `adv`) is drawn.
    pub fn hyphen_x(&self, adv: f32) -> f32 {
        match self.vis.last() {
            Some(v) if v.is_rtl() => v.x - adv,
            Some(v) => v.x + v.w,
            None => self.xs.last().copied().unwrap_or(0.0) - adv,
        }
    }
    /// Visual extents, left to right, of clusters `k0..k1`: one span on a left-to-right line,
    /// possibly several where bidirectional text splits a logical range.
    pub fn spans(&self, k0: usize, k1: usize) -> Vec<(f32, f32)> {
        let (k0, k1) = (k0.max(self.c0), k1.min(self.c1));
        if k0 >= k1 {
            return Vec::new();
        }
        if self.vis.is_empty() {
            return match (self.cl_left(k0), self.cl_left(k1).or_else(|| self.xs.last().copied())) {
                (Some(a), Some(b)) => vec![(a, b)],
                _ => Vec::new(),
            };
        }
        let mut boxes: Vec<(f32, f32)> =
            self.vis.get(k0 - self.c0..k1 - self.c0).unwrap_or(&[]).iter().filter(|v| v.w > 0.0).map(|v| (v.x, v.x + v.w)).collect();
        boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out: Vec<(f32, f32)> = Vec::new();
        for (a, b) in boxes {
            match out.last_mut() {
                Some(last) if a <= last.1 + 0.01 => last.1 = last.1.max(b),
                _ => out.push((a, b)),
            }
        }
        out
    }
}

/// The list label drawn on the first line.
#[derive(Clone, Debug)]
pub struct Label {
    pub text: String,
    /// Maximum line ascent of every shaped font in the label, including its leading.
    pub ascent: f32,
    /// Style of the label's last run (the suffix space and the bidi tests measure with it).
    pub style: u32,
    pub glyphs: Vec<Glyph>,
    /// Font runs over `glyphs`, in drawing order: font fallback can split a label ("1、" takes
    /// the digit from the Latin face and the comma from a CJK one), and each glyph id only means
    /// something in the face it was shaped with.
    pub runs: Vec<LabelRun>,
    pub x: f32,
    pub width: f32,
}

/// Ruby text laid out over its base text's cluster.
#[derive(Clone, Debug)]
pub struct RubyLayout {
    /// The base text's cluster (the one cluster standing for an [`InlineObject::Ruby`]).
    pub cluster: usize,
    /// Glyphs with their style (index into `ParaLayout::styles`); `dx` is from the cluster's left
    /// edge.
    pub glyphs: Vec<(u32, Glyph)>,
    /// The ruby text and each glyph's byte range in it.
    pub text: String,
    pub ranges: Vec<std::ops::Range<usize>>,
    /// The ruby's baseline above the base text's, points.
    pub raise: f32,
}

/// One font run of a list label.
#[derive(Clone, Debug)]
pub struct LabelRun {
    /// Index into `ParaLayout::styles`.
    pub style: u32,
    /// Glyph range in `Label::glyphs`.
    pub glyphs: std::ops::Range<usize>,
    /// The run's characters (for the PDF text layer).
    pub text: String,
}

/// A laid-out paragraph.
#[derive(Clone, Debug)]
pub struct ParaLayout {
    pub rp: ResolvedPara,
    pub styles: Vec<StyleRun>,
    pub glyphs: Vec<Glyph>,
    pub clusters: Vec<Cluster>,
    pub lines: Vec<Line>,
    pub label: Option<Label>,
    /// Sum of line heights (without space before/after).
    pub height: f32,
    pub text_len: usize,
    /// Paragraph contains fields whose text depends on the page.
    pub has_page_fields: bool,
    /// Note references (part id) in this paragraph, by cluster index.
    pub notes: Vec<(usize, u32)>,
    /// Proofing issues: (byte start, byte end, grammar?).
    pub issues: Vec<(usize, usize, bool)>,
    /// Drop cap: (clusters at the start that form it, lines it drops, width with its gap).
    pub drop_cap: Option<(usize, u8, f32)>,
    /// Clusters after which the line may break with a hyphen (soft hyphens, auto hyphenation), sorted.
    pub hyph_after: Vec<u32>,
    /// Bidi embedding level of each byte of the paragraph text (UAX #9, before the per-line
    /// rule L1). Empty when the paragraph is plain left-to-right text.
    pub bidi_levels: Vec<u8>,
    /// Width of the drop cap letter itself (without the gap to the text).
    pub drop_cap_w: f32,
    /// Laid-out equations by object index.
    pub maths: Vec<(usize, Arc<crate::math::MathLayout>)>,
    /// Ruby (Phonetic Guide) text over base-text clusters, in cluster order.
    pub rubies: Vec<RubyLayout>,
    /// Display equations (on lines of their own): cluster index and justification.
    pub displays: Vec<(usize, wordcraft_doc::math::MathJc)>,
    /// Byte ranges of the text laid out as nothing ([`left_out`]): hidden text that is not shown,
    /// tracked deletions in the final text. Resolved as laid out, table formatting included.
    pub left_out: Vec<std::ops::Range<usize>>,
    /// The text drawn by clusters that stand for an object (field results, note numbers,
    /// equations) rather than for the paragraph's own text, by cluster index, sorted, with each of
    /// the cluster's glyphs' byte range in that text (a ligature's glyph spans its letters).
    pub shown: Vec<(usize, String, Vec<std::ops::Range<usize>>)>,
    /// Shapes drawn around overstruck field text (enclosed characters, `EQ \o`), by cluster index.
    pub enclosures: Vec<(usize, Enclosed)>,
}

/// A shape drawn over a cluster (an enclosed character's circle, square, triangle or diamond),
/// relative to the cluster's left edge and baseline, in the cluster's text colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enclosed {
    pub kind: wordcraft_doc::para::ShapeKind,
    /// Left edge from the cluster's x; top above the baseline (up is positive), points.
    pub x: f32,
    pub top: f32,
    pub w: f32,
    pub h: f32,
    pub stroke: f32,
}

/// What a table style gives the text of a cell: its paragraph and run formatting, with the
/// cell's conditional formatting (header row, first column…) applied. It sits between document
/// defaults and the paragraph's style (`StyleSheet::resolve_para_in`, `resolve_char_in`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellText {
    pub para: ParaProps,
    pub chr: CharProps,
}

/// Inputs that change a paragraph's layout beyond its own content.
pub struct ParaEnv<'a> {
    pub doc: &'a Document,
    /// Column width, points.
    pub width: f32,
    pub label: Option<(String, Level)>,
    pub fields: &'a FieldCtx,
    pub show_hidden: bool,
    /// Show field codes instead of results ([`crate::LayoutOptions::field_codes`]).
    pub field_codes: bool,
    /// Leave tracked deletions out, like hidden text (the final, "No Markup" text).
    pub hide_deleted: bool,
    /// The table style's formatting for text in this paragraph's cell (`None` outside tables).
    pub table: Option<&'a CellText>,
    /// Check spelling (and grammar, as the options say) for squiggles; `None` = no squiggles.
    pub proofing: Option<crate::ProofOptions>,
    /// How digits look on screen (an unresolved `System` shows Western digits).
    pub numeral: NumeralMode,
    /// Areas text must flow around (floating objects), relative to the paragraph: x from the
    /// column's left edge, y from the top of the first line.
    pub exclusions: &'a [Exclusion],
    /// Automatic equation numbers used before this paragraph.
    pub eq_number: u32,
    pub read_line_spacing: Option<f32>,
    /// The section's document grid (`Grid::default()`: none).
    pub grid: Grid,
    /// Shrink inline drawings wider than the line to fit it: in text boxes, which don't clip what
    /// runs past their sides. Elsewhere such a drawing keeps its size and the page edge clips it.
    pub fit_objects: bool,
}

/// The section's document grid as layout applies it (`w:docGrid`, ECMA-376 §17.6.5).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Grid {
    /// Line pitch, points: line heights snap to multiples of it.
    pub line: Option<f32>,
    /// Character pitch, points: East Asian characters advance by it.
    pub chars: Option<f32>,
    /// The default font size the character pitch is based on, points.
    pub font_size: f32,
    /// `snapToChars`: all text snaps to the character grid, not just East Asian characters.
    pub snap_all: bool,
}

impl Grid {
    /// The grid of section `s` of `doc` (its default font size from the Normal style).
    pub fn of(doc: &Document, s: &wordcraft_doc::SectionProps) -> Grid {
        let Some(line) = s.grid_line_pitch() else { return Grid::default() };
        let size = doc.styles.resolve_char(Some("Normal"), &CharProps::default()).size;
        let font_size = if size.is_finite() { size.clamp(1.0, 1584.0) } else { 10.5 };
        let chars = s.grid_char_pitch(font_size);
        let snap_all = s.doc_grid.is_some_and(|g| g.kind == wordcraft_doc::section::DocGridType::SnapToChars);
        Grid { line: Some(line), chars, font_size, snap_all }
    }
    /// A cache key for paragraph layouts.
    pub fn key(&self) -> (u32, u32, u32, bool) {
        (self.line.map_or(0, f32::to_bits), self.chars.map_or(0, f32::to_bits), self.font_size.to_bits(), self.snap_all)
    }
    /// `h` rounded up to whole line pitches (unchanged without a line grid).
    pub fn snap_height(&self, h: f32) -> f32 {
        match self.line {
            Some(p) if h.is_finite() => ((h / p - 0.001).ceil().clamp(1.0, 10_000.0) * p).min(1e6),
            _ => h,
        }
    }
}

/// East Asian characters the character grid places on its pitch: CJK ideographs, kana, Hangul,
/// CJK punctuation and full-width forms.
pub fn is_east_asian(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF | 0x2E80..=0x2FDF | 0x2FF0..=0x303F | 0x3040..=0x30FF | 0x3100..=0x312F | 0x3130..=0x318F
        | 0x3190..=0x31FF | 0x3200..=0x4DBF | 0x4E00..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF01..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFF)
}

/// Character grid: East Asian characters (all text with `snapToChars`) take whole cells of the
/// pitch, the glyph centred in them. A character at the default size takes one cell, a larger one
/// as many as its size needs; Latin text stays proportional unless everything snaps.
fn snap_chars(b: &mut Builder, text: &str, grid: &Grid) {
    let Some(pitch) = grid.chars else { return };
    let base = grid.font_size.max(1.0);
    for c in b.clusters.iter_mut() {
        if !matches!(c.kind, ClKind::Text | ClKind::Space) || c.adv <= 0.0 {
            continue;
        }
        let Some(ch) = text.get(c.start..).and_then(|t| t.chars().next()) else { continue };
        let east_asian = is_east_asian(ch);
        if !(east_asian || (grid.snap_all && c.kind == ClKind::Text)) {
            continue;
        }
        let size = b.styles.get(c.style as usize).map_or(base, |st| st.size);
        let cells = if east_asian { (size / base).round() } else { (c.adv / pitch - 0.01).ceil() }.clamp(1.0, 64.0);
        let adv = cells * pitch;
        let shift = (adv - c.adv) / 2.0;
        for g in b.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
            g.dx += shift;
        }
        c.adv = adv;
    }
}

/// An area text wraps around.
#[derive(Clone, Debug, PartialEq)]
pub struct Exclusion {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
    /// Text only above and below (no text beside it).
    pub top_bottom: bool,
    /// Tight and Through wrapping: the outline text follows inside the area (relative to its
    /// top-left); `None` = the whole area.
    pub contour: Option<Arc<crate::contour::Contour>>,
    /// The sides of the object text may go on (Square, Tight, Through).
    pub side: WrapText,
}

struct Builder<'a> {
    env: &'a ParaEnv<'a>,
    spacing: LineSpacing,
    styles: Vec<StyleRun>,
    /// Hash of a style's format → (index in `styles`, small caps), see `style`.
    style_index: std::collections::HashMap<u64, Vec<(u32, bool)>>,
    glyphs: Vec<Glyph>,
    clusters: Vec<Cluster>,
    /// Bidi level per byte of the paragraph text (empty: all left to right).
    levels: Vec<u8>,
    /// Text drawn by clusters that stand for an object, by cluster index (see `ParaLayout::shown`).
    shown: Vec<(usize, String, Vec<std::ops::Range<usize>>)>,
}

/// What a piece of text is shaped as: one face, case, script formatting and direction.
#[derive(Clone, Copy)]
struct SegKey {
    face: Option<FaceRef>,
    small: bool,
    /// Formatted with the complex-script properties (Persian, Arabic, Hebrew…).
    complex: bool,
    /// Odd bidi level: shaped right to left.
    rtl: bool,
}

impl SegKey {
    fn same(&self, o: &SegKey) -> bool {
        self.face.map(|f| f.id()) == o.face.map(|f| f.id()) && self.small == o.small && self.complex == o.complex && self.rtl == o.rtl
    }
}

/// The first character of `c`'s capital (ß → S): the shaper's one-to-one mapping, and the
/// text read back for labels and field results, whose byte ranges can't grow.
fn upper_char(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

/// The character drawn for `c` in a run formatted `rc`: All Caps capitalises every letter,
/// Small Caps the lowercase ones (as the shaper draws them). The PDF text layer uses it so
/// the text read back is the text shown (#1473).
pub(crate) fn caps_char(rc: &ResolvedChar, c: char) -> char {
    if rc.caps || (rc.small_caps && c.is_lowercase()) { upper_char(c) } else { c }
}

/// Appends the text drawn for `c` in a run formatted `rc`: like [`caps_char`], but with a
/// letter's full capital (ß → SS), as body text draws it (#1581).
pub(crate) fn push_caps(rc: &ResolvedChar, c: char, out: &mut String) {
    if rc.caps || (rc.small_caps && c.is_lowercase()) { out.extend(c.to_uppercase()) } else { out.push(c) }
}

/// Whether `c`'s capital is more than one character (ß → SS), so All Caps draws more
/// characters than the document holds.
fn caps_expands(c: char) -> bool {
    c.to_uppercase().nth(1).is_some()
}

/// Combining marks and joiners (bidi classes NSM, BN: harakat, ZWNJ, ZWJ) belong to the
/// character before them: same font, same shaping run, so Arabic joining isn't broken.
fn is_mark(c: char) -> bool {
    use unicode_bidi::BidiClass::{BN, NSM};
    matches!(unicode_bidi::bidi_class(c), NSM | BN) && !c.is_control()
}

/// A digit Word's Numeral modes substitute: ASCII or Arabic-Indic (U+0660–U+0669).
fn is_numeral_digit(c: char) -> bool {
    c.is_ascii_digit() || ('\u{660}'..='\u{669}').contains(&c)
}

/// Characters skipped when looking for the text around a number: digits, spaces and the
/// Arabic decimal/thousands separators (٫ ٬).
fn is_number_inside(c: char) -> bool {
    is_numeral_digit(c) || ('\u{6f0}'..='\u{6f9}').contains(&c) || c == ' ' || c == '\t' || c == '\u{66b}' || c == '\u{66c}'
}

/// Which digits of run `text` sit in right-to-left surroundings (Word › Numeral › Context), by
/// byte offset. European numbers take an even bidi level, so a digit's own level says little;
/// the nearest neighbour on either side that isn't part of the number (digits, spaces,
/// separators) decides. One pass each way, so long runs of digits and spaces stay linear.
/// Offsets past `levels` count as left to right.
fn context_digit_dirs(text: &str, levels: &[u8], base: usize) -> Vec<bool> {
    let odd = |o: usize| levels.get(base.saturating_add(o)).is_some_and(|l| l % 2 == 1);
    let mut out = vec![false; text.len()];
    let mut left_rtl = false;
    for (o, c) in text.char_indices() {
        if !is_number_inside(c) {
            left_rtl = odd(o);
        } else if is_numeral_digit(c)
            && let Some(v) = out.get_mut(o)
        {
            *v = odd(o) || left_rtl;
        }
    }
    let mut right_rtl = false;
    for (o, c) in text.char_indices().rev() {
        if !is_number_inside(c) {
            right_rtl = odd(o);
        } else if right_rtl
            && is_numeral_digit(c)
            && let Some(v) = out.get_mut(o)
        {
            *v = true;
        }
    }
    out
}

/// Map a byte range of numeral-substituted display text back to the original text.
///
/// `map_starts[i]` is the display byte offset of the `i`th char and `orig_of[i]` its original
/// byte offset (`orig_of` ends with the original length as a sentinel). Substitution is
/// 1 char → 1 char, so both ends land on char boundaries; anything else falls back to the
/// end of the original text instead of panicking or slicing mid-character.
pub(crate) fn remap_range(map_starts: &[usize], orig_of: &[usize], orig_len: usize, ms: usize, me: usize) -> (usize, usize) {
    let n = map_starts.len();
    let i = map_starts.partition_point(|&b| b < ms).min(n);
    let j = map_starts.partition_point(|&b| b < me).min(n);
    let s = orig_of.get(i).copied().unwrap_or(orig_len);
    let e = orig_of.get(j).copied().unwrap_or(orig_len);
    (s.min(e), s.max(e))
}

/// Bidi embedding levels per byte of `text` (UAX #9 rules up to I2), or empty when the
/// paragraph is left to right and has no right-to-left characters.
pub fn bidi_levels(text: &str, rtl_para: bool) -> Vec<u8> {
    if !rtl_para && !text.chars().any(wordcraft_fonts::is_rtl) {
        return Vec::new();
    }
    let level = if rtl_para { unicode_bidi::Level::rtl() } else { unicode_bidi::Level::ltr() };
    let info = unicode_bidi::ParagraphBidiInfo::new(text, Some(level));
    info.levels.iter().map(|l| l.number()).collect()
}

/// Rule L2 of UAX #9: the visual order (left to right) of items with these levels: from the
/// highest level down to the lowest odd one, reverse every run at that level or above.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(min_odd) = levels.iter().copied().filter(|l| l % 2 == 1).min() else { return order };
    let max = levels.iter().copied().max().unwrap_or(0);
    let level_at = |order: &[usize], i: usize| order.get(i).and_then(|&o| levels.get(o)).copied().unwrap_or(0);
    let mut lvl = max;
    while lvl >= min_odd {
        let mut i = 0;
        while i < order.len() {
            if level_at(&order, i) >= lvl {
                let mut j = i;
                while j < order.len() && level_at(&order, j) >= lvl {
                    j += 1;
                }
                if let Some(run) = order.get_mut(i..j) {
                    run.reverse();
                }
                i = j;
            } else {
                i += 1;
            }
        }
        if lvl == 0 {
            break;
        }
        lvl -= 1;
    }
    order
}

/// Text Word sets in the East Asian font: CJK ideographs, kana, bopomofo, CJK punctuation and
/// fullwidth forms, and Hangul.
fn east_asian(c: char) -> bool {
    crate::kinsoku::is_east_asian(c) || matches!(u32::from(c), 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF)
}

/// Word draws a font that isn't installed in Cambria when the document's font table classes it
/// as roman (measured in Word; Calibri otherwise, which our own substitutes already match).
const MISSING_ROMAN_FONT: &str = "Cambria";

/// The family to look `font` up as: [`MISSING_ROMAN_FONT`] for a missing roman font, else `font`.
fn lookup_family<'f>(doc: &Document, font: &'f str) -> &'f str {
    let db = wordcraft_fonts::FontDb::global();
    let roman = doc.settings.roman_fonts.iter().any(|f| f.eq_ignore_ascii_case(font));
    if roman && !db.has_family(font) && !db.has_family(wordcraft_fonts::word::charset_base_family(font)) { MISSING_ROMAN_FONT } else { font }
}

impl<'a> Builder<'a> {
    /// Displayed form of `c` at run-relative byte `at` (Word › Numeral): one char in, one
    /// char out. `rtl` is the shaping direction; in Context mode `context` (from
    /// [`context_digit_dirs`], empty otherwise) gives each digit's surroundings, since European
    /// numbers take an even bidi level.
    fn numeral_char(&self, context: &[bool], at: usize, c: char, rtl: bool) -> char {
        let dir = if !context.is_empty() && is_numeral_digit(c) { context.get(at).copied().unwrap_or(false) } else { rtl };
        self.env.numeral.map(c, dir)
    }

    fn font_metrics(&self, face: &wordcraft_fonts::FontFace) -> (f64, f64) {
        if matches!(self.spacing, LineSpacing::Exactly(_)) {
            return wordcraft_fonts::word::fixed_line_metrics(face);
        }
        wordcraft_fonts::word::line_metrics(face)
    }

    /// Style index for a resolved char in `face` (caps-scaled `size` override for small caps).
    fn style(&mut self, rc: &Arc<ResolvedChar>, face_override: Option<FaceRef>, small: bool) -> u32 {
        let r = wordcraft_fonts::word::resolve(lookup_family(self.env.doc, &rc.font), rc.bold, rc.italic);
        let face = face_override.unwrap_or(r.face);
        // The display list reads every run's format (link target, strike kind, underline colour,
        // baseline shift…) from its StyleRun, so runs share one only when their formats are equal.
        // Keyed by a hash of the commonly varying fields (no allocation: this runs for every shaped
        // piece), then confirmed by comparing whole formats within the bucket.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (face.id(), small, &rc.font, rc.size.to_bits(), rc.bold, rc.italic, rc.color, rc.highlight, rc.shading).hash(&mut h);
            (rc.underline_color, rc.strike, rc.double_strike, &rc.link, rc.ins, rc.del, rc.hidden).hash(&mut h);
            h.finish()
        };
        let bucket = self.style_index.entry(key).or_default();
        // Formats that never compare equal (a NaN from a hostile file) get their own styles; a capped
        // scan keeps such a bucket from turning quadratic.
        for &(i, s) in bucket.iter().rev().take(64) {
            if s == small
                && let Some(st) = self.styles.get(i as usize)
                && st.face.id() == face.id()
                && (Arc::ptr_eq(&st.rc, rc) || *st.rc == **rc)
            {
                return i;
            }
        }
        let size = rc.draw_size() * if small { 0.8 } else { 1.0 };
        let (a, d) = self.font_metrics(&face);
        let k = size as f64 / face.upem.max(1.0);
        // A fallback face (letters the requested font lacks) is emboldened or slanted when it has
        // no bold or italic of its own.
        let (synth_bold, synth_italic) = match face_override {
            None => (r.synth_bold, r.synth_italic),
            Some(f) => (rc.bold && f.get().weight < 600.0, rc.italic && !f.get().italic),
        };
        let st = StyleRun {
            face,
            size,
            synth_bold,
            synth_italic,
            rc: rc.clone(),
            ascent: (a * k) as f32,
            descent: (d * k) as f32,
            shift: rc.baseline_shift(),
        };
        // A `u32` index (like the glyph ranges) counts more distinct formats than a paragraph can
        // hold in memory, so every format keeps its own style.
        let i = u32::try_from(self.styles.len()).unwrap_or(u32::MAX);
        self.styles.push(st);
        self.style_index.entry(key).or_default().push((i, small));
        i
    }

    /// Shape `text` (a piece of the paragraph starting at byte `base`) in one style and append clusters.
    fn shape(&mut self, text: &str, base: usize, rc: &Arc<ResolvedChar>, kind_override: Option<ClKind>) {
        self.shape_with(text, base, rc, kind_override, true);
    }

    /// `para_text`: `text` is paragraph text at `base`, so the paragraph's bidi levels apply
    /// (false for field results and list labels, which are shaped on their own).
    fn shape_with(&mut self, text: &str, base: usize, rc: &Arc<ResolvedChar>, kind_override: Option<ClKind>, para_text: bool) {
        if text.is_empty() {
            return;
        }
        // Complex-script characters (Persian, Arabic…) use the run's complex-script font, size,
        // bold and italic.
        let rc_cs: Option<Arc<ResolvedChar>> = text.chars().any(|c| rc.uses_complex(c)).then(|| Arc::new(rc.complex()));
        let resolved = wordcraft_fonts::word::resolve(lookup_family(self.env.doc, &rc.font), rc.bold, rc.italic);
        let primary = resolved.face;
        // Symbol-font characters are laid out as their Unicode equivalents.
        let sym = SymbolRun::of(&rc.font, &resolved);
        let show = |c: char| sym.map_or(c, |m| m.show(c));
        let primary_cs = rc_cs.as_ref().map(|r| wordcraft_fonts::word::resolve(lookup_family(self.env.doc, &r.font), r.bold, r.italic).face);
        // Chinese, Japanese and Korean characters use the run's East Asian font (`w:eastAsia`) when
        // it is installed and has them.
        let primary_ea = rc
            .font_ea
            .as_ref()
            .filter(|f| !f.eq_ignore_ascii_case(&rc.font) && text.chars().any(east_asian))
            .map(|f| wordcraft_fonts::word::resolve(f, rc.bold, rc.italic))
            .filter(|r| !r.substituted)
            .map(|r| r.face);
        let use_levels = para_text && !self.levels.is_empty();
        let context = if self.env.numeral == NumeralMode::Context && use_levels { context_digit_dirs(text, &self.levels, base) } else { Vec::new() };
        // One fallback face for the run's East Asian characters: the first face in fallback order
        // covering every character the primary font lacks, so the run draws in a single CJK face
        // instead of mixing faces per character (e.g. a bundled Japanese Mincho for JIS-covered
        // ideographs beside a Simplified-Chinese sans for the rest). Whitespace and characters the
        // primary covers don't constrain the choice. `None` keeps the per-character fallback.
        let ea_face: Option<FaceRef> = {
            let ea: Vec<char> = text
                .chars()
                .filter(|c| crate::kinsoku::is_east_asian(*c) && !primary.covers(*c) && !c.is_whitespace() && !c.is_control() && *c != SOFT_HYPHEN)
                .collect();
            if ea.is_empty() {
                None
            } else {
                wordcraft_fonts::FontDb::global().fallback_for_east_asian(&ea, primary.id(), rc.bold, rc.italic).map(|f| FaceRef::of(&f))
            }
        };
        // Split by font coverage (fallback faces), small caps case, script formatting and direction.
        let mut seg_start = 0;
        let mut cur: Option<SegKey> = None;
        let mut segs: Vec<(usize, usize, SegKey)> = Vec::new();
        for (i, c) in text.char_indices() {
            let rtl = use_levels && self.levels.get(base + i).is_some_and(|l| l % 2 == 1);
            // A symbol-font character becomes its Unicode symbol (#776).
            let c = show(c);
            // Displayed form (Word › Numeral): one char in, one char out. Coverage and the
            // fallback face are picked for the displayed char, so Hindi digits land in an
            // Arabic-capable face and Western digits in a Latin one.
            let dc = self.numeral_char(&context, i, c, rtl);
            let mark = is_mark(c);
            let complex = match cur {
                Some(k) if mark => k.complex,
                _ => rc_cs.is_some() && rc.uses_complex(dc),
            };
            let prim = if complex { primary_cs.unwrap_or(primary) } else { primary };
            let face = match cur {
                Some(k) if mark => k.face,
                _ if !complex && east_asian(c) && primary_ea.is_some_and(|f| f.covers(c)) => primary_ea,
                _ if prim.covers(dc) || c.is_whitespace() || c.is_control() || c == SOFT_HYPHEN => None,
                // Stay in the fallback face the word started in while it covers the letters.
                Some(SegKey { face: Some(f), .. }) if f.covers(dc) => Some(f),
                // The run's unified East Asian face, when one face covers the whole run.
                _ if !complex && ea_face.is_some() && crate::kinsoku::is_east_asian(c) => ea_face,
                _ => {
                    let (bold, italic) = if complex { (rc.bold_cs, rc.italic_cs) } else { (rc.bold, rc.italic) };
                    wordcraft_fonts::FontDb::global().fallback_styled(dc, prim.id(), bold, italic).map(|f| FaceRef::of(&f))
                }
            };
            let small = rc.small_caps && !rc.caps && c.is_lowercase();
            let k = SegKey { face, small, complex, rtl };
            match cur {
                Some(p) if p.same(&k) => {}
                Some(p) => {
                    segs.push((seg_start, i, p));
                    seg_start = i;
                    cur = Some(k);
                }
                None => cur = Some(k),
            }
        }
        if let Some(p) = cur {
            segs.push((seg_start, text.len(), p));
        }
        let no_kern = [wordcraft_fonts::feature_value(b"kern", 0)];
        for (a, b, key) in segs {
            let Some(sub) = text.get(a..b) else { continue };
            let rc = match (&rc_cs, key.complex) {
                (Some(cs), true) => cs.clone(),
                _ => rc.clone(),
            };
            let rc = &rc;
            let small = key.small;
            let si = self.style(rc, key.face, small);
            let Some(st) = self.styles.get(si as usize).cloned() else { continue };
            let upper = rc.caps || small;
            // All Caps and Small Caps draw a letter's full capital, which can be several
            // characters (ß → SS, ﬀ → FF, ŉ → ʼN), as Word does (#1581). A symbol-font run
            // draws its codes as symbols, never as expanded capitals.
            let expands = sym.is_none() && upper && sub.chars().any(caps_expands);
            // Displayed text: numeral substitution is 1 char → 1 char and a capital may be
            // several; `map_starts` holds where each document char's displayed form starts and
            // `orig_of` maps it back to the char's original byte offset (+ a `sub.len()` sentinel).
            let substituted = expands || sub.char_indices().any(|(i, c)| self.numeral_char(&context, a + i, c, key.rtl) != c);
            let mut mapped = String::new();
            let mut map_starts: Vec<usize> = Vec::new();
            let mut orig_of: Vec<usize> = Vec::new();
            if substituted {
                mapped = String::with_capacity(sub.len().saturating_add(8));
                for (i, c) in sub.char_indices() {
                    map_starts.push(mapped.len());
                    orig_of.push(i);
                    let d = self.numeral_char(&context, a + i, c, key.rtl);
                    if expands {
                        mapped.extend(d.to_uppercase());
                    } else {
                        mapped.push(d);
                    }
                }
                orig_of.push(sub.len());
            }
            let shape_text = if substituted { mapped.as_str() } else { sub };
            let map = |c: char| {
                let d = show(c);
                if d != c {
                    d
                } else if upper {
                    upper_char(c)
                } else {
                    c
                }
            };
            // The shaper kerns by default; Word kerns only when Kerning for fonts covers the size.
            let features: &[wordcraft_fonts::Feature] = if rc.kerned() { &[] } else { &no_kern };
            let shaped = if use_levels {
                wordcraft_fonts::shape_run(&st.face, shape_text, features, map, key.rtl)
            } else {
                wordcraft_fonts::shape(&st.face, shape_text, features, map)
            };
            let k = st.size / st.face.upem.max(1.0) as f32;
            let hscale = rc.scale / 100.0;
            // Group glyphs by cluster byte offset; graphemes may span several shaper clusters.
            // Complex-script syllables the graphemes split (Myanmar vowel signs, stacked
            // consonants) stay one cluster: the caret never stops inside them.
            let mut bounds: Vec<usize> = wordcraft_doc::cluster::cluster_starts(shape_text);
            if expands {
                // The capitals drawn for one character (the two S of ß) are one cluster: the
                // caret, selection and line breaking never stop inside the character.
                bounds.retain(|b| map_starts.binary_search(b).is_ok());
            }
            let mut gi = 0usize;
            for (bi, &gs) in bounds.iter().enumerate() {
                let ge = bounds.get(bi + 1).copied().unwrap_or(shape_text.len());
                let g0 = self.glyphs.len() as u32;
                let mut adv = 0.0f32;
                while let Some(g) = shaped.get(gi) {
                    if g.cluster >= ge {
                        break;
                    }
                    self.glyphs.push(Glyph { gid: g.gid, dx: adv + g.x_offset as f32 * k * hscale, dy: g.y_offset as f32 * k });
                    adv += g.x_advance as f32 * k * hscale;
                    gi += 1;
                }
                let g1 = self.glyphs.len() as u32;
                let s = shape_text.get(gs..ge).unwrap_or("");
                let ch = s.chars().next().unwrap_or(' ');
                let kind = kind_override.unwrap_or(if ch == ' ' || ch == '\u{3000}' {
                    ClKind::Space
                } else if ch == SOFT_HYPHEN {
                    ClKind::Marker
                } else {
                    ClKind::Text
                });
                let adv = if kind == ClKind::Marker { 0.0 } else { adv + rc.spacing };
                // Clusters index the document text, never the displayed text.
                let (cs, ce) = if substituted { remap_range(&map_starts, &orig_of, sub.len(), gs, ge) } else { (gs, ge) };
                self.clusters.push(Cluster {
                    start: base + a + cs,
                    end: base + a + ce,
                    adv,
                    kind,
                    style: si,
                    g0,
                    g1: if kind == ClKind::Marker { g0 } else { g1 },
                    break_after: false,
                    obj_h: 0.0,
                    obj_d: 0.0,
                    dot: s == "." || s == ",",
                });
            }
        }
    }

    /// One cluster for a whole object with its display text (fields, note references).
    fn shape_atomic(&mut self, text: &str, start: usize, end: usize, rc: &Arc<ResolvedChar>) {
        let before = self.clusters.len();
        self.shape_with(text, start, rc, None, false);
        let added: Vec<Cluster> = self.clusters.drain(before..).collect();
        let Some(first) = added.first() else {
            let si = self.style(rc, None, false);
            let g = self.glyphs.len() as u32;
            self.clusters.push(Cluster {
                start,
                end,
                adv: 0.0,
                kind: ClKind::Marker,
                style: si,
                g0: g,
                g1: g,
                break_after: false,
                obj_h: 0.0,
                obj_d: 0.0,
                dot: false,
            });
            return;
        };
        // Merge into one cluster: rebase glyph dx onto the first cluster, and keep each glyph's
        // text (a ligature's glyph takes the letters of the clusters after it that have none).
        let mut x = 0.0;
        let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
        for c in &added {
            for g in self.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                g.dx += x;
            }
            x += c.adv;
            let (a, b) = (c.start.saturating_sub(start), c.end.saturating_sub(start));
            if c.g1 <= c.g0 {
                let from = ranges.last().map(|r| r.start);
                for r in ranges.iter_mut().rev().take_while(|r| Some(r.start) == from) {
                    r.end = b;
                }
            }
            ranges.extend((c.g0..c.g1).map(|_| a..b));
        }
        let g0 = first.g0;
        let style = first.style;
        let g1 = added.last().map(|c| c.g1).unwrap_or(g0);
        self.shown.push((self.clusters.len(), text.to_string(), ranges));
        self.clusters.push(Cluster { start, end, adv: x, kind: ClKind::Text, style, g0, g1, break_after: false, obj_h: 0.0, obj_d: 0.0, dot: false });
    }

    /// Shape `texts` on top of one another as one cluster for `start..end`: each aligned within
    /// the cluster (at least `width` wide) and raised `dy`. Returns the cluster's index.
    #[allow(clippy::too_many_arguments)]
    fn shape_overstruck(
        &mut self,
        texts: &[&str],
        start: usize,
        end: usize,
        rc: &Arc<ResolvedChar>,
        align: wordcraft_doc::eq::OAlign,
        width: f32,
        dy: f32,
    ) -> Option<usize> {
        use wordcraft_doc::eq::OAlign;
        let (c0, s0) = (self.clusters.len(), self.shown.len());
        for t in texts {
            self.shape_atomic(t, start, end, rc);
        }
        let added: Vec<Cluster> = self.clusters.drain(c0..).collect();
        let shown: Vec<(usize, String, Vec<std::ops::Range<usize>>)> = self.shown.drain(s0..).collect();
        let adv = added.iter().filter(|c| c.kind == ClKind::Text).map(|c| c.adv).fold(width.max(0.0), f32::max);
        let (mut text, mut ranges) = (String::new(), Vec::new());
        let (mut g0, mut g1) = (u32::MAX, 0u32);
        for (ci, t, r) in &shown {
            let Some(c) = added.get(ci.saturating_sub(c0)) else { continue };
            let dx = match align {
                OAlign::Left => 0.0,
                OAlign::Center => (adv - c.adv) / 2.0,
                OAlign::Right => adv - c.adv,
            };
            for g in self.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                g.dx += dx;
                g.dy += dy;
            }
            let off = text.len();
            ranges.extend(r.iter().map(|x| x.start + off..x.end + off));
            text.push_str(t);
            g0 = g0.min(c.g0);
            g1 = g1.max(c.g1);
        }
        let g = self.glyphs.len() as u32;
        let (g0, g1) = if g0 <= g1 { (g0, g1) } else { (g, g) };
        let style = match added.iter().find(|c| c.kind == ClKind::Text) {
            Some(c) => c.style,
            None => self.style(rc, None, false),
        };
        let k = self.clusters.len();
        if !text.is_empty() {
            self.shown.push((k, text, ranges));
        }
        self.clusters.push(Cluster { start, end, adv, kind: ClKind::Text, style, g0, g1, break_after: false, obj_h: 0.0, obj_d: 0.0, dot: false });
        Some(k)
    }
}

/// Lay out an `EQ \o` field (ECMA-376 Part 1 §17.16.5.20): its arguments drawn on top of one
/// another, as one cluster. An enclosure symbol among them (○ □ △ ◇, see
/// [`wordcraft_doc::eq::EncloseShape`]) is drawn as a shape around the text rather than as a
/// glyph, so it never depends on the font: "shrink text" scales the text to fit inside a shape
/// one em high, "enlarge symbol" grows the shape around text at its own size.
fn overstrike(
    b: &mut Builder,
    o: &wordcraft_doc::eq::Overstrike,
    instr: &str,
    start: usize,
    end: usize,
    rc: &Arc<ResolvedChar>,
    enclosures: &mut Vec<(usize, Enclosed)>,
) {
    use wordcraft_doc::eq::{EncloseShape, EncloseStyle, Enclosure};
    use wordcraft_doc::para::ShapeKind;
    let shapes: Vec<EncloseShape> = o.args.iter().filter_map(|a| EncloseShape::from_symbol(a)).collect();
    let texts: Vec<&str> = o.args.iter().filter(|a| !a.is_empty() && EncloseShape::from_symbol(a).is_none()).map(String::as_str).collect();
    let Some(shape) = shapes.first().copied() else {
        b.shape_overstruck(&texts, start, end, rc, o.align, 0.0, 0.0);
        return;
    };
    let style = Enclosure::parse(instr).map(|e| e.style).unwrap_or_default();
    let s = rc.size.clamp(1.0, 1638.0);
    // The widest text at the run's own size.
    let (c0, g0, s0) = (b.clusters.len(), b.glyphs.len(), b.shown.len());
    for t in &texts {
        b.shape_atomic(t, start, end, rc);
    }
    let w0 = b.clusters.get(c0..).map(|cs| cs.iter().map(|c| c.adv).fold(0.0f32, f32::max)).unwrap_or(0.0).max(0.0);
    b.clusters.truncate(c0);
    b.glyphs.truncate(g0);
    b.shown.truncate(s0);
    // The shape's box (`bw` × `bh`, centred `cy` above the baseline) and the text's size.
    let cy = s * 0.34;
    let (bw, bh, size) = match style {
        EncloseStyle::Shrink => {
            // Room for the text inside each shape: (height, width) as fractions of the box.
            let (fh, fw) = match shape {
                EncloseShape::Circle => (0.62, 0.7),
                EncloseShape::Square => (0.68, 0.78),
                EncloseShape::Triangle => (0.45, 0.5),
                EncloseShape::Diamond => (0.5, 0.55),
            };
            let bh = s * 1.05;
            let mut t = s * fh;
            if w0 > 0.0 && w0 * t / s > bh * fw {
                t = bh * fw * s / w0;
            }
            (bh, bh, t.max(s * 0.2))
        }
        EncloseStyle::Enlarge => {
            let (kh, kw) = match shape {
                EncloseShape::Circle => (1.4, 1.4),
                EncloseShape::Square => (1.3, 1.3),
                EncloseShape::Triangle => (1.75, 2.0),
                EncloseShape::Diamond => (1.7, 1.8),
            };
            let bh = s * kh;
            (bh.max(w0 * kw), bh, s)
        }
    };
    // A triangle's middle is low; the text sits there.
    let ty = if shape == EncloseShape::Triangle { cy - bh * 0.13 } else { cy };
    let trc = if (size - rc.size).abs() > 0.01 {
        let mut r = (**rc).clone();
        r.size = size;
        Arc::new(r)
    } else {
        rc.clone()
    };
    let pad = s * 0.05;
    let Some(k) = b.shape_overstruck(&texts, start, end, &trc, o.align, bw + 2.0 * pad, ty - size * 0.34) else { return };
    let adv = b.clusters.get(k).map(|c| c.adv).unwrap_or(bw);
    let kind = match shape {
        EncloseShape::Circle => ShapeKind::Ellipse,
        EncloseShape::Square => ShapeKind::Rectangle,
        EncloseShape::Triangle => ShapeKind::Triangle,
        EncloseShape::Diamond => ShapeKind::Diamond,
    };
    let stroke = (s * 0.06).clamp(0.4, 4.0);
    enclosures.push((k, Enclosed { kind, x: (adv - bw) / 2.0, top: cy + bh / 2.0, w: bw, h: bh, stroke }));
}

impl Builder<'_> {
    /// One cluster for ruby `base` text (an [`InlineObject::Ruby`] at `start..end`), as wide as the
    /// wider of the base and the ruby, with the ruby text laid out over it by `align`. The line
    /// grows to fit the ruby through the cluster's `obj_h`.
    #[allow(clippy::too_many_arguments)]
    fn ruby(
        &mut self,
        base: &str,
        ruby: &str,
        align: RubyAlign,
        raise: f32,
        start: usize,
        end: usize,
        rc: &Arc<ResolvedChar>,
        ruby_rc: &Arc<ResolvedChar>,
    ) -> Option<RubyLayout> {
        self.shape_atomic(base, start, end, rc);
        let ci = self.clusters.len().checked_sub(1)?;
        let (bc, bg) = (self.clusters.len(), self.glyphs.len());
        self.shape_with(ruby, 0, ruby_rc, None, false);
        let cl: Vec<Cluster> = self.clusters.drain(bc..).collect();
        let ruby_w: f32 = cl.iter().map(|c| c.adv).sum();
        let base_w = self.clusters.get(ci).map_or(0.0, |c| c.adv);
        let w = base_w.max(ruby_w);
        let n = cl.len().max(1) as f32;
        // Where the narrower text starts, and the extra gap between its letters (ruby only).
        let lead = |room: f32| match align {
            RubyAlign::Left => 0.0,
            RubyAlign::Right | RubyAlign::RightVertical => room,
            _ => room / 2.0,
        };
        let (x0, gap) = match align {
            RubyAlign::DistributeLetter if cl.len() > 1 => (0.0, (w - ruby_w) / (n - 1.0)),
            RubyAlign::DistributeSpace => ((w - ruby_w) / n / 2.0, (w - ruby_w) / n),
            _ => (lead(w - ruby_w), 0.0),
        };
        let mut glyphs = Vec::new();
        let mut ranges = Vec::new();
        let mut x = x0;
        for c in &cl {
            for g in self.glyphs.get(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                glyphs.push((c.style, Glyph { dx: g.dx + x, ..*g }));
                ranges.push(c.start..c.end);
            }
            x += c.adv + gap;
        }
        self.glyphs.truncate(bg);
        // The base text centres (or aligns) under a wider ruby.
        let shift = lead(w - base_w);
        let ascent = cl.iter().filter_map(|c| self.styles.get(c.style as usize)).map(|s| s.ascent).fold(0.0f32, f32::max);
        let c = self.clusters.get_mut(ci)?;
        for g in self.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
            g.dx += shift;
        }
        c.adv = w;
        c.obj_h = raise + ascent;
        Some(RubyLayout { cluster: ci, glyphs, text: ruby.to_string(), ranges, raise })
    }
}

/// Whether text with this formatting is left out of the layout: hidden text unless it is shown, and
/// tracked deletions in the final text. It takes no room, draws nothing and is no place to break.
pub(crate) fn left_out(rc: &ResolvedChar, show_hidden: bool, hide_deleted: bool) -> bool {
    (rc.hidden && !show_hidden) || (rc.del.is_some() && hide_deleted)
}

/// Lay out one paragraph.
pub fn layout_para(p: &Paragraph, env: &ParaEnv) -> ParaLayout {
    let doc = env.doc;
    let mut rp = doc.styles.resolve_para_in(&p.props, env.table.map(|t| &t.para));
    if let Some(spacing) = env.read_line_spacing.filter(|s| s.is_finite() && (1.0..=2.0).contains(s)) {
        rp.line_spacing = wordcraft_doc::props::LineSpacing::Multiple(spacing);
    }
    let para_style = p.props.style.as_deref();
    let table_chr = env.table.map(|t| &t.chr);
    // Indents in characters are measured in the paragraph mark's font size.
    if rp.indent_chars.iter().any(|c| *c != 0.0) {
        rp.set_char_unit(doc.styles.resolve_char_in(para_style, table_chr, &p.mark).size);
    }
    // List level indents apply unless the paragraph sets its own, or, for a list from the style,
    // the style giving the list (or one based on it) does (`indents_over_list`). A paragraph
    // that switches its style's list off (`num: 0`) drops the styles' indents, as Word does.
    let own_left = p.props.indent_left.is_some() || p.props.indent_left_chars.is_some_and(|c| c != 0.0);
    let own_first = p.props.indent_first.is_some() || p.props.indent_first_chars.is_some_and(|c| c != 0.0);
    if let Some((_, lvl)) = &env.label {
        let (style_left, style_first) =
            if p.props.numbering.is_some() { (false, false) } else { doc.styles.indents_over_list(para_style.unwrap_or("Normal")) };
        if !own_left && !style_left {
            rp.indent_left = lvl.indent;
        }
        if !own_first && !style_first {
            rp.indent_first = -lvl.hanging;
        }
    } else if p.props.numbering.is_some_and(|n| n.num == 0) {
        if !own_left {
            rp.indent_left = 0.0;
        }
        if !own_first {
            rp.indent_first = 0.0;
        }
    }
    let resolve = |c: &CharProps| -> Arc<ResolvedChar> { Arc::new(doc.styles.resolve_char_in(para_style, table_chr, c)) };
    let levels = bidi_levels(&p.text, rp.bidi);
    let mut b = Builder {
        env,
        spacing: rp.line_spacing,
        styles: Vec::new(),
        style_index: Default::default(),
        glyphs: Vec::new(),
        clusters: Vec::new(),
        levels,
        shown: Vec::new(),
    };
    let mark_rc = resolve(&p.mark);
    let mark_style = b.style(&mark_rc, None, false);
    let mut has_page_fields = false;
    let mut notes = Vec::new();
    let mut obj_index = 0usize;
    // Drop cap: the first character, sized so its capital spans `lines` lines.
    let drop_first = (rp.drop_cap > 0)
        .then(|| p.text.chars().next())
        .flatten()
        .filter(|c| !matches!(*c, '\t' | LINE_BREAK | PAGE_BREAK | COLUMN_BREAK | OBJ | '\r' | ' '))
        .filter(|c| p.text.len() > c.len_utf8());
    let mut drop_cap = None;
    let mut drop_cap_w = 0.0;
    let mut maths = Vec::new();
    let mut enclosures = Vec::new();
    let mut rubies = Vec::new();
    let mut displays = Vec::new();
    let mut eq_counter = env.eq_number;
    let math_props = doc.settings.math.clone().unwrap_or_default();
    // The byte ranges of runs left out of the layout, in order, adjacent runs merged.
    let mut left: Vec<std::ops::Range<usize>> = Vec::new();
    // Each run's proofing language and whether it is checked (spelling, grammar, hyphenation).
    let mut runs: Vec<ProofRun> = Vec::new();
    for (range, props) in p.run_ranges() {
        let rc = resolve(props);
        runs.push(ProofRun::of(range.clone(), &rc));
        let Some(text) = p.text.get(range.clone()) else { continue };
        if left_out(&rc, b.env.show_hidden, b.env.hide_deleted) {
            match left.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => left.push(range.clone()),
            }
            let si = b.style(&rc, None, false);
            let g = b.glyphs.len() as u32;
            // Word still prints the note of a hidden reference mark (and numbers it); a deleted
            // reference is gone from the final text, note and all.
            let keeps_notes = !(rc.del.is_some() && b.env.hide_deleted);
            for (i, c) in text.char_indices() {
                let mut note = None;
                if c == OBJ {
                    if let Some(InlineObject::NoteRef { id, .. }) = p.objects.get(obj_index)
                        && keeps_notes
                    {
                        note = Some(*id);
                    }
                    obj_index += 1;
                }
                if let Some(id) = note {
                    notes.push((b.clusters.len(), id));
                }
                b.clusters.push(Cluster {
                    start: range.start + i,
                    end: range.start + i + c.len_utf8(),
                    adv: 0.0,
                    kind: ClKind::Marker,
                    style: si,
                    g0: g,
                    g1: g,
                    break_after: false,
                    obj_h: 0.0,
                    obj_d: 0.0,
                    dot: false,
                });
            }
            continue;
        }
        let mut seg = 0;
        if range.start == 0
            && let Some(c) = drop_first
            && let Some(first) = text.get(..c.len_utf8())
        {
            let line_h = (rc.size * 1.2).max(1.0);
            let lines = rp.drop_cap as f32;
            // Capital height is ~0.7 em: the letter's cap spans from line 1's caps to line N's baseline.
            let mut big = (*rc).clone();
            // `clamp` would panic here when the text itself is larger than 1000 pt.
            big.size = (((lines - 1.0) * line_h + rc.size * 0.7) / 0.7).min(1000.0).max(rc.size);
            let before = b.clusters.len();
            b.shape(first, 0, &Arc::new(big), None);
            let w: f32 = b.clusters.get(before..).map(|c| c.iter().map(|c| c.adv).sum()).unwrap_or(0.0);
            drop_cap = Some((b.clusters.len() - before, rp.drop_cap, w + rc.size * 0.3));
            drop_cap_w = w;
            // The letter hangs beside the text; lines flow at the indent `break_lines` adds.
            for c in b.clusters.iter_mut().skip(before) {
                c.adv = 0.0;
            }
            seg = c.len_utf8();
        }
        for (i, c) in text.char_indices() {
            if i < seg {
                continue;
            }
            let special = matches!(c, '\t' | LINE_BREAK | PAGE_BREAK | COLUMN_BREAK | OBJ | '\r');
            if !special {
                continue;
            }
            if let Some(s) = text.get(seg..i) {
                b.shape(s, range.start + seg, &rc, None);
            }
            seg = i + c.len_utf8();
            let (start, end) = (range.start + i, range.start + i + c.len_utf8());
            let si = b.style(&rc, None, false);
            let g = b.glyphs.len() as u32;
            let push = |b: &mut Builder, kind: ClKind, adv: f32, h: f32| {
                b.clusters.push(Cluster { start, end, adv, kind, style: si, g0: g, g1: g, break_after: false, obj_h: h, obj_d: 0.0, dot: false })
            };
            match c {
                '\t' => push(&mut b, ClKind::Tab, 0.0, 0.0),
                LINE_BREAK | '\r' => push(&mut b, ClKind::LineBreak, 0.0, 0.0),
                PAGE_BREAK => push(&mut b, ClKind::PageBreak, 0.0, 0.0),
                COLUMN_BREAK => push(&mut b, ClKind::ColumnBreak, 0.0, 0.0),
                _ => {
                    let k = obj_index;
                    obj_index += 1;
                    match p.objects.get(k) {
                        Some(o) if o.is_drawing() => {
                            // A hidden one takes no room, and with no height its line doesn't draw it.
                            if let Some((w, h, float)) = o.frame()
                                && float.wrap == wordcraft_doc::para::Wrap::Inline
                                && !float.hidden
                            {
                                // A picture wider than the line keeps its size, as in Word: it runs past
                                // the margin and the page edge clips it (see `ParaEnv::fit_objects`).
                                let maxw = (env.width - rp.indent_left.max(0.0) - rp.indent_right.max(0.0)).max(18.0);
                                let (w, h) = (w.clamp(1.0, 4000.0), h.clamp(1.0, 4000.0));
                                let s = if env.fit_objects && w > maxw { maxw / w } else { 1.0 };
                                // The line makes room for effects (shadows) around the picture too, and
                                // for a rotated one's bounds, as Word does.
                                let [el, et, er, eb] = float.effect_extent();
                                let (px, py) = float.spin_pad(w * s, h * s);
                                push(&mut b, ClKind::Object(k), w * s + el + er + 2.0 * px, h * s + et + eb + 2.0 * py);
                            } else {
                                push(&mut b, ClKind::Object(k), 0.0, 0.0);
                            }
                        }
                        Some(InlineObject::Field { instr, result, code, .. }) => {
                            if env.field_codes != *code {
                                // The code in braces on grey, like Word's field shading.
                                let mut shaded = (*rc).clone();
                                if shaded.highlight.is_none() && shaded.shading.is_none() {
                                    shaded.shading = Some(FIELD_SHADING);
                                }
                                b.shape_atomic(&field_code_text(instr), start, end, &Arc::new(shaded));
                            } else {
                                match wordcraft_doc::eq::parse_overstrike(instr) {
                                    Some(o) => overstrike(&mut b, &o, instr, start, end, &rc, &mut enclosures),
                                    None => {
                                        let (t, page_dep) = field_text_on(instr, result, env.fields, env.fields.page_at(start));
                                        has_page_fields |= page_dep;
                                        b.shape_atomic(&t, start, end, &rc);
                                    }
                                }
                            }
                        }
                        Some(InlineObject::NoteRef { kind, id, custom }) => {
                            let num = if custom.is_empty() {
                                let fmt = match kind {
                                    NoteKind::Footnote => doc.settings.footnote_format,
                                    NoteKind::Endnote => doc.settings.endnote_format,
                                };
                                env.fields.note_mark(*id, fmt)
                            } else {
                                custom.clone()
                            };
                            let mut sup = (*rc).clone();
                            sup.vert_align = wordcraft_doc::props::VertAlign::Superscript;
                            b.shape_atomic(&num, start, end, &Arc::new(sup));
                            notes.push((b.clusters.len().saturating_sub(1), *id));
                        }
                        Some(InlineObject::Equation { linear, display, math }) => {
                            let avail = (env.width - rp.indent_left.max(0.0) - rp.indent_right.max(0.0)).max(12.0);
                            let ml = crate::math::layout_equation(math, linear, &rc, *display, avail, &math_props, &mut eq_counter);
                            let st = b.styles.get(si as usize);
                            // The line is at least as tall as the text around it.
                            let (a, d) = st.map(|s| (s.ascent, s.descent)).unwrap_or((0.0, 0.0));
                            b.clusters.push(Cluster {
                                start,
                                end,
                                adv: ml.width,
                                kind: ClKind::Object(k),
                                style: si,
                                g0: g,
                                g1: g,
                                break_after: false,
                                obj_h: ml.ascent.max(a).max(0.01),
                                obj_d: ml.descent.max(d),
                                dot: false,
                            });
                            if *display {
                                displays.push((b.clusters.len() - 1, math.jc));
                            }
                            maths.push((k, Arc::new(ml)));
                        }
                        // A note separator's line: a short one, or one across the column.
                        Some(o) if o.as_note_separator().is_some() => {
                            let cont = o.as_note_separator() == Some(true);
                            let maxw = (env.width - rp.indent_left.max(0.0) - rp.indent_right.max(0.0)).max(1.0);
                            push(&mut b, ClKind::Object(k), if cont { maxw } else { crate::NOTE_SEP_W.min(maxw) }, 0.0);
                        }
                        Some(InlineObject::Ruby { base, ruby, align, size, raise, props: ruby_props, .. }) => {
                            let mut rr = (*resolve(&props.clone().overlaid(ruby_props))).clone();
                            let size = if size.is_finite() { size.clamp(1.0, 1584.0) } else { rc.size * 0.5 };
                            (rr.size, rr.size_cs, rr.position) = (size, size, 0.0);
                            rr.vert_align = wordcraft_doc::props::VertAlign::Baseline;
                            let raise = if raise.is_finite() { raise.clamp(0.0, 1584.0) } else { rc.size };
                            if let Some(r) = b.ruby(base, ruby, *align, raise, start, end, &rc, &Arc::new(rr)) {
                                rubies.push(r);
                            }
                        }
                        Some(InlineObject::Opaque { text, .. }) => b.shape_atomic(text, start, end, &rc),
                        _ => push(&mut b, ClKind::Marker, 0.0, 0.0),
                    }
                }
            }
        }
        if let Some(s) = text.get(seg..) {
            b.shape(s, range.start + seg, &rc, None);
        }
    }
    if rp.snap_to_grid {
        snap_chars(&mut b, &p.text, &env.grid);
    }
    // Break opportunities and hyphenation points come from the text as laid out: hidden text that is
    // not shown and, without markup, tracked deletions are left out, so a space or soft hyphen there
    // is no place to break, and words are hyphenated as they read.
    // The text and, for each char of it, its end there and in `p.text`.
    let (text, ends): (std::borrow::Cow<str>, Vec<(usize, usize)>) = if left.is_empty() {
        (p.text.as_str().into(), Vec::new())
    } else {
        let (mut text, mut ends) = (String::new(), Vec::new());
        let mut next = left.iter().peekable();
        for (i, c) in p.text.char_indices() {
            // `left` is in order: skip the ranges that end here, then is `i` in the next one?
            while next.next_if(|r| r.end <= i).is_some() {}
            if next.peek().is_some_and(|r| r.start <= i) {
                continue;
            }
            text.push(c);
            ends.push((text.len(), i + c.len_utf8()));
        }
        (text.into(), ends)
    };
    // The end of a char in `text` → the same place in `p.text`.
    let to_para = |i: usize| -> Option<usize> {
        if left.is_empty() { Some(i) } else { ends.binary_search_by_key(&i, |e| e.0).ok().and_then(|k| ends.get(k)).map(|e| e.1) }
    };
    let mut found = Vec::new();
    for (i, o) in unicode_linebreak::linebreaks(&text) {
        // Word keeps "and/or" and web addresses whole: no break right after a slash (a word
        // too long for the line still breaks anywhere).
        let after_slash =
            text.get(..i).is_some_and(|t| t.ends_with('/')) && text.get(i..).and_then(|t| t.chars().next()).is_some_and(char::is_alphanumeric);
        if (o == unicode_linebreak::BreakOpportunity::Allowed || i < text.len()) && !after_slash {
            found.push(i);
        }
    }
    // Myanmar has no spaces between words: it wraps between syllables.
    if text.chars().any(|c| matches!(u32::from(c), 0x1000..=0x109F | 0xA9E0..=0xA9FF | 0xAA60..=0xAA7F)) {
        found.extend(wordcraft_doc::cluster::myanmar_breaks(&text));
        found.sort_unstable();
        found.dedup();
    }
    // Asian typography: kinsoku and breaking Latin words anywhere.
    crate::kinsoku::apply(&text, &mut found, rp.kinsoku, rp.word_wrap);
    let opps: std::collections::HashSet<usize> = found.into_iter().filter_map(to_para).collect();
    for c in &mut b.clusters {
        c.break_after = opps.contains(&c.end) || matches!(c.kind, ClKind::Object(_) | ClKind::Tab);
    }

    // Word hides a list label on a paragraph containing only a page break. The list counter
    // has already advanced, so the following item keeps its number.
    let break_only =
        b.clusters.iter().any(|c| c.kind == ClKind::PageBreak) && b.clusters.iter().all(|c| matches!(c.kind, ClKind::PageBreak | ClKind::Marker));
    let label = env.label.as_ref().filter(|_| !break_only).map(|(text, lvl)| {
        let lc = p.mark.clone().overlaid(&lvl.chr);
        let rc = resolve(&lc);
        // A Symbol/Wingdings bullet (U+F0B7…) is drawn as its Unicode equivalent when the font
        // isn't installed (shaping maps it); the label's text reads as that character too.
        let shown = |t: &str| -> String { wordcraft_doc::symbol_fonts::map_text(&rc.font, t) };
        let before_c = b.clusters.len();
        let before_g = b.glyphs.len();
        b.shape_with(text, 0, &rc, None, false);
        let mut cl: Vec<Cluster> = b.clusters.drain(before_c..).collect();
        // A right-to-left paragraph's label reads right to left too ("1." shows as ".1").
        if rp.bidi {
            let lv = bidi_levels(text, true);
            let cl_lv: Vec<u8> = cl.iter().map(|c| lv.get(c.start).copied().unwrap_or(1)).collect();
            let order = visual_order(&cl_lv);
            cl = order.iter().filter_map(|&i| cl.get(i).cloned()).collect();
        }
        let mut glyphs = Vec::new();
        let mut runs: Vec<LabelRun> = Vec::new();
        let mut ascent = 0.0f32;
        let mut x = 0.0;
        let mut style = mark_style;
        for c in &cl {
            style = c.style;
            if let Some(st) = b.styles.get(style as usize) {
                let a = b.font_metrics(&st.face).0 * st.size as f64 / st.face.upem.max(1.0);
                ascent = ascent.max(a as f32 + st.shift.max(0.0));
            }
            let g0 = glyphs.len();
            for g in b.glyphs.get(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                glyphs.push(Glyph { dx: g.dx + x, ..*g });
            }
            // A label in an All Caps paragraph reads as the capitals drawn (#1473).
            let chars: String = shown(text.get(c.start..c.end).unwrap_or("")).chars().map(|ch| caps_char(&rc, ch)).collect();
            match runs.last_mut() {
                Some(r) if r.style == c.style => {
                    r.glyphs.end = glyphs.len();
                    r.text.push_str(&chars);
                }
                _ => runs.push(LabelRun { style: c.style, glyphs: g0..glyphs.len(), text: chars }),
            }
            x += c.adv;
        }
        b.glyphs.truncate(before_g);
        Label { text: shown(text), ascent, style, glyphs, runs, x: 0.0, width: x }
    });

    let mut pl = ParaLayout {
        rp,
        styles: b.styles,
        glyphs: b.glyphs,
        clusters: b.clusters,
        shown: b.shown,
        lines: Vec::new(),
        label,
        height: 0.0,
        text_len: p.text.len(),
        has_page_fields,
        notes,
        issues: env.proofing.map(|o| proof_issues(p, &runs, &o)).unwrap_or_default(),
        drop_cap,
        hyph_after: Vec::new(),
        bidi_levels: b.levels,
        drop_cap_w,
        maths,
        rubies,
        displays,
        left_out: Vec::new(),
        enclosures,
    };
    let auto = env.doc.settings.auto_hyphenation && !pl.rp.suppress_hyphens;
    pl.hyph_after = hyphenation_points(&text, to_para, &pl, auto, env.doc.settings.hyphenate_caps, &runs);
    pl.left_out = left;
    for k in pl.hyph_after.clone() {
        if let Some(c) = pl.clusters.get_mut(k as usize) {
            c.break_after = true;
        }
    }
    break_lines(&mut pl, env, mark_style, env.label.as_ref().map(|(_, l)| l));
    pl
}

/// Next tab stop after `x` (column coordinates): explicit stops, the hanging-indent implicit stop,
/// then default stops.
fn next_tab(x: f32, tabs: &[TabStop], default_tab: f32, hanging_at: Option<f32>) -> TabStop {
    let mut best: Option<TabStop> = tabs.iter().copied().filter(|t| t.pos > x + 0.01).min_by(|a, b| a.pos.total_cmp(&b.pos));
    if let Some(h) = hanging_at
        && h > x + 0.01
        && best.is_none_or(|t| h < t.pos)
    {
        best = Some(TabStop { pos: h, align: TabAlign::Left, leader: TabLeader::None });
    }
    best.unwrap_or_else(|| {
        let d = if default_tab > 1.0 { default_tab } else { 36.0 };
        // Left of the margin Word's arithmetic truncates toward zero: the next default stop is
        // the first multiple of the interval *at or after* `x`, so a tab that already sits on
        // one (a second tab in a row, or a line starting at -72 pt) stays put. Measured on Word
        // 2016+: a line starting at -72 pt with a leading tab stays at -72, at -60 goes to -36;
        // "a\t\tX" at -60 puts X at -36, not 0. Letterheads with negative indents rely on it.
        let n = if x < -0.01 { ((x - 0.01) / d).ceil() } else { ((x + 0.01) / d).floor() + 1.0 };
        TabStop { pos: (n * d).min(1e6), align: TabAlign::Left, leader: TabLeader::None }
    })
}

fn break_lines(pl: &mut ParaLayout, env: &ParaEnv, mark_style: u32, level: Option<&Level>) {
    let suffix = level.map(|l| l.suffix);
    let rp = pl.rp.clone();
    let width = env.width.max(12.0);
    let base_right = (width - rp.indent_right).max(1.0);
    let default_tab = env.doc.settings.default_tab;
    let first_left = rp.indent_left + rp.indent_first;
    // Word 2013+ (compatibility mode 15) fits more text on a justified line by shrinking its
    // spaces, up to MAX_SPACE_SHRINK of their width; never on lines with tabs.
    let shrink_spaces = rp.align == Align::Justify && env.doc.settings.compat_mode >= wordcraft_doc::COMPAT_MODE_CURRENT;
    let hanging_at = if rp.indent_first < 0.0 { Some(rp.indent_left) } else { None };
    // Hyphenation Options: the zone, and how many lines in a row may end with a hyphen.
    let zone = Some(env.doc.settings.hyphenation_zone).filter(|z| z.is_finite() && *z >= 0.0).unwrap_or(HYPHENATION_ZONE).min(1584.0);
    let hyph_limit = env.doc.settings.consecutive_hyphen_limit as usize;
    let n = pl.clusters.len();
    // A right-to-left paragraph is laid out from its start edge, as if mirrored: indents, tabs,
    // the list label and alignment are measured from the right. Lines are mirrored back (and
    // bidirectional text reordered) once broken; floating objects are mirrored in.
    let rtl_para = rp.bidi;
    // The document grid's line pitch, for paragraphs that snap to it.
    let grid = Some(env.grid).filter(|g| g.line.is_some() && rp.snap_to_grid);
    let grid = grid.as_ref();
    let bidi = rtl_para || !pl.bidi_levels.is_empty();
    let mirrored: Vec<Exclusion>;
    let exclusions: &[Exclusion] = if rtl_para {
        mirrored = env
            .exclusions
            .iter()
            .map(|e| Exclusion {
                left: width - e.right,
                right: width - e.left,
                contour: e.contour.as_ref().map(|c| Arc::new(c.mirrored(e.right - e.left))),
                // Mirrored, the object's left side is the line's right.
                side: match e.side {
                    WrapText::Left => WrapText::Right,
                    WrapText::Right => WrapText::Left,
                    s => s,
                },
                ..e.clone()
            })
            .collect();
        &mirrored
    } else {
        env.exclusions
    };
    let mut first_x0: Option<f32> = None;
    let mut hcache: Vec<(u32, u32, f32)> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    let mut i = 0usize;
    let mut top = 0.0f32;
    let mut first = true;
    let est_h = pl
        .styles
        .get(mark_style as usize)
        .map(|st| (st.ascent + st.descent) * if let LineSpacing::Multiple(m) = rp.line_spacing { m } else { 1.0 })
        .unwrap_or(14.0)
        .max(1.0);
    // The rest of the current row: spans on the far side of floating objects, left to right.
    let mut row_rest: VecDeque<(f32, f32)> = VecDeque::new();
    let mut row_first = 0usize;
    // Spans skipped in a row because the next word didn't fit (see `skip`).
    let mut skipped = 0;
    loop {
        // A line already sits on this row (a span before this one may have been skipped).
        let beside = !row_rest.is_empty() && lines.len() > row_first;
        // The span is narrowed by a floating object.
        let mut narrowed = !row_rest.is_empty();
        let row_lo = if first { first_left } else { rp.indent_left };
        let (left, right_edge) = if let Some(span) = row_rest.pop_front() {
            span
        } else {
            // Flow around floating objects: the spans beside them, or below them.
            let mut guard = 0;
            loop {
                let mut lo = if first { first_left } else { rp.indent_left };
                if let Some((_, dl, dw)) = pl.drop_cap
                    && lines.len() < dl as usize
                {
                    lo = first_left.max(rp.indent_left) + dw;
                }
                match row_spans(exclusions, top, est_h, lo, base_right).map(|mut spans| (spans.pop_front(), spans)) {
                    Ok((Some((a, b)), rest)) => {
                        narrowed = !rest.is_empty() || a > lo || b < base_right;
                        row_rest = rest;
                        row_first = lines.len();
                        break (a, b.max(a + 12.0));
                    }
                    Err(below) if below > top && guard < 50 => {
                        top = below;
                        guard += 1;
                    }
                    Ok((None, _)) | Err(_) => {
                        row_first = lines.len();
                        break (lo, base_right.max(lo + 12.0));
                    }
                }
            }
        };
        let mut x = left;
        // Label on the first line.
        if first && let Some(lab) = pl.label.as_mut() {
            // The number is aligned at the first-line indent: its left edge, centre or right edge.
            lab.x = match level.map(|l| l.align) {
                Some(Align::Center) => left - lab.width / 2.0,
                Some(Align::Right) => left - lab.width,
                _ => left,
            };
            let end = lab.x + lab.width;
            x = match suffix.unwrap_or(LevelSuffix::Tab) {
                LevelSuffix::Tab => {
                    let t = next_tab(end, &rp.tabs, default_tab, hanging_at).pos;
                    // The level's own tab stop after the number, when it comes first.
                    match level.and_then(|l| l.tab).filter(|p| p.is_finite() && *p > end + 0.01) {
                        Some(p) if p < t => p,
                        _ => t,
                    }
                }
                LevelSuffix::Space => end + pl.styles.get(lab.style as usize).map(|s| s.size * 0.25).unwrap_or(3.0),
                LevelSuffix::Nothing => end,
            };
        }
        let line_start_x = x;
        let c0 = i;
        // The first word doesn't fit a span narrowed by a floating object: it moves on whole.
        let mut skip = false;
        let mut xs: Vec<f32> = Vec::new();
        let mut leaders = Vec::new();
        let mut last_break: Option<usize> = None; // cluster index after which we may break
        let mut last_plain: Option<(usize, f32)> = None; // last non-hyphen break and the x after it
        // After `hyph_limit` hyphenated lines in a row this one may not end with a hyphen.
        let hyph_ok = hyph_limit == 0 || lines.iter().rev().take_while(|l| l.hyphen.is_some()).count() < hyph_limit;
        let mut end = LineEnd::Para;
        let mut j = i;
        let mut pending_tab: Option<(usize, TabStop, f32)> = None; // tab cluster, stop, x where tab started
        let mut space_w = 0.0f32; // width of the spaces so far on this line
        let mut has_tab = false;
        while j < n {
            let Some(c) = pl.clusters.get(j).cloned() else { break };
            match c.kind {
                ClKind::LineBreak | ClKind::PageBreak | ClKind::ColumnBreak => {
                    resolve_tab(pl, &mut pending_tab, &mut xs, c0, x, &mut x);
                    xs.push(x);
                    end = match c.kind {
                        ClKind::LineBreak => LineEnd::LineBreak,
                        ClKind::PageBreak => LineEnd::PageBreak,
                        _ => LineEnd::ColumnBreak,
                    };
                    j += 1;
                    break;
                }
                ClKind::Tab => {
                    has_tab = true;
                    resolve_tab(pl, &mut pending_tab, &mut xs, c0, x, &mut x);
                    let stop = next_tab(x, &rp.tabs, default_tab, hanging_at);
                    xs.push(x);
                    if stop.pos > right_edge + 0.01 && j > c0 && stop.align == TabAlign::Left {
                        // Tab past the right indent wraps (Word moves it to the next line).
                        if let Some(c) = pl.clusters.get_mut(j) {
                            c.adv = 0.0;
                        }
                        x = right_edge.max(x);
                        last_break = Some(j);
                        j += 1;
                        continue;
                    }
                    match stop.align {
                        TabAlign::Left | TabAlign::Bar | TabAlign::Clear => {
                            let w = (stop.pos - x).max(0.0);
                            if let Some(c) = pl.clusters.get_mut(j) {
                                c.adv = w;
                            }
                            if stop.leader != TabLeader::None {
                                leaders.push((j, stop.leader));
                            }
                            x += w;
                        }
                        _ => {
                            pending_tab = Some((j, stop, x));
                            if stop.leader != TabLeader::None {
                                leaders.push((j, stop.leader));
                            }
                        }
                    }
                    last_break = Some(j);
                    j += 1;
                    continue;
                }
                _ => {}
            }
            // A display equation sits on a line of its own.
            let is_display = pl.displays.iter().any(|(ci, _)| *ci == j);
            if is_display && (c0..j).any(|k| pl.clusters.get(k).is_some_and(|c| c.kind != ClKind::Marker)) {
                end = LineEnd::Wrap;
                break;
            }
            // Text after a right/centre tab grows leftwards into the tab's space first.
            let absorbs = pending_tab.is_some_and(|(tj, stop, _)| {
                let room = pl.clusters.get(tj).map(|t| t.adv).unwrap_or(0.0);
                match stop.align {
                    TabAlign::Right | TabAlign::Decimal => room >= c.adv,
                    TabAlign::Center => room >= c.adv / 2.0 && x + c.adv / 2.0 <= right_edge + 0.01,
                    _ => false,
                }
            });
            let shrink = if shrink_spaces && !has_tab { space_w * MAX_SPACE_SHRINK } else { 0.0 };
            let fits = absorbs || x + c.adv <= right_edge + shrink + 0.01 || c.kind == ClKind::Space || c.kind == ClKind::Marker;
            // A line's first item stays on it even when it doesn't fit; markers before it
            // (a content control's start, a bookmark) take no room and don't count (Word).
            if !fits && (c0..j).any(|k| pl.clusters.get(k).is_some_and(|c| c.kind != ClKind::Marker)) {
                // Wrap: back up to the last break opportunity on this line.
                end = LineEnd::Wrap;
                // Hyphenate only when the plain break leaves more than the hyphenation zone empty.
                if let (Some(bk), Some((pj, px))) = (last_break, last_plain)
                    && pj >= c0
                    && pj < bk
                    && pl.hyph_after.binary_search(&(bk as u32)).is_ok()
                    && right_edge - px <= zone
                {
                    last_break = Some(pj);
                }
                if let Some(bk) = last_break.filter(|bk| *bk >= c0) {
                    xs.truncate(bk + 1 - c0);
                    j = bk + 1;
                    // Recompute x for the kept clusters.
                    x = xs.last().copied().unwrap_or(line_start_x) + pl.clusters.get(bk).map(|c| c.adv).unwrap_or(0.0);
                } else {
                    // No opportunity: break here (a long word), unless the span is beside a
                    // floating object and the word fits the full line; then it tries the row's
                    // next span, or the next row. The word runs to the next break that isn't a
                    // hyphenation point.
                    let word: f32 = (c0..n)
                        .filter_map(|k| pl.clusters.get(k).map(|c| (k, c)))
                        .scan(false, |done, (k, c)| {
                            let end = c.break_after && pl.hyph_after.binary_search(&(k as u32)).is_err();
                            (!std::mem::replace(done, end)).then_some(c)
                        })
                        .filter(|c| c.kind != ClKind::Space)
                        .map(|c| c.adv)
                        .sum();
                    let full = base_right - row_lo;
                    skip = narrowed && skipped < 50 && word <= full + 0.01;
                }
                break;
            }
            xs.push(x);
            x += c.adv;
            if c.kind == ClKind::Space {
                space_w += c.adv;
            }
            // Decimal/center/right tab: shift pending text as it grows.
            if let Some((tj, stop, tx)) = pending_tab {
                let seg_w = x - tx;
                let shift = match stop.align {
                    TabAlign::Right => stop.pos - tx - seg_w,
                    TabAlign::Center => stop.pos - tx - seg_w / 2.0,
                    TabAlign::Decimal => {
                        // Width up to the first '.' in the segment.
                        let mut w = 0.0;
                        let mut found = false;
                        for k in tj + 1..=j {
                            let Some(cc) = pl.clusters.get(k) else { break };
                            let s = pl_text_at(cc);
                            if found {
                                break;
                            }
                            if s {
                                found = true;
                                break;
                            }
                            w += cc.adv;
                        }
                        stop.pos - tx - if found { w } else { seg_w }
                    }
                    _ => 0.0,
                };
                let shift = shift.max(0.0);
                let old = pl.clusters.get(tj).map(|c| c.adv).unwrap_or(0.0);
                if (shift - old).abs() > 0.001 {
                    let delta = shift - old;
                    if let Some(c) = pl.clusters.get_mut(tj) {
                        c.adv = shift;
                    }
                    let from = tj + 1 - c0;
                    for v in xs.iter_mut().skip(from) {
                        *v += delta;
                    }
                    x += delta;
                }
            }
            if is_display {
                // Markers right after it stay on its line; anything else starts a new one.
                j += 1;
                while j < n && pl.clusters.get(j).is_some_and(|c| c.kind == ClKind::Marker) {
                    xs.push(x);
                    j += 1;
                }
                end = if j >= n { LineEnd::Para } else { LineEnd::Wrap };
                break;
            }
            if c.break_after {
                let hyph = pl.hyph_after.binary_search(&(j as u32)).is_ok();
                if !hyph {
                    last_break = Some(j);
                    last_plain = Some((j, x));
                } else if hyph_ok && x + hyphen_glyph(pl, c.style, &mut hcache).2 <= right_edge + shrink + 0.01 {
                    last_break = Some(j);
                }
            }
            j += 1;
        }
        if skip {
            skipped += 1;
            if row_rest.is_empty() {
                top = if lines.len() > row_first {
                    top + even_row(lines.get_mut(row_first..).unwrap_or_default())
                } else {
                    // Nothing on this row: go below the first object that ends, as when no span is wide enough.
                    exclusions
                        .iter()
                        .filter(|e| e.bottom > top && e.top < top + est_h && e.right > row_lo && e.left < base_right)
                        .map(|e| e.bottom)
                        .reduce(f32::min)
                        .unwrap_or(top + est_h)
                };
            }
            continue;
        }
        if end == LineEnd::Para || end == LineEnd::Wrap {
            let xc = x;
            resolve_tab(pl, &mut pending_tab, &mut xs, c0, xc, &mut x);
        }
        if j >= n && end != LineEnd::Wrap && !matches!(end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) {
            end = LineEnd::Para;
        }
        let c1 = j.max(c0);
        // Ensure progress.
        let (c1, j) = if c1 == c0 && c0 < n { (c0 + 1, c0 + 1) } else { (c1, j) };
        while xs.len() < c1 - c0 {
            let last = xs.last().copied().unwrap_or(line_start_x);
            let adv = pl.clusters.get(c0 + xs.len().saturating_sub(1)).map(|c| c.adv).unwrap_or(0.0);
            xs.push(if xs.is_empty() { line_start_x } else { last + adv });
        }
        xs.truncate(c1 - c0);
        // A line wrapped at a hyphenation point shows a hyphen after its last cluster.
        let hyphen = (end == LineEnd::Wrap && c1 > c0 && pl.hyph_after.binary_search(&((c1 - 1) as u32)).is_ok())
            .then(|| pl.clusters.get(c1 - 1).map(|c| hyphen_glyph(pl, c.style, &mut hcache)))
            .flatten();
        let end_x = match (xs.last(), c1.checked_sub(1).and_then(|k| pl.clusters.get(k))) {
            (Some(lx), Some(c)) => lx + c.adv + hyphen.map_or(0.0, |h| h.2),
            _ => line_start_x,
        };
        xs.push(end_x);
        pending_tab = None;
        let _ = pending_tab;

        // Vertical metrics.
        let (mut asc, mut desc) = (0.0f32, 0.0f32);
        // The tallest picture on the line (it sits on the baseline).
        let mut obj_asc = 0.0f32;
        let mut any = false;
        let dropped = pl.drop_cap.map_or(0, |d| d.0);
        for (k, c) in pl.clusters.get(c0..c1).into_iter().flatten().enumerate() {
            // Spaces and tabs never make a line taller, whatever their size: Word sizes a line by
            // its other characters (a line of only them takes the paragraph mark's), measured in
            // Word for Windows with 9 pt text and 11–20 pt tabs, spaces and ideographic spaces.
            if matches!(c.kind, ClKind::Marker | ClKind::Space | ClKind::Tab) || c0 + k < dropped {
                continue;
            }
            if let Some(st) = pl.styles.get(c.style as usize) {
                // A picture sits on the baseline of its run: the line keeps that font's ascent
                // and descent, and grows above the baseline to fit a taller picture.
                let (a, d) = if matches!(c.kind, ClKind::Object(_)) && c.obj_h > 0.0 {
                    if pl.maths.iter().any(|(k, _)| c.kind == ClKind::Object(*k)) {
                        // An equation is text: its ascent and descent count like the text's.
                        (c.obj_h, c.obj_d)
                    } else {
                        obj_asc = obj_asc.max(c.obj_h);
                        (st.ascent, st.descent)
                    }
                } else {
                    // `obj_h` is set on text clusters only under ruby text, which the line grows to fit.
                    ((st.ascent + st.shift.max(0.0)).max(c.obj_h + st.shift), st.descent + (-st.shift).max(0.0))
                };
                asc = asc.max(a);
                desc = desc.max(d);
                any = true;
            }
        }
        if (!any || (asc == 0.0 && desc == 0.0))
            && let Some(st) = pl.styles.get(mark_style as usize)
        {
            asc = st.ascent;
            desc = st.descent;
        }
        let text_natural = asc + desc;
        let text_pictures = if obj_asc > asc { obj_asc + desc } else { 0.0 };
        let mut label_extra = 0.0;
        if first && let Some(lab) = &pl.label {
            if !matches!(rp.line_spacing, LineSpacing::Exactly(_)) {
                // Automatic/minimum spacing keeps text descent. Only the label's ascent
                // overflow adds room, unscaled by the automatic spacing multiplier.
                label_extra = (lab.ascent - asc).max(0.0);
                asc = asc.max(lab.ascent);
            } else {
                for st in lab.runs.iter().filter_map(|r| pl.styles.get(r.style as usize)) {
                    asc = asc.max(st.ascent);
                    desc = desc.max(st.descent);
                }
            }
        }
        // Keep the picture floor independent of list-label metrics and line-spacing multipliers.
        // shortcut: Word leaves less than a full descent below a line of only pictures (0.5pt to
        // 1.9pt in docxide-pdf's header fixtures case121/case123, where the descent is 3.3pt), and
        // no rule tried so far (none, only the text's descent) matched both; it needs Word probes.
        let natural = asc + desc;
        let pictures = if obj_asc > asc { obj_asc + desc } else { 0.0 };
        // Document grid: the line takes whole line pitches, its text centred in them.
        let (height, centred) = match (rp.line_spacing, grid) {
            (LineSpacing::Multiple(m), Some(g)) => ((g.snap_height(natural) * m).max(g.snap_height(pictures)), true),
            (LineSpacing::AtLeast(v), Some(g)) => (g.snap_height(natural.max(pictures)).max(v), true),
            (LineSpacing::Multiple(m), None) => ((text_natural * m + label_extra).max(text_pictures), false),
            (LineSpacing::AtLeast(v), None) => (natural.max(text_pictures).max(v), false),
            (LineSpacing::Exactly(v), _) => (v, false),
        };
        let baseline = if centred {
            let up = asc.max(obj_asc);
            top + ((height - up - desc) / 2.0).max(0.0) + up
        } else {
            top + baseline_offset(rp.line_spacing, asc, height, desc).max(obj_asc.min(height - desc))
        };

        // Alignment / justification (trailing spaces hang).
        let mut content_end = xs.last().copied().unwrap_or(line_start_x);
        for k in (c0..c1).rev() {
            match pl.clusters.get(k).map(|c| c.kind) {
                Some(ClKind::Space) | Some(ClKind::Marker) | Some(ClKind::LineBreak) | Some(ClKind::PageBreak) | Some(ClKind::ColumnBreak) => {
                    content_end = xs.get(k - c0).copied().unwrap_or(content_end);
                }
                _ => break,
            }
        }
        let slack = right_edge - content_end;
        let last_tab = (c0..c1).rev().find(|k| pl.clusters.get(*k).is_some_and(|c| c.kind == ClKind::Tab));
        let align_from = last_tab.map(|k| k + 1).unwrap_or(c0);
        let display = pl.displays.iter().find(|(ci, _)| *ci >= c0 && *ci < c1).map(|d| d.1);
        let shift = match (display, rp.align) {
            (Some(MathJc::Left), _) => 0.0,
            (Some(MathJc::Right), _) => slack,
            (Some(_), _) => slack / 2.0,
            (None, Align::Center) => slack / 2.0,
            (None, Align::Right) => slack,
            _ => 0.0,
        };
        if shift > 0.0 && slack > 0.0 {
            for v in xs.iter_mut().skip(align_from - c0) {
                *v += shift;
            }
            // The list label belongs to the start of the line: it moves with the text (Word keeps
            // the bullet attached when a list item is centred or aligned to the end). After a tab
            // stop in the line the label stays, as the text before the tab does.
            if first
                && last_tab.is_none()
                && let Some(lab) = pl.label.as_mut()
            {
                lab.x += shift;
            }
        }
        let justify = display.is_none() && ((rp.align == Align::Justify && end == LineEnd::Wrap) || rp.align == Align::Distribute);
        // A line that only fits with shrunk spaces is shrunk, the last line of the paragraph too.
        let shrink = display.is_none() && shrink_spaces && slack < 0.0 && last_tab.is_none();
        if shrink {
            shrink_line_spaces(pl, &mut xs, c0, c1, content_end, slack);
        } else if justify && slack > 0.0 {
            // Spaces inside the content (after the last tab).
            let spaces: Vec<usize> = (align_from..c1)
                .filter(|k| {
                    pl.clusters.get(*k).is_some_and(|c| c.kind == ClKind::Space) && xs.get(k - c0).copied().unwrap_or(f32::MAX) < content_end - 0.01
                })
                .collect();
            if rp.align == Align::Distribute || spaces.is_empty() {
                let cnt = (c1 - align_from).saturating_sub(1).max(1) as f32;
                if rp.align == Align::Distribute {
                    let per = slack / cnt;
                    for (q, v) in xs.iter_mut().enumerate().skip(align_from - c0) {
                        let k = (q + c0 - align_from) as f32;
                        *v += per * k.min(cnt);
                    }
                }
            } else {
                let per = slack / spaces.len() as f32;
                let mut add = 0.0;
                let mut si = 0;
                for k in align_from..c1 {
                    if let Some(v) = xs.get_mut(k - c0) {
                        *v += add;
                    }
                    if spaces.get(si) == Some(&k) {
                        add += per;
                        si += 1;
                    }
                }
                if let Some(v) = xs.last_mut() {
                    *v += add;
                }
            }
        }
        let start = pl.clusters.get(c0).map(|c| c.start).unwrap_or(pl.text_len);
        let stop = if c1 > c0 { pl.clusters.get(c1 - 1).map(|c| c.end).unwrap_or(pl.text_len) } else { start };
        if first_x0.is_none() {
            first_x0 = xs.first().copied();
        }
        let mut line = Line {
            top,
            height,
            baseline,
            c0,
            c1,
            xs,
            leaders,
            end,
            start,
            stop,
            left,
            right: right_edge,
            hyphen,
            beside,
            vis: Vec::new(),
            rtl: rtl_para,
        };
        if bidi {
            reorder_line(pl, &mut line, width);
        }
        lines.push(line);
        first = false;
        skipped = 0;
        i = j;
        // Text that wrapped continues on the row's far side; anything else ends the row.
        if end != LineEnd::Wrap || i >= n {
            row_rest.clear();
        }
        if row_rest.is_empty() {
            top += even_row(lines.get_mut(row_first..).unwrap_or_default());
        }
        if i >= n {
            // A paragraph ending with a line break gets an empty last line.
            if matches!(end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) {
                let (asc, desc) = pl.styles.get(mark_style as usize).map(|s| (s.ascent, s.descent)).unwrap_or((10.0, 3.0));
                let natural = asc + desc;
                let h = match (rp.line_spacing, grid) {
                    (LineSpacing::Multiple(m), Some(g)) => g.snap_height(natural) * m,
                    (LineSpacing::AtLeast(v), Some(g)) => g.snap_height(natural).max(v),
                    (LineSpacing::Multiple(m), None) => natural * m,
                    (LineSpacing::AtLeast(v), None) => natural.max(v),
                    (LineSpacing::Exactly(v), _) => v,
                };
                let x0 = rp.indent_left;
                let (x0, left, right) = if rtl_para { (width - x0, width - base_right, width - x0) } else { (x0, x0, base_right) };
                lines.push(Line {
                    top,
                    height: h,
                    baseline: top + if grid.is_some() { h - desc } else { baseline_offset(rp.line_spacing, asc, h, desc) },
                    c0: n,
                    c1: n,
                    xs: vec![x0],
                    leaders: Vec::new(),
                    end: LineEnd::Para,
                    start: pl.text_len,
                    stop: pl.text_len,
                    left,
                    right,
                    hyphen: None,
                    beside: false,
                    vis: Vec::new(),
                    rtl: rtl_para,
                });
                top += h;
            }
            break;
        }
        if lines.len() > 200_000 {
            break;
        }
    }
    // Hang the drop cap from the first line's caps down to line N's baseline, left of the text.
    if let Some((nc, dl, dw)) = pl.drop_cap
        && let (Some(l0), Some(ln)) = (lines.first(), lines.get((dl as usize).min(lines.len()).saturating_sub(1)))
    {
        let short = lines.len() < dl as usize;
        let dy = if short { (dl as f32 - 1.0) * l0.height } else { ln.baseline - l0.baseline };
        let dx = dw.min(first_x0.unwrap_or(0.0) - first_left.max(rp.indent_left));
        // Toward the start edge: left, or right (and from the letter's right edge) when mirrored.
        let dx = if rtl_para { pl.drop_cap_w - dx } else { dx };
        if short {
            top = top.max(l0.top + l0.height * dl as f32);
        }
        for c in pl.clusters.get(..nc).unwrap_or(&[]).to_vec() {
            for g in pl.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                g.dy -= dy;
                g.dx -= dx;
            }
        }
    }
    if rtl_para && let Some(lab) = pl.label.as_mut() {
        lab.x = width - lab.x - lab.width;
    }
    pl.height = top;
    pl.lines = lines;
}

/// Make a row's lines as tall as its tallest, and return that height.
fn even_row(row: &mut [Line]) -> f32 {
    let h = row.iter().map(|l| l.height).fold(0.0f32, f32::max);
    for l in row {
        l.baseline += h - l.height;
        l.height = h;
    }
    h
}

/// Finish a broken line of a bidirectional paragraph: resolve each cluster's level for the line
/// (UAX #9 rule L1), order the clusters visually (rule L2) and give each its box. `line.xs`
/// arrive in logical order measured from the paragraph's start edge (mirrored for a
/// right-to-left paragraph); afterwards `line.vis` holds the boxes and `xs` their left edges.
fn reorder_line(pl: &ParaLayout, line: &mut Line, width: f32) {
    let rtl_para = line.rtl;
    if rtl_para {
        let (l, r) = (line.left, line.right);
        line.left = width - r;
        line.right = width - l;
    }
    let (c0, c1) = (line.c0, line.c1);
    let n = c1.saturating_sub(c0);
    if n == 0 {
        if rtl_para {
            line.xs = line.xs.iter().map(|x| width - x).collect();
        }
        return;
    }
    let para_level = u8::from(rtl_para);
    let mut levels: Vec<u8> = (c0..c1).map(|k| pl.clusters.get(k).and_then(|c| pl.bidi_levels.get(c.start)).copied().unwrap_or(para_level)).collect();
    // L1: tabs and breaks, and the whitespace before them or at the end of the line, take the
    // paragraph level (so trailing spaces stay at the line's end edge).
    let mut trailing = true;
    for (i, k) in (c0..c1).enumerate().rev() {
        let kind = pl.clusters.get(k).map(|c| c.kind);
        let reset = match kind {
            Some(ClKind::Tab | ClKind::LineBreak) => {
                trailing = true;
                true
            }
            Some(ClKind::Space | ClKind::PageBreak | ClKind::ColumnBreak | ClKind::Marker) => trailing,
            _ => {
                trailing = false;
                false
            }
        };
        if reset && let Some(v) = levels.get_mut(i) {
            *v = para_level;
        }
    }
    if !rtl_para && levels.iter().all(|l| *l == 0) {
        return; // A left-to-right line of a paragraph with right-to-left text elsewhere.
    }
    let order = visual_order(&levels);
    let hyph = line.hyphen.map_or(0.0, |h| h.2);
    let mut w: Vec<f32> = (0..n).map(|i| line.xs.get(i + 1).zip(line.xs.get(i)).map_or(0.0, |(b, a)| b - a)).collect();
    if let Some(last) = w.last_mut() {
        *last -= hyph;
    }
    let total: f32 = w.iter().sum();
    let x0 = line.xs.first().copied().unwrap_or(0.0);
    let mut x = if rtl_para { width - x0 - total } else { x0 };
    let mut vis = vec![VisCl { x: 0.0, w: 0.0, level: para_level }; n];
    for &i in &order {
        if let (Some(v), Some(wi), Some(l)) = (vis.get_mut(i), w.get(i), levels.get(i)) {
            *v = VisCl { x, w: *wi, level: *l };
            x += *wi;
        }
    }
    line.vis = vis;
    let end = line.end_x();
    line.xs = line.vis.iter().map(|v| v.x).chain(std::iter::once(end)).collect();
}

/// Narrowest span text flows into beside a floating object, points. Word puts an empty
/// paragraph beside a floating table in a 21.3pt gap but not in an 18.6pt one.
const MIN_SPAN: f32 = 20.0;

/// How far below a line's top its baseline sits. Multiple spacing at or above single keeps the
/// text at the top of the line and puts the extra leading below the baseline; below single it
/// shrinks the whole box, ascent included (Word's output for docxide-pdf's fixtures: 1.15-spaced
/// Times New Roman 12pt sits one ascent below the line top, not 1.8pt lower). Exact and at-least
/// spacing keep the descent below the baseline and the rest above it.
fn baseline_offset(spacing: LineSpacing, asc: f32, height: f32, desc: f32) -> f32 {
    match spacing {
        LineSpacing::Multiple(m) => asc * m.min(1.0),
        LineSpacing::AtLeast(_) | LineSpacing::Exactly(_) => height - desc,
    }
}

/// The spans of a row at `top` (about `h` tall) between `lo` and `hi` that text can use around
/// the exclusions, left to right (Square wrapping uses both sides of an object). `Err(y)`: none
/// here, try again at `y` (below what's in the way).
fn row_spans(exclusions: &[Exclusion], top: f32, h: f32, lo: f32, hi: f32) -> Result<VecDeque<(f32, f32)>, f32> {
    let here: Vec<&Exclusion> = exclusions.iter().filter(|e| e.bottom > top && e.top < top + h && e.right > lo && e.left < hi).collect();
    if let Some(below) = here.iter().filter(|e| e.top_bottom).map(|e| e.bottom).reduce(f32::max) {
        return Err(below);
    }
    // What each object takes of the row: its whole area, or (Tight, Through) the stretches its
    // outline takes, as (left, right, bottom of the object's area).
    let mut taken: Vec<(f32, f32, f32)> = Vec::new();
    for e in &here {
        let first = taken.len();
        match &e.contour {
            None => taken.push((e.left, e.right, e.bottom)),
            Some(c) => taken.extend(c.occupied(top - e.top, top + h - e.top).into_iter().map(|(a, b)| (a + e.left, b + e.left, e.bottom))),
        }
        // Text on one side only: the object's other side of the row is taken too.
        let Some(mine) = taken.get(first..).filter(|t| !t.is_empty() && !e.side.is_both()) else { continue };
        let left = mine.iter().map(|t| t.0).fold(f32::INFINITY, f32::min);
        let right = mine.iter().map(|t| t.1).fold(f32::NEG_INFINITY, f32::max);
        let text_left = match e.side {
            WrapText::Left => true,
            WrapText::Right => false,
            // The side with more room on this line (left on a tie).
            _ => left - lo >= hi - right,
        };
        taken.truncate(first);
        taken.push(if text_left { (left, hi.max(right), e.bottom) } else { (lo.min(left), right, e.bottom) });
    }
    taken.retain(|(l, r, _)| *r > lo && *l < hi);
    if taken.is_empty() {
        return Ok(VecDeque::from([(lo, hi)]));
    }
    let mut spans = vec![(lo, hi)];
    for &(left, right, _) in &taken {
        spans = spans
            .into_iter()
            .flat_map(|(a, b)| if right <= a || left >= b { vec![(a, b)] } else { vec![(a, left.min(b)), (right.max(a), b)] })
            .collect();
    }
    spans.retain(|(a, b)| b - a >= MIN_SPAN);
    if spans.is_empty() {
        // Nothing wide enough: the line goes below the first object that ends.
        return Err(taken.iter().map(|t| t.2).reduce(f32::min).unwrap_or(top));
    }
    Ok(spans.into())
}

/// Word's default hyphenation zone (0.25"), points.
const HYPHENATION_ZONE: f32 = wordcraft_doc::DEFAULT_HYPHENATION_ZONE;

/// How much of their width the spaces of a justified line may give up to fit more text (Word
/// 2013+, compatibility mode 15).
const MAX_SPACE_SHRINK: f32 = 0.2;

/// Take `-slack` (an overflow) out of the line's inner spaces, in proportion to their widths and
/// never more than [`MAX_SPACE_SHRINK`] of them. Trailing spaces hang and are left alone.
fn shrink_line_spaces(pl: &ParaLayout, xs: &mut [f32], c0: usize, c1: usize, content_end: f32, slack: f32) {
    let spaces: Vec<(usize, f32)> = (c0..c1)
        .filter_map(|k| {
            let c = pl.clusters.get(k).filter(|c| c.kind == ClKind::Space)?;
            (xs.get(k - c0).copied().unwrap_or(f32::MAX) < content_end - 0.01).then_some((k, c.adv))
        })
        .collect();
    let total: f32 = spaces.iter().map(|s| s.1).sum();
    if total <= 0.0 || !slack.is_finite() {
        return;
    }
    let ratio = (-slack / total).clamp(0.0, MAX_SPACE_SHRINK);
    let mut take = 0.0;
    let mut next = spaces.iter().peekable();
    for k in c0..c1 {
        if let Some(v) = xs.get_mut(k - c0) {
            *v -= take;
        }
        if let Some((_, adv)) = next.next_if(|s| s.0 == k) {
            take += adv * ratio;
        }
    }
    if let Some(v) = xs.last_mut() {
        *v -= take;
    }
}

/// The hyphen glyph and advance in a style (cached per paragraph).
fn hyphen_glyph(pl: &ParaLayout, style: u32, cache: &mut Vec<(u32, u32, f32)>) -> (u32, u32, f32) {
    if let Some(h) = cache.iter().find(|h| h.0 == style) {
        return *h;
    }
    let h = pl
        .styles
        .get(style as usize)
        .and_then(|st| {
            let g = wordcraft_fonts::shape(&st.face, "-", &[], |c| c).into_iter().next()?;
            let k = st.size / st.face.upem.max(1.0) as f32 * st.rc.scale / 100.0;
            Some((style, g.gid, g.x_advance as f32 * k))
        })
        .unwrap_or((style, 0, 0.0));
    cache.push(h);
    h
}

/// Clusters after which a line may end with a hyphen: soft hyphens always, and dictionary or
/// pattern hyphenation points of each word, in the language of its run ([`ProofRun`]), when
/// automatic hyphenation is on (words in capitals only with `caps`). Both are found in `text`, the paragraph as laid out (without what
/// the layout leaves out, [`left_out`]), and `to_para` maps the end of a char there to the
/// paragraph's text.
fn hyphenation_points(text: &str, to_para: impl Fn(usize) -> Option<usize>, pl: &ParaLayout, auto: bool, caps: bool, runs: &[ProofRun]) -> Vec<u32> {
    let mut bytes: Vec<usize> = text.char_indices().filter(|(_, c)| *c == SOFT_HYPHEN).map(|(i, c)| i + c.len_utf8()).collect();
    // Vietnamese words are single syllables and never take a hyphen: no automatic hyphenation in
    // a Vietnamese paragraph, nor in a word written with a Vietnamese-only letter (`Nguyễn`).
    if auto && !wordcraft_proof::vi::looks_vietnamese(text) {
        let lim = wordcraft_proof::hyphen::Limits::default();
        let mut start: Option<usize> = None;
        for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
            let word_char = c.is_alphabetic() || c == '\'' || c == '\u{2019}';
            match (start, word_char) {
                (None, true) => start = Some(i),
                (Some(a), false) => {
                    // The patterns are for Latin-script languages: never hyphenate Persian/Arabic words.
                    // Words in capitals (`NASA`, `README`) only when the document allows it.
                    let all_caps = |w: &str| w.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
                    if let Some(w) = text.get(a..i)
                        && w.chars().count() >= lim.min_word
                        && !w.chars().any(wordcraft_fonts::is_rtl)
                        && !wordcraft_proof::vi::is_vietnamese_word(w)
                        && (caps || !all_caps(w))
                    {
                        // The word's language: its first letter's run, found by its place in `p.text`.
                        let first = w.chars().next().map_or(1, char::len_utf8);
                        let lang = to_para(a + first).map(|e| lang_at(runs, e.saturating_sub(first))).unwrap_or_default();
                        let offs: Vec<usize> = w.char_indices().map(|(o, _)| o).collect();
                        for pt in wordcraft_proof::hyphen::hyphen_points_in(w, &lim, lang) {
                            if let Some(o) = offs.get(pt) {
                                bytes.push(a + o);
                            }
                        }
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }
    let mut bytes: Vec<usize> = bytes.into_iter().filter_map(to_para).collect();
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes.sort_unstable();
    let mut out: Vec<u32> = pl
        .clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.kind, ClKind::Text | ClKind::Marker) && bytes.binary_search(&c.end).is_ok())
        .map(|(i, _)| i as u32)
        .collect();
    out.dedup();
    out
}

fn pl_text_at(c: &Cluster) -> bool {
    c.dot
}

fn resolve_tab(pl: &mut ParaLayout, pending: &mut Option<(usize, TabStop, f32)>, xs: &mut [f32], c0: usize, x: f32, xo: &mut f32) {
    // Finish a centre/right/decimal tab: its text (possibly empty) ends at `x`.
    let Some((tj, stop, tx)) = pending.take() else { return };
    let seg = x - tx - pl.clusters.get(tj).map(|c| c.adv).unwrap_or(0.0);
    let want = match stop.align {
        TabAlign::Right | TabAlign::Decimal => stop.pos - tx - seg,
        TabAlign::Center => stop.pos - tx - seg / 2.0,
        _ => return,
    }
    .max(0.0);
    let old = pl.clusters.get(tj).map(|c| c.adv).unwrap_or(0.0);
    let delta = want - old;
    if delta.abs() > 0.001 {
        if let Some(c) = pl.clusters.get_mut(tj) {
            c.adv = want;
        }
        for v in xs.iter_mut().skip(tj + 1 - c0) {
            *v += delta;
        }
        *xo += delta;
    }
}

impl ParaLayout {
    /// The line the list label is drawn on: the first, unless the paragraph opens with a page
    /// break, when Word draws it with the text on the next page.
    pub fn label_line(&self) -> usize {
        let opens_with_break = self.lines.len() > 1
            && self.lines.first().is_some_and(|l| {
                l.end == LineEnd::PageBreak
                    && self.clusters.get(l.c0..l.c1).is_some_and(|cs| cs.iter().all(|c| matches!(c.kind, ClKind::PageBreak | ClKind::Marker)))
            });
        usize::from(opens_with_break)
    }
    /// x of byte offset `off` within line `li` (clamped to the line).
    pub fn x_of(&self, li: usize, off: usize) -> Option<f32> {
        let l = self.lines.get(li)?;
        if !l.vis.is_empty() {
            return self.x_of_bidi(l, off);
        }
        for k in l.c0..l.c1 {
            let c = self.clusters.get(k)?;
            if off <= c.start {
                return l.xs.get(k - l.c0).copied();
            }
            if off < c.end {
                return l.xs.get(k - l.c0).copied();
            }
        }
        l.xs.last().copied()
    }
    /// The line holding offset `off` (a position at a wrap point belongs to the next line).
    pub fn line_of(&self, off: usize) -> usize {
        for (i, l) in self.lines.iter().enumerate() {
            let next_start = self.lines.get(i + 1).map(|n| n.start);
            match next_start {
                Some(ns) if off < ns => return i,
                Some(_) => {}
                None => return i,
            }
            if matches!(l.end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) && off < l.stop {
                return i;
            }
        }
        self.lines.len().saturating_sub(1)
    }
    /// Caret x on a bidirectional line: the leading edge of the cluster after `off` (the right
    /// edge of right-to-left text), except where `off` leaves an embedded run (the cluster before
    /// it has the higher level), which keeps the caret at that run's trailing edge, so each
    /// visual boundary of the common mixed cases has its own offset.
    fn x_of_bidi(&self, l: &Line, off: usize) -> Option<f32> {
        let k = (l.c0..l.c1).find(|&k| self.clusters.get(k).is_some_and(|c| off < c.end || off <= c.start));
        let Some(k) = k else { return Some(l.end_x()) };
        let c = self.clusters.get(k)?;
        let here = l.vis.get(k - l.c0)?;
        if off > c.start || k == l.c0 {
            return Some(here.leading());
        }
        let prev = l.vis.get(k - 1 - l.c0)?;
        Some(if prev.level > here.level { prev.trailing() } else { here.leading() })
    }

    /// Visual extents, left to right, of the text between byte offsets `from..to` on line `li`
    /// (one span unless bidirectional text splits the range).
    pub fn x_spans(&self, li: usize, from: usize, to: usize) -> Vec<(f32, f32)> {
        let Some(l) = self.lines.get(li) else { return Vec::new() };
        if l.vis.is_empty() {
            return match (self.x_of(li, from), self.x_of(li, to)) {
                (Some(a), Some(b)) => vec![(a, b)],
                _ => Vec::new(),
            };
        }
        let ks: Vec<usize> = (l.c0..l.c1).filter(|&k| self.clusters.get(k).is_some_and(|c| c.start < to && c.end > from)).collect();
        match (ks.first(), ks.last()) {
            (Some(&a), Some(&b)) => l.spans(a, b + 1),
            _ => Vec::new(),
        }
    }

    /// Caret offsets a click or arrow key can reach on line `li`: each cluster start and the line
    /// end, except after a line-ending break or past a wrapped line's trailing space.
    pub fn line_offsets(&self, li: usize) -> Vec<usize> {
        let Some(l) = self.lines.get(li) else { return Vec::new() };
        let mut v: Vec<usize> = (l.c0..l.c1).filter_map(|k| self.clusters.get(k).map(|c| c.start)).collect();
        let last_break =
            l.c1 > l.c0 && self.clusters.get(l.c1 - 1).is_some_and(|c| matches!(c.kind, ClKind::LineBreak | ClKind::PageBreak | ClKind::ColumnBreak));
        let wrapped = l.c1 > l.c0 && l.end == LineEnd::Wrap && self.lines.get(li + 1).is_some();
        if !last_break && !wrapped || l.c1 == l.c0 {
            v.push(l.stop);
        }
        v
    }

    /// Byte offset closest to x on line `li`.
    pub fn off_at_x(&self, li: usize, x: f32) -> usize {
        let Some(l) = self.lines.get(li) else { return self.text_len };
        if !l.vis.is_empty() {
            let mut best = (l.start, f32::MAX);
            for off in self.line_offsets(li) {
                let d = self.x_of(li, off).map_or(f32::MAX, |cx| (cx - x).abs());
                if d < best.1 {
                    best = (off, d);
                }
            }
            return best.0;
        }
        let mut best = l.start;
        let mut bestd = f32::MAX;
        for k in l.c0..=l.c1 {
            let Some(cx) = l.xs.get(k - l.c0) else { break };
            let off = if k < l.c1 { self.clusters.get(k).map(|c| c.start).unwrap_or(l.stop) } else { l.stop };
            // Don't place the caret after a line-ending break or wrap-trailing space on a wrapped line.
            if k == l.c1 && k > l.c0 {
                let lastc = self.clusters.get(k - 1);
                if lastc.is_some_and(|c| matches!(c.kind, ClKind::LineBreak | ClKind::PageBreak | ClKind::ColumnBreak)) {
                    continue;
                }
                if l.end == LineEnd::Wrap && self.lines.get(li + 1).is_some() {
                    continue;
                }
            }
            let d = (cx - x).abs();
            if d < bestd {
                bestd = d;
                best = off;
            }
        }
        best
    }
    /// Line ascent/descent of the paragraph mark (for empty paragraphs and caret height).
    pub fn caret_metrics(&self, li: usize) -> (f32, f32) {
        let Some(l) = self.lines.get(li) else { return (10.0, 3.0) };
        (l.baseline - l.top, l.top + l.height - l.baseline)
    }
}

/// One run's proofing: its bytes in the paragraph, its language (`w:lang`, resolved through the
/// styles: the document default, paragraph and character styles, direct formatting) and whether
/// it is left unchecked ("Do not check spelling or grammar", hidden text, hyperlinks).
#[derive(Clone, Debug, PartialEq)]
pub struct ProofRun {
    pub range: std::ops::Range<usize>,
    pub lang: wordcraft_proof::ProofLang,
    pub skip: bool,
}

impl ProofRun {
    fn of(range: std::ops::Range<usize>, rc: &ResolvedChar) -> ProofRun {
        ProofRun { range, lang: wordcraft_proof::ProofLang::from_tag(rc.lang.as_deref()), skip: rc.no_proof || rc.hidden || rc.link.is_some() }
    }
}

/// The proofing runs of a paragraph, in order. `table` is the cell's table-style character
/// formatting ([`CellText::chr`]) for a paragraph in a table, as layout resolves it.
pub fn proof_runs(doc: &Document, p: &Paragraph, table: Option<&CharProps>) -> Vec<ProofRun> {
    let style = p.props.style.as_deref();
    p.run_ranges().map(|(r, c)| ProofRun::of(r, &doc.styles.resolve_char_in(style, table, c))).collect()
}

/// The language of the run holding byte `off` (English past the last run).
fn lang_at(runs: &[ProofRun], off: usize) -> wordcraft_proof::ProofLang {
    let k = runs.partition_point(|r| r.range.end <= off);
    runs.get(k).filter(|r| r.range.start <= off).map(|r| r.lang).unwrap_or_default()
}

/// Spelling and grammar issues in a paragraph, in order: each word in its run's language
/// ([`wordcraft_proof::check_text_with`]: what spelling skips, and grammar only with `grammar`),
/// and none touching a run left unchecked.
pub fn paragraph_issues(p: &Paragraph, runs: &[ProofRun], spelling: &wordcraft_proof::Options, grammar: bool) -> Vec<wordcraft_proof::Issue> {
    if p.text.trim().is_empty() || p.text.len() > 100_000 {
        return Vec::new();
    }
    let text = proof_text(p);
    let spans: Vec<(std::ops::Range<usize>, wordcraft_proof::ProofLang)> = runs.iter().map(|r| (r.range.clone(), r.lang)).collect();
    let skip = |a: usize, b: usize| runs.iter().any(|r| r.skip && r.range.start < b && a < r.range.end);
    wordcraft_proof::check_text_with(&text, &spans, spelling, grammar).into_iter().filter(|i| !skip(i.start, i.end)).collect()
}

/// Spelling and grammar issues in a paragraph as squiggles: (start, end, is grammar).
fn proof_issues(p: &Paragraph, runs: &[ProofRun], opts: &crate::ProofOptions) -> Vec<(usize, usize, bool)> {
    paragraph_issues(p, runs, &opts.spelling, opts.grammar)
        .into_iter()
        .map(|i| (i.start, i.end, i.kind == wordcraft_proof::IssueKind::Grammar))
        .collect()
}

/// Text for proofing with the same byte offsets: inline objects and tracked deletions become
/// U+0001 bytes (neither words nor spaces).
pub fn proof_text(p: &wordcraft_doc::Paragraph) -> String {
    let mut out = String::with_capacity(p.text.len());
    for (i, c) in p.text.char_indices() {
        let deleted = c == wordcraft_doc::para::OBJ || p.props_of_char(i).del.is_some();
        if deleted {
            for _ in 0..c.len_utf8() {
                out.push('\u{1}');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod label_spacing_tests {
    use super::*;

    // Inject measured metric pairs after shaping so these regressions do not depend on
    // the fonts installed on the test host. Word for Mac 16.113.3, original Roboto probes.
    fn measured_label_lines(spacing: LineSpacing, label_metrics: (f32, f32), label_ascent: f32, wrap: bool) -> Vec<Line> {
        let doc = Document::new();
        let fields = FieldCtx::default();
        let level = Level { chr: CharProps { size: Some(24.0), ..Default::default() }, suffix: LevelSuffix::Nothing, ..Default::default() };
        let env = ParaEnv {
            doc: &doc,
            fields: &fields,
            width: 100.0,
            label: Some(("•".into(), level.clone())),
            show_hidden: false,
            hide_deleted: false,
            table: None,
            proofing: None,
            field_codes: false,
            grid: Grid::default(),
            exclusions: &[],
            eq_number: 0,
            numeral: crate::NumeralMode::default(),
            read_line_spacing: None,
            fit_objects: false,
        };
        let mut p = Paragraph::with_text(if wrap { "Alpha probe synthetic business text repeated" } else { "Alpha probe" }, Default::default());
        p.props.line_spacing = Some(spacing);
        let mut pl = layout_para(&p, &env);
        let label = pl.label.as_mut().unwrap();
        label.ascent = label_ascent;
        let label_style = label.style;
        for (i, st) in pl.styles.iter_mut().enumerate() {
            (st.ascent, st.descent) = if i == label_style as usize { label_metrics } else { (11.1328125, 2.9296875) };
        }
        let mark_style = pl.clusters[0].style;
        break_lines(&mut pl, &env, mark_style, Some(&level));
        pl.lines
    }

    #[test]
    fn large_label_overflow_is_not_multiplied() {
        for (multiple, expected) in [(1.0, 25.20), (1.25, 28.80), (2.0, 39.36)] {
            let lines = measured_label_lines(LineSpacing::Multiple(multiple), (22.265625, 5.859375), 22.265625, false);
            assert!((lines[0].height - expected).abs() <= 0.25, "{multiple}: {} vs {expected}", lines[0].height);
            if multiple == 1.0 {
                assert!((lines[0].height - lines[0].baseline - 2.9296875).abs() < 0.001, "label descent must not move the text baseline");
            }
        }
    }

    #[test]
    fn small_label_descent_does_not_inflate_text_spacing() {
        let lines = measured_label_lines(LineSpacing::Multiple(1.25), (8.499023, 3.5009766), 9.0, false);
        assert!((lines[0].height - 17.52).abs() <= 0.25, "{}", lines[0].height);
    }

    #[test]
    fn label_leading_and_earlier_font_ascent_are_retained() {
        // Apple Symbols 24 pt: full leading gives an 18 pt label ascent. The text
        // pitch is still Roboto 12 pt. A shorter final fallback run must not hide it.
        for metrics in [(16.998047, 7.001953), (8.0, 3.0)] {
            let lines = measured_label_lines(LineSpacing::Multiple(1.25), metrics, 18.0, false);
            assert!((lines[0].height - 24.48).abs() <= 0.25, "{}", lines[0].height);
        }
    }

    #[test]
    fn minimum_spacing_keeps_text_descent_and_label_leading() {
        for (metrics, ascent, expected) in
            [((22.265625, 5.859375), 22.265625, 25.20), ((8.499023, 3.5009766), 9.0, 13.92), ((16.998047, 7.001953), 18.0, 20.88)]
        {
            let lines = measured_label_lines(LineSpacing::AtLeast(12.0), metrics, ascent, false);
            assert!((lines[0].height - expected).abs() <= 0.25, "{}", lines[0].height);
        }
    }

    #[test]
    fn labels_keep_fixed_and_minimum_spacing_rules() {
        for (spacing, expected) in [(LineSpacing::Exactly(24.0), 24.0), (LineSpacing::AtLeast(30.0), 30.0)] {
            let lines = measured_label_lines(spacing, (22.265625, 5.859375), 22.265625, false);
            assert!((lines[0].height - expected).abs() < 0.001);
        }
    }

    #[test]
    fn label_overflow_only_affects_the_first_wrapped_line() {
        let lines = measured_label_lines(LineSpacing::Multiple(1.25), (22.265625, 5.859375), 22.265625, true);
        assert!(lines.len() > 1);
        assert!((lines[0].height - 28.80).abs() <= 0.25);
        for line in &lines[1..] {
            assert!((line.height - 17.52).abs() <= 0.25, "{}", line.height);
        }
    }
}
