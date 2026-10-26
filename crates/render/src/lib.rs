//! WordCraft rasteriser: draws a page's display list with vello_cpu.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use vello_cpu::kurbo::{self, Affine, BezPath, Shape};
use vello_cpu::peniko;
use vello_cpu::{Pixmap, RenderContext, Resources};
use wordcraft_doc::Document;
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::Rgb;
use wordcraft_fonts::FontDb;
use wordcraft_layout::Page;
use wordcraft_layout::display::Paint;
use wordcraft_layout::display::{DisplayOptions, Draw, Stroke, page_layers};
use wordcraft_metafile::{Item, Picture, PlacedItem, Seg};

mod color_glyph;
pub mod icon_lib;

/// Worker threads for rasterising (0 on the web, where there are no threads).
pub fn default_threads() -> u16 {
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism().map(|n| (n.get().saturating_sub(1)).clamp(1, 8) as u16).unwrap_or(2)
    }
}

mod context_pool;

/// Largest raster side vello_cpu handles comfortably.
pub const MAX_SIDE: u32 = 16_000;

/// A rendered image (premultiplied RGBA8, row-major).
#[derive(Clone, Default)]
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rendered {
    pub fn to_straight(&self) -> Vec<u8> {
        let mut out = self.pixels.clone();
        for px in out.as_chunks_mut::<4>().0 {
            let a = px[3] as u32;
            if a != 0 && a != 255 {
                for c in px.iter_mut().take(3) {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        out
    }
    /// Whether the pixels are exactly `width`×`height` RGBA, within a page raster's budget
    /// ([`wordcraft_geom::MAX_RASTER_PIXELS`]): the fields are public, so an encoder is never asked
    /// to pad a short buffer out to a huge claimed size (or, for JPEG, to assert on a mismatch).
    fn encodable(&self) -> bool {
        let budget = wordcraft_geom::MAX_RASTER_PIXELS.saturating_mul(4);
        wordcraft_geom::rgba_fits(self.width, self.height, budget)
            && wordcraft_geom::rgba_bytes(self.width, self.height).is_some_and(|b| b == self.pixels.len() as u64)
    }
    /// PNG bytes (empty on failure, logged).
    pub fn to_png(&self) -> Vec<u8> {
        if !self.encodable() {
            log::error!("PNG encode: {} bytes for {}×{} pixels", self.pixels.len(), self.width, self.height);
            return Vec::new();
        }
        let Some(img) = image::RgbaImage::from_raw(self.width, self.height, self.to_straight()) else {
            log::error!("PNG encode: bad size {}×{}", self.width, self.height);
            return Vec::new();
        };
        let mut buf = Vec::new();
        if let Err(e) = img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png) {
            log::error!("PNG encode: {e}");
            return Vec::new();
        }
        buf
    }
    /// JPEG bytes on white (empty on failure, logged).
    pub fn to_jpeg(&self, quality: u8) -> Vec<u8> {
        if !self.encodable() {
            log::error!("JPEG encode: {} bytes for {}×{} pixels", self.pixels.len(), self.width, self.height);
            return Vec::new();
        }
        let rgba = self.to_straight();
        let rgb: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let a = p[3] as u32;
                let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
                [mix(p[0]), mix(p[1]), mix(p[2])]
            })
            .collect();
        let mut buf = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
        let _ = image::ImageEncoder::write_image(enc, &rgb, self.width, self.height, image::ExtendedColorType::Rgb8);
        buf
    }
    /// Straight RGBA at (x, y) (transparent outside).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = ((y as usize * self.width as usize) + x as usize) * 4;
        let Some(p) = self.pixels.get(i..i + 4) else { return [0; 4] };
        let a = p[3] as u32;
        if a == 0 {
            return [0; 4];
        }
        let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
        [un(p[0]), un(p[1]), un(p[2]), p[3]]
    }
}

/// Paint `p` as the context's paint (gradients in page points, under the view transform).
fn set_paint(ctx: &mut RenderContext, p: &Paint, opts: &RenderOptions) {
    set_paint_inked(ctx, p, |c| opts.ink(c));
}

/// [`set_paint`] with each colour passed through `ink` (a dark page's text inking, #1252).
fn set_paint_inked(ctx: &mut RenderContext, p: &Paint, ink: impl Fn(Rgb) -> Rgb) {
    let stops = |stops: &[(f32, Rgb, f32)]| -> Vec<peniko::ColorStop> {
        stops.iter().map(|(o, c, a)| peniko::ColorStop::from((o.clamp(0.0, 1.0), color(ink(*c), *a)))).collect()
    };
    match p {
        Paint::Solid(c, a) => ctx.set_paint(color(ink(*c), *a)),
        Paint::Linear { p0, p1, stops: s } => ctx.set_paint(
            peniko::Gradient::new_linear((f64::from(p0.0), f64::from(p0.1)), (f64::from(p1.0), f64::from(p1.1))).with_stops(stops(s).as_slice()),
        ),
        Paint::Radial { c, r, stops: s } => {
            ctx.set_paint(peniko::Gradient::new_radial((f64::from(c.0), f64::from(c.1)), *r).with_stops(stops(s).as_slice()))
        }
    }
}

fn color(c: Rgb, alpha: f32) -> peniko::Color {
    peniko::Color::from_rgba8(c.0, c.1, c.2, (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Upper bound on the bytes the two decoded-media caches keep together (about 256 MB), half each;
/// past its half a cache starts over.
const CACHE_BYTES: usize = 256 * 1024 * 1024 / 2;
/// Upper bound on cached entries per cache.
const CACHE_ENTRIES: usize = 256;

/// Decoded media by media key, shared across renders, with an approximate byte budget. An entry knows
/// its source allocation weakly: a hit needs the same allocation, whose address can't be reused while
/// the entry lives, and the source bytes are still freed with the document.
struct MediaCache<T> {
    map: HashMap<String, Entry<T>>,
    bytes: usize,
}

struct Entry<T> {
    src: Weak<Vec<u8>>,
    val: T,
    size: usize,
}

impl<T: Clone> MediaCache<T> {
    fn get(&self, key: &str, src: &Arc<Vec<u8>>) -> Option<T> {
        let e = self.map.get(key)?;
        std::ptr::eq(e.src.as_ptr(), Arc::as_ptr(src)).then(|| e.val.clone())
    }

    /// Stores `val` (about `size` bytes) for `src`. Clears the cache first when the budget or entry
    /// cap is hit; an entry larger than the whole budget is not kept.
    fn insert(&mut self, key: &str, src: &Arc<Vec<u8>>, val: T, size: usize) {
        if size > CACHE_BYTES {
            return;
        }
        if let Some(old) = self.map.remove(key) {
            self.bytes = self.bytes.saturating_sub(old.size);
        }
        if self.map.len() >= CACHE_ENTRIES || self.bytes.saturating_add(size) > CACHE_BYTES {
            self.map.clear();
            self.bytes = 0;
        }
        self.map.insert(key.to_string(), Entry { src: Arc::downgrade(src), val, size });
        self.bytes = self.bytes.saturating_add(size);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn images() -> &'static Mutex<MediaCache<Option<Arc<Pixmap>>>> {
    static C: std::sync::OnceLock<Mutex<MediaCache<Option<Arc<Pixmap>>>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(MediaCache { map: HashMap::new(), bytes: 0 }))
}

fn vectors() -> &'static Mutex<MediaCache<Option<Arc<Vector>>>> {
    static C: std::sync::OnceLock<Mutex<MediaCache<Option<Arc<Vector>>>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(MediaCache { map: HashMap::new(), bytes: 0 }))
}

/// Largest side (pixels) a document picture may decode to: what a `Pixmap` can hold (`u16`
/// sides, which [`premul_pixmap`] enforces after decoding). Applying it as a decode limit rejects
/// a small file that declares a bigger image at header time, before anything is allocated.
const MAX_DECODE_SIDE: u32 = u16::MAX as u32;

/// Decode an encoded image to a premultiplied pixmap. The dimension limit above and the default
/// allocation cap (512 MB, what `load_from_memory` applied) bound the decode during paint.
pub fn decode_pixmap(bytes: &[u8]) -> Option<Pixmap> {
    // The shared probe also charges what the decoder holds beside its output (#3644).
    if !wordcraft_geom::picture::probe(bytes).is_ok_and(|p| p.fits(wordcraft_geom::MAX_DECODED_BYTES)) {
        return None;
    }
    decode_pixmap_within(bytes, wordcraft_geom::MAX_DECODED_BYTES)
}

/// [`decode_pixmap`] with an explicit RGBA byte budget (a small one in tests).
fn decode_pixmap_within(bytes: &[u8], budget: u64) -> Option<Pixmap> {
    use image::ImageDecoder;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_SIDE);
    limits.max_image_height = Some(MAX_DECODE_SIDE);
    let reader = || {
        let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
        r.limits(limits.clone());
        Some(r)
    };
    // The header gives the size and the decoder's pixel format (16-bit RGBA takes 8 bytes a pixel
    // before its RGBA8 copy); refuse before the decoder (and the RGBA copy) allocate anything.
    let (hw, hh, native) = {
        let d = reader()?.into_decoder().ok()?;
        let (w, h) = d.dimensions();
        (w, h, u64::from(d.color_type().bytes_per_pixel()))
    };
    if !wordcraft_geom::decode_peak_fits(hw, hh, native, budget) {
        // Logged once a picture: its refusal is cached with it (`pixmap_for`).
        log::warn!(
            "a {hw}×{hh} picture of {native} bytes a pixel needs more than {budget} bytes to decode: it is not drawn (in the text, a grey box takes its place)"
        );
        return None;
    }
    let img = reader()?.decode().ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    premul_pixmap(w, h, img.as_raw())
}

/// A premultiplied pixmap from straight RGBA8 rows. None for empty, oversized or mis-sized input
/// (the pixmap constructor asserts the length).
fn premul_pixmap(w: u32, h: u32, rgba: &[u8]) -> Option<Pixmap> {
    if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
        return None;
    }
    if rgba.len() != (w as usize * h as usize).saturating_mul(4) {
        return None;
    }
    let data: Vec<vello_cpu::color::PremulRgba8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let a = p[3] as u16;
            let m = |c: u8| ((c as u16 * a + 127) / 255) as u8;
            vello_cpu::color::PremulRgba8 { r: m(p[0]), g: m(p[1]), b: m(p[2]), a: p[3] }
        })
        .collect();
    Some(Pixmap::from_parts(data, w as u16, h as u16))
}

/// Pixel size of an encoded image.
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

/// The natural size in points of a Windows Metafile or Enhanced Metafile picture, and the file
/// extension it is stored under (`"wmf"` or `"emf"`), or None for anything else (#802). Very large
/// frames are scaled down to 22 inches on the longer side, keeping the shape.
pub fn metafile_size(bytes: &[u8]) -> Option<(f32, f32, &'static str)> {
    if !wordcraft_metafile::is_metafile(bytes) {
        return None;
    }
    let pic = wordcraft_metafile::parse(bytes).ok()?;
    let (w, h) = (pic.width, pic.height);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let k = (1584.0 / w.max(h)).min(1.0);
    // An EMF starts with its header record, type 1.
    let ext = if bytes.get(..4) == Some(&[1, 0, 0, 0]) { "emf" } else { "wmf" };
    Some((w * k, h * k, ext))
}

/// A metafile (WMF/EMF) picture, parsed once. The geometry comes placed by the metafile crate; the
/// bitmaps are pixmaps, one per picture item (None for paths and undecodable bitmaps).
struct Vector {
    pic: Picture,
    pixmaps: Vec<Option<Arc<Pixmap>>>,
}

impl Vector {
    /// Approximate memory held: pixels and path segments.
    fn bytes(&self) -> usize {
        let px: usize = self.pixmaps.iter().flatten().map(|p| p.width() as usize * p.height() as usize * 4).sum();
        let segs: usize = self
            .pic
            .items
            .iter()
            .map(|it| match it {
                Item::Path { segs, .. } => segs.len() * std::mem::size_of::<Seg>(),
                Item::Bitmap { .. } => 0,
                Item::Text(t) => t.text.len() + t.dx.len() * 4,
            })
            .sum();
        px + segs
    }
}

/// The metafile picture for a media key. None when the bytes are not a metafile or do not parse, so
/// the caller falls back to the raster path (and the grey box).
fn vector_for(doc: &Document, key: &str) -> Option<Arc<Vector>> {
    let bytes = doc.media.get(key)?;
    if !wordcraft_metafile::is_metafile(bytes) {
        return None;
    }
    if let Some(v) = lock(vectors()).get(key, bytes) {
        return v;
    }
    let v = build_vector(bytes).map(Arc::new);
    if v.is_none() {
        log::warn!("picture `{key}` is a metafile that could not be read");
    }
    let size = v.as_ref().map_or(0, |v| v.bytes());
    lock(vectors()).insert(key, bytes, v.clone(), size);
    v
}

/// Parses a metafile and decodes its bitmaps. None when the picture has no usable size.
fn build_vector(bytes: &[u8]) -> Option<Vector> {
    let mut pic = wordcraft_metafile::parse(bytes).ok()?;
    if !pic.has_size() {
        return None;
    }
    // The pixels move into the pixmaps; the picture keeps only the geometry.
    let pixmaps = pic.items.iter_mut().map(|it| it.take_pixels().and_then(|(w, h, px)| premul_pixmap(w, h, &px)).map(Arc::new)).collect();
    Some(Vector { pic, pixmaps })
}

