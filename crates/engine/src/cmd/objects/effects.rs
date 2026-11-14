//! Picture Format › Artistic Effects: our own drawings of the gallery's effects, each from the
//! picture's own pixels (no textures or assets). Every effect is drawn on at most
//! [`EFFECT_PIXELS`] pixels: a larger picture is shrunk first, and stays that size (its size on
//! the page doesn't change). All are repeatable: the same picture always gives the same result.

use image::{DynamicImage, RgbaImage};

/// The most pixels an effect draws.
const EFFECT_PIXELS: u64 = 2_000_000;

/// `img` with artistic effect `name` (one of [`super::look::EFFECTS`] other than the five drawn in
/// `picture.rs`); an unknown name leaves it as it is.
pub(super) fn artistic(img: DynamicImage, name: &str) -> DynamicImage {
    let rgba = bounded(img);
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return DynamicImage::ImageRgba8(rgba);
    }
    // The size effects scale with: lines and cells stay in proportion to the picture.
    let unit = (w.min(h) / 160).max(1) as usize;
    let out = match name {
        "marker" => {
            // Flat strokes of a few strong colours, outlined where the picture has edges.
            let k = saturate(&palette(&kuwahara(&rgba, 2 * unit), 12), 1.2);
            let edges = strength(sobel(&blur(&luma(&k), w, h, unit), w, h));
            map(&k, |i, c| {
                let e = edges.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                c.map(|v| v * (1.0 - 0.7 * e * e))
            })
        }
        "pencilGrayscale" => {
            let s = sketch(&rgba, 3 * unit);
            map(&rgba, |i, _| [s.get(i).copied().unwrap_or(255.0); 3])
        }
        "pencilSketch" => {
            let s = sketch(&rgba, 3 * unit);
            map(&rgba, |i, c| {
                let k = s.get(i).copied().unwrap_or(255.0) / 255.0;
                c.map(|v| (0.35 * v + 0.65 * 255.0) * k)
            })
        }
        "lineDrawing" => {
            let edges = strength(sobel(&blur(&luma(&rgba), w, h, unit.div_ceil(2)), w, h));
            map(&rgba, |i, _| {
                let e = ((edges.get(i).copied().unwrap_or(0.0) - 0.2) / 0.6).clamp(0.0, 1.0);
                [255.0 * (1.0 - e); 3]
            })
        }
        "chalkSketch" => {
            let edges = strength(sobel(&blur(&luma(&rgba), w, h, unit), w, h));
            map(&rgba, |i, c| {
                let (x, y) = ((i % w as usize) as u32, (i / w as usize) as u32);
                let grain = 0.55 + 0.6 * noise(x, y, 3);
                let chalk = ((edges.get(i).copied().unwrap_or(0.0) - 0.1) / 0.6).clamp(0.0, 1.0) * grain;
                c.map(|v| v * 0.25 + (235.0 - v * 0.25) * chalk)
            })
        }
        "paintStrokes" => saturate(&kuwahara(&rgba, 3 * unit), 1.15),
        "paintBrush" => saturate(&kuwahara(&rgba, 5 * unit), 1.1),
        "glowDiffused" => {
            let soft = blur_rgb(&rgba, 6 * unit);
            map2(&rgba, &soft, |a, b| a.zip_map(b, |x, y| 255.0 - (255.0 - x) * (255.0 - y) / 255.0).map(|v| v + (255.0 - v) * 0.12))
        }
        "lightScreen" => {
            let step = 3 * unit + 3;
            map(&rgba, |i, c| {
                let (x, y) = (i % w as usize, i / w as usize);
                let line = (x + y) % step < unit;
                let k = if line { 0.55 } else { 0.25 };
                c.map(|v| v + (255.0 - v) * k)
            })
        }
        "watercolorSponge" => {
            let k = saturate(&kuwahara(&rgba, 3 * unit), 1.2);
            let sponge = blur(
                &(0..(w as usize * h as usize)).map(|i| noise((i % w as usize) as u32, (i / w as usize) as u32, 11)).collect::<Vec<_>>(),
                w,
                h,
                unit,
            );
            map(&k, |i, c| {
                let s = 0.85 + 0.2 * sponge.get(i).copied().unwrap_or(0.5);
                c.map(|v| v * s)
            })
        }
        "filmGrain" => map(&rgba, |i, c| {
            let (x, y) = ((i % w as usize) as u32, (i / w as usize) as u32);
            let g = (noise(x, y, 5) + noise(x / 2, y / 2, 6) - 1.0) * 30.0;
            c.map(|v| 128.0 + (v - 128.0) * 0.9 + g)
        }),
        "mosaicBubbles" => bubbles(&rgba, (w.min(h) / 36).max(4)),
        "glass" => {
            let amp = (w.min(h) as f32 / 90.0).max(1.5);
            let field = |x: u32, y: u32, seed: u32| (smooth_noise(x, y, 6.0 * unit as f32, seed) - 0.5) * 2.0 * amp;
            RgbaImage::from_fn(w, h, |x, y| {
                let sx = (x as f32 + field(x, y, 21)).clamp(0.0, (w - 1) as f32) as u32;
                let sy = (y as f32 + field(x, y, 22)).clamp(0.0, (h - 1) as f32) as u32;
                *rgba.get_pixel(sx, sy)
            })
        }
        "cement" => {
            let tex = emboss(
                &blur(
                    &(0..(w as usize * h as usize)).map(|i| noise((i % w as usize) as u32, (i / w as usize) as u32, 13) * 255.0).collect::<Vec<_>>(),
                    w,
                    h,
                    1,
                ),
                w,
                h,
            );
            map(&rgba, |i, c| {
                let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
                let t = tex.get(i).copied().unwrap_or(0.0);
                c.map(|v| (0.5 * v + 0.5 * l) * 0.95 + t * 0.6)
            })
        }
        "texturizer" => map(&rgba, |i, c| {
            let (x, y) = ((i % w as usize) as f32, (i / w as usize) as f32);
            let p = 2.0 * std::f32::consts::PI / (4.0 * unit as f32);
            let weave = 0.92 + 0.06 * (x * p).sin() * (y * p).cos() + 0.04 * ((x + y) * p * 0.5).sin();
            c.map(|v| v * weave)
        }),
        "crisscrossEtching" => {
            let l = luma(&rgba);
            let step = 4 * unit;
            map(&rgba, |i, c| {
                let (x, y) = (i % w as usize, i / w as usize);
                let g = l.get(i).copied().unwrap_or(255.0);
                let ink = ((x + y) % step < unit && g < 150.0)
                    || ((x + step * 4 - y % (step * 4)) % step < unit && g < 95.0)
                    || (y % step < unit && g < 50.0);
                if ink { c.map(|v| v * 0.25) } else { c.map(|v| v * 0.15 + 255.0 * 0.85) }
            })
        }
        "pastelsSmooth" => {
            let soft = saturate(&blur_rgb(&kuwahara(&rgba, 2 * unit), unit), 1.3);
            map(&soft, |_, c| c.map(|v| v + (255.0 - v) * 0.2))
        }
        "plasticWrap" => {
            // Highlights where the surface faces the light most, by the picture's own steepest
            // slope, so a soft picture shines too.
            let l = blur(&luma(&rgba), w, h, 2 * unit);
            let shine = emboss(&l, w, h);
            let top = shine.iter().copied().fold(0.0f32, f32::max).max(1e-3);
            map(&rgba, |i, c| {
                let s = ((shine.get(i).copied().unwrap_or(0.0) / top - 0.25) / 0.75).clamp(0.0, 1.0);
                c.map(|v| v + (255.0 - v) * s * s * 0.9)
            })
        }
        "cutout" => palette(&kuwahara(&rgba, 3 * unit), 8),
        "photocopy" => {
            let l = luma(&rgba);
            let local = blur(&l, w, h, 8 * unit);
            map(&rgba, |i, _| {
                let d = l.get(i).copied().unwrap_or(255.0) - local.get(i).copied().unwrap_or(255.0);
                [(255.0 + d * 6.0).clamp(0.0, 255.0); 3]
            })
        }
        "glowEdges" => {
            let soft = blur_rgb(&rgba, unit);
            let planes: Vec<Vec<f32>> =
                (0..3).map(|ch| strength(sobel(&soft.pixels().map(|p| f32::from(p.0[ch])).collect::<Vec<_>>(), w, h))).collect();
            map(&rgba, |i, _| [0, 1, 2].map(|ch| planes.get(ch).and_then(|p| p.get(i)).copied().unwrap_or(0.0).min(1.0) * 255.0))
        }
        _ => rgba,
    };
    DynamicImage::ImageRgba8(out)
}

