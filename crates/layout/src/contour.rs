//! Contour wrapping: the outline Tight and Through wrapping follow around a floating object
//! (ECMA-376 Part 1, §20.4.2.17 `wrapThrough`, §20.4.2.18 `wrapTight`), and the picture
//! outlines worked out from transparency.
//!
//! A [`Contour`] is built once per placed object; each line then asks it which stretches of
//! the line's band it takes ([`Contour::occupied`]), a walk over its edges.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use wordcraft_doc::para::{Float, InlineObject, Wrap};
use wordcraft_doc::wrap::{ALPHA_GRID, MAX_WRAP_POINTS, alpha_outline, shape_outline};
use wordcraft_geom::{Rect, finite};

/// A wrap outline placed on the page, in the coordinates of its wrap area's top-left corner
/// (see [`crate::para::Exclusion`]).
#[derive(Clone, PartialEq)]
pub struct Contour {
    /// The object's outline (rotated and flipped as the object is), closed.
    pts: Vec<(f32, f32)>,
    /// Distance from text at the left and right, above and below.
    pad_x: f32,
    pad_top: f32,
    pad_bottom: f32,
    /// Through wrapping: text may also go into gaps inside the outline.
    through: bool,
    /// A digest of the above (paragraph layout cache keys print it instead of the points).
    id: u64,
}

impl std::fmt::Debug for Contour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Contour#{:x}", self.id)
    }
}

impl Contour {
    /// A contour around `pts` (page coordinates; non-finite points are dropped, at most
    /// [`MAX_WRAP_POINTS`]) kept `pad_x` from text at the sides and `pad_top` / `pad_bottom`
    /// above and below. Returns the wrap area (the outline's bounds grown by the distances)
    /// and the contour relative to its top-left; `None` without at least three points.
    pub fn new(pts: &[(f32, f32)], pad_x: f32, pad_top: f32, pad_bottom: f32, through: bool) -> Option<(Rect, Contour)> {
        let pts: Vec<(f32, f32)> = pts.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).take(MAX_WRAP_POINTS).collect();
        if pts.len() < 3 {
            return None;
        }
        let pad = |v: f32| finite(v).clamp(0.0, 1584.0);
        let (pad_x, pad_top, pad_bottom) = (pad(pad_x), pad(pad_top), pad(pad_bottom));
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in &pts {
            (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
        }
        let area = Rect::new(x0 - pad_x, y0 - pad_top, x1 - x0 + 2.0 * pad_x, y1 - y0 + pad_top + pad_bottom);
        let pts: Vec<(f32, f32)> = pts.into_iter().map(|(x, y)| (x - area.x, y - area.y)).collect();
        Some((area, Contour::with_id(pts, pad_x, pad_top, pad_bottom, through)))
    }

    fn with_id(pts: Vec<(f32, f32)>, pad_x: f32, pad_top: f32, pad_bottom: f32, through: bool) -> Contour {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (x, y) in &pts {
            (x.to_bits(), y.to_bits()).hash(&mut h);
        }
        (pad_x.to_bits(), pad_top.to_bits(), pad_bottom.to_bits(), through).hash(&mut h);
        Contour { pts, pad_x, pad_top, pad_bottom, through, id: h.finish() }
    }

    /// This contour mirrored left to right in a wrap area `width` wide (right-to-left text).
    pub fn mirrored(&self, width: f32) -> Contour {
        let pts = self.pts.iter().map(|(x, y)| (width - x, *y)).collect();
        Contour::with_id(pts, self.pad_x, self.pad_top, self.pad_bottom, self.through)
    }