fn rgba_color(c: [u8; 4]) -> peniko::Color {
    peniko::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Draws a metafile picture into `r` (page points): fills and strokes in page space (so strokes keep
/// their width under non-uniform scaling), bitmaps stretched, all clipped to `r` and faded by `alpha`.
/// Strokes have round caps and joins, as GDI draws them, and are at least one device pixel wide, so
/// cosmetic (width 0) pens show as hairlines at any zoom.
fn draw_vector(ctx: &mut RenderContext, v: &Vector, r: &kurbo::Rect, crop: &[f32; 4], alpha: f32, view: Affine) {
    ctx.set_transform(view);
    let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
    let faded = alpha < 1.0;
    if faded {
        ctx.push_opacity_layer(alpha);
    }
    ctx.push_clip_layer(&r.to_path(0.1));
    let pixel = device_pixel(view);
    for it in v.pic.place(*r, *crop) {
        match it {
            PlacedItem::Path { path, fill, stroke, even_odd } => {
                if let Some(f) = fill {
                    ctx.set_paint(rgba_color(f));
                    ctx.set_fill_rule(if even_odd { peniko::Fill::EvenOdd } else { peniko::Fill::NonZero });
                    ctx.fill_path(&path);
                }
                if let Some((s, w)) = stroke {
                    ctx.set_paint(rgba_color(s));
                    ctx.set_stroke(kurbo::Stroke::new(w.max(pixel)).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round));
                    ctx.stroke_path(&path);
                }
            }
            PlacedItem::Bitmap { index, rect } => {
                let Some(Some(pm)) = v.pixmaps.get(index) else { continue };
                let (pw, ph) = (pm.width().max(1) as f64, pm.height().max(1) as f64);
                ctx.set_paint(vello_cpu::Image {
                    image: vello_cpu::ImageSource::Pixmap(pm.clone()),
                    sampler: peniko::ImageSampler::default().with_quality(peniko::ImageQuality::Medium),
                });
                ctx.set_paint_transform(Affine::translate((rect.x0, rect.y0)) * Affine::scale_non_uniform(rect.width() / pw, rect.height() / ph));
                ctx.fill_rect(&rect);
                ctx.reset_paint_transform();
            }
            PlacedItem::Text { index, tf } => {
                if let Some(Item::Text(t)) = v.pic.items.get(index) {
                    draw_vector_text(ctx, t, view * tf);
                }
                ctx.set_transform(view);
            }
        }
    }
    ctx.set_fill_rule(peniko::Fill::NonZero);
    ctx.pop_layer();
    if faded {
        ctx.pop_layer();
    }
}

/// Draws a metafile text run: `tf` maps the run's own space (see [`PlacedItem::Text`]) to the device.
fn draw_vector_text(ctx: &mut RenderContext, t: &wordcraft_metafile::Text, tf: Affine) {
    let face = wordcraft_fonts::word::resolve(&t.font, t.bold, t.italic).face;
    let shaped = wordcraft_fonts::shape(&face, &t.text, &[], |c| c);
    let glyphs: Vec<_> = shaped.iter().map(|g| (g.cluster, f64::from(g.x_advance), f64::from(g.x_offset), f64::from(g.y_offset))).collect();
    let Some(lay) = t.layout(face.upem, face.ascent, face.descent, &glyphs) else { return };
    let k = lay.em / face.upem.max(1.0);
    ctx.set_paint(rgba_color(t.color));
    let db = FontDb::global();
    let canvas = kurbo::Rect::new(0.0, 0.0, f64::from(ctx.width()), f64::from(ctx.height()));
    let mut oversized = 0usize;
    for (g, (x, y)) in shaped.iter().zip(&lay.glyphs) {
        let o = db.outline(&face, g.gid);
        let m = tf * Affine::translate((*x, *y)) * Affine::scale(k);
        // A record's font size is file data: a glyph many canvases tall (or off the canvas) would
        // have its outline flattened into millions of segments for nothing, so it is left out.
        let b = m.transform_rect_bbox(o.bounding_box());
        let sane = [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && b.width() <= MAX_GLYPH_PIXELS && b.height() <= MAX_GLYPH_PIXELS;
        if !sane {
            oversized = oversized.saturating_add(1);
            continue;
        }
        if !b.overlaps(canvas) {
            continue;
        }
        ctx.set_transform(m);
        ctx.fill_path(&o);
    }
    note_oversized_glyphs(&OVERSIZED_GLYPHS_NOTED, oversized);
}

/// Largest metafile glyph drawn, in device pixels per side (four times the widest canvas).
const MAX_GLYPH_PIXELS: f64 = 65_536.0;

/// Whether metafile glyphs left out for their size have been logged ([`note_oversized_glyphs`]).
static OVERSIZED_GLYPHS_NOTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Logs that `n` glyphs of a metafile text run were left out for being larger than
/// [`MAX_GLYPH_PIXELS`] (or of no finite size), the first time any are this run (`noted`): a
/// page is drawn again on every repaint, so a warning each time would flood the log. True when
/// this call logged it.
fn note_oversized_glyphs(noted: &std::sync::atomic::AtomicBool, n: usize) -> bool {
    if n == 0 || noted.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    log::warn!(
        "a picture's text has {n} letter(s) larger than {MAX_GLYPH_PIXELS} pixels drawn, left out (logged once a run; later ones are left out too)"
    );
    true
}

/// One device pixel in page points under `view` (its mean axis scale); 1 for a degenerate transform.
fn device_pixel(view: Affine) -> f64 {
    let c = view.as_coeffs();
    let scale = (c[0].hypot(c[1]) + c[2].hypot(c[3])) / 2.0;
    if scale.is_finite() && scale > 1e-9 { 1.0 / scale } else { 1.0 }
}

/// Largest page-background picture decoded, in pixels (hostile files).
const MAX_BACKGROUND_PIXELS: u64 = 64 * 1024 * 1024;

/// Draw the page's picture or texture fill (Design › Page Color › Fill Effects) behind the content:
/// tiled at its natural size (96 dpi) for a `tile` fill, stretched over the page otherwise.
fn draw_page_picture(ctx: &mut RenderContext, doc: &Document, page: &Page, view: Affine) {
    let Some(pic) = &doc.settings.page_picture else { return };
    let Some(bytes) = doc.media.get(&pic.media) else { return };
    let (nw, nh) = match image_size(bytes) {
        Some((w, h)) if w > 0 && h > 0 && (w as u64) * (h as u64) <= MAX_BACKGROUND_PIXELS => (w as f64, h as f64),
        _ => return,
    };
    let Some(pm) = pixmap_for(doc, &pic.media) else { return };
    let (pw, ph) = (pm.width().max(1) as f64, pm.height().max(1) as f64);
    let (w, h) = (page.w as f64, (page.h as f64).min(1e6));
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return;
    }
    let tile = pic.fill_type.eq_ignore_ascii_case("tile");
    // Tiles at the picture's natural size, whatever size its cached pixmap is kept at.
    let (sx, sy) = if tile { (0.75 * nw / pw, 0.75 * nh / ph) } else { (w / pw, h / ph) };
    let extend = if tile { peniko::Extend::Repeat } else { peniko::Extend::Pad };
    ctx.set_transform(view);
    ctx.set_paint(vello_cpu::Image {
        image: vello_cpu::ImageSource::Pixmap(pm),
        sampler: peniko::ImageSampler::default().with_quality(peniko::ImageQuality::Medium).with_extend(extend),
    });
    ctx.set_paint_transform(Affine::scale_non_uniform(sx, sy));
    ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w, h));
    ctx.reset_paint_transform();
}

fn pixmap_for(doc: &Document, key: &str) -> Option<Arc<Pixmap>> {
    let bytes = doc.media.get(key)?;
    if let Some(v) = lock(images()).get(key, bytes) {
        return v;
    }
    let pm = decode_pixmap(bytes).map(|p| Arc::new(cacheable(p)));
    let size = pm.as_ref().map_or(0, |p| p.width() as usize * p.height() as usize * 4);
    lock(images()).insert(key, bytes, pm.clone(), size);
    pm
}

/// A decoded picture at a size the media cache keeps. One too large for the whole cache is
/// box-averaged down to half of it, so it is decoded once instead of on every render (#3665); pages
/// draw it scaled to its frame either way.
fn cacheable(pm: Pixmap) -> Pixmap {
    let bytes = pm.width() as usize * pm.height() as usize * 4;
    if bytes <= CACHE_BYTES {
        return pm;
    }
    shrink(&pm, (CACHE_BYTES / 2 / 4) as u64).unwrap_or(pm)
}

/// `pm` averaged over square blocks, the smallest whole number of pixels a side that brings it to
/// at most `max_pixels`; None when it already fits or none are allowed. Premultiplied pixels
/// average without colour fringes.
fn shrink(pm: &Pixmap, max_pixels: u64) -> Option<Pixmap> {
    let (w, h) = (usize::from(pm.width()), usize::from(pm.height()));
    if (w * h) as u64 <= max_pixels || max_pixels == 0 {
        return None;
    }
    let mut k = (((w * h) as f64 / max_pixels as f64).sqrt().ceil() as usize).max(2);
    while (w.div_ceil(k) * h.div_ceil(k)) as u64 > max_pixels {
        k += 1;
    }
    let (nw, nh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = Vec::with_capacity(nw * nh);
    let mut sums = vec![[0u64; 4]; nw];
    for (oy, band) in pm.data().chunks(w * k).enumerate() {
        sums.fill([0; 4]);
        for row in band.chunks(w) {
            for (sum, block) in sums.iter_mut().zip(row.chunks(k)) {
                for p in block {
                    sum[0] += u64::from(p.r);
                    sum[1] += u64::from(p.g);
                    sum[2] += u64::from(p.b);
                    sum[3] += u64::from(p.a);
                }
            }
        }
        let rows = (h - oy * k).min(k);
        for (ox, sum) in sums.iter().enumerate() {
            let n = (((w - ox * k).min(k)) * rows).max(1) as u64;
            let [r, g, b, a] = sum.map(|s| (s / n) as u8);
            out.push(vello_cpu::color::PremulRgba8 { r, g, b, a });
        }
    }
    Some(Pixmap::from_parts(out, nw as u16, nh as u16))
}

/// Options for a page raster.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub display: DisplayOptions,
    /// Paper colour (page colour from Design › Page Color, else white).
    pub paper: Rgb,
    /// Colour for formatting marks.
    pub mark_color: Rgb,
    /// Thicken glyphs slightly the way macOS draws text on screen ("font smoothing"), so pages
    /// look like they do in other Mac apps. For on-screen rasters only: exports stay exact.
    pub text_darkening: bool,
    /// Dark page (View › Switch Modes): every colour but pictures has its lightness inverted, so
    /// white paper turns black and black text white while hues stay the same. Screen only.
    pub dark: bool,
    /// The grey that black becomes on a dark page (white paper turns this, not pure black).
    pub dark_paper: u8,
    /// Read Mode's Page Color: paper drawn instead of the document's page color and page
    /// picture. `None` shows the document's page background. Screen only.
    pub read_paper: Option<Rgb>,
}

/// Default grey for a dark page's paper.
pub const DARK_PAPER: u8 = 0x33;

/// Read Mode's Sepia paper: Word's RGB (112, 66, 20) at 80% transparency over white.
pub const SEPIA_PAPER: Rgb = Rgb(0xE2, 0xD9, 0xD0);

impl RenderOptions {
    /// The on-screen color of the paper under `doc`'s pages: [`RenderOptions::read_paper`],
    /// else the document's page color, else [`RenderOptions::paper`].
    pub fn paper_of(&self, doc: &Document) -> Rgb {
        self.ink(self.read_paper.or(doc.settings.page_color).unwrap_or(self.paper))
    }

    /// The on-screen colour for a document colour (see [`RenderOptions::dark`]).
    pub fn ink(&self, c: Rgb) -> Rgb {
        if !self.dark {
            return c;
        }
        // Invert, then lift the blacks to the paper grey so the page isn't a black hole.
        let Rgb(r, g, b) = invert_lightness(c);
        let p = self.dark_paper as u32;
        let lift = |v: u8| (p + v as u32 * (255 - p) / 255).min(255) as u8;
        Rgb(lift(r), lift(g), lift(b))
    }

    /// The on-screen colour for a text background (highlight, character shading). A dark page
    /// dims it toward black, keeping its hue, instead of inverting it: inverting leaves bright
    /// hues such as yellow bright, so the white text on them can't be read (#1252). Every
    /// colour lands dark enough for white text (WCAG contrast ≥ 4.5); yellow turns a dim olive.
    pub fn ink_bg(&self, c: Rgb) -> Rgb {
        if !self.dark {
            return c;
        }
        let f = |v: u8| (DARK_TEXT_BG_FLOOR as u32 + v as u32 * DARK_TEXT_BG_SPAN as u32 / 255).min(255) as u8;
        Rgb(f(c.0), f(c.1), f(c.2))
    }

    /// The on-screen colour for text (or its underline) drawn on the text background `bg`, if
    /// any. A dark page dims those backgrounds ([`RenderOptions::ink_bg`]), so the text takes the
    /// lighter of its colour and its inverted colour (black, explicit or Automatic, turns white;
    /// Automatic white on a dark highlight stays white), and white if that still falls short of
    /// 4.5:1 against the dimmed background (pure blue or red on a dim olive). White always
    /// reaches it: no dimmed background is lighter than #4A4A4A.
    pub fn ink_text(&self, c: Rgb, bg: Option<Rgb>) -> Rgb {
        let inked = self.ink(c);
        let Some(bg) = bg.filter(|_| self.dark) else { return inked };
        let lighter = if c.luma() > inked.luma() { c } else { inked };
        if contrast(lighter, self.ink_bg(bg)) >= 4.5 { lighter } else { Rgb::WHITE }
    }
}

// The dim assumes the default dark paper grey (`DARK_PAPER`; the app never sets another
// `dark_paper`): the backgrounds stay close to it and under #4A4A4A, so white text on them
// reaches 4.5:1. Revisit both if the paper grey becomes configurable.
/// Darkest channel of a text background on a dark page ([`RenderOptions::ink_bg`]).
const DARK_TEXT_BG_FLOOR: u8 = 0x10;
/// How far a full channel lifts a dark page's text background above the floor.
const DARK_TEXT_BG_SPAN: u8 = 0x3A;

/// WCAG 2 contrast ratio between two sRGB colours (1 to 21).
fn contrast(a: Rgb, b: Rgb) -> f32 {
    let lin = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    let lum = |c: Rgb| 0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2);
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Invert HSL lightness, keeping hue and saturation: shifting every channel by
/// `255 - max - min` maps the lightest channel to `255 - min` and the darkest to `255 - max`.
pub fn invert_lightness(c: Rgb) -> Rgb {
    let (r, g, b) = (c.0 as i32, c.1 as i32, c.2 as i32);
    let d = 255 - r.max(g).max(b) - r.min(g).min(b);
    let f = |v: i32| (v + d).clamp(0, 255) as u8;
    Rgb(f(r), f(g), f(b))
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            display: DisplayOptions::default(),
            paper: Rgb::WHITE,
            mark_color: Rgb(0x2B, 0x57, 0x9A),
            text_darkening: false,
            dark: false,
            dark_paper: DARK_PAPER,
            read_paper: None,
        }
    }
}

/// Render a whole page at `scale` pixels per point.
pub fn render_page(doc: &Document, page: &Page, scale: f32, opts: &RenderOptions) -> Rendered {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let w = ((page.w * scale).ceil() as u32).clamp(1, MAX_SIDE);
    let h = ((page.h.min(1e6) * scale).ceil() as u32).clamp(1, MAX_SIDE);
    render_region(doc, page, w, h, Affine::scale(scale as f64), opts)
}

/// Render a page into a `w`×`h` raster with `view` (page points → pixels).
pub fn render_region(doc: &Document, page: &Page, w: u32, h: u32, view: Affine, opts: &RenderOptions) -> Rendered {
    render_region_capped(doc, page, w, h, view, opts, wordcraft_geom::MAX_RASTER_PIXELS)
}

/// [`render_region`] with an explicit pixel cap (a small one in tests).
fn render_region_capped(doc: &Document, page: &Page, w: u32, h: u32, view: Affine, opts: &RenderOptions, max_pixels: u64) -> Rendered {
    let (w, h) = (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE));
    let (cw, ch) = wordcraft_geom::fit_pixels(w, h, max_pixels);
    // Over the cap the whole picture is drawn smaller, not cropped.
    let view = if (cw, ch) == (w, h) { view } else { Affine::scale_non_uniform(cw as f64 / w as f64, ch as f64 / h as f64) * view };
    let (w16, h16) = (cw as u16, ch as u16);
    let mut ctx = context_pool::CONTEXTS.take(w16, h16);
    ctx.set_transform(Affine::IDENTITY);
    ctx.set_paint(color(opts.paper_of(doc), 1.0));
    ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w16 as f64, h16 as f64));
    if opts.read_paper.is_none() {
        draw_page_picture(&mut ctx, doc, page, view);
    }
    let visible = view.inverse().transform_rect_bbox(kurbo::Rect::new(0.0, 0.0, w16 as f64, h16 as f64)).inflate(40.0, 40.0);
    if let Some(wm) = &doc.settings.watermark {
        draw_watermark(&mut ctx, view, page, doc, wm, opts);
    }
    // A dimmed layer is drawn whole, then faded once, so its overlaps don't darken.
    for (alpha, items) in page_layers(doc, page, &opts.display) {
        let faded = alpha < 1.0 && !items.is_empty();
        if faded {
            ctx.push_opacity_layer(alpha);
        }
        for it in &items {
            draw(&mut ctx, doc, it, view, &visible, opts);
        }
        if faded {
            ctx.pop_layer();
        }
    }
    ctx.flush();
    let mut pixels = vec![0u8; w16 as usize * h16 as usize * 4];
    let mut res = Resources::new();
    if let Some(pm) = vello_cpu::PixmapMut::new(w16, h16, &mut pixels) {
        ctx.render(pm, &mut res);
    }
    context_pool::CONTEXTS.give_back(ctx);
    Rendered { width: w16 as u32, height: h16 as u32, pixels }
}

/// The on-screen colour behind page point (`x`, `y`): the paper (page colour), then any
/// shading, highlight or filled shape drawn over that point, blended in drawing order. Text,
/// pictures and turned or rotated content are left out. The caret takes its colour from this
/// so it shows against whatever lies behind it (#299).
pub fn background_at(doc: &Document, page: &Page, opts: &RenderOptions, x: f32, y: f32) -> Rgb {
    let paper = opts.paper_of(doc);
    if !(x.is_finite() && y.is_finite()) {
        return paper;
    }
    let mut bg = [paper.0 as f32, paper.1 as f32, paper.2 as f32];
    for (alpha, items) in page_layers(doc, page, &opts.display) {
        // A faded layer: its own colour there, blended in at its opacity.
        let mut layer = bg;
        blend_backgrounds(&items, wordcraft_geom::Point::new(x, y), opts, &mut layer, 0);
        for (v, n) in bg.iter_mut().zip(layer) {
            *v += (n - *v) * alpha;
        }
    }
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    Rgb(c(bg[0]), c(bg[1]), c(bg[2]))
}

fn blend_backgrounds(items: &[Draw], p: wordcraft_geom::Point, opts: &RenderOptions, bg: &mut [f32; 3], depth: u32) {
    if depth > 16 {
        return;
    }
    for it in items {
        let (color, alpha) = match it {
            Draw::Fill { rect, color, alpha, .. } if rect.contains(p) => (*color, *alpha),
            // The shape's own outline (an ellipse's corners are not inside it), in its fill's
            // colour and opacity there (#1500: gradients and transparency).
            Draw::Shape { rect, kind, fill: Some(fill), corner, .. }
                if *kind != ShapeKind::Line && rect.contains(p) && {
                    let r = kurbo::Rect::new(rect.x as f64, rect.y as f64, rect.right() as f64, rect.bottom() as f64);
                    kurbo::Shape::contains(&shape_path(*kind, r, *corner), kurbo::Point::new(p.x as f64, p.y as f64))
                } =>
            {
                fill.at(p.x, p.y)
            }
            Draw::Clip { rect, items } if rect.contains(p) => {
                blend_backgrounds(items, p, opts, bg, depth + 1);
                continue;
            }
            Draw::Figure { draws, .. } => {
                blend_backgrounds(draws, p, opts, bg, depth + 1);
                continue;
            }
            _ => continue,
        };
        let a = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
        let c = opts.ink(color);
        for (v, n) in bg.iter_mut().zip([c.0, c.1, c.2]) {
            *v += (n as f32 - *v) * a;
        }
    }
}