/// `img` as RGBA, shrunk to at most [`EFFECT_PIXELS`].
fn bounded(img: DynamicImage) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let n = u64::from(w) * u64::from(h);
    if n <= EFFECT_PIXELS {
        return img.into_rgba8();
    }
    let k = (EFFECT_PIXELS as f64 / n as f64).sqrt();
    let (nw, nh) = (((f64::from(w) * k) as u32).max(1), ((f64::from(h) * k) as u32).max(1));
    // Clamping a thin picture's short side to one can exceed the processing budget.
    let nh = nh.min((EFFECT_PIXELS / u64::from(nw)) as u32).max(1);
    let nw = nw.min((EFFECT_PIXELS / u64::from(nh)) as u32).max(1);
    image::imageops::thumbnail(&img.into_rgba8(), nw, nh)
}

trait Zip3 {
    fn zip_map(self, other: [f32; 3], f: impl Fn(f32, f32) -> f32) -> [f32; 3];
}

impl Zip3 for [f32; 3] {
    fn zip_map(self, other: [f32; 3], f: impl Fn(f32, f32) -> f32) -> [f32; 3] {
        [f(self[0], other[0]), f(self[1], other[1]), f(self[2], other[2])]
    }
}

fn rgb(p: &image::Rgba<u8>) -> [f32; 3] {
    [f32::from(p.0[0]), f32::from(p.0[1]), f32::from(p.0[2])]
}

