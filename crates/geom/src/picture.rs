//! Decoding a picture within a memory budget. Every crate that decodes a document's or the
//! clipboard's pictures goes through [`probe`] (or [`probe_dib`]): the header gives the format and
//! size, and the decode is refused before anything is allocated when its peak would pass the
//! budget. The peak counts what a decoder holds beside its output (a progressive JPEG's
//! coefficients, #3644), and a picture whose decoder can't be held to a budget is not decoded at
//! all (a TIFF with JPEG strips, #3645).

use std::borrow::Cow;

use craft_image_codecs::{
    ChannelLayout, CodecError, DecodeOptions, EncodeOptions, Format as CodecFormat, Image as CodecImage, Limits as CodecLimits, RasterColorType,
    SampleType, bmp_probe, decode_as_with, detect, encode as codec_encode, inspect_raster, jpeg_worst_case_coefficients, png_probe, probe_size,
    webp_probe,
};
use image::{DynamicImage, ImageBuffer, ImageFormat};

/// Why a picture isn't decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a raster picture WordCraft decodes (a metafile, SVG or other bytes).
    NotAPicture,
    /// A kind of picture that is never decoded, because its decoder can't be held to a budget.
    Unsupported(&'static str),
    /// Decoding would exceed the memory budget or size arithmetic overflowed.
    TooLarge { width: u32, height: u32 },
    /// The header or data is damaged, or the decoder rejects the input.
    Broken(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotAPicture => f.write_str("not a supported picture (PNG, JPEG, GIF, WebP, BMP or TIFF)"),
            Refusal::Unsupported(what) => f.write_str(what),
            Refusal::TooLarge { width, height } => write!(f, "the picture is too large ({width} × {height} pixels)"),
            Refusal::Broken(why) => write!(f, "the picture is damaged ({why})"),
        }
    }
}

impl std::error::Error for Refusal {}

fn broken(e: impl std::fmt::Display) -> Refusal {
    Refusal::Broken(e.to_string())
}

fn to_codec_format(format: ImageFormat) -> Option<CodecFormat> {
    match format {
        ImageFormat::Png => Some(CodecFormat::Png),
        ImageFormat::Jpeg => Some(CodecFormat::Jpeg),
        ImageFormat::Gif => Some(CodecFormat::Gif),
        ImageFormat::WebP => Some(CodecFormat::WebP),
        ImageFormat::Bmp => Some(CodecFormat::Bmp),
        ImageFormat::Tiff => Some(CodecFormat::Tiff),
        _ => None,
    }
}

fn from_codec_format(format: CodecFormat) -> Option<ImageFormat> {
    match format {
        CodecFormat::Png => Some(ImageFormat::Png),
        CodecFormat::Jpeg => Some(ImageFormat::Jpeg),
        CodecFormat::Gif => Some(ImageFormat::Gif),
        CodecFormat::WebP => Some(ImageFormat::WebP),
        CodecFormat::Bmp => Some(ImageFormat::Bmp),
        CodecFormat::Tiff => Some(ImageFormat::Tiff),
        _ => None,
    }
}

fn codec_limits(max_width: Option<u32>, max_height: Option<u32>, max_bytes: u64) -> CodecLimits {
    let mut limits = CodecLimits::default();
    if let Some(w) = max_width {
        limits.max_width = w;
    }
    if let Some(h) = max_height {
        limits.max_height = h;
    }
    if let (Some(w), Some(h)) = (max_width, max_height) {
        limits.max_pixels = u64::from(w).saturating_mul(u64::from(h));
    }
    limits.max_alloc = max_bytes;
    limits
}

fn dynamic_to_codec(img: &DynamicImage) -> Result<CodecImage, String> {
    let (w, h) = (img.width(), img.height());
    match img {
        DynamicImage::ImageLuma8(b) => CodecImage::from_u8(w, h, ChannelLayout::Gray, b.as_raw().clone()),
        DynamicImage::ImageLumaA8(b) => CodecImage::from_u8(w, h, ChannelLayout::GrayA, b.as_raw().clone()),
        DynamicImage::ImageRgb8(b) => CodecImage::from_u8(w, h, ChannelLayout::Rgb, b.as_raw().clone()),
        DynamicImage::ImageRgba8(b) => CodecImage::from_u8(w, h, ChannelLayout::Rgba, b.as_raw().clone()),
        DynamicImage::ImageLuma16(b) => CodecImage::from_u16(w, h, ChannelLayout::Gray, b.as_raw()),
        DynamicImage::ImageLumaA16(b) => CodecImage::from_u16(w, h, ChannelLayout::GrayA, b.as_raw()),
        DynamicImage::ImageRgb16(b) => CodecImage::from_u16(w, h, ChannelLayout::Rgb, b.as_raw()),
        DynamicImage::ImageRgba16(b) => CodecImage::from_u16(w, h, ChannelLayout::Rgba, b.as_raw()),
        _ => CodecImage::from_u8(w, h, ChannelLayout::Rgba, img.to_rgba8().into_raw()),
    }
    .map_err(|e| e.to_string())
}

fn codec_to_dynamic(img: CodecImage) -> Result<DynamicImage, Refusal> {
    let (w, h) = (img.width(), img.height());
    let layout = img.layout();
    let st = img.sample_type();
    if st == SampleType::U8 {
        let raw = img.data().to_vec();
        return match layout {
            ChannelLayout::Gray => ImageBuffer::from_raw(w, h, raw).map(DynamicImage::ImageLuma8),
            ChannelLayout::GrayA => ImageBuffer::from_raw(w, h, raw).map(DynamicImage::ImageLumaA8),
            ChannelLayout::Rgb => ImageBuffer::from_raw(w, h, raw).map(DynamicImage::ImageRgb8),
            ChannelLayout::Rgba => ImageBuffer::from_raw(w, h, raw).map(DynamicImage::ImageRgba8),
            _ => None,
        }
        .ok_or_else(|| Refusal::Broken("decoded image buffer size mismatch".into()));
    }
    let rgba8 = img.to_rgba8();
    ImageBuffer::from_raw(w, h, rgba8).map(DynamicImage::ImageRgba8).ok_or_else(|| Refusal::Broken("decoded image buffer size mismatch".into()))
}

/// Encode `img` as PNG via `craft-image-codecs`.
pub fn encode_png(img: &DynamicImage) -> Result<Vec<u8>, String> {
    encode_format(img, ImageFormat::Png)
}

/// Encode `img` in `format` via `craft-image-codecs`.
pub fn encode_format(img: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, String> {
    let codec_img = dynamic_to_codec(img)?;
    let codec_fmt = to_codec_format(format).ok_or_else(|| format!("unsupported format: {format:?}"))?;
    codec_encode(&codec_img, codec_fmt, &EncodeOptions::default()).map_err(|e| e.to_string())
}

/// Decode a raster image from memory using `craft-image-codecs` with default limits.
pub fn decode_memory(bytes: &[u8]) -> Result<DynamicImage, Refusal> {
    let codec_fmt = detect(bytes).ok_or(Refusal::NotAPicture)?;
    let img = decode_as_with(codec_fmt, bytes, &DecodeOptions { limits: CodecLimits::default(), keep_orientation: true }).map_err(broken)?;
    codec_to_dynamic(img)
}

/// Encode tightly-packed RGB8 pixels as JPEG at `quality` (1..=100) via `craft-image-codecs`.
pub fn encode_jpeg(rgb: &[u8], width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
    let codec_img = CodecImage::from_u8(width, height, ChannelLayout::Rgb, rgb.to_vec()).map_err(|e| e.to_string())?;
    let opts = EncodeOptions { jpeg_quality: quality.clamp(1, 100), ..EncodeOptions::default() };
    codec_encode(&codec_img, CodecFormat::Jpeg, &opts).map_err(|e| e.to_string())
}

/// Read `(width, height)` from a raster image header without decoding pixels.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let format = detect(bytes)?;
    match format {
        CodecFormat::Png => png_probe::pixel_model(bytes).ok().map(|m| (m.width, m.height)),
        CodecFormat::Jpeg | CodecFormat::Tiff | CodecFormat::Gif | CodecFormat::Bmp => {
            inspect_raster(bytes).ok().flatten().map(|i| (i.width, i.height))
        }
        CodecFormat::WebP => probe_size(bytes).ok(),
        _ => None,
    }
}

/// Guess the raster [`ImageFormat`] from magic bytes.
pub fn guess_format(bytes: &[u8]) -> Option<ImageFormat> {
    detect(bytes).and_then(from_codec_format)
}

/// Whether a raster picture's header indicates an alpha channel.
pub fn header_has_alpha(bytes: &[u8]) -> bool {
    probe(bytes).is_ok_and(|p| p.has_alpha())
}

/// What a picture's header says: its format, its size and the memory decoding it takes. Reading
/// it allocates no pixels.
#[derive(Clone, Debug)]
pub struct Probe<'a> {
    bytes: Cow<'a, [u8]>,
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    peak: u64,
    alpha: bool,
}