/// One line of `text` in `face` at `size` points, black on white, in a `w`×`h` point raster at
/// `scale` pixels per point, starting `margin` points in with the capitals centred in `h`: a font
/// list entry. Not the face's ascent: a font with a tall one (Amiri Quran's is 1.8 em) put the
/// name below the row (#2570). Outlines come straight from the face, not the shared outline
/// cache, so a face read just for the preview leaves nothing behind once dropped (#848).
pub fn render_text_line(face: &wordcraft_fonts::FontFace, text: &str, size: f32, w: f32, h: f32, margin: f32, scale: f32) -> Rendered {
    let fin = |v: f32, d: f32| if v.is_finite() && v >= 0.0 { v } else { d };
    let (size, w, h, margin) = (fin(size, 12.0).min(1000.0) as f64, fin(w, 1.0), fin(h, 1.0), fin(margin, 0.0).min(1000.0) as f64);
    let scale = if scale.is_finite() && scale > 0.0 { scale.min(16.0) } else { 1.0 };
    let (pw, ph) = (((w * scale).ceil() as u32).clamp(1, MAX_SIDE), ((h * scale).ceil() as u32).clamp(1, MAX_SIDE));
    let (cw, ch) = wordcraft_geom::fit_pixels(pw, ph, wordcraft_geom::MAX_RASTER_PIXELS);
    let (w16, h16) = (cw as u16, ch as u16);
    let mut ctx = RenderContext::new_with(w16, h16, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() });
    ctx.set_transform(Affine::IDENTITY);
    ctx.set_paint(color(Rgb(255, 255, 255), 1.0));
    ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w16 as f64, h16 as f64));
    ctx.set_paint(color(Rgb(0, 0, 0), 1.0));
    let upem = if face.upem.is_finite() && face.upem > 0.0 { face.upem } else { 1000.0 };
    let k = size / upem;
    let view = Affine::scale_non_uniform(cw as f64 / pw as f64, ch as f64 / ph as f64) * Affine::scale(scale as f64);
    let cap = if face.cap_height.is_finite() { face.cap_height.clamp(upem * 0.3, upem) } else { upem * 0.7 };
    let baseline = (h as f64 + cap * k) / 2.0;
    let mut x = margin;
    let colour = face.has_color_glyphs();
    for g in wordcraft_fonts::shape(face, text, &[], |c| c) {
        let at = view * Affine::translate((x + g.x_offset as f64 * k, baseline - g.y_offset as f64 * k)) * Affine::scale(k);
        if colour && let Some(d) = color_glyph::drawable(face, g.gid, size * scale as f64, false) {
            color_glyph::draw(&mut ctx, &d, at, upem, color(Rgb(0, 0, 0), 1.0), 1.0);
            ctx.set_paint(color(Rgb(0, 0, 0), 1.0));
        } else {
            ctx.set_transform(at);
            ctx.fill_path(&face.glyph_outline(g.gid));
        }
        x += g.x_advance as f64 * k;
        if x > w as f64 {
            break;
        }
    }
    ctx.flush();
    let mut pixels = vec![0u8; w16 as usize * h16 as usize * 4];
    let mut res = Resources::new();
    if let Some(pm) = vello_cpu::PixmapMut::new(w16, h16, &mut pixels) {
        ctx.render(pm, &mut res);
    }
    Rendered { width: w16 as u32, height: h16 as u32, pixels }
}

fn draw_watermark(ctx: &mut RenderContext, view: Affine, page: &Page, doc: &Document, wm: &wordcraft_doc::Watermark, opts: &RenderOptions) {
    let r = wordcraft_fonts::word::resolve(wordcraft_layout::para::lookup_alternate(doc, &wm.font), false, false);
    let face = r.face;
    let glyphs = wordcraft_fonts::shape(&face, &wm.text, &[], |c| c);
    let upem = face.upem.max(1.0);
    let raw_w: f64 = glyphs.iter().map(|g| g.x_advance as f64).sum::<f64>() / upem;
    if raw_w <= 0.0 {
        return;
    }
    // The document page, not a Print Markup area beside it. The text stretched over its box
    // (Word's `v:textpath fitshape`): its advance across the box, its em down it with the em's foot
    // on the box's bottom; the box centred on the margins and, when diagonal, turned 45°.
    let pw = (page.w - page.markup.max(0.0)).max(1.0) as f64;
    let [bw, bh] = wm.frame(pw as f32, page.h).map(f64::from);
    let size = bh.min(2000.0);
    let k = size / upem;
    let stretch = bw / (raw_w * size);
    let descent = (wordcraft_fonts::word::line_metrics(&face).1 / upem).clamp(0.0, 1.0);
    let angle = if wm.diagonal { -std::f64::consts::FRAC_PI_4 } else { 0.0 };
    let b = page.body;
    let (cx, cy) =
        if b.w > 0.0 && b.h > 0.0 { (f64::from(b.x + b.w / 2.0), f64::from(b.y + b.h / 2.0)) } else { (pw / 2.0, page.h.min(2000.0) as f64 / 2.0) };
    let base = Affine::translate((cx, cy))
        * Affine::rotate(angle)
        * Affine::translate((-bw / 2.0, bh / 2.0 - descent * bh))
        * Affine::scale_non_uniform(stretch, 1.0);
    ctx.set_paint(color(opts.ink(wm.color), if wm.semitransparent { 0.35 } else { 0.8 }));
    let db = FontDb::global();
    let mut x = 0.0;
    for g in &glyphs {
        let o = db.outline(&face, g.gid);
        ctx.set_transform(view * base * Affine::translate((x, 0.0)) * Affine::scale(k));
        ctx.fill_path(&o);
        x += g.x_advance as f64 * k;
    }
}

/// How far, in device pixels, macOS-style font smoothing pushes each side of a glyph outline out
/// at `ppem` device pixels per em. Values from Pathfinder (MIT/Apache-2.0), which measured them
/// against macOS: 1.21% of the pixel size horizontally and 1.25 times that vertically, each
/// capped at 0.3 px, and no darkening past 72 px per em. We stroke the outline, which grows it
/// evenly, so this is the mean of the two.
pub fn stem_darkening(ppem: f64) -> f64 {
    const FACTOR: [f64; 2] = [0.0121, 0.0121 * 1.25];
    const MAX_PX: f64 = 0.3;
    const MAX_PPEM: f64 = 72.0;
    if !ppem.is_finite() || ppem <= 0.0 || ppem > MAX_PPEM {
        return 0.0;
    }
    FACTOR.iter().map(|f| (ppem * f).min(MAX_PX)).sum::<f64>() / 2.0
}

/// A wavy line from `(x0, y0)` to `(x1, y1)`; at most 20 000 units long, so a huge extent can't make it endless.
fn wave_path(x0: f64, y0: f64, x1: f64, y1: f64, amp: f64) -> BezPath {
    let mut p = BezPath::new();
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len = (dx * dx + dy * dy).sqrt();
    if !len.is_finite() || len < 1e-3 {
        return p;
    }
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let step = amp * 2.0;
    p.move_to((x0, y0));
    // Past about 1e16 a step no longer changes coordinates: stop instead of pushing curves forever.
    if x0 + ux * step == x0 && y0 + uy * step == y0 {
        return p;
    }
    let mut t = 0.0;
    let mut up = true;
    while t < len && t < 20_000.0 {
        let nt = (t + step).min(len);
        if nt <= t {
            break;
        }
        let mid_t = (t + nt) / 2.0;
        let sign = if up { -1.0 } else { 1.0 };
        let cx = x0 + ux * mid_t + nx * amp * sign;
        let cy = y0 + uy * mid_t + ny * amp * sign;
        let ex = x0 + ux * nt;
        let ey = y0 + uy * nt;
        p.quad_to((cx, cy), (ex, ey));
        t = nt;
        up = !up;
    }
    p
}

/// Most pieces one dashed stroke may be cut into; past it the stroke is drawn solid. kurbo's dash
/// iterator makes no progress along a segment when a dash is too small next to it (it never ends),
/// and every piece is more stroke to build.
const MAX_DASH_PIECES: f64 = 100_000.0;

/// `dashes` to stroke a path at most `len` long with, or none (solid) when they can't be drawn: a
/// negative or non-finite length, a pattern that adds up to nothing, or more than
/// [`MAX_DASH_PIECES`] pieces along the path.
fn safe_dashes(dashes: impl IntoIterator<Item = f64>, len: f64) -> Vec<f64> {
    let d: Vec<f64> = dashes.into_iter().take(MAX_DASH_PIECES as usize + 1).collect();
    let period: f64 = d.iter().sum();
    let ok = d.iter().all(|v| v.is_finite() && *v >= 0.0) && period > 0.0 && period.is_finite() && len.is_finite();
    // Each repeat of the pattern is up to `d.len()` pieces; a partial repeat at each end counts too.
    if ok && (len / period + 2.0) * d.len() as f64 <= MAX_DASH_PIECES { d } else { Vec::new() }
}

/// An upper bound on `path`'s length: its control polygon's (a curve is no longer than it).
fn path_len_bound(path: &BezPath) -> f64 {
    let (mut len, mut at, mut start) = (0.0, kurbo::Point::ZERO, kurbo::Point::ZERO);
    for el in path.elements() {
        let pts: &[kurbo::Point] = match el {
            kurbo::PathEl::MoveTo(p) => {
                (at, start) = (*p, *p);
                continue;
            }
            kurbo::PathEl::LineTo(p) => std::slice::from_ref(p),
            kurbo::PathEl::QuadTo(a, b) => &[*a, *b],
            kurbo::PathEl::CurveTo(a, b, c) => &[*a, *b, *c],
            kurbo::PathEl::ClosePath => std::slice::from_ref(&start),
        };
        for p in pts {
            len += at.distance(*p);
            at = *p;
        }
    }
    len
}

