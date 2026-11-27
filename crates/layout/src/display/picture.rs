//! Picture-effect masks, shared by screen and PDF output.

use std::collections::VecDeque;
use std::sync::Arc;

use image::{GrayImage, Luma, Rgba, RgbaImage, imageops};
use wordcraft_doc::{effects::Glow, picfx::PicFx};
use wordcraft_geom::Rect;

/// A straight RGBA mask in a frame relative to the picture's top-left corner.
#[derive(Clone)]
pub struct PictureEffect {
    pub rect: Rect,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// Cache identity excludes page position: repeated instances can share masks.
pub fn picture_effect_key(media: &str, w: f32, h: f32, crop: [f32; 4], fx: PicFx) -> String {
    format!("{media}#{:?},{:?},{:?},{fx:?}", w.to_bits(), h.to_bits(), crop.map(f32::to_bits))
}

/// Picture effects use a printer-grid origin, separately from the main image.
/// Keeping placement outside mask generation lets identical masks share a cache
/// even when their pictures occupy different positions on the page.
pub fn picture_effect_origin(x: f32, y: f32, dx: f32, dy: f32) -> (f32, f32) {
    let origin = |v: f32, offset: f32| ((((v * 20.0).trunc() / 20.0 + offset) / 0.12).round() - 1.0) * 0.12;
    (origin(x, dx), origin(y, dy))
}

fn effect_padding(radius: f32, step_emu: u32) -> f32 {
    ((radius * 12_700.0).round() as u32).div_ceil(step_emu) as f32 * (step_emu as f32 / 12_700.0)
}

/// A glow is a dilated, blurred alpha silhouette. The outer shadow is cast by
/// the source *and* its glow. Public PDF observations at several radii establish
/// sigma=radius/6 for the glow and sigma=blur/3 for the shadow. Quantize each
/// complete mask once, rather than compositing many barely visible 8-bit bands.
pub fn picture_effects(source: &[u8], w: f32, h: f32, crop: [f32; 4], fx: PicFx) -> Vec<PictureEffect> {
    let effects = fx.shape_effects();
    let glow = effects.glow.filter(|g| g.size > 0.0);
    if effects.shadow.is_none() && glow.is_none() || !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return Vec::new();
    }
    let glow_size = glow.map_or(0.0, |g| g.size);
    let blur = effects.shadow.map_or(0.0, |s| s.blur);
    let decoded = wordcraft_geom::picture::decode_for_drawing(source, wordcraft_geom::MAX_DECODED_BYTES, 4096).ok().map(|p| p.into_rgba8());
    let mut out = Vec::new();
    if let Some(shadow) = effects.shadow {
        // Glow and shadow select their grids independently. Only exceptionally
        // large glows reduce this further to bound the dilation's work.
        let scale = effect_scale(blur, 8).min(64.0 / glow_size.max(0.01));
        let pad = effect_padding(glow_size, 3_048) + effect_padding(blur, 1_524);
        let (frame, scale, mut silhouette) = silhouette(decoded.as_ref(), w, h, crop, pad, scale);
        if let Some(g) = glow {
            let glow = glow_mask(&silhouette, g, scale);
            for (a, &g) in silhouette.as_mut().iter_mut().zip(glow.as_raw()) {
                *a = (u32::from(*a) + (u32::from(g) * (255 - u32::from(*a)) + 127) / 255).min(255) as u8;
            }
        }
        let mask = blurred(&silhouette, shadow.blur * scale / 3.0);
        let (dx, dy) = shadow.offset();
        out.push(coloured(&mask, Rect::new(frame.x + dx, frame.y + dy, frame.w, frame.h), shadow.color, shadow.opacity()));
    }
    if let Some(g) = glow {
        let (frame, scale, base) = silhouette(decoded.as_ref(), w, h, crop, effect_padding(glow_size, 3_048), effect_scale(glow_size, 16));
        out.push(coloured(&glow_mask(&base, g, scale), frame, g.color, 1.0));
    }
    out
}

/// Effects have separate sampling grids. A glow permits twice the physical
/// radius of a shadow at a given resolution; both start at 200 dpi and reduce
/// resolution in integer steps to keep their convolution kernels bounded.
fn effect_scale(radius: f32, pixels_per_step: u32) -> f32 {
    let base = 200.0 / 72.0;
    // Compare in OOXML's integer EMUs: a radius exactly on the boundary still
    // uses the finer grid (2.88 pt shadow / 5.76 pt glow at 200 dpi).
    let emu = (radius * 12_700.0).round() as u32;
    let step_emu = pixels_per_step * (914_400 / 200);
    base / emu.div_ceil(step_emu).max(1) as f32
}

