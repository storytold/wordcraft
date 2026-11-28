//! Decode workspace beyond the raster, encoded-data copies and JPEG coefficients.
//! Estimates follow the pinned decoder implementations. They are debits against
//! the alternate-image budget, separate from the limits passed to the codecs.

use craft_image_codecs::{Format, TiffPage};

const KIB: u64 = 1024;
const JPEG_FIXED: u64 = 96 * KIB;

/// The workspace debit, or `None` when the inspection that sets it hit a limit of its own (more chunks than are
/// examined): the assessment is then not made, never graded from a guess.
pub(crate) fn decode(format: Format, bytes: &[u8], width: u32, height: u32, tiff: Option<&TiffPage>) -> Option<u64> {
    let row = u64::from(width);
    let encoded = bytes.len() as u64;
    Some(match format {
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
        Format::Tiff => match tiff {
            Some(page) => tiff_workspace(bytes, page)?,
            None => 0,
        },
        _ => 0,
    })
}

fn tiff_workspace(bytes: &[u8], page: &TiffPage) -> Option<u64> {
    let (chunk_width, chunk_height) = crate::tiff_admission::chunk_dimensions(bytes, page);
    let bands = u64::from(page.height).div_ceil(chunk_height.max(1)).max(1);
    #[cfg(not(target_arch = "wasm32"))]
    let workers = rayon::current_num_threads() as u64;
    #[cfg(target_arch = "wasm32")]
    let workers = 1;
    let samples = u64::from(page.samples_per_pixel).clamp(1, 64);
    let sample_bytes = u64::from(page.bits_per_sample).div_ceil(8).clamp(1, 8);
    // Converted or planar strips and padded edge tiles can have more stored
    // samples than their output. Charge each simultaneous reader's scratch.
    let chunk = chunk_width.saturating_mul(chunk_height).saturating_mul(samples).saturating_mul(sample_bytes);
    // tiff 0.11 builds the reader of each strip or tile (`create_reader`), so every worker holds the
    // nested decoder its compression kind creates:
    // - Deflate: flate2's 32 KiB input buffer, miniz_oxide's 32 KiB window and about 11 KiB of
    //   Huffman tables (`DecompressorOxide`): under 80 KiB.
    // - LZW: a BufReader of up to 32 KiB, and weezl's 4096-entry link table (16 KiB), depth table
    //   (8 KiB) and 4 KiB output buffer: under 64 KiB.
    // - Fax: a default 8 KiB BufReader, two transition vectors of one u16 a pixel and a line buffer
    //   of a byte a pixel (`fax` 0.2 `Group4Decoder`, `stream.rs` `Group4Reader`).
    // - PackBits and raw: streaming, no state worth modelling.
    // - JPEG: zune-jpeg's row, MCU and upsampler state (the fixed term and the per-32-pixel term),
    //   the compressed strip with its tables (once, moved into the decoder), the decoded output
    //   (`decode`: `vec![0; output_buffer_size]`, one byte a sample), and, for a progressive image
    //   or a baseline one with several scans, a 16-bit coefficient plane for every component padded
    //   to whole MCUs (`mcu_prog.rs` `block[i]`, `mcu.rs` `progressive_mcus`). The layout kind is
    //   in the strip's own markers, which the directory does not hold, so the coefficient planes
    //   (at most two bytes for each sample padded to 32 in both directions) are charged for every
    //   JPEG strip: a bound over all layout kinds.
    let compression = match page.compression {
        8 | 32946 => 128 * KIB,
        5 => 64 * KIB,
        7 => jpeg_strip_decoder(bytes, page, (chunk_width, chunk_height), samples, chunk)?,
        4 => (16 * KIB).saturating_add(8u64.saturating_mul(chunk_width)),
        _ => 8 * KIB,
    };
    // Each worker also opens its own `tiff` decoder, which rebuilds the page's directory and tables.
    let opened = crate::tiff_admission::worker_open_bytes(bytes, page);
    let per_worker = compression.saturating_add(chunk.saturating_mul(2)).saturating_add(opened);
    let palette = if page.photometric == Some(3) { 6u64 << page.bits_per_sample.min(16) } else { 0 };
    let extra_planes =
        u64::from(page.width).saturating_mul(u64::from(page.height)).saturating_mul(samples.saturating_sub(1)).saturating_mul(sample_bytes);
    Some((24 * KIB + palette).saturating_add(per_worker.saturating_mul(workers.min(bands))).saturating_add(extra_planes))
}

