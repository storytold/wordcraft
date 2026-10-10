//! Device-independent bitmaps. BITMAPINFOHEADER DIBs at 1/4/8/16/24/32 bpp with BI_RGB or BI_BITFIELDS
//! decode to RGBA8 rows, top-down. Compressed (RLE, JPEG, PNG) and V4/V5-only features are not decoded.

use crate::{MAX_PIXELS, bytes::Bytes};

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
/// Most palette entries a DIB can carry (the colour table never exceeds 2^8).
const MAX_PALETTE: usize = 256;
const ROP_SRCAND: u32 = 0x0088_00C6;
const ROP_SRCPAINT: u32 = 0x00EE_0086;
const ROP_SRCINVERT: u32 = 0x0066_0046;

/// A decoded picture. `bottom_up` records the DIB's native row order, which source rectangles use.
pub(crate) struct Rgba {
    pub w: usize,
    pub h: usize,
    pub bottom_up: bool,
    pub px: Vec<u8>,
}

/// How a decoded bitmap combines with the destination.
#[derive(Clone, Copy)]
pub(crate) enum Blend {
    /// A raster operation (BITBLT, STRETCHDIB and friends).
    Rop(u32),
    /// EMR_ALPHABLEND: a constant alpha (0..=255), times the per-pixel alpha when `per_pixel` is set
    /// (AC_SRC_ALPHA: the source colours are premultiplied by it).
    Alpha { constant: u8, per_pixel: bool },
}

impl Blend {
    /// Whether the decoder must keep the 32-bit alpha byte of each pixel.
    pub fn per_pixel(self) -> bool {
        matches!(self, Blend::Alpha { per_pixel: true, .. })
    }
}

struct Header {
    w: usize,
    h: usize,
    top_down: bool,
    bpp: usize,
    masks: [u32; 3],
    pal_off: usize,
    pal_n: usize,
}

impl Header {
    fn read(info: Bytes) -> Option<Header> {
        let size = info.u32(0)? as usize;
        let w = info.i32(4)?;
        let h = info.i32(8)?;
        let bpp = info.u16(14)? as usize;
        let comp = info.u32(16)?;
        let clr_used = info.u32(32)? as usize;
        if size < 40 || w <= 0 || h == 0 || !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) || !matches!(comp, BI_RGB | BI_BITFIELDS) {
            return None;
        }
        let masks = if comp == BI_BITFIELDS {
            [info.u32(40)?, info.u32(44)?, info.u32(48)?]
        } else if bpp == 16 {
            [0x7C00, 0x03E0, 0x001F]
        } else {
            [0xFF_0000, 0xFF00, 0xFF]
        };
        let masks_len = if comp == BI_BITFIELDS && size == 40 { 12 } else { 0 };
        let pal_off = size.checked_add(masks_len)?;
        // Indexed formats take 2^bpp entries when clrUsed is 0. Deeper formats carry only the
        // entries clrUsed names (WMF DIBs can have them), still capped.
        let pal_n = match (bpp <= 8, clr_used) {
            (true, 0) => 1usize << bpp,
            (true, n) => n.min(1usize << bpp),
            (false, n) => n.min(MAX_PALETTE),
        };
        Some(Header { w: w as usize, h: h.unsigned_abs() as usize, top_down: h < 0, bpp, masks, pal_off, pal_n })
    }
}

/// Byte length of the info part (header, masks, palette) of a DIB, so the pixel bits follow it.
pub(crate) fn info_len(dib: Bytes) -> Option<usize> {
    let h = Header::read(dib)?;
    h.pal_off.checked_add(h.pal_n.checked_mul(4)?)
}

