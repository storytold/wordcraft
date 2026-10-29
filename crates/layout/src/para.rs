//! Paragraph layout: resolve runs, shape them into clusters, break lines (first-fit, as word
//! processors do), place tabs, list labels and alignment.

use std::collections::VecDeque;
use std::sync::Arc;

use wordcraft_doc::math::MathJc;
use wordcraft_doc::numbering::{Level, LevelSuffix};
use wordcraft_doc::para::{COLUMN_BREAK, InlineObject, LINE_BREAK, NoteKind, OBJ, PAGE_BREAK, PTabAlign, PTabBase, RubyAlign, SOFT_HYPHEN, WrapText};
use wordcraft_doc::props::{Align, CharProps, LineSpacing, ParaProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::resolve::{ResolvedChar, ResolvedPara};
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::{Document, Paragraph};
use wordcraft_fonts::FaceRef;

use crate::NumeralMode;
use crate::fields::{FIELD_SHADING, FieldCtx, field_code_text, field_text_on, reference_link};

mod bidi;
mod builder;
mod chars;
mod grid;
mod justify;
mod lines;
mod model;
mod paragraph;
mod proof;
mod tabs;

pub use bidi::{bidi_levels, visual_order};
pub(crate) use builder::left_out;
#[cfg(test)]
use chars::{MISSING_ROMAN_FONT, lookup_family};
pub(crate) use chars::{caps_char, push_caps};
#[cfg(test)]
pub(crate) use chars::{ligature_features, remap_range, thai_run_face};
pub use grid::{Exclusion, Grid, is_east_asian};
#[cfg(test)]
use lines::break_lines;
pub use model::{CellText, ClKind, Cluster, Enclosed, Glyph, Label, LabelRun, Line, LineEnd, PARA_MARK_WIDTH, ParaEnv, RubyLayout, StyleRun, VisCl};
pub use paragraph::layout_para;
#[cfg(test)]
pub(crate) use paragraph::word_widths;
/// Most lines one paragraph lays out: a guard against hangs on hostile input. A longer
/// paragraph is cut there ([`ParaLayout::truncated`]) and counted in
/// [`crate::DocLayout::truncated_paragraphs`].
pub const MAX_PARA_LINES: usize = 200_000;

#[cfg(any(test, feature = "test-limits"))]
thread_local! {
    static LINE_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_PARA_LINES) };
}

/// The line limit in force ([`MAX_PARA_LINES`], or a test's smaller one).
pub(crate) fn line_limit() -> usize {
    #[cfg(any(test, feature = "test-limits"))]
    return LINE_LIMIT.with(std::cell::Cell::get);
    #[cfg(not(any(test, feature = "test-limits")))]
    MAX_PARA_LINES
}

/// Run `f` with a smaller line limit (tests; other crates' tests through the `test-limits`
/// feature).
#[cfg(any(test, feature = "test-limits"))]
pub fn with_line_limit<T>(limit: usize, f: impl FnOnce() -> T) -> T {
    let old = LINE_LIMIT.with(|c| c.replace(limit));
    let r = f();
    LINE_LIMIT.with(|c| c.set(old));
    r
}

pub use proof::{ProofRun, is_list_item, paragraph_issues, proof_runs, proof_text};

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
    /// Drawing segments that start inside a whole-paragraph grapheme/complex-script cluster.
    /// Font or run formatting can split the drawing, but cannot introduce an edit/wrap stop.
    joined_cluster_starts: Vec<usize>,
    /// Paragraph contains fields whose text depends on the page.
    pub has_page_fields: bool,
    /// Note references (part id) in this paragraph, by cluster index.
    pub notes: Vec<(usize, u32)>,
    /// Proofing issues: (byte start, byte end, grammar?).
    pub issues: Vec<(usize, usize, bool)>,
    /// Drop cap: (clusters at the start that form it, lines it drops, the indent of those lines:
    /// the letter's width and its distance from the text, or 0 in the margin).
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
    /// The paragraph reached [`MAX_PARA_LINES`]: text after its last line isn't laid out.
    pub truncated: bool,
    /// Positional tabs (laid out as tab clusters): cluster index, alignment, base and leader.
    pub ptabs: Vec<(usize, PTabAlign, PTabBase, TabLeader)>,
    /// Byte ranges of the text laid out as nothing ([`left_out`]): hidden text that is not shown,
    /// tracked deletions in the final text. Resolved as laid out, table formatting included.
    pub left_out: Vec<std::ops::Range<usize>>,
    /// The text drawn by clusters that stand for an object (field results, note numbers,
    /// equations) rather than for the paragraph's own text, by cluster index, sorted, with each of
    /// the cluster's glyphs' byte range in that text (a ligature's glyph spans its letters).
    pub shown: Vec<(usize, String, Vec<std::ops::Range<usize>>)>,
    /// Shapes drawn around overstruck field text (enclosed characters, `EQ \o`), by cluster index.
    pub enclosures: Vec<(usize, Enclosed)>,
    /// Room after a cluster between Asian text and Latin text or digits ([`auto_space`]), by
    /// cluster index, sorted. It is part of the cluster's advance, but takes no room at a line's end.
    pub auto_space: Vec<(usize, f32)>,
}

