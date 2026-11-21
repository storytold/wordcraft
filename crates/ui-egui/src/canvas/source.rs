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
use super::pages::{PageKeys, page_doc, page_render_options, page_shows, view_key};

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
    rendered_pages: Arc<std::sync::Mutex<std::collections::HashMap<usize, super::pages::RenderedPage>>>,
    telemetry: Arc<std::sync::Mutex<super::CanvasTelemetry>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
}

impl WordPages {
    /// The source of `layout` for this pass. The page fingerprints move out of the app for the pass; give them back
    /// with [`WordPages::into_keys`].
    pub(super) fn new(app: &mut WordApp, layout: &Arc<DocLayout>, dim_body: bool, hidden: Option<(usize, Pos)>) -> Self {
        let view = view_key(app, dim_body, hidden.as_ref().map(|(_, p)| p));
        let options = page_render_options(app, dim_body);
        let doc = Arc::new(page_doc(app).clone());
        let rendered_pages = Arc::clone(&app.canvas.rendered_pages);
        let telemetry = Arc::clone(&app.canvas.telemetry);
        let generation = Arc::clone(&app.canvas.generation);
        WordPages {
            doc,
            layout: layout.clone(),
            keys: RefCell::new(std::mem::take(&mut app.canvas.page_keys)),
            view,
            options,
            hidden,
            rendered_pages,
            telemetry,
            generation,
        }
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

impl ThumbSource<usize> for WordPages {
    fn revision(&self, page: &usize, _request: &ThumbRequest) -> Revision {
        let mut h = DefaultHasher::new();
        page_shows(&mut self.keys.borrow_mut(), &self.layout, *page, self.view).hash(&mut h);
        Revision::new(h.finish())
    }

    fn render(&self, page: &usize, request: &ThumbRequest) -> ThumbRender {
        let index = *page;
        let Some(p) = self.layout.pages.get(index) else { return ThumbRender::Blank };
        let (pw, ph) = (dim(p.w, 1e5) as f32, dim(p.h, 1e6) as f32);
        let whole = request.region.is_none();
        let (x0, y0, x1, y1) = match request.region {
            Some(r) => (r.min.x.clamp(0.0, 1.0), r.min.y.clamp(0.0, 1.0), r.max.x.clamp(0.0, 1.0), r.max.y.clamp(0.0, 1.0)),
            None => (0.0, 0.0, 1.0, 1.0),
        };
        let area = wordcraft_geom::Rect::new(x0 * pw, y0 * ph, ((x1 - x0) * pw).max(0.01), ((y1 - y0) * ph).max(0.01));
        let longest = area.w.max(area.h);
        let cap = wordcraft_render::MAX_SIDE as f32 / longest;
        let scale = request.side as f32 / longest;
        let scale = if scale.is_finite() { scale.clamp(0.01, cap.max(0.01)) } else { 1.0 };
        let mut options = self.options.clone();
        options.display.hide_object = self.hidden.as_ref().filter(|(hidden, _)| *hidden == index).map(|(_, pos)| pos.clone());

        let current_bands = self.keys.borrow_mut().snapshot(&self.layout, index).1;
        let total_rows = ((p.h.min(1e6) * scale).ceil() as u32).clamp(1, wordcraft_render::MAX_SIDE) as usize;
        let target_revision = Revision::of(&(self.revision(&index, request), request.side, None::<[u32; 4]>));

        if whole && request.patchable {
            let patch_rows = {
                let cache = self.rendered_pages.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                cache.get(&index).and_then(|prev| {
                    if Some(prev.revision) == request.resident_revision && prev.view == self.view && prev.scale.to_bits() == scale.to_bits() {
                        prev.bands.changed_rows(&current_bands, scale, total_rows)
                    } else {
                        None
                    }
                })
            };

            if let Some(rows) = patch_rows {
                let band_rows = rows.end.saturating_sub(rows.start);
                if band_rows > 0 && rows.end <= total_rows {
                    let y = rows.start as f32 / scale;
                    let band_area = wordcraft_geom::Rect::new(0.0, y, pw, (band_rows as f32 - 0.01).max(0.01) / scale);
                    let t0 = craft::platform::clock::monotonic_ms();
                    let img = wordcraft_render::render_area(&self.doc, p, band_area, scale, &options);
                    let elapsed = craft::platform::clock::monotonic_ms() - t0;
                    {
                        let mut telem = self.telemetry.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        telem.partial_renders += 1;
                        telem.raster_ms += elapsed;
                    }
                    let job_generation = self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    {
                        let mut cache = self.rendered_pages.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        cache.insert(
                            index,
                            super::pages::RenderedPage {
                                revision: target_revision,
                                view: self.view,
                                scale,
                                bands: current_bands,
                                generation: job_generation,
                            },
                        );
                    }
                    let image = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
                    return ThumbRender::Region { origin: [0, rows.start as u32], image };
                } else if band_rows == 0 {
                    let job_generation = self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    {
                        let mut cache = self.rendered_pages.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        cache.insert(
                            index,
                            super::pages::RenderedPage {
                                revision: target_revision,
                                view: self.view,
                                scale,
                                bands: current_bands,
                                generation: job_generation,
                            },
                        );
                    }
                    return ThumbRender::Region { origin: [0, 0], image: egui::ColorImage::new([0, 0], vec![]) };
                }
            }
        }

        let (doc, layout) = (Arc::clone(&self.doc), Arc::clone(&self.layout));
        let rendered_pages = Arc::clone(&self.rendered_pages);
        let telemetry = Arc::clone(&self.telemetry);
        let view = self.view;
        let bands = current_bands;
        let job_generation = self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        ThumbRender::Later(Box::new(move |_cancel| {
            let Some(page) = layout.pages.get(index) else { return Ok(LoadedPicture::Blank) };
            let t0 = craft::platform::clock::monotonic_ms();
            let img = if whole {
                wordcraft_render::render_page(&doc, page, scale, &options)
            } else {
                wordcraft_render::render_area(&doc, page, area, scale, &options)
            };
            let elapsed = craft::platform::clock::monotonic_ms() - t0;
            {
                let mut telem = telemetry.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                telem.full_renders += 1;
                telem.raster_ms += elapsed;
            }
            if whole {
                let mut cache = rendered_pages.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let obsolete = cache.get(&index).is_some_and(|existing| existing.generation > job_generation);
                if !obsolete {
                    cache.insert(index, super::pages::RenderedPage { revision: target_revision, view, scale, bands, generation: job_generation });
                }
            }
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