/// Each pixel's colour from its index and colour; alpha kept.
fn map(img: &RgbaImage, f: impl Fn(usize, [f32; 3]) -> [f32; 3]) -> RgbaImage {
    let mut out = img.clone();
    for (i, p) in out.pixels_mut().enumerate() {
        let c = f(i, rgb(p));
        p.0 = [c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8, p.0[3]];
    }
    out
}

/// Each pixel's colour from the same pixel of `a` and `b` (the same size); `a`'s alpha kept.
fn map2(a: &RgbaImage, b: &RgbaImage, f: impl Fn([f32; 3], [f32; 3]) -> [f32; 3]) -> RgbaImage {
    let mut out = a.clone();
    for (p, q) in out.pixels_mut().zip(b.pixels()) {
        let c = f(rgb(p), rgb(q));
        p.0 = [c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8, p.0[3]];
    }
    out
}

fn saturate(img: &RgbaImage, k: f32) -> RgbaImage {
    map(img, |_, c| {
        let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        c.map(|v| l + (v - l) * k)
    })
}

fn luma(img: &RgbaImage) -> Vec<f32> {
    img.pixels().map(|p| 0.299 * f32::from(p.0[0]) + 0.587 * f32::from(p.0[1]) + 0.114 * f32::from(p.0[2])).collect()
}

/// A repeatable value in 0..1 for a pixel.
fn noise(x: u32, y: u32, seed: u32) -> f32 {
    let mut v = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_246_822_519);
    v = (v ^ (v >> 13)).wrapping_mul(1_274_126_177);
    ((v ^ (v >> 16)) & 0xffff) as f32 / 65535.0
}

