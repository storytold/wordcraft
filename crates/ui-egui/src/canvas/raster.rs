//! Page rasters on the GPU. A page that is small at the current zoom is one texture. A bigger one
//! (zoomed in, or a very tall Web/Draft page) is a whole-page underlay at a modest size with sharp
//! tiles over the part in view, so what a frame rasterises is bounded by the window, not the zoom.
//!
//! A wheel or pinch zoom stretches what is already on the GPU and rasterises nothing new, so it
//! keeps the display's frame rate; once the zoom holds still, the pages sharpen within a time
//! budget per frame. Edits and discrete zooms (a command, the Zoom box) render at once.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use egui::{Rect, TextureHandle, TextureId, pos2, vec2};
use wordcraft_doc::Pos;
use wordcraft_layout::Page;

use super::WordApp;
use super::pages::{page_doc, page_image, page_render_options, texel_aligned_rect, visible_tiles};

/// A page's raster on the GPU, or one tile of it: the content key it shows and its scale in
/// device pixels per page point.
pub(super) struct Raster {
    content: u64,
    scale: f32,
    tex: TextureHandle,
}

impl Raster {
    #[cfg(test)]
    pub(super) fn sized(&self) -> (f32, [usize; 2]) {
        (self.scale, self.tex.size())
    }
}

/// One tile of page `page` rasterised at the scale with these bits, in a grid of [`TILE_PX`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct TileId {
    page: usize,
    scale: u32,
    row: usize,
    col: usize,
}

/// Device pixels on a tile's side.
pub(super) const TILE_PX: usize = 1024;
/// Most device pixels a page gets as one texture; bigger at the current zoom, it is tiled. A
/// Letter or A4 page at 100% on a 2x display (about 3.6 MP) is still one.
const WHOLE_PAGE_PX: f32 = 4.5e6;
/// Device pixels of a tiled page's underlay, shown stretched where its tiles aren't ready.
const UNDERLAY_PX: f32 = 2.0e6;
/// A wheel or pinch zoom is under way while its last step is this recent (seconds).
const ZOOM_SETTLE: f64 = 0.12;
/// For this long after a zoom gesture (seconds), pages sharpen within [`BUDGET_MS`] a frame.
const SHARPEN: f64 = 1.0;
/// What a frame may spend rasterising to sharpen pages or fill tiles just out of view (ms).
const BUDGET_MS: f64 = 4.0;
/// Tiles one frame renders at once for an edit or a discrete zoom: more than a window's worth.
const URGENT_TILES: usize = 16;

/// One page's share of a frame.
pub(super) struct PageJob<'a> {
    pub(super) index: usize,
    pub(super) page: &'a Page,
    /// The page on screen.
    pub(super) rect: Rect,
    /// The part of the screen the page shows in (the view cut to Hide White Space's band).
    pub(super) view: Rect,
    /// Around the view: tiles here are rendered ahead when a frame has time to spare.
    pub(super) near: Rect,
    /// Screen points per page point.
    pub(super) screen_scale: f32,
    /// The page's content and view key ([`super::pages::page_key`]).
    pub(super) content: u64,
    /// The object a drag took off this page, left out of its raster, which is rendered at once.
    pub(super) hide: Option<Pos>,
}

/// What a page draws, back to front, and where the page sits on screen (snapped to device
/// pixels when one texture shows it).
pub(super) struct PageDraw {
    pub(super) rect: Rect,
    pub(super) images: Vec<(TextureId, Rect)>,
}

struct Prefetch {
    id: TileId,
    content: u64,
    hide: Option<Pos>,
}

/// One frame's rasterising: the renders it did, the tiles to keep, and work left for later frames.
pub(super) struct Rasterizer {
    t0: f64,
    /// Seconds since the last wheel or pinch zoom step.
    gesture_age: f64,
    dim_body: bool,
    ppp: f32,
    /// The GPU's largest texture side, in pixels.
    max_tex: f32,
    /// Device pixels on a tile's side: [`TILE_PX`], unless textures can't be that big.
    tile: usize,
    /// Whole pages and underlays rendered this frame.
    rendered: usize,
    tiles_rendered: usize,
    keep: HashSet<TileId>,
    prefetch: Vec<Prefetch>,
    deferred: bool,
}

