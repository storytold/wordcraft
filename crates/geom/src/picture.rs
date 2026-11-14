//! Decoding a picture within a memory budget. Every crate that decodes a document's or the
//! clipboard's pictures goes through [`probe`] (or [`probe_dib`]): the header gives the format and
//! size, and the decode is refused before anything is allocated when its peak would pass the
//! budget. The peak counts what a decoder holds beside its output (a progressive JPEG's
//! coefficients, #3644), and a picture whose decoder can't be held to a budget is not decoded at
//! all (a TIFF with JPEG strips, #3645).

use std::borrow::Cow;

use craft_image_codecs::{
    ChannelLayout, CodecError, DecodeOptions, EncodeOptions, Format as CodecFormat, Image as CodecImage, Limits as CodecLimits, RasterColorType,
    SampleType, decode_as_with, detect, encode as codec_encode, inspect_raster, probe_size,
};
use image::{DynamicImage, ImageBuffer, ImageFormat};

/// Why a picture isn't decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a raster picture WordCraft decodes (a metafile, SVG or other bytes).
    NotAPicture,
    /// A kind of picture that is never decoded, because its decoder can't be held to a budget.
    Unsupported(&'static str),
    /// Decoding it would take more memory than the budget.
    TooLarge { width: u32, height: u32 },
    /// The header or the data is damaged.
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
        CodecFormat::Png => png_chunks(bytes).map(|(w, h, _, _)| (w, h)),
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
/// codes) and PackBits. The `tiff` crate decodes a JPEG strip (7) with a decoder of its own and
/// default limits, whatever size the TIFF declares, so such a TIFF is not decoded (#3645).
const TIFF_COMPRESSIONS: [u16; 6] = [1, 4, 5, 8, 32946, 32773];

/// Read a picture's header: PNG, JPEG, GIF, WebP, BMP or TIFF.
pub fn probe(bytes: &[u8]) -> Result<Probe<'_>, Refusal> {
    let codec_fmt = detect(bytes).ok_or(Refusal::NotAPicture)?;
    let format = from_codec_format(codec_fmt).ok_or(Refusal::NotAPicture)?;
    let held = match format {
        ImageFormat::Png | ImageFormat::Gif | ImageFormat::WebP | ImageFormat::Bmp => 0,
        ImageFormat::Jpeg => jpeg_coefficients(bytes),
        ImageFormat::Tiff => {
            if !tiff_compression(bytes).is_some_and(|c| TIFF_COMPRESSIONS.contains(&c)) {
                return Err(Refusal::Unsupported("TIFF pictures with JPEG or an unknown compression can't be read"));
            }
            0
        }
        _ => return Err(Refusal::NotAPicture),
    };
    if format == ImageFormat::Png {
        let (width, height, native, alpha) = png_chunks(bytes).ok_or_else(|| Refusal::Broken("its PNG header can't be read".into()))?;
        let (width, height, peak) = sized(width, height, native, 0)?;
        return Ok(Probe { bytes: Cow::Borrowed(bytes), format, width, height, peak, alpha });
    }
    if format == ImageFormat::WebP {
        let (width, height) = probe_size(bytes).map_err(broken)?;
        let (width, height, peak) = sized(width, height, 4, 0)?;
        let alpha = bytes.get(12..16) == Some(b"VP8X") && bytes.get(20).is_some_and(|b| b & 0x10 != 0);
        return Ok(Probe { bytes: Cow::Borrowed(bytes), format, width, height, peak, alpha });
    }
    let info = inspect_raster(bytes).map_err(broken)?.ok_or_else(|| Refusal::Broken("its header can't be read".into()))?;
    let native = u64::from(info.channels.max(1)) * u64::from(info.bits_per_sample.max(8) / 8);
    let (width, height, peak) = sized(info.width, info.height, native, held)?;
    let alpha = matches!(info.color_type, RasterColorType::GrayAlpha | RasterColorType::Rgba | RasterColorType::CmykAlpha)
        || info.transparent_index.is_some();
    Ok(Probe { bytes: Cow::Borrowed(bytes), format, width, height, peak, alpha })
}

