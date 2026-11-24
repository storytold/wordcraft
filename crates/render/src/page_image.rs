//! Page images at print resolutions (#7743): a page drawn band by band straight into a PNG, so a
//! page at 1200 or 1500 DPI keeps every pixel asked for instead of being shrunk to the on-screen
//! raster cap ([`wordcraft_geom::MAX_RASTER_PIXELS`]). Only one band of pixels is held at a time;
//! the PNG records the resolution (`pHYs`), so other programs print it at the page's real size.

use vello_cpu::kurbo::Affine;
use wordcraft_doc::Document;
use wordcraft_layout::Page;

use crate::{RenderOptions, render_region};

/// Most pixels one page image may have. A Letter page at 1500 DPI is 210 million; this leaves room
/// for larger paper while keeping a page within seconds of drawing.
pub const MAX_PAGE_IMAGE_PIXELS: u64 = 500_000_000;
/// Longest side of a page image, pixels: a 22-inch page (Word's largest) at 2400 DPI is 52 800.
pub const MAX_PAGE_IMAGE_SIDE: u32 = 60_000;
/// Rows drawn together: a band is the image's width by this many rows.
const BAND_ROWS: u32 = 256;
/// Widest piece of a band drawn at once (the rasteriser's side limit is [`crate::MAX_SIDE`]).
const TILE_COLS: u32 = 4096;
/// Points per inch.
const POINTS_PER_INCH: f32 = 72.0;
/// Inches per metre, for the PNG's pixels-per-metre resolution.
const INCHES_PER_METRE: f64 = 1.0 / 0.0254;

/// The size, in pixels, of `page` drawn at `scale` pixels per point, or why it can't be drawn: a
/// scale that isn't a positive number, or an image past [`MAX_PAGE_IMAGE_SIDE`] or
/// [`MAX_PAGE_IMAGE_PIXELS`].
pub fn page_image_size(page: &Page, scale: f32) -> Result<(u32, u32), String> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(format!("{scale} isn't a usable resolution"));
    }
    // Rounded as [`crate::render_page`] rounds, so both give the same size.
    let side = |pt: f32| -> Option<u32> {
        let px = (pt.max(0.0) * scale).ceil();
        (px.is_finite() && px <= MAX_PAGE_IMAGE_SIDE as f32).then(|| (px as u32).max(1))
    };
    let dpi = scale * POINTS_PER_INCH;
    let too_large = || format!("the page is too large for an image at {dpi:.0} DPI; choose a lower resolution");
    let (Some(w), Some(h)) = (side(page.w), side(page.h)) else { return Err(too_large()) };
    if u64::from(w) * u64::from(h) > MAX_PAGE_IMAGE_PIXELS {
        return Err(format!(
            "the page would be {w} × {h} pixels at {dpi:.0} DPI, more than the {} million a page image can have; choose a lower resolution",
            MAX_PAGE_IMAGE_PIXELS / 1_000_000
        ));
    }
    Ok((w, h))
}

/// `page` as PNG bytes at `scale` pixels per point (72 × `scale` DPI), with that resolution
/// recorded in the file. The same pixels as [`crate::render_page`] where that fits the on-screen
/// cap; past it, the full size rather than a smaller picture.
pub fn page_png(doc: &Document, page: &Page, scale: f32, opts: &RenderOptions) -> Result<Vec<u8>, String> {
    page_png_in(doc, page, scale, opts, BAND_ROWS, TILE_COLS)
}

