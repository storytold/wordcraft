//! Picture colour corrections read from a file: brightness, contrast and transparency
//! (DrawingML `a:lum` and `a:alphaModFix` inside `a:blip`, ECMA-376 §20.1.8.42 and §20.1.8.4;
//! VML `v:imagedata` `gain` and `blacklevel`, which Word's picture watermark "Washout" sets).
//!
//! Values are kept in OOXML units: thousandths of a percent, so 100 000 is 100 %. All zero is
//! the picture as it is.
//!
//! The pixel formula was measured on Word (#1711, #1766): Word bakes the corrected pixels into
//! the picture it embeds in a PDF export, so a synthetic grey and colour ramp exported with
//! 20 `a:lum` combinations and 6 VML `gain`/`blacklevel` pairs gives the curve. Every channel is
//! mapped on its own (no hue or luminance coupling) by one straight line, clamped to 0..=255
//! only at the end:
//!
//! `y = s·(x − 127.5) + 127.5 + 127.5·b·(1 + s)`, with `b = bright / 100 000`, `c = contrast /
//! 100 000`, `s = 1 + c` when `c ≤ 0` and `s = 1 / (1 − c)` when `c > 0`
//!
//! (as if half the brightness shift were applied before the contrast and half after). Measured
//! slope and intercept of Word's output for grey input `x` (PNG pictures, exact to ±1 level):
//! bright 20 000 → 1.00, +51; bright −40 000 → 1.00, −103; contrast 80 000 → 5.00, −510;
//! contrast −80 000 → 0.20, +101.4; (40 000, −40 000) → 0.60, +132.2; (80 000, −80 000) →
//! 0.20, +223.8 (also the #1766 header picture: fitted 0.197..0.201, +223.6..224.0);
//! (−40 000, 40 000) → 1.667, −221.7; (30 000, 60 000) → 2.50, −57.8.
//!
//! VML: `gain` is the slope itself and `blacklevel` half the brightness — Word draws `gain` 0.3,
//! `blacklevel` 0.35 (Washout) as 0.30, +205.3, the same as `a:lum` (70 000, −70 000), and
//! `gain` 1, `blacklevel` 0.2 as the very same picture as `bright="40000"`.

use serde::{Deserialize, Serialize};

/// The largest magnitude of a correction (100 %).
pub const FX_MAX: i32 = 100_000;

/// Brightness, contrast and transparency of a picture (see the module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct PicFx {
    /// −100 000 (black) ..= 100 000 (white).
    pub bright: i32,
    /// −100 000 (flat grey) ..= 100 000 (hard threshold).
    pub contrast: i32,
    /// 0 (opaque) ..= 100 000 (invisible).
    pub transparency: i32,
}

impl PicFx {
    /// Corrections clamped to their ranges.
    pub fn new(bright: i32, contrast: i32, transparency: i32) -> PicFx {
        PicFx { bright: bright.clamp(-FX_MAX, FX_MAX), contrast: contrast.clamp(-FX_MAX, FX_MAX), transparency: transparency.clamp(0, FX_MAX) }
    }

    /// The same corrections, each clamped to its range (for values that came from anywhere).
    pub fn clamped(self) -> PicFx {
        PicFx::new(self.bright, self.contrast, self.transparency)
    }

    /// No correction at all.
    pub fn is_identity(&self) -> bool {
        *self == PicFx::default()
    }

    /// Brightness or contrast changes the pixels (transparency alone doesn't).
    pub fn changes_pixels(&self) -> bool {
        let f = self.clamped();
        f.bright != 0 || f.contrast != 0
    }

    /// The picture's opacity, 0..=1 (1 − transparency).
    pub fn opacity(&self) -> f32 {
        1.0 - self.transparency.clamp(0, FX_MAX) as f32 / FX_MAX as f32
    }

    /// The corrections a VML `v:imagedata` gives with `gain` and `blacklevel` (already parsed to
    /// plain numbers, 1 and 0 when missing): contrast from the slope `gain`, brightness twice the
    /// black level (see the module docs). Non-finite values count as missing.
    pub fn from_vml(gain: f64, blacklevel: f64) -> PicFx {
        let gain = if gain.is_finite() { gain.clamp(0.0, 1.0e6) } else { 1.0 };
        let black = if blacklevel.is_finite() { blacklevel.clamp(-0.5, 0.5) } else { 0.0 };
        // s = 1 + c (c ≤ 0) or 1 / (1 − c) (c > 0).
        let c = if gain <= 1.0 { gain - 1.0 } else { 1.0 - 1.0 / gain };
        let fx = |v: f64| (v * FX_MAX as f64).round().clamp(-(FX_MAX as f64), FX_MAX as f64) as i32;
        PicFx::new(fx(2.0 * black), fx(c), 0)
    }

    /// `gain` and `blacklevel` for VML (the inverse of [`PicFx::from_vml`]).
    pub fn to_vml(&self) -> (f64, f64) {
        (self.slope(), self.clamped().bright as f64 / FX_MAX as f64 / 2.0)
    }

