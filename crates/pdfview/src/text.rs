//! The PDF text layer, used for selecting and copying text over a read-only page bitmap.
//!
//! Rather than re-implementing a content-stream interpreter (text matrices, `TJ` kerning, CID
//! encodings and `/ToUnicode` CMaps), this module reuses hayro's interpreter through its public
//! [`Device`] trait. `draw_glyph` is called for every glyph — including invisible text, which
//! hayro documents as the hook for text extraction — with the glyph's Unicode value and the
//! complete transform that placed it. All we add is turning those origins into rectangles and
//! grouping them into reading-order lines.

use hayro::hayro_interpret::font::Glyph;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro::hayro_syntax::page::Page;
use kurbo::{Affine, BezPath, Point, Rect as KurboRect};

use crate::{PageSize, PdfRect, page_transform};

/// Cap on glyphs kept from one page. A hostile content stream can loop forever producing text.
const MAX_CHARS_PER_PAGE: usize = 200_000;
/// Smallest/largest font size we believe, in points.
const MIN_SIZE: f64 = 1.0;
const MAX_SIZE: f64 = 2000.0;
/// Glyphs whose baselines differ by more than this (relative to the font size) start a new line.
const LINE_TOLERANCE: f64 = 0.35;

/// One selectable unit: the Unicode text of a glyph plus the rectangle it occupies, in page
/// points with a top-left origin. `line` is the reading-order row within the page.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfChar {
    pub rect: PdfRect,
    pub text: String,
    pub line: usize,
}

/// A glyph as the interpreter reported it, before rectangles are worked out.
#[derive(Clone, Debug)]
struct RawGlyph {
    x: f64,
    y: f64,
    size: f64,
    text: String,
}

#[derive(Default)]
struct Collector {
    glyphs: Vec<RawGlyph>,
}

impl<'a> Device<'a> for Collector {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, glyph_transform: Affine, _: &Paint<'a>, mode: &GlyphDrawMode) {
        // Fill+Stroke renders the same glyph twice; keep one copy.
        if matches!(mode, GlyphDrawMode::Stroke(_)) || self.glyphs.len() >= MAX_CHARS_PER_PAGE {
            return;
        }
        let Some(uni) = glyph.as_unicode() else { return };
        let text = match uni {
            BfString::Char(c) => c.to_string(),
            BfString::String(s) => s,
        };
        if text.is_empty() || text.chars().all(|c| c.is_control()) {
            return;
        }
        let full = transform * glyph_transform;
        let [a, b, _, d, _, _] = full.as_coeffs();
        // hayro scales glyph outlines by 1/1000 of an em in glyph_transform; undo that to get
        // points. Rotated/vertical runs are outside the horizontal selection model.
        let size_x = (a * a + b * b).sqrt() * 1000.0;
        let size_y = (b * b + d * d).sqrt() * 1000.0;
        if !size_x.is_finite() || !size_y.is_finite() || size_x < MIN_SIZE || size_x > MAX_SIZE || b.abs() > a.abs() * 0.2 {
            return;
        }
        let p = full * Point::new(0.0, 0.0);
        if !p.x.is_finite() || !p.y.is_finite() {
            return;
        }
        self.glyphs.push(RawGlyph { x: p.x, y: p.y, size: size_x, text });
    }
}

fn is_wide(text: &str) -> bool {
    text.chars().next().is_some_and(|c| (c as u32) >= 0x2E80)
}

/// Group glyph origins into lines, then give each one a rectangle. The interpreter's order is
/// usually reading order, but column layouts and figure captions are only correct once sorted
/// by position, so we sort by baseline and then by x.
fn to_chars(mut raw: Vec<RawGlyph>, size: PageSize) -> Vec<PdfChar> {
    raw.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x)));
    let mut lines: Vec<Vec<RawGlyph>> = Vec::new();
    for g in raw {
        let tol = (g.size * LINE_TOLERANCE).max(1.0);
        let fits = lines.last_mut().is_some_and(|line| line.first().is_some_and(|f| (f.y - g.y).abs() <= tol));
        if !fits {
            lines.push(Vec::new());
        }
        if let Some(line) = lines.last_mut() {
            line.push(g);
        }
    }

    let mut out: Vec<PdfChar> = Vec::new();
    for (line_no, line) in lines.into_iter().enumerate() {
        // Some producers draw the same glyph twice with a sub-point offset for a faux bold or
        // shadow effect. Drop the duplicates *before* measuring: otherwise the first glyph's
        // advance would be that 0.6 pt offset instead of its real width.
        let mut clean: Vec<RawGlyph> = Vec::with_capacity(line.len());
        for g in line {
            if clean.last().is_some_and(|p| p.text == g.text && (p.x - g.x).abs() < 0.75) {
                continue;
            }
            clean.push(g);
        }
        for (i, g) in clean.iter().enumerate() {
            let next_x = clean.get(i + 1).map(|n| n.x);
            let advance = next_x.map(|nx| nx - g.x).filter(|d| *d > 0.1 && *d < g.size * 4.0);
            let w = advance.unwrap_or(g.size * if is_wide(&g.text) { 1.0 } else { 0.5 });
            let top = g.y - g.size * 0.85;
            let bottom = g.y + g.size * 0.25;
            let rect = PdfRect::new(g.x as f32, top as f32, (g.x + w) as f32, bottom as f32).clipped_to(size);
            if rect.is_empty() {
                continue;
            }
            out.push(PdfChar { rect, text: g.text.clone(), line: line_no });
            if out.len() >= MAX_CHARS_PER_PAGE {
                return out;
            }
        }
    }
    out
}

/// Extract the text layer of one page. Never panics on malformed content: the caller wraps this
/// in `catch_unwind`, and every derived number is checked before it becomes a rectangle.
pub fn extract(page: &Page<'_>, size: PageSize) -> Vec<PdfChar> {
    let initial = page_transform(page);
    let cache = InterpreterCache::new();
    let mut ctx =
        Context::new(initial, KurboRect::new(0.0, 0.0, f64::from(size.w), f64::from(size.h)), &cache, page.xref(), InterpreterSettings::default());
    let mut device = Collector::default();
    interpret_page(page, &mut ctx, &mut device);
    to_chars(device.glyphs, size)
}

/// Nearest character to a page point, used to anchor a drag selection. Returns its index in the
/// page's text vector. Characters are searched in reading order; the closest centre wins, and a
/// click in the white space above a line still snaps to that line.
pub fn char_at(chars: &[PdfChar], x: f32, y: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, c) in chars.iter().enumerate() {
        let r = &c.rect;
        let dx = (r.x0 - x).max(x - r.x1).max(0.0);
        let dy = (r.y0 - y).max(y - r.y1).max(0.0);
        // Horizontal distance dominates: a click to the right of a line extends that line.
        let d = dx + dy * 2.0;
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}
