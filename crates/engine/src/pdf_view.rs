//! A PDF opened for reading, wrapped for the session and the canvas.
//!
//! The canvas lays out `DocLayout` pages and paints a raster for each. A PDF supplies both: an
//! empty page of the PDF's own size per sheet, and a bitmap rendered on demand. Nothing about a
//! PDF ever enters `Document`, so it cannot be edited, saved or recorded in the undo history.

use std::sync::Arc;

use wordcraft_geom::Rect;
use wordcraft_layout::{DocLayout, Page};
use wordcraft_render::Rendered;

/// The read-only view the session holds while a PDF is open.
pub struct PdfView {
    inner: wordcraft_pdfview::PdfView,
    layout: Arc<DocLayout>,
}

impl PdfView {
    pub fn new(inner: wordcraft_pdfview::PdfView) -> Self {
        let mut pages = Vec::with_capacity(inner.page_count());
        for i in 0..inner.page_count() {
            let Some(size) = inner.page_size(i) else { break };
            pages.push(Page {
                w: size.w,
                h: size.h,
                number: (i as u32).saturating_add(1),
                // No margins: the PDF bitmap covers the whole sheet.
                body: Rect::new(0.0, 0.0, size.w, size.h),
                ..Default::default()
            });
        }
        let layout = DocLayout { pages, ..Default::default() };
        Self { inner, layout: Arc::new(layout) }
    }

    pub fn page_count(&self) -> usize {
        self.inner.page_count()
    }

    /// Page rectangles for `canvas::geometry`: zoom, columns, scrolling and placement all keep
    /// working exactly as they do for a laid-out document.
    pub fn layout(&self) -> Arc<DocLayout> {
        self.layout.clone()
    }

    /// The page as a premultiplied RGBA raster. A page that fails to render becomes an empty
    /// raster rather than an error the UI would have to unwind through (never-crash standard).
    pub fn render(&self, index: usize, scale: f32) -> Rendered {
        match self.inner.render(index, scale) {
            Ok(raster) => Rendered { width: raster.width, height: raster.height, pixels: raster.pixels },
            Err(e) => {
                log::warn!("PDF page {index}: {e}");
                Rendered::default()
            }
        }
    }
}
