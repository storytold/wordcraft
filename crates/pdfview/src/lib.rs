//! Read-only PDF page viewing.
//!
//! PDFs are opened as a *view*, not converted into an editable [`wordcraft_doc::Document`]:
//! each page stays a bitmap, so opening a PDF never makes it editable, saveable or part of an
//! undo history. Parsing (`hayro-syntax`) and rasterising (`hayro` on vello_cpu) are pure Rust,
//! with no native library to ship and no WASM glue, so this crate builds on every WordCraft
//! target including the web.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::{RenderCache, RenderSettings, render};

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

/// An open, read-only PDF. Cheap to share behind an `Arc`; the expensive per-page raster is
/// produced on demand by [`PdfView::render`].
pub struct PdfView {
    pdf: Pdf,
    pages: Vec<PageSize>,
}

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
        Ok(Self { pdf, pages: sizes })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page_size(&self, index: usize) -> Option<PageSize> {
        self.pages.get(index).copied()
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