/// [`noise`] on a grid of `cell` pixels, blended between the grid's points.
fn smooth_noise(x: u32, y: u32, cell: f32, seed: u32) -> f32 {
    let cell = cell.max(1.0);
    let (fx, fy) = (x as f32 / cell, y as f32 / cell);
    let (gx, gy) = (fx as u32, fy as u32);
    let (tx, ty) = (fx - gx as f32, fy - gy as f32);
    let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let n = |a: u32, b: u32| noise(a, b, seed);
    let top = n(gx, gy) * (1.0 - tx) + n(gx + 1, gy) * tx;
    let bottom = n(gx, gy + 1) * (1.0 - tx) + n(gx + 1, gy + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Box blur of a `w` × `h` plane, twice (close to a Gaussian), radius `r`.
fn blur(src: &[f32], w: u32, h: u32, r: usize) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let once = |src: &[f32]| {
        let rows = box_pass(src, w, h, w, 1, r);
        box_pass(&rows, h, w, 1, w, r)
    };
    once(&once(src))
}

/// One running-mean pass along `lines` lines of `len` values, `stride` apart, `step` between
/// neighbours.
fn box_pass(src: &[f32], len: usize, lines: usize, stride: usize, step: usize, r: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; src.len()];
    let mut prefix = vec![0.0f64; len + 1];
    for line in 0..lines {
        let at = |k: usize| line * stride + k * step;
        for k in 0..len {
            let before = prefix.get(k).copied().unwrap_or(0.0);
            if let Some(p) = prefix.get_mut(k + 1) {
                *p = before + f64::from(src.get(at(k)).copied().unwrap_or(0.0));
            }
        }
        for k in 0..len {
            let (lo, hi) = (k.saturating_sub(r), (k + r + 1).min(len));
            let sum = prefix.get(hi).copied().unwrap_or(0.0) - prefix.get(lo).copied().unwrap_or(0.0);
            if let Some(o) = out.get_mut(at(k)) {
                *o = (sum / (hi - lo).max(1) as f64) as f32;
            }
        }
    }
    out
}

fn blur_rgb(img: &RgbaImage, r: usize) -> RgbaImage {
    let (w, h) = img.dimensions();
    let planes: Vec<Vec<f32>> = (0..3).map(|ch| blur(&img.pixels().map(|p| f32::from(p.0[ch])).collect::<Vec<_>>(), w, h, r)).collect();
    map(img, |i, _| [0, 1, 2].map(|ch| planes.get(ch).and_then(|p| p.get(i)).copied().unwrap_or(0.0)))
}

/// Sobel gradient magnitude of a `w` × `h` plane (edges clamped).
fn sobel(src: &[f32], w: u32, h: u32) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let at = |x: usize, y: usize| src.get(y.min(h - 1) * w + x.min(w - 1)).copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (l, r, u, d) = (x.saturating_sub(1), x + 1, y.saturating_sub(1), y + 1);
            let gx = at(r, u) + 2.0 * at(r, y) + at(r, d) - at(l, u) - 2.0 * at(l, y) - at(l, d);
            let gy = at(l, d) + 2.0 * at(x, d) + at(r, d) - at(l, u) - 2.0 * at(x, u) - at(r, u);
            out.push((gx * gx + gy * gy).sqrt() / 4.0);
        }
    }
    out
}

/// Edge magnitudes over the picture's own strong edges (its 95th percentile): 1 is a strong edge
/// for this picture, so a soft picture draws as many lines as a sharp one.
fn strength(mut edges: Vec<f32>) -> Vec<f32> {
    let mut sorted = edges.clone();
    let k = (sorted.len() * 95 / 100).min(sorted.len().saturating_sub(1));
    let top = match sorted.get(k) {
        Some(_) => *sorted.select_nth_unstable_by(k, f32::total_cmp).1,
        None => 1.0,
    };
    let top = top.max(4.0);
    for e in &mut edges {
        *e /= top;
    }
    edges
}

