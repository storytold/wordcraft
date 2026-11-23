//! A frosted ribbon: the page scrolling up under it shows through as a soft blur.
//!
//! Each page near the top of the view leaves a small thumbnail. Every frame the pages that would lie under the ribbon,
//! were the canvas to continue upward, are sampled into a strip. The strip is stretched over the ribbon with linear
//! filtering, which is the blur.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::layers::ShapeIdx;
use egui::{Color32, ColorImage, Context, LayerId, Rect, Shape, TextureHandle, TextureOptions, pos2, vec2};

use crate::WordApp;

/// Screen points covered by one texel of the strip.
pub const CELL: f32 = 4.0;

/// Page thumbnails are rendered at this many pixels per point: one texel per [`CELL`].
const THUMB_SCALE: f32 = 1.0 / CELL;

/// How far above the view's top, in screen points, a page may lie and still feed the strip.
const BAND: f32 = 400.0;

/// The height of the ribbon's tab strip, in points: the tint over it is the tab strip's, the rest the body's.
const TAB_STRIP_H: f32 = 30.0;

/// How much of the ribbon's own colour lies over the blur (0 to 255), in a dark theme.
pub const TINT_ALPHA: u8 = 214;

/// The same in a light theme, where the page and the ribbon are both pale and little shows.
pub const LIGHT_TINT_ALPHA: u8 = 125;

/// A page as one texel per thumbnail pixel, premultiplied over its paper colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumb {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Thumb {
    /// Flatten `img` over `paper`. Pixels are premultiplied, so they go over `paper` by adding the paper scaled by the
    /// missing alpha.
    pub fn of(img: &ColorImage, paper: Color32) -> Thumb {
        let [w, h] = img.size;
        let paper = [paper.r() as f32, paper.g() as f32, paper.b() as f32];
        let px = img
            .pixels
            .iter()
            .map(|c| {
                let miss = 1.0 - c.a() as f32 / 255.0;
                [c.r() as f32 + paper[0] * miss, c.g() as f32 + paper[1] * miss, c.b() as f32 + paper[2] * miss]
            })
            .collect();
        Thumb { w: w.max(1), h: h.max(1), px }
    }

    /// The colour at (`u`, `v`) in 0..1 of the page, bilinear between texel centres. `None` where the thumbnail has no
    /// texels, so the caller falls back to paper.
    fn sample(&self, u: f32, v: f32) -> Option<[f32; 3]> {
        if self.px.is_empty() {
            return None;
        }
        let fx = (u * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fy = (v * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: usize, y: usize| self.px.get(y * self.w + x).copied().unwrap_or([255.0; 3]);
        let mut out = [0.0; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let top = at(x0, y0)[c] * (1.0 - tx) + at(x1, y0)[c] * tx;
            let bottom = at(x0, y1)[c] * (1.0 - tx) + at(x1, y1)[c] * tx;
            *o = top * (1.0 - ty) + bottom * ty;
        }
        Some(out)
    }
}

/// What a page contributes to the strip: where it is, and its look.
pub struct PageView<'a> {
    pub rect: Rect,
    pub thumb: Option<&'a Thumb>,
    pub paper: Color32,
}

/// The strip for `area` (screen points): `pages` over `background`, then softened.
pub fn compose(area: Rect, pages: &[PageView], background: Color32) -> ColorImage {
    let w = (area.width() / CELL).ceil().clamp(1.0, 1024.0) as usize;
    let h = (area.height() / CELL).ceil().clamp(1.0, 1024.0) as usize;
    let (cw, ch) = (area.width() / w as f32, area.height() / h as f32);
    let bg = [background.r() as f32, background.g() as f32, background.b() as f32];
    let mut raw = vec![bg; w * h];
    for page in pages.iter().filter(|p| p.rect.intersects(area) && p.rect.width() > 0.0 && p.rect.height() > 0.0) {
        let paper = [page.paper.r() as f32, page.paper.g() as f32, page.paper.b() as f32];
        for y in 0..h {
            for x in 0..w {
                let at = pos2(area.min.x + (x as f32 + 0.5) * cw, area.min.y + (y as f32 + 0.5) * ch);
                if !page.rect.contains(at) {
                    continue;
                }
                let (u, v) = ((at.x - page.rect.min.x) / page.rect.width(), (at.y - page.rect.min.y) / page.rect.height());
                if let Some(slot) = raw.get_mut(y * w + x) {
                    *slot = page.thumb.and_then(|t| t.sample(u, v)).unwrap_or(paper);
                }
            }
        }
    }
    for _ in 0..PASSES {
        box_pass(&mut raw, w, h, true);
        box_pass(&mut raw, w, h, false);
    }
    let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    let pixels = raw.iter().map(|c| Color32::from_rgb(byte(c[0]), byte(c[1]), byte(c[2]))).collect();
    ColorImage::new([w, h], pixels)
}

