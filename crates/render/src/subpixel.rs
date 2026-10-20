//! Subpixel (LCD) text for on-screen page rasters, as FreeType draws text with `rgba` set in
//! fontconfig (Firefox and the other apps on Linux and the BSDs): each glyph is rasterised at three
//! times the horizontal resolution, smoothed with FreeType's default LCD filter, and each third of
//! a pixel lights its own colour channel, so stems sit on a third of a pixel instead of a whole one.
//!
//! The page is drawn by vello_cpu in sections: everything up to a run of subpixel text is rendered
//! onto the page's pixels (src-over), then the run is blended into those pixels per channel, so
//! text stays above what is under it and below what is drawn after it. Only for opaque rasters
//! shown one texel per screen pixel (the canvas pages at rest): a stretched or rotated one would
//! smear the coloured edges.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use vello_cpu::kurbo::{self, Affine, BezPath, Shape};
use vello_cpu::peniko;
use vello_cpu::{RenderContext, Resources};
use wordcraft_doc::props::Rgb;
use wordcraft_fonts::{FaceRef, FontDb};

/// The order of a screen's colour subpixels, left to right (fontconfig's `rgba`: `rgb`, `bgr`).
/// Vertical orders aren't drawn this way: their text stays greyscale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subpixel {
    Rgb,
    Bgr,
}

/// FreeType's default LCD filter (`FT_LCD_FILTER_DEFAULT`): each subpixel takes in its two
/// neighbours on each side, which keeps the colour fringes faint. Sums to 256.
const LCD_FILTER: [u32; 5] = [8, 77, 86, 77, 8];

/// Glyph masks kept (per face, glyph, size, third of a pixel, style).
const MASK_CACHE_MAX: usize = 20_000;
/// Largest glyph mask side, in pixels: larger glyphs (display sizes) stay greyscale.
const MASK_SIDE_MAX: f64 = 512.0;

/// A page's drawing target: vello_cpu, and with subpixel text, the pixels its sections are
/// rendered onto with the text blended in. Derefs to the [`RenderContext`] for everything else.
///
/// Text runs wait to be blended, and what vello_cpu draws waits to be rendered: at the end, or
/// when something is drawn over text that waits, the section is rendered and the text blended
/// onto it. Text drawn after something it overlaps comes out right that way, as the section is
/// under it; only something drawn over earlier text needs that text on the pixels first. Most
/// pages are so rendered in one or two sections.
pub(crate) struct Painter {
    ctx: RenderContext,
    lcd: Option<Lcd>,
}

struct Lcd {
    order: Subpixel,
    width: u16,
    height: u16,
    pixels: Vec<u8>,
    resources: Resources,
    /// Something was drawn through vello_cpu since the last section was rendered.
    dirty: bool,
    /// Text (and lines drawn over it) waiting to be blended onto the section, in drawing order.
    runs: Vec<Waiting>,
    /// The open clips (page path, its transform, device rectangle), innermost last: a section is
    /// rendered with them closed (vello_cpu renders only without open layers) and they are opened
    /// again after; subpixel text is clipped to their device rectangles.
    clips: Vec<(BezPath, Affine, kurbo::Rect)>,
    /// Something drawn through vello_cpu comes with a note of where ([`Painter::note`]); before the
    /// first note it may be anywhere.
    armed: bool,
    /// How many faded layers are open ([`Painter::push_opacity`]): text in them is greyscale.
    faded: u32,
}

/// What waits to be blended onto the section: a run of subpixel text (its glyphs' masks and
/// origin pixels), or a rectangle (a straight solid line drawn over text that waits, which a
/// section would otherwise have to be rendered for); in a colour, opacity (0–255), clip and
/// device bounds.
struct Waiting {
    placed: Vec<(Arc<Mask>, i64, i64)>,
    rect: Option<kurbo::Rect>,
    rgb: [u8; 3],
    alpha: u32,
    clip: PixelRect,
    bounds: kurbo::Rect,
}

impl Waiting {
    /// Does device rectangle `d` meet it (any of a run's glyphs)?
    fn meets(&self, d: kurbo::Rect) -> bool {
        self.bounds.overlaps(d)
            && (self.rect.is_some()
                || self.placed.iter().any(|(m, x, y)| {
                    let r = kurbo::Rect::new(
                        (x + m.left) as f64,
                        (y + m.top) as f64,
                        (x + m.left + i64::from(m.width)) as f64,
                        (y + m.top + i64::from(m.height)) as f64,
                    );
                    r.overlaps(d)
                }))
    }
}

