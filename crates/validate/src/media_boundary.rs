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
    let header = le32(bytes, 14)?;
    let (bits, compression) = if header == 12 { (le16(24)?, 0) } else { (le16(28)?, le32(bytes, 30)?) };
    let pixels = match compression {
        1 | 2 => bmp_rle_end(bytes, offset, compression == 2, height)?,
        _ => {
            let stride = width.checked_mul(bits.max(1))?.checked_add(31)? / 32 * 4;
            offset.checked_add(stride.checked_mul(height)?)?
        }
    };
    // The decoder also reads the colour table (and the bit masks of a bitfields image) that follow the header,
    // wherever the pixel offset points: a layout whose pixels start inside them overlaps, and the boundary
    // covers both. Entries are 3 bytes after a core header and 4 after any other; an image of at most 8 bits
    // has `biClrUsed` of them, or all `1 << bits` when that is 0.
    let entries = match (header, bits) {
        (_, 0) => 0,
        (12, 1..=8) => 1usize << bits,
        (12, _) => 0,
        (_, 1..=8) => match le32(bytes, 46)? {
            0 => 1usize << bits,
            used => used.min(1usize << bits),
        },
        _ => 0,
    };
    let masks = if header == 40 && matches!(compression, 3 | 6) { 12 + 4 * usize::from(compression == 6) } else { 0 };
    let table = 14usize.checked_add(header)?.checked_add(entries.checked_mul(if header == 12 { 3 } else { 4 })?)?.checked_add(masks)?;
    // Never past the data the part holds: a cut-off table is the decoder's failure, not a larger image.
    Some(pixels.max(table.min(bytes.len())))
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
/// `bytes` is exactly what the decoder was given ([`coded_image`]'s output), so the name is a
/// function of the decoder's input: a chunk after the RIFF end is not part of it. Both walks follow
/// the real structure (chunk and segment lengths, the `0xFF` marker introducer).
///
/// WebP: `image-webp` decodes only VP8 keyframes. A frame tag with the keyframe bit set is an
/// interframe, which every `UnsupportedFeature` it raises is about (`decoder.rs`, `vp8.rs`). The
/// adapter decodes the first frame of an animation (`webp.rs`), so only the first `VP8 ` chunk is
/// named; whatever a later frame holds is not claimed.
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
        Format::WebP => (webp_frame_tag(bytes)? & 1 != 0).then_some("a first VP8 frame that is not a keyframe"),
        Format::Jpeg => matches!(jpeg_frame_marker(bytes)?, 0xc5..=0xc7 | 0xcd..=0xcf).then_some("a differential (hierarchical) JPEG frame"),
        _ => None,
    }
}

/// The 24-bit frame tag of the frame the decoder SELECTS and decodes, found by the chunk dispatch `image-webp` makes
/// (`decoder.rs`): a plain file is its first chunk (`VP8 ` is the frame, `VP8L` a lossless image that names nothing,
/// anything else a file the decoder refuses); an extended file (`VP8X`) is its chunks collected in order, an
/// animated one decoding the first `ANMF` and a still one needing exactly one of `VP8 ` and `VP8L`. Top-level chunks
/// of an animated file are not frames. Inside the first `ANMF` the image is the first child (`VP8 `, `VP8L`, or `ALPH`
/// followed by the chunk that is decoded as `VP8 `); any other first child is rejected by the decoder, so it names
/// nothing, and the walk never leaves the frame it is in. The tag is read only from a complete frame header (a chunk of
/// at least ten bytes, the size of a keyframe's header, inside its container).
fn webp_frame_tag(bytes: &[u8]) -> Option<u32> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let (kind, body, end) = webp_chunk(bytes, 12, bytes.len())?;
    match kind {
        b"VP8 " => vp8_tag(bytes, body, end),
        b"VP8X" => {
            let animated = bytes.get(body)? & 2 != 0;
            let (mut first_vp8, mut has_vp8l, mut has_anim, mut first_anmf) = (None, false, false, None);
            let mut at = end.checked_add(end.checked_sub(body)? & 1)?;
            while let Some((kind, body, end)) = webp_chunk(bytes, at, bytes.len()) {
                match kind {
                    b"VP8 " => first_vp8 = first_vp8.or(Some((body, end))),
                    b"VP8L" => has_vp8l = true,
                    b"ANIM" => has_anim = true,
                    b"ANMF" => first_anmf = first_anmf.or(Some((body, end))),
                    _ => {}
                }
                at = end.checked_add(end.checked_sub(body)? & 1)?;
            }
            if !animated {
                return match (first_vp8, has_vp8l) {
                    (Some((body, end)), false) => vp8_tag(bytes, body, end),
                    _ => None,
                };
            }
            let (body, end) = first_anmf.filter(|_| has_anim)?;
            let inner = body.checked_add(16).filter(|&inner| inner <= end)?;
            let (kind, body, chunk_end) = webp_chunk(bytes, inner, end)?;
            match kind {
                b"VP8 " => vp8_tag(bytes, body, chunk_end),
                b"ALPH" => {
                    let next = chunk_end.checked_add(chunk_end.checked_sub(body)? & 1)?;
                    let (_, body, chunk_end) = webp_chunk(bytes, next, end)?;
                    vp8_tag(bytes, body, chunk_end)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// One RIFF chunk at `at` inside a container ending at `limit`: its tag, where its data starts and where it ends.
fn webp_chunk(bytes: &[u8], at: usize, limit: usize) -> Option<(&[u8], usize, usize)> {
    let body = at.checked_add(8).filter(|&body| body <= limit)?;
    let end = body.checked_add(le32(bytes, at.checked_add(4)?)?).filter(|&end| end <= limit)?;
    Some((bytes.get(at..at.checked_add(4)?)?, body, end))
}

/// The frame tag of a `VP8 ` chunk whose data is `body..end`, when its header is complete.
fn vp8_tag(bytes: &[u8], body: usize, end: usize) -> Option<u32> {
    (end.checked_sub(body)? >= 10).then_some(())?;
    Some(u32::from_le_bytes([*bytes.get(body)?, *bytes.get(body.checked_add(1)?)?, *bytes.get(body.checked_add(2)?)?, 0]))
}

/// The marker of the first start-of-frame segment.
fn jpeg_frame_marker(bytes: &[u8]) -> Option<u8> {
    if bytes.get(..2)? != b"\xff\xd8" {
        return None;
    }
    let mut at = 2usize;
    loop {
        // A marker is introduced by 0xFF, possibly repeated as fill; any other byte is not one.
        if *bytes.get(at)? != 0xff {
            return None;
        }
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        match marker {
            // A frame header is a whole segment: the length, the precision, both dimensions, the component count and
            // that many component descriptors; a marker byte alone, a cut-off segment or a short table is no frame.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                let length = be16(bytes, at)?;
                // The declared component table (one three-byte descriptor each, at least one) is wholly inside it.
                let components = usize::from(*bytes.get(at.checked_add(7)?)?);
                let table = components.checked_mul(3)?.checked_add(8)?;
                return (components >= 1 && length >= table && bytes.get(at..at.checked_add(length)?).is_some()).then_some(marker);
            }
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