/// TIFF compressions decoded within the caller's limits: none, CCITT Group 4, LZW, Deflate (both
/// codes) and `PackBits`. The `tiff` crate decodes a JPEG strip (7) with a decoder of its own and
/// default limits, whatever size the TIFF declares, so such a TIFF is not decoded (#3645).
const TIFF_COMPRESSIONS: [u16; 6] = [1, 4, 5, 8, 32946, 32773];

/// Read a picture's header: PNG, JPEG, GIF, WebP, BMP or TIFF.
///
/// # Errors
///
/// Returns `Refusal::NotAPicture` for an unsupported format, `Refusal::Unsupported` for a TIFF
/// whose compression cannot be read or budgeted (including truncation that prevents reading
/// its compression metadata), `Refusal::TooLarge`
/// on size arithmetic overflow, and `Refusal::Broken` for other header or inspection errors,
/// including TIFF errors after compression metadata is accepted.
pub fn probe(bytes: &[u8]) -> Result<Probe<'_>, Refusal> {
    let codec_fmt = detect(bytes).ok_or(Refusal::NotAPicture)?;
    let format = from_codec_format(codec_fmt).ok_or(Refusal::NotAPicture)?;
    let held = match format {
        ImageFormat::Png | ImageFormat::Gif | ImageFormat::WebP | ImageFormat::Bmp => 0,
        ImageFormat::Jpeg => jpeg_worst_case_coefficients(bytes),
        ImageFormat::Tiff => {
            if !tiff_compression(bytes).is_some_and(|c| TIFF_COMPRESSIONS.contains(&c)) {
                return Err(Refusal::Unsupported("TIFF pictures with JPEG or an unknown compression can't be read"));
            }
            0
        }
        _ => return Err(Refusal::NotAPicture),
    };
    if format == ImageFormat::Png {
        // Measured from `IHDR` and the chunk headers: an ICC profile (`iCCP`) isn't inflated, though the
        // decoder's own header read inflates one up to its 512 MiB limit.
        let m = png_probe::pixel_model(bytes).map_err(|_| Refusal::Broken("its PNG header can't be read".into()))?;
        let (width, height, native, alpha) = (m.width, m.height, u64::from(m.bytes_per_pixel()), m.has_alpha());
        let (width, height, peak) = sized(width, height, native, 0)?;
        return Ok(Probe { bytes: Cow::Borrowed(bytes), format, width, height, peak, alpha });
    }
    if format == ImageFormat::WebP {
        let (width, height) = probe_size(bytes).map_err(broken)?;
        let (width, height, peak) = sized(width, height, 4, 0)?;
        let alpha = webp_probe::has_alpha(bytes).unwrap_or(false);
        return Ok(Probe { bytes: Cow::Borrowed(bytes), format, width, height, peak, alpha });
    }
    let info = inspect_raster(bytes).map_err(broken)?.ok_or_else(|| Refusal::Broken("its header can't be read".into()))?;
    let native = u64::from(info.channels.max(1)) * u64::from(info.bits_per_sample.max(8) / 8);
    let (width, height, peak) = sized(info.width, info.height, native, held)?;
    let alpha = matches!(info.color_type, RasterColorType::GrayAlpha | RasterColorType::Rgba | RasterColorType::CmykAlpha)
        || info.transparent_index.is_some();
    Ok(Probe { bytes: Cow::Borrowed(bytes), format, width, height, peak, alpha })
}

/// [`bmp_probe::dib_to_bmp`], its failure (the BMP's bytes can't be allocated) as the refusal a
/// picture too large to hold gets, with the size its header declares (0 × 0 when it can't be read).
fn dib_to_bmp(dib: &[u8]) -> Result<Vec<u8>, Refusal> {
    bmp_probe::dib_to_bmp(dib).map_err(|_| {
        let (width, height) = dib_header_dims(dib).unwrap_or((0, 0));
        Refusal::TooLarge { width, height }
    })
}

/// Read a DIB's header: a BMP without its 14-byte file header, as the Windows clipboard holds
/// one (`CF_DIB`, `CF_DIBV5`). Its layout is first made the one the decoder reads (see
/// [`bmp_probe::dib_layout`]).
///
/// # Errors
///
/// Returns `Refusal::Broken` for an invalid DIB header and `Refusal::TooLarge` on size arithmetic
/// overflow.
pub fn probe_dib(dib: Vec<u8>) -> Result<Probe<'static>, Refusal> {
    let dib = bmp_probe::dib_layout(dib);
    let bmp = dib_to_bmp(&dib)?;
    let (width, height, native, alpha) = match inspect_raster(&bmp) {
        Ok(Some(info)) => {
            let native = u64::from(info.channels.max(1)) * u64::from(info.bits_per_sample.max(8) / 8);
            let alpha = matches!(info.color_type, RasterColorType::GrayAlpha | RasterColorType::Rgba);
            (info.width, info.height, native, alpha)
        }
        Ok(None) => return Err(Refusal::Broken("its DIB header can't be read".into())),
        Err(CodecError::LimitExceeded(_)) => {
            let (w, h) = dib_header_dims(&dib).ok_or_else(|| Refusal::Broken("its DIB header can't be read".into()))?;
            (w, h, 4, false)
        }
        Err(e) => return Err(broken(e)),
    };
    let (width, height, peak) = sized(width, height, native, 0)?;
    Ok(Probe { bytes: Cow::Owned(bmp), format: ImageFormat::Bmp, width, height, peak, alpha })
}

/// A DIB's width and height, read by the shared BMP probe. Header sizes other than 12 (core) and
/// 40 or more (info) are refused here: the probe would read them as info headers.
fn dib_header_dims(dib: &[u8]) -> Option<(u32, u32)> {
    let bi_size = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?);
    if bi_size != 12 && bi_size < 40 {
        return None;
    }
    // Only the first 12 header bytes are read, so only they are wrapped: this runs for DIBs too
    // big to decode, which must not be copied whole.
    craft_image_codecs::bmp_probe::dimensions(&bmp_probe::dib_to_bmp(dib.get(..12).unwrap_or(dib)).ok()?)
}

/// A `width` × `height` picture of `native` bytes a pixel, with its peak (see [`peak_bytes`]).
fn sized(width: u32, height: u32, native: u64, held: u64) -> Result<(u32, u32, u64), Refusal> {
    if width == 0 || height == 0 {
        return Err(Refusal::Broken("it has no pixels".into()));
    }
    let peak = peak_bytes(width, height, native, held).ok_or(Refusal::TooLarge { width, height })?;
    Ok((width, height, peak))
}

impl Probe<'_> {
    /// Bytes decoding takes at its peak.
    pub fn peak_bytes(&self) -> u64 {
        self.peak
    }

    /// Whether the picture has transparency, from its header.
    pub fn has_alpha(&self) -> bool {
        self.alpha
    }

    /// Whether decoding fits `budget` bytes.
    pub fn fits(&self, budget: u64) -> bool {
        self.peak <= budget
    }

    /// Decode, refused before anything is allocated when the peak would pass `budget` bytes. The
    /// decoder is held to the header's size and to `budget` as well.
    ///
    /// # Errors
    ///
    /// Returns `Refusal::TooLarge` when the picture does not fit `budget` or the decoder
    /// exceeds its limits, `Refusal::NotAPicture` for an unsupported format, and
    /// `Refusal::Broken` for other decoding or pixel-conversion errors.
    pub fn decode(&self, budget: u64) -> Result<DynamicImage, Refusal> {
        if !self.fits(budget) {
            return Err(Refusal::TooLarge { width: self.width, height: self.height });
        }
        let codec_fmt = to_codec_format(self.format).ok_or(Refusal::NotAPicture)?;
        let opts = DecodeOptions { limits: codec_limits(Some(self.width), Some(self.height), budget), keep_orientation: true };
        let img = decode_as_with(codec_fmt, &self.bytes, &opts).map_err(|e| match e {
            CodecError::LimitExceeded(_) => Refusal::TooLarge { width: self.width, height: self.height },
            other => broken(other),
        })?;
        codec_to_dynamic(img)
    }
}

/// [`probe`], then [`Probe::decode`] within `budget` bytes.
///
/// # Errors
///
/// Returns the probing or decoding `Refusal`; see [`probe`] and [`Probe::decode`].
pub fn decode(bytes: &[u8], budget: u64) -> Result<DynamicImage, Refusal> {
    probe(bytes)?.decode(budget)
}

/// Why a picture can't be drawn, as strict export reports it (#6005).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawFailure {
    /// A picture format we know, but it doesn't decode (damaged, truncated, too large).
    Undecodable,
    /// Bytes that are not a picture format we draw.
    UnsupportedFormat,
}

impl DrawFailure {
    /// The words reports use.
    pub fn label(self) -> &'static str {
        match self {
            DrawFailure::Undecodable => "undecodable",
            DrawFailure::UnsupportedFormat => "unsupported format",
        }
    }
}

