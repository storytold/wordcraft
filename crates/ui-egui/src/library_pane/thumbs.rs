//! The Library pane's thumbnails, drawn off the UI thread (#2868) by the engine's
//! [`Drawer`], so a folder of camera photos scrolls without a stall.
//!
//! The pane the Craft apps share asks for each thumbnail once and keeps whatever it is given: an
//! error becomes a "?" until Refresh. So, until it can be told a thumbnail is still being drawn
//! (to raise for storytold/craft-libs#1584), a picture not drawn yet is answered with a
//! placeholder of the pane's own blank-cell colour and shape, and when the drawer has finished
//! it, the texture the pane made of the placeholder is given the thumbnail ([`swap`]): the pane
//! names its textures `library {file} {index} {width}`, which the pane's tests here pin. The pane
//! keeps a cell the shape of the first image it was given, so the thumbnail comes in the
//! placeholder's shape with the picture centred in it ([`letterboxed`]), never stretched. A
//! picture that can't be drawn gets the pane's "?" the same way.
//!
//! What waits is dropped when the pane shows another folder or tab, or is closed; the pane then
//! reads its folders again, so none of the placeholders it held is left blank. A new search only
//! puts the pictures it shows first (they were asked for last): the ones it hides keep their
//! placeholders in the pane, which would show them blank on going back.

use std::collections::{HashMap, VecDeque};

use egui::{Color32, ColorImage, TextureOptions};
use image::RgbaImage;
use image::imageops::FilterType;
use serde_json::Value;
use wordcraft_engine::library::drawer::{self, Answer, Drawer, Key};
use wordcraft_engine::library::{MAX_THUMB_HEIGHT, Pictures, THUMB_WIDTHS, Thumbnail};

use crate::WordApp;

/// A thumbnail as the shared pane names it: the file as listed, the picture, the width in pixels.
type PaneKey = (String, usize, u32);

/// Placeholders held at once: the pane keeps at most 256 thumbnails.
const MAX_HELD: usize = 256;
/// Time spent per frame putting finished thumbnails in place, in seconds.
const SWAP_BUDGET: f64 = 0.004;
/// The placeholder: the pane's own shape for a cell still waiting (`Labels::aspect`, 0.75).
const PLACEHOLDER: (u32, u32) = (4, 3);
/// A failed thumbnail's size, with the "?" drawn in it.
pub(super) const FAILED: [usize; 2] = [168, 126];

/// The drawer and what the pane holds of it: app-wide, as the pane's state is.
#[derive(Default)]
pub struct Thumbs {
    drawer: Option<Drawer>,
    /// The drawer could not start (the browser build, no threads): thumbnails are drawn when
    /// the pane asks, as before.
    off: bool,
    ctx: Option<egui::Context>,
    /// A cell's colours: the pane's blank cell, and its "?".
    colors: (Color32, Color32),
    /// The placeholder for `colors.0`, as the pane is answered.
    placeholder: Option<(Color32, Value)>,
    failed: Option<((Color32, Color32), ColorImage)>,
    /// The placeholders the pane holds, with what each is waiting for; oldest first in `order`.
    held: HashMap<PaneKey, Key>,
    order: VecDeque<PaneKey>,
    /// Finished thumbnails not yet put in place.
    finished: VecDeque<(Key, Result<Thumbnail, String>)>,
    /// The Library folders, the folder and the tab the pane showed.
    view: Option<(Vec<String>, Option<String>, String)>,
    /// Frames counted by [`tick`], and the last one the pane was shown in.
    frame: u64,
    shown: u64,
}

impl Thumbs {
    /// Pictures waiting to be drawn, and placeholders the pane holds.
    pub fn waiting(&self) -> (usize, usize) {
        (self.drawer.as_ref().map_or(0, Drawer::queued), self.held.len())
    }

    /// Is a picture being drawn?
    pub fn drawing(&self) -> bool {
        self.drawer.as_ref().is_some_and(Drawer::busy)
    }

    fn hold(&mut self, pane: PaneKey, key: Key) {
        if self.held.insert(pane.clone(), key).is_none() {
            self.order.push_back(pane);
        }
        while self.order.len() > MAX_HELD {
            if let Some(old) = self.order.pop_front() {
                self.held.remove(&old);
            }
        }
    }

    fn release(&mut self, pane: &PaneKey) {
        if self.held.remove(pane).is_some() {
            self.order.retain(|k| k != pane);
        }
    }

