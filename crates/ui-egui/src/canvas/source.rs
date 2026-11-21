//! The document's pages as the page view's source: sizes from the layout, pictures rendered by the raster
//! renderer and versioned by what the pages show.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::behavior::kurbo::Size;
use craft::pages::ui::PageSource;
use craft::ui::primitives::textures::{LoadedPicture, Revision, ThumbRender, ThumbRequest, ThumbSource};
use wordcraft_doc::{Document, Pos};
use wordcraft_layout::DocLayout;

use super::WordApp;
use super::pages::{PageKeys, lcd_for, page_doc, page_render_options, page_shows, picture_pixels, raster_scale, view_key};

/// A page dimension (points) safe to lay out: finite, at least 1 pt, at most `cap`.
fn dim(v: f32, cap: f32) -> f64 {
    f64::from(if v.is_finite() { v.clamp(1.0, cap) } else { 72.0 })
}

/// The pages of one pass's layout. It owns what it reads (the page view holds it while the body edits the app), so
/// the document is a copy taken for the pass: its blocks, styles and numbering are shared, not duplicated.
pub(super) struct WordPages {
    doc: Arc<Document>,
    layout: Arc<DocLayout>,
    keys: RefCell<PageKeys>,
    view: u64,
    options: wordcraft_render::RenderOptions,
    /// The page a drag took an object off, and the object: rendered without it.
    hidden: Option<(usize, Pos)>,
    /// Screen points per document point in the view these pages show in, when it is one whose pages are drawn at
    /// that scale (the page view); `None` where the view sizes its pages itself (Read Mode's columns). With the
    /// screen's density it gives the scale at which a picture is the screen's own ([`raster_scale`]).
    view_scale: Option<f32>,
    /// Where the page view put each page (page, its place in screen points) the last time the view was at rest;
    /// empty for a view that sizes its pages itself. A picture is made exactly as many pixels each way as its
    /// place is ([`picture_pixels`]).
    placed: Vec<(usize, egui::Rect)>,
    /// File › Options › General › Screen text smoothing, and what the system says of the screen's text.
    smoothing: wordcraft_engine::cmd::view::TextSmoothing,
    screen_text: crate::screen_text::ScreenText,
}

/// The page view asks for pictures whose longest side is a multiple of this many pixels
/// (`ThumbRequest::side`).
const SIDE_STEP: u32 = 16;

impl WordPages {
    /// The source of `layout` for this pass. The page fingerprints move out of the app for the pass; give them back
    /// with [`WordPages::into_keys`].
    /// `view_scale` is the view's screen points per document point where its pages are drawn at exactly that scale,
    /// so that their pictures can be rasterised one pixel per device pixel.
    pub(super) fn new(app: &mut WordApp, layout: &Arc<DocLayout>, dim_body: bool, hidden: Option<(usize, Pos)>, view_scale: Option<f32>) -> Self {
        let view_scale = view_scale.filter(|scale| scale.is_finite() && *scale > 0.0);
        let placed = if view_scale.is_some() { app.canvas.settled_pages.clone() } else { Vec::new() };
        let view = {
            // A picture rasterised for another zoom is not this zoom's, even where the side asked for is the same.
            let mut h = DefaultHasher::new();
            (view_key(app, dim_body, hidden.as_ref().map(|(_, p)| p)), view_scale.map(f32::to_bits)).hash(&mut h);
            h.finish()
        };
        let (smoothing, screen_text) = (app.session.prefs.text_smoothing, crate::screen_text::current());
        let options = page_render_options(app, dim_body);
        let doc = Arc::new(page_doc(app).clone());
        let keys = RefCell::new(std::mem::take(&mut app.canvas.page_keys));
        WordPages { doc, layout: layout.clone(), keys, view, options, hidden, view_scale, placed, smoothing, screen_text }
    }

    /// The pass's document snapshot, shared with the accessibility describer so the pass holds one copy.
    pub(super) fn doc(&self) -> Arc<Document> {
        Arc::clone(&self.doc)
    }

    /// The page fingerprints, to put back into the canvas state.
    pub(super) fn into_keys(self) -> PageKeys {
        self.keys.into_inner()
    }
}

/// What a request for a page's picture is rasterised as.
pub(super) struct RasterPlan {
    /// The whole page, or only `area` of it.
    pub(super) whole: bool,
    /// The part of the page to rasterise, in document points.
    pub(super) area: wordcraft_geom::Rect,
    /// Pixels per document point.
    pub(super) scale: f32,
    /// The picture's size in pixels where it is made to fill its place on screen exactly; `None` for one made at
    /// `scale`, a whole pixel up each way.
    pub(super) pixels: Option<(u32, u32)>,
    pub(super) options: wordcraft_render::RenderOptions,
}