/// Three box passes along an axis come close to a Gaussian.
const PASSES: usize = 3;
/// Cells each side in a box pass: wider across than down, so the blur is stretched sideways.
const RADIUS_X: usize = 5;
const RADIUS_Y: usize = 2;

/// Average each cell with its neighbours along one axis; the edge cell repeats.
fn box_pass(px: &mut [[f32; 3]], w: usize, h: usize, along_x: bool) {
    let (lines, len, radius) = if along_x { (h, w, RADIUS_X) } else { (w, h, RADIUS_Y) };
    let at = |l: usize, i: usize| if along_x { l * w + i } else { i * w + l };
    let width = (2 * radius + 1) as f32;
    for l in 0..lines {
        let src: Vec<[f32; 3]> = (0..len).map(|i| px.get(at(l, i)).copied().unwrap_or([0.0; 3])).collect();
        for i in 0..len {
            let mut sum = [0.0_f32; 3];
            for k in 0..=2 * radius {
                let c = src.get((i + k).saturating_sub(radius).min(len - 1)).copied().unwrap_or([0.0; 3]);
                (0..3).for_each(|j| sum[j] += c[j]);
            }
            if let Some(slot) = px.get_mut(at(l, i)) {
                *slot = [sum[0] / width, sum[1] / width, sum[2] / width];
            }
        }
    }
}

/// The strip's texture, kept between frames and rewritten only when the picture changes.
#[derive(Clone)]
struct Strip {
    tex: TextureHandle,
    image: ColorImage,
}

/// Show `image` as the strip's texture and return it.
fn upload(ctx: &Context, image: ColorImage) -> egui::TextureId {
    let id = strip_id(ctx);
    let kept: Option<Strip> = ctx.data(|d| d.get_temp(id));
    let strip = match kept {
        Some(mut s) => {
            if s.image != image {
                s.tex.set(image.clone(), TextureOptions::LINEAR);
                s.image = image;
            }
            s
        }
        None => Strip { tex: ctx.load_texture("ribbon_blur", image.clone(), TextureOptions::LINEAR), image },
    };
    let tex_id = strip.tex.id();
    ctx.data_mut(|d| d.insert_temp(id, strip));
    tex_id
}

/// Keyed by viewport: each window keeps its own strip.
fn strip_id(ctx: &Context) -> egui::Id {
    egui::Id::unique(("ribbon_blur_strip", ctx.viewport_id()))
}

/// The blur's per-window state: the thumbnails of the pages near the view's top, and the slot the strip is painted
/// into, reserved by the ribbon this frame.
#[derive(Default)]
pub struct BlurState {
    thumbs: HashMap<usize, (u64, Thumb)>,
    slot: Option<(LayerId, ShapeIdx, Rect)>,
}

impl BlurState {
    /// Reserve the slot the strip is painted into, at the ribbon's current place in `layer`. Call before the ribbon is
    /// drawn, and [`BlurState::set_ribbon`] after.
    pub fn reserve(ui: &mut egui::Ui) -> (LayerId, ShapeIdx) {
        (ui.layer_id(), ui.painter().add(Shape::Noop))
    }

