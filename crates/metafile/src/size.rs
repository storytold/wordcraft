//! The size of a metafile picture without playing its records.

use crate::bytes::Bytes;
use crate::{Error, emf, f, parse, wmf};

/// A placeable WMF header is 22 bytes, followed by the 9-word standard header.
const PLACEABLE_KEY: u32 = 0x9AC6_CDD7;
const PLACEABLE_LEN: usize = 22;
const DEFAULT_INCH: f64 = 1440.0;

/// The size of a WMF or EMF picture in points (device or logical units when the file states no
/// size), the same as `parse(bytes)` would put in the picture's `width` and `height`, or the error
/// `parse` would give.
///
/// An EMF and a placeable WMF carry their frame in the header, so the size is read from there
/// without decoding a bitmap or building a path. Any other file (a plain WMF, an EMF without a
/// usable frame) takes the full parse.
pub fn size(bytes: &[u8]) -> Result<(f32, f32), Error> {
    if let Some(size) = header_size(Bytes(bytes)) {
        return Ok(size);
    }
    let pic = parse(bytes)?;
    Ok((pic.width, pic.height))
}

/// The size when the header decides it and the file has a first record (otherwise `parse` fails
/// with `NoRecords`, so the header alone is not enough). None sends the caller to the full parse.
pub(crate) fn header_size(b: Bytes) -> Option<(f32, f32)> {
    if emf::is_emf(b) {
        return emf_size(b);
    }
    if wmf::is_wmf(b) && b.u32(0) == Some(PLACEABLE_KEY) {
        return placeable_wmf_size(b);
    }
    None
}

/// `rclFrame` mapped to points. The frame must pass the checks `build` applies to it.
fn emf_size(b: Bytes) -> Option<(f32, f32)> {
    let hsize = b.u32(4)? as usize;
    b.slice(8, 16)?;
    let frame = emf::frame_from_header(b)?;
    if !frame.rect.iter().all(|v| v.is_finite()) || frame.rect[2] <= 0.0 || frame.rect[3] <= 0.0 {
        return None;
    }
    // The records start after the header; the parse needs one that is not the end record.
    let pos = hsize.max(88);
    let (typ, len) = (b.u32(pos)?, b.u32(pos.checked_add(4)?)? as usize);
    if len < 8 || !len.is_multiple_of(4) || b.slice(pos, len).is_none() || typ == 14 {
        return None;
    }
    let (w, h) = frame.size?;
    Some((f(w).unwrap_or(0.0), f(h).unwrap_or(0.0)))
}

/// The placeable header's size in points, for a file whose records leave the target mapping alone
/// (see [`sets_the_mapping`]); the full parse decides the others.
fn placeable_wmf_size(b: Bytes) -> Option<(f32, f32)> {
    let [l, t, r, bt] = [6, 8, 10, 12].map(|o| b.i16(o).map(f64::from));
    let (l, t, r, bt) = (l?, t?, r?, bt?);
    let inch = b.u16(14).filter(|v| *v > 0).map_or(DEFAULT_INCH, f64::from);
    let (w, h) = (r - l, bt - t);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let hsize = b.u16(PLACEABLE_LEN + 2)? as usize;
    if hsize < 9 {
        return None;
    }
    let pos = PLACEABLE_LEN.checked_add(hsize.checked_mul(2)?)?;
    let (words, func) = (b.u32(pos)? as usize, b.u16(pos.checked_add(4)?)?);
    let len = words.checked_mul(2).filter(|l| *l >= 6)?;
    if func == 0 || b.slice(pos, len).is_none() {
        return None;
    }
    if sets_the_mapping(b, pos) {
        return None;
    }
    Some((f(w * 72.0 / inch)?, f(h * 72.0 / inch)?))
}

/// Whether a record from `pos` on is SetMapMode (0x0103) or SetViewportExt (0x020E). Those can change
/// the frame the full parse ends with (a zero-size viewport leaves none), so the header size is not
/// enough. The records are only walked, not played, with the same stops as `wmf::parse`.
fn sets_the_mapping(b: Bytes, mut pos: usize) -> bool {
    while let (Some(words), Some(func)) = (b.u32(pos), pos.checked_add(4).and_then(|o| b.u16(o))) {
        let Some(len) = (words as usize).checked_mul(2).filter(|l| *l >= 6) else { return false };
        if func == 0 || b.slice(pos, len).is_none() {
            return false;
        }
        if matches!(func, 0x0103 | 0x020E) {
            return true;
        }
        let Some(next) = pos.checked_add(len) else { return false };
        pos = next;
    }
    false
}
