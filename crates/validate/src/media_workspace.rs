//! Decode workspace beyond the raster, encoded-data copies and JPEG coefficients.
//! Estimates follow the pinned decoder implementations. They are debits against
//! the alternate-image budget, separate from the limits passed to the codecs.

use craft_image_codecs::{Format, TiffPage};

const KIB: u64 = 1024;
const JPEG_FIXED: u64 = 96 * KIB;

pub(crate) fn decode(format: Format, bytes: &[u8], width: u32, height: u32, tiff: Option<&TiffPage>) -> u64 {
    let row = u64::from(width);
    let encoded = bytes.len() as u64;
    match format {
        // png/fdeflate: two live table pairs at an APNG frame reset, inflater
        // state, buffered input, and transformed/unfiltered scratch rows.
        Format::Png => 64 * KIB + 16 * row,
        // zune-jpeg and craft-jpeg: DHT/DQT and arithmetic contexts, MCU row
        // and upsampler memory, and copies of APP data. Coefficients are separate.
        Format::Jpeg => (JPEG_FIXED + 256 * row.div_ceil(32) * 32).saturating_add(encoded.saturating_mul(4)),
        // image/gif/weezl: the 4096-entry link and depth tables, decode buffer,
        // BufReader, palettes and extension staging. One first frame is decoded.
        Format::Gif => (48 * KIB + 4 * row).saturating_add(encoded),
        // image BMP: palette and bitfield state, aligned row and RLE scratch.
        Format::Bmp => 8 * KIB + 8 * row,
        Format::WebP => webp(bytes, width, height),
        Format::Tiff => tiff.map_or(0, |page| tiff_workspace(bytes, page)),
        _ => 0,
    }
}

fn tiff_workspace(bytes: &[u8], page: &TiffPage) -> u64 {
    let (chunk_width, chunk_height) = crate::tiff_admission::chunk_dimensions(bytes, page);
    let bands = u64::from(page.height).div_ceil(chunk_height.max(1)).max(1);
    #[cfg(not(target_arch = "wasm32"))]
    let workers = rayon::current_num_threads() as u64;
    #[cfg(target_arch = "wasm32")]
    let workers = 1;
    // tiff 0.11 uses a fresh reader per worker decoding a strip or row of tiles.
    // Deflate has a 32 KiB flate2 buffer plus window and state. LZW uses the
    // weezl tables and a buffer of up to 32 KiB. JPEG uses zune-jpeg. Fax uses
    // an input buffer plus line and transition data. Raw/PackBits stream rows.
    let compression = match page.compression {
        8 | 32946 => 128 * KIB,
        5 => 64 * KIB,
        7 => JPEG_FIXED.saturating_add(chunk_width.div_ceil(32).saturating_mul(256 * 32)).saturating_add((bytes.len() as u64).saturating_mul(2)),
        4 => (16 * KIB).saturating_add(8u64.saturating_mul(chunk_width)),
        _ => 8 * KIB,
    };
    let samples = u64::from(page.samples_per_pixel).clamp(1, 64);
    let sample_bytes = u64::from(page.bits_per_sample).div_ceil(8).clamp(1, 8);
    // Converted or planar strips and padded edge tiles can have more stored
    // samples than their output. Charge each simultaneous reader's scratch.
    let chunk = chunk_width.saturating_mul(chunk_height).saturating_mul(samples).saturating_mul(sample_bytes);
    // Each worker also opens its own `tiff` decoder, which rebuilds the page's directory and tables.
    let opened = crate::tiff_admission::worker_open_bytes(bytes, page);
    let per_worker = compression.saturating_add(chunk.saturating_mul(2)).saturating_add(opened);
    let palette = if page.photometric == Some(3) { 6u64 << page.bits_per_sample.min(16) } else { 0 };
    let extra_planes =
        u64::from(page.width).saturating_mul(u64::from(page.height)).saturating_mul(samples.saturating_sub(1)).saturating_mul(sample_bytes);
    (24 * KIB + palette).saturating_add(per_worker.saturating_mul(workers.min(bands))).saturating_add(extra_planes)
}

fn webp(bytes: &[u8], width: u32, height: u32) -> u64 {
    // The shared allocation probe already covers the coded frames and canvas.
    let blocks = u64::from(width).div_ceil(16) * u64::from(height).div_ceil(16);
    // VP8's Y/U/V planes use 16×16 + 2×8×8 bytes per block, plus block state
    // and predictor borders. VP8L/ALPH have a color cache, live Huffman/tree
    // and code-length tables, transform and entropy images, plus coded data.
    (128 * KIB).saturating_add(blocks.saturating_mul(512)).saturating_add((bytes.len() as u64).saturating_mul(8))
}
