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

mod media;

#[cfg(test)]
use media::{CACHE_BYTES, MediaCache, OVERSIZED_GLYPHS_NOTED, cacheable, decode_pixmap_within, note_oversized_glyphs, shrink};
pub use media::{decode_pixmap, image_size, metafile_size};
use media::{draw_page_picture, draw_vector, lock, pixmap_for, premul_pixmap, vector_for};

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
        draw_watermark(&mut ctx, view, page, wm, opts);
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

fn draw_watermark(ctx: &mut RenderContext, view: Affine, page: &Page, wm: &wordcraft_doc::Watermark, opts: &RenderOptions) {
    let r = wordcraft_fonts::word::resolve(&wm.font, false, false);
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

/// Where and how a display item is rasterized: `view` maps its current point coordinates to
/// pixels, and `visible` is a culling area in those point coordinates.
struct Frame<'a> {
    doc: &'a Document,
    opts: &'a RenderOptions,
    view: Affine,
    visible: &'a kurbo::Rect,
}

/// The rectangle in kurbo's terms.
fn kurbo_rect(rect: &wordcraft_geom::Rect) -> kurbo::Rect {
    kurbo::Rect::new(rect.x as f64, rect.y as f64, rect.right() as f64, rect.bottom() as f64)
}

/// Rasterize one display item with `view` mapping its current point coordinates to pixels.
/// `visible` is a culling area in those point coordinates; nested clips intersect it and turns
/// transform it into the child's coordinates. Shape and path strokes apply their stored caps,
/// while ink uses its tool's round ends and joins.
fn draw(ctx: &mut RenderContext, doc: &Document, it: &Draw, view: Affine, visible: &kurbo::Rect, opts: &RenderOptions) {
    let fr = Frame { doc, opts, view, visible };
    match it {
        Draw::Figure { draws, .. } => {
            for d in draws {
                draw(ctx, doc, d, view, visible, opts);
            }
        }
        Draw::Fill { .. } => draw_fill(ctx, &fr, it),
        Draw::Line { .. } => draw_line(ctx, &fr, it),
        Draw::Glyphs { .. } => draw_glyphs(ctx, &fr, it),
        Draw::Mark { .. } => draw_mark(ctx, &fr, it),
        Draw::MarkText { .. } => draw_mark_text(ctx, &fr, it),
        Draw::Image { .. } => draw_image(ctx, &fr, it),
        Draw::Path { .. } => draw_path(ctx, &fr, it),
        Draw::Art { .. } => draw_art(ctx, &fr, it),
        Draw::Ink { .. } => draw_ink(ctx, &fr, it),
        Draw::Shape { .. } => draw_shape(ctx, &fr, it),
        Draw::Clip { rect, items } => {
            let r = kurbo_rect(rect);
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

fn draw_fill(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Fill { rect, color: c, alpha, text_bg } = it else { return };
    let r = kurbo_rect(rect);
    if !r.overlaps(*fr.visible) {
        return;
    }
    ctx.set_transform(fr.view);
    ctx.set_paint(color(if *text_bg { fr.opts.ink_bg(*c) } else { fr.opts.ink(*c) }, *alpha));
    ctx.fill_rect(&r);
}

fn draw_line(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Line { x0, y0, x1, y1, width, color: c, stroke, alpha, text_bg } = it else { return };
    ctx.set_transform(fr.view);
    ctx.set_paint(color(fr.opts.ink_text(*c, *text_bg), *alpha));
    let w = *width as f64;
    let (x0, y0, x1, y1) = (*x0 as f64, *y0 as f64, *x1 as f64, *y1 as f64);
    // Flat ends, like the PDF export and Word: a round cap sticks half the width out past
    // every end, blobbing over the neighbouring border at a table's corners (#2892). Wavy
    // underlines keep round ends, as the PDF draws them.
    let mut st = kurbo::Stroke::new(w).with_caps(kurbo::Cap::Butt);
    if let Some(d) = stroke.dashes() {
        st = st.with_dashes(0.0, safe_dashes(d.iter().map(|k| f64::from(*k) * w), (x1 - x0).hypot(y1 - y0)));
    }
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    match *stroke {
        Stroke::Wave => {
            let amp = (w * 1.2).max(0.8);
            ctx.set_stroke(kurbo::Stroke::new(w * 0.8));
            ctx.stroke_path(&wave_path(x0, y0, x1, y1, amp));
        }
        // Three thin lines, spaced as a double's, across the line's direction (so a vertical
        // border's lines stand side by side).
        Stroke::Triple => {
            let (nx, ny) = (-dy / len * w * 1.8, dx / len * w * 1.8);
            ctx.set_stroke(kurbo::Stroke::new(w * 0.6));
            for k in [-1.0, 0.0, 1.0] {
                stroke_segment(ctx, (x0, y0, x1, y1), (nx * k, ny * k));
            }
        }
        Stroke::Double => {
            let (nx, ny) = (-dy / len * w * 0.9, dx / len * w * 0.9);
            ctx.set_stroke(kurbo::Stroke::new(w * 0.6).with_caps(kurbo::Cap::Butt));
            stroke_segment(ctx, (x0, y0, x1, y1), (nx, ny));
            stroke_segment(ctx, (x0, y0, x1, y1), (-nx, -ny));
        }
        _ => {
            ctx.set_stroke(st);
            stroke_segment(ctx, (x0, y0, x1, y1), (0.0, 0.0));
        }
    }
}

/// Stroke the line from (`x0`, `y0`) to (`x1`, `y1`), moved by (`ox`, `oy`).
fn stroke_segment(ctx: &mut RenderContext, (x0, y0, x1, y1): (f64, f64, f64, f64), (ox, oy): (f64, f64)) {
    let mut p = BezPath::new();
    p.move_to((x0 + ox, y0 + oy));
    p.line_to((x1 + ox, y1 + oy));
    ctx.stroke_path(&p);
}

fn draw_glyphs(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Glyphs { face, size, glyphs, color: c, alpha, synth_bold, synth_italic, text_bg, x_scale, .. } = it else { return };
    // Invisible glyphs (WordArt letters are drawn from their outlines) carry text only.
    if *alpha <= 0.0 {
        return;
    }
    let (view, visible, opts) = (fr.view, fr.visible, fr.opts);
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

fn draw_mark(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Mark { x, baseline, size, ch, color: mark } = it else { return };
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
    ctx.set_paint(color(fr.opts.ink(mark.unwrap_or(fr.opts.mark_color)), 0.9));
    ctx.set_transform(fr.view * Affine::translate((*x as f64, *baseline as f64)) * Affine::scale(k));
    ctx.fill_path(&o);
}

fn draw_mark_text(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::MarkText { x, baseline, size, text } = it else { return };
    let base_face = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    ctx.set_paint(color(fr.opts.ink(fr.opts.mark_color), 0.9));
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
        ctx.set_transform(fr.view * Affine::translate((gx, *baseline as f64)) * Affine::scale(k));
        ctx.fill_path(&o);
        gx += face.advance(gid) * k;
    }
}

fn draw_image(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Image { rect, media, crop, alpha, .. } = it else { return };
    let (doc, view) = (fr.doc, fr.view);
    let r = kurbo_rect(rect);
    if !r.overlaps(*fr.visible) {
        return;
    }
    if let Some(v) = vector_for(doc, media) {
        draw_vector(ctx, &v, &r, crop, *alpha, view);
        return;
    }
    let Some(pm) = pixmap_for(doc, media) else {
        ctx.set_transform(view);
        ctx.set_paint(color(fr.opts.ink(Rgb(0xD0, 0xD0, 0xD0)), 1.0));
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

fn draw_path(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Path { segs, fill, stroke, stroke_width, dash, cap } = it else { return };
    let path = wordcraft_layout::display::seg_path(segs);
    if path.elements().is_empty() {
        return;
    }
    ctx.set_transform(fr.view);
    if let Some(f) = fill {
        set_paint(ctx, f, fr.opts);
        ctx.fill_path(&path);
    }
    if let Some(s) = stroke {
        // Width 0 (or none) is a hairline.
        let w = if stroke_width.is_finite() && *stroke_width > 0.0 { stroke_width.clamp(0.25, 200.0) } else { 0.75 };
        ctx.set_paint(color(fr.opts.ink(*s), 1.0));
        ctx.set_stroke(
            kurbo::Stroke::new(w as f64)
                .with_caps(screen_cap(*cap))
                .with_dashes(0.0, safe_dashes(dash.iter().map(|v| f64::from(*v)), path_len_bound(&path))),
        );
        ctx.stroke_path(&path);
    }
}

fn draw_art(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Art { path, fx, color: c, alpha, text_bg } = it else { return };
    let Some(b) = wordcraft_layout::wordart::path_box(path) else { return };
    if !b.inflate(200.0, 200.0).overlaps(*fr.visible) {
        return;
    }
    ctx.set_transform(fr.view);
    for l in wordcraft_layout::wordart::art_layers(path, fx, *c, *alpha) {
        // The letters read against their highlight like plain text (#1252); shadows don't.
        let ink = |c: Rgb| if l.letters { fr.opts.ink_text(c, *text_bg) } else { fr.opts.ink(c) };
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

fn draw_ink(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Ink { pts, color: c, width, alpha } = it else { return };
    let Some(path) = ink_path(pts) else { return };
    if !path.bounding_box().inflate(*width as f64, *width as f64).overlaps(*fr.visible) {
        return;
    }
    ctx.set_transform(fr.view);
    ctx.set_paint(color(fr.opts.ink(*c), *alpha));
    let w = if width.is_finite() { width.clamp(0.25, 200.0) } else { 1.0 };
    ctx.set_stroke(kurbo::Stroke::new(w as f64).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round));
    ctx.stroke_path(&path);
}

/// A shape's outline and how it is painted, for its shadow, glow and soft edges.
struct ShapeGeom {
    r: kurbo::Rect,
    kind: ShapeKind,
    corner: Option<u32>,
    can_fill: bool,
    stroked: bool,
}

/// A shape's shadow and glow, behind it: bands of its silhouette grown by `g` points and moved
/// by `d`, in the current paint. `sw` is the outline's width.
fn shape_shadow_and_glow(ctx: &mut RenderContext, fr: &Frame, (geom, sw): (&ShapeGeom, f64), fx: &wordcraft_doc::effects::ShapeEffects) {
    let r = geom.r;
    let silhouette = |ctx: &mut RenderContext, g: f32, d: kurbo::Vec2| {
        let g = g as f64;
        if geom.can_fill {
            let grow = g + if geom.stroked { sw / 2.0 } else { 0.0 };
            let rr = r.inflate(grow, grow) + d;
            if rr.width() > 0.0 && rr.height() > 0.0 {
                ctx.fill_path(&shape_path(geom.kind, rr, geom.corner));
            }
        } else if sw + 2.0 * g > 0.05 {
            ctx.set_stroke(kurbo::Stroke::new(sw + 2.0 * g));
            ctx.stroke_path(&shape_path(geom.kind, r + d, geom.corner));
        }
    };
    if let Some(sh) = fx.shadow {
        let (dx, dy) = sh.offset();
        for (g, a) in wordcraft_doc::effects::bands(-sh.blur / 2.0, sh.blur / 2.0, sh.opacity()) {
            ctx.set_paint(color(fr.opts.ink(sh.color), a));
            silhouette(ctx, g, kurbo::Vec2::new(dx as f64, dy as f64));
        }
    }
    if let Some(gl) = fx.glow {
        for (g, a) in wordcraft_doc::effects::bands(0.0, gl.size, gl.opacity()) {
            ctx.set_paint(color(fr.opts.ink(gl.color), a));
            silhouette(ctx, g, kurbo::Vec2::ZERO);
        }
    }
}

/// Soft edges: the fill fades out toward the outline (which fades with it).
fn shape_soft_edges(ctx: &mut RenderContext, fr: &Frame, geom: &ShapeGeom, (rad, f): (f32, &Paint)) {
    for (g, a) in wordcraft_doc::effects::bands(-rad, 0.0, 1.0) {
        let rr = geom.r.inflate(g as f64, g as f64);
        if rr.width() > 0.0 && rr.height() > 0.0 {
            set_paint(ctx, &f.faded(a), fr.opts);
            ctx.fill_path(&shape_path(geom.kind, rr, geom.corner));
        }
    }
}

fn draw_shape(ctx: &mut RenderContext, fr: &Frame, it: &Draw) {
    let Draw::Shape { rect, kind, fill, stroke, stroke_width, effects, corner, dash, cap } = it else { return };
    let r = kurbo_rect(rect);
    let path = shape_path(*kind, r, *corner);
    ctx.set_transform(fr.view);
    let fx = effects.sanitized();
    let geom = ShapeGeom { r, kind: *kind, corner: *corner, can_fill: *kind != ShapeKind::Line && fill.is_some(), stroked: stroke.is_some() };
    if !fx.is_empty() && (geom.can_fill || geom.stroked) {
        let sw = if stroke_width.is_finite() { stroke_width.clamp(0.25, 200.0) as f64 } else { 0.75 };
        shape_shadow_and_glow(ctx, fr, (&geom, sw), &fx);
    }
    if let (Some(rad), Some(f), true) = (fx.soft_edge, fill, geom.can_fill) {
        shape_soft_edges(ctx, fr, &geom, (rad.min(rect.w.min(rect.h) / 2.0).max(0.0), f));
        return;
    }
    if let Some(f) = fill {
        set_paint(ctx, f, fr.opts);
        ctx.fill_path(&path);
    }
    if let Some(s) = stroke {
        ctx.set_paint(color(fr.opts.ink(*s), 1.0));
        ctx.set_stroke(
            kurbo::Stroke::new(if stroke_width.is_nan() { 0.25 } else { stroke_width.clamp(0.25, 200.0) } as f64)
                .with_caps(screen_cap(*cap))
                .with_dashes(0.0, safe_dashes(dash.iter().map(|v| f64::from(*v)), path_len_bound(&path))),
        );
        ctx.stroke_path(&path);
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

/// Render the page `area` (points) at `scale` px/pt — for previews and thumbnails.
pub fn render_area(doc: &Document, page: &Page, area: wordcraft_geom::Rect, scale: f32, opts: &RenderOptions) -> Rendered {
    let wordcraft_geom::Rect { x, y, w, h } = area;
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let pw = ((w * scale).ceil() as u32).clamp(1, MAX_SIDE);
    let ph = ((h * scale).ceil() as u32).clamp(1, MAX_SIDE);
    let view = Affine::scale(scale as f64) * Affine::translate((-x as f64, -y as f64));
    render_region(doc, page, pw, ph, view, opts)
}

/// Draw `items` (page points) alone, on nothing, over the page `area` (points) at `scale` px/pt:
/// no paper, no watermark. A dragged object's preview.
pub fn render_items(doc: &Document, items: &[Draw], area: wordcraft_geom::Rect, scale: f32, opts: &RenderOptions) -> Rendered {
    let wordcraft_geom::Rect { x, y, w, h } = area;
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
mod tests;

#[cfg(test)]
mod cache_tests;
