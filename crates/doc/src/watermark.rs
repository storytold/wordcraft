//! Design › Watermark › Picture watermark: a picture drawn behind the text of every page.
//!
//! The picture's size is shared by everything that draws or saves it. A scale is a percentage of
//! the picture's own size (its pixels at 96 ppi). Auto is as large as fits inside the margins of
//! the first section that names the page's header ([`Document::watermark_fit`]), the same
//! on screen, in a PDF and in the saved file, whatever pages follow. The picture is centred in
//! the page's own text area, so mirrored margins move it as they move the text.

use serde::{Deserialize, Serialize};
use wordcraft_geom::Rect;

use crate::Document;
use crate::picfx::PicFx;
use crate::section::SectionProps;

/// Largest picture watermark scale, percent (the top of Word's Scale list).
pub const MAX_SCALE: f32 = 500.0;

/// Smallest picture watermark scale, percent.
pub const MIN_SCALE: f32 = 1.0;

/// Longest side of a placed picture watermark, points.
const MAX_SIDE: f32 = 31_680.0;

/// Word's Washout as `v:imagedata` values: `gain` 0.3 and `blacklevel` 0.35, in 65536ths.
pub const WASHOUT_GAIN: f64 = 19_661.0 / 65_536.0;
pub const WASHOUT_BLACK_LEVEL: f64 = 22_938.0 / 65_536.0;

/// A picture's own size in points (its pixels at 96 ppi), from its header: nothing is decoded.
/// None when it isn't a picture WordCraft reads.
pub fn own_size(bytes: &[u8]) -> Option<(f32, f32)> {
    let p = wordcraft_geom::picture::probe(bytes).ok()?;
    (p.width > 0 && p.height > 0).then_some((p.width as f32 * 0.75, p.height as f32 * 0.75))
}

/// A picture watermark.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatermarkPicture {
    /// Key into [`crate::Document::media`].
    pub media: String,
    /// Percent of the picture's own size (its pixels at 96 ppi); None for Auto, as large as fits
    /// inside the margins.
    pub scale: Option<f32>,
    /// Word's Washout: the picture faded so the text over it can be read.
    pub washout: bool,
}

impl Default for WatermarkPicture {
    fn default() -> Self {
        WatermarkPicture { media: String::new(), scale: None, washout: true }
    }
}

impl WatermarkPicture {
    /// The corrections the picture is drawn with: Word's Washout, else none. The renderers draw
    /// it through the same picture path as any picture with corrections, so Washout costs no
    /// more memory than the picture itself.
    pub fn fx(&self) -> PicFx {
        if self.washout { PicFx::from_vml(WASHOUT_GAIN, WASHOUT_BLACK_LEVEL) } else { PicFx::default() }
    }

    /// Whether `fx` (read from a file's `gain` and `blacklevel`) is Word's Washout, to within
    /// rounding of the values a file writes.
    pub fn is_washout(fx: PicFx) -> bool {
        let w = PicFx::from_vml(WASHOUT_GAIN, WASHOUT_BLACK_LEVEL);
        fx.transparency == 0 && fx.bright.abs_diff(w.bright) <= 100 && fx.contrast.abs_diff(w.contrast) <= 100
    }

    /// The picture's size in points, for a picture `own_w` × `own_h` points: a scale's percentage
    /// of it, or at Auto as large as fits `fit` (width, height). None for an empty or unusable
    /// size.
    pub fn size(&self, own_w: f32, own_h: f32, fit: (f32, f32)) -> Option<(f32, f32)> {
        let pos = |v: f32| v.is_finite() && v > 0.0;
        if !(pos(own_w) && pos(own_h)) {
            return None;
        }
        let k = match self.scale {
            Some(s) if s.is_finite() => s.clamp(MIN_SCALE, MAX_SCALE) / 100.0,
            _ => {
                if !(pos(fit.0) && pos(fit.1)) {
                    return None;
                }
                (fit.0 / own_w).min(fit.1 / own_h)
            }
        };
        let k = k.min(MAX_SIDE / own_w.max(own_h));
        let (w, h) = (own_w * k, own_h * k);
        (pos(w) && pos(h)).then_some((w, h))
    }

    /// Where the picture goes: [`WatermarkPicture::size`], centred in `body`, the page's text
    /// area (a scale may spill past it).
    pub fn place(&self, own_w: f32, own_h: f32, fit: (f32, f32), body: Rect) -> Option<Rect> {
        let (w, h) = self.size(own_w, own_h, fit)?;
        let ok = |v: f32| v.is_finite();
        if !(ok(body.x) && ok(body.y) && ok(body.w) && ok(body.h)) {
            return None;
        }
        Some(Rect::new(body.x + (body.w - w) / 2.0, body.y + (body.h - h) / 2.0, w, h))
    }
}

