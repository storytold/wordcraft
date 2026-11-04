use super::{Affine, Arc, Document, FontDb, HashMap, Item, Mutex, Page, Picture, Pixmap, PlacedItem, RenderContext, Seg, Shape, Weak, kurbo, peniko};

/// Upper bound on the bytes the two decoded-media caches keep together (about 256 MB), half each;
/// past its half a cache starts over.
pub(crate) const CACHE_BYTES: usize = 256 * 1024 * 1024 / 2;

/// Upper bound on cached entries per cache.
const CACHE_ENTRIES: usize = 256;

/// Decoded media by media key, shared across renders, with an approximate byte budget. An entry knows
/// its source allocation weakly: a hit needs the same allocation, whose address can't be reused while
/// the entry lives, and the source bytes are still freed with the document.
pub(crate) struct MediaCache<T> {
    pub(crate) map: HashMap<String, Entry<T>>,
    pub(crate) bytes: usize,
}

pub(crate) struct Entry<T> {
    src: Weak<Vec<u8>>,
    val: T,
    size: usize,
}

impl<T: Clone> MediaCache<T> {
    pub(crate) fn get(&self, key: &str, src: &Arc<Vec<u8>>) -> Option<T> {
        let e = self.map.get(key)?;
        std::ptr::eq(e.src.as_ptr(), Arc::as_ptr(src)).then(|| e.val.clone())
    }

    /// Stores `val` (about `size` bytes) for `src`. Clears the cache first when the budget or entry
    /// cap is hit; an entry larger than the whole budget is not kept.
    pub(crate) fn insert(&mut self, key: &str, src: &Arc<Vec<u8>>, val: T, size: usize) {
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

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
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
pub(crate) fn decode_pixmap_within(bytes: &[u8], budget: u64) -> Option<Pixmap> {
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
pub(crate) fn premul_pixmap(w: u32, h: u32, rgba: &[u8]) -> Option<Pixmap> {
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
pub(crate) struct Vector {
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
pub(crate) fn vector_for(doc: &Document, key: &str) -> Option<Arc<Vector>> {
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
pub(crate) fn draw_vector(ctx: &mut RenderContext, v: &Vector, r: &kurbo::Rect, crop: &[f32; 4], alpha: f32, view: Affine) {
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
pub(crate) static OVERSIZED_GLYPHS_NOTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Logs that `n` glyphs of a metafile text run were left out for being larger than
/// [`MAX_GLYPH_PIXELS`] (or of no finite size), the first time any are this run (`noted`): a
/// page is drawn again on every repaint, so a warning each time would flood the log. True when
/// this call logged it.
pub(crate) fn note_oversized_glyphs(noted: &std::sync::atomic::AtomicBool, n: usize) -> bool {
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
pub(crate) fn draw_page_picture(ctx: &mut RenderContext, doc: &Document, page: &Page, view: Affine) {
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

pub(crate) fn pixmap_for(doc: &Document, key: &str) -> Option<Arc<Pixmap>> {
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
pub(crate) fn cacheable(pm: Pixmap) -> Pixmap {
    let bytes = pm.width() as usize * pm.height() as usize * 4;
    if bytes <= CACHE_BYTES {
        return pm;
    }
    shrink(&pm, (CACHE_BYTES / 2 / 4) as u64).unwrap_or(pm)
}

/// `pm` averaged over square blocks, the smallest whole number of pixels a side that brings it to
/// at most `max_pixels`; None when it already fits or none are allowed. Premultiplied pixels
/// average without colour fringes.
pub(crate) fn shrink(pm: &Pixmap, max_pixels: u64) -> Option<Pixmap> {
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