    /// Remember where the ribbon ended up, with the slot [`BlurState::reserve`] gave.
    pub fn set_ribbon(&mut self, slot: (LayerId, ShapeIdx), ribbon: Rect) {
        self.slot = Some((slot.0, slot.1, ribbon));
    }
}

/// Make the thumbnails for the pages in the band above `view_top` and paint the strip under the ribbon. Call once per
/// frame, after the canvas has laid out its pages. Nothing is kept or drawn while the blur is off.
pub fn update(app: &mut WordApp, ctx: &Context) {
    let slot = app.canvas.blur.slot.take();
    if !app.ribbon_glass() {
        app.canvas.blur.thumbs.clear();
        return;
    }
    let Some(view) = app.canvas.view_rect() else { return };
    refresh_thumbs(app, view.min.y);
    let Some((layer, idx, ribbon)) = slot else { return };
    let t = crate::theme::Tokens::get(ctx);
    let paper = paper_of(app);
    let shift = vec2(0.0, ribbon.max.y - view.min.y);
    let pages: Vec<PageView> = app
        .canvas
        .laid_page_rects()
        .iter()
        .enumerate()
        .map(|(i, r)| PageView { rect: r.translate(shift), thumb: app.canvas.blur.thumbs.get(&i).map(|(_, thumb)| thumb), paper })
        .collect();
    let id = upload(ctx, compose(ribbon, &pages, t.canvas));
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let tab_strip = Rect::from_min_max(ribbon.min, pos2(ribbon.max.x, ribbon.min.y + TAB_STRIP_H));
    let shape = Shape::Vec(vec![
        Shape::image(id, ribbon, full, Color32::WHITE),
        Shape::rect_filled(tab_strip, egui::CornerRadius::ZERO, glass(t.tab_strip)),
        Shape::rect_filled(Rect::from_min_max(pos2(ribbon.min.x, tab_strip.max.y), ribbon.max), egui::CornerRadius::ZERO, glass(t.ribbon)),
    ]);
    ctx.layer_painter(layer).with_clip_rect(ribbon).set(idx, shape);
}

/// Keep a thumbnail for each page in the band above the view's top, rendered again only when the document or the
/// dark paper changed.
fn refresh_thumbs(app: &mut WordApp, view_top: f32) {
    let wanted: Vec<usize> = app
        .canvas
        .laid_page_rects()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.width() > 0.0 && r.max.y > view_top - BAND && r.min.y < view_top)
        .map(|(i, _)| i)
        .collect();
    app.canvas.blur.thumbs.retain(|i, _| wanted.contains(i));
    let key = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        app.session.rev().hash(&mut h);
        app.session.view.dark_mode.hash(&mut h);
        h.finish()
    };
    let layout = app.session.layout();
    for i in wanted {
        if app.canvas.blur.thumbs.get(&i).is_some_and(|(k, _)| *k == key) {
            continue;
        }
        let Some(page) = layout.pages.get(i) else { continue };
        let longest = page.w.max(page.h).max(1.0);
        let scale = (THUMB_SCALE).min(wordcraft_render::MAX_SIDE as f32 / longest);
        let img = wordcraft_render::render_page(crate::canvas::page_doc(app), page, scale, &crate::canvas::screen_render_options());
        let ci = ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
        let thumb = Thumb::of(&ci, paper_of(app));
        app.canvas.blur.thumbs.insert(i, (key, thumb));
    }
}

/// The colour of a page's paper.
fn paper_of(app: &WordApp) -> Color32 {
    if app.session.view.dark_mode { Color32::from_gray(wordcraft_render::DARK_PAPER) } else { Color32::WHITE }
}

/// `c` made see-through, so the blur shows under it.
pub fn glass(c: Color32) -> Color32 {
    let light = c.r() as u32 + c.g() as u32 + c.b() as u32 > 384;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), if light { LIGHT_TINT_ALPHA } else { TINT_ALPHA })
}