/// Decodes a DIB given as its info part (header plus masks and palette) and its pixel bits. The
/// 32-bit alpha byte is kept only when `keep_alpha` is set (per-pixel alpha blends); otherwise pixels
/// are opaque.
pub(crate) fn decode(info: Bytes, bits: Bytes, keep_alpha: bool) -> Option<Rgba> {
    let hd = Header::read(info)?;
    let pixels = hd.w.checked_mul(hd.h)?;
    if pixels == 0 || pixels > MAX_PIXELS {
        return None;
    }
    let stride = hd.w.checked_mul(hd.bpp)?.checked_add(31)? / 32 * 4;
    let data = bits.slice(0, stride.checked_mul(hd.h)?)?;
    let pal = info.slice(hd.pal_off, hd.pal_n * 4)?;
    let mut px = vec![0u8; pixels * 4];
    for y in 0..hd.h {
        let src_row = if hd.top_down { y } else { hd.h - 1 - y };
        let row = src_row * stride;
        for x in 0..hd.w {
            let p = pixel(data, row, x, &hd, pal, keep_alpha)?;
            let o = (y * hd.w + x) * 4;
            px.get_mut(o..o + 4)?.copy_from_slice(&p);
        }
    }
    Some(Rgba { w: hd.w, h: hd.h, bottom_up: !hd.top_down, px })
}

fn pixel(data: &[u8], row: usize, x: usize, hd: &Header, pal: &[u8], keep_alpha: bool) -> Option<[u8; 4]> {
    let [m_r, m_g, m_b] = hd.masks;
    let [r, g, b] = match hd.bpp {
        1 => {
            let b = *data.get(row + x / 8)?;
            palette(pal, usize::from((b >> (7 - x % 8)) & 1))
        }
        4 => {
            let b = *data.get(row + x / 2)?;
            palette(pal, usize::from(if x.is_multiple_of(2) { b >> 4 } else { b & 0x0F }))
        }
        8 => palette(pal, usize::from(*data.get(row + x)?)),
        24 => {
            let [b, g, r] = data.get(row + x * 3..row + x * 3 + 3)?.try_into().ok()?;
            [r, g, b]
        }
        16 => {
            let o = row + x * 2;
            let v = u32::from(u16::from_le_bytes(data.get(o..o + 2)?.try_into().ok()?));
            [field(v, m_r), field(v, m_g), field(v, m_b)]
        }
        _ => {
            let o = row + x * 4;
            let v = u32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?);
            let a = if keep_alpha { (v >> 24) as u8 } else { 255 };
            return Some([field(v, m_r), field(v, m_g), field(v, m_b), a]);
        }
    };
    Some([r, g, b, 255])
}

/// Palette entries are BGRX quads; an index past the table reads as black rather than failing the image.
fn palette(pal: &[u8], i: usize) -> [u8; 3] {
    pal.get(i * 4..i * 4 + 4).and_then(|c| <[u8; 4]>::try_from(c).ok()).map_or([0, 0, 0], |c| [c[2], c[1], c[0]])
}

/// Extracts one channel from a masked pixel value and scales it to 8 bits.
fn field(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = u64::from(mask >> shift);
    let val = u64::from((v & mask) >> shift);
    ((val * 255 + max / 2) / max) as u8
}

/// Clips a source rectangle `src` = `[x, y, w, h]` to the bitmap and returns the cropped pixels (top-down)
/// with the part of the destination `dst` that they fill. `src` is in DIB pixel coordinates (`y` counted
/// from the DIB's origin, so bottom-up images count from the bottom); a non-positive source size means the
/// whole bitmap. Clipping shrinks the destination by the same ratio and shifts it by the clipped-off part,
/// so the rest of the image keeps its place. A negative destination extent is kept (the caller mirrors).
pub(crate) fn clip_blit(img: &Rgba, src: [i64; 4], dst: [f64; 4]) -> Option<(Rgba, [f64; 4])> {
    let (iw, ih) = (img.w as i64, img.h as i64);
    let [sx, sy, sw, sh] = if src[2] > 0 && src[3] > 0 { src } else { [0, 0, iw, ih] };
    // The request's top row in top-down order.
    let top = if img.bottom_up { ih.saturating_sub(sy).saturating_sub(sh) } else { sy };
    let (x0, x1) = (sx.max(0), sx.saturating_add(sw).min(iw));
    let (r0, r1) = (top.max(0), top.saturating_add(sh).min(ih));
    if x1 <= x0 || r1 <= r0 {
        return None;
    }
    let (kx, ky) = (dst[2] / sw as f64, dst[3] / sh as f64);
    let out = [dst[0] + (x0 - sx) as f64 * kx, dst[1] + (r0 - top) as f64 * ky, (x1 - x0) as f64 * kx, (r1 - r0) as f64 * ky];
    let (x0, x1, r0, r1) = (x0 as usize, x1 as usize, r0 as usize, r1 as usize);
    let (ow, oh) = (x1 - x0, r1 - r0);
    let mut px = Vec::with_capacity(ow * oh * 4);
    for row in r0..r1 {
        let s = (row * img.w + x0) * 4;
        px.extend_from_slice(img.px.get(s..s + ow * 4)?);
    }
    Some((Rgba { w: ow, h: oh, bottom_up: false, px }, out))
}