impl From<&Refusal> for DrawFailure {
    fn from(r: &Refusal) -> Self {
        match r {
            Refusal::NotAPicture | Refusal::Unsupported(_) => DrawFailure::UnsupportedFormat,
            Refusal::TooLarge { .. } | Refusal::Broken(_) => DrawFailure::Undecodable,
        }
    }
}

/// Decode a raster picture for drawing: the one decision the PDF export, the raster renderer and
/// strict export's check all take. The decoder is held to `max_side` pixels a side and the header
/// is charged against `budget` bytes before anything is allocated. A picture with no pixels or a
/// side past `max_side` is refused.
pub fn decode_for_drawing(bytes: &[u8], budget: u64, max_side: u32) -> Result<DynamicImage, Refusal> {
    let p = probe(bytes)?;
    if p.width > max_side || p.height > max_side {
        return Err(Refusal::Broken("it has no pixels or a side past the limit".into()));
    }
    if !p.fits(budget) {
        return Err(Refusal::TooLarge { width: p.width, height: p.height });
    }
    let codec_fmt = to_codec_format(p.format).ok_or(Refusal::NotAPicture)?;
    let opts = DecodeOptions { limits: codec_limits(Some(max_side), Some(max_side), budget), keep_orientation: true };
    let img = decode_as_with(codec_fmt, bytes, &opts).map_err(|e| match e {
        CodecError::LimitExceeded(_) => Refusal::TooLarge { width: p.width, height: p.height },
        other => broken(other),
    })?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 || w > max_side || h > max_side {
        return Err(Refusal::Broken("it has no pixels or a side past the limit".into()));
    }
    codec_to_dynamic(img)
}

/// Largest page-background picture the page raster draws, in pixels (hostile files).
pub const MAX_BACKGROUND_PIXELS: u64 = 64 * 1024 * 1024;

/// The pixel size of a page-background picture the page raster draws (Design > Page Color > Fill
/// Effects), or None when it leaves the background out: not a raster picture it can read (a
/// metafile is not) or one with no pixels or more than [`MAX_BACKGROUND_PIXELS`]. The raster and
/// the strict check both ask this, so they can't drift.
pub fn background_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let (w, h) = dimensions(bytes)?;
    (w > 0 && h > 0 && u64::from(w) * u64::from(h) <= MAX_BACKGROUND_PIXELS).then_some((w, h))
}

/// Whether a raster picture decodes for drawing, by the rule the exporters use ([`probe`] within
/// [`crate::MAX_DECODED_BYTES`], then [`decode_for_drawing`] with `max_side`).
pub fn check_for_drawing(bytes: &[u8], max_side: u32) -> Result<(), DrawFailure> {
    let fail = |r: Refusal| DrawFailure::from(&r);
    if !probe(bytes).map_err(fail)?.fits(crate::MAX_DECODED_BYTES) {
        return Err(DrawFailure::Undecodable);
    }
    decode_for_drawing(bytes, crate::MAX_DECODED_BYTES, max_side).map(|_| ()).map_err(fail)
}

/// Bytes decoding a `width` × `height` picture holds at its peak: the decoder's output (`native`
/// bytes a pixel) beside what the decoder keeps while it decodes (`held`), then the output beside
/// its RGBA8 conversion, then that beside a premultiplied copy. None on overflow.
fn peak_bytes(width: u32, height: u32, native: u64, held: u64) -> Option<u64> {
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    let decoding = pixels.checked_mul(native)?.checked_add(held)?;
    let converting = pixels.checked_mul(native.checked_add(4)?)?;
    let premultiplying = pixels.checked_mul(8)?;
    Some(decoding.max(converting).max(premultiplying))
}

