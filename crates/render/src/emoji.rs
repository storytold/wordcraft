//! Colour emoji for the interface: the same emoji fonts and colour-glyph path the page draws with
//! ([`crate::color_glyph`]), rasterised into a small straight-alpha RGBA image that the UI can show
//! as a texture. The Navigation pane uses it for the emoji in heading labels, which the interface
//! fonts only have monochrome.

use std::sync::Arc;

use vello_cpu::kurbo::{Affine, Rect};
use vello_cpu::{PixmapMut, Resources};

use crate::color_glyph::{self, Drawable};
use crate::context_pool::CONTEXTS;

/// A colour emoji raster: straight (not premultiplied) RGBA, row-major, top-down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The largest side an emoji raster may have, pixels: the UI asks for a line's height, not more.
const MAX_SIDE: u32 = 512;

/// `text` (an emoji or an emoji sequence such as `⚖️` or `👩‍💻`) drawn from a colour emoji font,
/// about `px` pixels tall (the em square), cropped to its ink, in straight-alpha RGBA. `None` when
/// the system has no colour emoji font or the glyphs carry no colour data, so the caller keeps the
/// interface font's monochrome glyph.
pub fn emoji_image(text: &str, px: u32) -> Option<EmojiImage> {
    let px = px.clamp(4, MAX_SIDE) as f64;
    let chars: Vec<char> = text.chars().collect();
    if chars.iter().all(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let face = wordcraft_fonts::FontDb::global().color_emoji_face(&chars, u32::MAX, false, false)?;
    // A monochrome face (the run fallback when no colour emoji family is installed) would draw a
    // flat silhouette: the interface font's own glyph is better than that.
    if !wordcraft_fonts::COLOR_EMOJI.contains(&face.family.as_str()) {
        return None;
    }
    let glyphs = wordcraft_fonts::shape(&face, text, &[], |c| c);
    if glyphs.is_empty() {
        return None;
    }
    let upem = face.upem;
    if !(upem.is_finite() && upem > 0.0) {
        return None;
    }
    let scale = px / upem;

    // Drawables with their pen positions (font units, y down from the baseline), and the union of
    // their boxes: the image is sized from that, then cropped to the ink.
    let mut pen = 0.0f64;
    let mut placed: Vec<(Arc<Drawable>, (f64, f64))> = Vec::new();
    let mut bounds: Option<Rect> = None;
    for g in &glyphs {
        let origin = (pen + g.x_offset as f64, g.y_offset as f64);
        if let Some(d) = color_glyph::drawable(&face, g.gid, px, true) {
            let b = match &*d {
                Drawable::Bitmap { rect, .. } => Rect::new(rect.x0 + origin.0, rect.y0 + origin.1, rect.x1 + origin.0, rect.y1 + origin.1),
                // COLR glyphs have no bitmap box: take the em square, trimmed later by the ink.
                Drawable::Layers(_) => Rect::new(origin.0 - 0.1 * upem, origin.1 - 0.95 * upem, origin.0 + 1.1 * upem, origin.1 + 0.1 * upem),
            };
            bounds = Some(match bounds {
                Some(u) => u.union(b),
                None => b,
            });
            placed.push((d, origin));
        }
        pen += g.x_advance as f64;
    }
    let b = bounds?;
    // A wide sequence (a ZWJ family at 512 px) can need more than MAX_SIDE: shrink the drawing so
    // its ink fits the image instead of being clipped at the right or bottom edge.
    let raw_w = b.width() * scale + 2.0;
    let raw_h = b.height() * scale + 2.0;
    let shrink = (MAX_SIDE as f64 / raw_w).min(MAX_SIDE as f64 / raw_h).min(1.0);
    let scale = scale * shrink;
    let w = (b.width() * scale).ceil() + 2.0;
    let h = (b.height() * scale).ceil() + 2.0;
    if !(w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0) {
        return None;
    }
    let (w, h) = ((w as u32).clamp(1, MAX_SIDE), (h as u32).clamp(1, MAX_SIDE));

    let mut ctx = CONTEXTS.take(w as u16, h as u16);
    ctx.set_transform(Affine::IDENTITY);
    // Font units (pen at the origin) → the image, with one pixel of margin.
    let to_image = Affine::translate((1.0 - b.x0 * scale, 1.0 - b.y0 * scale)) * Affine::scale(scale);
    for (d, origin) in &placed {
        color_glyph::draw(&mut ctx, d, to_image * Affine::translate(*origin), upem, vello_cpu::peniko::Color::WHITE, 1.0);
    }
    ctx.flush();
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let mut res = Resources::new();
    if let Some(pm) = PixmapMut::new(w as u16, h as u16, &mut pixels) {
        ctx.render(pm, &mut res);
    }
    CONTEXTS.give_back(ctx);

    crop_straight(&pixels, w, h)
}

/// Crop premultiplied RGBA rows to the ink and un-premultiply them.
fn crop_straight(pixels: &[u8], w: u32, h: u32) -> Option<EmojiImage> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if pixels[((y * w + x) * 4 + 3) as usize] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == u32::MAX {
        return None;
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = ((y * w + x) * 4) as usize;
            let a = pixels[i + 3] as u32;
            let straight = |v: u8| if a == 0 || a == 255 { v } else { ((v as u32 * 255 + a / 2) / a).min(255) as u8 };
            rgba.extend_from_slice(&[straight(pixels[i]), straight(pixels[i + 1]), straight(pixels[i + 2]), a as u8]);
        }
    }
    Some(EmojiImage { width: cw, height: ch, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_colour_emoji() -> bool {
        wordcraft_fonts::FontDb::global().color_emoji_face(&['📘'], u32::MAX, false, false).is_some()
    }

    /// A colour emoji font, when the system has one, gives a non-empty image with actual colour in
    /// it — what the Navigation pane paints over the interface font's monochrome glyph.
    #[test]
    fn a_colour_emoji_rasterises_in_colour() {
        if !has_colour_emoji() {
            return;
        }
        let img = emoji_image("📘", 32).expect("the blue book rasterises");
        assert!(img.width > 4 && img.height > 4, "cropped to the ink: {img:?}");
        assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
        let colourful = img.rgba.chunks(4).any(|p| {
            p[3] > 200 && {
                let (hi, lo) = (*p[0..3].iter().max().unwrap(), *p[0..3].iter().min().unwrap());
                hi - lo > 40
            }
        });
        assert!(colourful, "the book is blue, not grey");
    }

    #[test]
    fn several_markers_rasterise() {
        if !has_colour_emoji() {
            return;
        }
        for m in ["📘", "🔄", "⚖️", "📊", "📝"] {
            assert!(emoji_image(m, 20).is_some(), "{m} must rasterise");
        }
    }

    /// Junk input and text-only strings give no image: the caller keeps the monochrome glyph.
    #[test]
    fn text_and_blank_strings_have_no_image() {
        assert!(emoji_image("   ", 16).is_none());
        assert!(emoji_image("", 16).is_none());
        if has_colour_emoji() {
            assert!(emoji_image("conceito", 16).is_none(), "letters are not a colour emoji");
        }
    }

    /// The size asked for is respected (the em square, about), and nonsense sizes are clamped
    /// instead of asking vello for a huge pixmap.
    #[test]
    fn the_requested_size_is_respected() {
        if !has_colour_emoji() {
            return;
        }
        let small = emoji_image("📘", 8).expect("small");
        let big = emoji_image("📘", 96).expect("big");
        assert!(big.height > small.height, "bigger size, taller image: {small:?} {big:?}");
        assert!(emoji_image("📘", 0).is_some(), "clamped, not zero");
    }
}
