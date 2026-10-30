//! A frosted ribbon: the page scrolling up under it shows through as a soft blur.
//!
//! Each rendered page leaves a tiny thumbnail. Every frame the pages that would lie under
//! the ribbon, were the canvas to continue upward, are sampled into a small strip. The
//! strip is stretched over the ribbon with linear filtering, which is the blur.

use egui::{Color32, ColorImage, Context, Rect, TextureHandle, TextureOptions, pos2, vec2};

/// Screen points covered by one texel of a thumbnail or of the strip.
pub const CELL: f32 = 4.0;

/// How much of the ribbon's own colour lies over the blur (0 to 255), in a dark theme.
pub const TINT_ALPHA: u8 = 214;

/// The same in a light theme, where the page and the ribbon are both pale and little shows.
pub const LIGHT_TINT_ALPHA: u8 = 125;

/// A page averaged down to cells of about [`CELL`] points, over its paper colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumb {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Thumb {
    /// Average `img` in blocks of `block` pixels. Pixels are premultiplied, so they go over
    /// `paper` by adding the paper scaled by the missing alpha.
    pub fn of(img: &ColorImage, block: usize, paper: Color32) -> Thumb {
        let block = block.max(1);
        let [iw, ih] = img.size;
        let (w, h) = (iw.div_ceil(block).max(1), ih.div_ceil(block).max(1));
        let paper = [paper.r() as f32, paper.g() as f32, paper.b() as f32];
        let mut px = Vec::with_capacity(w * h);
        for ty in 0..h {
            for tx in 0..w {
                let (mut sum, mut n) = ([0.0_f32; 3], 0.0_f32);
                for y in ty * block..((ty + 1) * block).min(ih) {
                    for x in tx * block..((tx + 1) * block).min(iw) {
                        let Some(c) = img.pixels.get(y * iw + x) else { continue };
                        let miss = 1.0 - c.a() as f32 / 255.0;
                        sum[0] += c.r() as f32 + paper[0] * miss;
                        sum[1] += c.g() as f32 + paper[1] * miss;
                        sum[2] += c.b() as f32 + paper[2] * miss;
                        n += 1.0;
                    }
                }
                let n = n.max(1.0);
                px.push([sum[0] / n, sum[1] / n, sum[2] / n]);
            }
        }
        Thumb { w, h, px }
    }

    /// The colour at (`u`, `v`) in 0..1 of the page, bilinear between cell centres.
    fn sample(&self, u: f32, v: f32) -> [f32; 3] {
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
        out
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
                    *slot = page.thumb.map_or(paper, |t| t.sample(u, v));
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
pub fn upload(ctx: &Context, name: &str, image: ColorImage) -> egui::TextureId {
    let id = egui::Id::unique(("ribbon_blur_strip", name));
    let kept: Option<Strip> = ctx.data(|d| d.get_temp(id));
    let strip = match kept {
        Some(mut s) => {
            if s.image != image {
                s.tex.set(image.clone(), TextureOptions::LINEAR);
                s.image = image;
            }
            s
        }
        None => Strip { tex: ctx.load_texture(format!("ribbon_blur_{name}"), image.clone(), TextureOptions::LINEAR), image },
    };
    let tex_id = strip.tex.id();
    ctx.data_mut(|d| d.insert_temp(id, strip));
    tex_id
}

/// Paint the strip under the ribbon into the slot `slot` reserved before the ribbon drew.
/// `view_top` is where the document view starts: the page position that the ribbon's lower
/// edge stands for.
pub fn paint(app: &crate::WordApp, ui: &egui::Ui, slot: egui::layers::ShapeIdx, ribbon: Rect, view_top: f32) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let paper = if app.session.view.dark_mode { Color32::from_gray(wordcraft_render::DARK_PAPER) } else { Color32::WHITE };
    let shift = vec2(0.0, ribbon.max.y - view_top);
    let pages: Vec<PageView> = app
        .canvas
        .page_rects
        .iter()
        .enumerate()
        .map(|(i, r)| PageView { rect: r.translate(shift), thumb: app.canvas.thumbs.get(&i), paper })
        .collect();
    let id = upload(ui.ctx(), "ribbon", compose(ribbon, &pages, t.canvas));
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().with_clip_rect(ribbon).set(slot, egui::Shape::image(id, ribbon, full, Color32::WHITE));
}