/// Rasterize one display item with `view` mapping its current point coordinates to pixels.
/// `visible` is a culling area in those point coordinates; nested clips intersect it and turns
/// transform it into the child's coordinates. Shape and path strokes apply their stored caps,
/// while ink uses its tool's round ends and joins.
fn draw(ctx: &mut RenderContext, doc: &Document, it: &Draw, view: Affine, visible: &kurbo::Rect, opts: &RenderOptions) {
    match it {
        Draw::Figure { draws, .. } => {
            for d in draws {
                draw(ctx, doc, d, view, visible, opts);
            }
        }
        Draw::Fill { rect, color: c, alpha, text_bg } => {
            let r = kurbo::Rect::new(rect.x as f64, rect.y as f64, rect.right() as f64, rect.bottom() as f64);
            if !r.overlaps(*visible) {
                return;
            }
            ctx.set_transform(view);
            ctx.set_paint(color(if *text_bg { opts.ink_bg(*c) } else { opts.ink(*c) }, *alpha));
            ctx.fill_rect(&r);
        }
        Draw::Line { x0, y0, x1, y1, width, color: c, stroke, alpha, text_bg } => {
            ctx.set_transform(view);
            ctx.set_paint(color(opts.ink_text(*c, *text_bg), *alpha));
            let w = *width as f64;
            // Flat ends, like the PDF export and Word: a round cap sticks half the width out past
            // every end, blobbing over the neighbouring border at a table's corners (#2892). Wavy
            // underlines keep round ends, as the PDF draws them.
            let mut st = kurbo::Stroke::new(w).with_caps(kurbo::Cap::Butt);
            let (x0, y0, x1, y1) = (*x0 as f64, *y0 as f64, *x1 as f64, *y1 as f64);
            if let Some(d) = stroke.dashes() {
                st = st.with_dashes(0.0, safe_dashes(d.iter().map(|k| f64::from(*k) * w), (x1 - x0).hypot(y1 - y0)));
            }
            let line = |ctx: &mut RenderContext, ox: f64, oy: f64| {
                let mut p = BezPath::new();
                p.move_to((x0 + ox, y0 + oy));
                p.line_to((x1 + ox, y1 + oy));
                ctx.stroke_path(&p);
            };
            if *stroke == Stroke::Wave {
                let amp = (w * 1.2).max(0.8);
                let p = wave_path(x0, y0, x1, y1, amp);
                ctx.set_stroke(kurbo::Stroke::new(w * 0.8));
                ctx.stroke_path(&p);
                return;
            }
            if *stroke == Stroke::Triple {
                // Three thin lines, spaced as a double's, across the line's direction (so a
                // vertical border's lines stand side by side).
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                let (nx, ny) = (-dy / len * w * 1.8, dx / len * w * 1.8);
                ctx.set_stroke(kurbo::Stroke::new(w * 0.6));
                for k in [-1.0, 0.0, 1.0] {
                    line(ctx, nx * k, ny * k);
                }
                return;
            }
            if *stroke == Stroke::Double {
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                let (nx, ny) = (-dy / len * w * 0.9, dx / len * w * 0.9);
                ctx.set_stroke(kurbo::Stroke::new(w * 0.6).with_caps(kurbo::Cap::Butt));
                line(ctx, nx, ny);
                line(ctx, -nx, -ny);
                return;
            }
            ctx.set_stroke(st);
            line(ctx, 0.0, 0.0);
        }
        Draw::Glyphs { face, size, glyphs, color: c, alpha, synth_bold, synth_italic, text_bg, x_scale, .. } => {
            // Invisible glyphs (WordArt letters are drawn from their outlines) carry text only.
            if *alpha <= 0.0 {
                return;
            }
            let db = FontDb::global();
            let k = *size as f64 / face.upem.max(1.0);
            ctx.set_paint(color(opts.ink_text(*c, *text_bg), *alpha));
            let skew = if *synth_italic { Affine::new([1.0, 0.0, -0.21, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
            // Scaled letters (`w:w`): each glyph stretched across from its pen position.
            let sx = if x_scale.is_finite() && *x_scale > 0.0 { f64::from(*x_scale) } else { 1.0 };
            let skew = Affine::scale_non_uniform(sx, 1.0) * skew;
            // Outline strokes, in font units: synthetic bold, then on-screen stem darkening (a
            // stroke grows the outline by half its width on each side). Darkening is skipped for
            // translucent text, where the stroke overlapping the fill would show as a darker rim.
            let px_per_pt = view.determinant().abs().sqrt();
            let darken_px = if opts.text_darkening && *alpha >= 1.0 { stem_darkening(*size as f64 * px_per_pt) } else { 0.0 };
            let darken = if darken_px > 0.0 && k * px_per_pt > 0.0 { 2.0 * darken_px / (k * px_per_pt) } else { 0.0 };
            let outline_stroke = if *synth_bold { face.upem * 0.03 } else { 0.0 } + darken;
            if outline_stroke > 0.0 {
                let join = if darken > 0.0 { kurbo::Join::Round } else { kurbo::Join::Miter };
                ctx.set_stroke(kurbo::Stroke::new(outline_stroke).with_join(join));
            }
            // Emoji and other colour glyphs (#1269) are drawn from the font's colour data.
            let colour = face.has_color_glyphs();
            let ppem = *size as f64 * px_per_pt;
            for (gid, x, y) in glyphs {
                let (gx, gy) = (*x as f64, *y as f64);
                // Written so a NaN position is skipped too.
                let near = gx >= visible.x0 - 100.0 && gx <= visible.x1 && gy >= visible.y0 && gy <= visible.y1 + 200.0;
                if !near {
                    continue;
                }
                if colour && let Some(d) = color_glyph::drawable(face, *gid, ppem, true) {
                    let at = view * Affine::translate((gx, gy)) * skew * Affine::scale(k);
                    color_glyph::draw(ctx, &d, at, face.upem, color(opts.ink(*c), 1.0), *alpha);
                    ctx.set_paint(color(opts.ink(*c), *alpha));
                    continue;
                }
                let o = db.outline(face, *gid);
                if o.elements().is_empty() {
                    continue;
                }
                ctx.set_transform(view * Affine::translate((gx, gy)) * skew * Affine::scale(k));
                ctx.fill_path(&o);
                if outline_stroke > 0.0 {
                    ctx.stroke_path(&o);
                }
            }
        }
        Draw::Mark { x, baseline, size, ch, color: mark } => {
            let face = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
            let gid = face.glyph_for(*ch);
            let face = if gid == 0 {
                match FontDb::global().fallback_for(*ch, face.id()) {
                    Some(f) => wordcraft_fonts::FaceRef::of(&f),
                    None => return,
                }
            } else {
                face
            };
            let gid = face.glyph_for(*ch);
            let o = FontDb::global().outline(&face, gid);
            let k = *size as f64 / face.upem.max(1.0);
            ctx.set_paint(color(opts.ink(mark.unwrap_or(opts.mark_color)), 0.9));
            ctx.set_transform(view * Affine::translate((*x as f64, *baseline as f64)) * Affine::scale(k));
            ctx.fill_path(&o);
        }
        Draw::MarkText { x, baseline, size, text } => {
            let base_face = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
            ctx.set_paint(color(opts.ink(opts.mark_color), 0.9));
            let mut gx = *x as f64;
            for ch in text.chars() {
                let mut face = base_face;
                if face.glyph_for(ch) == 0
                    && let Some(f) = FontDb::global().fallback_for(ch, face.id())
                {
                    face = wordcraft_fonts::FaceRef::of(&f);
                }
                let gid = face.glyph_for(ch);
                let k = *size as f64 / face.upem.max(1.0);
                let o = FontDb::global().outline(&face, gid);
                ctx.set_transform(view * Affine::translate((gx, *baseline as f64)) * Affine::scale(k));
                ctx.fill_path(&o);
                gx += face.advance(gid) * k;
            }
        }
        Draw::Image { rect, media, crop, alpha, .. } => {
            let r = kurbo::Rect::new(rect.x as f64, rect.y as f64, rect.right() as f64, rect.bottom() as f64);
            if !r.overlaps(*visible) {
                return;
            }
            if let Some(v) = vector_for(doc, media) {
                draw_vector(ctx, &v, &r, crop, *alpha, view);
                return;
            }
            let Some(pm) = pixmap_for(doc, media) else {
                ctx.set_transform(view);
                ctx.set_paint(color(opts.ink(Rgb(0xD0, 0xD0, 0xD0)), 1.0));
                ctx.fill_rect(&r);
                return;
            };
            let (pw, ph) = (pm.width().max(1) as f64, pm.height().max(1) as f64);
            let [cl, ct, vis_w, vis_h] = wordcraft_metafile::crop_window(*crop);
            let sx = r.width() / (pw * vis_w);
            let sy = r.height() / (ph * vis_h);
            ctx.set_transform(view);
            // vello_cpu panics on image paints with sampler alpha != 1, so fade through a layer.
            let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
            let faded = alpha < 1.0;
            if faded {
                ctx.push_opacity_layer(alpha);
            }
            ctx.set_paint(vello_cpu::Image {
                image: vello_cpu::ImageSource::Pixmap(pm),
                sampler: peniko::ImageSampler::default().with_quality(peniko::ImageQuality::Medium),
            });
            ctx.set_paint_transform(Affine::translate((r.x0 - cl * pw * sx, r.y0 - ct * ph * sy)) * Affine::scale_non_uniform(sx, sy));
            ctx.fill_rect(&r);
            ctx.reset_paint_transform();
            if faded {
                ctx.pop_layer();
            }
        }
        Draw::Path { segs, fill, stroke, stroke_width, dash, cap } => {
            let path = wordcraft_layout::display::seg_path(segs);
            if path.elements().is_empty() {
                return;
            }
            ctx.set_transform(view);
            if let Some(f) = fill {
                set_paint(ctx, f, opts);
                ctx.fill_path(&path);
            }
            if let Some(s) = stroke {
                // Width 0 (or none) is a hairline.
                let w = if stroke_width.is_finite() && *stroke_width > 0.0 { stroke_width.clamp(0.25, 200.0) } else { 0.75 };
                ctx.set_paint(color(opts.ink(*s), 1.0));
                ctx.set_stroke(
                    kurbo::Stroke::new(w as f64)
                        .with_caps(screen_cap(*cap))
                        .with_dashes(0.0, safe_dashes(dash.iter().map(|v| f64::from(*v)), path_len_bound(&path))),
                );
                ctx.stroke_path(&path);
            }
        }
        Draw::Art { path, fx, color: c, alpha, text_bg } => {
            let Some(b) = wordcraft_layout::wordart::path_box(path) else { return };
            if !b.inflate(200.0, 200.0).overlaps(*visible) {
                return;
            }
            ctx.set_transform(view);
            for l in wordcraft_layout::wordart::art_layers(path, fx, *c, *alpha) {
                // The letters read against their highlight like plain text (#1252); shadows don't.
                let ink = |c: Rgb| if l.letters { opts.ink_text(c, *text_bg) } else { opts.ink(c) };
                set_paint_inked(ctx, &l.paint, ink);
                match l.stroke {
                    Some(w) => {
                        ctx.set_stroke(kurbo::Stroke::new(f64::from(w.clamp(0.1, 400.0))).with_join(kurbo::Join::Round));
                        ctx.stroke_path(&l.path);
                    }
                    None => ctx.fill_path(&l.path),
                }
            }
        }
        Draw::Ink { pts, color: c, width, alpha } => {
            let Some(path) = ink_path(pts) else { return };
            if !path.bounding_box().inflate(*width as f64, *width as f64).overlaps(*visible) {
                return;
            }
            ctx.set_transform(view);
            ctx.set_paint(color(opts.ink(*c), *alpha));
            let w = if width.is_finite() { width.clamp(0.25, 200.0) } else { 1.0 };
            ctx.set_stroke(kurbo::Stroke::new(w as f64).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round));
            ctx.stroke_path(&path);
        }
        Draw::Shape { rect, kind, fill, stroke, stroke_width, effects, corner, dash, cap } => {
            let r = kurbo::Rect::new(rect.x as f64, rect.y as f64, rect.right() as f64, rect.bottom() as f64);
            let path = shape_path(*kind, r, *corner);
            ctx.set_transform(view);
            let fx = effects.sanitized();
            let can_fill = *kind != ShapeKind::Line && fill.is_some();
            if !fx.is_empty() && (can_fill || stroke.is_some()) {
                let sw = if stroke_width.is_finite() { stroke_width.clamp(0.25, 200.0) as f64 } else { 0.75 };
                // The shape's silhouette grown by `g` points and moved by `d`, in the current paint.
                let silhouette = |ctx: &mut RenderContext, g: f32, d: kurbo::Vec2| {
                    let g = g as f64;
                    if can_fill {
                        let grow = g + if stroke.is_some() { sw / 2.0 } else { 0.0 };
                        let rr = r.inflate(grow, grow) + d;
                        if rr.width() > 0.0 && rr.height() > 0.0 {
                            ctx.fill_path(&shape_path(*kind, rr, *corner));
                        }
                    } else if sw + 2.0 * g > 0.05 {
                        ctx.set_stroke(kurbo::Stroke::new(sw + 2.0 * g));
                        ctx.stroke_path(&shape_path(*kind, r + d, *corner));
                    }
                };
                if let Some(sh) = fx.shadow {
                    let (dx, dy) = sh.offset();
                    for (g, a) in wordcraft_doc::effects::bands(-sh.blur / 2.0, sh.blur / 2.0, sh.opacity()) {
                        ctx.set_paint(color(opts.ink(sh.color), a));
                        silhouette(ctx, g, kurbo::Vec2::new(dx as f64, dy as f64));
                    }
                }
                if let Some(gl) = fx.glow {
                    for (g, a) in wordcraft_doc::effects::bands(0.0, gl.size, gl.opacity()) {
                        ctx.set_paint(color(opts.ink(gl.color), a));
                        silhouette(ctx, g, kurbo::Vec2::ZERO);
                    }
                }
            }
            // Soft edges: the fill fades out toward the outline (which fades with it).
            if let (Some(rad), Some(f), true) = (fx.soft_edge, fill, can_fill) {
                let rad = rad.min(rect.w.min(rect.h) / 2.0).max(0.0);
                for (g, a) in wordcraft_doc::effects::bands(-rad, 0.0, 1.0) {
                    let rr = r.inflate(g as f64, g as f64);
                    if rr.width() > 0.0 && rr.height() > 0.0 {
                        set_paint(ctx, &f.faded(a), opts);
                        ctx.fill_path(&shape_path(*kind, rr, *corner));
                    }
                }
                return;
            }
            if let Some(f) = fill {
                set_paint(ctx, f, opts);
                ctx.fill_path(&path);
            }
            if let Some(s) = stroke {
                ctx.set_paint(color(opts.ink(*s), 1.0));
                ctx.set_stroke(
                    kurbo::Stroke::new(if stroke_width.is_nan() { 0.25 } else { stroke_width.clamp(0.25, 200.0) } as f64)
                        .with_caps(screen_cap(*cap))
                        .with_dashes(0.0, safe_dashes(dash.iter().map(|v| f64::from(*v)), path_len_bound(&path))),
                );
                ctx.stroke_path(&path);
            }
        }
        Draw::Clip { rect, items } => {
            let r = kurbo::Rect::new(rect.x as f64, rect.y as f64, rect.right() as f64, rect.bottom() as f64);
            if !r.is_finite() || !r.overlaps(*visible) {
                return;
            }
            ctx.set_transform(view);
            ctx.push_clip_layer(&r.to_path(0.1));
            let local = r.intersect(*visible);
            for it in items {
                draw(ctx, doc, it, view, &local, opts);
            }
            ctx.pop_layer();
        }
        Draw::Turned { x, y, turn, items } => {
            let m = Affine::new(Draw::turn_matrix(*turn, *x, *y).map(f64::from));
            let local = m.inverse().transform_rect_bbox(*visible);
            for it in items {
                draw(ctx, doc, it, view * m, &local, opts);
            }
        }
        Draw::Rotated { cx, cy, spin, items } => {
            let m = Affine::new(spin.matrix(*cx, *cy).map(f64::from));
            let local = m.inverse().transform_rect_bbox(*visible);
            for it in items {
                draw(ctx, doc, it, view * m, &local, opts);
            }
        }
    }
}

/// The line through an ink stroke's points (finite ones only); a single point is a dot.
fn ink_path(pts: &[(f32, f32)]) -> Option<BezPath> {
    let mut p = BezPath::new();
    let mut last = None;
    for &(x, y) in pts.iter().filter(|(x, y)| x.is_finite() && y.is_finite() && x.abs() < 1e6 && y.abs() < 1e6) {
        let pt = (x as f64, y as f64);
        if last.is_none() {
            p.move_to(pt);
        } else {
            p.line_to(pt);
        }
        last = Some(pt);
    }
    // Only sanitized points reach the path: a tap is drawn at the one usable point.
    let only = last?;
    if p.elements().len() == 1 {
        // A tap: a zero-length line, which round caps draw as a dot.
        p.line_to(only);
    }
    Some(p)
}

/// Map the document cap to the raster backend without changing ink-tool stroke styles.
fn screen_cap(cap: wordcraft_doc::connector::LineCap) -> kurbo::Cap {
    match cap {
        wordcraft_doc::connector::LineCap::Flat => kurbo::Cap::Butt,
        wordcraft_doc::connector::LineCap::Square => kurbo::Cap::Square,
        wordcraft_doc::connector::LineCap::Round => kurbo::Cap::Round,
    }
}

/// Outline of a basic shape in a rectangle; `corner` is a rounded rectangle's
/// ([`wordcraft_doc::connector::ShapeExtra::corner`]).
pub fn shape_path(kind: ShapeKind, r: kurbo::Rect, corner: Option<u32>) -> BezPath {
    let (cx, cy) = (r.center().x, r.center().y);
    let poly = |pts: &[(f64, f64)]| {
        let mut p = BezPath::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                p.move_to((*x, *y));
            } else {
                p.line_to((*x, *y));
            }
        }
        p.close_path();
        p
    };
    match kind {
        // A freeform is drawn from its own paths; without them, its frame.
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::Freeform => r.to_path(0.1),
        ShapeKind::RoundedRectangle => {
            let radius = wordcraft_doc::connector::corner_radius(corner, r.width() as f32, r.height() as f32);
            kurbo::RoundedRect::from_rect(r, f64::from(radius)).to_path(0.1)
        }
        ShapeKind::Ellipse => kurbo::Ellipse::from_rect(r).to_path(0.1),
        ShapeKind::FlowchartOr => {
            let mut p = kurbo::Ellipse::from_rect(r).to_path(0.1);
            p.move_to((r.x0, cy));
            p.line_to((r.x1, cy));
            p.move_to((cx, r.y0));
            p.line_to((cx, r.y1));
            p
        }
        ShapeKind::Triangle => poly(&[(cx, r.y0), (r.x1, r.y1), (r.x0, r.y1)]),
        ShapeKind::RightTriangle => poly(&[(r.x0, r.y0), (r.x1, r.y1), (r.x0, r.y1)]),
        ShapeKind::Diamond => poly(&[(cx, r.y0), (r.x1, cy), (cx, r.y1), (r.x0, cy)]),
        // DrawingML's `line`: top-left to bottom-right (flips give the other directions).
        ShapeKind::Line => {
            let mut p = BezPath::new();
            p.move_to((r.x0, r.y0));
            p.line_to((r.x1, r.y1));
            p
        }
        // Connectors are drawn as paths (`Draw::Path`); this is their frame's diagonal.
        ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => {
            let mut p = BezPath::new();
            p.move_to((r.x0, r.y0));
            p.line_to((r.x1, r.y1));
            p
        }
        ShapeKind::Arrow | ShapeKind::ArrowLeft | ShapeKind::ArrowUp | ShapeKind::ArrowDown => {
            let (w, h) = (r.width(), r.height());
            let pts: Vec<(f64, f64)> = wordcraft_doc::wrap::arrow_outline(kind, w as f32, h as f32)
                .iter()
                .map(|&(u, v)| (r.x0 + f64::from(u) * w, r.y0 + f64::from(v) * h))
                .collect();
            poly(&pts)
        }
        ShapeKind::Star => {
            let mut pts = Vec::new();
            for i in 0..10 {
                let a = std::f64::consts::PI * (i as f64) / 5.0 - std::f64::consts::FRAC_PI_2;
                let rad = if i % 2 == 0 { 1.0 } else { 0.4 };
                pts.push((cx + a.cos() * r.width() / 2.0 * rad, cy + a.sin() * r.height() / 2.0 * rad));
            }
            poly(&pts)
        }
        ShapeKind::Heart => {
            let (w, h) = (r.width(), r.height());
            let mut p = BezPath::new();
            p.move_to((cx, r.y0 + h * 0.3));
            p.curve_to((cx, r.y0), (r.x0, r.y0), (r.x0, r.y0 + h * 0.3));
            p.curve_to((r.x0, r.y0 + h * 0.6), (cx - w * 0.1, r.y0 + h * 0.75), (cx, r.y1));
            p.curve_to((cx + w * 0.1, r.y0 + h * 0.75), (r.x1, r.y0 + h * 0.6), (r.x1, r.y0 + h * 0.3));
            p.curve_to((r.x1, r.y0), (cx, r.y0), (cx, r.y0 + h * 0.3));
            p.close_path();
            p
        }
    }
}

/// Render the page area (`x`, `y`, `w`×`h` points) at `scale` px/pt — for previews and thumbnails.
pub fn render_area(doc: &Document, page: &Page, x: f32, y: f32, w: f32, h: f32, scale: f32, opts: &RenderOptions) -> Rendered {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let pw = ((w * scale).ceil() as u32).clamp(1, MAX_SIDE);
    let ph = ((h * scale).ceil() as u32).clamp(1, MAX_SIDE);
    let view = Affine::scale(scale as f64) * Affine::translate((-x as f64, -y as f64));
    render_region(doc, page, pw, ph, view, opts)
}

