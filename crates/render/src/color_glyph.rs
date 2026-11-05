//! Colour glyphs (#1269): emoji and other glyphs whose font draws them from COLR paint graphs or
//! bitmap strikes (sbix, CBDT) instead of outlines. The fonts crate reads the colour data
//! ([`FontFace::color_glyph`]); this module decodes bitmaps, caches the result per glyph and size,
//! and turns it into vello fills. The colours come from the font: a dark page doesn't invert them,
//! the same as pictures. Only the font's "text colour" entries follow the run's colour.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use vello_cpu::kurbo::{self, Affine, Point, Rect, Shape};
use vello_cpu::peniko;
use vello_cpu::{Pixmap, RenderContext};
use wordcraft_fonts::FontFace;
use wordcraft_fonts::color::{BitmapBytes, ColorGlyph, ColorOp, ColorPaint, CompositeMode, Extend, GlyphColor, Stop};

/// A colour glyph ready to draw: COLR ops, or a decoded bitmap and where it goes (font units, y
/// down from the pen position).
pub(crate) enum Drawable {
    Layers(Vec<ColorOp>),
    Bitmap { pixmap: Arc<Pixmap>, rect: Rect },
}

impl Drawable {
    fn bytes(&self) -> usize {
        match self {
            Drawable::Layers(ops) => wordcraft_fonts::color::ops_bytes(ops),
            Drawable::Bitmap { pixmap, .. } => pixmap.width() as usize * pixmap.height() as usize * 4,
        }
    }
}

/// Upper bounds on the colour-glyph cache: entries and approximate bytes. Past either it starts over.
const CACHE_ENTRIES: usize = 4096;
const CACHE_BYTES: usize = 64 * 1024 * 1024;

type Key = (u32, u32, u32);

#[derive(Default)]
struct Cache {
    map: HashMap<Key, Option<Arc<Drawable>>>,
    bytes: usize,
}

fn cache() -> &'static Mutex<Cache> {
    static C: std::sync::OnceLock<Mutex<Cache>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache::default()))
}

/// The strike size asked for at `ppem` device pixels per em: the next power of two, so zooming
/// doesn't decode a bitmap per zoom step, and a strike at least as big as needed is preferred.
fn ppem_bucket(ppem: f64) -> u32 {
    let p = if ppem.is_finite() { ppem.clamp(1.0, 4096.0) } else { 64.0 };
    (p.ceil() as u32).next_power_of_two()
}

/// Glyph `gid` of `face` in colour at about `ppem` device pixels per em, None when it has no colour
/// data (draw its outline). `cached` keeps the result in the shared cache (keyed by face id); a face
/// read just for a preview passes false so nothing outlives it.
pub(crate) fn drawable(face: &FontFace, gid: u32, ppem: f64, cached: bool) -> Option<Arc<Drawable>> {
    let bucket = ppem_bucket(ppem);
    if !cached {
        return build(face, gid, bucket).map(Arc::new);
    }
    let key = (face.id(), gid, bucket);
    if let Some(v) = crate::lock(cache()).map.get(&key) {
        return v.clone();
    }
    let d = build(face, gid, bucket).map(Arc::new);
    let size = d.as_ref().map_or(0, |d| d.bytes()).saturating_add(64);
    let mut c = crate::lock(cache());
    if c.map.len() >= CACHE_ENTRIES || c.bytes.saturating_add(size) > CACHE_BYTES {
        c.map.clear();
        c.bytes = 0;
    }
    if size <= CACHE_BYTES {
        c.map.insert(key, d.clone());
        c.bytes = c.bytes.saturating_add(size);
    }
    d
}

fn build(face: &FontFace, gid: u32, ppem: u32) -> Option<Drawable> {
    match face.color_glyph(gid, ppem as f32)? {
        ColorGlyph::Layers(ops) => Some(Drawable::Layers(ops)),
        ColorGlyph::Bitmap(b) => {
            let pixmap = match &b.data {
                BitmapBytes::Png(png) => decode_png(png, b.width, b.height)?,
                BitmapBytes::Bgra(bgra) => bgra_pixmap(bgra, b.width, b.height)?,
            };
            Some(Drawable::Bitmap { pixmap: Arc::new(pixmap), rect: b.rect })
        }
    }
}

