//! Read-only PDF page viewing.
//!
//! PDFs are opened as a *view*, not converted into an editable [`wordcraft_doc::Document`]:
//! each page stays a bitmap, so opening a PDF never makes it editable, saveable or part of an
//! undo history. Parsing (`hayro-syntax`) and rasterising (`hayro` on vello_cpu) are pure Rust,
//! with no native library to ship and no WASM glue, so this crate builds on every WordCraft
//! target including the web.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod links;
mod text;

pub use links::{LinkTarget, PdfLink, link_at};
pub use text::{PdfChar, char_at};

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::object::Rect as PdfBox;
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::{RenderCache, RenderSettings, render};
use kurbo::{Affine, Point};

/// Largest PDF accepted. A PDF is kept in memory for as long as it is open.
pub const MAX_PDF_BYTES: usize = 512 * 1024 * 1024;
/// Cap on the page tree: a hostile file can claim an enormous /Count.
pub const MAX_PAGES: usize = 20_000;
/// Longest side of a page raster, in pixels (matches `wordcraft_render::MAX_SIDE`).
pub const MAX_SIDE: u32 = 16_000;
/// Total page raster budget, in pixels. Oversized requests are scaled down, never refused.
pub const MAX_RASTER_PIXELS: u64 = 100_000_000;
/// Page box limits, in points. PDF allows huge media boxes; clamping keeps the layout sane.
const MAX_SIDE_PT: f32 = 20_000.0;
const MIN_SIDE_PT: f32 = 1.0;

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("not a PDF file")]
    NotPdf,
    #[error("PDF is larger than the {MAX_PDF_BYTES}-byte limit ({0} bytes)")]
    TooLarge(usize),
    #[error("PDF could not be read: {0}")]
    Parse(String),
    #[error("encrypted PDFs are not supported")]
    Encrypted,
    #[error("PDF has no pages")]
    NoPages,
    #[error("PDF has more than {MAX_PAGES} pages")]
    TooManyPages,
    #[error("no PDF page at index {0}")]
    NoSuchPage(usize),
    #[error("PDF page could not be rasterised")]
    Rasterise,
}

/// A page raster: premultiplied RGBA8, row-major, exactly what `wordcraft_render::Rendered`
/// holds. Keeping the type here avoids an L3 → L3 dependency on `wordcraft-render`.
#[derive(Clone, Debug, Default)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A page's dimensions in points, after its /Rotate is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub w: f32,
    pub h: f32,
}

/// A rectangle in page points with a top-left origin — the same coordinate space as the page
/// bitmap, the link annotations and the text layer. Keeping it here avoids a dependency on
/// `wordcraft-geom` (L0) from this L3 crate, and on egui from anything below L6.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PdfRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl PdfRect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }

    /// Transform a PDF rectangle (y-up user space) through a page's initial transform into
    /// y-down page coordinates, then keep only the part inside the page.
    pub fn from_pdf(r: PdfBox, t: Affine, page: PageSize) -> Self {
        let mut min = (f64::INFINITY, f64::INFINITY);
        let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (x, y) in [(r.x0, r.y0), (r.x1, r.y0), (r.x0, r.y1), (r.x1, r.y1)] {
            let p = t * Point::new(x, y);
            if !p.x.is_finite() || !p.y.is_finite() {
                continue;
            }
            min.0 = min.0.min(p.x);
            min.1 = min.1.min(p.y);
            max.0 = max.0.max(p.x);
            max.1 = max.1.max(p.y);
        }
        if !min.0.is_finite() {
            return Self::default();
        }
        Self::new(min.0 as f32, min.1 as f32, max.0 as f32, max.1 as f32).clipped_to(page)
    }

    pub fn clipped_to(self, page: PageSize) -> Self {
        let f = |v: f32, hi: f32| if v.is_finite() { v.clamp(0.0, hi) } else { 0.0 };
        Self::new(f(self.x0, page.w), f(self.y0, page.h), f(self.x1, page.w), f(self.y1, page.h))
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    pub fn intersects(&self, o: &Self) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }

    pub fn is_empty(&self) -> bool {
        self.x1 - self.x0 < 0.05 || self.y1 - self.y0 < 0.05
    }

    pub fn area(&self) -> f32 {
        (self.x1 - self.x0).max(0.0) * (self.y1 - self.y0).max(0.0)
    }
}

/// hayro's own renderer uses this transform (with `invert_y = true`) before scaling; reusing it
/// makes link rectangles and glyph origins line up exactly with the bitmap.
pub(crate) fn page_transform(page: &Page<'_>) -> Affine {
    page.initial_transform(true).to_kurbo()
}