/// Applies a blend to decoded pixels before they are drawn. The picture has no destination pixels, so
/// raster operations are drawn as they look over white paper: SRCAND (destination AND source) leaves
/// white source pixels out and draws the rest; SRCPAINT and SRCINVERT leave black ones out. Other
/// operations draw the source as copied. Constant alpha scales every pixel's alpha, after a
/// per-pixel (premultiplied) source is made straight.
pub(crate) fn prepare(img: &mut Rgba, blend: Blend) {
    match blend {
        Blend::Rop(ROP_SRCAND) => clear_where(img, [255, 255, 255]),
        Blend::Rop(ROP_SRCPAINT | ROP_SRCINVERT) => clear_where(img, [0, 0, 0]),
        Blend::Rop(_) => {}
        Blend::Alpha { constant, per_pixel } => {
            for p in img.px.as_chunks_mut::<4>().0 {
                if per_pixel {
                    unpremultiply(p);
                }
                p[3] = (u16::from(p[3]) * u16::from(constant) / 255) as u8;
            }
        }
    }
}

/// Makes the pixels of colour `rgb` transparent: where a raster operation keeps the destination.
fn clear_where(img: &mut Rgba, rgb: [u8; 3]) {
    for p in img.px.as_chunks_mut::<4>().0 {
        if p[..3] == rgb {
            p[3] = 0;
        }
    }
}

/// AC_SRC_ALPHA sources are premultiplied; the model holds straight RGBA, so divide the colour by the
/// alpha (rounded, capped at 255). A transparent pixel keeps its colour bytes.
fn unpremultiply(p: &mut [u8; 4]) {
    let a = u32::from(p[3]);
    if a == 0 || a == 255 {
        return;
    }
    for c in &mut p[..3] {
        *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
    }
}

/// Mirrors a decoded image left-right and/or top-bottom. Rows come from the buffer as chunks, so a
/// short buffer flips only the rows it has.
pub(crate) fn flip(img: &mut Rgba, horizontal: bool, vertical: bool) {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 {
        return;
    }
    let row = w * 4;
    if horizontal {
        for line in img.px.chunks_exact_mut(row) {
            for x in 0..w / 2 {
                for c in 0..4 {
                    line.swap(x * 4 + c, (w - 1 - x) * 4 + c);
                }
            }
        }
    }
    if vertical {
        // Rows the buffer actually holds: a short buffer flips only those.
        let n = h.min(img.px.len() / row);
        for y in 0..n / 2 {
            swap_rows(&mut img.px, y * row, (n - 1 - y) * row, row);
        }
    }
}

/// Swaps the `len` bytes at `a` with those at `b` (`a < b`); does nothing when either is out of range.
fn swap_rows(px: &mut [u8], a: usize, b: usize, len: usize) {
    let Some((head, tail)) = px.split_at_mut_checked(b) else { return };
    if let (Some(top), Some(bottom)) = (head.get_mut(a..a.saturating_add(len)), tail.get_mut(..len)) {
        top.swap_with_slice(bottom);
    }
}