/// Decode a bitmap glyph's PNG, which may be at most the `w`×`h` its strike claims (and at most
/// [`MAX_BITMAP_SIDE`](wordcraft_fonts::color::MAX_BITMAP_SIDE) a side): a hostile font could claim
/// a small glyph and hand over a huge image, so the size is read from the header before decoding.
/// A smaller, non-empty image is drawn stretched to the glyph's box: CBDT-only fonts have no
/// outline to fall back to.
fn decode_png(png: &[u8], w: u32, h: u32) -> Option<Pixmap> {
    let side = wordcraft_fonts::color::MAX_BITMAP_SIDE;
    if w == 0 || h == 0 || w > side || h > side {
        return None;
    }
    let fits = |(iw, ih): (u32, u32)| iw > 0 && ih > 0 && iw <= w && ih <= h;
    if !fits(glyph_png_reader(png, w, h).into_dimensions().ok()?) {
        return None;
    }
    let img = glyph_png_reader(png, w, h).decode().ok()?.to_rgba8();
    let (iw, ih) = img.dimensions();
    if !fits((iw, ih)) {
        return None;
    }
    crate::premul_pixmap(iw, ih, img.as_raw())
}

/// A reader for a bitmap glyph's PNG with the glyph's budget: at most `w`×`h` and about two RGBA
/// buffers of it. Every read goes through it, the header probe too: reading the header inflates
/// metadata such as an ICC profile, which the default budget would let run to hundreds of MB.
fn glyph_png_reader(png: &[u8], w: u32, h: u32) -> image::ImageReader<std::io::Cursor<&[u8]>> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(w);
    limits.max_image_height = Some(h);
    limits.max_alloc = Some(u64::from(w).saturating_mul(u64::from(h)).saturating_mul(8).saturating_add(64 * 1024));
    let mut r = image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    r.limits(limits);
    r
}

/// A pixmap from premultiplied BGRA rows.
fn bgra_pixmap(bgra: &[u8], w: u32, h: u32) -> Option<Pixmap> {
    if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
        return None;
    }
    let n = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    let data: Vec<vello_cpu::color::PremulRgba8> = bgra
        .get(..n)?
        .as_chunks::<4>()
        .0
        .iter()
        // Premultiplied channels can't exceed alpha; clamp in case the font says otherwise.
        .map(|p| vello_cpu::color::PremulRgba8 { r: p[2].min(p[3]), g: p[1].min(p[3]), b: p[0].min(p[3]), a: p[3] })
        .collect();
    Some(Pixmap::from_parts(data, w as u16, h as u16))
}

/// Draw a colour glyph. `at` maps font units (y down, pen at the origin) to device pixels; `fg` is
/// the run's text colour for the font's "text colour" entries; `alpha` fades the whole glyph.
pub(crate) fn draw(ctx: &mut RenderContext, d: &Drawable, at: Affine, upem: f64, fg: peniko::Color, alpha: f32) {
    if !usable(at) {
        return;
    }
    let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
    let faded = alpha < 1.0;
    if faded {
        ctx.push_opacity_layer(alpha);
    }
    match d {
        Drawable::Bitmap { pixmap, rect } => {
            let (pw, ph) = (pixmap.width().max(1) as f64, pixmap.height().max(1) as f64);
            ctx.set_transform(at);
            ctx.set_paint(vello_cpu::Image {
                image: vello_cpu::ImageSource::Pixmap(pixmap.clone()),
                sampler: peniko::ImageSampler::default().with_quality(peniko::ImageQuality::Medium),
            });
            ctx.set_paint_transform(Affine::translate((rect.x0, rect.y0)) * Affine::scale_non_uniform(rect.width() / pw, rect.height() / ph));
            ctx.fill_rect(rect);
            ctx.reset_paint_transform();
        }
        Drawable::Layers(ops) => draw_ops(ctx, ops, at * Affine::FLIP_Y, upem, fg),
    }
    if faded {
        ctx.pop_layer();
    }
}

