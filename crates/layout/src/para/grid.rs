use super::builder::Builder;
use super::model::ClKind;
use super::{Arc, CharProps, Document, WrapText};

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
pub(super) fn snap_chars(b: &mut Builder, text: &str, grid: &Grid) {
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

/// Automatic spacing (`w:autoSpaceDE`, `w:autoSpaceDN`): a quarter of the Asian character's font
/// size between Asian text and Latin text or digits that touch, across runs. Measured on Word's
/// PDF of a Chinese document at 16 pt (#2358), as JIS X 4051 asks. A field or note number is
/// classed by the text it shows. The room goes after the first of the two characters; returns it
/// by cluster index.
pub(super) fn auto_space(b: &mut Builder, text: &str, de: bool, dn: bool) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    if !de && !dn {
        return out;
    }
    let mut shown = b.shown.iter().map(|s| (s.0, s.1.as_str())).peekable();
    // The previous text cluster: index, its last character, its font size.
    let mut prev: Option<(usize, char, f32)> = None;
    for k in 0..b.clusters.len() {
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
            _ => text.get(c.start..c.end).unwrap_or(""),
        };
        let (Some(first), Some(last)) = (s.chars().next(), s.chars().next_back()) else {
            prev = None;
            continue;
        };
        let size = b.styles.get(c.style as usize).map_or(0.0, |st| st.size);
        if let Some((pk, pc, psize)) = prev
            && let Some(side) = crate::kinsoku::auto_space(pc, first, de, dn)
        {
            let asian = if side == crate::kinsoku::AsianSide::Left { psize } else { size };
            let gap = if asian.is_finite() { (asian / 4.0).clamp(0.0, 400.0) } else { 0.0 };
            if let Some(pc) = b.clusters.get_mut(pk) {
                pc.adv += gap;
                out.push((pk, gap));
            }
        }
        prev = Some((k, last, size));
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
    crate::kinsoku::is_east_asian(c) || matches!(u32::from(c), 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF)
}