impl std::ops::Deref for Painter {
    type Target = RenderContext;
    fn deref(&self) -> &RenderContext {
        &self.ctx
    }
}

impl std::ops::DerefMut for Painter {
    fn deref_mut(&mut self) -> &mut RenderContext {
        if let Some(lcd) = &mut self.lcd {
            // Drawn without a note of where: it may be over text that waits.
            if !lcd.armed && !lcd.runs.is_empty() {
                flush(&mut self.ctx, lcd);
            }
            lcd.dirty = true;
        }
        &mut self.ctx
    }
}

impl Painter {
    /// A painter over `ctx` (`width`×`height`), drawing text with `subpixel` order when given.
    pub(crate) fn new(ctx: RenderContext, width: u16, height: u16, subpixel: Option<Subpixel>) -> Painter {
        let lcd = subpixel.map(|order| Lcd {
            order,
            width,
            height,
            pixels: vec![0; usize::from(width) * usize::from(height) * 4],
            resources: Resources::new(),
            dirty: false,
            runs: Vec::new(),
            clips: Vec::new(),
            armed: false,
            faded: 0,
        });
        Painter { ctx, lcd }
    }

    /// Note where the next thing drawn through vello_cpu lands on the raster (device pixels;
    /// `None`: anywhere): text that waits under it is blended first.
    pub(crate) fn note(&mut self, device: Option<kurbo::Rect>) {
        if let Some(lcd) = &mut self.lcd {
            let over = match device {
                Some(d) if d.is_finite() => lcd.runs.iter().any(|r| r.meets(d)),
                _ => !lcd.runs.is_empty(),
            };
            if over {
                flush(&mut self.ctx, lcd);
            }
            lcd.armed = true;
        }
    }

    /// Open a clip (`path` under `transform`, covering `device` on the raster).
    pub(crate) fn push_clip(&mut self, path: &BezPath, transform: Affine, device: kurbo::Rect) {
        self.ctx.set_transform(transform);
        self.ctx.push_clip_layer(path);
        if let Some(lcd) = &mut self.lcd {
            lcd.clips.push((path.clone(), transform, device));
        }
    }

    /// Close the innermost clip.
    pub(crate) fn pop_clip(&mut self) {
        self.ctx.pop_layer();
        if let Some(lcd) = &mut self.lcd {
            lcd.clips.pop();
        }
    }

    /// Open a layer faded to `alpha` as a whole. Text in it is drawn greyscale, through vello_cpu:
    /// subpixel text is blended straight onto the pixels, past the layer. Text that waits is
    /// blended first, so no section is rendered while the layer is open.
    pub(crate) fn push_opacity(&mut self, alpha: f32) {
        if let Some(lcd) = &mut self.lcd {
            if !lcd.runs.is_empty() {
                flush(&mut self.ctx, lcd);
            }
            lcd.faded += 1;
        }
        self.ctx.push_opacity_layer(alpha);
    }

    /// Close the layer [`push_opacity`](Self::push_opacity) opened.
    pub(crate) fn pop_opacity(&mut self) {
        self.ctx.pop_layer();
        if let Some(lcd) = &mut self.lcd {
            lcd.faded = lcd.faded.saturating_sub(1);
            lcd.dirty = true;
        }
    }

    /// The finished raster (premultiplied RGBA8, `width`×`height`) and the context, to keep.
    pub(crate) fn finish(mut self, width: u16, height: u16) -> (Vec<u8>, RenderContext) {
        let pixels = match self.lcd.take() {
            Some(mut lcd) => {
                flush(&mut self.ctx, &mut lcd);
                lcd.pixels
            }
            None => {
                self.ctx.flush();
                let mut pixels = vec![0u8; usize::from(width) * usize::from(height) * 4];
                let mut res = Resources::new();
                if let Some(pm) = vello_cpu::PixmapMut::new(width, height, &mut pixels) {
                    self.ctx.render(pm, &mut res);
                }
                pixels
            }
        };
        (pixels, self.ctx)
    }