/// Play COLR ops (font units, y up) under `root`. Fills cover a generous box around the em square,
/// clipped by the glyph's clips. Layers left open by a malformed list are closed at the end.
fn draw_ops(ctx: &mut RenderContext, ops: &[ColorOp], root: Affine, upem: f64, fg: peniko::Color) {
    let upem = if upem.is_finite() && upem > 0.0 { upem } else { 1000.0 };
    let area = Rect::new(-upem, -upem, upem * 2.0, upem * 2.0);
    let mut transforms = vec![root];
    let mut open = 0usize;
    for op in ops {
        let cur = transforms.last().copied().unwrap_or(root);
        match op {
            ColorOp::PushTransform(t) => transforms.push(cur * *t),
            ColorOp::PopTransform => {
                if transforms.len() > 1 {
                    transforms.pop();
                }
            }
            ColorOp::PushClip(path) => {
                // A degenerate or non-finite transform clips everything away: push an empty clip so
                // the matching pop still has a layer to close.
                if usable(cur) {
                    ctx.set_transform(cur);
                    ctx.push_clip_layer(path);
                } else {
                    ctx.set_transform(root);
                    ctx.push_clip_layer(&kurbo::BezPath::new());
                }
                open += 1;
            }
            ColorOp::PushLayer(mode) => {
                ctx.push_blend_layer(blend_mode(*mode));
                open += 1;
            }
            ColorOp::PopClip | ColorOp::PopLayer => {
                if open > 0 {
                    ctx.pop_layer();
                    open -= 1;
                }
            }
            ColorOp::Fill(paint) => fill(ctx, paint, cur, root, area, fg),
        }
    }
    for _ in 0..open {
        ctx.pop_layer();
    }
}

/// Fill `area` (in root space) with `paint` defined in `cur` space.
fn fill(ctx: &mut RenderContext, paint: &ColorPaint, cur: Affine, root: Affine, area: Rect, fg: peniko::Color) {
    if !usable(cur) {
        return;
    }
    let region = (cur.inverse() * root) * area.to_path(0.1);
    let solid = |ctx: &mut RenderContext, c: GlyphColor| {
        ctx.set_transform(cur);
        ctx.set_paint(resolve(c, fg));
        ctx.fill_path(&region);
    };
    let (gradient, stops, extend, transform) = match paint {
        ColorPaint::Solid(c) => return solid(ctx, *c),
        ColorPaint::Linear { p0, p1, stops, extend } => {
            if !finite(&[p0.x, p0.y, p1.x, p1.y]) {
                return;
            }
            (peniko::Gradient::new_linear(*p0, *p1), stops, extend, cur)
        }
        ColorPaint::Radial { c0, r0, c1, r1, stops, extend } => {
            if !finite(&[c0.x, c0.y, c1.x, c1.y, *r0 as f64, *r1 as f64]) || *r1 <= 0.0 {
                if let Some(s) = stops.last() {
                    solid(ctx, s.color);
                }
                return;
            }
            (peniko::Gradient::new_two_point_radial(*c0, r0.max(0.0), *c1, *r1), stops, extend, cur)
        }
        ColorPaint::Sweep { c0, start_angle, end_angle, stops, extend } => {
            if !finite(&[c0.x, c0.y, *start_angle as f64, *end_angle as f64]) {
                return;
            }
            let mut end = *end_angle;
            if *start_angle == end {
                if *extend != Extend::Pad {
                    return;
                }
                // A sweep needs distinct angles.
                end += 0.01;
            }
            // Peniko sweeps clockwise in y-down space; COLR's (as skrifa reports them) are in this
            // y-up space, so sweep in a flipped frame.
            let g = peniko::Gradient::new_sweep(Point::new(c0.x, -c0.y), start_angle.to_radians(), end.to_radians());
            (g, stops, extend, cur * Affine::FLIP_Y)
        }
    };
    match stops.as_slice() {
        [] => {}
        [only] => solid(ctx, only.color),
        _ => {
            let stops: Vec<peniko::ColorStop> = stops.iter().map(|s: &Stop| (s.offset.clamp(-1e6, 1e6), resolve(s.color, fg)).into()).collect();
            let g = gradient.with_stops(stops.as_slice()).with_extend(peniko_extend(*extend));
            ctx.set_transform(transform);
            ctx.set_paint(g);
            let region = transform.inverse() * root * area.to_path(0.1);
            ctx.fill_path(&region);
        }
    }
}

