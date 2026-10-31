use super::builder::Builder;
use super::model::ClKind;
use super::{Arc, CharProps, Document, WrapText};

/// The section's document grid as layout applies it (`w:docGrid`, ECMA-376 §17.6.5).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Grid {
    /// Line pitch, points: line heights snap to multiples of it.
    pub line: Option<f32>,
    /// Character pitch at the default font size, points.
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

    /// Multiple spacing scales the grid pitch, while tall text retains its required rows.
    /// Word: on a 14.35 pt grid, 11 pt Japanese at 1.5 lines takes 21.525 pt, whereas
    /// 12/16 pt text takes 28.7 pt at 0.5, 1.0 and 1.5 lines alike.
    pub(super) fn multiple_height(&self, h: f32, multiple: f32) -> f32 {
        self.snap_height(h).max(self.line.unwrap_or(h) * multiple)
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

/// Word's enabled font kerning removes redundant half-em side bearings between Japanese
/// punctuation, including across run boundaries. This is separate from line-end compression.
/// Apply before the document grid, and only to full-em clusters: a font's own half-width or
/// already-kerned punctuation must not be tightened twice. Whitespace and objects break pairs.
pub(super) fn kern_punctuation(b: &mut Builder, text: &str, base: usize, from: usize) {
    let opening = |c: char| "〈《「『【〔〖〘〚（［｛".contains(c);
    let closing = |c: char| "〉》」』】〕〗〙〛）］｝、。，．".contains(c);
    let mut prev: Option<(usize, char, f32)> = None;
    for i in from..b.clusters.len() {
        let Some(c) = b.clusters.get(i) else { break };
        let item = (|| {
            let st = b.styles.get(c.style as usize)?;
            if c.kind != ClKind::Text || !st.rc.kerned() || st.rc.fit_text.is_some_and(|f| f.width > 0.0) {
                return None;
            }
            let mut chars = text.get(c.start.checked_sub(base)?..c.end.checked_sub(base)?)?.chars();
            let ch = chars.next()?;
            let em = st.size * st.rc.scale / 100.0;
            let bold = if st.synth_bold { em * wordcraft_fonts::word::BOLD_WIDTH_EM } else { 0.0 };
            let tracking = st.rc.spacing * if b.env.doc.settings.balance_single_byte_double_byte_width { 2.0 } else { 1.0 };
            // Compare the underlying shaped width with an em so a font's own half-width
            // kerning prevents a second compression. Word's punctuation bearing includes
            // tracking: at 12 pt, each pair loses 5.5/6/6.5 pt with -1/0/+1 pt spacing.
            (chars.next().is_none() && (opening(ch) || closing(ch)) && em > 0.0 && (c.adv - tracking - bold - em).abs() < em * 0.01)
                .then_some((ch, (em + tracking).max(0.0)))
        })();
        let Some((ch, cell)) = item else {
            prev = None;
            continue;
        };
        if let Some((pi, pc, pcell)) = prev
            && (closing(pc) || opening(ch))
            && let Some(p) = b.clusters.get_mut(pi)
        {
            p.adv -= pcell.min(cell) * 0.5;
        }
        prev = Some((i, ch, cell));
    }
}

/// Character grid: `linesAndChars` adds the grid's character-spacing delta to each cluster's
/// own advance. The legacy width-balancing setting halves that delta for non-East-Asian text.
/// In particular, a run larger than Normal must not be squeezed into Normal's cell while its
/// glyph stays full size. `snapToChars` instead centres characters in whole cells; Latin text
/// stays proportional unless everything snaps.
pub(super) fn snap_chars(b: &mut Builder, text: &str, grid: &Grid) {
    let Some(pitch) = grid.chars else { return };
    let base = grid.font_size.max(1.0);
    for c in b.clusters.iter_mut() {
        if !matches!(c.kind, ClKind::Text | ClKind::Space) || c.adv <= 0.0 {
            continue;
        }
        // A manual width controls this group's glyphs and advances together; applying
        // the character grid first would squeeze the advances without the outlines.
        if b.styles.get(c.style as usize).is_some_and(|st| !st.rc.snap_to_grid || st.rc.fit_text.is_some_and(|f| f.width > 0.0)) {
            continue;
        }
        let Some(ch) = text.get(c.start..).and_then(|t| t.chars().next()) else { continue };
        let east_asian = is_east_asian(ch);
        if grid.snap_all && !east_asian && c.kind != ClKind::Text {
            continue;
        }
        let adv = if grid.snap_all {
            let size = b.styles.get(c.style as usize).map_or(base, |st| st.size);
            let cells = if east_asian { (size / base).round() } else { (c.adv / pitch - 0.01).ceil() }.clamp(1.0, 64.0);
            cells * pitch
        } else {
            // `charSpace` is a delta in 1/4096 pt from Normal's advance, not an absolute
            // width for every run (ECMA-376 §17.6.5). Word applies it to Latin letters and
            // spaces too; legacy width balancing uses half the delta for those clusters.
            let delta = pitch - base;
            let delta = if !east_asian && b.env.doc.settings.balance_single_byte_double_byte_width { delta / 2.0 } else { delta };
            (c.adv + delta).clamp(0.1, 1e6)
        };
        // An additive character-spacing delta changes the next origin, not this
        // glyph's origin. Only a fixed character cell centres its outline.
        let shift = if grid.snap_all { (adv - c.adv) / 2.0 } else { 0.0 };
        for g in b.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
            g.dx += shift;
        }
        c.adv = adv;
    }
}