    /// Draw a run of glyphs as subpixel text, if this raster has it: `glyphs` are (id, x, baseline)
    /// in page points under `view` (upright, one scale: the caller checks), hinted at `ppem`
    /// device pixels per em, `italic` slanted and outlined by `stroke_px` (synthetic bold), in
    /// `color` at `alpha`. Returns false, drawing nothing, when the raster has no subpixel text or
    /// a glyph can't be drawn so; the caller then draws the run greyscale.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn subpixel_run(
        &mut self,
        face: FaceRef,
        glyphs: &[(u32, f32, f32)],
        ppem: f32,
        view: Affine,
        visible: &kurbo::Rect,
        italic: bool,
        stroke_px: f64,
        color: Rgb,
        alpha: f32,
    ) -> bool {
        if self.lcd.as_ref().is_none_or(|lcd| lcd.faded > 0) || !alpha.is_finite() || alpha <= 0.0 || !stroke_px.is_finite() {
            return false;
        }
        // Every mask first: a run is drawn whole one way or the other.
        let mut placed = Vec::with_capacity(glyphs.len());
        for (gid, x, y) in glyphs {
            let (gx, gy) = (f64::from(*x), f64::from(*y));
            // Written so a NaN position is skipped too.
            let near = gx >= visible.x0 - 100.0 && gx <= visible.x1 && gy >= visible.y0 && gy <= visible.y1 + 200.0;
            if !near {
                continue;
            }
            let at = view * kurbo::Point::new(gx, gy);
            if !at.x.is_finite() || !at.y.is_finite() {
                continue;
            }
            // The glyph's origin to a third of a pixel; its baseline on a whole pixel, as hinted.
            let thirds = (at.x * 3.0).round();
            let (px, phase) = ((thirds / 3.0).floor(), thirds.rem_euclid(3.0));
            let Some(mask) = glyph_mask(face, *gid, ppem, phase as u8, italic, stroke_px) else { return false };
            placed.push((mask, px as i64, at.y.round() as i64));
        }
        let Some(lcd) = &mut self.lcd else { return false };
        let bounds = placed.iter().fold(None::<kurbo::Rect>, |b, (m, x, y)| {
            let r = kurbo::Rect::new(
                (x + m.left) as f64,
                (y + m.top) as f64,
                (x + m.left + i64::from(m.width)) as f64,
                (y + m.top + i64::from(m.height)) as f64,
            );
            Some(b.map_or(r, |b| b.union(r)))
        });
        if let Some(bounds) = bounds {
            let rgb = [color.0, color.1, color.2];
            let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
            lcd.runs.push(Waiting { placed, rect: None, rgb, alpha, clip: lcd.clip_bounds(), bounds });
        }
        true
    }

    /// Draw `rect` (device pixels: a straight solid line) in `color` at `alpha` after the text that
    /// waits, if it meets that text: no section need be rendered for it. Returns false, drawing
    /// nothing, otherwise (it is drawn through vello_cpu as usual).
    pub(crate) fn rect_over_text(&mut self, rect: kurbo::Rect, color: Rgb, alpha: f32) -> bool {
        let Some(lcd) = &mut self.lcd else { return false };
        if !rect.is_finite() || rect.area() <= 0.0 || !alpha.is_finite() || alpha <= 0.0 {
            return false;
        }
        // Where its edges' coverage may reach.
        let reach = rect.expand();
        if !lcd.runs.iter().any(|w| w.meets(reach)) {
            return false;
        }
        let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
        lcd.runs.push(Waiting {
            placed: Vec::new(),
            rect: Some(rect),
            rgb: [color.0, color.1, color.2],
            alpha,
            clip: lcd.clip_bounds(),
            bounds: reach,
        });
        true
    }
}

