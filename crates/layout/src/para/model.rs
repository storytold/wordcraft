use super::grid::{Exclusion, Grid};
use super::*;

/// How a run set in one of Word's symbol fonts (Symbol, Wingdings, Wingdings 2/3, Webdings)
/// shows its characters. Those fonts are proprietary and usually missing: a substitute has other
/// glyphs, or none, at their codes, so each character is drawn as its Unicode equivalent
/// ([`wordcraft_doc::symbol_fonts`]) from a face that has it. The text itself is unchanged.
#[derive(Clone, Copy)]
pub(super) struct SymbolRun<'a> {
    family: &'a str,
    /// The symbol font itself is installed (`face` is it).
    installed: bool,
    face: FaceRef,
}

impl<'a> SymbolRun<'a> {
    pub(super) fn of(family: &'a str, r: &wordcraft_fonts::word::Resolved) -> Option<Self> {
        wordcraft_doc::symbol_fonts::is_symbol_font(family).then_some(SymbolRun { family, installed: !r.substituted, face: r.face })
    }

    /// The character drawn for `c`.
    pub(super) fn show(&self, c: char) -> char {
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

/// Whether a bullet level's `text` has a private-use character that no installed font draws:
/// its font isn't installed (or lacks it) and it isn't a symbol-font code with a Unicode
/// equivalent. Private-use codes mean something different in every font, so another face's
/// glyph there is an unrelated icon, not a bullet.
pub(super) fn undrawable_bullet(text: &str, lvl: &Level, rc: &ResolvedChar) -> bool {
    let private = |c: char| matches!(u32::from(c), 0xE000..=0xF8FF | 0xF0000..=0x10FFFF);
    lvl.format == NumFormat::Bullet
        && text.chars().any(|c| {
            private(c) && wordcraft_doc::symbol_fonts::symbol_char(&rc.font, c).is_none() && {
                let r = wordcraft_fonts::word::resolve(&rc.font, rc.bold, rc.italic);
                r.substituted || !r.face.covers(c)
            }
        })
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

impl StyleRun {
    /// How far the run reaches above and below the line's baseline. Word sizes a superscript
    /// or subscript's line at the full font size and leaves out its raise or drop; only a
    /// `w:position` raise or lowering counts (Word for Mac: an 11pt subscript beside 11pt text
    /// keeps the 12.675pt line, and a line of 24pt subscript is as tall as plain 24pt text).
    pub fn line_extent(&self) -> (f32, f32) {
        let draw = self.rc.draw_size();
        let k = if draw > 0.0 { self.rc.size / draw } else { 1.0 };
        let raise = self.rc.position;
        (self.ascent * k + raise.max(0.0), self.descent * k + (-raise).max(0.0))
    }
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

/// One drawing segment of a grapheme cluster (or object, tab, break).
/// Font/style changes may split its drawing; paragraph edit/wrap boundaries keep it atomic.
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
    /// The text's own ascent and descent on this line, without line-spacing leading (the caret's extent).
    pub text_asc: f32,
    pub text_desc: f32,
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

/// The list label drawn on the first text line (after an opening page break, if any).
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
    /// Which tracked deletions to leave out, like hidden text (all of them in the final, "No
    /// Markup" text).
    pub hide_deleted: crate::HideDeleted,
    /// The table style's formatting for text in this paragraph's cell (`None` outside tables).
    pub table: Option<&'a CellText>,
    /// Check spelling (and grammar, as the options say) for squiggles; `None` = no squiggles.
    pub proofing: Option<crate::ProofOptions>,
    /// The byte range of the word being typed in this paragraph: it gets no spelling squiggle
    /// until the caret leaves it ([`crate::LayoutOptions::typing_word`]).
    pub typing_word: Option<(usize, usize)>,
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
    /// Heights of the paragraph's inline text boxes that resize to fit their text, by object
    /// index: they take these instead of their saved heights.
    pub fitted: &'a [(usize, f32)],
}