    /// The contrast slope `s` (capped at 255: contrast 100 % is a threshold).
    fn slope(&self) -> f64 {
        let c = self.contrast.clamp(-FX_MAX, FX_MAX) as f64 / FX_MAX as f64;
        if c <= 0.0 { 1.0 + c } else { (1.0 / (1.0 - c).max(1.0 / 255.0)).min(255.0) }
    }

    /// The lookup table every colour channel goes through.
    pub fn table(&self) -> [u8; 256] {
        let s = self.slope();
        let b = self.bright.clamp(-FX_MAX, FX_MAX) as f64 / FX_MAX as f64;
        let mut t = [0u8; 256];
        for (x, slot) in t.iter_mut().enumerate() {
            let y = s * (x as f64 - 127.5) + 127.5 + 127.5 * b * (1.0 + s);
            *slot = if y.is_finite() { y.round().clamp(0.0, 255.0) as u8 } else { 0 };
        }
        t
    }

    /// Apply brightness and contrast to straight (not premultiplied) RGBA8 pixels in place. The
    /// alpha byte is left alone (transparency is drawn as an opacity, see [`PicFx::opacity`]); a
    /// trailing partial pixel is ignored.
    pub fn apply_rgba(&self, rgba: &mut [u8]) {
        if !self.changes_pixels() {
            return;
        }
        let t = self.table();
        for px in rgba.as_chunks_mut::<4>().0 {
            for c in px.iter_mut().take(3) {
                *c = t.get(usize::from(*c)).copied().unwrap_or(*c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Word's own output for grey input 0, 128 and 255 (PNG pictures in a PDF export).
    #[test]
    fn matches_word() {
        let cases: [(PicFx, [u8; 3]); 6] = [
            (PicFx::new(40_000, -40_000, 0), [132, 209, 255]),
            (PicFx::new(80_000, -80_000, 0), [224, 249, 255]),
            (PicFx::new(-40_000, 40_000, 0), [0, 0, 203]),
            (PicFx::new(0, 80_000, 0), [0, 130, 255]),
            (PicFx::new(20_000, 0, 0), [51, 179, 255]),
            (PicFx::from_vml(19661.0 / 65536.0, 22938.0 / 65536.0), [205, 244, 255]),
        ];
        for (fx, want) in cases {
            let t = fx.table();
            for (x, w) in [0usize, 128, 255].into_iter().zip(want) {
                let got = t[x];
                assert!(got.abs_diff(w) <= 2, "{fx:?}: {x} → {got}, Word {w}");
            }
        }
    }

    #[test]
    fn channels_apart_alpha_kept() {
        let fx = PicFx::new(80_000, -80_000, 0);
        let mut px = [255, 0, 0, 77, 0, 128, 255, 255];
        fx.apply_rgba(&mut px);
        // Word: red (255, 0, 0) → (255, 224, 224); 128 → 249 (the formula gives 250.0).
        assert_eq!(px, [255, 224, 224, 77, 224, 250, 255, 255]);
        let mut odd = [0u8; 6];
        fx.apply_rgba(&mut odd);
        assert_eq!(odd[4..], [0, 0]);
    }

    #[test]
    fn vml_mapping() {
        // 16.16 fixed point: 19661f and 22938f are 0.3 and 0.35 to within 1e-5.
        let washout = PicFx::from_vml(19661.0 / 65536.0, 22938.0 / 65536.0);
        assert!((washout.bright - 70_000).abs() <= 2 && (washout.contrast + 70_000).abs() <= 2, "{washout:?}");
        assert_eq!(PicFx::from_vml(1.0, 0.2), PicFx::new(40_000, 0, 0));
        assert_eq!(PicFx::from_vml(2.0, 0.0), PicFx::new(0, 50_000, 0));
        assert!(PicFx::from_vml(1.0, 0.0).is_identity());
        for (g, b) in [(f64::NAN, f64::INFINITY), (-1.0e30, 1.0e30), (1.0e300, -7.0), (0.0, 0.0)] {
            let fx = PicFx::from_vml(g, b);
            assert_eq!(fx, fx.clamped(), "{g} {b}");
            let _ = fx.table();
        }
    }

    #[test]
    fn hostile_values_clamp() {
        for v in [i32::MIN, -100_001, 0, 100_001, i32::MAX] {
            let fx = PicFx { bright: v, contrast: v, transparency: v };
            let _ = fx.table();
            assert!((0.0..=1.0).contains(&fx.opacity()));
            let mut px = [1u8, 2, 3, 4];
            fx.apply_rgba(&mut px);
            assert_eq!(px[3], 4);
        }
        assert_eq!(PicFx::new(0, 0, 25_000).opacity(), 0.75);
        assert!(!PicFx::new(0, 0, 25_000).changes_pixels());
        // Contrast 100 %: a threshold at mid-grey, no NaN.
        let t = PicFx::new(0, FX_MAX, 0).table();
        assert_eq!((t[0], t[127], t[128], t[255]), (0, 0, 255, 255));
    }
}