impl Rasterizer {
    /// `now` and `zoom_gesture_at` are egui input times (seconds).
    pub(super) fn new(now: f64, zoom_gesture_at: f64, dim_body: bool, ppp: f32, max_tex: f32) -> Self {
        let gesture_age = now - zoom_gesture_at;
        Rasterizer {
            t0: crate::now_ms(),
            gesture_age: if gesture_age.is_nan() { f64::INFINITY } else { gesture_age },
            dim_body,
            ppp,
            max_tex,
            tile: TILE_PX.min(max_tex as usize).max(1),
            rendered: 0,
            tiles_rendered: 0,
            keep: HashSet::new(),
            prefetch: Vec::new(),
            deferred: false,
        }
    }

    /// Renders done this frame.
    pub(super) fn rendered(&self) -> usize {
        self.rendered + self.tiles_rendered
    }

    fn zooming(&self) -> bool {
        self.gesture_age < ZOOM_SETTLE
    }

    fn budget_left(&self) -> bool {
        crate::now_ms() - self.t0 < BUDGET_MS
    }

    /// Whether a render that only sharpens (the right content at another scale is on screen)
    /// goes ahead now: never during a zoom gesture, within the budget just after one, else when
    /// `cap` allows (a discrete zoom renders at once).
    fn may_sharpen(&self, cap: bool) -> bool {
        if self.zooming() {
            false
        } else if self.gesture_age < SHARPEN {
            self.budget_left()
        } else {
            cap
        }
    }

    /// Brings page `job` up to date as far as this frame allows and says what to draw.
    pub(super) fn page(&mut self, app: &mut WordApp, ctx: &egui::Context, job: PageJob<'_>) -> PageDraw {
        let s = job.screen_scale * self.ppp;
        let s = if s.is_finite() && s > 0.0 { s.min(1e3) } else { 1.0 };
        let (w, h) = (job.page.w.clamp(1.0, 1e6), job.page.h.clamp(1.0, 1e6));
        if w * h * s * s <= WHOLE_PAGE_PX && w * s <= self.max_tex && h * s <= self.max_tex {
            self.whole(app, ctx, &job, s)
        } else {
            self.tiled(app, ctx, &job, s)
        }
    }

    fn whole(&mut self, app: &mut WordApp, ctx: &egui::Context, job: &PageJob<'_>, s: f32) -> PageDraw {
        let i = job.index;
        let forced = job.hide.is_some();
        let render = match app.canvas.textures.get(&i).map(|r| (r.content == job.content, r.scale == s)) {
            Some((true, true)) => false,
            Some((true, false)) => forced || self.may_sharpen(self.rendered < 2),
            Some((false, _)) => forced || self.rendered < 2,
            None => forced || self.rendered < 4,
        };
        if render {
            let img = rasterize(app, job.page, None, s, self.dim_body, job.hide.as_ref());
            upload(ctx, &mut app.canvas.textures, i, || format!("page{i}"), img, job.content, s);
            self.rendered += 1;
        }
        let Some(r) = app.canvas.textures.get(&i) else {
            self.deferred = true;
            return PageDraw { rect: job.rect, images: Vec::new() };
        };
        self.deferred |= r.content != job.content || r.scale != s;
        // On the pixel grid at this scale's raster size even while stretched, so the page doesn't
        // shift when it sharpens.
        let size = vec2(raster_px(job.page.w, s) as f32, raster_px(job.page.h, s) as f32);
        let rect = texel_aligned_rect(job.rect, size, self.ppp);
        PageDraw { rect, images: vec![(r.tex.id(), rect)] }
    }