/// The room Auto has in a section: its text area's width and height.
fn fit_of(s: &SectionProps) -> (f32, f32) {
    (s.text_width(), s.text_height())
}

impl Document {
    /// The room (width, height; points) a picture watermark at Auto fills on a page showing
    /// header `header` (None: no header, which the saved file gives a header of its own to hold
    /// the watermark): that of the first section naming the header, in document order, as the
    /// DOCX writer and reader do. A header no section names is sized by the last section.
    pub fn watermark_fit(&self, header: Option<u32>) -> (f32, f32) {
        let even = self.settings.even_odd_headers;
        // Which slots an earlier section has filled (named, or given the watermark's own header),
        // that later ones inherit.
        let mut filled = [false; 3];
        for (_, s) in self.sections() {
            let own = [s.headers.default, s.headers.first, s.headers.even];
            let wants = [true, s.title_page, even];
            let mut hit = false;
            for ((o, w), f) in own.into_iter().zip(wants).zip(filled.iter_mut()) {
                match (header, o) {
                    (Some(h), Some(o)) => hit |= h == o,
                    // A slot the pages show with no header of its own, nor an earlier one's.
                    (None, None) => hit |= w && !*f,
                    _ => {}
                }
                *f |= o.is_some() || w;
            }
            if hit {
                return fit_of(s);
            }
        }
        fit_of(&self.last_section)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rounded to a hundredth of a point: (x, y, w, h).
    fn r(r: Option<Rect>) -> Option<(f32, f32, f32, f32)> {
        let q = |v: f32| (v * 100.0).round() / 100.0;
        r.map(|r| (q(r.x), q(r.y), q(r.w), q(r.h)))
    }

    #[test]
    fn picture_is_placed_in_the_text_area() {
        let body = Rect::new(72.0, 72.0, 468.0, 648.0);
        let fit = (468.0, 648.0);
        // Auto: as large as fits, centred.
        let auto = WatermarkPicture::default();
        assert_eq!(r(auto.place(200.0, 100.0, fit, body)), Some((72.0, 279.0, 468.0, 234.0)));
        assert_eq!(r(auto.place(100.0, 400.0, fit, body)), Some((225.0, 72.0, 162.0, 648.0)));
        // A scale is a percentage of the picture's own size, and may spill past the margins.
        let half = WatermarkPicture { scale: Some(50.0), ..Default::default() };
        assert_eq!(r(half.place(200.0, 100.0, fit, body)), Some((256.0, 371.0, 100.0, 50.0)));
        let big = WatermarkPicture { scale: Some(500.0), ..Default::default() };
        assert_eq!(r(big.place(200.0, 100.0, fit, body)), Some((-194.0, 146.0, 1000.0, 500.0)));
        // Mirrored margins move the page's text area, and the picture with it; Auto's size is
        // still the section's.
        let mirrored = Rect::new(108.0, 72.0, 468.0, 648.0);
        assert_eq!(r(auto.place(200.0, 100.0, fit, mirrored)), Some((108.0, 279.0, 468.0, 234.0)));
        // Hostile sizes and scales.
        let wild = WatermarkPicture { scale: Some(1e30), ..Default::default() };
        assert_eq!(r(wild.place(200.0, 100.0, fit, body)).map(|r| r.2), Some(1000.0));
        let tiny = WatermarkPicture { scale: Some(-5.0), ..Default::default() };
        assert_eq!(r(tiny.place(200.0, 100.0, fit, body)).map(|r| r.2), Some(2.0));
        let nan = WatermarkPicture { scale: Some(f32::NAN), ..Default::default() };
        assert_eq!(r(nan.place(200.0, 100.0, fit, body)).map(|r| r.2), Some(468.0));
        assert!(auto.place(0.0, 100.0, fit, body).is_none());
        assert!(auto.place(f32::INFINITY, 100.0, fit, body).is_none());
        assert!(auto.place(200.0, 100.0, (-1.0, 10.0), body).is_none());
        let huge = auto.size(1e30, 1e30, (1e38, 1e38)).unwrap();
        assert!(huge.0 <= MAX_SIDE + 1.0 && huge.1 <= MAX_SIDE + 1.0);
    }

    #[test]
    fn washout_is_the_vml_values() {
        let on = WatermarkPicture::default();
        assert_eq!(on.fx(), PicFx::from_vml(WASHOUT_GAIN, WASHOUT_BLACK_LEVEL));
        assert!(WatermarkPicture::is_washout(on.fx()) && WatermarkPicture::is_washout(PicFx::from_vml(0.3, 0.35)));
        assert!(!WatermarkPicture::is_washout(PicFx::default()));
        assert!(!WatermarkPicture::is_washout(PicFx::from_vml(0.5, 0.1)));
        assert!(WatermarkPicture { washout: false, ..on }.fx().is_identity());
    }
}