fn glow_mask(base: &GrayImage, glow: Glow, scale: f32) -> GrayImage {
    let grown = dilate(base, (glow.size * scale / 2.0).floor() as u32);
    let mut mask = blurred(&grown, glow.size * scale / 6.0);
    for a in mask.as_mut() {
        *a = (f32::from(*a) * glow.opacity()).round() as u8;
    }
    mask
}

fn silhouette(decoded: Option<&RgbaImage>, w: f32, h: f32, crop: [f32; 4], pad: f32, scale: f32) -> (Rect, f32, GrayImage) {
    // All temporary masks stay below four million pixels, even for large pages.
    let fw = w + 2.0 * pad;
    let fh = h + 2.0 * pad;
    let (pw, ph) = wordcraft_geom::fit_pixels((fw * scale).ceil() as u32, (fh * scale).ceil() as u32, 4_000_000);
    // Pixel counts are integers; the physical canvas is not. Expanding one axis
    // to preserve a common pixel pitch moves the effect's opposite edge.
    let (sx, sy) = (pw as f32 / fw, ph as f32 / fh);
    let scale = sx.min(sy).min(scale);
    let frame = Rect::new(-pad, -pad, fw, fh);
    let [cl, ct, cw, ch] = crate::crop_window(crop);
    let base = GrayImage::from_fn(pw, ph, |x, y| {
        let px = (x as f32 + 0.5) / sx - pad;
        let py = (y as f32 + 0.5) / sy - pad;
        if px < 0.0 || py < 0.0 || px >= w || py >= h {
            return Luma([0]);
        }
        let a = decoded.map_or(255, |im| {
            let sx = (cl + f64::from(px / w) * cw) * f64::from(im.width());
            let sy = (ct + f64::from(py / h) * ch) * f64::from(im.height());
            if sx < 0.0 || sy < 0.0 || sx >= f64::from(im.width()) || sy >= f64::from(im.height()) {
                0
            } else {
                im.get_pixel(sx as u32, sy as u32)[3]
            }
        });
        Luma([a])
    });
    (frame, scale, base)
}

fn blurred(mask: &GrayImage, sigma: f32) -> GrayImage {
    if sigma < 0.1 { mask.clone() } else { imageops::blur(mask, sigma) }
}

fn coloured(mask: &GrayImage, rect: Rect, color: wordcraft_doc::props::Rgb, opacity: f32) -> PictureEffect {
    let palette: [Rgba<u8>; 256] = std::array::from_fn(|a| effect_pixel(color, (a as f32 * opacity).round() as u8));
    let rgba = RgbaImage::from_fn(mask.width(), mask.height(), |x, y| palette[usize::from(mask.get_pixel(x, y)[0])]);
    PictureEffect { rect, width: mask.width(), height: mask.height(), rgba: Arc::new(rgba.into_raw()) }
}

/// Retain the effect's colour quantization across the straight RGBA interface
/// used by both painters. The intermediate conversion to straight colour uses
/// a truncated reciprocal; replacing it with exact division changes the ink.
/// Round the final straight encoding so an RGBA8 painter recovers the desired
/// premultiplied byte, including at very low alpha.
fn effect_pixel(color: wordcraft_doc::props::Rgb, alpha: u8) -> Rgba<u8> {
    let a = u32::from(alpha);
    if a == 0 {
        // PDF may resample straight colour and the soft mask independently.
        // Retain the hidden colour rather than introducing a black fringe.
        return Rgba([color.0, color.1, color.2, 0]);
    }
    let reciprocal = (255 << 16) / a;
    let channel = |c: u8| {
        let premul = (u32::from(c) * a + 127) / 255;
        let straight = ((premul * reciprocal) >> 16).min(255);
        let ink = straight * a / 255;
        ((ink * 255 + a / 2) / a) as u8
    };
    Rgba([channel(color.0), channel(color.1), channel(color.2), alpha])
}