    fn tiled(&mut self, app: &mut WordApp, ctx: &egui::Context, job: &PageJob<'_>, s: f32) -> PageDraw {
        let i = job.index;
        let forced = job.hide.is_some();
        let tile = self.tile;
        let bits = s.to_bits();
        let (rows, cols) = visible_tiles(job.rect, job.view, job.screen_scale, s, tile);
        let mut complete = true;
        let mut sharp = Vec::new();
        for row in rows.clone() {
            for col in cols.clone() {
                let id = TileId { page: i, scale: bits, row, col };
                self.keep.insert(id);
                let have = app.canvas.tiles.get(&id).map(|r| r.content == job.content);
                let render = match have {
                    Some(true) => false,
                    Some(false) => forced || self.tiles_rendered < URGENT_TILES,
                    None => forced || self.may_sharpen(self.tiles_rendered < URGENT_TILES),
                };
                if have == Some(true) {
                    sharp.push(id);
                } else if render {
                    // An empty tile (past the page's last pixel) has nothing to show or wait for.
                    if self.render_tile(app, ctx, job.page, id, job.content, job.hide.as_ref()) {
                        sharp.push(id);
                    }
                } else {
                    complete = false;
                }
            }
        }
        // Tiles just out of view, rendered ahead with time to spare (`finish`). One an edit made
        // stale waits until it comes into view: rendering ahead after every keystroke is waste.
        let (near_rows, near_cols) = visible_tiles(job.rect, job.near, job.screen_scale, s, tile);
        for row in near_rows {
            for col in near_cols.clone() {
                let id = TileId { page: i, scale: bits, row, col };
                if self.keep.insert(id) && !app.canvas.tiles.contains_key(&id) {
                    self.prefetch.push(Prefetch { id, content: job.content, hide: job.hide.clone() });
                }
            }
        }
        // The underlay, only where tiles are missing (a zoom gesture, sharpening after one): an
        // edit makes it stale, and rendering it again after every keystroke is waste.
        if !complete && app.canvas.textures.get(&i).is_none_or(|r| r.content != job.content) {
            if forced || self.rendered < 4 {
                let u = underlay_scale(job.page, self.max_tex).min(s);
                let img = rasterize(app, job.page, None, u, self.dim_body, job.hide.as_ref());
                upload(ctx, &mut app.canvas.textures, i, || format!("page{i}"), img, job.content, u);
                self.rendered += 1;
            } else {
                self.deferred = true;
            }
        }
        self.deferred |= !complete;
        let mut images = Vec::new();
        if let Some(r) = app.canvas.textures.get(&i).filter(|r| r.content == job.content) {
            images.push((r.tex.id(), job.rect));
        }
        // Tiles from before the zoom, stretched, until this scale's tiles are all in.
        if !complete {
            for (id, r) in app.canvas.tiles.iter().filter(|(id, r)| id.page == i && id.scale != bits && r.content == job.content) {
                let at = tile_rect(job.rect.min, *id, tile, job.screen_scale / r.scale, r.tex.size_vec2());
                if at.intersects(job.view) {
                    self.keep.insert(*id);
                    images.push((r.tex.id(), at));
                }
            }
        }
        let snap = |v: f32| (v * self.ppp).round() / self.ppp;
        let base = pos2(snap(job.rect.min.x), snap(job.rect.min.y));
        for id in sharp {
            if let Some(r) = app.canvas.tiles.get(&id) {
                images.push((r.tex.id(), tile_rect(base, id, tile, job.screen_scale / s, r.tex.size_vec2())));
            }
        }
        PageDraw { rect: job.rect, images }
    }