/// Automatic spacing (`w:autoSpaceDE`, `w:autoSpaceDN`): a quarter of the Asian character's font
/// size between Asian text and Latin text or digits that touch, across runs. Measured on Word's
/// PDF of a Chinese document at 16 pt (#2358), as JIS X 4051 asks. A field or note number is
/// classed by the text it shows. The room goes after the first of the two characters; returns it
/// by cluster index.
pub(super) fn auto_space(b: &mut Builder, text: &str, de: bool, dn: bool) -> Vec<(usize, f32)> {
    auto_space_range(b, text, 0, 0, de, dn)
}

/// Apply the same boundary spacing before an atomic result is merged into one cluster.
/// Whitespace ends adjacency, so explicit and repeated spaces retain their shaped widths.
pub(super) fn auto_space_range(b: &mut Builder, text: &str, base: usize, from: usize, de: bool, dn: bool) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    if !de && !dn {
        return out;
    }
    let shown_start = b.shown.partition_point(|s| s.0 < from);
    let mut shown = b.shown.get(shown_start..).unwrap_or(&[]).iter().map(|s| (s.0, s.1.as_str())).peekable();
    // The previous text cluster: index, its last character, its font size.
    let mut prev: Option<(usize, char, f32, bool)> = None;
    for k in from..b.clusters.len() {
        let Some(c) = b.clusters.get(k) else { break };
        match c.kind {
            ClKind::Text => {}
            ClKind::Marker if c.adv == 0.0 => continue,
            _ => {
                prev = None;
                continue;
            }
        }
        while shown.next_if(|s| s.0 < k).is_some() {}
        let s = match shown.peek() {
            Some(&(i, t)) if i == k => t,
            _ => c.start.checked_sub(base).zip(c.end.checked_sub(base)).and_then(|(a, z)| text.get(a..z)).unwrap_or(""),
        };
        let (Some(first), Some(last)) = (s.chars().next(), s.chars().next_back()) else {
            prev = None;
            continue;
        };
        let size = b.styles.get(c.style as usize).map_or(0.0, |st| st.size);
        let on_grid = b.env.grid.chars.is_some() && b.styles.get(c.style as usize).is_some_and(|st| st.rc.snap_to_grid);
        if let Some((pk, pc, psize, previous_on_grid)) = prev
            && !(b.env.grid.snap_all && on_grid && previous_on_grid)
            && let Some(side) = crate::kinsoku::auto_space(pc, first, de, dn)
        {
            let asian = if side == crate::kinsoku::AsianSide::Left { psize } else { size };
            let gap = if asian.is_finite() { (asian / 4.0).clamp(0.0, 400.0) } else { 0.0 };
            if let Some(pc) = b.clusters.get_mut(pk) {
                pc.adv += gap;
                out.push((pk, gap));
            }
        }
        prev = Some((k, last, size, on_grid));
    }
    out
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
    /// A floating table counts a line touching its top as overlapping; shapes do not (#3977).
    pub table: bool,
}

impl Exclusion {
    /// Whether this area keeps text out of the row, including exact top contact for tables.
    pub(super) fn intersects_row(&self, top: f32, h: f32, table_h: f32, lo: f32, hi: f32) -> bool {
        let reaches = if self.table { self.top <= top + table_h } else { self.top < top + h };
        self.bottom > top && reaches && self.right > lo && self.left < hi
    }
}

/// Text Word sets in the East Asian font: CJK ideographs, kana, bopomofo, CJK punctuation and
/// fullwidth forms, and Hangul.
pub(super) fn east_asian(c: char) -> bool {
    crate::kinsoku::is_east_asian(c) || matches!(u32::from(c), 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF | 0x203B | 0x25CB | 0x25CE)
}