/// [`page_png`] with the band height and tile width given (small ones in tests, to cross seams).
fn page_png_in(doc: &Document, page: &Page, scale: f32, opts: &RenderOptions, band_rows: u32, tile_cols: u32) -> Result<Vec<u8>, String> {
    let (w, h) = page_image_size(page, scale)?;
    let (band_rows, tile_cols) = (band_rows.clamp(1, BAND_ROWS), tile_cols.clamp(1, TILE_COLS));
    let encode_error = |e: png::EncodingError| format!("PNG encode: {e}");
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let ppm = (f64::from(scale) * f64::from(POINTS_PER_INCH) * INCHES_PER_METRE).round();
    // Saturates only for an absurd scale, which an empty page can pass the size check with.
    let ppm = ppm.clamp(1.0, f64::from(u32::MAX)) as u32;
    encoder.set_pixel_dims(Some(png::PixelDimensions { xppu: ppm, yppu: ppm, unit: png::Unit::Meter }));
    let mut writer = encoder.write_header().map_err(encode_error)?;
    let mut stream = writer.stream_writer().map_err(encode_error)?;
    let row_bytes = w as usize * 4;
    let mut band = vec![0u8; row_bytes * band_rows as usize];
    let mut y0 = 0u32;
    while y0 < h {
        let rows = band_rows.min(h - y0);
        let mut x0 = 0u32;
        while x0 < w {
            let cols = tile_cols.min(w - x0);
            // Page points → this tile's pixels: the whole image's transform, moved by whole pixels.
            let view = Affine::translate((-f64::from(x0), -f64::from(y0))) * Affine::scale(f64::from(scale));
            let tile = render_region(doc, page, cols, rows, view, opts);
            if (tile.width, tile.height) != (cols, rows) {
                return Err(format!("a {cols} × {rows} piece of the page drew as {} × {}", tile.width, tile.height));
            }
            let straight = tile.to_straight();
            let tile_row = cols as usize * 4;
            for (r, src) in straight.chunks_exact(tile_row).enumerate() {
                let at = r * row_bytes + x0 as usize * 4;
                if let Some(dst) = band.get_mut(at..at + tile_row) {
                    dst.copy_from_slice(src);
                }
            }
            x0 += cols;
        }
        let filled = band.get(..row_bytes * rows as usize).unwrap_or_default();
        std::io::Write::write_all(&mut stream, filled).map_err(|e| format!("PNG encode: {e}"))?;
        y0 += rows;
    }
    stream.finish().map_err(encode_error)?;
    writer.finish().map_err(encode_error)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

    fn sample() -> (Document, Page) {
        let mut doc = Document::from_text("Page images at print resolution.\rA second paragraph, with more words to wrap.");
        doc.last_section.page_w = 200.0;
        doc.last_section.page_h = 150.0;
        for m in [&mut doc.last_section.margin_left, &mut doc.last_section.margin_right] {
            *m = 20.0;
        }
        doc.last_section.margin_top = 20.0;
        doc.last_section.margin_bottom = 20.0;
        let l = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        let page = l.pages.first().cloned().unwrap();
        (doc, page)
    }

    fn decode(png_bytes: &[u8]) -> (png::OutputInfo, Vec<u8>, Option<png::PixelDimensions>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info().unwrap();
        let dims = reader.info().pixel_dims;
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info, buf, dims)
    }

    /// Drawn in bands and tiles, the page has exactly the pixels it has drawn whole: seams between
    /// pieces don't show.
    #[test]
    fn banded_page_has_the_pixels_of_the_whole_page() {
        let (doc, page) = sample();
        let opts = RenderOptions::default();
        let whole = crate::render_page(&doc, &page, 1.5, &opts);
        let (info, pixels, _) = decode(&page_png_in(&doc, &page, 1.5, &opts, 7, 33).unwrap());
        assert_eq!((info.width, info.height), (whole.width, whole.height));
        assert_eq!(pixels, whole.to_straight());
    }

    /// The PNG says its resolution, so it prints at the page's size.
    #[test]
    fn page_png_records_its_resolution() {
        let (doc, page) = sample();
        let (info, _, dims) = decode(&page_png(&doc, &page, 300.0 / 72.0, &RenderOptions::default()).unwrap());
        assert_eq!((info.width, info.height), (834, 625), "200 × 150 pt at 300 DPI");
        let dims = dims.unwrap();
        assert_eq!(dims.unit, png::Unit::Meter);
        assert_eq!((dims.xppu, dims.yppu), (11_811, 11_811), "300 DPI is 11 811 pixels a metre");
    }

    /// Past the on-screen raster cap a page image keeps its full size rather than shrinking.
    #[test]
    fn page_past_the_screen_cap_keeps_its_size() {
        let (_, mut page) = sample();
        // 1200 DPI on Letter paper: 10 200 × 13 200, past the 100 million pixel screen cap.
        (page.w, page.h) = (612.0, 792.0);
        assert_eq!(page_image_size(&page, 1200.0 / 72.0), Ok((10_200, 13_200)));
        const { assert!(10_200u64 * 13_200 > wordcraft_geom::MAX_RASTER_PIXELS) };
    }

    /// Hostile scales and huge pages are refused, never drawn or allocated.
    #[test]
    fn unusable_sizes_are_refused() {
        let (doc, mut page) = sample();
        let opts = RenderOptions::default();
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(page_png(&doc, &page, scale, &opts).is_err(), "{scale}");
        }
        (page.w, page.h) = (1584.0, 1584.0);
        let err = page_png(&doc, &page, 2400.0 / 72.0, &opts).unwrap_err();
        assert!(err.contains("choose a lower resolution"), "{err}");
        page.h = f32::INFINITY;
        assert!(page_image_size(&page, 1.0).is_err());
    }
}
