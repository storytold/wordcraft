//! WordCraft rasteriser: draws a page's display list with vello_cpu.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use vello_cpu::kurbo::{self, Affine, BezPath, Shape};
use vello_cpu::peniko;
use vello_cpu::{Pixmap, RenderContext, Resources};
use wordcraft_doc::Document;
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::Rgb;
use wordcraft_fonts::FontDb;
use wordcraft_layout::Page;
use wordcraft_layout::display::{DisplayOptions, Draw, Stroke, page_display};
use wordcraft_metafile::{Item, Picture, PlacedItem, Seg};

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
    /// PNG bytes (empty on failure, logged).
    pub fn to_png(&self) -> Vec<u8> {
        let mut px = self.to_straight();
        px.resize(self.width as usize * self.height as usize * 4, 0);
        let Some(img) = image::RgbaImage::from_raw(self.width, self.height, px) else {
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
    pub fn to_jpeg(&self, quality: u8) -> Vec<u8> {
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

fn color(c: Rgb, alpha: f32) -> peniko::Color {
    peniko::Color::from_rgba8(c.0, c.1, c.2, (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Upper bound on the bytes the two decoded-media caches keep together (about 256 MB), half each;
/// past its half a cache starts over.
const CACHE_BYTES: usize = 256 * 1024 * 1024 / 2;
/// Upper bound on cached entries per cache.
const CACHE_ENTRIES: usize = 256;

/// Decoded media by media key, shared across renders, with an approximate byte budget. An entry holds
/// the source bytes it was decoded from, so a hit needs the same allocation (`Arc::ptr_eq`), and that
/// allocation cannot be freed and its address reused while the entry lives.
struct MediaCache<T> {
    map: HashMap<String, Entry<T>>,
    bytes: usize,
}

struct Entry<T> {
    src: Arc<Vec<u8>>,
    val: T,
    size: usize,
}

impl<T: Clone> MediaCache<T> {
    fn get(&self, key: &str, src: &Arc<Vec<u8>>) -> Option<T> {
        let e = self.map.get(key)?;
        Arc::ptr_eq(&e.src, src).then(|| e.val.clone())
    }

    /// Stores `val` (about `size` bytes) with its source, whose bytes the entry keeps alive and so counts
    /// too. Clears the cache first when the budget or entry cap is hit; an entry larger than the whole
    /// budget is not kept.
    fn insert(&mut self, key: &str, src: &Arc<Vec<u8>>, val: T, size: usize) {
        let size = size.saturating_add(src.len());
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
        self.map.insert(key.to_string(), Entry { src: src.clone(), val, size });
        self.bytes = self.bytes.saturating_add(size);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn images() -> &'static Mutex<MediaCache<Option<Arc<Pixmap>>>> {
    static C: std::sync::OnceLock<Mutex<MediaCache<Option<Arc<Pixmap>>>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(MediaCache { map: HashMap::new(), bytes: 0 }))
}

fn vectors() -> &'static Mutex<MediaCache<Option<Arc<Vector>>>> {
    static C: std::sync::OnceLock<Mutex<MediaCache<Option<Arc<Vector>>>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(MediaCache { map: HashMap::new(), bytes: 0 }))
}

/// Decode an encoded image to a premultiplied pixmap.
pub fn decode_pixmap(bytes: &[u8]) -> Option<Pixmap> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
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
        }
    }
    ctx.set_fill_rule(peniko::Fill::NonZero);
    ctx.pop_layer();
    if faded {
        ctx.pop_layer();
    }
}

/// One device pixel in page points under `view` (its mean axis scale); 1 for a degenerate transform.
fn device_pixel(view: Affine) -> f64 {
    let c = view.as_coeffs();
    let scale = (c[0].hypot(c[1]) + c[2].hypot(c[3])) / 2.0;
    if scale.is_finite() && scale > 1e-9 { 1.0 / scale } else { 1.0 }
}

fn pixmap_for(doc: &Document, key: &str) -> Option<Arc<Pixmap>> {
    let bytes = doc.media.get(key)?;
    if let Some(v) = lock(images()).get(key, bytes) {
        return v;
    }
    let pm = decode_pixmap(bytes).map(Arc::new);
    let size = pm.as_ref().map_or(0, |p| p.width() as usize * p.height() as usize * 4);
    lock(images()).insert(key, bytes, pm.clone(), size);
    pm
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
}

/// Default grey for a dark page's paper.
pub const DARK_PAPER: u8 = 0x33;

impl RenderOptions {
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
    let (w16, h16) = (w.clamp(1, MAX_SIDE) as u16, h.clamp(1, MAX_SIDE) as u16);
    let mut ctx = RenderContext::new_with(w16, h16, vello_cpu::RenderSettings { num_threads: default_threads(), ..Default::default() });
    ctx.set_transform(Affine::IDENTITY);
    ctx.set_paint(color(opts.ink(doc.settings.page_color.unwrap_or(opts.paper)), 1.0));
    ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w16 as f64, h16 as f64));
    let items = page_display(doc, page, &opts.display);
    let visible = view.inverse().transform_rect_bbox(kurbo::Rect::new(0.0, 0.0, w16 as f64, h16 as f64)).inflate(40.0, 40.0);
    if let Some(wm) = &doc.settings.watermark {
        draw_watermark(&mut ctx, view, page, wm, opts);
    }
    for it in &items {
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

fn draw_watermark(ctx: &mut RenderContext, view: Affine, page: &Page, wm: &wordcraft_doc::Watermark, opts: &RenderOptions) {
    let r = wordcraft_fonts::word::resolve(&wm.font, false, false);
    let face = r.face;
    let glyphs = wordcraft_fonts::shape(&face, &wm.text, &[], |c| c);
    let upem = face.upem.max(1.0);
    let raw_w: f64 = glyphs.iter().map(|g| g.x_advance as f64).sum::<f64>() / upem;
    if raw_w <= 0.0 {
        return;
    }
    let diag = if wm.diagonal { ((page.w as f64).powi(2) + (page.h.min(2000.0) as f64).powi(2)).sqrt() } else { page.w as f64 };
    let size = (diag * 0.5 / raw_w).min(200.0);
    let k = size / upem;
    let angle = if wm.diagonal { -((page.h.min(2000.0) as f64) / page.w.max(1.0) as f64).atan() } else { 0.0 };
    let base = Affine::translate((page.w as f64 / 2.0, page.h.min(2000.0) as f64 / 2.0))
        * Affine::rotate(angle)
        * Affine::translate((-raw_w * size / 2.0, size * 0.35));
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

fn draw(ctx: &mut RenderContext, doc: &Document, it: &Draw, view: Affine, visible: &kurbo::Rect, opts: &RenderOptions) {
    match it {
        Draw::Figure { draws, .. } => {
            for d in draws {
                draw(ctx, doc, d, view, visible, opts);
            }
        }
        Draw::Fill { rect, color: c, alpha } => {
            let r = kurbo::Rect::new(rect.x as f64, rect.y as f64, rect.right() as f64, rect.bottom() as f64);
            if !r.overlaps(*visible) {
                return;
            }
            ctx.set_transform(view);
            ctx.set_paint(color(opts.ink(*c), *alpha));
            ctx.fill_rect(&r);
        }
        Draw::Line { x0, y0, x1, y1, width, color: c, stroke, alpha } => {
            ctx.set_transform(view);
            ctx.set_paint(color(opts.ink(*c), *alpha));
            let w = *width as f64;
            let mut st = kurbo::Stroke::new(w);
            match stroke {
                Stroke::Dotted => st = st.with_dashes(0.0, [w, w * 2.0]),
                Stroke::Dashed => st = st.with_dashes(0.0, [w * 4.0, w * 2.0]),
                _ => {}
            }
            let line = |ctx: &mut RenderContext, dy: f64| {
                let mut p = BezPath::new();
                p.move_to((*x0 as f64, *y0 as f64 + dy));
                p.line_to((*x1 as f64, *y1 as f64 + dy));
                ctx.stroke_path(&p);
            };
            if *stroke == Stroke::Wave {
                let mut p = BezPath::new();
                let (a, b) = ((*x0).min(*x1) as f64, (*x0).max(*x1) as f64);
                let y = *y0 as f64;
                let amp = (w * 1.2).max(0.8);
                let step = amp * 2.0;
                p.move_to((a, y));
                let mut x = a;
                let mut up = true;
                while x < b && x - a < 20_000.0 {
                    let nx = (x + step).min(b);
                    p.quad_to(((x + nx) / 2.0, if up { y - amp } else { y + amp }), (nx, y));
                    x = nx;
                    up = !up;
                }
                ctx.set_stroke(kurbo::Stroke::new(w * 0.8));
                ctx.stroke_path(&p);
                return;
            }
            if *stroke == Stroke::Double {
                ctx.set_stroke(kurbo::Stroke::new(w * 0.6));
                line(ctx, -w * 0.9);
                line(ctx, w * 0.9);
                return;
            }
            ctx.set_stroke(st);
            line(ctx, 0.0);
        }
        Draw::Glyphs { face, size, glyphs, color: c, alpha, synth_bold, synth_italic, .. } => {
            let db = FontDb::global();
            let k = *size as f64 / face.upem.max(1.0);
            ctx.set_paint(color(opts.ink(*c), *alpha));
            let skew = if *synth_italic { Affine::new([1.0, 0.0, -0.21, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
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
            for (gid, x, y) in glyphs {
                let (gx, gy) = (*x as f64, *y as f64);
                // Written so a NaN position is skipped too.
                let near = gx >= visible.x0 - 100.0 && gx <= visible.x1 && gy >= visible.y0 && gy <= visible.y1 + 200.0;
                if !near {
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
        Draw::Image { rect, media, crop, alpha } => {
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
            let [cl, ct, cr, cb] = crop.map(|v| if v.is_finite() { v.clamp(0.0, 0.95) as f64 } else { 0.0 });
            let vis_w = (1.0 - cl - cr).max(0.05);
            let vis_h = (1.0 - ct - cb).max(0.05);
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
        Draw::Path { segs, fill, stroke, stroke_width } => {
            let path = wordcraft_layout::display::seg_path(segs);
            if path.elements().is_empty() {
                return;
            }
            ctx.set_transform(view);
            if let Some(f) = fill {
                ctx.set_paint(color(opts.ink(*f), 1.0));
                ctx.fill_path(&path);
            }
            if let Some(s) = stroke {
                // Width 0 (or none) is a hairline.
                let w = if stroke_width.is_finite() && *stroke_width > 0.0 { stroke_width.clamp(0.25, 200.0) } else { 0.75 };
                ctx.set_paint(color(opts.ink(*s), 1.0));
                ctx.set_stroke(kurbo::Stroke::new(w as f64));
                ctx.stroke_path(&path);
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
        Draw::Shape { rect, kind, fill, stroke, stroke_width, effects } => {
            let r = kurbo::Rect::new(rect.x as f64, rect.y as f64, rect.right() as f64, rect.bottom() as f64);
            let path = shape_path(*kind, r);
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
                            ctx.fill_path(&shape_path(*kind, rr));
                        }
                    } else if sw + 2.0 * g > 0.05 {
                        ctx.set_stroke(kurbo::Stroke::new(sw + 2.0 * g));
                        ctx.stroke_path(&shape_path(*kind, r + d));
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
                        ctx.set_paint(color(opts.ink(*f), a));
                        ctx.fill_path(&shape_path(*kind, rr));
                    }
                }
                return;
            }
            if let Some(f) = fill {
                ctx.set_paint(color(opts.ink(*f), 1.0));
                ctx.fill_path(&path);
            }
            if let Some(s) = stroke {
                ctx.set_paint(color(opts.ink(*s), 1.0));
                ctx.set_stroke(kurbo::Stroke::new(stroke_width.max(0.25) as f64));
                ctx.stroke_path(&path);
            }
        }
        Draw::Turned { x, y, turn, items } => {
            let m = Affine::new(Draw::turn_matrix(*turn, *x, *y).map(f64::from));
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
    let first = pts.first()?;
    if p.elements().len() == 1 {
        // A tap: a zero-length line, which round caps draw as a dot.
        p.line_to((first.0 as f64, first.1 as f64));
    }
    last.map(|_| p)
}

/// Outline of a basic shape in a rectangle.
pub fn shape_path(kind: ShapeKind, r: kurbo::Rect) -> BezPath {
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
        ShapeKind::RoundedRectangle => kurbo::RoundedRect::from_rect(r, r.width().min(r.height()) * 0.16).to_path(0.1),
        ShapeKind::Ellipse => kurbo::Ellipse::from_rect(r).to_path(0.1),
        ShapeKind::Triangle => poly(&[(cx, r.y0), (r.x1, r.y1), (r.x0, r.y1)]),
        ShapeKind::Diamond => poly(&[(cx, r.y0), (r.x1, cy), (cx, r.y1), (r.x0, cy)]),
        ShapeKind::Line => {
            let mut p = BezPath::new();
            p.move_to((r.x0, r.y1));
            p.line_to((r.x1, r.y0));
            p
        }
        ShapeKind::Arrow => {
            let h = r.height();
            poly(&[
                (r.x0, cy - h * 0.2),
                (r.x1 - h * 0.5, cy - h * 0.2),
                (r.x1 - h * 0.5, r.y0),
                (r.x1, cy),
                (r.x1 - h * 0.5, r.y1),
                (r.x1 - h * 0.5, cy + h * 0.2),
                (r.x0, cy + h * 0.2),
            ])
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

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

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
                fill: Some(Rgb(0, 0, 255)),
                stroke: None,
                stroke_width: 0.0,
                float: Default::default(),
                story: None,
                freeform: None,
                effects,
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
        let shadow = Shadow { color: Rgb::BLACK, transparency: 30.0, blur: 2.0, distance: 8.0, angle: 0.0 };
        let img = draw(ShapeEffects { shadow: Some(shadow), ..Default::default() });
        assert_eq!(blue(&img), (x0, y0, x1, y1), "the shape itself is unchanged");
        assert!(right(&img) > 100, "shadow to the right: {}", right(&img));
        assert_eq!(left(&img), 0, "nothing on the left");
        let img = draw(ShapeEffects { glow: Some(Glow { color: Rgb(255, 0, 0), size: 6.0, transparency: 0.0 }), ..Default::default() });
        let reddish = |x: u32, y: u32| matches!(img.pixel(x, y), [r, g, _, _] if r > 200 && g < 200);
        assert!(reddish(x1 + 2, (y0 + y1) / 2) && reddish(x0 - 2, (y0 + y1) / 2), "glow on both sides");
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
    fn hostile_scale_is_clamped() {
        let d = Document::new();
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(&d, &l.pages[0], f32::NAN, &RenderOptions::default());
        assert_eq!(img.width, 612);
        let big = render_page(&d, &l.pages[0], 1e9, &RenderOptions::default());
        assert!(big.width <= MAX_SIDE);
        assert!(decode_pixmap(b"not an image").is_none());
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
        let obj = wordcraft_doc::para::InlineObject::Image { media, w: 40.0, h: 40.0, alt: String::new(), float: Default::default(), crop: [0.0; 4] };
        hp.insert_object(0, obj, &Default::default()).unwrap();
        let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
        d.last_section.headers.default = Some(id);
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let opts = RenderOptions::default();
        assert!(opts.display.dim_header);
        let img = render_page(&d, &l.pages[0], 1.0, &opts);
        assert!(!img.to_png().is_empty());
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
        let obj = wordcraft_doc::para::InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop: [0.0; 4] };
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

    #[test]
    fn cosmetic_metafile_pen_is_one_device_pixel_wide() {
        // MOVETO (0, 50), LINETO (100, 50) with the default cosmetic black pen: a hairline across the middle.
        let mut emf = red_right_half_emf()[..88].to_vec();
        emf.extend([emf_rec(27, &le32(&[0, 50])), emf_rec(54, &le32(&[100, 50])), emf_rec(14, &[0; 12])].concat());
        let mut d = Document::new();
        let media = d.add_media(emf, "emf");
        let mut p = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop: [0.0; 4] };
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

    #[test]
    fn unreadable_metafile_keeps_the_grey_box() {
        let mut d = Document::new();
        // The header alone has no records, so the metafile does not parse.
        let media = d.add_media(red_right_half_emf()[..88].to_vec(), "emf");
        let mut p = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop: [0.0; 4] };
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
        for k in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Star, ShapeKind::Heart, ShapeKind::Arrow, ShapeKind::Line] {
            assert!(!shape_path(k, r).elements().is_empty());
        }
    }
}

#[cfg(test)]
mod cache_tests;
