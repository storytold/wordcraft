use super::types::Img;
use super::*;

// ---------------------------------------------------------------------------------------------
// Shared helpers

/// Image extension from bytes, if it's a format we can display.
pub fn sniff_image(data: &[u8]) -> Option<&'static str> {
    match image::guess_format(data).ok()? {
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::Jpeg => Some("jpeg"),
        image::ImageFormat::Gif => Some("gif"),
        image::ImageFormat::WebP => Some("webp"),
        image::ImageFormat::Bmp => Some("bmp"),
        _ => None,
    }
}

/// Pixel size of encoded image bytes.
pub fn image_px(data: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?.into_dimensions().ok()
}

/// Decode an encoded image within the budget pictures are drawn within, so a small file that
/// declares huge dimensions can't exhaust memory when a picture is re-encoded on export. `None`
/// when it would be too large or doesn't decode.
pub fn decode_bounded(data: &[u8]) -> Option<image::DynamicImage> {
    wordcraft_geom::picture::decode(data, wordcraft_geom::MAX_DECODED_BYTES).ok()
}

/// A picture from bytes with an optional display size (points); the natural size (96 dpi) is
/// used otherwise, shrunk to fit a 6.5" column.
pub fn make_img(data: impl Into<Arc<Vec<u8>>>, w: Option<f32>, h: Option<f32>, alt: &str) -> Option<Img> {
    let data = data.into();
    let ext = sniff_image(&data)?;
    let (pw, ph) = image_px(&data).map(|(a, b)| (a as f32 * 0.75, b as f32 * 0.75)).unwrap_or((72.0, 72.0));
    let (pw, ph) = (pw.max(1.0), ph.max(1.0));
    let (mut w, mut h) = match (w.filter(|v| v.is_finite() && *v > 0.0), h.filter(|v| v.is_finite() && *v > 0.0)) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * ph / pw),
        (None, Some(h)) => (h * pw / ph, h),
        (None, None) => (pw, ph),
    };
    if w > 468.0 {
        h *= 468.0 / w;
        w = 468.0;
    }
    Some(Img { data, ext: ext.to_string(), w: w.clamp(1.0, 1584.0), h: h.clamp(1.0, 1584.0), alt: alt.to_string() })
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let b = [ch.first().copied().unwrap_or(0), ch.get(1).copied().unwrap_or(0), ch.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, v) in idx.iter().enumerate() {
            if i <= ch.len() {
                out.push(B64.get(*v as usize).copied().unwrap_or(b'A') as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Lenient base64 (standard or URL-safe alphabet; whitespace and padding ignored).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Decode a `data:` URI (base64 or percent-encoded).
pub fn data_uri(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.trim().strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    if head.ends_with(";base64") { base64_decode(data) } else { Some(percent_decode(data)) }
}

pub fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%'
            && let Some(h) = b.get(i + 1..i + 3)
            && h.iter().all(u8::is_ascii_hexdigit)
            && let Some(hex) = std::str::from_utf8(h).ok()
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

pub fn mime_of(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}