    /// Renders tile `id` of `page` into the tile cache; false when it is empty.
    fn render_tile(&mut self, app: &mut WordApp, ctx: &egui::Context, page: &Page, id: TileId, content: u64, hide: Option<&Pos>) -> bool {
        let s = f32::from_bits(id.scale);
        let tile = self.tile;
        let (tw, th) = (raster_px(page.w, s), raster_px(page.h, s));
        let (x0, y0) = (id.col.saturating_mul(tile), id.row.saturating_mul(tile));
        let (w, h) = (tw.saturating_sub(x0).min(tile), th.saturating_sub(y0).min(tile));
        if w == 0 || h == 0 {
            return false;
        }
        // The tiny subtraction keeps render_area's ceil() from rounding up to an extra pixel.
        let area = wordcraft_geom::Rect::new(x0 as f32 / s, y0 as f32 / s, (w as f32 - 0.01).max(0.01) / s, (h as f32 - 0.01).max(0.01) / s);
        let img = rasterize(app, page, Some(area), s, self.dim_body, hide);
        upload(ctx, &mut app.canvas.tiles, id, || format!("page{}-tile{}-{}", id.page, id.row, id.col), img, content, s);
        self.tiles_rendered += 1;
        true
    }

    /// Renders tiles near the view with what is left of the budget, drops tiles no longer
    /// needed, and asks for the frames still-missing work needs.
    pub(super) fn finish(&mut self, app: &mut WordApp, ctx: &egui::Context, pages: &[Page]) {
        for p in std::mem::take(&mut self.prefetch) {
            if self.zooming() || !self.budget_left() {
                self.deferred = true;
                break;
            }
            if let Some(page) = pages.get(p.id.page) {
                self.render_tile(app, ctx, page, p.id, p.content, p.hide.as_ref());
            }
        }
        app.canvas.tiles.retain(|id, _| self.keep.contains(id));
        if self.deferred {
            if self.zooming() {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((ZOOM_SETTLE - self.gesture_age).clamp(0.0, ZOOM_SETTLE)));
            } else {
                ctx.request_repaint();
            }
        }
    }
}

/// Device pixels across `len` page points at `s` pixels per point.
fn raster_px(len: f32, s: f32) -> usize {
    (len.clamp(1.0, 1e6) * s).ceil() as usize
}

/// Pixels per point for a tiled page's underlay: about [`UNDERLAY_PX`], within a texture.
fn underlay_scale(page: &Page, max_tex: f32) -> f32 {
    let (w, h) = (page.w.clamp(1.0, 1e6), page.h.clamp(1.0, 1e6));
    (UNDERLAY_PX / (w * h)).sqrt().min(max_tex / w).min(max_tex / h).max(1e-3)
}

/// Where tile `id` (`size` texels) goes on screen: `origin` is the page's corner and `k` screen
/// points per texel.
fn tile_rect(origin: egui::Pos2, id: TileId, tile: usize, k: f32, size: egui::Vec2) -> Rect {
    let at = vec2(id.col.saturating_mul(tile) as f32, id.row.saturating_mul(tile) as f32);
    Rect::from_min_size(origin + at * k, size * k)
}

fn rasterize(app: &WordApp, page: &Page, area: Option<wordcraft_geom::Rect>, s: f32, dim_body: bool, hide: Option<&Pos>) -> egui::ColorImage {
    let mut opts = page_render_options(app, dim_body);
    // Only on the page it is on: the `Pos` can't match an object of another page.
    opts.display.hide_object = hide.cloned();
    let img = match area {
        Some(a) => wordcraft_render::render_area(page_doc(app), page, a, s, &opts),
        None => wordcraft_render::render_page(page_doc(app), page, s, &opts),
    };
    page_image(&img)
}

fn upload<K: Eq + Hash>(
    ctx: &egui::Context,
    map: &mut HashMap<K, Raster>,
    key: K,
    name: impl FnOnce() -> String,
    img: egui::ColorImage,
    content: u64,
    scale: f32,
) {
    match map.get_mut(&key) {
        Some(r) => {
            r.tex.set(img, egui::TextureOptions::LINEAR);
            r.content = content;
            r.scale = scale;
        }
        None => {
            let tex = ctx.load_texture(name(), img, egui::TextureOptions::LINEAR);
            map.insert(key, Raster { content, scale, tex });
        }
    }
}