impl ParaLayout {
    /// Returns whether `off` is a legal caret and edit boundary, that is, not the interior start of a
    /// joined grapheme cluster.
    fn is_edit_boundary(&self, off: usize) -> bool {
        self.joined_cluster_starts.binary_search(&off).is_err()
    }

    /// At a legal group start, include every following drawing segment of that grapheme.
    /// Interior segments keep their own advance; they are never independent wrap candidates.
    fn atomic_advance(&self, k: usize) -> f32 {
        let Some(c) = self.clusters.get(k) else { return 0.0 };
        let mut advance = c.adv;
        if self.joined_cluster_starts.is_empty() || !self.is_edit_boundary(c.start) {
            return advance;
        }
        for next in self.clusters.get(k + 1..).into_iter().flatten() {
            if self.is_edit_boundary(next.start) {
                break;
            }
            advance += next.adv;
        }
        advance
    }

    /// The automatic space in cluster `k`'s advance ([`auto_space`]), 0 without one.
    fn auto_space_after(&self, k: usize) -> f32 {
        self.auto_space.binary_search_by_key(&k, |a| a.0).ok().and_then(|i| self.auto_space.get(i)).map_or(0.0, |a| a.1)
    }
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
        let off = if self.is_edit_boundary(off) {
            off
        } else {
            // Explicit offsets at a font boundary inside a grapheme use its leading edge too.
            let k = self.clusters.partition_point(|c| c.start < off);
            self.clusters.get(..k).into_iter().flatten().rev().find(|c| self.is_edit_boundary(c.start)).map_or(0, |c| c.start)
        };
        let mut from = l.c0;
        self.x_from(l, off, &mut from)
    }
    /// [`Self::x_of`] on line `l`, looking for the cluster at `off` from cluster `*from` on (the
    /// clusters before it must all end at or before `off`: so they do when `from` was left there
    /// by an offset no greater), and leaving `from` at the cluster found. Walks a line's carets
    /// in order in one pass over its clusters.
    fn x_from(&self, l: &Line, off: usize, from: &mut usize) -> Option<f32> {
        if !l.vis.is_empty() {
            return self.x_of_bidi(l, off, from);
        }
        for k in *from..l.c1 {
            crate::work::steps(1);
            let c = self.clusters.get(k)?;
            if off <= c.start || off < c.end {
                *from = k;
                return l.xs.get(k - l.c0).copied();
            }
        }
        *from = l.c1;
        l.xs.last().copied()
    }
    /// Each caret offset on line `li` ([`Self::line_offsets`]) with its x ([`Self::x_of`]).
    pub fn line_carets(&self, li: usize) -> Vec<(usize, Option<f32>)> {
        let Some(l) = self.lines.get(li) else { return Vec::new() };
        let (mut from, mut last) = (l.c0, 0);
        self.line_offsets(li)
            .into_iter()
            .map(|off| {
                if off < last {
                    from = l.c0;
                }
                last = off;
                (off, self.x_from(l, off, &mut from))
            })
            .collect()
    }
    /// The farthest right a caret goes on left-to-right line `li`: spaces at the end of a line
    /// hang past its right edge, but the caret after them stays at the edge, as in Word (#2146);
    /// text that overflows the line still carries it. `None` on bidirectional lines.
    pub fn caret_limit(&self, li: usize) -> Option<f32> {
        let l = self.lines.get(li)?;
        if !l.vis.is_empty() {
            return None;
        }
        let ink = (l.c0..l.c1).rev().find(|&k| self.clusters.get(k).is_some_and(|c| c.kind != ClKind::Space));
        let end = match ink {
            Some(k) => l.xs.get(k + 1 - l.c0).copied(),
            None => l.xs.first().copied(),
        };
        Some(end.unwrap_or(l.left).max(l.right))
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
    fn x_of_bidi(&self, l: &Line, off: usize, from: &mut usize) -> Option<f32> {
        let k = (*from..l.c1).find(|&k| {
            crate::work::steps(1);
            self.clusters.get(k).is_some_and(|c| off < c.end || off <= c.start)
        });
        *from = k.unwrap_or(l.c1);
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

    /// Caret offsets a click or arrow key can reach on line `li`: whole-grapheme starts and the
    /// line end, except after a line-ending break or past a wrapped line's trailing space.
    pub fn line_offsets(&self, li: usize) -> Vec<usize> {
        let Some(l) = self.lines.get(li) else { return Vec::new() };
        let mut v: Vec<usize> =
            (l.c0..l.c1).filter_map(|k| self.clusters.get(k).filter(|c| self.is_edit_boundary(c.start)).map(|c| c.start)).collect();
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
            for (off, cx) in self.line_carets(li) {
                let d = cx.map_or(f32::MAX, |cx| (cx - x).abs());
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
            if !self.is_edit_boundary(off) {
                continue;
            }
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
    /// The caret's ascent/descent on a line: the text's own, without line-spacing leading, kept inside the line box.
    pub fn caret_metrics(&self, li: usize) -> (f32, f32) {
        let Some(l) = self.lines.get(li) else { return (10.0, 3.0) };
        (l.text_asc.min(l.baseline - l.top), l.text_desc.min(l.top + l.height - l.baseline))
    }
}

#[cfg(test)]
mod label_spacing_tests {
    use super::*;

    const ROBOTO_METRICS: (f32, f32) = (11.1328125, 2.9296875);

    // Inject measured metric pairs after shaping so these regressions do not depend on
    // the fonts installed on the test host. Word for Mac 16.113.3, original Roboto probes.
    fn measured_label_lines(spacing: LineSpacing, label_metrics: (f32, f32), label_ascent: f32, wrap: bool, text_metrics: (f32, f32)) -> Vec<Line> {
        let doc = Document::new();
        let fields = FieldCtx::default();
        let level = Level { chr: CharProps { size: Some(24.0), ..Default::default() }, suffix: LevelSuffix::Nothing, ..Default::default() };
        let env = ParaEnv {
            doc: &doc,
            fields: &fields,
            width: 100.0,
            label: Some(("•".into(), level.clone())),
            show_hidden: false,
            hide_deleted: Default::default(),
            table: None,
            proofing: None,
            typing_word: None,
            field_codes: false,
            grid: Grid::default(),
            exclusions: &[],
            eq_number: 0,
            numeral: crate::NumeralMode::default(),
            read_line_spacing: None,
            fit_objects: false,
            fitted: &[],
        };
        let mut p = Paragraph::with_text(if wrap { "Alpha probe synthetic business text repeated" } else { "Alpha probe" }, Default::default());
        p.props.line_spacing = Some(spacing);
        let mut pl = layout_para(&p, &env);
        let label = pl.label.as_mut().unwrap();
        label.ascent = label_ascent;
        let label_style = label.style;
        for (i, st) in pl.styles.iter_mut().enumerate() {
            (st.ascent, st.descent) = if i == label_style as usize { label_metrics } else { text_metrics };
        }
        let mark_style = pl.clusters[0].style;
        break_lines(&mut pl, &env, mark_style, Some(&level));
        pl.lines
    }

    #[test]
    fn fallback_label_ascent() {
        let body = (1921.0 / 2048.0 * 11.0, 434.0 / 2048.0 * 11.0);
        let low = measured_label_lines(LineSpacing::Multiple(1.15), (9.802246, 2.379395), 9.802246, false, body);
        let tall = measured_label_lines(LineSpacing::Multiple(1.15), (11.0, 3.586), 11.0, false, body);
        assert!((low[0].height - 14.546265).abs() < 0.001);
        assert!((tall[0].height - low[0].height - 0.682129).abs() < 0.001);
    }

    #[test]
    fn large_label_overflow_is_not_multiplied() {
        for (multiple, expected) in [(1.0, 25.20), (1.25, 28.80), (2.0, 39.36)] {
            let lines = measured_label_lines(LineSpacing::Multiple(multiple), (22.265625, 5.859375), 22.265625, false, ROBOTO_METRICS);
            assert!((lines[0].height - expected).abs() <= 0.25, "{multiple}: {} vs {expected}", lines[0].height);
            if multiple == 1.0 {
                assert!((lines[0].height - lines[0].baseline - 2.9296875).abs() < 0.001, "label descent must not move the text baseline");
            }
        }
    }

    #[test]
    fn small_label_descent_does_not_inflate_text_spacing() {
        let lines = measured_label_lines(LineSpacing::Multiple(1.25), (8.499023, 3.5009766), 9.0, false, ROBOTO_METRICS);
        assert!((lines[0].height - 17.52).abs() <= 0.25, "{}", lines[0].height);
    }

    #[test]
    fn label_leading_and_earlier_font_ascent_are_retained() {
        // Apple Symbols 24 pt: full leading gives an 18 pt label ascent. The text
        // pitch is still Roboto 12 pt. A shorter final fallback run must not hide it.
        for metrics in [(16.998047, 7.001953), (8.0, 3.0)] {
            let lines = measured_label_lines(LineSpacing::Multiple(1.25), metrics, 18.0, false, ROBOTO_METRICS);
            assert!((lines[0].height - 24.48).abs() <= 0.25, "{}", lines[0].height);
        }
    }

    #[test]
    fn minimum_spacing_keeps_text_descent_and_label_leading() {
        for (metrics, ascent, expected) in
            [((22.265625, 5.859375), 22.265625, 25.20), ((8.499023, 3.5009766), 9.0, 13.92), ((16.998047, 7.001953), 18.0, 20.88)]
        {
            let lines = measured_label_lines(LineSpacing::AtLeast(12.0), metrics, ascent, false, ROBOTO_METRICS);
            assert!((lines[0].height - expected).abs() <= 0.25, "{}", lines[0].height);
        }
    }

    #[test]
    fn labels_keep_fixed_and_minimum_spacing_rules() {
        for (spacing, expected) in [(LineSpacing::Exactly(24.0), 24.0), (LineSpacing::AtLeast(30.0), 30.0)] {
            let lines = measured_label_lines(spacing, (22.265625, 5.859375), 22.265625, false, ROBOTO_METRICS);
            assert!((lines[0].height - expected).abs() < 0.001);
        }
    }

    #[test]
    fn label_overflow_only_affects_the_first_wrapped_line() {
        let lines = measured_label_lines(LineSpacing::Multiple(1.25), (22.265625, 5.859375), 22.265625, true, ROBOTO_METRICS);
        assert!(lines.len() > 1);
        assert!((lines[0].height - 28.80).abs() <= 0.25);
        for line in &lines[1..] {
            assert!((line.height - 17.52).abs() <= 0.25, "{}", line.height);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ASCII shortcuts give the bidi class's answer for every character.
    #[test]
    fn strong_and_mark_shortcuts_match_the_bidi_class() {
        use unicode_bidi::BidiClass::{AL, BN, L, NSM, R};
        for c in (0..=0x10ffff).filter_map(char::from_u32) {
            let class = unicode_bidi::bidi_class(c);
            let strong = match class {
                R | AL => Some(true),
                L => Some(false),
                _ => None,
            };
            assert_eq!(bidi::strong_rtl(c), strong, "{:04x}", c as u32);
            assert_eq!(chars::is_mark(c), matches!(class, NSM | BN) && !c.is_control(), "{:04x}", c as u32);
        }
    }

    #[test]
    fn symbols_ignore_roman_hint() {
        let mut d = Document::new();
        d.settings.roman_fonts = vec!["Symbol (PCL6)".into(), "Wingdings".into(), "No Such Roman Font".into()];
        for family in ["Symbol (PCL6)", "Wingdings"] {
            assert_eq!(lookup_family(&d, family), family);
        }
        assert_eq!(lookup_family(&d, "No Such Roman Font"), MISSING_ROMAN_FONT);
    }
}
