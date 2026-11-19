//! The coded image inside an alternate-encoding media part.
//!
//! The assessment is a function of the coded image only. For each format the image ends at
//! the format's own boundary: the JPEG end-of-image marker, the PNG `IEND` chunk, the GIF
//! trailer, the BMP file size or pixel array, or the RIFF container size. Bytes after that
//! boundary have any content and any length, and are padding that is neither assessed nor
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

/// The declared file size, or the end of an uncompressed pixel array when none is declared.
fn bmp_end(bytes: &[u8]) -> Option<usize> {
    if bytes.get(..2)? != b"BM" {
        return None;
    }
    let declared = le32(bytes, 2).filter(|&size| size >= 26 && le32(bytes, 10).is_some_and(|offset| offset < size));
    // BITMAPINFOHEADER and later: width, height, planes, bit count, compression.
    let pixels = (|| {
        let offset = le32(bytes, 10)?;
        let width = le32(bytes, 18)?;
        let height = i32::from_le_bytes(bytes.get(22..26)?.try_into().ok()?).unsigned_abs() as usize;
        let bits = usize::from(u16::from_le_bytes(bytes.get(28..30)?.try_into().ok()?));
        let uncompressed = matches!(le32(bytes, 30)?, 0 | 3);
        let stride = width.checked_mul(bits)?.checked_add(31)? / 32 * 4;
        uncompressed.then(|| stride.checked_mul(height)?.checked_add(offset)).flatten()
    })();
    match (declared, pixels) {
        (Some(declared), Some(pixels)) => Some(declared.max(pixels)),
        (declared, pixels) => declared.or(pixels),
    }
}

/// The RIFF container size plus its eight-byte header.
fn webp_end(bytes: &[u8]) -> Option<usize> {
    le32(bytes, 4)?.checked_add(8).filter(|&size| size >= 20)
}