/// Paint the strip now, over `area` of the canvas: the pages as they lie, without a shift.
pub fn paint_band(app: &crate::WordApp, ui: &egui::Ui, area: Rect) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let paper = if app.session.view.dark_mode { Color32::from_gray(wordcraft_render::DARK_PAPER) } else { Color32::WHITE };
    let pages: Vec<PageView> =
        app.canvas.page_rects.iter().enumerate().map(|(i, r)| PageView { rect: *r, thumb: app.canvas.thumbs.get(&i), paper }).collect();
    let id = upload(ui.ctx(), "ruler", compose(area, &pages, t.canvas));
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(id, area, full, Color32::WHITE);
}

/// While set, the ribbon draws over the strip: its hover and press fills stay see-through.
pub fn set_drawing(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("ribbon_blur_drawing"), on));
}

/// `fill` for a ribbon button's hover, press or check, see-through while the strip shows.
pub fn button_fill(ctx: &Context, fill: Color32) -> Color32 {
    let on = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("ribbon_blur_drawing"))).unwrap_or(false);
    if on { fill.gamma_multiply(0.6) } else { fill }
}

/// A menu's fill, a little see-through and shaded (darker in a dark theme, lighter in a
/// light one), so the page behind it does not fight the text.
pub fn menu_glass(c: Color32, dark: bool) -> Color32 {
    let shade = if dark { Color32::BLACK } else { Color32::WHITE };
    let c = c.lerp_to_gamma(shade, 0.5);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 242)
}

/// Paint `rect` fading down from `top` opaque to the see-through ribbon fill at its foot.
pub fn fade_down(painter: &egui::Painter, rect: Rect, top: Color32) {
    let foot = glass(top);
    let mut mesh = egui::Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    let corners = [(rect.left_top(), top), (rect.right_top(), top), (rect.right_bottom(), foot), (rect.left_bottom(), foot)];
    for (pos, color) in corners {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// `c` made see-through, so the blur shows under it.
pub fn glass(c: Color32) -> Color32 {
    let light = c.r() as u32 + c.g() as u32 + c.b() as u32 > 384;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), if light { LIGHT_TINT_ALPHA } else { TINT_ALPHA })
}

/// The strip's size in texels, once it has been made.
#[cfg(test)]
pub fn strip_size(ctx: &Context) -> Option<[usize; 2]> {
    let kept: Option<Strip> = ctx.data(|d| d.get_temp(egui::Id::unique(("ribbon_blur_strip", "ribbon"))));
    kept.map(|s| s.image.size)
}

/// Downsample factor that gives cells of about [`CELL`] points at `ppp` pixels per point.
pub fn block_for(ppp: f32) -> usize {
    (CELL * ppp.clamp(0.25, 8.0)).round().max(1.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, c: Color32) -> ColorImage {
        ColorImage::new([w, h], vec![c; w * h])
    }

    #[test]
    fn a_transparent_page_thumbnail_is_the_paper() {
        let t = Thumb::of(&flat(40, 40, Color32::TRANSPARENT), 8, Color32::WHITE);
        assert_eq!(t.sample(0.5, 0.5), [255.0; 3]);
    }

    #[test]
    fn ink_darkens_its_cell() {
        let mut img = flat(16, 16, Color32::TRANSPARENT);
        for y in 0..8 {
            for x in 0..8 {
                img.pixels[y * 16 + x] = Color32::BLACK;
            }
        }
        let t = Thumb::of(&img, 8, Color32::WHITE);
        assert!(t.sample(0.25, 0.25)[0] < 1.0);
        assert!(t.sample(0.75, 0.75)[0] > 254.0);
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
