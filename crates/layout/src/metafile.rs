//! Drawing rules the page renderer and the PDF export share for metafile pictures and picture
//! crops, so the two outputs draw the same file the same way.

/// Most dashes and gaps one metafile stroke is split into. A path that a pen's pattern would split
/// into more draws solid.
pub const MAX_METAFILE_DASHES: f64 = 100_000.0;

/// The crop sides a picture is drawn with: each side of `crop` (left, top, right, bottom fractions
/// of the source) at least 0. A negative side draws as no crop, not as padding around the source.
pub fn crop_sides(crop: [f32; 4]) -> [f32; 4] {
    crop.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 })
}

/// The part of a picture's source that `crop` keeps, as fractions of the source: [left, top,
/// width, height] (`craft_metafile::crop_window` of [`crop_sides`]).
pub fn crop_window(crop: [f32; 4]) -> [f64; 4] {
    craft_metafile::crop_window(crop_sides(crop))
}

/// The miter limit a metafile pen is drawn with: its own when it is finite and at least 1,
/// otherwise 10.
pub fn metafile_miter_limit(limit: f32) -> f32 {
    if limit.is_finite() && limit >= 1.0 { limit } else { 10.0 }
}

/// The dash lengths (page points) a metafile pen draws a path `length` points long with, or None
/// for a solid line. `dash` is the placed pen's pattern (`craft_metafile::LineStyle::dash`, page
/// points) and `pixel` is one output pixel in points.
///
/// A cosmetic pen (`cosmetic`: width 0) has its pattern scaled so that its shortest dash or gap is
/// one pixel, at any zoom. A pattern with a negative or non-finite length draws solid, and so does
/// one whose period is shorter than a pixel or that would split the path into more than
/// [`MAX_METAFILE_DASHES`] dashes and gaps.
pub fn metafile_dashes(dash: &[f32], cosmetic: bool, pixel: f64, length: f64) -> Option<Vec<f64>> {
    if dash.is_empty() || !pixel.is_finite() || pixel <= 0.0 {
        return None;
    }
    let mut out: Vec<f64> = dash.iter().map(|d| f64::from(*d)).collect();
    if !out.iter().all(|d| d.is_finite() && *d >= 0.0) {
        return None;
    }
    if cosmetic {
        let shortest = out.iter().copied().filter(|d| *d > 0.0).fold(f64::INFINITY, f64::min);
        if !shortest.is_finite() {
            return None;
        }
        let k = pixel / shortest;
        out.iter_mut().for_each(|d| *d *= k);
    }
    let period: f64 = out.iter().sum();
    if !period.is_finite() || period < pixel {
        return None;
    }
    // Each length of the pattern starts a new dash or gap.
    let pieces = out.len() as f64 * if length.is_finite() { length.max(0.0) / period } else { f64::INFINITY };
    (pieces <= MAX_METAFILE_DASHES).then_some(out)
}

/// The alpha-weighted average color of straight RGBA pixels (transparent when there are none): a
/// metafile pattern fill whose tile is too small to show is filled with it.
pub fn metafile_average_color(px: &[u8]) -> [u8; 4] {
    let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for p in px.as_chunks::<4>().0 {
        let w = u64::from(p[3]);
        r += u64::from(p[0]) * w;
        g += u64::from(p[1]) * w;
        b += u64::from(p[2]) * w;
        a += w;
        n += 1;
    }
    if a == 0 || n == 0 {
        return [0; 4];
    }
    [(r / a) as u8, (g / a) as u8, (b / a) as u8, (a / n) as u8]
}
