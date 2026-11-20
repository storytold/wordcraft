//! The coded image inside an alternate-encoding media part, and layouts its header declines.
//!
//! The assessment is a function of the coded image only. For each format the image ends at
//! the format's own boundary: the JPEG end-of-image marker, the PNG `IEND` chunk, the GIF
//! trailer, the end of the BMP pixel data (a declared file size is only a claim), or the RIFF
//! container size. Bytes after that boundary have any content and any length, and are padding that is neither assessed nor
//! charged. A boundary is found by walking the format's structure from the start, never by
//! trimming the tail of the part. Without a boundary (a cut-off image) every byte is data.

use craft_image_codecs::Format;

/// The prefix of `bytes` that is the coded image. TIFF is bounded by its directory graph instead.
pub(crate) fn coded_image(format: Format, bytes: &[u8]) -> &[u8] {
    let end = match format {
        Format::Png => png_end(bytes),
        Format::Jpeg => jpeg_end(bytes),
        Format::Gif => gif_end(bytes),
        Format::Bmp => bmp_end(bytes),
        Format::WebP => webp_end(bytes),
        _ => None,
    };
    end.and_then(|end| bytes.get(..end)).unwrap_or(bytes)
}

fn be16(bytes: &[u8], at: usize) -> Option<usize> {
    bytes.get(at..at.checked_add(2)?).and_then(|b| b.try_into().ok()).map(u16::from_be_bytes).map(usize::from)
}

fn be32(bytes: &[u8], at: usize) -> Option<usize> {
    bytes.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes).and_then(|n| usize::try_from(n).ok())
}

fn le32(bytes: &[u8], at: usize) -> Option<usize> {
    bytes.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).and_then(|n| usize::try_from(n).ok())
}

/// The end of the `IEND` chunk, reached by walking chunk lengths from the signature.
fn png_end(bytes: &[u8]) -> Option<usize> {
    let mut at = bytes.strip_prefix(b"\x89PNG\r\n\x1a\n").map(|rest| bytes.len() - rest.len())?;
    loop {
        let length = be32(bytes, at)?;
        let kind = bytes.get(at.checked_add(4)?..at.checked_add(8)?)?;
        let next = at.checked_add(12)?.checked_add(length)?;
        if next > bytes.len() {
            return None;
        }
        if kind == b"IEND" {
            return Some(next);
        }
        at = next;
    }
}

/// The end of the first end-of-image marker, reached by walking segments and skipping scan data.
fn jpeg_end(bytes: &[u8]) -> Option<usize> {
    if bytes.get(..2)? != b"\xff\xd8" {
        return None;
    }
    let mut at = 2usize;
    loop {
        // Markers may be preceded by any number of 0xFF fill bytes.
        if *bytes.get(at)? != 0xff {
            return None;
        }
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        match marker {
            0xd9 => return Some(at),
            0x00 => return None,
            0x01 | 0xd0..=0xd8 => {}
            _ => {
                let length = be16(bytes, at)?;
                if length < 2 {
                    return None;
                }
                at = at.checked_add(length)?;
                if marker == 0xda {
                    at = scan_end(bytes, at)?;
                }
            }
        }
    }
}

/// The position of the next marker after entropy-coded data: not a stuffed zero, restart or fill.
fn scan_end(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        at += bytes.get(at..)?.iter().position(|&byte| byte == 0xff)?;
        match *bytes.get(at + 1)? {
            0x00 | 0xd0..=0xd7 => at += 2,
            0xff => at += 1,
            _ => return Some(at),
        }
    }
}

/// The end of the GIF trailer, reached by walking the screen descriptor, blocks and sub-blocks.
fn gif_end(bytes: &[u8]) -> Option<usize> {
    if !matches!(bytes.get(..6)?, b"GIF87a" | b"GIF89a") {
        return None;
    }
    let table = |packed: u8| if packed & 0x80 != 0 { 3usize << (usize::from(packed & 7) + 1) } else { 0 };
    let mut at = 13usize.checked_add(table(*bytes.get(10)?))?;
    loop {
        match *bytes.get(at)? {
            0x3b => return at.checked_add(1),
            0x21 => at = sub_blocks_end(bytes, at.checked_add(2)?)?,
            0x2c => {
                let descriptor = at.checked_add(10)?;
                let packed = *bytes.get(at.checked_add(9)?)?;
                // The local color table, then the LZW minimum code size, then the data.
                at = sub_blocks_end(bytes, descriptor.checked_add(table(packed))?.checked_add(1)?)?;
            }
            _ => return None,
        }
    }
}

/// The position after a run of length-prefixed sub-blocks, which ends with a zero length.
fn sub_blocks_end(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let length = usize::from(*bytes.get(at)?);
        at = at.checked_add(1)?.checked_add(length)?;
        if length == 0 {
            return Some(at);
        }
    }
}

/// The end of the pixel data, from the layout the decoder reads. The declared file size is a claim
/// about the layout: it may extend the image (data after the pixels that the file counts as its
/// own) but never cuts into pixels the layout needs, so a short or zero size changes nothing.
fn bmp_end(bytes: &[u8]) -> Option<usize> {
    if bytes.get(..2)? != b"BM" {
        return None;
    }
    let layout = bmp_layout_end(bytes)?;
    let declared = le32(bytes, 2).filter(|&size| size >= layout && size <= bytes.len());
    Some(declared.unwrap_or(layout))
}