/// Render what was drawn through vello_cpu since the last section onto the pixels (src-over),
/// with the open clips closed for it and opened again after, then blend the text that waits.
fn flush(ctx: &mut RenderContext, lcd: &mut Lcd) {
    if lcd.dirty {
        for _ in &lcd.clips {
            ctx.pop_layer();
        }
        ctx.flush();
        if let Some(pm) = vello_cpu::PixmapMut::new(lcd.width, lcd.height, &mut lcd.pixels) {
            let settings = vello_cpu::RasterizerSettings { composite_mode: vello_cpu::CompositeMode::SrcOver, ..Default::default() };
            ctx.render_with(pm, &mut lcd.resources, settings);
        }
        ctx.reset();
        for (path, transform, _) in &lcd.clips {
            ctx.set_transform(*transform);
            ctx.push_clip_layer(path);
        }
        lcd.dirty = false;
    }
    let runs = std::mem::take(&mut lcd.runs);
    for run in &runs {
        if let Some(rect) = run.rect {
            lcd.fill(rect, run.rgb, run.alpha, run.clip);
        }
        for (mask, x, y) in &run.placed {
            lcd.blend(mask, *x, *y, run.rgb, run.alpha, run.clip);
        }
    }
}

/// Pixel bounds (x0, y0, x1, y1), end exclusive.
type PixelRect = (i64, i64, i64, i64);

impl Lcd {
    /// The pixels subpixel text may touch: the raster within every open clip (a pixel counts when
    /// its centre is inside).
    fn clip_bounds(&self) -> PixelRect {
        let mut b = (0, 0, i64::from(self.width), i64::from(self.height));
        for (_, _, r) in &self.clips {
            let (x0, y0, x1, y1) = if r.is_finite() { (r.x0.round(), r.y0.round(), r.x1.round(), r.y1.round()) } else { (0.0, 0.0, 0.0, 0.0) };
            b = (b.0.max(x0 as i64), b.1.max(y0 as i64), b.2.min(x1 as i64), b.3.min(y1 as i64));
        }
        b
    }

    /// Blend `mask` with its origin at pixel (`x`, `y`) in `rgb` at `alpha` (0–255), within `clip`.
    /// Each channel takes its own subpixel's coverage over an opaque pixel; a translucent one (no
    /// paper under it) takes the mean coverage as plain src-over, having no colour to fringe.
    fn blend(&mut self, mask: &Mask, x: i64, y: i64, rgb: [u8; 3], alpha: u32, clip: PixelRect) {
        let (mw, mh) = (i64::from(mask.width), i64::from(mask.height));
        let (x0, y0) = (x.saturating_add(mask.left), y.saturating_add(mask.top));
        // The mask's columns and rows inside the clip (which lies within the raster).
        let (c0, c1) = (clip.0.saturating_sub(x0).clamp(0, mw), clip.2.saturating_sub(x0).clamp(0, mw));
        let (r0, r1) = (clip.1.saturating_sub(y0).clamp(0, mh), clip.3.saturating_sub(y0).clamp(0, mh));
        if c0 >= c1 || r0 >= r1 {
            return;
        }
        let width = i64::from(self.width);
        let bgr = self.order == Subpixel::Bgr;
        let a = |c: u8| if alpha == 255 { u32::from(c) } else { u32::from(c) * alpha / 255 };
        for row in r0..r1 {
            // A row of the mask and the pixels it covers, side by side.
            let m = (row * mw + c0) as usize..(row * mw + c1) as usize;
            let at = ((y0 + row) * width + x0 + c0) as usize * 4;
            let (Some(cov), Some(dst)) = (mask.cov.get(m), self.pixels.get_mut(at..at + (c1 - c0) as usize * 4)) else { continue };
            for (c, d) in cov.iter().zip(dst.as_chunks_mut::<4>().0) {
                let [first, middle, last] = *c;
                if first | middle | last == 0 {
                    continue;
                }
                let (r, g, b) = if bgr { (last, middle, first) } else { (first, middle, last) };
                let (ar, ag, ab) = (a(r), a(g), a(b));
                if d[3] == 255 {
                    d[0] = mix(d[0], rgb[0], ar);
                    d[1] = mix(d[1], rgb[1], ag);
                    d[2] = mix(d[2], rgb[2], ab);
                } else {
                    // Premultiplied src-over with the mean coverage.
                    let cov = (ar + ag + ab) / 3;
                    for (i, s) in rgb.iter().enumerate() {
                        d[i] = ((u32::from(*s) * cov + u32::from(d[i]) * (255 - cov) + 127) / 255) as u8;
                    }
                    d[3] = ((255 * cov + u32::from(d[3]) * (255 - cov) + 127) / 255) as u8;
                }
            }
        }
    }
}