/// Grayscale dilation by a disk. Each horizontal chord uses a sliding maximum,
/// so work is O(pixels * radius), with radius bounded above by the raster scale.
fn dilate(src: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 {
        return src.clone();
    }
    let (w, h) = src.dimensions();
    let mut out = GrayImage::new(w, h);
    let r = radius as i32;
    let mut queue = VecDeque::<u32>::new();
    for dy in -r..=r {
        let half = ((r * r - dy * dy) as f32).sqrt().floor() as u32;
        for y in 0..h {
            let sy = i64::from(y) + i64::from(dy);
            if sy < 0 || sy >= i64::from(h) {
                continue;
            }
            let row = &src.as_raw()[sy as usize * w as usize..(sy as usize + 1) * w as usize];
            queue.clear();
            let mut end = 0;
            for x in 0..w {
                let right = x.saturating_add(half).min(w - 1);
                while end <= right {
                    while queue.back().is_some_and(|&i| row[i as usize] <= row[end as usize]) {
                        queue.pop_back();
                    }
                    queue.push_back(end);
                    end += 1;
                }
                while queue.front().is_some_and(|&i| i.saturating_add(half) < x) {
                    queue.pop_front();
                }
                if let Some(&i) = queue.front() {
                    let at = (y * w + x) as usize;
                    let value = out.as_raw()[at].max(row[i as usize]);
                    out.as_mut()[at] = value;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};

    fn source(transparent_right: bool) -> Vec<u8> {
        let image = RgbaImage::from_fn(16, 16, |x, _| Rgba([20, 180, 60, if transparent_right && x >= 8 { 0 } else { 255 }]));
        wordcraft_geom::picture::encode_format(&image::DynamicImage::from(image), image::ImageFormat::Png).unwrap()
    }

    fn alpha(mask: &PictureEffect, x: f32, y: f32) -> u8 {
        let x = ((x - mask.rect.x) / mask.rect.w * mask.width as f32) as u32;
        let y = ((y - mask.rect.y) / mask.rect.h * mask.height as f32) as u32;
        mask.rgba[((y * mask.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn translucent_effect_colours_retain_their_observed_premultiplied_ink() {
        use wordcraft_doc::props::Rgb;
        // Generated grayscale and mixed-colour reference controls. The same
        // colour has different quantization at different opacities.
        for (color, alpha, expected) in [
            (Rgb(100, 100, 100), 51, [20; 3]),
            (Rgb(100, 100, 100), 77, [29; 3]),
            (Rgb(100, 100, 100), 128, [49; 3]),
            (Rgb(100, 100, 100), 153, [59; 3]),
            (Rgb(100, 100, 100), 204, [80; 3]),
            (Rgb(78, 13, 127), 121, [36, 5, 59]),
            (Rgb(61, 196, 75), 207, [49, 158, 60]),
            (Rgb(196, 45, 253), 90, [68, 15, 88]),
            (Rgb(255, 255, 255), 128, [128; 3]),
        ] {
            let Rgba([r, g, b, a]) = effect_pixel(color, alpha);
            let ink = [r, g, b].map(|c| ((u32::from(c) * u32::from(a) + 127) / 255) as u8);
            assert_eq!(ink, expected);
            assert_eq!(a, alpha);
        }
        assert_eq!(effect_pixel(Rgb(78, 13, 127), 0), Rgba([78, 13, 127, 0]));
        assert_eq!(effect_pixel(Rgb(78, 13, 127), 255), Rgba([78, 13, 127, 255]));
    }

    #[test]
    fn a_picture_glows_half_opacity_contour_stays_inside_the_observed_band() {
        let bytes = source(false);
        for (size, outer, inner) in [(2.0, -1.0, -0.5), (5.0, -2.5, -2.0)] {
            let fx = PicFx::default().with_shape_effects(ShapeEffects { glow: Some(Glow { size, ..Default::default() }), ..Default::default() });
            let masks = picture_effects(&bytes, 120.0, 60.0, [0.0; 4], fx);
            let mask = &masks[0];
            let half = alpha(mask, 60.0, 30.0) / 2;
            assert!(alpha(mask, outer, 30.0) < half, "the outer sample is outside the half-opacity contour: {size}");
            assert!(alpha(mask, inner, 30.0) > half, "the inner sample is inside the half-opacity contour: {size}");
        }
    }

    #[test]
    fn the_glow_also_casts_an_outer_shadow() {
        let bytes = source(false);
        let effects = ShapeEffects { shadow: Some(Shadow { distance: 0.0, blur: 4.0, ..Default::default() }), ..Default::default() };
        let plain = picture_effects(&bytes, 40.0, 30.0, [0.0; 4], PicFx::default().with_shape_effects(effects));
        let combined = picture_effects(
            &bytes,
            40.0,
            30.0,
            [0.0; 4],
            PicFx::default().with_shape_effects(ShapeEffects { glow: Some(Glow { size: 5.0, ..Default::default() }), ..effects }),
        );
        assert!(alpha(&combined[0], -2.0, 15.0) > alpha(&plain[0], -2.0, 15.0), "the glow broadens the cast shadow");
        assert_eq!(alpha(&combined[0], 20.0, 15.0), alpha(&plain[0], 20.0, 15.0), "the interior keeps the requested opacity");
    }

    #[test]
    fn a_coarser_shadow_does_not_change_the_glow_sampling_grid() {
        let bytes = source(false);
        let effects = ShapeEffects { glow: Some(Glow { size: 5.0, ..Default::default() }), ..Default::default() };
        let plain = picture_effects(&bytes, 40.0, 30.0, [0.0; 4], PicFx::default().with_shape_effects(effects));
        let combined = picture_effects(
            &bytes,
            40.0,
            30.0,
            [0.0; 4],
            PicFx::default().with_shape_effects(ShapeEffects { shadow: Some(Shadow { blur: 4.0, ..Default::default() }), ..effects }),
        );
        assert_eq!(plain[0].rgba, combined[1].rgba, "adding a shadow preserves the complete glow mask");
        assert_eq!(plain[0].rect, combined[1].rect);
        assert!(combined[0].width as f32 / combined[0].rect.w < combined[1].width as f32 / combined[1].rect.w);
    }

    #[test]
    fn an_exact_effect_radius_boundary_keeps_the_finer_grid() {
        for (pixels, boundary) in [(8, 2.88), (16, 5.76)] {
            assert_eq!(effect_scale(boundary, pixels), effect_scale(0.0, pixels));
            assert!(effect_scale(boundary + 0.01, pixels) < effect_scale(boundary, pixels));
            assert_eq!(effect_scale(boundary * 2.0, pixels), effect_scale(boundary + 0.01, pixels));
        }
    }

    #[test]
    fn effect_origins_share_sub_twip_positions_but_follow_the_picture() {
        let start = picture_effect_origin(80.0, 100.0, 0.0, 0.0);
        assert_eq!(start, picture_effect_origin(80.03, 100.03, 0.0, 0.0));
        let moved = picture_effect_origin(80.12, 100.12, 0.0, 0.0);
        assert!(moved.0 > start.0 && moved.1 > start.1);
    }

    #[test]
    fn a_zero_radius_glow_does_not_recolour_translucent_source_pixels() {
        let fx = PicFx::default().with_shape_effects(ShapeEffects { glow: Some(Glow { size: 0.0, ..Default::default() }), ..Default::default() });
        assert!(picture_effects(&source(true), 40.0, 30.0, [0.0; 4], fx).is_empty());
    }

    #[test]
    fn a_wide_glow_does_not_coarsen_a_sharp_shadow() {
        let masks = picture_effects(
            &source(false),
            40.0,
            30.0,
            [0.0; 4],
            PicFx::default().with_shape_effects(ShapeEffects {
                shadow: Some(Shadow { blur: 1.0, ..Default::default() }),
                glow: Some(Glow { size: 15.0, ..Default::default() }),
                ..Default::default()
            }),
        );
        assert!(masks[0].width as f32 / masks[0].rect.w > masks[1].width as f32 / masks[1].rect.w);
    }

    #[test]
    fn effects_follow_source_alpha_and_the_crop_window() {
        let bytes = source(true);
        let fx = PicFx::default().with_shape_effects(ShapeEffects {
            glow: Some(Glow { size: 3.0, ..Default::default() }),
            shadow: Some(Shadow { distance: 0.0, ..Default::default() }),
            ..Default::default()
        });
        let whole = picture_effects(&bytes, 40.0, 30.0, [0.0; 4], fx);
        for mask in &whole {
            assert!(alpha(mask, 10.0, 15.0) > alpha(mask, 35.0, 15.0), "the transparent half casts no rectangular decoration");
        }
        let cropped = picture_effects(&bytes, 40.0, 30.0, [0.5, 0.0, 0.0, 0.0], fx);
        assert!(
            cropped.iter().all(|m| m.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0)),
            "a fully transparent crop casts neither glow nor shadow"
        );
    }
}