/// An open, read-only PDF. Cheap to share behind an `Arc`; the expensive per-page raster is
/// produced on demand by [`PdfView::render`].
pub struct PdfView {
    pdf: Pdf,
    pages: Vec<PageSize>,
    /// Link annotations, parsed once at open time (cheap: dictionaries only).
    links: Vec<Vec<PdfLink>>,
    /// Text layers are expensive (a full interpretation per page), so extract them on demand
    /// and keep them for the lifetime of the view. `Mutex` gives interior mutability behind the
    /// `Arc` the UI holds without making `PdfView` itself mutable.
    text_cache: Mutex<Vec<Option<Arc<Vec<PdfChar>>>>>,
}

/// Largest copied selection, in characters. A hostile file must not be able to make Ctrl+C
/// allocate without bound.
const MAX_COPY_CHARS: usize = 4_000_000;

impl PdfView {
    /// Parse `bytes` and collect every page's size. Nothing is rasterised yet.
    pub fn open(bytes: Vec<u8>) -> Result<Self, PdfError> {
        if bytes.len() > MAX_PDF_BYTES {
            return Err(PdfError::TooLarge(bytes.len()));
        }
        // A PDF header may be preceded by up to 1024 bytes of junk (some producers add a
        // shebang). Look for the signature in that window rather than only at offset 0.
        let head = bytes.get(..bytes.len().min(1024)).unwrap_or_default();
        if !head.windows(5).any(|w| w == b"%PDF-") {
            return Err(PdfError::NotPdf);
        }
        // Parsing untrusted bytes is the one place a third-party crate could panic; a bad PDF
        // must be an error, never a lost document (never-crash standard).
        let parsed = catch_unwind(AssertUnwindSafe(|| {
            Pdf::new(bytes).map_err(|e| match e {
                LoadPdfError::Decryption(_) => PdfError::Encrypted,
                LoadPdfError::Invalid => PdfError::Parse("invalid PDF structure".into()),
            })
        }));
        let pdf = match parsed {
            Ok(Ok(pdf)) => pdf,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(PdfError::Parse("PDF parser failed".into())),
        };
        let sizes = match catch_unwind(AssertUnwindSafe(|| page_sizes(&pdf))) {
            Ok(r) => r?,
            Err(_) => return Err(PdfError::Parse("PDF page tree is invalid".into())),
        };
        // Annotation dictionaries are pure Rust parsing, but a corrupt page tree can still make
        // hayro panic; keep the same never-crash guarantee as the page-size pass.
        let links = match catch_unwind(AssertUnwindSafe(|| links::parse_links(&pdf, &sizes))) {
            Ok(l) => l,
            Err(_) => {
                log::warn!("PDF link annotations could not be read");
                vec![Vec::new(); sizes.len()]
            }
        };
        let text_cache = (0..sizes.len()).map(|_| None).collect();
        Ok(Self { pdf, pages: sizes, links, text_cache: Mutex::new(text_cache) })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page_size(&self, index: usize) -> Option<PageSize> {
        self.pages.get(index).copied()
    }

    /// The clickable links of a page, in page-point coordinates.
    pub fn links(&self, index: usize) -> &[PdfLink] {
        self.links.get(index).map_or(&[], Vec::as_slice)
    }

    /// The link under a point of a page, if any.
    pub fn link_at(&self, index: usize, x: f32, y: f32) -> Option<LinkTarget> {
        link_at(self.links(index), x, y).cloned()
    }

    /// The page's text layer, in reading order. Extracted once, then cached. A page whose
    /// content stream fails yields an empty (not missing) layer so callers stay simple.
    pub fn text(&self, index: usize) -> Option<Arc<Vec<PdfChar>>> {
        if index >= self.page_count() {
            return None;
        }
        let cache = self.text_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(Some(chars)) = cache.get(index) {
            return Some(chars.clone());
        }
        drop(cache);

        let page = self.pdf.pages().get(index)?;
        let size = self.page_size(index)?;
        let chars = match catch_unwind(AssertUnwindSafe(|| text::extract(page, size))) {
            Ok(c) => Arc::new(c),
            Err(_) => {
                log::warn!("PDF page {index} text could not be extracted");
                Arc::new(Vec::new())
            }
        };
        let mut cache = self.text_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = cache.get_mut(index) {
            *slot = Some(chars.clone());
        }
        Some(chars)
    }

    /// Text covered by a selection spanning two character endpoints (page, character index),
    /// inclusive. Reading order is `(page, char)`; reversed drags are normalised. Line breaks
    /// and page breaks are inserted so the clipboard holds what the reader sees.
    pub fn selection_text(&self, a: (usize, usize), b: (usize, usize)) -> String {
        let (start, end) = if (a.0, a.1) <= (b.0, b.1) { (a, b) } else { (b, a) };
        if start.0 >= self.page_count() {
            return String::new();
        }
        let last_page = end.0.min(self.page_count().saturating_sub(1));
        let mut out = String::new();
        let mut written = 0usize;
        let mut prev: Option<(usize, usize)> = None;
        for p in start.0..=last_page {
            let Some(chars) = self.text(p) else { continue };
            let lo = if p == start.0 { start.1.min(chars.len()) } else { 0 };
            let hi = if p == last_page { end.1.saturating_add(1).min(chars.len()) } else { chars.len() };
            if lo >= hi {
                continue;
            }
            for c in chars.iter().take(hi).skip(lo) {
                if let Some((pp, pl)) = prev
                    && (pp != p || pl != c.line)
                    && !out.ends_with('\n')
                {
                    out.push('\n');
                }
                out.push_str(&c.text);
                written += c.text.chars().count().max(1);
                prev = Some((p, c.line));
                if written >= MAX_COPY_CHARS {
                    return out;
                }
            }
        }
        out
    }

    /// Rasterise page `index` at `scale` pixels per point. The scale is clamped and, if the
    /// resulting bitmap would exceed the side or pixel budgets, reduced proportionally so the
    /// whole page still fits. Returns an empty raster on a parser failure rather than panicking.
    pub fn render(&self, index: usize, scale: f32) -> Result<Raster, PdfError> {
        let size = self.page_size(index).ok_or(PdfError::NoSuchPage(index))?;
        let scale = if scale.is_finite() && scale > 0.0 { scale.clamp(0.05, 8.0) } else { 1.0 };
        let (w, h, eff_scale) = raster_size(size, scale);
        let page = self.pdf.pages().get(index).ok_or(PdfError::NoSuchPage(index))?;
        let settings = RenderSettings {
            x_scale: eff_scale,
            y_scale: eff_scale,
            width: Some(w.min(u32::from(u16::MAX)) as u16),
            height: Some(h.min(u32::from(u16::MAX)) as u16),
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            // A cache per call is slightly slower than one per document but keeps this type
            // free of self-references (RenderCache borrows the page's lifetime).
            let cache = RenderCache::new();
            let pix = render(page, &cache, &InterpreterSettings::default(), &settings);
            Raster { width: u32::from(pix.width()), height: u32::from(pix.height()), pixels: pix.data_as_u8_slice().to_vec() }
        }));
        match result {
            Ok(raster) => Ok(raster),
            Err(_) => {
                log::warn!("PDF page {index} could not be rendered");
                Err(PdfError::Rasterise)
            }
        }
    }
}