/// The compression of a classic TIFF's first image (the one the image crate decodes): 1 when the
/// tag is missing. None when the header can't be read, or when the tag appears more than once:
/// the decoder keeps the last copy, so a harmless first one must not hide a JPEG one after it.
fn tiff_compression(bytes: &[u8]) -> Option<u16> {
    craft_tiff::probe::first_image_compression(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hand-written reader the shared probe replaced; kept as the oracle.
    fn dib_header_dims_old(dib: &[u8]) -> Option<(u32, u32)> {
        let bi_size = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?);
        if bi_size == 12 {
            let w = u16::from_le_bytes(dib.get(4..6)?.try_into().ok()?);
            let h = u16::from_le_bytes(dib.get(6..8)?.try_into().ok()?);
            Some((u32::from(w), u32::from(h)))
        } else if bi_size >= 40 {
            let w = i32::from_le_bytes(dib.get(4..8)?.try_into().ok()?);
            let h = i32::from_le_bytes(dib.get(8..12)?.try_into().ok()?);
            Some((u32::try_from(w).ok()?, h.unsigned_abs()))
        } else {
            None
        }
    }

    fn hdr(size: u32, w: i32, h: i32, len: usize) -> Vec<u8> {
        let mut v = [0u8; 124].to_vec();
        v.resize(124.max(len), 0);
        v[0..4].copy_from_slice(&size.to_le_bytes());
        v[4..8].copy_from_slice(&w.to_le_bytes());
        v[8..12].copy_from_slice(&h.to_le_bytes());
        v.truncate(len);
        v
    }

    #[test]
    fn dib_dims_match_the_hand_written_reader() {
        let sizes = [0u32, 11, 12, 13, 16, 39, 40, 41, 52, 108, 124, u32::MAX];
        let dims = [(0, 0), (1, 1), (3, -2), (0, 5), (7, i32::MIN), (i32::MIN, 1), (-1, 1), (65536, 70000), (i32::MAX, i32::MAX)];
        for &s in &sizes {
            for &(w, h) in &dims {
                for len in 0..=130 {
                    let d = hdr(s, w, h, len);
                    assert_eq!(dib_header_dims(&d), dib_header_dims_old(&d), "size {s} w {w} h {h} len {len}");
                }
            }
        }
        assert_eq!(dib_header_dims(&hdr(40, 3, -2, 40)), Some((3, 2)));
        let mut core = hdr(12, 3, 0, 12);
        core[6..8].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(dib_header_dims(&core), Some((3, 2)));
    }

    #[test]
    fn dib_dims_never_panic_on_junk() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut buf = [0u8; 64];
        for _ in 0..2000 {
            for b in buf.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *b = x as u8;
            }
            for n in 0..=buf.len() {
                assert_eq!(dib_header_dims(&buf[..n]), dib_header_dims_old(&buf[..n]));
                let _ = probe_dib(buf[..n].to_vec());
            }
        }
    }

    fn encode(img: impl Into<DynamicImage>, format: ImageFormat) -> Vec<u8> {
        let dyn_img = img.into();
        let codec_img = dynamic_to_codec(&dyn_img).unwrap();
        let codec_fmt = to_codec_format(format).unwrap();
        codec_encode(&codec_img, codec_fmt, &EncodeOptions::default()).unwrap()
    }

    #[test]
    fn drawing_check_tells_unsupported_from_undecodable() {
        let png = encode(image::RgbaImage::new(4, 4), ImageFormat::Png);
        assert_eq!(check_for_drawing(&png, 100), Ok(()));
        assert_eq!(check_for_drawing(b"junk", 100), Err(DrawFailure::UnsupportedFormat));
        assert_eq!(check_for_drawing(&png[..png.len() - 20], 100), Err(DrawFailure::Undecodable));
        // A side past the limit is refused, by the same function the exporters draw with.
        assert_eq!(check_for_drawing(&png, 3), Err(DrawFailure::Undecodable));
        assert!(decode_for_drawing(&png, crate::MAX_DECODED_BYTES, 100).is_ok());
        assert_eq!(DrawFailure::Undecodable.label(), "undecodable");
    }

    /// A PNG whose header declares `w` × `h` pixels, with no pixel data.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut png = encode(image::RgbaImage::new(1, 1), ImageFormat::Png);
        png[16..20].copy_from_slice(&w.to_be_bytes());
        png[20..24].copy_from_slice(&h.to_be_bytes());
        // The IHDR CRC covers the chunk type and data.
        let crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        png
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    /// `png` with a chunk of `kind` holding `data` placed after IHDR.
    fn with_chunk(png: &[u8], kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        let crc = crc32(&chunk[4..]);
        chunk.extend_from_slice(&crc.to_be_bytes());
        let mut out = png.get(..33).unwrap().to_vec();
        out.extend(chunk);
        out.extend_from_slice(&png[33..]);
        out
    }

    #[test]
    fn a_png_is_measured_from_its_chunk_headers() {
        // An ICC profile isn't inflated to measure a picture: one whose deflate stream is junk
        // (the decoder refused it while reading the header) still measures, and isn't decoded.
        let rgb = encode(image::RgbImage::new(4, 2), ImageFormat::Png);
        let icc = with_chunk(&rgb, b"iCCP", b"p\0\0\xFF\xFF\xFF\xFF");
        let p = probe(&icc).unwrap();
        assert_eq!((p.width, p.height, p.peak_bytes()), (4, 2, 4 * 2 * 8));
        assert!(!p.has_alpha());
        assert_eq!(probe(&rgb).unwrap().peak_bytes(), p.peak_bytes(), "as the decoder measures it");
        // Transparency: an alpha channel, or tRNS before the image data.
        assert!(probe(&encode(image::GrayAlphaImage::new(2, 2), ImageFormat::Png)).unwrap().has_alpha());
        let trns = with_chunk(&rgb, b"tRNS", &[0, 0, 0, 0, 0, 0]);
        let p = probe(&trns).unwrap();
        assert!(p.has_alpha());
        assert_eq!(p.decode(1 << 20).map(|i| i.color().has_alpha()), Ok(true), "the decoder agrees");
        // No image data: broken.
        assert!(probe(&rgb[..33]).is_err());
    }

    #[test]
    fn small_pictures_decode_and_peaks_count_every_buffer() {
        let png = encode(image::RgbaImage::from_pixel(16, 8, image::Rgba([1, 2, 3, 4])), ImageFormat::Png);
        let p = probe(&png).unwrap();
        assert_eq!((p.format, p.width, p.height), (ImageFormat::Png, 16, 8));
        assert_eq!(p.peak_bytes(), 16 * 8 * 8, "RGBA8: the decode beside its premultiplied copy");
        assert!(p.fits(1024) && !p.fits(1023));
        assert_eq!(p.decode(1 << 20).unwrap().to_rgba8().get_pixel(3, 3).0, [1, 2, 3, 4]);
        assert_eq!(p.decode(1023), Err(Refusal::TooLarge { width: 16, height: 8 }));
        assert_eq!(peak_bytes(16, 8, 8, 0), Some(16 * 8 * 12), "RGBA16 beside its RGBA8 conversion");
        assert_eq!(peak_bytes(16, 8, 3, 16 * 8 * 8), Some(16 * 8 * 11), "coefficients beside the RGB output");
        assert_eq!(peak_bytes(u32::MAX, u32::MAX, 8, 0), None);
        for bytes in [&b"not a picture"[..], b"", b"\x89PNG"] {
            assert!(probe(bytes).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn a_huge_header_is_refused_before_decoding() {
        let png = png_header(60_000, 60_000);
        let p = probe(&png).unwrap();
        assert_eq!((p.width, p.height), (60_000, 60_000));
        assert!(!p.fits(crate::MAX_DECODED_BYTES));
        assert_eq!(decode(&png, crate::MAX_DECODED_BYTES).unwrap_err(), Refusal::TooLarge { width: 60_000, height: 60_000 });
    }

    /// A baseline JPEG turned progressive in its headers (SOF2, a DC-only first scan), `w` × `h`
    /// with its three components unsubsampled.
    fn progressive_header(w: u16, h: u16) -> Vec<u8> {
        let mut jpeg = encode(image::RgbImage::from_pixel(16, 16, image::Rgb([200, 30, 30])), ImageFormat::Jpeg);
        let sof = jpeg.windows(2).position(|m| m == [0xFF, 0xC0]).unwrap();
        jpeg[sof + 1] = 0xC2;
        jpeg[sof + 5..sof + 7].copy_from_slice(&h.to_be_bytes());
        jpeg[sof + 7..sof + 9].copy_from_slice(&w.to_be_bytes());
        assert_eq!(jpeg[sof + 9], 3);
        for c in 0..3 {
            jpeg[sof + 11 + 3 * c] = 0x11;
        }
        let sos = jpeg.windows(2).position(|m| m == [0xFF, 0xDA]).unwrap();
        let n = usize::from(jpeg[sos + 4]);
        jpeg[sos + 6 + 2 * n] = 0;
        jpeg
    }

    #[test]
    fn progressive_jpeg_coefficients_are_charged() {
        let jpeg = progressive_header(2048, 2048);
        let p = probe(&jpeg).unwrap();
        assert_eq!((p.width, p.height), (2048, 2048));
        // RGB output (3 bytes a pixel) beside 2 bytes a coefficient of three full-size components:
        // 9 bytes a pixel, more than the 8 two RGBA buffers take.
        assert_eq!(p.peak_bytes(), 2048 * 2048 * 9);
        assert!(!p.fits(2048 * 2048 * 8));
        // Baseline: decoded an MCU row at a time, nothing held.
        let baseline = encode(image::RgbImage::new(64, 64), ImageFormat::Jpeg);
        assert_eq!(probe(&baseline).unwrap().peak_bytes(), 64 * 64 * 8);
    }

    /// A JPEG's headers up to the first scan: frames `(marker, precision, w, h, components)`, then
    /// a scan of `scan` components (none: no scan, the end of the image).
    fn jpeg_headers(frames: &[(u8, u8, u16, u16, &[u8])], scan: Option<u8>) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0];
        for &(marker, prec, w, h, comps) in frames {
            b.extend_from_slice(&[0xFF, marker]);
            #[expect(clippy::cast_possible_truncation, reason = "Fixtures hold at most four components.")]
            let (n, len) = ((comps.len() / 3) as u8, (8 + comps.len()) as u16);
            b.extend(len.to_be_bytes());
            b.push(prec);
            b.extend(h.to_be_bytes());
            b.extend(w.to_be_bytes());
            b.push(n);
            b.extend_from_slice(comps);
        }
        if let Some(n) = scan {
            b.extend_from_slice(&[0xFF, 0xDA, 0, 6 + 2 * n, n]);
            (1..=n).for_each(|c| b.extend_from_slice(&[c, 0]));
            b.extend_from_slice(&[0, 63, 0]);
        }
        b.extend_from_slice(&[0xFF, 0xD9]);
        b
    }

    const C444: &[u8] = &[1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0];
    const C420: &[u8] = &[1, 0x22, 0, 2, 0x11, 0, 3, 0x11, 0];
    const GREY: &[u8] = &[1, 0x11, 0];

    /// The JPEG budget before the cutover to the shared rule, as the oracle: the shared charge is
    /// never less, and is equal wherever it adds nothing.
    #[test]
    fn jpeg_charge_is_never_less_than_the_local_one_it_replaced() {
        let wide = &[1, 0xFF, 0, 2, 0xFF, 0, 3, 0xFF, 0];
        let cmyk = &[1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 4, 0x11, 0];
        let zero = &[1, 0x00, 0, 2, 0x00, 0, 3, 0x00, 0];
        // (case, whether the shared rule is expected to charge more).
        let cases: Vec<(Vec<u8>, bool)> = vec![
            (jpeg_headers(&[(0xC0, 8, 640, 480, C420)], Some(3)), false),
            (jpeg_headers(&[(0xC0, 8, 640, 480, GREY)], Some(1)), false),
            (jpeg_headers(&[(0xC0, 8, 640, 480, C444)], Some(1)), true),
            (jpeg_headers(&[(0xC1, 8, 640, 480, C420)], Some(2)), true),
            (jpeg_headers(&[(0xC0, 8, 640, 480, C444)], None), false),
            (jpeg_headers(&[(0xC2, 8, 640, 480, C420)], Some(1)), false),
            (jpeg_headers(&[(0xC2, 8, 640, 480, C444)], Some(3)), false),
            (jpeg_headers(&[(0xC2, 12, 640, 480, C444)], Some(1)), false),
            (jpeg_headers(&[(0xC2, 8, 640, 480, cmyk)], Some(1)), false),
            (jpeg_headers(&[(0xC2, 8, 640, 480, wide)], None), true),
            (jpeg_headers(&[(0xC2, 8, 640, 480, zero)], None), false),
            (jpeg_headers(&[(0xC6, 8, 640, 480, C444)], None), false),
            (jpeg_headers(&[(0xCA, 8, 640, 480, C444)], None), false),
            (jpeg_headers(&[(0xCE, 8, 640, 480, C444)], None), false),
            (jpeg_headers(&[(0xC0, 8, 64, 64, GREY), (0xC2, 8, 6000, 4000, C444), (0xC2, 8, 100, 100, C444)], Some(1)), false),
            (jpeg_headers(&[(0xC2, 8, 65535, 65535, cmyk)], None), false),
            (jpeg_headers(&[], None), false),
            (vec![0xFF, 0xD8], false),
            (vec![], false),
            (b"not a jpeg at all".to_vec(), false),
        ];
        let mut grew = 0;
        for (b, expect_more) in &cases {
            let (old, new) = (oracle_jpeg_coefficients(b), jpeg_worst_case_coefficients(b));
            assert!(new >= old, "{b:02x?}: {new} < {old}");
            assert_eq!(new > old, *expect_more, "{b:02x?}: {old} -> {new}");
            grew += usize::from(new > old);
        }
        assert_eq!(grew, 3);
        // Every prefix of a frame with a cut header, and pseudo-random damage: never less.
        let full = jpeg_headers(&[(0xC2, 8, 3000, 2000, C420), (0xC0, 8, 100, 100, C444)], Some(1));
        for n in 0..=full.len() {
            assert!(jpeg_worst_case_coefficients(&full[..n]) >= oracle_jpeg_coefficients(&full[..n]), "prefix {n}");
        }
        let mut seed = 0x9e37_79b9u32;
        for _ in 0..3000 {
            let mut f = full.clone();
            for _ in 0..3 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let (i, v) = ((seed >> 8) as usize % f.len(), (seed >> 24) as u8);
                f[i] = v;
            }
            assert!(jpeg_worst_case_coefficients(&f) >= oracle_jpeg_coefficients(&f), "{f:02x?}");
        }
    }

    #[test]
    fn a_baseline_jpeg_with_a_short_first_scan_is_charged_its_coefficients() {
        // 3-component 4:4:4 SOF0 of 8000 x 8000 (64 Mpx) whose first scan has one component: the
        // decoder holds 2 bytes a coefficient of all three until the other scans, 6 bytes a pixel.
        let b = jpeg_headers(&[(0xC0, 8, 8000, 8000, C444)], Some(1));
        let (old, new) = (oracle_jpeg_coefficients(&b), jpeg_worst_case_coefficients(&b));
        assert_eq!((old, new), (0, 8000 * 8000 * 6));
        // 3 + 6 bytes a pixel beside the output passes the 512 MiB budget; it fit as 7 (the RGBA8 conversion).
        let (before, after) = (peak_bytes(8000, 8000, 3, old), peak_bytes(8000, 8000, 3, new));
        assert_eq!((before, after), (Some(8000 * 8000 * 8), Some(8000 * 8000 * 9)));
        assert!(before.is_some_and(|p| p <= crate::MAX_DECODED_BYTES) && after.is_some_and(|p| p > crate::MAX_DECODED_BYTES));
        // The same through `probe`, on a real JPEG whose headers are made so.
        let mut jpeg = encode(image::RgbImage::from_pixel(16, 16, image::Rgb([200, 30, 30])), ImageFormat::Jpeg);
        let sof = jpeg.windows(2).position(|m| m == [0xFF, 0xC0]).unwrap();
        jpeg[sof + 5..sof + 7].copy_from_slice(&8000u16.to_be_bytes());
        jpeg[sof + 7..sof + 9].copy_from_slice(&8000u16.to_be_bytes());
        (0..3).for_each(|c| jpeg[sof + 11 + 3 * c] = 0x11);
        let sos = jpeg.windows(2).position(|m| m == [0xFF, 0xDA]).unwrap();
        jpeg.splice(sos..sos + 14, [0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0]);
        let p = probe(&jpeg).unwrap();
        assert_eq!((p.width, p.height, p.peak_bytes()), (8000, 8000, 8000 * 8000 * 9));
        assert!(!p.fits(crate::MAX_DECODED_BYTES));
    }

    #[test]
    fn jpeg_coefficients_follow_a_marker_walk() {
        // SOI, an APP0 segment, SOF2, then the scan.
        let jpeg = jpeg_headers(&[(0xC2, 8, 64, 64, GREY)], Some(1));
        assert_eq!(jpeg_worst_case_coefficients(&jpeg), 64 * 64 * 2);
        // 4:2:0: MCUs of 16 x 16 holding 4 + 1 + 1 blocks; a 1-pixel-wide picture still takes one.
        assert_eq!(jpeg_worst_case_coefficients(&jpeg_headers(&[(0xC2, 8, 1, 16, C420)], None)), 128 * 6);
    }

    /// A little-endian TIFF header whose first IFD holds just a compression tag.
    fn tiff_with_compression(compression: u16) -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        t.extend(8u32.to_le_bytes());
        t.extend(1u16.to_le_bytes());
        t.extend(259u16.to_le_bytes());
        t.extend(3u16.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        t.extend(u32::from(compression).to_le_bytes());
        t.extend(0u32.to_le_bytes());
        t
    }

    #[test]
    fn tiffs_with_jpeg_strips_are_never_decoded() {
        assert_eq!(tiff_compression(&tiff_with_compression(7)), Some(7));
        assert!(matches!(probe(&tiff_with_compression(7)), Err(Refusal::Unsupported(_))));
        assert!(matches!(probe(&tiff_with_compression(6)), Err(Refusal::Unsupported(_))), "old-style JPEG");
        let mut be = b"MM\0*\0\0\0\x08\0\x01\x01\x03\0\x03\0\0\0\x01\0\x07\0\0".to_vec();
        assert_eq!(tiff_compression(&be), Some(7), "big-endian: the short is left-justified");
        be.truncate(12);
        assert_eq!(tiff_compression(&be), None, "cut short");
        // A second compression tag (the one the decoder keeps) can't hide behind an allowed first.
        let mut twice = b"II*\0".to_vec();
        twice.extend(8u32.to_le_bytes());
        twice.extend(2u16.to_le_bytes());
        for compression in [1u32, 7] {
            twice.extend(259u16.to_le_bytes());
            twice.extend(3u16.to_le_bytes());
            twice.extend(1u32.to_le_bytes());
            twice.extend(compression.to_le_bytes());
        }
        twice.extend(0u32.to_le_bytes());
        assert_eq!(tiff_compression(&twice), None, "duplicate compression tags");
        assert!(matches!(probe(&twice), Err(Refusal::Unsupported(_))));
        // An uncompressed TIFF reads as before.
        let tiff = encode(image::RgbImage::from_pixel(5, 3, image::Rgb([9, 8, 7])), ImageFormat::Tiff);
        assert_eq!(tiff_compression(&tiff), Some(1));
        assert_eq!(decode(&tiff, 1 << 20).unwrap().to_rgb8().get_pixel(4, 2).0, [9, 8, 7]);
    }

    /// A BITMAPV5HEADER DIB of 32-bit pixels, bottom-up, with `alpha_mask`: `BI_RGB`, or with
    /// `bitfields` the usual masks and `BI_BITFIELDS`, their copy after the header when `copy`.
    fn dib(w: i32, h: i32, alpha_mask: u32, bitfields: bool, copy: bool, pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut d = vec![0u8; 124];
        d[0..4].copy_from_slice(&124u32.to_le_bytes());
        d[4..8].copy_from_slice(&w.to_le_bytes());
        d[8..12].copy_from_slice(&h.to_le_bytes());
        d[12..14].copy_from_slice(&1u16.to_le_bytes());
        d[14..16].copy_from_slice(&32u16.to_le_bytes());
        d[52..56].copy_from_slice(&alpha_mask.to_le_bytes());
        if bitfields {
            d[16..20].copy_from_slice(&3u32.to_le_bytes());
            for (at, m) in [(40, 0x00FF_0000u32), (44, 0x0000_FF00), (48, 0x0000_00FF)] {
                d[at..at + 4].copy_from_slice(&m.to_le_bytes());
            }
            if copy {
                let masks = d[40..52].to_vec();
                d.extend(masks);
            }
        }
        for p in pixels {
            d.extend(p);
        }
        d
    }

    #[test]
    fn clipboard_dibs_keep_their_alpha_in_every_layout() {
        // BGRA in memory: a half-transparent red pixel and an opaque blue one.
        let px = [[0, 0, 255, 128], [255, 0, 0, 255]];
        // `BI_RGB` with an alpha mask (Windows programs); bit fields without the masks' copy
        // after the header (arboard), and with it (Windows).
        for (bitfields, copy) in [(false, false), (true, false), (true, true)] {
            let p = probe_dib(dib(2, 1, 0xFF00_0000, bitfields, copy, &px)).unwrap();
            assert_eq!((p.format, p.width, p.height), (ImageFormat::Bmp, 2, 1));
            let rgba = p.decode(1 << 20).unwrap().to_rgba8();
            assert_eq!(rgba.get_pixel(0, 0).0, [255, 0, 0, 128], "{bitfields} {copy}");
            assert_eq!(rgba.get_pixel(1, 0).0, [0, 0, 255, 255], "{bitfields} {copy}");
        }
        // Without an alpha mask, `BI_RGB` pixels are opaque.
        let p = probe_dib(dib(2, 1, 0, false, false, &px)).unwrap();
        assert_eq!(p.decode(1 << 20).unwrap().to_rgba8().get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn huge_or_damaged_clipboard_dibs_are_refused() {
        let p = probe_dib(dib(60_000, 60_000, 0xFF00_0000, false, false, &[])).unwrap();
        assert_eq!((p.width, p.height), (60_000, 60_000));
        assert!(!p.fits(crate::MAX_DECODED_BYTES));
        assert!(matches!(p.decode(crate::MAX_DECODED_BYTES), Err(Refusal::TooLarge { .. })));
        assert!(probe_dib(vec![0; 8]).is_err());
        // Cut short or lying about its header: no pixels come out, and nothing panics.
        let mut short = dib(2, 1, 0xFF00_0000, true, false, &[]);
        short.truncate(100);
        let mut lying = dib(2, 1, 0xFF00_0000, true, false, &[[0; 4], [0; 4]]);
        lying[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        for d in [short, lying] {
            assert!(probe_dib(d).and_then(|p| p.decode(1 << 20)).is_err());
        }
    }

    // The local JPEG budget before it moved to `craft_image_codecs::jpeg_worst_case_coefficients` (#778), kept as the oracle.
    /// Bytes of DCT coefficients a progressive JPEG's decoder keeps for the whole picture while it
    /// reads the scans: zune-jpeg holds an `i16` per coefficient of every component, in whole MCUs.
    /// 0 for a baseline JPEG, decoded an MCU row at a time. Each frame header before the first scan
    /// counts and the largest wins; headers this can't follow are charged as the costliest frame.
    fn oracle_jpeg_coefficients(bytes: &[u8]) -> u64 {
        let mut at = 2;
        let mut most = 0u64;
        loop {
            if bytes.get(at) != Some(&0xFF) {
                return oracle_worst_coefficients(bytes).max(most);
            }
            while bytes.get(at) == Some(&0xFF) {
                at += 1;
            }
            let Some(&code) = bytes.get(at) else { return oracle_worst_coefficients(bytes).max(most) };
            at += 1;
            match code {
                // Markers without a length: start of image, TEM, restarts.
                0xD8 | 0x01 | 0xD0..=0xD7 => continue,
                // End of image, start of scan: every frame header is behind.
                0xD9 | 0xDA => return most,
                _ => {}
            }
            let len = match (bytes.get(at), bytes.get(at + 1)) {
                (Some(&hi), Some(&lo)) => usize::from(u16::from_be_bytes([hi, lo])),
                _ => 0,
            };
            let Some(segment) = bytes.get(at + 2..at + len).filter(|_| len >= 2) else { return oracle_worst_coefficients(bytes).max(most) };
            // SOF2, SOF6, SOF10 and SOF14 are the progressive frames.
            if matches!(code, 0xC2 | 0xC6 | 0xCA | 0xCE) {
                let Some(held) = oracle_frame_coefficients(segment) else { return oracle_worst_coefficients(bytes).max(most) };
                most = most.max(held);
            }
            at += len;
        }
    }

    /// `oracle_jpeg_coefficients` of one progressive frame header (after its length): the components'
    /// blocks per MCU (horizontal × vertical sampling) times the MCUs that cover the picture.
    fn oracle_frame_coefficients(sof: &[u8]) -> Option<u64> {
        let height = u64::from(u16::from_be_bytes([*sof.get(1)?, *sof.get(2)?]));
        let width = u64::from(u16::from_be_bytes([*sof.get(3)?, *sof.get(4)?]));
        let n = usize::from(*sof.get(5)?);
        let sampling: Vec<(u64, u64)> = sof
            .get(6..6 + 3 * n)?
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[_, s, _]| (u64::from(s >> 4).clamp(1, 4), u64::from(s & 15).clamp(1, 4)))
            .collect();
        let (h_max, v_max) = sampling.iter().fold((1, 1), |(h, v), &(a, b)| (h.max(a), v.max(b)));
        let mcus = width.div_ceil(8 * h_max).checked_mul(height.div_ceil(8 * v_max))?;
        sampling.iter().try_fold(0u64, |sum, &(h, v)| sum.checked_add(mcus.checked_mul(2 * 64 * h * v)?))
    }

    /// The coefficients of the costliest progressive frame of the size the image crate reads from
    /// `bytes`: four components, each sampled 4 × 4, in MCUs of 32 pixels a side.
    fn oracle_worst_coefficients(bytes: &[u8]) -> u64 {
        let Some((w, h)) = dimensions(bytes) else { return 0 };
        let mcus = u64::from(w).div_ceil(32) * u64::from(h).div_ceil(32);
        mcus.saturating_mul(2 * 64 * 16 * 4)
    }

    // The local DIB wrapping before it moved to `craft_image_codecs::bmp_probe` (#778), kept as the oracle.
    /// Prepend a 14-byte `BITMAPFILEHEADER` to a normalized DIB so `craft-image-codecs` can inspect and decode it as BMP.
    fn oracle_dib_to_bmp(dib: &[u8]) -> Vec<u8> {
        let u16_at = |at: usize| dib.get(at..at + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
        let u32_at = |at: usize| dib.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
        let bi_size = u32_at(0).unwrap_or(40);
        let offset = if bi_size == 12 {
            let bpp = u16_at(10).unwrap_or(24);
            let palette = if bpp <= 8 { (1u32 << bpp) * 3 } else { 0 };
            14 + bi_size + palette
        } else {
            let bpp = u16_at(14).unwrap_or(24);
            let compression = u32_at(16).unwrap_or(0);
            let clr_used = u32_at(32).unwrap_or(0);
            let palette_entries = if clr_used != 0 {
                clr_used
            } else if bpp <= 8 {
                1u32 << bpp
            } else {
                0
            };
            let masks = if compression == 3 && bi_size >= 40 { 12 } else { 0 };
            14u32.saturating_add(bi_size).saturating_add(masks).saturating_add(palette_entries.saturating_mul(4))
        };
        let file_size = u32::try_from(14 + dib.len()).unwrap_or(u32::MAX);
        let mut bmp = Vec::with_capacity(14 + dib.len());
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&file_size.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 4]);
        bmp.extend_from_slice(&offset.to_le_bytes());
        bmp.extend_from_slice(dib);
        bmp
    }

    /// A DIB with a V4 or V5 header in the layout the decoder reads. A 32-bit one whose alpha mask
    /// is set but whose compression says `BI_RGB` is marked `BI_BITFIELDS` (with the usual colour
    /// masks when it gives none), so its alpha is kept as Windows programs mean it; arboard makes the
    /// same change. Bit fields then need the colour masks after the header too, where Windows puts a
    /// copy and the decoder skips them; a DIB without that copy (arboard writes one) gets it.
    fn oracle_dib_layout(mut dib: Vec<u8>) -> Vec<u8> {
        let u32_at = |d: &[u8], at: usize| d.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
        let Some(size) = u32_at(&dib, 0).filter(|&s| s >= 108).and_then(|s| usize::try_from(s).ok()) else { return dib };
        let put = |d: &mut Vec<u8>, at: usize, v: u32| {
            if let Some(b) = d.get_mut(at..at + 4) {
                b.copy_from_slice(&v.to_le_bytes());
            }
        };
        if dib.get(14..16) == Some(&[32, 0]) && u32_at(&dib, 16) == Some(0) && u32_at(&dib, 52) == Some(0xFF00_0000) {
            put(&mut dib, 16, 3);
            if (40..52).step_by(4).all(|at| u32_at(&dib, at) == Some(0)) {
                put(&mut dib, 40, 0x00FF_0000);
                put(&mut dib, 44, 0x0000_FF00);
                put(&mut dib, 48, 0x0000_00FF);
            }
        }
        if u32_at(&dib, 16) == Some(3)
            && size <= dib.len()
            && let Some(masks) = dib.get(40..52).map(<[u8]>::to_vec)
            && dib.get(size..size + 12) != Some(masks.as_slice())
        {
            dib.splice(size..size, masks);
        }
        dib
    }

    /// The size and peak `probe_dib` reported before the cutover, as `(width, height, peak, alpha, bytes)`.
    fn oracle_probe_dib(dib: Vec<u8>) -> Result<(u32, u32, u64, bool, Vec<u8>), Refusal> {
        let dib = oracle_dib_layout(dib);
        let bmp = oracle_dib_to_bmp(&dib);
        let (width, height, native, alpha) = match inspect_raster(&bmp) {
            Ok(Some(info)) => {
                let native = u64::from(info.channels.max(1)) * u64::from(info.bits_per_sample.max(8) / 8);
                (info.width, info.height, native, matches!(info.color_type, RasterColorType::GrayAlpha | RasterColorType::Rgba))
            }
            Ok(None) => return Err(Refusal::Broken("its DIB header can't be read".into())),
            Err(CodecError::LimitExceeded(_)) => {
                let (w, h) = dib_header_dims(&dib).ok_or_else(|| Refusal::Broken("its DIB header can't be read".into()))?;
                (w, h, 4, false)
            }
            Err(e) => return Err(broken(e)),
        };
        let (width, height, peak) = sized(width, height, native, 0)?;
        Ok((width, height, peak, alpha, bmp))
    }

    /// A DIB with a header of `size` bytes (12, 40, 52, 56, 64, 108 or 124), `bpp`, `compression`
    /// and `clr_used`, followed by `tail` bytes.
    fn dib_with(size: u32, bpp: u16, compression: u32, clr_used: u32, alpha_mask: u32, tail: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; usize::try_from(size).unwrap()];
        d[0..4].copy_from_slice(&size.to_le_bytes());
        if size == 12 {
            d[4..6].copy_from_slice(&3u16.to_le_bytes());
            d[6..8].copy_from_slice(&2u16.to_le_bytes());
            d[8..10].copy_from_slice(&1u16.to_le_bytes());
            d[10..12].copy_from_slice(&bpp.to_le_bytes());
        } else {
            d[4..8].copy_from_slice(&3i32.to_le_bytes());
            d[8..12].copy_from_slice(&2i32.to_le_bytes());
            d[12..14].copy_from_slice(&1u16.to_le_bytes());
            d[14..16].copy_from_slice(&bpp.to_le_bytes());
            d[16..20].copy_from_slice(&compression.to_le_bytes());
            d[32..36].copy_from_slice(&clr_used.to_le_bytes());
            if size >= 56 {
                d[52..56].copy_from_slice(&alpha_mask.to_le_bytes());
            }
            if size >= 52 {
                for (at, m) in [(40, 0x00FF_0000u32), (44, 0x0000_FF00), (48, 0x0000_00FF)] {
                    d[at..at + 4].copy_from_slice(&m.to_le_bytes());
                }
            }
        }
        d.extend_from_slice(tail);
        d
    }

    fn crafted_dibs() -> Vec<Vec<u8>> {
        let px = [[0u8, 0, 255, 128]; 6].concat();
        let masks = [0x00u8, 0, 0xFF, 0, 0, 0xFF, 0, 0, 0xFF, 0, 0, 0];
        let mut v = Vec::new();
        for size in [12u32, 40, 52, 56, 64, 108, 124] {
            for bpp in [1u16, 4, 8, 16, 24, 32] {
                for compression in [0u32, 1, 2, 3, 6] {
                    for clr_used in [0u32, 2, 300, u32::MAX] {
                        for alpha_mask in [0u32, 0xFF00_0000] {
                            for tail in [&[][..], &px[..], &masks[..], &[&masks[..], &px[..]].concat()[..]] {
                                v.push(dib_with(size, bpp, compression, clr_used, alpha_mask, tail));
                            }
                        }
                    }
                }
            }
        }
        // Clipboard layouts: the V5 helper's three, and a lying header size.
        let q = [[0, 0, 255, 128], [255, 0, 0, 255]];
        for (bitfields, copy) in [(false, false), (true, false), (true, true)] {
            v.push(dib(2, 1, 0xFF00_0000, bitfields, copy, &q));
            v.push(dib(2, 1, 0, bitfields, copy, &q));
        }
        v.push(dib(60_000, 60_000, 0xFF00_0000, false, false, &[]));
        let mut lying = dib(2, 1, 0xFF00_0000, true, false, &q);
        lying[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        v.push(lying);
        v
    }

    // craft-image-codecs fixed two DIB behaviours after the cutover (craft-libs #2062); the oracle
    // keeps the old ones, so these predicates name the inputs where the two must differ.

    fn le32(d: &[u8], at: usize) -> Option<u32> {
        d.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)
    }

    /// A 52- or 56-byte `BI_BITFIELDS` header: its masks are inside the header, so the pixel offset
    /// no longer counts 12 appended mask bytes. Returns the new, independently computed offset.
    fn v2v3_bitfields_offset(d: &[u8]) -> Option<u32> {
        let size = le32(d, 0).unwrap_or(40);
        if !matches!(size, 52 | 56) || le32(d, 16).unwrap_or(0) != 3 {
            return None;
        }
        let bpp = d.get(14..16).and_then(|b| b.try_into().ok()).map_or(24, u16::from_le_bytes);
        let clr_used = le32(d, 32).unwrap_or(0);
        let entries = if clr_used != 0 {
            clr_used
        } else if bpp <= 8 {
            1u32 << bpp
        } else {
            0
        };
        Some(14u32.saturating_add(size).saturating_add(entries.saturating_mul(4)))
    }

    /// A V4/V5 bit-fields DIB whose 12 bytes after the header equal the masks but whose payload is
    /// no longer than the pixel rows need: those bytes are pixels, not a copy, so a copy is inserted.
    /// Returns the masks the shared code inserts at `size`.
    fn exact_payload_equal_to_masks(d: &[u8]) -> Option<(usize, [u8; 12])> {
        let size = usize::try_from(le32(d, 0).filter(|&s| s >= 108)?).ok()?;
        if size > d.len() {
            return None;
        }
        let promote = d.get(14..16) == Some(&[32, 0][..]) && le32(d, 16) == Some(0) && le32(d, 52) == Some(0xFF00_0000);
        let mut masks: [u8; 12] = d.get(40..52)?.try_into().ok()?;
        if promote && masks == [0; 12] {
            masks = [0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00];
        }
        if !(promote || le32(d, 16) == Some(3)) || d.get(size..size + 12) != Some(masks.as_slice()) {
            return None;
        }
        let w = u64::try_from(i32::from_le_bytes(d.get(4..8)?.try_into().ok()?)).ok()?;
        let h = u64::from(i32::from_le_bytes(d.get(8..12)?.try_into().ok()?).unsigned_abs());
        let bpp = u64::from(u16::from_le_bytes(d.get(14..16)?.try_into().ok()?));
        let need = usize::try_from(w.checked_mul(bpp)?.div_ceil(32).checked_mul(4)?.checked_mul(h)?).ok()?;
        (d.len() - size <= need).then_some((size, masks))
    }

    #[test]
    fn dib_wrapping_is_byte_equal_to_the_local_one_it_replaced() {
        let (mut n, mut v2v3, mut exact) = (0, 0, 0);
        for d in crafted_dibs() {
            // Every truncation point of the header region, then the whole.
            let cuts = (0..=d.len().min(140)).chain([d.len()]);
            for cut in cuts {
                let d = &d[..cut];
                n += 1;
                let (new, old) = (dib_to_bmp(d).unwrap(), oracle_dib_to_bmp(d));
                if let Some(offset) = v2v3_bitfields_offset(d) {
                    // New behaviour: only the offset differs, and it no longer counts the 12 mask bytes.
                    v2v3 += 1;
                    let mut expected = old;
                    expected[10..14].copy_from_slice(&offset.to_le_bytes());
                    assert_eq!(new, expected, "wrap, 52/56-byte bit fields, cut {cut}");
                } else {
                    assert_eq!(new, old, "wrap, cut {cut}");
                }
                let (new, old) = (bmp_probe::dib_layout(d.to_vec()), oracle_dib_layout(d.to_vec()));
                if let Some((size, masks)) = exact_payload_equal_to_masks(d) {
                    // New behaviour: the bytes after the header are pixels, so a mask copy is inserted.
                    exact += 1;
                    let mut expected = old;
                    expected.splice(size..size, masks);
                    assert_eq!(new, expected, "layout, exact payload equal to the masks, cut {cut}");
                } else {
                    assert_eq!(new, old, "layout, cut {cut}");
                }
            }
        }
        assert!(n > 100_000, "{n}");
        // The exclusions are small and present: a predicate that matched nothing would hide a regression.
        eprintln!("DIB corpus: {n} cases, {v2v3} excluded as 52/56-byte bit fields, {exact} as exact payload equal to the masks");
        assert!(v2v3 > 0 && exact > 0 && v2v3 + exact < n / 10, "{v2v3} {exact}");
    }

    #[test]
    fn probe_dib_is_unchanged_by_the_shared_wrapping() {
        for d in crafted_dibs() {
            for cut in [0, 4, 11, 12, 14, 40, 52, 100, 124, 130, d.len()] {
                let d = d[..cut.min(d.len())].to_vec();
                // The two DIB fixes change the bytes handed to the decoder; the wrapping test above pins
                // them, so skip those inputs here.
                if v2v3_bitfields_offset(&d).is_some() || exact_payload_equal_to_masks(&d).is_some() {
                    continue;
                }
                let new = probe_dib(d.clone());
                match (new, oracle_probe_dib(d.clone())) {
                    (Ok(p), Ok((w, h, peak, alpha, bmp))) => {
                        assert_eq!((p.width, p.height, p.peak, p.alpha), (w, h, peak, alpha), "{d:02x?}");
                        assert_eq!(&*p.bytes, bmp.as_slice());
                    }
                    (Err(a), Err(b)) => assert_eq!(a, b, "{d:02x?}"),
                    (a, b) => panic!("{d:02x?}: {:?} vs {:?}", a.map(|p| p.width), b.map(|o| o.0)),
                }
            }
        }
    }

    // The local PNG header read before it moved to `craft_image_codecs::png_probe` (#778), kept as the oracle.
    /// A PNG's width, height, the bytes a pixel its decoder gives (samples under 8 bits widened to 8,
    /// a palette to RGB, `tRNS` to an alpha channel) and whether it has transparency, from `IHDR` and
    /// the chunk headers before the image data. The decoder's own header read inflates an ICC profile
    /// (`iCCP`) up to its 512 MiB default limit, for a picture that may only be measured.
    fn oracle_png_chunks(bytes: &[u8]) -> Option<(u32, u32, u64, bool)> {
        let be = |at: usize| bytes.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_be_bytes);
        // The 8-byte signature, then IHDR: length, type, width, height, bit depth, colour type.
        if bytes.get(12..16) != Some(b"IHDR".as_slice()) {
            return None;
        }
        let (width, height) = (be(16)?, be(20)?);
        let (depth, color) = (*bytes.get(24)?, *bytes.get(25)?);
        let mut trns = false;
        let mut at = 8usize;
        loop {
            let len = usize::try_from(be(at)?).ok()?;
            match bytes.get(at.checked_add(4)?..at.checked_add(8)?)? {
                b"IDAT" => break,
                b"IEND" => return None,
                b"tRNS" => trns = true,
                _ => {}
            }
            at = at.checked_add(12)?.checked_add(len)?;
        }
        let (channels, alpha) = match color {
            0 => (1 + u64::from(trns), trns),
            2 | 3 => (3 + u64::from(trns), trns),
            4 => (2, true),
            6 => (4, true),
            _ => return None,
        };
        Some((width, height, channels * if depth == 16 { 2 } else { 1 }, alpha))
    }

    /// WordCraft's WebP alpha rule before the cutover: the `VP8X` flag, nothing else.
    fn oracle_webp_alpha(bytes: &[u8]) -> bool {
        bytes.get(12..16) == Some(b"VP8X") && bytes.get(20).is_some_and(|b| b & 0x10 != 0)
    }

    fn new_png(b: &[u8]) -> Option<(u32, u32, u64, bool)> {
        png_probe::pixel_model(b).ok().map(|m| (m.width, m.height, u64::from(m.bytes_per_pixel()), m.has_alpha()))
    }

    /// A PNG with the given IHDR fields and chunks before a (junk) `IDAT`.
    fn png_with(width: u32, height: u32, depth: u8, color: u8, before_idat: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
            out.extend((data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0; 4]);
        };
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = [width.to_be_bytes(), height.to_be_bytes()].concat();
        ihdr.extend_from_slice(&[depth, color, 0, 0, 0]);
        chunk(&mut b, b"IHDR", &ihdr);
        for (k, d) in before_idat {
            chunk(&mut b, k, d);
        }
        chunk(&mut b, b"IDAT", &[1, 2, 3]);
        chunk(&mut b, b"IEND", &[]);
        b
    }

    #[test]
    fn png_pixel_model_matches_the_local_header_read_for_every_valid_header() {
        let valid: [(u8, &[u8]); 5] = [(0, &[1, 2, 4, 8, 16]), (2, &[8, 16]), (3, &[1, 2, 4, 8]), (4, &[8, 16]), (6, &[8, 16])];
        let mut n = 0;
        for (color, depths) in valid {
            for &depth in depths {
                for (trns, icc) in [(false, false), (true, false), (true, true), (false, true)] {
                    let mut extra: Vec<(&[u8; 4], Vec<u8>)> = vec![];
                    if icc {
                        extra.push((b"iCCP", vec![0xAB; 300]));
                    }
                    if trns {
                        extra.push((b"tRNS", vec![0, 0]));
                    }
                    let b = png_with(5, 7, depth, color, &extra);
                    assert!(new_png(&b).is_some(), "colour {color} depth {depth}");
                    assert_eq!(new_png(&b), oracle_png_chunks(&b), "colour {color} depth {depth} trns {trns}");
                    // And `probe` reports what the old read gave it: the size and the peak.
                    let (w, h, native, alpha) = oracle_png_chunks(&b).unwrap();
                    let p = probe(&b).unwrap();
                    assert_eq!((p.width, p.height, p.has_alpha(), Some(p.peak_bytes())), (w, h, alpha, peak_bytes(w, h, native, 0)));
                    assert_eq!(dimensions(&b), Some((5, 7)));
                    n += 1;
                }
            }
        }
        assert_eq!(n, 4 * 15);
        // Real encoder output, every layout the image types give.
        for img in [
            DynamicImage::ImageLuma8(image::GrayImage::new(3, 2)),
            DynamicImage::ImageLumaA8(image::GrayAlphaImage::new(3, 2)),
            DynamicImage::ImageRgb8(image::RgbImage::new(3, 2)),
            DynamicImage::ImageRgba8(image::RgbaImage::new(3, 2)),
            DynamicImage::ImageRgb16(image::ImageBuffer::new(3, 2)),
            DynamicImage::ImageRgba16(image::ImageBuffer::new(3, 2)),
        ] {
            let b = encode(img, ImageFormat::Png);
            assert_eq!(new_png(&b), oracle_png_chunks(&b));
        }
    }

    /// What the old read accepted and `pixel_model` refuses is a header the decoder can't use
    /// either; `probe` then says the PNG header can't be read (`Broken`) and `dimensions` is None.
    #[test]
    fn png_pixel_model_is_stricter_only_on_headers_that_are_not_valid_pngs() {
        let ok = png_with(5, 7, 8, 2, &[]);
        let mut cases: Vec<(&str, Vec<u8>)> = vec![
            ("depth 3 for RGB", png_with(5, 7, 3, 2, &[])),
            ("depth 16 for palette", png_with(5, 7, 16, 3, &[])),
            ("depth 4 for RGBA", png_with(5, 7, 4, 6, &[])),
            ("zero width", png_with(0, 7, 8, 2, &[])),
            ("zero height", png_with(5, 0, 8, 2, &[])),
            ("width past i32::MAX", png_with(1 << 31, 1, 8, 2, &[])),
            ("height past i32::MAX", png_with(1, u32::MAX, 8, 2, &[])),
        ];
        let many: Vec<(&[u8; 4], Vec<u8>)> = (0..70_000).map(|_| (b"tEXt", Vec::new())).collect();
        cases.push(("70,000 chunks before the image data", png_with(5, 7, 8, 2, &many)));
        for (what, b) in &cases {
            assert!(oracle_png_chunks(b).is_some(), "{what}: the old read accepted it");
            assert_eq!(new_png(b), None, "{what}");
            assert_eq!(dimensions(b), None, "{what}");
            assert!(matches!(probe(b), Err(Refusal::Broken(_))), "{what}");
            assert!(!header_has_alpha(b), "{what}");
        }
        // Everything else the old read refused (no IHDR, unknown colour, a wrong IHDR length that
        // throws its chunk walk off, no image data) is still refused.
        let mut short_ihdr = ok.clone();
        short_ihdr[8..12].copy_from_slice(&12u32.to_be_bytes());
        for b in [png_with(5, 7, 8, 5, &[]), b"\x89PNG\r\n\x1a\n".to_vec(), ok[..33].to_vec(), short_ihdr] {
            assert_eq!(new_png(&b), None);
            assert_eq!(oracle_png_chunks(&b), None);
        }
        // Truncations and damage: where the new read answers, it answers as the old one did.
        let full = png_with(5, 7, 8, 6, &[(b"tRNS", vec![0]), (b"gAMA", vec![0; 4])]);
        for n in 0..=full.len() {
            if new_png(&full[..n]).is_some() {
                assert_eq!(new_png(&full[..n]), oracle_png_chunks(&full[..n]), "prefix {n}");
            }
        }
        let mut seed = 0x9e37_79b9u32;
        for _ in 0..3000 {
            let mut f = full.clone();
            for _ in 0..3 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let (i, v) = ((seed >> 8) as usize % f.len(), (seed >> 24) as u8);
                f[i] = v;
            }
            if new_png(&f).is_some() {
                assert_eq!(new_png(&f), oracle_png_chunks(&f), "{f:02x?}");
            }
        }
    }

    /// A RIFF WebP with one chunk.
    fn riff_webp(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend((4 + 8 + payload.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WEBP");
        b.extend_from_slice(kind);
        b.extend((payload.len() as u32).to_le_bytes());
        b.extend_from_slice(payload);
        b
    }

    #[test]
    fn webp_alpha_follows_vp8x_and_now_vp8l() {
        // VP8X: the flag, as before.
        for flags in [0u8, 0x10, 0x20, 0x30] {
            let mut payload = vec![flags, 0, 0, 0];
            payload.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // 1 x 1 canvas
            let b = riff_webp(b"VP8X", &payload);
            assert_eq!(webp_probe::has_alpha(&b).unwrap(), oracle_webp_alpha(&b), "flags {flags:#x}");
            assert_eq!(oracle_webp_alpha(&b), flags & 0x10 != 0);
        }
        // A bare lossless picture (VP8L) says so in its header: 1 x 1, `alpha_is_used` (bit 28).
        let with = riff_webp(b"VP8L", &[0x2f, 0, 0, 0, 0x10, 0, 0, 0]);
        let without = riff_webp(b"VP8L", &[0x2f, 0, 0, 0, 0x00, 0, 0, 0]);
        assert!(!oracle_webp_alpha(&with), "the old rule never saw it");
        assert!(webp_probe::has_alpha(&with).unwrap());
        assert!(!webp_probe::has_alpha(&without).unwrap());
        // A real encoder's lossless RGBA WebP.
        let enc = encode(image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 8, 7, 100])), ImageFormat::WebP);
        assert_eq!(probe(&enc).unwrap().has_alpha(), webp_probe::has_alpha(&enc).unwrap());
        // Lossy VP8 never has alpha; and input that isn't a RIFF WebP reads as none, as before.
        assert!(!webp_probe::has_alpha(&riff_webp(b"VP8 ", &[0; 10])).unwrap());
        for junk in [&b"not a webp at all"[..], b"", b"RIFF", b"RIFF\0\0\0\0WEBPVP8L"] {
            assert!(!webp_probe::has_alpha(junk).unwrap_or(false));
        }
    }
}
