//! Bounded, loss-aware preparation of native-ink fallback pictures for ODF.
use image::ImageEncoder;
use std::{
    io::{self, Write},
    sync::Arc,
};

const BUDGET: u64 = wordcraft_geom::MAX_DECODED_BYTES;

/// PNG CRC-32 (ISO 3309), covering the chunk type and data, not its length.
fn png_crc(bytes: &[u8]) -> u32 {
    const fn table() -> [u32; 256] {
        let mut t = [0; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut bit = 0;
            while bit < 8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
                bit += 1;
            }
            t[i] = c;
            i += 1;
        }
        t
    }
    const TABLE: [u32; 256] = table();
    let mut crc = u32::MAX;
    for byte in bytes {
        crc = TABLE[((crc ^ u32::from(*byte)) & 255) as usize] ^ (crc >> 8);
    }
    !crc
}

/// Only complete, valid, simple 8-bit PNGs can be re-encoded without dropping colour profiles,
/// animation, orientation or other metadata. Check all chunk CRCs, including IEND: the image
/// decoder reads the pixels but need not consume the final chunk.
fn plain_png(data: &[u8]) -> bool {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") || data.len() < 33 || data.get(24) != Some(&8) || !matches!(data.get(25), Some(2 | 6)) {
        return false;
    }
    let mut at = 8usize;
    let mut saw_data = false;
    while let Some(header) = data.get(at..at.saturating_add(8)) {
        let Ok(len) = <[u8; 4]>::try_from(&header[..4]) else { return false };
        let len = u32::from_be_bytes(len) as usize;
        let Some(end) = at.checked_add(12).and_then(|n| n.checked_add(len)) else { return false };
        let Some(chunk) = data.get(at + 4..end.saturating_sub(4)) else { return false };
        let Some(crc) = data.get(end.saturating_sub(4)..end).and_then(|b| <[u8; 4]>::try_from(b).ok()) else { return false };
        if png_crc(chunk) != u32::from_be_bytes(crc) {
            return false;
        }
        match &header[4..8] {
            b"IHDR" if at == 8 && len == 13 => {}
            b"IDAT" if at > 8 => saw_data = true,
            b"IEND" if len == 0 && saw_data => return end == data.len(),
            _ => return false,
        }
        at = end;
    }
    false
}