    /// The placeholder answer in the pane's blank-cell colour, made once per colour.
    fn placeholder(&mut self) -> Option<Value> {
        let bg = self.colors.0;
        if let Some((c, v)) = &self.placeholder
            && *c == bg
        {
            return Some(v.clone());
        }
        let [r, g, b, a] = bg.to_srgba_unmultiplied();
        let img = image::RgbaImage::from_pixel(PLACEHOLDER.0, PLACEHOLDER.1, image::Rgba([r, g, b, a]));
        let png = wordcraft_geom::picture::encode_png(&image::DynamicImage::ImageRgba8(img)).ok()?;
        let v = drawer::answer(&Thumbnail { png: std::sync::Arc::new(png), width: PLACEHOLDER.0, height: PLACEHOLDER.1 });
        self.placeholder = Some((bg, v.clone()));
        Some(v)
    }

    fn failed_image(&mut self) -> ColorImage {
        match &self.failed {
            Some((c, img)) if *c == self.colors => img.clone(),
            _ => {
                let img = question_mark(self.colors.0, self.colors.1);
                self.failed = Some((self.colors, img.clone()));
                img
            }
        }
    }
}

/// What the pane names a thumbnail by, from its `library.thumbnail` parameters.
fn pane_key(params: &Value) -> Option<PaneKey> {
    let file = params["file"].as_str()?.to_string();
    let item = params[wordcraft_engine::library::NOUNS.one].as_u64().and_then(|i| usize::try_from(i).ok())?;
    let width = params["width"].as_u64().and_then(|w| u32::try_from(w).ok())?;
    Some((file, item, width))
}

/// `library.thumbnail` for the pane: drawn, a placeholder while it is drawn, or `None` to run
/// the command as it is (no drawer here, or not the pane's own parameters).
pub(super) fn ask(app: &mut WordApp, params: &Value) -> Option<Result<Value, String>> {
    let t = &mut app.library_thumbs;
    if t.off {
        return None;
    }
    let pane = pane_key(params)?;
    let ctx = t.ctx.clone()?;
    if t.drawer.is_none() {
        match Drawer::start(Pictures, move || ctx.request_repaint()) {
            Ok(d) => t.drawer = Some(d),
            Err(_) => {
                t.off = true;
                return None;
            }
        }
    }
    let answer = t.drawer.as_ref()?.ask(&app.session, params);
    let t = &mut app.library_thumbs;
    match answer {
        Ok(Answer::Drawn(v)) => {
            t.release(&pane);
            Some(Ok(v))
        }
        Ok(Answer::Drawing(key)) => {
            let v = t.placeholder()?;
            t.hold(pane, key);
            Some(Ok(v))
        }
        // The worker stopped: draw as the pane asks from now on.
        Err(_) if !t.drawer.as_ref().is_some_and(Drawer::alive) => {
            t.drawer = None;
            t.off = true;
            None
        }
        Err(e) => {
            t.release(&pane);
            Some(Err(e))
        }
    }
}

/// Before the pane is drawn: note it is shown, and put finished thumbnails in place.
pub(super) fn before_pane(app: &mut WordApp, ctx: &egui::Context, blank: Color32, mark: Color32) {
    let t = &mut app.library_thumbs;
    t.ctx = Some(ctx.clone());
    t.colors = (blank, mark);
    t.shown = t.frame;
    swap(t, ctx);
}

/// After the pane is drawn (which settles its tab, and may have changed its folder): drop what
/// waits when it shows another folder or tab than it did.
pub(super) fn after_pane(app: &mut WordApp) {
    let view = (app.session.prefs.library_folders.clone(), app.library.folder.clone(), app.library.tab.clone());
    if app.library_thumbs.view.as_ref().is_some_and(|v| *v != view) {
        forget(app);
    }
    app.library_thumbs.view = Some(view);
}

/// Once a frame, before anything is drawn (from [`WordApp::logic`]).
pub(crate) fn begin_frame(app: &mut WordApp) {
    app.library_thumbs.frame = app.library_thumbs.frame.wrapping_add(1);
}

/// Once a frame, after every window is drawn (from [`WordApp::ui`]): when none drew the pane,
/// it was closed or hidden, and what waits for it is dropped.
pub(crate) fn end_frame(app: &mut WordApp) {
    let t = &app.library_thumbs;
    if t.shown != t.frame && t.waiting() != (0, 0) {
        forget(app);
    }
}

/// Drop the pictures waiting; the pane forgets the placeholders it holds and asks again for the
/// thumbnails it shows (drawn ones are answered at once).
fn forget(app: &mut WordApp) {
    let t = &mut app.library_thumbs;
    if let Some(d) = &t.drawer {
        d.cancel();
        // Kept by the drawer: asked for again, they are answered from there.
        d.take_done();
    }
    t.finished.clear();
    if !t.held.is_empty() {
        t.held.clear();
        t.order.clear();
        app.library.refresh();
    }
}