/// Draw `items` (page points) alone, on nothing, over the page area (`x`, `y`, `w`×`h` points)
/// at `scale` px/pt: no paper, no watermark. A dragged object's preview.
pub fn render_items(doc: &Document, items: &[Draw], x: f32, y: f32, w: f32, h: f32, scale: f32, opts: &RenderOptions) -> Rendered {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let (pw, ph) = (((w * scale).ceil() as u32).clamp(1, MAX_SIDE), ((h * scale).ceil() as u32).clamp(1, MAX_SIDE));
    let (cw, ch) = wordcraft_geom::fit_pixels(pw, ph, wordcraft_geom::MAX_RASTER_PIXELS);
    let (w16, h16) = (cw as u16, ch as u16);
    let view = Affine::scale_non_uniform(cw as f64 / pw as f64, ch as f64 / ph as f64)
        * Affine::scale(scale as f64)
        * Affine::translate((-x as f64, -y as f64));
    let mut ctx = RenderContext::new_with(w16, h16, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() });
    let visible = view.inverse().transform_rect_bbox(kurbo::Rect::new(0.0, 0.0, w16 as f64, h16 as f64)).inflate(40.0, 40.0);
    for it in items {
        draw(&mut ctx, doc, it, view, &visible, opts);
    }
    ctx.flush();
    let mut pixels = vec![0u8; w16 as usize * h16 as usize * 4];
    let mut res = Resources::new();
    if let Some(pm) = vello_cpu::PixmapMut::new(w16, h16, &mut pixels) {
        ctx.render(pm, &mut res);
    }
    Rendered { width: w16 as u32, height: h16 as u32, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_layout::display::page_display;
    use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

    /// The first and last pixel rows with ink in a black-on-white [`render_text_line`] raster.
    fn ink_rows(img: &Rendered) -> Option<(usize, usize)> {
        let w = img.width as usize * 4;
        let rows: Vec<usize> = img.pixels.chunks(w).enumerate().filter(|(_, r)| r.chunks(4).any(|p| p[0] < 128)).map(|(i, _)| i).collect();
        Some((*rows.first()?, *rows.last()?))
    }

    /// #2570: a font with a very tall ascent (Amiri Quran's is 1.8 em, for stacked marks) put its
    /// font list entry's baseline below the row, so the name was cut in half, and every tall-ascent
    /// font sat lower than its neighbours. The entry's capitals are now centred in the row,
    /// whatever the ascent.
    #[test]
    fn a_tall_ascent_font_list_entry_sits_where_the_others_do() {
        // Source Sans 3 renamed (same length) with its ascent set to Amiri Quran's 1815/1000 em.
        let mut data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        for (from, to) in [(utf16("Source Sans 3"), utf16("Talled Sans 3")), (b"Source Sans 3".to_vec(), b"Talled Sans 3".to_vec())] {
            let mut i = 0;
            while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
                data[i + at..i + at + to.len()].copy_from_slice(&to);
                i += at + to.len();
            }
        }
        let table = |data: &[u8], tag: &[u8; 4]| {
            let n = u16::from_be_bytes([data[4], data[5]]) as usize;
            (0..n)
                .map(|k| 12 + k * 16)
                .find(|&r| &data[r..r + 4] == tag)
                .map(|r| u32::from_be_bytes(data[r + 8..r + 12].try_into().unwrap()) as usize)
                .unwrap()
        };
        let upem = u16::from_be_bytes(data[table(&data, b"head") + 18..][..2].try_into().unwrap()) as f64;
        let tall = ((upem * 1.815) as i16).to_be_bytes();
        let (hhea, os2) = (table(&data, b"hhea"), table(&data, b"OS/2"));
        for at in [hhea + 4, os2 + 68, os2 + 74] {
            data[at..at + 2].copy_from_slice(&tall);
        }
        assert!(wordcraft_fonts::FontDb::global().add_font(data) > 0);
        let db = wordcraft_fonts::FontDb::global();
        let tall_face = db.face("Talled Sans 3", "Regular");
        let normal = db.face(wordcraft_fonts::FALLBACK_FAMILY, "Regular");
        assert_eq!(tall_face.family, "Talled Sans 3");
        assert!(tall_face.ascent > normal.ascent * 1.5, "patched ascent {} vs {}", tall_face.ascent, normal.ascent);
        // The font list's entry: 13 pt text in a 244×22 pt row, 2 pt in.
        let entry = |f: &wordcraft_fonts::FontFace| ink_rows(&render_text_line(f, "Amiri Quran", 13.0, 244.0, 22.0, 2.0, 2.0));
        let (top, bottom) = entry(&normal).unwrap();
        let (tall_top, tall_bottom) = entry(&tall_face).unwrap();
        assert_eq!((tall_top, tall_bottom), (top, bottom), "same glyphs, same place in the row");
        assert!(top > 0 && bottom < 43, "the name isn't clipped: ink rows {top}..={bottom} of 44");
        // Centred: as much room above the capitals as below the descender, within a few pixels.
        let (above, below) = (top as i32, 43 - bottom as i32);
        assert!((above - below).abs() <= 6, "rows above {above}, below {below}");
    }

    /// #299: the caret's background is the page colour, a highlight, or the dark page.
    #[test]
    fn background_at_caret_follows_page_colour_highlight_and_dark_mode() {
        use wordcraft_doc::Pos;
        use wordcraft_doc::props::Highlight;
        let mut d = Document::from_text("Hello WordCraft");
        let mid = |d: &Document, off: usize| {
            let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
            let c = l.caret_on(&Pos::body(0, off), 0).unwrap();
            let page = l.pages[c.page].clone();
            (page, c.x, c.top + c.height / 2.0)
        };
        let light = RenderOptions::default();
        let dark = RenderOptions { dark: true, ..Default::default() };
        let (page, x, y) = mid(&d, 2);
        assert_eq!(background_at(&d, &page, &light, x, y), Rgb::WHITE);
        assert_eq!(background_at(&d, &page, &dark, x, y), Rgb(DARK_PAPER, DARK_PAPER, DARK_PAPER));
        assert_eq!(background_at(&d, &page, &light, f32::NAN, f32::INFINITY), Rgb::WHITE, "junk points fall back to the paper");
        d.settings.page_color = Some(Rgb(0x10, 0x10, 0x40));
        assert_eq!(background_at(&d, &page, &light, x, y), Rgb(0x10, 0x10, 0x40));
        d.settings.page_color = None;
        d.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| c.highlight = Some(Highlight::Black)).unwrap();
        let (page, x, y) = mid(&d, 2);
        assert_eq!(background_at(&d, &page, &light, x, y), Rgb::BLACK, "inside the highlight");
        let (page, x, y) = mid(&d, 9);
        assert_eq!(background_at(&d, &page, &light, x, y), Rgb::WHITE, "past the highlight");
    }

    /// #299: a filled shape colours the caret's background only inside its outline: an
    /// ellipse's corner shows what lies beneath, its centre the fill.
    #[test]
    fn background_follows_a_shapes_outline_not_its_box() {
        let opts = RenderOptions::default();
        let ellipse = Draw::Shape {
            rect: wordcraft_geom::Rect::new(100.0, 100.0, 200.0, 100.0),
            kind: ShapeKind::Ellipse,
            fill: Some(wordcraft_layout::display::Paint::solid(Rgb::BLACK)),
            stroke: None,
            stroke_width: 0.0,
            effects: Default::default(),
            corner: None,
            dash: Vec::new(),
            cap: Default::default(),
        };
        let at = |x: f32, y: f32| {
            let mut bg = [255.0; 3];
            blend_backgrounds(std::slice::from_ref(&ellipse), wordcraft_geom::Point::new(x, y), &opts, &mut bg, 0);
            bg
        };
        assert_eq!(at(200.0, 150.0), [0.0; 3], "the centre is the fill");
        assert_eq!(at(103.0, 103.0), [255.0; 3], "the box's corner is outside the ellipse");
        assert_eq!(at(350.0, 150.0), [255.0; 3], "outside the box");
    }

    #[test]
    fn renders_text_dark_pixels() {
        let d = Document::from_text("Hello WordCraft");
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
        assert_eq!((img.width, img.height), (612, 792));
        // Somewhere in the first text line there are dark pixels; the margins are white.
        let dark = (72..300).flat_map(|x| (72..100).map(move |y| (x, y))).filter(|(x, y)| img.pixel(*x, *y)[0] < 128).count();
        assert!(dark > 30, "dark {dark}");
        assert_eq!(img.pixel(5, 5), [255, 255, 255, 255]);
        assert!(!img.to_png().is_empty());
    }

    /// The watermark's font, missing: drawn in the font table's alternate when it is installed,
    /// else in the font's own substitute, as before; never in the roman fallback (Cambria).
    #[test]
    fn a_watermark_in_a_missing_roman_font_takes_only_the_font_tables_alternate() {
        const MISSING: &str = "No Such Roman Watermark Font";
        let pixels = |font: &str, roman: bool, alt: Option<&str>| {
            let mut d = Document::new();
            d.settings.watermark = Some(wordcraft_doc::Watermark { text: "COPY".into(), font: font.into(), diagonal: false, ..Default::default() });
            if roman {
                d.settings.roman_fonts = vec![font.into()];
            }
            d.settings.font_alt_names = alt.map(|a| vec![(font.to_string(), a.to_string())]).unwrap_or_default();
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            render_page(&d, &l.pages[0], 0.5, &RenderOptions::default()).pixels
        };
        let missing = pixels(MISSING, false, None);
        assert!(pixels("Cambria", false, None) != missing, "Cambria draws the watermark differently");
        assert!(pixels(MISSING, true, None) == missing, "roman, no alternate: the font's own substitute");
        assert!(pixels(MISSING, true, Some("Also Not Installed")) == missing, "roman, a missing alternate: the font's own substitute");
        assert!(
            pixels(MISSING, true, Some("JetBrains Mono")) == pixels("JetBrains Mono", false, None),
            "roman, an installed alternate: the alternate"
        );
    }

    /// Table Layout › Text Direction (#226): a turned cell's text is painted running down the
    /// cell, not across it.
    #[test]
    fn turned_cell_text_is_painted_turned() {
        use wordcraft_doc::props::TextDirection;
        let ink = |dir: TextDirection| {
            let mut d = Document::from_text("before\nafter");
            let mut t = wordcraft_doc::Table::new(1, 2, 400.0);
            t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("WWWWWWWWWWWW", Default::default()))];
            t.rows[0].cells[0].props.text_direction = dir;
            d.insert_block(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
            // Dark pixels inside the first cell (x 72..272, clear of its borders), well below
            // the first text line.
            (80..264).flat_map(|x| (140..190).map(move |y| (x, y))).filter(|(x, y)| img.pixel(*x, *y)[0] < 128).count()
        };
        assert_eq!(ink(TextDirection::Horizontal), 0, "horizontal text stays in its line");
        assert!(ink(TextDirection::Down) > 30, "down");
        assert!(ink(TextDirection::Up) > 30, "up");
    }

    /// Shape Effects (#275): an outer shadow paints dark pixels beside the shape on the side it is
    /// cast to, and none on the other; a glow surrounds it.
    #[test]
    fn shape_shadow_is_cast_offset_from_the_shape() {
        use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
        use wordcraft_doc::para::InlineObject;
        let draw = |effects: ShapeEffects| {
            let mut d = Document::from_text("");
            let shape = InlineObject::Shape {
                kind: ShapeKind::Rectangle,
                w: 100.0,
                h: 60.0,
                fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(0, 0, 255))),
                stroke: None,
                stroke_width: 0.0,
                float: Default::default(),
                story: None,
                freeform: None,
                effects,
                extra: Default::default(),
            };
            let at = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
            d.insert_object(&at, shape, &Default::default()).unwrap();
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            render_page(&d, &l.pages[0], 1.0, &RenderOptions::default())
        };
        let plain = draw(ShapeEffects::default());
        let blue = |img: &Rendered| {
            let px: Vec<(u32, u32)> = (0..612)
                .flat_map(|x| (0..400).map(move |y| (x, y)))
                .filter(|(x, y)| matches!(img.pixel(*x, *y), [r, _, b, _] if r < 40 && b > 200))
                .collect();
            let (x0, x1) = (px.iter().map(|p| p.0).min().unwrap(), px.iter().map(|p| p.0).max().unwrap());
            let (y0, y1) = (px.iter().map(|p| p.1).min().unwrap(), px.iter().map(|p| p.1).max().unwrap());
            (x0, y0, x1, y1)
        };
        let (x0, y0, x1, y1) = blue(&plain);
        assert!(x1 - x0 > 90 && y1 - y0 > 50, "{:?}", (x0, y0, x1, y1));
        // Grey (not white, not blue) pixels in a strip just outside an edge.
        let grey = |img: &Rendered, xs: std::ops::Range<u32>, ys: std::ops::Range<u32>| {
            xs.flat_map(|x| ys.clone().map(move |y| (x, y)))
                .filter(|(x, y)| matches!(img.pixel(*x, *y), [r, g, b, _] if r < 200 && r == g && g == b))
                .count()
        };
        let right = |img: &Rendered| grey(img, x1 + 2..x1 + 6, y0 + 10..y1);
        let left = |img: &Rendered| grey(img, x0.saturating_sub(6)..x0.saturating_sub(2), y0 + 10..y1);
        assert_eq!((right(&plain), left(&plain)), (0, 0));
        let shadow = Shadow { color: Rgb::BLACK, transparency: 30.0, blur: 2.0, distance: 8.0, angle: 0.0, rot_with_shape: false };
        let img = draw(ShapeEffects { shadow: Some(shadow), ..Default::default() });
        assert_eq!(blue(&img), (x0, y0, x1, y1), "the shape itself is unchanged");
        assert!(right(&img) > 100, "shadow to the right: {}", right(&img));
        assert_eq!(left(&img), 0, "nothing on the left");
        let img = draw(ShapeEffects { glow: Some(Glow { color: Rgb(255, 0, 0), size: 6.0, transparency: 0.0 }), ..Default::default() });
        let reddish = |x: u32, y: u32| matches!(img.pixel(x, y), [r, g, _, _] if r > 200 && g < 200);
        assert!(reddish(x1 + 2, (y0 + y1) / 2) && reddish(x0 - 2, (y0 + y1) / 2), "glow on both sides");
    }

    /// #1500: gradient fills shade across the shape, circular ones from the centre out, and a
    /// transparent fill lets what is behind show through.
    #[test]
    fn shape_gradients_and_transparency_are_drawn() {
        use std::sync::Arc;
        use wordcraft_doc::fill::{Fill, Gradient, GradientFlip, GradientKind, PathShape};
        use wordcraft_doc::para::{Float, InlineObject, Wrap};
        use wordcraft_doc::wordart::GradStop;
        let shape = |fill: Fill, x: f32| InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 100.0,
            h: 100.0,
            fill: Some(fill),
            stroke: None,
            stroke_width: 0.0,
            float: Float { wrap: Wrap::InFrontOfText, x, y: 0.0, ..Default::default() },
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        let stops = vec![GradStop { pos: 0.0, color: Rgb::BLACK, transparency: 0.0 }, GradStop { pos: 100.0, color: Rgb::WHITE, transparency: 0.0 }];
        let grad =
            |kind| Fill::Gradient(Arc::new(Gradient { stops: stops.clone(), kind, tile: None, rot_with_shape: true, flip: GradientFlip::None }));
        let mut d = Document::from_text("");
        let at = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
        // Left to right: top-to-bottom linear, circular, navy under 30%-opaque white.
        let objs = [
            shape(grad(GradientKind::Linear { angle: 90.0, scaled: false }), 0.0),
            shape(grad(GradientKind::Path { shape: PathShape::Circle, focus: [50.0; 4] }), 150.0),
            shape(Fill::solid(Rgb(0, 0, 0x80)), 300.0),
            shape(Fill::Solid { color: Rgb::WHITE, transparency: 70.0 }, 300.0),
        ];
        for o in objs.into_iter().rev() {
            d.insert_object(&at, o, &Default::default()).unwrap();
        }
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
        // Each shape's frame (the dark corner of the linear one is its top-left pixel).
        let dark = |x0: u32, x1: u32| -> Vec<(u32, u32)> {
            (x0..x1).flat_map(|x| (0..400).map(move |y| (x, y))).filter(|(x, y)| img.pixel(*x, *y)[0] < 40).collect()
        };
        let lin = dark(60, 200);
        let (lx, ly) = (lin.iter().map(|p| p.0).min().unwrap(), lin.iter().map(|p| p.1).min().unwrap());
        let luma = |x: u32, y: u32| img.pixel(x, y)[0];
        assert!(
            luma(lx + 50, ly + 5) < luma(lx + 50, ly + 50) && luma(lx + 50, ly + 50) < luma(lx + 50, ly + 95),
            "dark at the top, light at the bottom"
        );
        let (cx, cy) = (lx + 150 + 50, ly + 50);
        assert!(luma(cx, cy) < 30 && luma(cx - 48, cy - 48) > 200, "black at the centre, white at the corners");
        let [r, _, b, _] = img.pixel(lx + 300 + 50, cy);
        assert!((150..230).contains(&b) && (60..120).contains(&r), "white over navy blends: {r} {b}");
    }

    #[test]
    fn stem_darkening_follows_the_macos_curve() {
        assert_eq!(stem_darkening(0.0), 0.0);
        assert_eq!(stem_darkening(f64::NAN), 0.0);
        assert_eq!(stem_darkening(-5.0), 0.0);
        // Small text grows in proportion to its pixel size...
        assert!((stem_darkening(10.0) - 10.0 * 0.0121 * 2.25 / 2.0).abs() < 1e-9);
        // ...body text on a Retina screen hits the 0.3 px cap...
        assert_eq!(stem_darkening(30.0), 0.3);
        assert_eq!(stem_darkening(72.0), 0.3);
        // ...and display sizes aren't darkened at all.
        assert_eq!(stem_darkening(72.5), 0.0);
    }

    /// Ink in a raster: the sum of how far each pixel is from white.
    fn ink(img: &Rendered) -> u64 {
        img.pixels.as_chunks::<4>().0.iter().map(|p| 255 - p[0] as u64).sum()
    }

    #[test]
    fn text_darkening_adds_ink_on_screen_only() {
        let d = Document::from_text("Marketing analytics lead with twelve years of measurement work.");
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let plain = render_page(&d, &l.pages[0], 2.0, &RenderOptions::default());
        let dark = render_page(&d, &l.pages[0], 2.0, &RenderOptions { text_darkening: true, ..Default::default() });
        let (a, b) = (ink(&plain), ink(&dark));
        // About 0.3 px more on every edge of an 11 pt line at 2x: clearly more ink, not bold.
        assert!(b as f64 > a as f64 * 1.08 && (b as f64) < a as f64 * 1.6, "plain {a}, darkened {b}");
        // Off by default: exports and thumbnails are untouched.
        assert!(!RenderOptions::default().text_darkening);
    }

    #[test]
    fn page_picture_is_painted_behind_the_text() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 200, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut d = Document::new();
        d.media.insert("bg.png".into(), Arc::new(png));
        for kind in ["tile", "frame"] {
            d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: "bg.png".into(), fill_type: kind.into(), title: String::new() });
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
            let i = ((300 * img.width + 300) * 4) as usize;
            assert_eq!(&img.pixels[i..i + 3], &[10, 200, 30], "{kind}");
        }
        // An undecodable picture leaves the paper alone.
        d.media.insert("bg.png".into(), Arc::new(b"junk".to_vec()));
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
        assert_eq!(&img.pixels[0..3], &[255, 255, 255]);
    }

    /// Read Mode's Page Color: the reading paper replaces the document's page color and page
    /// picture on screen, in the raster and under the caret; the text keeps its color on Sepia.
    #[test]
    fn read_paper_replaces_the_page_background() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 200, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut d = Document::from_text("Hello WordCraft");
        d.media.insert("bg.png".into(), Arc::new(png));
        d.settings.page_color = Some(Rgb(0x10, 0x10, 0x40));
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let page = &l.pages[0];
        let at = |img: &Rendered, x: u32, y: u32| {
            let i = ((y * img.width + x) * 4) as usize;
            [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2]]
        };
        let sepia = RenderOptions { read_paper: Some(SEPIA_PAPER), ..Default::default() };
        let white = RenderOptions { read_paper: Some(Rgb::WHITE), ..Default::default() };
        assert_eq!(at(&render_page(&d, page, 1.0, &RenderOptions::default()), 5, 5), [0x10, 0x10, 0x40], "the page color");
        assert_eq!(at(&render_page(&d, page, 1.0, &sepia), 5, 5), [0xE2, 0xD9, 0xD0]);
        assert_eq!(at(&render_page(&d, page, 1.0, &white), 5, 5), [255, 255, 255]);
        assert_eq!(background_at(&d, page, &sepia, 5.0, 5.0), SEPIA_PAPER);
        assert_eq!(background_at(&d, page, &white, 5.0, 5.0), Rgb::WHITE);
        let inverse = RenderOptions { read_paper: Some(Rgb::WHITE), dark: true, ..Default::default() };
        assert_eq!(background_at(&d, page, &inverse, 5.0, 5.0), Rgb(DARK_PAPER, DARK_PAPER, DARK_PAPER));
        d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: "bg.png".into(), fill_type: "tile".into(), title: String::new() });
        assert_eq!(at(&render_page(&d, page, 1.0, &RenderOptions::default()), 300, 300), [10, 200, 30], "the page picture");
        assert_eq!(at(&render_page(&d, page, 1.0, &sepia), 300, 300), [0xE2, 0xD9, 0xD0]);
        // The text is drawn in its own color on sepia paper: its darkest pixel is within 8 levels of its value on white.
        d.settings.page_picture = None;
        d.settings.page_color = None;
        let darkest = |img: &Rendered| img.pixels.chunks(4).map(|p| p[0].min(p[1]).min(p[2])).min().unwrap_or(255);
        let plain = render_page(&d, page, 2.0, &RenderOptions::default());
        let on_sepia = render_page(&d, page, 2.0, &sepia);
        assert!(darkest(&plain) < 0x40 && darkest(&on_sepia) <= darkest(&plain) + 8, "{} {}", darkest(&plain), darkest(&on_sepia));
    }

    /// A PNG signature, an IHDR declaring `w`×`h` 8-bit RGBA and a short IDAT (all CRCs valid, so
    /// the header parses), with no real pixel data.
    fn ihdr_only_png(w: u32, h: u32) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            let mut crc = 0xFFFF_FFFFu32;
            for b in &body {
                crc ^= *b as u32;
                for _ in 0..8 {
                    crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
                }
            }
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(&body);
            out.extend_from_slice(&(!crc).to_be_bytes());
        }
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = w.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &[0x78, 0x9c, 0, 0]);
        out
    }

    #[test]
    fn decode_budget_is_checked_from_the_header() {
        let mut buf = Vec::new();
        let img = image::RgbaImage::from_fn(16, 8, |x, y| image::Rgba([x as u8, y as u8, 9, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        // Decode, the RGBA conversion and the premultiplied copy are charged: two buffers at peak.
        assert!(decode_pixmap_within(&buf, 1024).is_some());
        assert!(decode_pixmap_within(&buf, 1023).is_none());
        // 16-bit RGBA decodes to 8 bytes a pixel, alive with its RGBA8 copy: 12 bytes a pixel.
        let mut wide = Vec::new();
        let img16 = image::ImageBuffer::<image::Rgba<u16>, _>::from_fn(16, 8, |x, y| image::Rgba([x as u16 * 999, y as u16, 9, 65535]));
        image::DynamicImage::ImageRgba16(img16).write_to(&mut std::io::Cursor::new(&mut wide), image::ImageFormat::Png).unwrap();
        assert!(decode_pixmap_within(&wide, 1024).is_none(), "two RGBA8 buffers' worth is not enough for RGBA16");
        assert!(decode_pixmap_within(&wide, 1536).is_some());
        // A header (valid CRC) claiming 60000x60000 with no pixel data reads fine, then the budget refuses it.
        let hdr = ihdr_only_png(60_000, 60_000);
        assert_eq!(image_size(&hdr), Some((60_000, 60_000)));
        assert!(decode_pixmap(&hdr).is_none());
    }

    /// A `Rendered` whose pixels don't match its claimed size isn't padded out to that size: a
    /// 60000×60000 claim over no pixels would allocate 14 GB. It, and any mismatch, encodes to
    /// nothing (JPEG used to assert on the length).
    #[test]
    fn encoders_refuse_pixels_that_dont_match_the_size() {
        let huge = Rendered { width: 60_000, height: 60_000, pixels: Vec::new() };
        assert!(huge.to_png().is_empty());
        assert!(huge.to_jpeg(80).is_empty());
        let short = Rendered { width: 2, height: 2, pixels: vec![255; 4] };
        assert!(short.to_png().is_empty());
        assert!(short.to_jpeg(80).is_empty());
        let long = Rendered { width: 1, height: 1, pixels: vec![255; 8] };
        assert!(long.to_png().is_empty());
        let ok = Rendered { width: 2, height: 2, pixels: vec![255; 16] };
        assert!(!ok.to_png().is_empty());
        assert!(!ok.to_jpeg(80).is_empty());
    }

    #[test]
    fn page_raster_is_capped_by_total_pixels() {
        let d = Document::new();
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let opts = RenderOptions::default();
        let big = render_region_capped(&d, &l.pages[0], 3000, 2000, Affine::scale(4.0), &opts, 100_000);
        assert!(big.width as u64 * big.height as u64 <= 100_000, "{}x{}", big.width, big.height);
        assert!(big.width > 300 && big.height > 200);
        assert_eq!(big.pixels.len() as u64, big.width as u64 * big.height as u64 * 4);
        let ok = render_region_capped(&d, &l.pages[0], 300, 200, Affine::IDENTITY, &opts, 100_000);
        assert_eq!((ok.width, ok.height), (300, 200));
    }

    #[test]
    fn hostile_scale_is_clamped() {
        let d = Document::new();
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(&d, &l.pages[0], f32::NAN, &RenderOptions::default());
        assert_eq!(img.width, 612);
        let big = render_page(&d, &l.pages[0], 1e9, &RenderOptions::default());
        assert!(big.width <= MAX_SIDE);
        assert!(decode_pixmap(b"not an image").is_none());
    }

    /// A JPEG whose headers say progressive (SOF2, a DC-only first scan) and `w` × `h`, its three
    /// components unsubsampled, with no pixel data that size.
    fn progressive_jpeg_header(w: u16, h: u16) -> Vec<u8> {
        let mut jpeg = Vec::new();
        image::RgbImage::new(16, 16).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        let sof = jpeg.windows(2).position(|m| m == [0xFF, 0xC0]).unwrap();
        jpeg[sof + 1] = 0xC2;
        jpeg[sof + 5..sof + 7].copy_from_slice(&h.to_be_bytes());
        jpeg[sof + 7..sof + 9].copy_from_slice(&w.to_be_bytes());
        for c in 0..3 {
            jpeg[sof + 11 + 3 * c] = 0x11;
        }
        let sos = jpeg.windows(2).position(|m| m == [0xFF, 0xDA]).unwrap();
        let n = usize::from(jpeg[sos + 4]);
        jpeg[sos + 6 + 2 * n] = 0;
        jpeg
    }

    #[test]
    fn progressive_jpeg_coefficients_count_toward_the_decode_budget() {
        // Two RGBA buffers of 7900 × 7900 fit 512 MB, but RGB output beside three full-size
        // coefficient planes (9 bytes a pixel) doesn't (#3644).
        let jpeg = progressive_jpeg_header(7900, 7900);
        assert!(wordcraft_geom::decode_fits(7900, 7900, wordcraft_geom::MAX_DECODED_BYTES));
        assert!(decode_pixmap(&jpeg).is_none());
        let mut baseline = Vec::new();
        image::RgbImage::new(64, 64).write_to(&mut std::io::Cursor::new(&mut baseline), image::ImageFormat::Jpeg).unwrap();
        assert!(decode_pixmap(&baseline).is_some(), "other JPEGs still draw");
    }

    /// #3665: a picture too large for the whole cache is kept at half of it, so it is cached
    /// instead of decoded again on every render.
    #[test]
    fn pictures_too_large_for_the_cache_are_kept_smaller() {
        let px = vello_cpu::color::PremulRgba8 { r: 40, g: 80, b: 120, a: 200 };
        // 6000 × 6000 is 144 MB of RGBA, past the 128 MB cache.
        let big = Pixmap::from_parts(vec![px; 6000 * 6000], 6000, 6000);
        let small = cacheable(big);
        let bytes = small.width() as usize * small.height() as usize * 4;
        assert!(bytes <= CACHE_BYTES / 2, "{} x {}", small.width(), small.height());
        assert!(small.width() >= 3000 && small.width() == small.height(), "aspect kept");
        assert!(small.data().iter().all(|p| (p.r, p.g, p.b, p.a) == (40, 80, 120, 200)), "flat stays flat");
        let mut cache = MediaCache { map: HashMap::new(), bytes: 0 };
        // A source file larger than the whole cache (an uncompressed BMP, say) doesn't count against it.
        let src = Arc::new(vec![0u8; CACHE_BYTES + 1]);
        cache.insert("big", &src, Some(Arc::new(small)), bytes);
        assert!(cache.get("big", &src).is_some(), "cached");
        // Pictures the cache holds as they are keep their size.
        assert_eq!(cacheable(Pixmap::from_parts(vec![px; 100 * 50], 100, 50)).width(), 100);
    }

    #[test]
    fn shrinking_averages_premultiplied_pixels() {
        let p = |v: u8| vello_cpu::color::PremulRgba8 { r: v, g: v, b: v, a: 255 };
        let pm = Pixmap::from_parts(vec![p(0), p(100), p(200), p(40)], 4, 1);
        let out = shrink(&pm, 2).unwrap();
        assert_eq!((out.width(), out.height()), (2, 1));
        assert_eq!(out.data().iter().map(|q| q.r).collect::<Vec<_>>(), [50, 120]);
        assert!(shrink(&pm, 4).is_none() && shrink(&pm, 0).is_none());
    }

    #[test]
    fn dimmed_header_image_renders() {
        let mut d = Document::from_text("body");
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let media = d.add_media(png, "png");
        let mut hp = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image {
            media,
            w: 40.0,
            h: 40.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        };
        hp.insert_object(0, obj, &Default::default()).unwrap();
        let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
        d.last_section.headers.default = Some(id);
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let opts = RenderOptions::default();
        assert!(opts.display.dim_header);
        let img = render_page(&d, &l.pages[0], 1.0, &opts);
        assert!(!img.to_png().is_empty());
    }

    /// #2892: a rule is stroked with flat ends, as the PDF export and Word draw them. Round caps
    /// used to stick half the width out past every end — a blob over the neighbouring border at
    /// each corner of a table with thick borders, while the exported PDF had square corners.
    #[test]
    fn a_thick_rule_ends_flat_not_round() {
        let d = Document::new();
        let rule = |stroke| Draw::Line { x0: 10.0, y0: 10.0, x1: 30.0, y1: 10.0, width: 8.0, color: Rgb::BLACK, stroke, alpha: 1.0, text_bg: None };
        let shot = |stroke| render_items(&d, &[rule(stroke)], 0.0, 0.0, 40.0, 20.0, 1.0, &RenderOptions::default());
        let ink = |img: &Rendered, x: u32, y: u32| img.pixel(x, y)[3] > 128;
        // An 8 pt rule from x=10 to x=30 covers y 6..14, and nothing beyond either end.
        let img = shot(Stroke::Solid);
        assert!(ink(&img, 15, 10), "the rule itself is drawn");
        assert!(!ink(&img, 6, 10) && !ink(&img, 33, 10), "no round cap past x=10 or x=30");
        // The double style's two strands end flat as well: its upper strand is centred on y=2.8
        // with a 4.8 pt width, so x=31 lies in the half-disc a round cap would add.
        let img = shot(Stroke::Double);
        assert!(ink(&img, 15, 3), "the double rule's upper strand is drawn");
        assert!(!ink(&img, 31, 3), "the upper strand ends at x=30");
    }

    /// A dimmed header is its full-strength picture faded once, as Word dims it: where a header
    /// table's borders meet they are the tone of the borders, not darker.
    #[test]
    fn dimmed_header_borders_dont_darken_where_they_meet() {
        let mut d = Document::from_text("");
        let t = wordcraft_doc::Table::new(2, 2, 200.0);
        let blocks = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t)), wordcraft_doc::para_block(wordcraft_doc::Paragraph::new())];
        let id = d.add_part(wordcraft_doc::PartKind::Header, blocks);
        d.last_section.headers.default = Some(id);
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let render = |dim_header: bool| {
            let mut opts = RenderOptions::default();
            opts.display.dim_header = dim_header;
            render_page(&d, &l.pages[0], 3.0, &opts)
        };
        let (full, dim) = (render(false), render(true));
        let mut ink = 0;
        for y in 0..full.height {
            for x in 0..full.width {
                let (f, g) = (full.pixel(x, y)[0] as f32, dim.pixel(x, y)[0] as f32);
                ink += usize::from(f < 128.0);
                let want = 255.0 - 0.45 * (255.0 - f);
                assert!((g - want).abs() <= 2.0, "({x}, {y}): {g} dimmed, {want} wanted ({f} at full strength)");
            }
        }
        assert!(ink > 100, "the borders are drawn: {ink}");
    }

    /// #2946: a dimmed header's items are faded as one group, so two of its lines that cross come
    /// out the same tone as the lines themselves, and so does a border two cells share. Faded item
    /// by item, the crossing kept 0.55² of the background instead of 0.55, and a shared border read
    /// darker than the outer ones.
    #[test]
    fn dimmed_header_borders_keep_one_tone_where_they_meet() {
        let mut d = Document::from_text("Body");
        // Two rows and two columns: the middle border is drawn by both cells beside it, and a cell's
        // top and left rules cross at its top-left corner.
        let header = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(wordcraft_doc::Table::new(2, 2, 300.0)))];
        let id = d.add_part(wordcraft_doc::PartKind::Header, header);
        d.last_section.headers.default = Some(id);
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let page = &l.pages[0];
        let rules: Vec<(f32, f32, f32, f32)> = page
            .header
            .iter()
            .filter_map(|it| if let wordcraft_layout::Placed::Rule { x0, y0, x1, y1, .. } = it { Some((*x0, *y0, *x1, *y1)) } else { None })
            .collect();
        let (vx, vy0, _, vy1) = *rules.iter().filter(|(x0, _, x1, _)| x0 == x1).min_by(|a, b| a.0.total_cmp(&b.0)).expect("a vertical rule");
        let (hx0, hy, hx1, _) =
            *rules.iter().find(|(_, y0, _, y1)| y0 == y1 && (y0 - vy0).abs() < 0.01).expect("the horizontal rule at the cell's top");
        assert!(hx0 <= vx && vx <= hx1 && vy0 <= hy && hy <= vy1, "the rules cross: ({hx0}, {hy}, {hx1}) across ({vx}, {vy0}, {vy1})");
        let bottom = rules.iter().filter(|(_, y0, _, y1)| y0 == y1).map(|(_, y, _, _)| *y).fold(f32::NEG_INFINITY, f32::max);
        let row = hy + (bottom - hy) * 0.25; // inside the first row, clear of any rule across it
        // The darkest pixel in a small box: the tone of a line, however it is antialiased.
        let scale = 8.0;
        let darkest = |img: &Rendered, cx: f32, cy: f32| {
            let (px, py) = ((cx * scale) as i64, (cy * scale) as i64);
            (px - 2..=px + 2)
                .flat_map(|x| (py - 2..=py + 2).map(move |y| (x, y)))
                .filter(|(x, y)| *x >= 0 && *y >= 0)
                .map(|(x, y)| img.pixel(x as u32, y as u32)[0])
                .min()
                .expect("a pixel")
        };
        let dimmed = render_page(&d, page, scale, &RenderOptions::default());
        let undimmed = RenderOptions { display: DisplayOptions { dim_header: false, ..Default::default() }, ..Default::default() };
        let undimmed = render_page(&d, page, scale, &undimmed);
        // One border, well clear of any crossing or shared edge: the tone the others must keep.
        let line = darkest(&dimmed, vx + 30.0, hy);
        assert!(line > 0, "the dimmed border is drawn, faded: {line}");
        assert_eq!(darkest(&dimmed, vx, hy), line, "the crossing is darker than the line");
        assert_eq!(darkest(&dimmed, hx0 + 150.0, row), line, "a shared border is darker than the outer ones");
        assert_eq!(darkest(&undimmed, vx, hy), darkest(&undimmed, vx + 30.0, hy), "undimmed too");
        assert!(darkest(&undimmed, vx + 30.0, hy) < line, "the header is faded");
    }

    #[test]
    fn dark_page_inverts_lightness() {
        assert_eq!(invert_lightness(Rgb::WHITE), Rgb::BLACK);
        assert_eq!(invert_lightness(Rgb::BLACK), Rgb::WHITE);
        // Pure hues keep their colour; dark blue turns light blue.
        assert_eq!(invert_lightness(Rgb(255, 0, 0)), Rgb(255, 0, 0));
        assert_eq!(invert_lightness(Rgb(0, 0, 0x80)), Rgb(0x7F, 0x7F, 0xFF));
        let d = Document::from_text("Hello WordCraft");
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let opts = RenderOptions { dark: true, ..Default::default() };
        assert_eq!(opts.ink(Rgb::WHITE), Rgb(DARK_PAPER, DARK_PAPER, DARK_PAPER));
        assert_eq!(opts.ink(Rgb::BLACK), Rgb::WHITE);
        let img = render_page(&d, &l.pages[0], 1.0, &opts);
        assert_eq!(img.pixel(5, 5), [DARK_PAPER, DARK_PAPER, DARK_PAPER, 255]);
        let light = (72..300).flat_map(|x| (72..100).map(move |y| (x, y))).filter(|(x, y)| img.pixel(*x, *y)[0] > 128).count();
        assert!(light > 30, "light {light}");
    }

    /// #2534: legacy font effects are drawn: outlined letters are hollow; shadowed and embossed
    /// ones have a grey copy down-right, imprinted ones up-left.
    #[test]
    fn legacy_font_effects_are_drawn() {
        use wordcraft_doc::CharProps;
        use wordcraft_doc::wordart::{TextEffects, TextFill};
        const SIZE: f32 = 40.0;
        const SCALE: f32 = 2.0;
        const MIN_DIRECTION_PIXELS: usize = 100;
        let shot = |c: CharProps, offset: f32| {
            let mut d = Document::from_text("x");
            let cp = CharProps { size: Some(SIZE), bold: Some(true), ..c };
            d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("HHHH", cp))];
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            render_area(&d, &l.pages[0], -offset, -offset, 400.0, 150.0, SCALE, &RenderOptions::default())
        };
        let counts = |img: &Rendered| {
            let (mut dark, mut grey) = (0usize, 0usize);
            for (x, y) in (100..800).flat_map(|x| (100..300).map(move |y| (x, y))) {
                let v = img.pixel(x, y)[0];
                dark += usize::from(v < 60);
                grey += usize::from((120..=136).contains(&v));
            }
            (dark, grey)
        };
        let (plain_dark, plain_grey) = counts(&shot(CharProps::default(), 0.0));
        assert!(plain_dark > 2000, "plain {plain_dark}");
        let (dark, _) = counts(&shot(CharProps { outline: Some(true), ..Default::default() }, 0.0));
        assert!(dark < plain_dark / 3, "outline is hollow: {dark} vs {plain_dark}");
        // An explicit fill keeps the reference on the same Art path as legacy effects.
        let reference = || CharProps {
            text_effects: Some(Box::new(TextEffects { fill: Some(TextFill::Solid { color: Rgb::BLACK, transparency: 0.0 }), ..Default::default() })),
            ..Default::default()
        };
        let base = shot(reference(), 0.0);
        for (c, sign) in [
            (CharProps { shadow: Some(true), ..Default::default() }, 1.0),
            (CharProps { emboss: Some(true), ..Default::default() }, 1.0),
            (CharProps { engrave: Some(true), ..Default::default() }, -1.0),
        ] {
            let img = shot(c, 0.0);
            let (_, grey) = counts(&img);
            assert!(grey > plain_grey * 2, "a grey copy: {grey} vs {plain_grey}");
            // A 2pt diagonal displacement at 40pt has sqrt(2)pt on each axis.
            // Check the visible copy against its translated mask, accounting for occlusion.
            let offset = sign * std::f32::consts::SQRT_2;
            let expected = shot(reference(), offset);
            let opposite = shot(reference(), -offset);
            let (mut visible, mut directional) = (0usize, 0usize);
            for (x, y) in (100..800).flat_map(|x| (100..300).map(move |y| (x, y))) {
                let v = img.pixel(x, y)[0];
                if !(120..=136).contains(&v) || base.pixel(x, y)[0] != 255 {
                    continue;
                }
                visible += 1;
                assert!(expected.pixel(x, y)[0] < 255, "grey outside translated Art mask: sign {sign}, pixel {x},{y}");
                directional += usize::from(expected.pixel(x, y)[0] < 60 && opposite.pixel(x, y)[0] == 255);
            }
            assert!(directional > MIN_DIRECTION_PIXELS, "directional grey: sign {sign}, {directional} of {visible}");
            eprintln!("spatial shadow sign={sign} grey={grey} visible={visible} directional={directional}");
        }
    }

    /// A document whose only text is a yellow-highlighted run.
    fn yellow_highlight_doc() -> (Document, wordcraft_layout::DocLayout) {
        let cp = wordcraft_doc::CharProps { highlight: Some(wordcraft_doc::props::Highlight::Yellow), size: Some(24.0), ..Default::default() };
        let mut d = Document::from_text("x");
        d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("HHHH HHHH", cp))];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        (d, l)
    }

    /// #1252: on a dark page a highlight is drawn dimmed (keeping its hue) so the white text on it
    /// reads; a light page is unchanged.
    #[test]
    fn dark_page_dims_a_highlight_raster() {
        let (d, l) = yellow_highlight_doc();
        let items = page_display(&d, &l.pages[0], &DisplayOptions::default());
        let r = items
            .iter()
            .find_map(|i| match i {
                Draw::Fill { rect, color, .. } if *color == Rgb(255, 255, 0) => Some(*rect),
                _ => None,
            })
            .unwrap();
        // Just inside the top-left corner of the highlight: above the capitals, so pure background.
        let (px, py) = ((r.x + 1.5) as u32, (r.y + 1.5) as u32);
        let light = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
        assert_eq!(light.pixel(px, py), [255, 255, 0, 255], "light page keeps bright yellow");
        let dark = render_page(&d, &l.pages[0], 1.0, &RenderOptions { dark: true, ..Default::default() });
        let [r8, g8, b8, _] = dark.pixel(px, py);
        assert!(r8 < 0x60 && g8 < 0x60, "dark page highlight is dim, got {:?}", (r8, g8, b8));
        assert!(r8.abs_diff(g8) <= 4 && g8 > b8.saturating_add(0x20), "dark page highlight stays yellow in hue, got {:?}", (r8, g8, b8));
    }

    /// #1252: on a dark page every highlight colour is dimmed so the text drawn on it reads
    /// (Automatic and explicit black text both draw white); a light page is the identity.
    #[test]
    fn dark_page_text_backgrounds_keep_text_readable() {
        use wordcraft_doc::props::{Highlight, TextColor};
        use wordcraft_layout::display::text_color;
        let dark = RenderOptions { dark: true, ..Default::default() };
        let light = RenderOptions::default();
        let colours: Vec<Rgb> = Highlight::ALL.iter().filter_map(|h| h.rgb()).collect();
        assert_eq!(colours.len(), 16);
        for c in colours {
            let bg = dark.ink_bg(c);
            for (what, tc) in [("automatic", text_color(&TextColor::Auto, Some(c))), ("black", Rgb::BLACK)] {
                let fg = dark.ink_text(tc, Some(c));
                assert_eq!(fg, Rgb::WHITE, "{what} text on {c:?} draws white");
                let k = contrast(fg, bg);
                assert!(k >= 4.5, "{what} text on {c:?}: {fg:?} on {bg:?} has contrast {k:.2}");
            }
            assert_eq!(light.ink_bg(c), c, "light page keeps {c:?}");
            assert_eq!(light.ink_text(c, Some(c)), c, "light page keeps text {c:?}");
        }
        // Yellow turns a dim olive (~#4a4a10), still yellow in hue.
        let Rgb(r, g, b) = dark.ink_bg(Rgb(255, 255, 0));
        assert!(r.abs_diff(0x4A) <= 8 && g.abs_diff(0x4A) <= 8 && b.abs_diff(0x10) <= 8, "yellow → {:?}", (r, g, b));
        assert!(r == g && g > b);
        // Text off a highlight keeps the plain inversion.
        assert_eq!(dark.ink_text(Rgb::WHITE, None), dark.ink(Rgb::WHITE));
    }

    /// #1252 review: explicit coloured text (pure blue, red, every highlight colour) on every
    /// dimmed highlight still reaches 4.5:1 on a dark page.
    #[test]
    fn dark_page_coloured_text_on_highlights_reads() {
        use wordcraft_doc::props::Highlight;
        let dark = RenderOptions { dark: true, ..Default::default() };
        let colours: Vec<Rgb> = Highlight::ALL.iter().filter_map(|h| h.rgb()).collect();
        let texts: Vec<Rgb> = [Rgb(0, 0, 255), Rgb(255, 0, 0)].into_iter().chain(colours.iter().copied()).collect();
        for &h in &colours {
            let bg = dark.ink_bg(h);
            for &tc in &texts {
                let fg = dark.ink_text(tc, Some(h));
                let k = contrast(fg, bg);
                assert!(k >= 4.5, "text {tc:?} on {h:?}: {fg:?} on {bg:?} has contrast {k:.2}");
            }
        }
    }

    /// #1252 review: an Automatic underline on a dark highlight (black) draws light on a dark page,
    /// like the text it underlines, not the dark grey plain inversion gives white.
    #[test]
    fn dark_page_underline_on_a_dark_highlight_reads() {
        use wordcraft_doc::props::{Highlight, Underline};
        let cp =
            wordcraft_doc::CharProps { highlight: Some(Highlight::Black), underline: Some(Underline::Thick), size: Some(24.0), ..Default::default() };
        let mut d = Document::from_text("x");
        d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("HHHH HHHH", cp))];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let items = page_display(&d, &l.pages[0], &DisplayOptions::default());
        let (x0, x1, y, w) = items
            .iter()
            .find_map(|i| match i {
                Draw::Line { x0, x1, y0, width, .. } => Some((*x0, *x1, *y0, *width)),
                _ => None,
            })
            .unwrap();
        assert!(w >= 1.5, "thick underline {w}");
        let (px, py) = (((x0 + x1) / 2.0) as u32, y.round() as u32);
        let dark = render_page(&d, &l.pages[0], 4.0, &RenderOptions { dark: true, ..Default::default() });
        let [r8, g8, b8, _] = dark.pixel(px * 4, py * 4);
        assert!(r8 > 0xC0 && g8 > 0xC0 && b8 > 0xC0, "underline draws light, got {:?}", (r8, g8, b8));
    }

    /// #2534 review: Automatic text with a legacy shadow (drawn from its outlines) on a black
    /// highlight draws light on a dark page, like plain text does, not dark grey on near-black.
    #[test]
    fn dark_page_shadowed_text_on_a_dark_highlight_reads() {
        use wordcraft_doc::props::Highlight;
        let cp = wordcraft_doc::CharProps { highlight: Some(Highlight::Black), shadow: Some(true), size: Some(24.0), ..Default::default() };
        let mut d = Document::from_text("x");
        d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("HHHH HHHH", cp))];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let items = page_display(&d, &l.pages[0], &DisplayOptions::default());
        let b = items
            .iter()
            .find_map(|i| match i {
                Draw::Art { path, text_bg, .. } => {
                    assert!(text_bg.is_some(), "the run's highlight is the letters' background");
                    wordcraft_layout::wordart::path_box(path)
                }
                _ => None,
            })
            .unwrap();
        let dark = render_page(&d, &l.pages[0], 2.0, &RenderOptions { dark: true, ..Default::default() });
        let light = (b.x0 as u32 * 2..b.x1 as u32 * 2)
            .flat_map(|x| (b.y0 as u32 * 2..b.y1 as u32 * 2).map(move |y| (x, y)))
            .filter(|&(x, y)| dark.pixel(x, y)[..3].iter().all(|&v| v > 0xE0))
            .count();
        assert!(light > 200, "letters draw light, {light} light pixels");
    }

    /// One EMF record: type, size, payload (a multiple of 4 bytes).
    fn emf_rec(typ: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = typ.to_le_bytes().to_vec();
        v.extend(((8 + payload.len()) as u32).to_le_bytes());
        v.extend(payload);
        v
    }

    fn le32(vals: &[i32]) -> Vec<u8> {
        vals.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// A 25 mm square EMF (device units 0.04 per 0.01 mm, so 100 device units) with a red rectangle over
    /// its right half.
    fn red_right_half_emf() -> Vec<u8> {
        let mut h = vec![0u8; 88];
        h[0..4].copy_from_slice(&1u32.to_le_bytes());
        h[4..8].copy_from_slice(&88u32.to_le_bytes());
        h[8..24].copy_from_slice(&le32(&[0, 0, 100, 100]));
        h[24..40].copy_from_slice(&le32(&[0, 0, 2500, 2500]));
        h[40..44].copy_from_slice(&0x464D_4520u32.to_le_bytes());
        h[72..88].copy_from_slice(&le32(&[1000, 1000, 250, 250]));
        let brush = emf_rec(39, &le32(&[1, 0, 0x0000_00FF, 0]));
        let select = emf_rec(37, &le32(&[1]));
        let rect = emf_rec(43, &le32(&[50, 0, 100, 100]));
        let eof = emf_rec(14, &[0; 12]);
        [h, brush, select, rect, eof].concat()
    }

    #[test]
    fn metafile_picture_is_drawn_as_vectors() {
        let mut d = Document::new();
        let media = d.add_media(red_right_half_emf(), "emf");
        let mut p = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image {
            media,
            w: 70.0,
            h: 70.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        };
        p.insert_object(0, obj, &Default::default()).unwrap();
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let opts = RenderOptions::default();
        let rect = page_display(&d, &l.pages[0], &opts.display)
            .iter()
            .find_map(|i| match i {
                Draw::Image { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("a picture on the page");
        let img = render_page(&d, &l.pages[0], 1.0, &opts);
        let y = (rect.y + rect.h / 2.0) as u32;
        let right = img.pixel((rect.x + rect.w * 0.75) as u32, y);
        let left = img.pixel((rect.x + rect.w * 0.25) as u32, y);
        assert!(right[0] > 200 && right[1] < 60 && right[2] < 60, "right half should be red: {right:?}");
        assert!(left[..3].iter().all(|c| *c > 200), "left half should be paper, not the grey box: {left:?}");
    }

    /// #1899: a crop that keeps a sliver of the picture (3% here; a header's bar can be 2.7% of a
    /// page-sized metafile) stretches that sliver over the frame, as the file says, for vector and
    /// bitmap pictures alike. Both pictures are paper on the left half and red on the right.
    #[test]
    fn a_crop_keeping_a_sliver_fills_the_frame_with_it() {
        let mut png = Vec::new();
        image::RgbaImage::from_fn(100, 100, |x, _| image::Rgba(if x >= 50 { [255, 0, 0, 255] } else { [255; 4] }))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        for (bytes, ext) in [(red_right_half_emf(), "emf"), (png, "png")] {
            // (crop, where in the frame, red?): 47–50% of the source is all paper; 50–53% all red.
            for (crop, at, red) in [([0.47, 0.47, 0.5, 0.5], 0.7, false), ([0.5, 0.47, 0.47, 0.5], 0.3, true)] {
                let mut d = Document::new();
                let media = d.add_media(bytes.clone(), ext);
                let mut p = wordcraft_doc::Paragraph::new();
                let obj = wordcraft_doc::para::InlineObject::Image {
                    media,
                    w: 70.0,
                    h: 70.0,
                    alt: String::new(),
                    float: Default::default(),
                    crop,
                    ole: None,
                };
                p.insert_object(0, obj, &Default::default()).unwrap();
                d.body = vec![wordcraft_doc::para_block(p)];
                let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
                let opts = RenderOptions::default();
                let rect = page_display(&d, &l.pages[0], &opts.display)
                    .iter()
                    .find_map(|i| match i {
                        Draw::Image { rect, .. } => Some(*rect),
                        _ => None,
                    })
                    .expect("a picture on the page");
                let img = render_page(&d, &l.pages[0], 1.0, &opts);
                let px = img.pixel((rect.x + rect.w * at) as u32, (rect.y + rect.h * at) as u32);
                let is_red = px[0] > 200 && px[1] < 60 && px[2] < 60;
                let is_paper = px[..3].iter().all(|c| *c > 200);
                assert!(if red { is_red } else { is_paper }, "{ext} crop {crop:?} at {at}: {px:?}");
            }
        }
    }

    #[test]
    fn cosmetic_metafile_pen_is_one_device_pixel_wide() {
        // MOVETO (0, 50), LINETO (100, 50) with the default cosmetic black pen: a hairline across the middle.
        let mut emf = red_right_half_emf()[..88].to_vec();
        emf.extend([emf_rec(27, &le32(&[0, 50])), emf_rec(54, &le32(&[100, 50])), emf_rec(14, &[0; 12])].concat());
        let mut d = Document::new();
        let media = d.add_media(emf, "emf");
        let mut p = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image {
            media,
            w: 70.0,
            h: 70.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        };
        p.insert_object(0, obj, &Default::default()).unwrap();
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let opts = RenderOptions::default();
        let rect = page_display(&d, &l.pages[0], &opts.display)
            .iter()
            .find_map(|i| match i {
                Draw::Image { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("a picture on the page");
        let scale = 2.0;
        let img = render_page(&d, &l.pages[0], scale, &opts);
        let (x, y) = (((rect.x + rect.w / 2.0) * scale) as u32, ((rect.y + rect.h / 2.0) * scale) as u32);
        let darkest = (y - 3..=y + 3).map(|y| img.pixel(x, y)[0]).min().unwrap_or(255);
        // A 0.1 pt line would cover a fifth of a pixel (about 204); one device pixel covers at least half of one.
        assert!(darkest < 140, "the hairline should be clearly visible: {darkest}");
    }

    /// #1764: an EMF picture's text (EXTCREATEFONTINDIRECTW, SELECTOBJECT, SETTEXTCOLOR, EXTTEXTOUTW)
    /// is painted, not dropped.
    #[test]
    fn metafile_text_records_are_painted() {
        let mut font = emf_rec(82, &le32(&[1, -20, 0, 0, 0, 700, 0, 0]));
        let mut name: Vec<u8> = "Arial".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        name.resize(64, 0);
        font.extend(name);
        font[4..8].copy_from_slice(&104u32.to_le_bytes());
        // "WWWW" at (5, 40), top-aligned, no dx array.
        let mut text = emf_rec(84, &le32(&[0, 0, 100, 100, 1, 0, 0, 5, 40, 4, 76, 0, 0, 0, 0, 0, 0]));
        text.extend("WWWW".encode_utf16().flat_map(|u| u.to_le_bytes()));
        text[4..8].copy_from_slice(&84u32.to_le_bytes());
        let mut emf = red_right_half_emf()[..88].to_vec();
        emf.extend([font, emf_rec(37, &le32(&[1])), emf_rec(24, &le32(&[0])), text, emf_rec(14, &[0; 12])].concat());
        let mut d = Document::new();
        let media = d.add_media(emf, "emf");
        let mut p = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image {
            media,
            w: 70.0,
            h: 70.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        };
        p.insert_object(0, obj, &Default::default()).unwrap();
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let opts = RenderOptions::default();
        let rect = page_display(&d, &l.pages[0], &opts.display)
            .iter()
            .find_map(|i| match i {
                Draw::Image { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("a picture on the page");
        let scale = 2.0;
        let img = render_page(&d, &l.pages[0], scale, &opts);
        let (x0, y0) = ((rect.x * scale) as u32, (rect.y * scale) as u32);
        let (x1, y1) = (((rect.x + rect.w) * scale) as u32, ((rect.y + rect.h) * scale) as u32);
        let dark = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| img.pixel(x, y)[0] < 100).count();
        assert!(dark > 50, "the text is painted inside the picture: {dark} dark pixels");
    }

    /// An EMF text run whose anchor sits inside the picture but whose font is billions of units
    /// tall: each glyph's curves would span billions of device pixels, which the rasteriser
    /// subdivides for ever. Glyphs that big (or off the raster) are left out; the page renders.
    #[test]
    fn metafile_text_with_a_huge_font_is_left_out() {
        let mut font = emf_rec(82, &le32(&[1, -2_000_000_000, 0, 0, 0, 400, 0, 0]));
        let mut name: Vec<u8> = "Arial".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        name.resize(64, 0);
        font.extend(name);
        font[4..8].copy_from_slice(&104u32.to_le_bytes());
        let mut text = emf_rec(84, &le32(&[0, 0, 100, 100, 1, 0, 0, 5, 40, 4, 76, 0, 0, 0, 0, 0, 0]));
        text.extend("WOWS".encode_utf16().flat_map(|u| u.to_le_bytes()));
        text[4..8].copy_from_slice(&84u32.to_le_bytes());
        let mut emf = red_right_half_emf()[..88].to_vec();
        emf.extend([font, emf_rec(37, &le32(&[1])), emf_rec(24, &le32(&[0])), text, emf_rec(14, &[0; 12])].concat());
        let mut d = Document::new();
        let media = d.add_media(emf, "emf");
        let mut p = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image {
            media,
            w: 70.0,
            h: 70.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        };
        p.insert_object(0, obj, &Default::default()).unwrap();
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let start = std::time::Instant::now();
        let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
        // About 20 ms without the huge glyphs; drawing them ran past 4 minutes without finishing.
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
        assert_eq!(img.width, 612);
        // Leaving them out is logged (once a run, so it may have been by another test already).
        assert!(OVERSIZED_GLYPHS_NOTED.load(std::sync::atomic::Ordering::Relaxed), "the left-out glyphs are logged");
    }

    /// Glyphs left out for their size are logged the first time only: a page is drawn on every
    /// repaint. A run with none left out logs nothing and doesn't use up the warning.
    #[test]
    fn oversized_glyphs_are_logged_once_a_run() {
        let noted = std::sync::atomic::AtomicBool::new(false);
        assert!(!note_oversized_glyphs(&noted, 0), "nothing left out, nothing logged");
        assert!(!noted.load(std::sync::atomic::Ordering::Relaxed));
        assert!(note_oversized_glyphs(&noted, 4), "the first glyphs left out are logged");
        assert!(!note_oversized_glyphs(&noted, 2), "later ones aren't");
    }

    #[test]
    fn unreadable_metafile_keeps_the_grey_box() {
        let mut d = Document::new();
        // The header alone has no records, so the metafile does not parse.
        let media = d.add_media(red_right_half_emf()[..88].to_vec(), "emf");
        let mut p = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image {
            media,
            w: 70.0,
            h: 70.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        };
        p.insert_object(0, obj, &Default::default()).unwrap();
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let opts = RenderOptions::default();
        let rect = page_display(&d, &l.pages[0], &opts.display)
            .iter()
            .find_map(|i| match i {
                Draw::Image { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("a picture on the page");
        let img = render_page(&d, &l.pages[0], 1.0, &opts);
        assert_eq!(img.pixel((rect.x + rect.w / 2.0) as u32, (rect.y + rect.h / 2.0) as u32), [0xD0, 0xD0, 0xD0, 255]);
    }

    #[test]
    fn shapes_have_paths() {
        let r = kurbo::Rect::new(0.0, 0.0, 10.0, 10.0);
        for k in [
            ShapeKind::Rectangle,
            ShapeKind::Ellipse,
            ShapeKind::FlowchartOr,
            ShapeKind::Star,
            ShapeKind::Heart,
            ShapeKind::Arrow,
            ShapeKind::ArrowLeft,
            ShapeKind::ArrowUp,
            ShapeKind::ArrowDown,
            ShapeKind::Line,
        ] {
            assert!(!shape_path(k, r, None).elements().is_empty());
        }
    }

    #[test]
    fn dashed_outline_has_gaps() {
        use wordcraft_doc::{
            connector::ShapeExtra,
            line_dash::LineDash,
            para::{Anchor, Float, InlineObject, Wrap},
        };
        let image = |dash| {
            let shape = InlineObject::Shape {
                kind: ShapeKind::Rectangle,
                w: 70.0,
                h: 28.0,
                fill: None,
                stroke: Some(Rgb(192, 0, 0)),
                stroke_width: 1.0,
                float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
                story: None,
                freeform: None,
                effects: Default::default(),
                extra: ShapeExtra { dash, ..Default::default() },
            };
            let mut p = wordcraft_doc::Paragraph::new();
            p.insert_object(0, shape, &Default::default()).unwrap();
            let mut d = Document::new();
            d.body = vec![wordcraft_doc::para_block(p)];
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            render_page(&d, &l.pages[0], 4.0, &RenderOptions::default())
        };
        let solid = image(LineDash::Solid);
        let dashed = image(LineDash::Dash);
        assert!(solid.pixel(422, 400)[1] < 80);
        assert_eq!(dashed.pixel(422, 400), [255, 255, 255, 255]);
        assert!(dashed.pixel(406, 400)[1] < 80);
    }

    /// Distinguish flat, square and round endpoints for both preset lines and connector paths.
    #[test]
    fn shape_line_caps_reach_the_screen() {
        use wordcraft_doc::{
            connector::{LineCap, ShapeExtra},
            para::{Anchor, Float, InlineObject, Wrap},
        };
        // A preset line uses Draw::Shape; a straight connector uses Draw::Path.
        for kind in [ShapeKind::Line, ShapeKind::StraightConnector] {
            for cap in [LineCap::Flat, LineCap::Square, LineCap::Round] {
                let shape = InlineObject::Shape {
                    kind,
                    w: 40.0,
                    h: 0.0,
                    fill: None,
                    stroke: Some(Rgb::BLACK),
                    stroke_width: 10.0,
                    float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
                    story: None,
                    freeform: None,
                    effects: Default::default(),
                    extra: ShapeExtra { cap, ..Default::default() },
                };
                let mut p = wordcraft_doc::Paragraph::new();
                p.insert_object(0, shape, &Default::default()).unwrap();
                let mut d = Document::new();
                d.body = vec![wordcraft_doc::para_block(p)];
                let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
                let image = render_page(&d, &l.pages[0], 4.0, &RenderOptions::default());
                let painted = |x, y| image.pixel(x, y)[0] < 80;
                assert!(painted(404, 400), "line interior: {kind:?} {cap:?}");
                assert_eq!(painted(388, 400), cap != LineCap::Flat, "past the endpoint: {kind:?} {cap:?}");
                assert_eq!(painted(384, 384), cap == LineCap::Square, "square corner: {kind:?} {cap:?}");
            }
        }
    }

    #[test]
    fn zero_width_connector_dashes() {
        use wordcraft_doc::{
            connector::ShapeExtra,
            line_dash::LineDash,
            para::{Anchor, Float, InlineObject, Wrap},
        };
        let shape = InlineObject::Shape {
            kind: ShapeKind::ElbowConnector,
            w: 70.0,
            h: 28.0,
            fill: None,
            stroke: Some(Rgb(192, 0, 0)),
            stroke_width: 0.0,
            float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: ShapeExtra { dash: LineDash::Dash, ..Default::default() },
        };
        let mut p = wordcraft_doc::Paragraph::new();
        p.insert_object(0, shape, &Default::default()).unwrap();
        let mut d = Document::new();
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let image = render_page(&d, &l.pages[0], 8.0, &RenderOptions::default());
        // A 0.75 pt stroke has a 3 pt dash followed by a 2.25 pt gap.
        assert!(image.pixel(812, 800)[1] < 80, "inside the first dash");
        assert_eq!(image.pixel(832, 800), [255, 255, 255, 255], "inside the first gap");
    }

    /// Dash lengths too small to cut the path make kurbo's dash iterator spin without moving along
    /// it (and every piece is more stroke to build): such a pattern is drawn solid, as are negative
    /// and non-finite ones. A normal dash pattern still dashes.
    #[test]
    fn hostile_dash_patterns_draw_solid_and_finish() {
        use wordcraft_doc::graphic::PathSeg;
        let line = |dash: Vec<f32>| Draw::Path {
            segs: vec![PathSeg::Move(10.0, 20.0), PathSeg::Line(90.0, 20.0)],
            fill: None,
            stroke: Some(Rgb(0, 0, 0)),
            stroke_width: 2.0,
            dash,
            cap: Default::default(),
        };
        let start = std::time::Instant::now();
        for dash in [vec![1e-30, 1e-30], vec![1e-30], vec![-1.0, 2.0], vec![f32::NAN, 1.0], vec![f32::INFINITY, 1.0], vec![0.0, 0.0]] {
            let img = render_items(&Document::new(), &[line(dash.clone())], 0.0, 0.0, 100.0, 40.0, 1.0, &RenderOptions::default());
            assert!(img.pixel(50, 20)[3] > 200, "{dash:?} draws solid");
        }
        // Milliseconds drawn solid; drawn as dashes they ran past 4 minutes without finishing.
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
        let dashed = render_items(&Document::new(), &[line(vec![10.0, 10.0])], 0.0, 0.0, 100.0, 40.0, 1.0, &RenderOptions::default());
        assert!(dashed.pixel(15, 20)[3] > 200 && dashed.pixel(25, 20)[3] == 0, "a normal pattern dashes");
        assert!(safe_dashes([3.0, 1.0], 1000.0).len() == 2);
        assert!(safe_dashes([1e-3, 1e-3], 1000.0).is_empty(), "a million pieces");
        assert!(safe_dashes(std::iter::repeat_n(1.0, 200_000), 1.0).is_empty(), "too many entries");
    }

    #[test]
    fn flowchart_or_spokes() {
        let r = kurbo::Rect::new(0.0, 0.0, 10.0, 10.0);
        let path = shape_path(ShapeKind::FlowchartOr, r, None);
        let elements = path.elements();
        for pair in [
            [kurbo::PathEl::MoveTo((0.0, 5.0).into()), kurbo::PathEl::LineTo((10.0, 5.0).into())],
            [kurbo::PathEl::MoveTo((5.0, 0.0).into()), kurbo::PathEl::LineTo((5.0, 10.0).into())],
        ] {
            assert!(elements.windows(2).any(|w| w == pair), "missing spoke {pair:?}");
        }
    }

    /// A rounded rectangle laid out and rendered: its corner pixel is painted when the corner is
    /// square, left white with the default corner; turned a quarter, the same.
    #[test]
    fn rendered_rounded_rectangles_follow_their_corner() {
        use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
        let rendered = |corner: Option<u32>, rot: f32| {
            let float = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, rot, ..Default::default() };
            let shape = wordcraft_doc::para::InlineObject::Shape {
                kind: ShapeKind::RoundedRectangle,
                w: 200.0,
                h: 200.0,
                fill: Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(0, 0, 0))),
                stroke: None,
                stroke_width: 0.0,
                float,
                story: None,
                freeform: None,
                effects: Default::default(),
                extra: wordcraft_doc::connector::ShapeExtra { corner, ..Default::default() },
            };
            let mut p = wordcraft_doc::Paragraph::new();
            p.insert_object(0, shape, &Default::default()).unwrap();
            let mut d = Document::new();
            d.body = vec![wordcraft_doc::para_block(p)];
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            render_page(&d, &l.pages[0], 1.0, &RenderOptions::default())
        };
        for rot in [0.0, 90.0] {
            // 3 pt in from the frame's top-left corner, and its middle.
            let (corner, middle) = ((103, 103), (200, 200));
            let square = rendered(Some(0), rot);
            assert_eq!(square.pixel(corner.0, corner.1)[0], 0, "square corner, turned {rot}");
            let rounded = rendered(None, rot);
            assert_eq!(rounded.pixel(corner.0, corner.1)[0], 255, "default corner, turned {rot}");
            assert_eq!(rounded.pixel(middle.0, middle.1)[0], 0, "filled, turned {rot}");
        }
    }

    /// A rounded rectangle's corner is its file's (`roundRect`'s `adj`), not always 16%.
    #[test]
    fn rounded_rectangle_corners_follow_their_radius() {
        use kurbo::Shape;
        let r = kurbo::Rect::new(0.0, 0.0, 200.0, 100.0);
        let inside = |corner, pt: (f64, f64)| shape_path(ShapeKind::RoundedRectangle, r, corner).contains(pt.into());
        // Two points inside the top-left corner: 1 pt and 4 pt in from both edges.
        let (tip, near) = ((1.0, 1.0), (4.0, 4.0));
        assert!(inside(Some(0), tip), "square corner");
        assert!(!inside(Some(3_565), tip) && inside(Some(3_565), near), "3.565 pt corner");
        assert!(!inside(None, near), "16 pt corner by default");
        assert!(!inside(Some(u32::MAX), (14.0, 14.0)) && inside(Some(u32::MAX), (100.0, 1.0)), "round ends");
    }

    /// #611: a line runs as DrawingML's `line` preset does, top-left to bottom-right (Word's
    /// files flip it for the other diagonal), not bottom-left to top-right.
    #[test]
    fn a_line_runs_from_top_left_to_bottom_right() {
        let p = shape_path(ShapeKind::Line, kurbo::Rect::new(10.0, 20.0, 30.0, 60.0), None);
        assert_eq!(p.elements(), &[kurbo::PathEl::MoveTo((10.0, 20.0).into()), kurbo::PathEl::LineTo((30.0, 60.0).into())]);
    }
}

#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod tests_color_glyphs;