/// What one worker's nested zune-jpeg decoder holds for a strip or tile, from the JPEG's OWN header: its SOF
/// dimensions and component layout set the row and upsampling state and the decoded output (a byte a sample), and
/// its scan kind the 16-bit coefficient planes (a progressive frame, or a baseline one with several scans; none for
/// a single-scan baseline). The decoder allocates from that header, not from the strip or tile size the TIFF
/// directory declares, so charging the directory's geometry under- or over-charges whenever they differ. The output
/// is charged only beyond the chunk-sized scratch the caller charges beside this (a JPEG that fills its strip adds
/// nothing; one larger than its strip adds the difference). A worker
/// may meet any chunk, so the largest is charged. The marker walk runs from the chunk's start to its SOF wherever
/// that lies, bounded by the chunk. Only where a chunk's header is not there to be read (the chunk really is cut off)
/// does the directory's geometry stand in, as a bound over every layout kind; more chunks than are examined is a
/// limit of the inspection, so no charge is made (`None`: not assessed).
fn jpeg_strip_decoder(bytes: &[u8], page: &TiffPage, directory: (u64, u64), samples: u64, scratch: u64) -> Option<u64> {
    const CHUNKS: usize = 1 << 16;
    let state =
        |width: u64| JPEG_FIXED.saturating_add(width.div_ceil(32).saturating_mul(256 * 32)).saturating_add((bytes.len() as u64).saturating_mul(2));
    let (width, height) = directory;
    let padded = |side: u64| side.div_ceil(32).saturating_mul(32);
    let declared = state(width)
        .saturating_add(width.saturating_mul(height).saturating_mul(samples))
        .saturating_add(padded(width).saturating_mul(padded(height)).saturating_mul(samples).saturating_mul(2));
    let chunks = crate::tiff_admission::chunks(bytes, page, CHUNKS);
    let mut most = 0u64;
    if chunks.len() >= CHUNKS {
        return None;
    }
    let mut unread = chunks.is_empty();
    for (start, length) in chunks {
        let whole = bytes.get(start..start.saturating_add(length).min(bytes.len()));
        let Some(info) = whole.and_then(craft_image_codecs::jpeg_info) else {
            unread = true;
            continue;
        };
        let (frame_width, frame_height) = (u64::from(info.width), u64::from(info.height));
        let output = frame_width.saturating_mul(frame_height).saturating_mul(info.layout.channels() as u64);
        let coefficients = whole.map_or(0, craft_image_codecs::jpeg_worst_case_coefficients);
        // The output the worker's chunk-sized scratch (charged beside this) already covers is not charged twice.
        most = most.max(state(frame_width).saturating_add(output.saturating_sub(scratch)).saturating_add(coefficients));
    }
    Some(if unread { most.max(declared) } else { most })
}

fn webp(bytes: &[u8], width: u32, height: u32) -> u64 {
    // The shared allocation probe already covers the coded frames and canvas.
    let blocks = u64::from(width).div_ceil(16) * u64::from(height).div_ceil(16);
    // VP8's Y/U/V planes use 16×16 + 2×8×8 bytes per block, plus block state
    // and predictor borders. VP8L/ALPH have a color cache, live Huffman/tree
    // and code-length tables, transform and entropy images, plus coded data.
    (128 * KIB).saturating_add(blocks.saturating_mul(512)).saturating_add((bytes.len() as u64).saturating_mul(8))
}