/// Where the pixel data ends, read the way the codec's own BMP probe reads the layout
/// (`bmp_probe.rs`, `bmp_layout` and `dimensions`): a 12-byte core header has 16-bit dimensions,
/// bits per pixel at 24 and is never compressed; a larger header has 32-bit dimensions, bits per
/// pixel at 28 and the compression at 30. Compression 1 and 2 are RLE8 and RLE4, whose extent is
/// found by walking the commands; every other value is rows of `bits` per pixel padded to 4 bytes.
/// `None` when the header or the RLE stream is cut off, so the whole part stays data.
fn bmp_layout_end(bytes: &[u8]) -> Option<usize> {
    let le16 = |at: usize| bytes.get(at..at.checked_add(2)?).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes).map(usize::from);
    let offset = le32(bytes, 10)?;
    let (width, height) = craft_image_codecs::bmp_probe::dimensions(bytes)?;
    let (width, height) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    let (bits, compression) = if le32(bytes, 14)? == 12 { (le16(24)?, 0) } else { (le16(28)?, le32(bytes, 30)?) };
    match compression {
        1 | 2 => bmp_rle_end(bytes, offset, compression == 2, height),
        _ => {
            let stride = width.checked_mul(bits.max(1))?.checked_add(31)? / 32 * 4;
            offset.checked_add(stride.checked_mul(height)?)
        }
    }
}

/// The end of an RLE8 or RLE4 command stream: its end-of-bitmap code, or the last row's commands
/// and an end-of-bitmap code that immediately follows them. Only the rows matter, so a run and a
/// delta's horizontal move need no position.
fn bmp_rle_end(bytes: &[u8], mut at: usize, rle4: bool, height: usize) -> Option<usize> {
    let mut y = 0usize;
    while y < height {
        let (count, value) = (*bytes.get(at)?, *bytes.get(at.checked_add(1)?)?);
        at = at.checked_add(2)?;
        match (count, value) {
            (0, 0) => y = y.saturating_add(1),
            (0, 1) => return Some(at),
            (0, 2) => {
                y = y.saturating_add(usize::from(*bytes.get(at.checked_add(1)?)?));
                at = at.checked_add(2)?;
            }
            (0, n) => {
                let stored = if rle4 { usize::from(n).div_ceil(2) } else { usize::from(n) };
                at = at.checked_add(stored + (stored & 1))?;
            }
            _ => {}
        }
    }
    let end = if bytes.get(at..at.checked_add(2)?) == Some(&[0, 1]) { at.checked_add(2)? } else { at };
    (end <= bytes.len()).then_some(end)
}

/// The RIFF container size plus its eight-byte header.
fn webp_end(bytes: &[u8]) -> Option<usize> {
    le32(bytes, 4)?.checked_add(8).filter(|&size| size >= 20)
}

/// A layout the locked decoders refuse by construction, named from the header alone, for a codec
/// whose adapter reports the refusal as plain malformed data. Genuinely damaged data in a layout
/// the decoder supports is not named here, so it stays an unexplained decoding error.
///
/// WebP: `image-webp` decodes only VP8 keyframes. A frame tag with the keyframe bit set is an
/// interframe, which every `UnsupportedFeature` it raises is about (`decoder.rs`, `vp8.rs`).
///
/// JPEG: `zune-jpeg` reads the sequential and progressive Huffman frames (SOF0 to SOF2) and
/// `craft-jpeg` the arithmetic, lossless and non-8-bit ones, so the differential (hierarchical)
/// frames SOF5 to SOF7 and SOF13 to SOF15 are refused as a format error.
///
/// PNG, GIF, BMP and TIFF adapters keep the codec's own unsupported error, which the caller
/// reports through its discriminant; the validator decodes PNG through the `png` crate directly,
/// which has no unsupported-layout error.
pub(crate) fn declined_layout(format: Format, bytes: &[u8]) -> Option<&'static str> {
    match format {
        Format::WebP => (webp_frame_tag(bytes)? & 1 != 0).then_some("a VP8 frame that is not a keyframe"),
        Format::Jpeg => matches!(jpeg_frame_marker(bytes)?, 0xc5..=0xc7 | 0xcd..=0xcf).then_some("a differential (hierarchical) JPEG frame"),
        _ => None,
    }
}

/// The 24-bit frame tag of the first `VP8 ` chunk, found by walking RIFF chunks and animation frames.
fn webp_frame_tag(bytes: &[u8]) -> Option<u32> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let mut at = 12usize;
    loop {
        let kind = bytes.get(at..at.checked_add(4)?)?;
        let size = le32(bytes, at.checked_add(4)?)?;
        let body = at.checked_add(8)?;
        match kind {
            b"VP8 " => {
                return Some(u32::from_le_bytes([*bytes.get(body)?, *bytes.get(body.checked_add(1)?)?, *bytes.get(body.checked_add(2)?)?, 0]));
            }
            // An animation frame holds its own chunks after sixteen bytes of frame geometry.
            b"ANMF" => at = body.checked_add(16)?,
            _ => at = body.checked_add(size)?.checked_add(size & 1)?,
        }
    }
}

/// The marker of the first start-of-frame segment.
fn jpeg_frame_marker(bytes: &[u8]) -> Option<u8> {
    if bytes.get(..2)? != b"\xff\xd8" {
        return None;
    }
    let mut at = 2usize;
    loop {
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        match marker {
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => return Some(marker),
            0xd9 | 0xda | 0x00 => return None,
            0x01 | 0xd0..=0xd8 => {}
            _ => {
                let length = be16(bytes, at)?;
                if length < 2 {
                    return None;
                }
                at = at.checked_add(length)?;
            }
        }
    }
}