/// Give the pane's placeholders the thumbnails finished for them, for at most [`SWAP_BUDGET`].
fn swap(t: &mut Thumbs, ctx: &egui::Context) {
    let Some(d) = &t.drawer else { return };
    t.finished.extend(d.take_done());
    if t.held.is_empty() {
        t.finished.clear();
        return;
    }
    let start = web_time::Instant::now();
    while let Some((key, thumb)) = t.finished.pop_front() {
        let waiting: Vec<PaneKey> = t.held.iter().filter(|(_, k)| **k == key).map(|(p, _)| p.clone()).collect();
        if waiting.is_empty() {
            continue;
        }
        let image = match thumb.ok().and_then(|th| letterboxed(&th.png, key.width)) {
            Some(img) => img,
            None => t.failed_image(),
        };
        let textures = ctx.tex_manager();
        let mut textures = textures.write();
        for pane in &waiting {
            let name = format!("library {} {} {}", pane.0, pane.1, pane.2);
            let id = textures.allocated().find(|(_, m)| m.name == name).map(|(id, _)| *id);
            // Gone when the pane let it go; asked for again, it is answered drawn.
            if let Some(id) = id {
                textures.set(id, egui::epaint::ImageDelta::full(image.clone(), TextureOptions::LINEAR));
            }
        }
        drop(textures);
        for pane in &waiting {
            t.release(pane);
        }
        if start.elapsed().as_secs_f64() > SWAP_BUDGET {
            break;
        }
    }
    if !t.finished.is_empty() {
        ctx.request_repaint();
    }
}

/// A thumbnail's PNG decoded, when it is a thumbnail's size.
fn decoded(png: &[u8]) -> Option<RgbaImage> {
    let max = u64::from(*THUMB_WIDTHS.end()) * u64::from(MAX_THUMB_HEIGHT) * 4;
    let img = wordcraft_geom::picture::decode(png, max).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || w > *THUMB_WIDTHS.end() || h > MAX_THUMB_HEIGHT {
        return None;
    }
    Some(img)
}

/// A finished thumbnail as the texture of a cell the pane gave the placeholder: the pane keeps a
/// cell the shape of the first image it was given, so the thumbnail comes in the placeholder's
/// shape, at most `width` pixels wide, the picture centred in it at its own shape and the bands
/// clear. `None` when the PNG isn't a thumbnail's size.
fn letterboxed(png: &[u8], width: u32) -> Option<ColorImage> {
    let img = decoded(png)?;
    let (pw, ph) = (u64::from(PLACEHOLDER.0), u64::from(PLACEHOLDER.1));
    let (w, h) = (u64::from(img.width()), u64::from(img.height()));
    // The smallest box of the placeholder's shape the thumbnail fits in, no wider than asked for.
    let boxed = w.max((h * pw).div_ceil(ph));
    let most = u64::from(width.clamp(*THUMB_WIDTHS.start(), *THUMB_WIDTHS.end()));
    let img = if boxed > most {
        let fit = |side: u64| u32::try_from(((side * most + boxed / 2) / boxed).max(1)).ok();
        image::imageops::resize(&img, fit(w)?, fit(h)?, FilterType::Triangle)
    } else {
        img
    };
    let (iw, ih) = (u64::from(img.width()), u64::from(img.height()));
    let cw = boxed.min(most).max(iw);
    let ch = ((cw * ph + pw / 2) / pw).max(ih);
    let mut canvas = RgbaImage::new(u32::try_from(cw).ok()?, u32::try_from(ch).ok()?);
    image::imageops::replace(&mut canvas, &img, i64::try_from((cw - iw) / 2).ok()?, i64::try_from((ch - ih) / 2).ok()?);
    Some(ColorImage::from_rgba_unmultiplied([usize::try_from(cw).ok()?, usize::try_from(ch).ok()?], canvas.as_raw()))
}