    /// The stretches (x0, x1), left to right, that the object and its distances from text take
    /// of the band `top..bottom` (both relative to the wrap area's top-left). Tight: one
    /// stretch, from the outline's leftmost to its rightmost point in the band. Through: the
    /// outline's own pieces, so text can go into the gaps between them. Empty when the band
    /// misses the outline.
    pub fn occupied(&self, top: f32, bottom: f32) -> Vec<(f32, f32)> {
        // Distance above the object reaches down to bands above it, and the other way round.
        let (y0, y1) = (top - self.pad_bottom, bottom + self.pad_top);
        if !(y0.is_finite() && y1.is_finite()) || y1 < y0 {
            return Vec::new();
        }
        let n = self.pts.len();
        let mut spans: Vec<(f32, f32)> = Vec::new();
        // The outline's part in the band is bounded by its edges (clipped to the band) and by
        // the band's top and bottom lines where they're inside it: what those cover across is
        // what the outline covers.
        for (i, &(ax, ay)) in self.pts.iter().enumerate() {
            let Some(&(bx, by)) = self.pts.get((i + 1) % n) else { continue };
            let (lo, hi) = (ay.min(by), ay.max(by));
            if hi < y0 || lo > y1 {
                continue;
            }
            if (by - ay).abs() < 1e-6 {
                spans.push((ax.min(bx), ax.max(bx)));
                continue;
            }
            let x_at = |y: f32| ax + (bx - ax) * (y - ay) / (by - ay);
            let (xa, xb) = (x_at(lo.max(y0)), x_at(hi.min(y1)));
            spans.push((xa.min(xb), xa.max(xb)));
        }
        for y in [y0, y1] {
            let mut xs: Vec<f32> = Vec::new();
            for (i, &(ax, ay)) in self.pts.iter().enumerate() {
                let Some(&(bx, by)) = self.pts.get((i + 1) % n) else { continue };
                if (ay <= y) != (by <= y) {
                    xs.push(ax + (bx - ax) * (y - ay) / (by - ay));
                }
            }
            xs.sort_by(f32::total_cmp);
            spans.extend(xs.as_chunks::<2>().0.iter().map(|[a, b]| (*a, *b)));
        }
        if spans.is_empty() {
            return spans;
        }
        // Distance from text at the sides widens each piece; pieces that then touch merge.
        let mut spans: Vec<(f32, f32)> = spans.into_iter().map(|(a, b)| (a - self.pad_x, b + self.pad_x)).collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f32, f32)> = Vec::with_capacity(spans.len());
        for (a, b) in spans {
            match merged.last_mut() {
                Some(last) if a <= last.1 + 0.01 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        if !self.through && merged.len() > 1 {
            let (a, b) = (merged.first().map_or(0.0, |s| s.0), merged.last().map_or(0.0, |s| s.1));
            return vec![(a, b)];
        }
        merged
    }
}

/// The outline (unit coordinates across its frame, unrotated) that Tight and Through wrapping
/// follow around `o`: its wrap polygon, a shape's geometry, or a picture's opaque pixels.
/// `None`: its rectangle (or it isn't wrapped that way). `picture` works out a picture's
/// outline from its media key and crop.
pub fn wrap_outline(o: &InlineObject, picture: impl FnOnce(&str, [f32; 4]) -> Option<Arc<Vec<(f32, f32)>>>) -> Option<Arc<Vec<(f32, f32)>>> {
    let (w, h, float) = o.frame()?;
    if !matches!(float.wrap, Wrap::Tight | Wrap::Through) {
        return None;
    }
    if let Some(p) = &float.wrap_polygon {
        return Some(Arc::new(p.unit()));
    }
    match o {
        InlineObject::Shape { kind, .. } => shape_outline(*kind, w, h).map(Arc::new),
        InlineObject::Image { media, crop, .. } => picture(media, *crop),
        _ => None,
    }
}

/// The wrap area of a Tight or Through object at `r` (its unrotated frame) following `outline`
/// (unit coordinates), turned and mirrored as the object is.
pub fn contour_area(r: Rect, float: &Float, outline: &[(f32, f32)]) -> Option<(Rect, Contour)> {
    let d = |v: f32| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 };
    let spin = float.spin();
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let pts: Vec<(f32, f32)> = outline.iter().take(MAX_WRAP_POINTS).map(|(u, v)| spin.apply(cx, cy, r.x + u * r.w, r.y + v * r.h)).collect();
    Contour::new(&pts, d(float.dist), d(float.dist_top), d(float.dist_bottom), float.wrap == Wrap::Through)
}

/// Largest picture (each side, pixels) whose transparency is looked at.
const MAX_PICTURE_SIDE: u32 = 8192;