/// A PNG's width, height, the bytes a pixel its decoder gives (samples under 8 bits widened to 8,
/// a palette to RGB, `tRNS` to an alpha channel) and whether it has transparency, from `IHDR` and
/// the chunk headers before the image data. The decoder's own header read inflates an ICC profile
/// (`iCCP`) up to its 512 MiB default limit, for a picture that may only be measured.
fn png_chunks(bytes: &[u8]) -> Option<(u32, u32, u64, bool)> {
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

/// Prepend a 14-byte `BITMAPFILEHEADER` to a normalized DIB so `craft-image-codecs` can inspect and decode it as BMP.
fn dib_to_bmp(dib: &[u8]) -> Vec<u8> {
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

/// Read a DIB's header: a BMP without its 14-byte file header, as the Windows clipboard holds
/// one (`CF_DIB`, `CF_DIBV5`). Its layout is first made the one the decoder reads (see
/// [`dib_layout`]).
pub fn probe_dib(dib: Vec<u8>) -> Result<Probe<'static>, Refusal> {
    let dib = dib_layout(dib);
    let bmp = dib_to_bmp(&dib);
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

fn dib_header_dims(dib: &[u8]) -> Option<(u32, u32)> {
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

/// Bytes of DCT coefficients a progressive JPEG's decoder keeps for the whole picture while it
/// reads the scans: zune-jpeg holds an `i16` per coefficient of every component, in whole MCUs.
/// 0 for a baseline JPEG, decoded an MCU row at a time. Each frame header before the first scan
/// counts and the largest wins; headers this can't follow are charged as the costliest frame.
fn jpeg_coefficients(bytes: &[u8]) -> u64 {
    let mut at = 2;
    let mut most = 0u64;
    loop {
        if bytes.get(at) != Some(&0xFF) {
            return worst_coefficients(bytes).max(most);
        }
        while bytes.get(at) == Some(&0xFF) {
            at += 1;
        }
        let Some(&code) = bytes.get(at) else { return worst_coefficients(bytes).max(most) };
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
        let Some(segment) = bytes.get(at + 2..at + len).filter(|_| len >= 2) else { return worst_coefficients(bytes).max(most) };
        // SOF2, SOF6, SOF10 and SOF14 are the progressive frames.
        if matches!(code, 0xC2 | 0xC6 | 0xCA | 0xCE) {
            let Some(held) = frame_coefficients(segment) else { return worst_coefficients(bytes).max(most) };
            most = most.max(held);
        }
        at += len;
    }
}

/// [`jpeg_coefficients`] of one progressive frame header (after its length): the components'
/// blocks per MCU (horizontal × vertical sampling) times the MCUs that cover the picture.
fn frame_coefficients(sof: &[u8]) -> Option<u64> {
    let height = u64::from(u16::from_be_bytes([*sof.get(1)?, *sof.get(2)?]));
    let width = u64::from(u16::from_be_bytes([*sof.get(3)?, *sof.get(4)?]));
    let n = usize::from(*sof.get(5)?);
    let sampling: Vec<(u64, u64)> =
        sof.get(6..6 + 3 * n)?.as_chunks::<3>().0.iter().map(|&[_, s, _]| (u64::from(s >> 4).clamp(1, 4), u64::from(s & 15).clamp(1, 4))).collect();
    let (h_max, v_max) = sampling.iter().fold((1, 1), |(h, v), &(a, b)| (h.max(a), v.max(b)));
    let mcus = width.div_ceil(8 * h_max).checked_mul(height.div_ceil(8 * v_max))?;
    sampling.iter().try_fold(0u64, |sum, &(h, v)| sum.checked_add(mcus.checked_mul(2 * 64 * h * v)?))
}

/// The coefficients of the costliest progressive frame of the size the image crate reads from
/// `bytes`: four components, each sampled 4 × 4, in MCUs of 32 pixels a side.
fn worst_coefficients(bytes: &[u8]) -> u64 {
    let Some((w, h)) = dimensions(bytes) else { return 0 };
    let mcus = u64::from(w).div_ceil(32) * u64::from(h).div_ceil(32);
    mcus.saturating_mul(2 * 64 * 16 * 4)
}

/// The compression of a classic TIFF's first image (the one the image crate decodes): 1 when the
/// tag is missing. None when the header can't be read, or when the tag appears more than once:
/// the decoder keeps the last copy, so a harmless first one must not hide a JPEG one after it.
fn tiff_compression(bytes: &[u8]) -> Option<u16> {
    let le = match bytes.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = [*bytes.get(at)?, *bytes.get(at.checked_add(1)?)?];
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let mut found = None;
    for i in 0..usize::from(u16_at(ifd)?) {
        let entry = ifd.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        if u16_at(entry)? != 259 {
            continue;
        }
        if found.is_some() || u32_at(entry.checked_add(4)?)? != 1 {
            return None;
        }
        found = Some(match u16_at(entry.checked_add(2)?)? {
            3 => u16_at(entry.checked_add(8)?)?,
            4 => u16::try_from(u32_at(entry.checked_add(8)?)?).ok()?,
            _ => return None,
        });
    }
    Some(found.unwrap_or(1))
}

/// A DIB with a V4 or V5 header in the layout the decoder reads. A 32-bit one whose alpha mask
/// is set but whose compression says `BI_RGB` is marked `BI_BITFIELDS` (with the usual colour
/// masks when it gives none), so its alpha is kept as Windows programs mean it; arboard makes the
/// same change. Bit fields then need the colour masks after the header too, where Windows puts a
/// copy and the decoder skips them; a DIB without that copy (arboard writes one) gets it.
fn dib_layout(mut dib: Vec<u8>) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn coefficients_follow_sampling_and_mcu_padding() {
        // P, height, width, components (id, sampling, table).
        let sof = |w: u16, h: u16, comps: &[u8]| {
            let mut s = vec![8];
            s.extend(h.to_be_bytes());
            s.extend(w.to_be_bytes());
            s.push((comps.len() / 3) as u8);
            s.extend(comps);
            s
        };
        // Four components, no subsampling: 2 bytes a coefficient, 8 a pixel.
        assert_eq!(frame_coefficients(&sof(64, 64, &[1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 4, 0x11, 0])), Some(64 * 64 * 8));
        // 4:2:0: MCUs of 16 × 16 holding 4 + 1 + 1 blocks; a 1-pixel-wide picture still takes one.
        assert_eq!(frame_coefficients(&sof(1, 16, &[1, 0x22, 0, 2, 0x11, 0, 3, 0x11, 0])), Some(128 * 6));
        let mut short = sof(1, 16, &[1, 0x22, 0, 2, 0x11, 0]);
        short[5] = 3;
        assert_eq!(frame_coefficients(&short), None, "three components declared, two there");
        // A marker walk over SOI, an APP0 segment, SOF2, then the scan.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC2];
        let f = sof(64, 64, &[1, 0x11, 0]);
        jpeg.extend(((f.len() + 2) as u16).to_be_bytes());
        jpeg.extend(&f);
        jpeg.extend([0xFF, 0xDA]);
        assert_eq!(jpeg_coefficients(&jpeg), 64 * 64 * 2);
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
}