impl Lcd {
    /// Fill `rect` (device pixels) in `rgb` at `alpha` (0–255) within `clip`: each pixel takes the
    /// area of it the rectangle covers, as vello_cpu's analytic coverage gives it.
    fn fill(&mut self, rect: kurbo::Rect, rgb: [u8; 3], alpha: u32, clip: PixelRect) {
        let (x0, y0) = ((rect.x0.floor() as i64).max(clip.0), (rect.y0.floor() as i64).max(clip.1));
        let (x1, y1) = ((rect.x1.ceil() as i64).min(clip.2), (rect.y1.ceil() as i64).min(clip.3));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = i64::from(self.width);
        // How much of pixel `p` (along one axis) lies between `a` and `b`; in f32, as vello_cpu
        // reckons coverage, so the two round alike.
        let (rx0, ry0, rx1, ry1) = (rect.x0 as f32, rect.y0 as f32, rect.x1 as f32, rect.y1 as f32);
        let span = |p: i64, a: f32, b: f32| (((p + 1) as f32).min(b) - (p as f32).max(a)).clamp(0.0, 1.0);
        // The first and last columns (the same one for a rectangle within a pixel).
        let (cx0, cx1) = (span(x0, rx0, rx1), span(x1 - 1, rx0, rx1));
        for py in y0..y1 {
            let cy = span(py, ry0, ry1);
            let at = (py * width + x0) as usize * 4;
            let Some(row) = self.pixels.get_mut(at..at + (x1 - x0) as usize * 4) else { continue };
            let last = (x1 - x0 - 1) as usize;
            // Coverage to 8 bits as vello_cpu takes it, then the opacity; a row's inner pixels
            // share one coverage, only its two ends differ.
            let cov = |cx: f32| {
                let m = (cy * cx * 255.0).round() as u32;
                if alpha >= 255 { m } else { (m * alpha + 255) >> 8 }
            };
            let (first, inner, end) = (cov(cx0), cov(1.0), cov(cx1));
            for (i, d) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let c = if i == 0 {
                    first
                } else if i == last {
                    end
                } else {
                    inner
                };
                over(d, rgb, c);
            }
        }
    }
}

/// Pixel `d` (premultiplied) with `rgb` laid over it at `cov`/255: per channel over an opaque
/// pixel, premultiplied src-over over a translucent one.
fn over(d: &mut [u8; 4], rgb: [u8; 3], cov: u32) {
    if cov == 0 {
        return;
    }
    if d[3] == 255 {
        d[0] = mix(d[0], rgb[0], cov);
        d[1] = mix(d[1], rgb[1], cov);
        d[2] = mix(d[2], rgb[2], cov);
    } else {
        for (i, s) in rgb.iter().enumerate() {
            d[i] = ((u32::from(*s) * cov + u32::from(d[i]) * (255 - cov) + 127) / 255) as u8;
        }
        d[3] = ((255 * cov + u32::from(d[3]) * (255 - cov) + 127) / 255) as u8;
    }
}

/// `dst` moved towards `src` by `cov`/255, rounded as vello_cpu's 8-bit pipeline composites a
/// solid colour at partial coverage (`(dst·(255−cov) + src·cov + 255) >> 8`; full coverage is
/// the colour), so what is drawn here and what vello_cpu draws agree to the bit.
fn mix(dst: u8, src: u8, cov: u32) -> u8 {
    if cov >= 255 {
        return src;
    }
    let (d, s) = (u32::from(dst), u32::from(src));
    ((d * (255 - cov) + s * cov + 255) >> 8) as u8
}

/// A glyph's subpixel coverage: per pixel, its three subpixels' coverage (0–255), left to right;
/// `left`/`top` place the mask's first pixel relative to the glyph's origin pixel.
pub(crate) struct Mask {
    left: i64,
    top: i64,
    width: u32,
    height: u32,
    cov: Vec<[u8; 3]>,
}

type MaskKey = (u32, u32, u32, u8, bool, u64);
type CoverageKey = (u32, u32, u32, bool, u64);

fn mask_cache() -> &'static Mutex<HashMap<MaskKey, Option<Arc<Mask>>>> {
    static CACHE: OnceLock<Mutex<HashMap<MaskKey, Option<Arc<Mask>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Glyph coverages (one per glyph, size and style), from which its masks at each third of a pixel