/// The pane's "?" for a picture that can't be drawn, on its blank-cell colour, drawn here
/// (anti-aliased from its outline's distance).
fn question_mark(bg: Color32, fg: Color32) -> ColorImage {
    let [w, h] = FAILED;
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    // The glyph is about as tall, against the cell, as the pane's 16-point one.
    let s = h as f32 * 0.14;
    let half = s * 0.08;
    let (ax, ay, r) = (cx, cy - s * 0.22, s * 0.21);
    // The hook runs from the left, over the top, to 40° below the right.
    let end = 40f32.to_radians();
    let (ex, ey) = (ax + r * end.cos(), ay + r * end.sin());
    let stem = [(ex, ey), (cx, cy + s * 0.08), (cx, cy + s * 0.16)];
    let (dx, dy, dr) = (cx, cy + s * 0.36, s * 0.1);
    let segment = |px: f32, py: f32, (x0, y0): (f32, f32), (x1, y1): (f32, f32)| {
        let (vx, vy) = (x1 - x0, y1 - y0);
        let len = vx * vx + vy * vy;
        let t = if len > 0.0 { (((px - x0) * vx + (py - y0) * vy) / len).clamp(0.0, 1.0) } else { 0.0 };
        (px - x0 - t * vx).hypot(py - y0 - t * vy)
    };
    let (b, f) = (bg.to_srgba_unmultiplied(), fg.to_srgba_unmultiplied());
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (ox, oy) = (px - ax, py - ay);
            let arc = if oy.atan2(ox) <= end { (ox.hypot(oy) - r).abs() } else { (px - (ax - r)).hypot(py - ay).min((px - ex).hypot(py - ey)) };
            let line = segment(px, py, stem[0], stem[1]).min(segment(px, py, stem[1], stem[2]));
            let d = (arc.min(line) - half).min((px - dx).hypot(py - dy) - dr);
            let cover = (0.5 - d).clamp(0.0, 1.0);
            for c in 0..4 {
                let (bc, fc) = (f32::from(b.get(c).copied().unwrap_or(0)), f32::from(f.get(c).copied().unwrap_or(0)));
                pixels.push((bc + (fc - bc) * cover).round() as u8);
            }
        }
    }
    ColorImage::from_rgba_unmultiplied([w, h], &pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_mark_is_drawn_in_the_middle_of_the_blank_cell() {
        let (bg, fg) = (Color32::from_rgb(240, 240, 240), Color32::from_rgb(90, 90, 90));
        let img = question_mark(bg, fg);
        assert_eq!(img.size, FAILED);
        let marked = img.pixels.iter().filter(|p| **p != bg).count();
        // Some ink, not a filled box.
        assert!(marked > 50 && marked < img.pixels.len() / 8, "{marked}");
        // The corners are the blank cell.
        assert_eq!(img.pixels.first(), Some(&bg));
        assert_eq!(img.pixels.last(), Some(&bg));
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 40, 255]));
        wordcraft_geom::picture::encode_png(&image::DynamicImage::ImageRgba8(img)).unwrap()
    }

    #[test]
    fn only_thumbnail_sized_pngs_become_textures() {
        assert_eq!(decoded(&png(512, 40)).map(|i| i.dimensions()), Some((512, 40)));
        assert!(decoded(&png(513, 40)).is_none());
        assert!(decoded(&png(8, MAX_THUMB_HEIGHT + 1)).is_none());
        assert!(decoded(b"not a png").is_none());
        assert!(letterboxed(b"not a png", 160).is_none());
    }

    /// The opaque part of `img`: its left, top, width and height.
    fn picture_in(img: &ColorImage) -> (usize, usize, usize, usize) {
        let [w, h] = img.size;
        let opaque = |x: usize, y: usize| img.pixels.get(y * w + x).is_some_and(|p| p.a() > 0);
        let cols: Vec<usize> = (0..w).filter(|&x| (0..h).any(|y| opaque(x, y))).collect();
        let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| opaque(x, y))).collect();
        let (x0, y0) = (cols.first().copied().unwrap_or(0), rows.first().copied().unwrap_or(0));
        (x0, y0, cols.len(), rows.len())
    }

    #[test]
    fn a_late_thumbnail_keeps_the_placeholder_shape_with_the_picture_centred_at_its_own() {
        // Already the shape: as drawn.
        let img = letterboxed(&png(160, 120), 160).unwrap();
        assert_eq!((img.size, picture_in(&img)), ([160, 120], (0, 0, 160, 120)));
        // A portrait photo: clear bands left and right.
        let img = letterboxed(&png(120, 160), 160).unwrap();
        assert_eq!(img.size, [160, 120]);
        let (x, y, w, h) = picture_in(&img);
        assert_eq!((y, h), (0, 120));
        assert_eq!(x, (160 - w) / 2, "centred");
        assert!((w as f32 / h as f32 - 0.75).abs() < 0.02, "{w} × {h}");
        // A panorama: bands above and below.
        let img = letterboxed(&png(400, 100), 256).unwrap();
        assert_eq!(img.size, [256, 192]);
        let (x, y, w, h) = picture_in(&img);
        assert_eq!((x, w, h), (0, 256, 64));
        assert_eq!(y, (192 - 64) / 2, "centred");
        // A small picture isn't enlarged: the pane scales the cell.
        let img = letterboxed(&png(30, 40), 160).unwrap();
        assert_eq!(img.size, [54, 41]);
        assert_eq!(picture_in(&img), (12, 0, 30, 40));
        // The tallest thumbnail stays within the width asked for, a pixel wide at least.
        let img = letterboxed(&png(1, MAX_THUMB_HEIGHT), 512).unwrap();
        assert_eq!(img.size, [512, 384]);
        assert_eq!(picture_in(&img).2, 1);
    }
}