/// The outline of a picture's opaque pixels within `crop` (left, top, right, bottom fractions),
/// in unit coordinates of the cropped picture. `None` for pictures without transparency (or
/// that don't decode): their rectangle is their outline.
pub fn picture_outline(bytes: &[u8], crop: [f32; 4]) -> Option<Vec<(f32, f32)>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    // Formats that can't be transparent aren't decoded.
    if matches!(reader.format(), None | Some(image::ImageFormat::Jpeg)) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PICTURE_SIDE);
    limits.max_image_height = Some(MAX_PICTURE_SIDE);
    limits.max_alloc = Some(512 << 20);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    if !img.color().has_alpha() {
        return None;
    }
    let img = img.into_rgba8();
    let (iw, ih) = img.dimensions();
    let frac = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let [cl, ct, cr, cb] = crop.map(frac);
    let x0 = ((iw as f32 * cl) as u32).min(iw);
    let y0 = ((ih as f32 * ct) as u32).min(ih);
    let x1 = ((iw as f32 * (1.0 - cr)) as u32).clamp(x0, iw);
    let y1 = ((ih as f32 * (1.0 - cb)) as u32).clamp(y0, ih);
    let (cw, ch) = (x1 - x0, y1 - y0);
    if cw == 0 || ch == 0 {
        return None;
    }
    // A grid of cells at most ALPHA_GRID a side; a cell is opaque when a pixel sampled in it is.
    let cell = cw.max(ch).div_ceil(ALPHA_GRID as u32).max(1);
    let (gw, gh) = (cw.div_ceil(cell), ch.div_ceil(cell));
    let step = (cell / 8).max(1);
    let opaque = |gx: usize, gy: usize| {
        let (px0, py0) = (x0 + gx as u32 * cell, y0 + gy as u32 * cell);
        let (px1, py1) = ((px0 + cell).min(x1), (py0 + cell).min(y1));
        (py0..py1)
            .step_by(step as usize)
            .any(|y| (px0..px1).step_by(step as usize).any(|x| img.get_pixel_checked(x, y).is_some_and(|p| p.0[3] >= 128)))
    };
    let pts = alpha_outline(gw as usize, gh as usize, opaque)?;
    // The grid may reach a little past the picture (its last cells are partial).
    let (sx, sy) = ((gw * cell) as f32 / cw as f32, (gh * cell) as f32 / ch as f32);
    Some(pts.into_iter().map(|(x, y)| ((x * sx).min(1.0), (y * sy).min(1.0))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diamond(through: bool) -> Contour {
        Contour::new(&[(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)], 0.0, 0.0, 0.0, through).unwrap().1
    }

    #[test]
    fn occupied_follows_the_outline() {
        let c = diamond(false);
        let s = c.occupied(0.0, 10.0);
        assert_eq!(s.len(), 1);
        assert!((s[0].0 - 40.0).abs() < 0.01 && (s[0].1 - 60.0).abs() < 0.01, "{s:?}");
        let s = c.occupied(45.0, 55.0);
        assert!((s[0].0 - 0.0).abs() < 0.01 && (s[0].1 - 100.0).abs() < 0.01, "{s:?}");
        assert!(c.occupied(120.0, 130.0).is_empty());
        // A band wholly inside the outline.
        let big = Contour::new(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 5.0, 0.0, 0.0, false).unwrap().1;
        assert_eq!(big.occupied(40.0, 50.0), vec![(0.0, 110.0)]);
    }

    #[test]
    fn through_keeps_interior_gaps() {
        // A U: two prongs joined at the bottom.
        let u = [(0.0, 0.0), (20.0, 0.0), (20.0, 80.0), (80.0, 80.0), (80.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        let (_, tight) = Contour::new(&u, 0.0, 0.0, 0.0, false).unwrap();
        let (_, through) = Contour::new(&u, 0.0, 0.0, 0.0, true).unwrap();
        assert_eq!(tight.occupied(10.0, 20.0).len(), 1);
        assert_eq!(through.occupied(10.0, 20.0).len(), 2);
        assert_eq!(through.occupied(85.0, 90.0).len(), 1);
    }

    #[test]
    fn pictures_wrap_around_their_opaque_pixels() {
        let png = |img: image::DynamicImage| {
            let mut buf = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
            buf
        };
        // An opaque disc in the middle of a transparent 64 × 64 picture.
        let disc = image::RgbaImage::from_fn(64, 64, |x, y| {
            let (dx, dy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
            image::Rgba([0, 0, 0, if dx * dx + dy * dy < 400.0 { 255 } else { 0 }])
        });
        let pts = picture_outline(&png(disc.clone().into()), [0.0; 4]).unwrap();
        assert!(pts.len() >= 8 && pts.len() <= wordcraft_doc::wrap::MAX_DERIVED_POINTS);
        assert!(pts.iter().all(|(x, y)| (0.15..=0.85).contains(x) && (0.15..=0.85).contains(y)), "{pts:?}");
        // Cropped to its left half, the disc reaches the right edge.
        let half = picture_outline(&png(disc.into()), [0.0, 0.0, 0.5, 0.0]).unwrap();
        assert!(half.iter().any(|(x, _)| *x > 0.99), "{half:?}");
        // No transparency: the rectangle.
        assert!(picture_outline(&png(image::RgbImage::new(8, 8).into()), [0.0; 4]).is_none());
    }

    #[test]
    fn hostile_points_are_dropped() {
        assert!(Contour::new(&[(f32::NAN, 0.0), (1.0, 1.0), (f32::INFINITY, 2.0)], 0.0, 0.0, 0.0, false).is_none());
        let c = diamond(true);
        assert!(c.occupied(f32::NAN, 1.0).is_empty());
        assert!(picture_outline(b"not a picture", [0.0; 4]).is_none());
    }
}