/// are filtered: a third of a pixel is one column of the coverage, so rasterising once serves all
/// three.
fn coverage_cache() -> &'static Mutex<HashMap<CoverageKey, Option<Arc<Coverage>>>> {
    static CACHE: OnceLock<Mutex<HashMap<CoverageKey, Option<Arc<Coverage>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A glyph's coverage at three times the horizontal resolution (0–255 per subpixel, `width`×
/// `height`, rows top to bottom), its first column `left` subpixels right of its origin pixel's
/// left edge and its first row `top` rows below the baseline's.
pub(crate) struct Coverage {
    left: i64,
    top: i64,
    width: usize,
    height: usize,
    cov: Vec<u8>,
}

/// The mask of glyph `gid` hinted at `ppem`, its origin `phase` thirds of a pixel right of a pixel
/// edge, cached. `None` when the glyph has no hinted outline at this size or is too large.
fn glyph_mask(face: FaceRef, gid: u32, ppem: f32, phase: u8, italic: bool, stroke_px: f64) -> Option<Arc<Mask>> {
    let key = (face.id(), gid, ppem.to_bits(), phase, italic, stroke_px.to_bits());
    if let Some(m) = mask_cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return m.clone();
    }
    let ckey = (face.id(), gid, ppem.to_bits(), italic, stroke_px.to_bits());
    let known = coverage_cache().lock().unwrap_or_else(|e| e.into_inner()).get(&ckey).cloned();
    let coverage = match known {
        Some(c) => c,
        None => {
            let c = FontDb::global().hinted_outline(&face, gid, ppem).and_then(|o| raster(&o, italic, stroke_px)).map(Arc::new);
            let mut cache = coverage_cache().lock().unwrap_or_else(|e| e.into_inner());
            if cache.len() >= MASK_CACHE_MAX {
                cache.clear();
            }
            cache.insert(ckey, c.clone());
            c
        }
    };
    let mask = coverage.map(|c| Arc::new(filtered(&c, phase)));
    let mut cache = mask_cache().lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= MASK_CACHE_MAX {
        cache.clear();
    }
    cache.insert(key, mask.clone());
    mask
}

/// Glyph-mask contexts kept per thread, per size class: making a vello_cpu context costs more than
/// rasterising a glyph in it (about 29 of 38 µs), so a mask reuses one at least its size.
const MASK_CONTEXTS_MAX: usize = 16;

/// Run `f` with a cleared vello_cpu context at least `w`×`h` and this thread's resources.
fn with_mask_context(w: u16, h: u16, f: impl FnOnce(&mut RenderContext, &mut Resources)) {
    type Contexts = (HashMap<(u16, u16), RenderContext>, Resources);
    thread_local! {
        static CONTEXTS: std::cell::RefCell<Option<Contexts>> = const { std::cell::RefCell::new(None) };
    }
    // Sizes rounded up, so glyphs of a size share a context.
    let class = (w.div_ceil(48).saturating_mul(48).max(48), h.div_ceil(16).saturating_mul(16).max(16));
    CONTEXTS.with_borrow_mut(|slot| {
        let (contexts, res) = slot.get_or_insert_with(|| (HashMap::new(), Resources::new()));
        if !contexts.contains_key(&class) && contexts.len() >= MASK_CONTEXTS_MAX {
            contexts.clear();
        }
        let ctx = contexts
            .entry(class)
            .or_insert_with(|| RenderContext::new_with(class.0, class.1, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() }));
        ctx.reset();
        f(ctx, res);
    });
}

/// Rasterise `outline` (device pixels, y-down, origin at the glyph's origin) at three times the
/// horizontal resolution, `phase` thirds of a pixel to the right, and filter it into a [`Mask`].
#[cfg(test)]
pub(crate) fn rasterize(outline: &BezPath, phase: u8, italic: bool, stroke_px: f64) -> Option<Mask> {
    raster(outline, italic, stroke_px).map(|c| filtered(&c, phase))
}