/// The picture in its `n` most typical colours (k-means on at most 4096 of its pixels, starting
/// from brightness quantiles, so the result is always the same).
fn palette(img: &RgbaImage, n: usize) -> RgbaImage {
    let all: Vec<[f32; 3]> = img.pixels().map(rgb).collect();
    let step = (all.len() / 4096).max(1);
    let mut sample: Vec<[f32; 3]> = all.iter().copied().step_by(step).collect();
    if sample.is_empty() {
        return img.clone();
    }
    let luma3 = |c: &[f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    sample.sort_by(|a, b| luma3(a).total_cmp(&luma3(b)));
    let n = n.clamp(1, sample.len());
    let mut centres: Vec<[f32; 3]> = sample
        .chunks(sample.len().div_ceil(n))
        .map(|c| {
            let k = c.len().max(1) as f32;
            c.iter().fold([0.0f32; 3], |a, z| [a[0] + z[0] / k, a[1] + z[1] / k, a[2] + z[2] / k])
        })
        .collect();
    let nearest = |centres: &[[f32; 3]], z: [f32; 3]| {
        let d = |c: &[f32; 3]| (c[0] - z[0]).powi(2) + (c[1] - z[1]).powi(2) + (c[2] - z[2]).powi(2);
        centres.iter().enumerate().min_by(|a, b| d(a.1).total_cmp(&d(b.1))).map_or(0, |(i, _)| i)
    };
    for _ in 0..8 {
        let mut sums = vec![([0.0f32; 3], 0u32); centres.len()];
        for z in &sample {
            if let Some((s, k)) = sums.get_mut(nearest(&centres, *z)) {
                for (a, v) in s.iter_mut().zip(z) {
                    *a += v;
                }
                *k += 1;
            }
        }
        for (c, (s, k)) in centres.iter_mut().zip(sums) {
            if k > 0 {
                *c = s.map(|v| v / k as f32);
            }
        }
    }
    map(img, |i, c| all.get(i).map_or(c, |z| centres.get(nearest(&centres, *z)).copied().unwrap_or(c)))
}

/// Light from the top left on a `w` × `h` height field: positive where it faces the light.
fn emboss(src: &[f32], w: u32, h: u32) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let at = |x: usize, y: usize| src.get(y.min(h - 1) * w + x.min(w - 1)).copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            out.push(at(x.saturating_sub(1), y.saturating_sub(1)) - at(x + 1, y + 1));
        }
    }
    out
}

/// Pencil sketch: the brightness divided by its blurred negative (a colour dodge), 0 to 255.
fn sketch(img: &RgbaImage, r: usize) -> Vec<f32> {
    let (w, h) = img.dimensions();
    let l = luma(img);
    let inv: Vec<f32> = l.iter().map(|v| 255.0 - v).collect();
    let soft = blur(&inv, w, h, r);
    l.iter().zip(soft).map(|(v, b)| (v * 255.0 / (255.0 - b).max(1.0)).min(255.0)).collect()
}