impl WordPages {
    /// What of page `index` a picture for `request` shows and how large it is made: whether it is the whole page,
    /// the area in document points, the pixels per point, whether that is the screen's own scale, and the picture's
    /// size in pixels where it is made to fill its place exactly. `None` for a page the layout doesn't have.
    fn geometry(&self, index: usize, request: &ThumbRequest) -> Option<(bool, wordcraft_geom::Rect, f32, bool, Option<(u32, u32)>)> {
        let p = self.layout.pages.get(index)?;
        let (pw, ph) = (dim(p.w, 1e5) as f32, dim(p.h, 1e6) as f32);
        let whole = request.region.is_none();
        let (x0, y0, x1, y1) = match request.region {
            Some(r) => (r.min.x.clamp(0.0, 1.0), r.min.y.clamp(0.0, 1.0), r.max.x.clamp(0.0, 1.0), r.max.y.clamp(0.0, 1.0)),
            None => (0.0, 0.0, 1.0, 1.0),
        };
        let area = wordcraft_geom::Rect::new(x0 * pw, y0 * ph, ((x1 - x0) * pw).max(0.01), ((y1 - y0) * ph).max(0.01));
        let longest = area.w.max(area.h);
        let cap = wordcraft_render::MAX_SIDE as f32 / longest;
        // At the screen's own scale where the view's zoom accounts for the side asked for, so no glyph is resampled
        // on its way to the screen; else at the side asked for.
        let (scale, own) = raster_scale(request.side, SIDE_STEP, longest, self.view_scale.map(|s| s * request.pixels_per_point));
        let own = own && scale <= cap;
        let scale = if scale.is_finite() { scale.clamp(0.01, cap.max(0.01)) } else { 1.0 };
        // Exactly as many pixels each way as the picture's place on screen has, where that is known and is this
        // scale's: a pixel more and the page view scales the picture into its place.
        let place = self.placed.iter().find(|(page, _)| *page == index).map(|(_, rect)| *rect);
        let pixels =
            place.filter(|_| own).and_then(|place| picture_pixels(place, request.region, request.pixels_per_point, (area.w * scale, area.h * scale)));
        Some((whole, area, scale, own, pixels))
    }

    /// How page `index` is rasterised for `request`; `None` for a page the layout doesn't have.
    pub(super) fn plan(&self, index: usize, request: &ThumbRequest) -> Option<RasterPlan> {
        let (whole, area, scale, own, pixels) = self.geometry(index, request)?;
        let mut options = self.options.clone();
        options.display.hide_object = self.hidden.as_ref().filter(|(hidden, _)| *hidden == index).map(|(_, pos)| pos.clone());
        // Subpixel text only on a picture that is the screen's own: scaled, each colour's edge would miss its stripe.
        options.lcd = lcd_for(self.smoothing, self.screen_text.subpixel, self.screen_text.stripes, request.pixels_per_point).filter(|_| own);
        Some(RasterPlan { whole, area, scale, pixels, options })
    }
}

impl ThumbSource<usize> for WordPages {
    fn revision(&self, page: &usize, request: &ThumbRequest) -> Revision {
        let mut h = DefaultHasher::new();
        page_shows(&mut self.keys.borrow_mut(), &self.layout, *page, self.view).hash(&mut h);
        // A picture made for another place's pixel size is not this place's.
        self.geometry(*page, request).and_then(|(.., pixels)| pixels).hash(&mut h);
        Revision::new(h.finish())
    }

    fn render(&self, page: &usize, request: &ThumbRequest) -> ThumbRender {
        let index = *page;
        let Some(RasterPlan { whole, area, scale, pixels, options }) = self.plan(index, request) else { return ThumbRender::Blank };
        let (doc, layout) = (Arc::clone(&self.doc), Arc::clone(&self.layout));
        ThumbRender::Later(Box::new(move |_cancel| {
            let Some(page) = layout.pages.get(index) else { return Ok(LoadedPicture::Blank) };
            let img = if let Some((w, h)) = pixels {
                wordcraft_render::render_area_to(&doc, page, area, w, h, &options)
            } else if whole {
                wordcraft_render::render_page(&doc, page, scale, &options)
            } else {
                wordcraft_render::render_area(&doc, page, area, scale, &options)
            };
            // Both are premultiplied RGBA8, so the pixels go across as they are.
            Ok(LoadedPicture::Image(egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels)))
        }))
    }
}

impl PageSource for WordPages {
    fn page_count(&self) -> usize {
        self.layout.pages.len()
    }

    fn page_size(&self, page: usize) -> Size {
        match self.layout.pages.get(page) {
            Some(p) => Size::new(dim(p.w, 1e5), dim(p.h, 1e6)),
            None => Size::new(72.0, 72.0),
        }
    }
}