/// The strip's size in texels, once it has been made for this viewport.
#[cfg(test)]
pub fn strip_size(ctx: &Context) -> Option<[usize; 2]> {
    let kept: Option<Strip> = ctx.data(|d| d.get_temp(strip_id(ctx)));
    kept.map(|s| s.image.size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, c: Color32) -> ColorImage {
        ColorImage::new([w, h], vec![c; w * h])
    }

    #[test]
    fn a_transparent_page_thumbnail_is_the_paper() {
        let t = Thumb::of(&flat(40, 40, Color32::TRANSPARENT), Color32::WHITE);
        assert_eq!(t.sample(0.5, 0.5), Some([255.0; 3]));
    }

    #[test]
    fn ink_darkens_its_cell() {
        let mut img = flat(16, 16, Color32::TRANSPARENT);
        for y in 0..8 {
            for x in 0..8 {
                img.pixels[y * 16 + x] = Color32::BLACK;
            }
        }
        let t = Thumb::of(&img, Color32::WHITE);
        assert!(t.sample(0.25, 0.25).unwrap()[0] < 1.0);
        assert!(t.sample(0.75, 0.75).unwrap()[0] > 254.0);
    }

    #[test]
    fn the_strip_shows_a_page_under_it_and_the_background_beside_it() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 80.0));
        let page = PageView { rect: Rect::from_min_size(pos2(200.0, -50.0), vec2(300.0, 100.0)), thumb: None, paper: Color32::WHITE };
        let img = compose(area, &[page], Color32::from_gray(100));
        let (w, h) = (img.size[0], img.size[1]);
        assert_eq!(img.pixels[2 * w + 2], Color32::from_gray(100), "left of the page");
        assert_eq!(img.pixels[2 * w + w - 3], Color32::WHITE, "under the page");
        assert_eq!(img.pixels[(h - 1) * w + w - 3], Color32::from_gray(100), "below the page's end");
    }

    #[test]
    fn a_page_wholly_outside_the_area_adds_nothing() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(64.0, 32.0));
        let far = PageView { rect: Rect::from_min_size(pos2(0.0, 500.0), vec2(64.0, 64.0)), thumb: None, paper: Color32::WHITE };
        let img = compose(area, &[far], Color32::from_gray(90));
        assert!(img.pixels.iter().all(|c| *c == Color32::from_gray(90)));
    }

    #[test]
    fn degenerate_areas_still_give_a_strip() {
        let img = compose(Rect::from_min_size(pos2(0.0, 0.0), vec2(0.0, 0.0)), &[], Color32::BLACK);
        assert_eq!(img.size, [1, 1]);
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;
    use serde_json::json;
    use wordcraft_doc::Document;
    use wordcraft_engine::Session;

    fn editor() -> WordApp {
        let mut a = WordApp::new(Session::new(Document::from_text("Hello WordCraft")), Default::default());
        a.run("ui.ribbonBlur", json!({"value": true})).unwrap();
        a
    }

    fn drawn(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a)
    }

    #[test]
    fn the_blur_is_a_ui_command_off_by_default_and_saved() {
        let mut a = WordApp::new(Session::new(Document::new()), Default::default());
        assert!(!a.ui.ribbon_blur, "off by default");
        assert_eq!(a.run("ui.ribbonBlur", json!({"value": true})).unwrap(), json!({"ribbonBlur": true}));
        assert!(a.run("ui.ribbonBlur", json!({"value": "yes"})).is_err());
        assert!(a.ui.ribbon_blur, "a rejected value keeps the setting");
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""ribbonBlur":true"#), "{saved}");
    }

    #[test]
    fn the_strip_is_drawn_over_the_editor_only() {
        let mut h = drawn(editor());
        h.run_steps(6);
        assert!(strip_size(&h.ctx).is_some(), "the strip was made");
        h.state_mut().run("ui.backstage", json!({"value": true})).unwrap();
        assert!(!h.state().ribbon_glass(), "File covers the window");
        h.state_mut().run("ui.backstage", json!({"value": false})).unwrap();
        h.state_mut().run("ui.ribbonBlur", json!({"value": false})).unwrap();
        h.run_steps(2);
        assert!(!h.state().ribbon_glass(), "off is solid");
        assert!(h.state().canvas.blur.thumbs.is_empty(), "no thumbnails kept while off");
    }
}