/// Size of all filtered scanlines, including Adam7 pass rows. These are 8-bit RGB or RGBA.
fn scanline_bytes(data: &[u8], w: u32, h: u32) -> Option<u64> {
    let pixel = match data.get(25)? {
        2 => 3u64,
        6 => 4,
        _ => return None,
    };
    let w = u64::from(w);
    let h = u64::from(h);
    let pass = |x: u64, y: u64, dx: u64, dy: u64| -> Option<u64> {
        let pw = w.saturating_sub(x).div_ceil(dx);
        let ph = h.saturating_sub(y).div_ceil(dy);
        if pw == 0 || ph == 0 {
            return Some(0);
        }
        pw.checked_mul(pixel)?.checked_add(1)?.checked_mul(ph)
    };
    match data.get(28)? {
        0 => pass(0, 0, 1, 1),
        1 => [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
            .into_iter()
            .try_fold(0u64, |sum, (x, y, dx, dy)| sum.checked_add(pass(x, y, dx, dy)?)),
        _ => None,
    }
}

/// Inflate the concatenated IDAT stream without retaining pixels, requiring an exact zlib end,
/// checksum and scanline length. `plain_png` has already checked each chunk's bounds and CRC.
fn complete_png_stream(data: &[u8], expected: u64) -> bool {
    use miniz_oxide::{
        DataFormat, MZFlush, MZStatus,
        inflate::stream::{InflateState, inflate},
    };
    let mut state = InflateState::new_boxed(DataFormat::Zlib);
    let mut output = [0u8; 8192];
    let mut produced = 0u64;
    let mut ended = false;
    let mut at = 33usize;
    loop {
        let Some(header) = data.get(at..at.saturating_add(8)) else { return false };
        let Ok(len) = <[u8; 4]>::try_from(&header[..4]) else { return false };
        let len = u32::from_be_bytes(len) as usize;
        let Some(start) = at.checked_add(8) else { return false };
        let Some(end) = start.checked_add(len) else { return false };
        if &header[4..8] == b"IEND" {
            return ended && produced == expected;
        }
        if &header[4..8] != b"IDAT" {
            return false;
        }
        let Some(mut input) = data.get(start..end) else { return false };
        if ended && !input.is_empty() {
            return false;
        }
        while !input.is_empty() && !ended {
            let result = inflate(&mut state, input, &mut output, MZFlush::None);
            let Some(rest) = input.get(result.bytes_consumed..) else { return false };
            input = rest;
            let Some(total) = produced.checked_add(result.bytes_written as u64) else { return false };
            produced = total;
            if produced > expected {
                return false;
            }
            match result.status {
                Ok(MZStatus::StreamEnd) => ended = true,
                Ok(MZStatus::Ok) if result.bytes_consumed != 0 || result.bytes_written != 0 => {}
                _ => return false,
            }
        }
        if ended && !input.is_empty() {
            return false;
        }
        let Some(next) = end.checked_add(4) else { return false };
        at = next;
        // Inflate may have buffered output after consuming a chunk. Drain it before asking for
        // more input; a no-progress buffer result means it genuinely needs the next chunk.
        while !ended {
            let result = inflate(&mut state, &[], &mut output, MZFlush::None);
            let Some(total) = produced.checked_add(result.bytes_written as u64) else { return false };
            produced = total;
            if produced > expected {
                return false;
            }
            match result.status {
                Ok(MZStatus::StreamEnd) => ended = true,
                Ok(MZStatus::Ok) if result.bytes_written > 0 => {}
                Err(miniz_oxide::MZError::Buf) if result.bytes_written == 0 => break,
                _ => return false,
            }
        }
    }
}

struct Limited {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("picture export budget exceeded"));
        }
        let needed = self.bytes.len() + buf.len();
        if needed > self.bytes.capacity() {
            let target = needed.max(self.bytes.capacity().saturating_mul(2)).min(self.limit);
            self.bytes.try_reserve_exact(target - self.bytes.len()).map_err(|_| io::Error::other("picture export allocation failed"))?;
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn transform(data: &[u8], fx: wordcraft_doc::picfx::PicFx, remaining: &mut u64) -> Result<Vec<u8>, String> {
    let refuse = |why: &str| format!("ODT: cannot preserve native ink picture effects: {why}");
    if u64::try_from(data.len()).unwrap_or(u64::MAX) > *remaining || !plain_png(data) {
        return Err(refuse("unsupported, metadata-bearing, or over-budget picture"));
    }
    let probe = wordcraft_geom::picture::probe(data).map_err(|e| refuse(&e.to_string()))?;
    // Charge each image's decode peak and encoded input/output against one document-wide budget.
    let scanlines = scanline_bytes(data, probe.width, probe.height).ok_or_else(|| refuse("invalid PNG dimensions"))?;
    // Validation inflates once in addition to the decoder; charge its work and fixed buffers.
    let charge = probe
        .peak_bytes()
        .checked_add(data.len() as u64)
        .and_then(|n| n.checked_add(scanlines))
        .and_then(|n| n.checked_add(std::mem::size_of::<miniz_oxide::inflate::stream::InflateState>() as u64 + 8192))
        .ok_or_else(|| refuse("picture budget exceeded"))?;
    *remaining = remaining.checked_sub(charge).ok_or_else(|| refuse("picture budget exceeded"))?;
    if !complete_png_stream(data, scanlines) {
        return Err(refuse("invalid or incomplete PNG image stream"));
    }
    let mut rgba = probe.decode(BUDGET).map_err(|e| refuse(&e.to_string()))?.into_rgba8();
    fx.apply_rgba(rgba.as_mut());
    // Opacity is applied to existing straight alpha; 8-bit PNG quantizes to nearest integer.
    for p in rgba.pixels_mut() {
        p.0[3] = (f32::from(p.0[3]) * fx.opacity()).round() as u8;
    }
    let mut out = Limited { bytes: Vec::new(), limit: usize::try_from(*remaining).unwrap_or(usize::MAX) };
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgba.as_raw(), rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)
        .map_err(|e| refuse(&e.to_string()))?;
    *remaining = remaining.checked_sub(out.bytes.capacity() as u64).ok_or_else(|| refuse("picture budget exceeded"))?;
    Ok(out.bytes)
}