/// The [`Coverage`] of `outline` (device pixels, y-down, origin at the glyph's origin), slanted
/// when `italic` and outlined by `stroke_px`: `None` for a glyph too large for a mask.
pub(crate) fn raster(outline: &BezPath, italic: bool, stroke_px: f64) -> Option<Coverage> {
    let skew = if italic { Affine::new([1.0, 0.0, -0.21, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
    // Subpixel space: x in thirds of a pixel from the origin pixel's left edge.
    let to_sub = Affine::scale_non_uniform(3.0, 1.0) * skew;
    let half = stroke_px.max(0.0) / 2.0;
    let b = to_sub.transform_rect_bbox(outline.bounding_box().inflate(half, half));
    if !b.is_finite() || b.width() > MASK_SIDE_MAX * 3.0 || b.height() > MASK_SIDE_MAX {
        return None;
    }
    if b.width() <= 0.0 || b.height() <= 0.0 {
        return Some(Coverage { left: 0, top: 0, width: 0, height: 0, cov: Vec::new() });
    }
    let (left, top) = (b.x0.floor() as i64, b.y0.floor() as i64);
    let (w, h) = (u16::try_from(b.x1.ceil() as i64 - left).ok()?, u16::try_from(b.y1.ceil() as i64 - top).ok()?);
    let mut px = vec![0u8; usize::from(w) * usize::from(h) * 4];
    with_mask_context(w, h, |ctx, res| {
        ctx.set_transform(Affine::translate((-left as f64, -top as f64)) * to_sub);
        ctx.set_paint(peniko::Color::WHITE);
        ctx.fill_path(outline);
        if stroke_px > 0.0 {
            ctx.set_stroke(kurbo::Stroke::new(stroke_px));
            ctx.stroke_path(outline);
        }
        ctx.flush();
        // The context may be larger than the coverage: what lies past it is cut off.
        if let Some(pm) = vello_cpu::PixmapMut::new(w, h, &mut px) {
            ctx.render(pm, res);
        }
    });
    let cov = px.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
    Some(Coverage { left, top, width: usize::from(w), height: usize::from(h), cov })
}

/// The [`Mask`] of `coverage` with the glyph's origin `phase` thirds of a pixel right of a pixel
/// edge (its coverage `phase` columns further right), through FreeType's LCD filter: whole pixels
/// around it, reaching the filter's two subpixels past each side.
pub(crate) fn filtered(coverage: &Coverage, phase: u8) -> Mask {
    let first = coverage.left + i64::from(phase.min(2));
    let (left, right) = ((first - 2).div_euclid(3), (first + coverage.width as i64 + 2 + 2).div_euclid(3));
    let width = (right - left).max(0) as usize;
    let mut cov = Vec::with_capacity(width * coverage.height);
    for row in coverage.cov.chunks(coverage.width.max(1)).take(coverage.height) {
        // Subpixel `s` (from the origin pixel's left edge) of this row, 0 outside the coverage.
        let at = |s: i64| usize::try_from(s - first).ok().and_then(|i| row.get(i)).map_or(0, |c| u32::from(*c));
        let filter = |s: i64| -> u8 {
            let sum: u32 = LCD_FILTER.iter().zip(-2..=2).map(|(w, k)| w * at(s + k)).sum();
            (sum / 256).min(255) as u8
        };
        for p in left..right {
            cov.push([filter(p * 3), filter(p * 3 + 1), filter(p * 3 + 2)]);
        }
    }
    Mask { left, top: coverage.top, width: width as u32, height: coverage.height as u32, cov }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The coverage a glyph mask's subpixels get from a fresh context exactly the mask's size.
    fn fresh(outline: &BezPath, phase: u8, italic: bool, stroke_px: f64, m: &Mask) -> Vec<u8> {
        let skew = if italic { Affine::new([1.0, 0.0, -0.21, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
        let to_sub = Affine::translate((f64::from(phase), 0.0)) * Affine::scale_non_uniform(3.0, 1.0) * skew;
        let (w3, h) = ((m.width * 3) as u16, m.height as u16);
        let mut ctx = RenderContext::new_with(w3, h, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() });
        ctx.set_transform(Affine::translate(((-m.left * 3) as f64, -m.top as f64)) * to_sub);
        ctx.set_paint(peniko::Color::WHITE);
        ctx.fill_path(outline);
        if stroke_px > 0.0 {
            ctx.set_stroke(kurbo::Stroke::new(stroke_px));
            ctx.stroke_path(outline);
        }
        ctx.flush();
        let mut px = vec![0u8; usize::from(w3) * usize::from(h) * 4];
        if let Some(pm) = vello_cpu::PixmapMut::new(w3, h, &mut px) {
            ctx.render(pm, &mut Resources::new());
        }
        px.chunks(4).map(|p| p[3]).collect()
    }

    /// `rect` (device pixels) in `rgb` over a `bg` raster, by vello_cpu and by [`Lcd::fill`].
    fn both_fills(rect: kurbo::Rect, rgb: [u8; 3], bg: [u8; 3]) -> (Vec<u8>, Vec<u8>) {
        let (w, h) = (16u16, 10u16);
        let mut ctx = RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() });
        ctx.set_paint(peniko::Color::from_rgba8(bg[0], bg[1], bg[2], 255));
        ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        ctx.set_paint(peniko::Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
        ctx.fill_rect(&rect);
        ctx.flush();
        let mut vello = vec![0u8; usize::from(w) * usize::from(h) * 4];
        if let Some(pm) = vello_cpu::PixmapMut::new(w, h, &mut vello) {
            ctx.render(pm, &mut Resources::new());
        }
        let mut lcd = Lcd {
            order: Subpixel::Rgb,
            width: w,
            height: h,
            pixels: [bg[0], bg[1], bg[2], 255].repeat(usize::from(w) * usize::from(h)),
            resources: Resources::new(),
            dirty: false,
            runs: Vec::new(),
            clips: Vec::new(),
            armed: false,
            faded: 0,
        };
        lcd.fill(rect, rgb, 255, (0, 0, i64::from(w), i64::from(h)));
        (vello, lcd.pixels)
    }

    #[test]
    fn rectangles_drawn_over_text_match_vello_to_the_bit() {
        // Table borders and cell shading drawn after text are filled here, not by vello_cpu:
        // fractional edges, thin lines, a rectangle inside one pixel, colours over colours.
        let rects = [
            (2.06, 1.3, 9.94, 7.5),
            (0.5, 0.25, 15.75, 0.96),
            (4.2, 3.1, 4.7, 3.9),
            (3.0, 2.0, 11.0, 8.0),
            (-2.0, -1.0, 7.333, 4.667),
            (12.9, 8.6, 20.0, 12.0),
        ];
        for (x0, y0, x1, y1) in rects {
            for (rgb, bg) in [([193, 228, 245], [255, 255, 255]), ([0, 0, 0], [193, 228, 245]), ([21, 96, 130], [240, 10, 99])] {
                let (vello, ours) = both_fills(kurbo::Rect::new(x0, y0, x1, y1), rgb, bg);
                assert!(vello == ours, "({x0}, {y0})–({x1}, {y1}) {rgb:?} over {bg:?}");
            }
        }
    }

    #[test]
    fn masks_from_reused_contexts_match_fresh_ones() {
        // Glyphs of several sizes, interleaved, so each context is reused by others before.
        let face = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
        let db = FontDb::global();
        let mut checked = 0;
        for ppem in [9.0, 21.27, 47.5, 15.0] {
            for c in "Wgi@%Q".chars() {
                let Some(o) = db.hinted_outline(&face, face.glyph_for(c), ppem) else { continue };
                for (phase, italic, stroke) in [(0, false, 0.0), (2, true, 0.8), (1, false, 0.0)] {
                    let m = rasterize(&o, phase, italic, stroke).unwrap();
                    let cov = fresh(&o, phase, italic, stroke, &m);
                    let w3 = m.width as usize * 3;
                    // The mask is the filtered coverage: compare it filtered the same way.
                    for (i, got) in m.cov.iter().enumerate() {
                        let (row, col) = (i / m.width as usize, i % m.width as usize);
                        let line = &cov[row * w3..(row + 1) * w3];
                        let f = |s: usize| -> u8 {
                            let sum: u32 = LCD_FILTER
                                .iter()
                                .enumerate()
                                .map(|(k, w)| (s + k).checked_sub(2).and_then(|j| line.get(j)).map_or(0, |c| u32::from(*c) * w))
                                .sum();
                            (sum / 256).min(255) as u8
                        };
                        assert_eq!(*got, [f(col * 3), f(col * 3 + 1), f(col * 3 + 2)], "{c} at {ppem} px, phase {phase}");
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked >= 60, "{checked}");
    }
}