/// Kuwahara filter of radius `r`: each pixel takes the mean colour of the calmest of the four
/// boxes it is a corner of, so areas flatten while edges stay (a painted look). Summed-area
/// tables keep it linear in the picture's size.
fn kuwahara(img: &RgbaImage, r: usize) -> RgbaImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let stride = w + 1;
    // Sums of r, g, b, brightness and brightness squared over [0, x) × [0, y).
    let mut sums = vec![[0.0f64; 5]; stride * (h + 1)];
    for y in 0..h {
        let mut row = [0.0f64; 5];
        for x in 0..w {
            let c = img.get_pixel_checked(x as u32, y as u32).map(rgb).unwrap_or_default();
            let l = f64::from(0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]);
            let v = [f64::from(c[0]), f64::from(c[1]), f64::from(c[2]), l, l * l];
            for (a, b) in row.iter_mut().zip(v) {
                *a += b;
            }
            let above = sums.get(y * stride + x + 1).copied().unwrap_or_default();
            if let Some(s) = sums.get_mut((y + 1) * stride + x + 1) {
                for k in 0..5 {
                    s[k] = above[k] + row[k];
                }
            }
        }
    }
    let at = |x: usize, y: usize| sums.get(y * stride + x).copied().unwrap_or_default();
    // The sums over [x0, x1) × [y0, y1).
    let boxed = |x0: usize, y0: usize, x1: usize, y1: usize| {
        let (a, b, c, d) = (at(x1, y1), at(x0, y1), at(x1, y0), at(x0, y0));
        let n = ((x1 - x0) * (y1 - y0)).max(1) as f64;
        [0, 1, 2, 3, 4].map(|k| (a[k] - b[k] - c[k] + d[k]) / n)
    };
    let mut out = img.clone();
    for (x, y, p) in out.enumerate_pixels_mut() {
        let (x, y) = (x as usize, y as usize);
        let (x0, y0, x1, y1) = (x.saturating_sub(r), y.saturating_sub(r), (x + r + 1).min(w), (y + r + 1).min(h));
        let quads = [boxed(x0, y0, x + 1, y + 1), boxed(x, y0, x1, y + 1), boxed(x0, y, x + 1, y1), boxed(x, y, x1, y1)];
        let calm = quads.iter().min_by(|a, b| (a[4] - a[3] * a[3]).total_cmp(&(b[4] - b[3] * b[3])));
        if let Some(q) = calm {
            p.0 = [q[0].clamp(0.0, 255.0) as u8, q[1].clamp(0.0, 255.0) as u8, q[2].clamp(0.0, 255.0) as u8, p.0[3]];
        }
    }
    out
}

/// Mosaic Bubbles: the picture in cells of `cell` pixels, each a lit disc of its mean colour.
fn bubbles(img: &RgbaImage, cell: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (cw, ch) = (w.div_ceil(cell), h.div_ceil(cell));
    let small = image::imageops::resize(img, cw.max(1), ch.max(1), image::imageops::FilterType::Triangle);
    let r = cell as f32 * 0.48;
    RgbaImage::from_fn(w, h, |x, y| {
        let (gx, gy) = (x / cell, y / cell);
        let c = small.get_pixel_checked(gx, gy).map(rgb).unwrap_or_default();
        let (cx, cy) = ((gx * cell) as f32 + cell as f32 / 2.0, (gy * cell) as f32 + cell as f32 / 2.0);
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        let d = (dx * dx + dy * dy).sqrt() / r;
        let a = img.get_pixel_checked(x, y).map_or(255, |p| p.0[3]);
        let out = if d <= 1.0 {
            // Lit from the top left, darker towards the rim.
            let light = (1.0 - ((dx + r * 0.35).powi(2) + (dy + r * 0.35).powi(2)).sqrt() / r).max(0.0);
            c.map(|v| v * (1.0 - 0.25 * d * d) + (255.0 - v) * 0.5 * light * light)
        } else {
            c.map(|v| v * 0.35)
        };
        image::Rgba([out[0].clamp(0.0, 255.0) as u8, out[1].clamp(0.0, 255.0) as u8, out[2].clamp(0.0, 255.0) as u8, a])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_pictures_stay_within_the_effect_pixel_budget() {
        for (w, h) in [(8_000_000, 1), (1, 8_000_000), (4, 3)] {
            let img = bounded(DynamicImage::ImageRgba8(RgbaImage::new(w, h)));
            assert!(img.width() > 0 && img.height() > 0);
            assert!(img.width() <= w && img.height() <= h);
            assert!(u64::from(img.width()) * u64::from(img.height()) <= EFFECT_PIXELS, "{w}×{h} became {:?}", img.dimensions());
        }
    }
}
