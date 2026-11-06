//! Pixel-budget arithmetic shared by every crate that decodes or allocates rasters: sizes are
//! checked before anything is allocated, so a hostile header or request cannot exhaust memory.

/// Bytes a decoded picture may take as RGBA8 (what the `image` crate's default decode budget is).
pub const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;

/// Most pixels one page raster may hold (400 MB of RGBA8); larger requests are scaled down.
pub const MAX_RASTER_PIXELS: u64 = 100_000_000;

/// Bytes of a `w`×`h` RGBA8 raster, None on overflow.
pub fn rgba_bytes(w: u32, h: u32) -> Option<u64> {
    u64::from(w).checked_mul(u64::from(h))?.checked_mul(4)
}

/// Whether a `w`×`h` RGBA8 raster fits `budget` bytes.
pub fn rgba_fits(w: u32, h: u32, budget: u64) -> bool {
    w > 0 && h > 0 && rgba_bytes(w, h).is_some_and(|b| b <= budget)
}

/// Whether decoding a `w`×`h` picture fits `budget` bytes at its peak: the decoder's buffer and
/// the RGBA conversion (or its premultiplied copy) are alive together, so two RGBA buffers. For a
/// decoder whose buffer isn't four bytes a pixel, see [`decode_peak_fits`].
pub fn decode_fits(w: u32, h: u32, budget: u64) -> bool {
    decode_peak_fits(w, h, 4, budget)
}

/// [`decode_fits`] for a decoder that hands back `native` bytes a pixel (8 for 16-bit RGBA, 16 for
/// float RGBA): its buffer lives alongside the RGBA8 conversion, then the conversion alongside its
/// premultiplied copy, so the peak is the larger of `native + 4` and `8` bytes a pixel.
pub fn decode_peak_fits(w: u32, h: u32, native: u64, budget: u64) -> bool {
    let per_pixel = native.saturating_add(4).max(8);
    w > 0 && h > 0 && u64::from(w).checked_mul(u64::from(h)).and_then(|p| p.checked_mul(per_pixel)).is_some_and(|b| b <= budget)
}

/// `w`×`h` shrunk (aspect kept, each side at least 1) until it holds at most `max_pixels`.
pub fn fit_pixels(w: u32, h: u32, max_pixels: u64) -> (u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    let max_pixels = max_pixels.max(1);
    let area = u64::from(w) * u64::from(h);
    if area <= max_pixels {
        return (w, h);
    }
    #[expect(clippy::cast_precision_loss, reason = "Floating point only approximates the scale; the integer clamps below enforce every bound.")]
    let scale = (max_pixels as f64 / area as f64).sqrt();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The scale is finite and within 0..=1; truncation floors a positive dimension before clamping."
    )]
    let new_height = ((f64::from(h) * scale) as u64).clamp(1, u64::from(h));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The scale is finite and within 0..=1; truncation floors a positive dimension before clamping."
    )]
    let new_width = ((f64::from(w) * scale) as u64).clamp(1, u64::from(w)).min(max_pixels / new_height).max(1);
    let new_height = new_height.min(max_pixels / new_width).max(1);
    #[expect(clippy::cast_possible_truncation, reason = "Both dimensions are clamped to their original u32 values above.")]
    (new_width as u32, new_height as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_budget_boundary() {
        assert_eq!(rgba_bytes(16, 8), Some(512));
        assert!(rgba_fits(16, 8, 512));
        assert!(!rgba_fits(16, 8, 511));
        assert!(!rgba_fits(0, 8, 512), "empty is not a picture");
        assert!(decode_fits(16, 8, 1024));
        assert!(!decode_fits(16, 8, 1023), "decode + conversion hold two buffers");
        assert!(!decode_fits(0, 8, u64::MAX));
    }

    /// A 16-bit RGBA decode (8 bytes a pixel) lives alongside its 4-byte RGBA8 conversion: 12 bytes
    /// a pixel at the peak, not the 8 two RGBA8 buffers take.
    #[test]
    fn wide_decoder_buffers_are_charged_at_their_size() {
        assert!(decode_peak_fits(16, 8, 4, 1024), "RGBA8: two RGBA8 buffers");
        assert!(decode_peak_fits(16, 8, 1, 1024), "grey: the premultiplied copy is the peak");
        assert!(!decode_peak_fits(16, 8, 8, 1024), "RGBA16 needs 1536 bytes");
        assert!(decode_peak_fits(16, 8, 8, 1536));
        assert!(!decode_peak_fits(16, 8, 16, 2559), "RGBA32F: 20 bytes a pixel");
        assert!(!decode_peak_fits(u32::MAX, u32::MAX, u64::MAX, u64::MAX), "overflow is refused");
    }

    #[test]
    fn fit_pixels_keeps_normal_sizes_and_shrinks_huge_ones() {
        assert_eq!(fit_pixels(612, 792, MAX_RASTER_PIXELS), (612, 792));
        let (w, h) = fit_pixels(16_000, 16_000, 1_000_000);
        assert!(u64::from(w) * u64::from(h) <= 1_000_000 && w >= 900, "{w}x{h}");
        let (w, h) = fit_pixels(16_000, 1, 100);
        assert!(u64::from(w) * u64::from(h) <= 100 && h == 1, "{w}x{h}");
        assert_eq!(fit_pixels(0, 5, 10).0, 1);
    }
}
