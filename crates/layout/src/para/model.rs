use super::grid::{Exclusion, Grid};
use super::{Arc, CharProps, Document, FaceRef, FieldCtx, Level, NumFormat, NumeralMode, ParaProps, ResolvedChar, TabLeader};

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
        let u = wordcraft_doc::symbol_fonts::symbol_char(self.family, c).unwrap_or(c);
        // Common symbol-font bullets, arrows, boxes and check marks have widely covered
        // look-alikes: use the first covered one when no loaded or system face has `u`.
        // Also applies when an exporter writes the mapped Unicode code point directly inside a
        // Wingdings/Symbol run.
        let alts: &[char] = match u {
            '\u{26AB}' | '\u{2B24}' => &['\u{25CF}'],
            '\u{25FC}' | '\u{25FE}' | '\u{2B1B}' => &['\u{25A0}'],
            '\u{2B9A}' => &['\u{27A2}', '\u{25BA}', '\u{25B8}'],
            '\u{27A2}' => &['\u{25BA}', '\u{25B8}'],
            '\u{2B98}' => &['\u{25C4}', '\u{25C2}'],
            '\u{2B99}' => &['\u{25B2}'],
            '\u{2B9B}' => &['\u{25BC}'],
            '\u{274F}' | '\u{2751}' | '\u{2752}' | '\u{2B1C}' => &['\u{25A1}'],
            '\u{1F78E}' | '\u{1F78F}' => &['\u{25A1}', '\u{25FB}'],
            '\u{2756}' => &['\u{25C6}'],
            '\u{1F868}' | '\u{1F878}' | '\u{21E6}' | '\u{2B88}' => &['\u{2190}'],
            '\u{1F869}' | '\u{1F879}' | '\u{21E7}' | '\u{2B89}' => &['\u{2191}'],
            '\u{1F86A}' | '\u{1F87A}' | '\u{21E8}' | '\u{2B8A}' => &['\u{2192}'],
            '\u{1F86B}' | '\u{1F87B}' | '\u{21E9}' | '\u{2B8B}' => &['\u{2193}'],
            '\u{1F5F8}' => &['\u{2713}', '\u{2714}', '\u{221A}'],
            '\u{1F5F9}' => &['\u{2611}', '\u{2713}', '\u{2714}'],
            '\u{2611}' => &['\u{2713}', '\u{2714}'],
            '\u{1F5F5}' | '\u{1F5F7}' => &['\u{2612}', '\u{2717}', '\u{00D7}'],
            '\u{2612}' => &['\u{2717}', '\u{00D7}'],
            '\u{1F5F6}' => &['\u{2717}', '\u{00D7}'],
            '\u{2217}' => &['*'],
            _ => &[],
        };
        if alts.is_empty() {
            return u;
        }
        let has = |ch: char| self.face.covers(ch) || wordcraft_fonts::FontDb::global().fallback_for(ch, self.face.id()).is_some();
        if has(u) {
            return u;
        }
        alts.iter().copied().find(|&a| has(a)).unwrap_or(u)
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
    /// The face the run's font resolves to, which `face` replaces where it lacks a character (a
    /// fallback face, the East Asian font for CJK): the face new text typed in the run asks for.
    pub requested_face: FaceRef,
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
        let (mut a, mut d) = (self.ascent * k + raise.max(0.0), self.descent * k + (-raise).max(0.0));
        // Emphasis marks sit above the characters (or below), and the line makes room for them
        // (see `display::lines::emphasis_mark`).
        let size = if self.rc.size.is_finite() { self.rc.size.clamp(0.0, 1638.0) } else { 0.0 };
        match self.rc.emphasis {
            wordcraft_doc::props::EmphasisMark::None => {}
            wordcraft_doc::props::EmphasisMark::UnderDot => d = d.max(size * 0.42 + (-raise).max(0.0)),
            _ => a = a.max(size * 1.32 + raise.max(0.0)),
        }
        (a, d)
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
    /// The paragraph goes on from the line of the one before, whose mark is hidden (#3986): its
    /// list number is counted and sets its indents, but isn't drawn.
    pub joined: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_run_lookalikes_preserve_shape_and_checkbox_semantics() {
        crate::test_fonts();
        let resolved = wordcraft_fonts::word::resolve("Wingdings", false, false);
        let run = SymbolRun::of("Wingdings", &resolved).expect("Wingdings is a symbol font");
        let run2 = SymbolRun::of("Wingdings 2", &resolved).expect("Wingdings 2 is a symbol font");

        // U+1F78F (MEDIUM WHITE SQUARE, Wingdings 0x6F) and U+1F78E (LIGHT WHITE SQUARE, 0xA8)
        // must stay squares, never map to diamonds.
        let squares = ['\u{1F78E}', '\u{1F78F}', '\u{25A1}', '\u{25FB}'];
        for c in ['\u{F06F}', '\u{F0A8}', '\u{1F78F}', '\u{1F78E}'] {
            let shown = run.show(c);
            assert!(squares.contains(&shown), "square {c:?} mapped to non-square {shown:?}");
            assert_ne!(shown, '\u{25C6}');
            assert_ne!(shown, '\u{25C7}');
        }

        // U+2756 (Wingdings 0x76) is a diamond and stays a diamond.
        let shown_diamond = run.show('\u{F076}');
        assert!(matches!(shown_diamond, '\u{2756}' | '\u{25C6}'), "diamond mapped to {shown_diamond:?}");

        // Checked ballot boxes must stay checked/checkmark, never degrade to an unchecked box (□).
        let checked = ['\u{1F5F9}', '\u{2611}', '\u{2713}', '\u{2714}'];
        for c in ['\u{F0FE}', '\u{1F5F9}', '\u{2611}'] {
            let shown = run.show(c);
            assert!(checked.contains(&shown), "checked box {c:?} mapped to {shown:?}");
            assert_ne!(shown, '\u{25A1}', "checked box {c:?} must never degrade to unchecked □");
        }

        // Crossed ballot boxes must stay crossed/X, never degrade to an unchecked box (□).
        let crossed = ['\u{1F5F7}', '\u{1F5F5}', '\u{2612}', '\u{2717}', '\u{00D7}'];
        for (r, c) in [(run, '\u{F0FD}'), (run2, '\u{F051}'), (run, '\u{1F5F7}'), (run, '\u{1F5F5}'), (run, '\u{2612}')] {
            let shown = r.show(c);
            assert!(crossed.contains(&shown), "crossed box {c:?} mapped to {shown:?}");
            assert_ne!(shown, '\u{25A1}', "crossed box {c:?} must never degrade to unchecked □");
        }
    }
}