/// Can a transform be drawn under (finite and invertible)?
fn usable(t: Affine) -> bool {
    t.as_coeffs().iter().all(|v| v.is_finite()) && t.determinant().abs() >= 1e-12
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() < 1e7)
}

fn resolve(c: GlyphColor, fg: peniko::Color) -> peniko::Color {
    match c {
        GlyphColor::Rgba([r, g, b, a]) => peniko::Color::from_rgba8(r, g, b, a),
        GlyphColor::Foreground(a) => fg.multiply_alpha(if a.is_finite() { a.clamp(0.0, 1.0) } else { 1.0 }),
    }
}

fn peniko_extend(e: Extend) -> peniko::Extend {
    match e {
        Extend::Repeat => peniko::Extend::Repeat,
        Extend::Reflect => peniko::Extend::Reflect,
        _ => peniko::Extend::Pad,
    }
}

fn blend_mode(m: CompositeMode) -> peniko::BlendMode {
    use peniko::{Compose as C, Mix as M};
    let (mix, compose) = match m {
        CompositeMode::Clear => (M::Normal, C::Clear),
        CompositeMode::Src => (M::Normal, C::Copy),
        CompositeMode::Dest => (M::Normal, C::Dest),
        CompositeMode::DestOver => (M::Normal, C::DestOver),
        CompositeMode::SrcIn => (M::Normal, C::SrcIn),
        CompositeMode::DestIn => (M::Normal, C::DestIn),
        CompositeMode::SrcOut => (M::Normal, C::SrcOut),
        CompositeMode::DestOut => (M::Normal, C::DestOut),
        CompositeMode::SrcAtop => (M::Normal, C::SrcAtop),
        CompositeMode::DestAtop => (M::Normal, C::DestAtop),
        CompositeMode::Xor => (M::Normal, C::Xor),
        CompositeMode::Plus => (M::Normal, C::Plus),
        CompositeMode::Screen => (M::Screen, C::SrcOver),
        CompositeMode::Overlay => (M::Overlay, C::SrcOver),
        CompositeMode::Darken => (M::Darken, C::SrcOver),
        CompositeMode::Lighten => (M::Lighten, C::SrcOver),
        CompositeMode::ColorDodge => (M::ColorDodge, C::SrcOver),
        CompositeMode::ColorBurn => (M::ColorBurn, C::SrcOver),
        CompositeMode::HardLight => (M::HardLight, C::SrcOver),
        CompositeMode::SoftLight => (M::SoftLight, C::SrcOver),
        CompositeMode::Difference => (M::Difference, C::SrcOver),
        CompositeMode::Exclusion => (M::Exclusion, C::SrcOver),
        CompositeMode::Multiply => (M::Multiply, C::SrcOver),
        CompositeMode::HslHue => (M::Hue, C::SrcOver),
        CompositeMode::HslSaturation => (M::Saturation, C::SrcOver),
        CompositeMode::HslColor => (M::Color, C::SrcOver),
        CompositeMode::HslLuminosity => (M::Luminosity, C::SrcOver),
        _ => (M::Normal, C::SrcOver),
    };
    peniko::BlendMode::new(mix, compose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_fonts::color::{GlyphColor, Stop};

    fn render(ops: &[ColorOp]) -> crate::Rendered {
        let mut ctx = RenderContext::new(40, 40);
        ctx.set_paint(peniko::Color::WHITE);
        ctx.fill_rect(&Rect::new(0.0, 0.0, 40.0, 40.0));
        let d = Drawable::Layers(ops.to_vec());
        // 1000 upem → 20 px, pen at (10, 30).
        draw(&mut ctx, &d, Affine::translate((10.0, 30.0)) * Affine::scale(0.02), 1000.0, peniko::Color::from_rgba8(0, 128, 0, 255), 1.0);
        ctx.flush();
        let mut pixels = vec![0u8; 40 * 40 * 4];
        if let Some(pm) = vello_cpu::PixmapMut::new(40, 40, &mut pixels) {
            ctx.render(pm, &mut vello_cpu::Resources::new());
        }
        crate::Rendered { width: 40, height: 40, pixels }
    }

    fn square() -> kurbo::BezPath {
        Rect::new(0.0, 0.0, 1000.0, 1000.0).to_path(0.1)
    }

    /// A clipped solid fill lands inside the glyph's em box above the baseline, in the palette
    /// colour; the text-colour entry follows the run's colour.
    #[test]
    fn colr_ops_fill_the_clip_in_palette_colours() {
        let red = GlyphColor::Rgba([255, 0, 0, 255]);
        let img = render(&[ColorOp::PushClip(square()), ColorOp::Fill(ColorPaint::Solid(red)), ColorOp::PopClip]);
        assert_eq!(img.pixel(20, 20), [255, 0, 0, 255], "inside the em box");
        assert_eq!(img.pixel(20, 35), [255, 255, 255, 255], "below the baseline");
        assert_eq!(img.pixel(5, 20), [255, 255, 255, 255], "left of the pen");
        let img = render(&[ColorOp::PushClip(square()), ColorOp::Fill(ColorPaint::Solid(GlyphColor::Foreground(1.0))), ColorOp::PopClip]);
        assert_eq!(img.pixel(20, 20), [0, 128, 0, 255]);
    }

    #[test]
    fn colr_gradients_paint_and_hostile_ops_never_panic() {
        let stops =
            vec![Stop { offset: 0.0, color: GlyphColor::Rgba([255, 0, 0, 255]) }, Stop { offset: 1.0, color: GlyphColor::Rgba([0, 0, 255, 255]) }];
        let linear = ColorPaint::Linear { p0: Point::new(0.0, 0.0), p1: Point::new(1000.0, 0.0), stops: stops.clone(), extend: Extend::Pad };
        let img = render(&[ColorOp::PushClip(square()), ColorOp::Fill(linear), ColorOp::PopClip]);
        let (l, r) = (img.pixel(11, 20), img.pixel(29, 20));
        assert!(l[0] > 200 && l[2] < 60, "red at the start: {l:?}");
        assert!(r[2] > 200 && r[0] < 60, "blue at the end: {r:?}");
        let c = Point::new(500.0, 500.0);
        let hostile = [
            ColorOp::PopClip,
            ColorOp::PopTransform,
            ColorOp::PushTransform(Affine::scale(0.0)),
            ColorOp::Fill(ColorPaint::Solid(GlyphColor::Foreground(f32::NAN))),
            ColorOp::PopTransform,
            ColorOp::PushTransform(Affine::new([f64::NAN; 6])),
            ColorOp::Fill(ColorPaint::Radial { c0: c, r0: -5.0, c1: c, r1: f32::INFINITY, stops: stops.clone(), extend: Extend::Pad }),
            ColorOp::PopTransform,
            ColorOp::PushLayer(CompositeMode::Multiply),
            ColorOp::Fill(ColorPaint::Sweep { c0: c, start_angle: 0.0, end_angle: 0.0, stops: stops.clone(), extend: Extend::Pad }),
            ColorOp::Fill(ColorPaint::Sweep { c0: c, start_angle: 0.0, end_angle: 0.0, stops, extend: Extend::Repeat }),
            ColorOp::Fill(ColorPaint::Linear { p0: c, p1: c, stops: vec![], extend: Extend::Pad }),
            ColorOp::PushTransform(Affine::new([f64::NAN; 6])),
            ColorOp::PushClip(square()),
            ColorOp::Fill(ColorPaint::Solid(GlyphColor::Rgba([255, 0, 0, 255]))),
            ColorOp::PopClip,
            ColorOp::PopTransform,
            ColorOp::PushTransform(Affine::scale(0.0)),
            ColorOp::PushClip(square()),
            ColorOp::PopClip,
            ColorOp::PopTransform,
            ColorOp::PushClip(square()),
        ];
        let _ = render(&hostile);
    }

    #[test]
    fn bgra_bitmaps_are_swizzled_and_short_data_refused() {
        let pm = bgra_pixmap(&[10, 20, 30, 255, 0, 0, 0, 0], 2, 1).unwrap();
        assert_eq!(pm.width(), 2);
        assert!(bgra_pixmap(&[1, 2, 3], 1, 1).is_none());
        assert!(decode_png(b"not a png", 4, 4).is_none());
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![];
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A bitmap glyph's PNG may not be bigger than its strike claims: a bigger one is refused before
    /// it is decoded, so a hostile font can't claim a tiny glyph and make us decode a huge image. A
    /// smaller one is drawn (CBDT-only fonts have no outline to fall back to).
    #[test]
    fn bitmap_png_larger_than_its_claimed_size_is_refused() {
        assert_eq!(decode_png(&png(4, 4), 4, 4).map(|p| (p.width(), p.height())), Some((4, 4)));
        assert_eq!(decode_png(&png(2, 2), 4, 4).map(|p| (p.width(), p.height())), Some((2, 2)), "smaller than claimed");
        assert!(decode_png(&png(8, 8), 4, 4).is_none(), "wider and taller than claimed");
        assert!(decode_png(&png(8, 4), 4, 4).is_none(), "wider than claimed");
        assert!(decode_png(&png(4, 8), 4, 4).is_none(), "taller than claimed");
        let side = wordcraft_fonts::color::MAX_BITMAP_SIDE;
        assert!(decode_png(&png(4, 4), side + 1, 4).is_none(), "past the cap");
    }

    /// A tiny bitmap glyph whose PNG carries a big ICC profile (it compresses to almost nothing):
    /// the header probe already inflates it, so the probe is held to the glyph's budget too, not
    /// the decoder's default hundreds of MB. Past the budget the profile is dropped (the glyph is
    /// still drawn, without it).
    #[test]
    fn bitmap_png_header_probe_keeps_the_glyph_budget() {
        use image::ImageDecoder;
        let mut out = vec![];
        let mut enc = image::codecs::png::PngEncoder::new(&mut out);
        image::ImageEncoder::set_icc_profile(&mut enc, vec![0u8; 4 << 20]).unwrap();
        image::ImageEncoder::write_image(enc, &[200, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8).unwrap();
        assert!(out.len() < 64 * 1024, "the profile compresses: {} bytes", out.len());
        let profile = |r: image::ImageReader<std::io::Cursor<&[u8]>>| r.into_decoder().unwrap().icc_profile().unwrap().map(|p| p.len());
        let unbounded = image::ImageReader::with_format(std::io::Cursor::new(out.as_slice()), image::ImageFormat::Png);
        assert_eq!(profile(unbounded), Some(4 << 20), "the default budget inflates the 4 MB profile");
        assert_eq!(profile(glyph_png_reader(&out, 1, 1)), None, "the glyph budget stops it at the header");
        assert_eq!(decode_png(&out, 1, 1).map(|p| (p.width(), p.height())), Some((1, 1)));
        assert_eq!(glyph_png_reader(&png(4, 4), 4, 4).into_dimensions().ok(), Some((4, 4)), "a plain glyph reads");
    }
}