fn sane_side(v: f32) -> f32 {
    if v.is_finite() { v.clamp(MIN_SIDE_PT, MAX_SIDE_PT) } else { MIN_SIDE_PT }
}

fn page_sizes(pdf: &Pdf) -> Result<Vec<PageSize>, PdfError> {
    let mut sizes = Vec::new();
    for page in pdf.pages().iter() {
        if sizes.len() >= MAX_PAGES {
            return Err(PdfError::TooManyPages);
        }
        let (w, h) = page.render_dimensions();
        sizes.push(PageSize { w: sane_side(w), h: sane_side(h) });
    }
    if sizes.is_empty() {
        return Err(PdfError::NoPages);
    }
    Ok(sizes)
}

/// Fit a page into the raster budgets, scaling both axes by the same factor so the content and
/// the bitmap never disagree (a clipped page is much worse than a slightly softer one).
fn raster_size(size: PageSize, scale: f32) -> (u32, u32, f32) {
    let dw = f64::from(size.w) * f64::from(scale);
    let dh = f64::from(size.h) * f64::from(scale);
    let side = f64::from(MAX_SIDE);
    let budget = MAX_RASTER_PIXELS as f64;
    let mut factor = 1.0f64;
    if dw > 0.0 {
        factor = factor.min(side / dw);
    }
    if dh > 0.0 {
        factor = factor.min(side / dh);
    }
    if dw > 0.0 && dh > 0.0 {
        factor = factor.min((budget / (dw * dh)).sqrt());
    }
    let factor = if factor.is_finite() { factor.clamp(1e-6, 1.0) } else { 1.0 };
    let eff = f64::from(scale) * factor;
    let w = (f64::from(size.w) * eff).floor().clamp(1.0, side) as u32;
    let h = (f64::from(size.h) * eff).floor().clamp(1.0, side) as u32;
    (w, h, eff as f32)
}

#[cfg(test)]
mod tests;