/// One export shares its cache and total budget across body and page stories.
pub(super) struct InkPictures {
    remaining: u64,
    cache: std::collections::HashMap<(usize, wordcraft_doc::picfx::PicFx), (Arc<Vec<u8>>, Arc<Vec<u8>>)>,
    pub error: Option<String>,
    pub names: std::collections::HashMap<usize, String>,
}
impl Default for InkPictures {
    fn default() -> Self {
        Self { remaining: BUDGET, cache: Default::default(), error: None, names: Default::default() }
    }
}
impl InkPictures {
    pub fn bake(&mut self, source: &Arc<Vec<u8>>, fx: wordcraft_doc::picfx::PicFx) -> Option<Arc<Vec<u8>>> {
        if self.error.is_some() {
            return None;
        }
        let key = (Arc::as_ptr(source) as usize, fx.clamped());
        if let Some((_, baked)) = self.cache.get(&key) {
            return Some(baked.clone());
        }
        match transform(source, key.1, &mut self.remaining) {
            Ok(bytes) => {
                let baked = Arc::new(bytes);
                self.cache.insert(key, (source.clone(), baked.clone()));
                Some(baked)
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }
    pub fn retained(&mut self) {
        if self.error.is_none() {
            self.error = Some("ODT: cannot preserve retained markup and native ink picture effects together".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::picfx::PicFx;

    fn png(data: &[u8], width: u32, color: image::ExtendedColorType) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out).write_image(data, width, 1, color).unwrap();
        out
    }

    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = (payload.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(&png_crc(&out[4..]).to_be_bytes());
        out
    }

    #[test]
    fn ink_picture_rejects_incomplete_or_excess_zlib_stream() {
        let source = png(&[10, 20, 30, 40], 1, image::ExtendedColorType::Rgba8);
        let encoded = miniz_oxide::deflate::compress_to_vec_zlib(&[0, 10, 20, 30, 40], 6);
        let make = |chunks: &[&[u8]]| {
            let mut file = source[..33].to_vec();
            for data in chunks {
                file.extend(chunk(b"IDAT", data));
            }
            file.extend(chunk(b"IEND", &[]));
            file
        };
        let split = encoded.len() / 2;
        let fx = PicFx::new(1, 0, 0);
        assert!(transform(&make(&[&encoded[..split], &encoded[split..]]), fx, &mut BUDGET.clone()).is_ok());
        let pieces: Vec<&[u8]> = encoded.chunks(1).collect();
        assert!(transform(&make(&pieces), fx, &mut BUDGET.clone()).is_ok());
        let mut appended = encoded.clone();
        appended.extend_from_slice(b"junk");
        let extra_raw = miniz_oxide::deflate::compress_to_vec_zlib(&[0, 10, 20, 30, 40, 99], 6);
        let failures: Vec<_> = [
            ("extra IDAT", make(&[&encoded, b"junk"])),
            ("extra compressed bytes", make(&[&appended])),
            ("truncated stream", make(&[&encoded[..encoded.len() - 1]])),
            ("extra decoded bytes", make(&[&extra_raw])),
        ]
        .into_iter()
        .filter_map(|(name, bytes)| transform(&bytes, fx, &mut BUDGET.clone()).is_ok().then_some(name))
        .collect();
        assert!(failures.is_empty(), "accepted {failures:?}");
    }

    #[test]
    fn ink_picture_validates_interlaced_and_large_png_streams() {
        let source = png(&[10, 20, 30, 40], 1, image::ExtendedColorType::Rgba8);
        let mut interlaced = source.clone();
        interlaced[28] = 1;
        let crc = png_crc(&interlaced[12..29]);
        interlaced[29..33].copy_from_slice(&crc.to_be_bytes());
        assert!(transform(&interlaced, PicFx::new(1, 0, 0), &mut BUDGET.clone()).is_ok());
        let mut header = Vec::new();
        image::codecs::png::PngEncoder::new(&mut header).write_image(&[0; 8 * 8 * 4], 8, 8, image::ExtendedColorType::Rgba8).unwrap();
        header.truncate(33);
        header[28] = 1;
        let crc = png_crc(&header[12..29]);
        header[29..33].copy_from_slice(&crc.to_be_bytes());
        // Adam7 for 8×8 has 64 pixels and 15 filter bytes across all seven passes.
        header.extend(chunk(b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&[0; 271], 6)));
        header.extend(chunk(b"IEND", &[]));
        assert!(transform(&header, PicFx::new(1, 0, 0), &mut BUDGET.clone()).is_ok());
        let pixels: Vec<u8> = (0..256 * 256 * 4).map(|n| ((n * 73 + n / 51) % 256) as u8).collect();
        let mut large = Vec::new();
        image::codecs::png::PngEncoder::new(&mut large).write_image(&pixels, 256, 256, image::ExtendedColorType::Rgba8).unwrap();
        assert!(transform(&large, PicFx::new(1, 0, 0), &mut BUDGET.clone()).is_ok());
    }

    #[test]
    fn ink_picture_rejects_corrupt_or_malformed_png_end() {
        let source = png(&[10, 20, 30, 40], 1, image::ExtendedColorType::Rgba8);
        let mut crc = source.clone();
        *crc.last_mut().unwrap() ^= 1;
        let mut extra = source.clone();
        extra.push(0);
        let mut malformed = source.clone();
        let pos = malformed.len() - 12;
        malformed[pos..pos + 4].copy_from_slice(&1u32.to_be_bytes());
        malformed.insert(pos + 8, 0);
        let crc_pos = pos + 9;
        let valid_crc = png_crc(&malformed[pos + 4..crc_pos]);
        malformed[crc_pos..crc_pos + 4].copy_from_slice(&valid_crc.to_be_bytes());
        let mut duplicate_header = source.clone();
        duplicate_header.splice(33..33, source[8..33].iter().copied());
        let mut bad_idat = source.clone();
        let idat = bad_idat.windows(4).position(|w| w == b"IDAT").unwrap();
        bad_idat[idat + 4] ^= 1;
        assert_eq!(png_crc(b"123456789"), 0xcbf4_3926);
        let fx = PicFx::new(1, 0, 0);
        let failures: Vec<_> = [
            ("corrupt IEND CRC", crc),
            ("trailing data", extra),
            ("nonempty IEND", malformed),
            ("duplicate IHDR", duplicate_header),
            ("corrupt IDAT", bad_idat),
        ]
        .into_iter()
        .filter_map(|(name, data)| transform(&data, fx, &mut BUDGET.clone()).is_ok().then_some(name))
        .collect();
        assert!(failures.is_empty(), "accepted {failures:?}");
    }

    #[test]
    fn shared_picture_cache_charges_once_and_distinguishes_effects() {
        let source = Arc::new(png(&[10, 20, 30, 255], 1, image::ExtendedColorType::Rgba8));
        let first_fx = PicFx::new(20_000, 0, 0);
        let other_fx = PicFx::new(-20_000, 0, 0);
        let mut cache = InkPictures::default();
        let first = cache.bake(&source, first_fx).unwrap();
        let remaining = cache.remaining;
        for _ in 0..1000 {
            let repeated = cache.bake(&source, first_fx).unwrap();
            assert!(Arc::ptr_eq(&first, &repeated));
        }
        assert_eq!(cache.remaining, remaining);
        let other = cache.bake(&source, other_fx).unwrap();
        assert!(!Arc::ptr_eq(&first, &other));
        assert_ne!(first.as_slice(), other.as_slice());
        assert!(cache.remaining < remaining);
    }

    #[test]
    fn ink_picture_transform_keeps_and_scales_each_alpha() {
        let source = png(&[20, 80, 200, 0, 20, 80, 200, 128, 20, 80, 200, 255], 3, image::ExtendedColorType::Rgba8);
        let fx = PicFx::new(20_000, -40_000, 25_000);
        let result = transform(&source, fx, &mut BUDGET.clone()).unwrap();
        let actual = image::load_from_memory(&result).unwrap().into_rgba8();
        let mut expected = vec![20, 80, 200, 0, 20, 80, 200, 128, 20, 80, 200, 255];
        fx.apply_rgba(&mut expected);
        for (i, alpha) in [0, 96, 191].into_iter().enumerate() {
            expected[i * 4 + 3] = alpha;
        }
        assert_eq!(actual.as_raw(), &expected);
    }

    #[test]
    fn ink_picture_transform_refuses_metadata_wide_and_over_budget() {
        let source = png(&[10, 20, 30, 40], 1, image::ExtendedColorType::Rgba8);
        let fx = PicFx::new(1, 0, 0);
        assert!(transform(&source, fx, &mut 1).is_err());
        assert!(transform(b"not a raster", fx, &mut BUDGET.clone()).is_err());
        let wide = png(&[0; 8], 1, image::ExtendedColorType::Rgba16);
        assert!(transform(&wide, fx, &mut BUDGET.clone()).is_err());
        let mut metadata = source.clone();
        // A valid empty ancillary chunk is enough: no metadata is silently discarded.
        metadata.splice(33..33, [0, 0, 0, 0, b't', b'E', b'X', b't', 0, 0, 0, 0]);
        assert!(transform(&metadata, fx, &mut BUDGET.clone()).is_err());
        let mut remaining = BUDGET;
        transform(&source, fx, &mut remaining).unwrap();
        assert!(remaining < BUDGET - source.len() as u64);
        let charge = BUDGET - remaining;
        let mut shared = charge;
        transform(&source, fx, &mut shared).unwrap();
        assert!(transform(&source, fx, &mut shared).is_err());
    }
}
