//! Inline pictures: `sprmCPicLocation` → a `PICF` in the Data stream → embedded image bytes
//! ([MS-DOC] §2.9.190 PICF, §1.3.5 Pictures).

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;

/// `PICF.cbHeader` — the fixed size of the PICF structure.
const CB_HEADER: usize = 0x44;
/// Offsets inside PICF: `PICMID.dxaGoal/dyaGoal/mx/my` start at 28.
const OFF_PICMID: usize = 28;

/// Most distinct pictures one document materialises from the Data stream.
const MAX_PICTURES: usize = 5_000;
/// Most picture bytes one document copies out of the Data stream, all pictures together.
pub(crate) const MAX_MEDIA_BYTES: usize = 256 << 20;

/// The document's pictures, keyed by their location in the Data stream: anchors that share a
/// `fc_pic` share one media entry, and the total bytes copied are capped, so a file with
/// thousands of anchors on one huge picture costs one copy, not thousands.
pub(crate) struct Pictures {
    /// fc_pic → (media key, width, height) of pictures already materialised.
    by_fc: HashMap<u32, (String, f32, f32)>,
    /// The media entries, merged into the document at the end.
    pub(crate) media: BTreeMap<String, Arc<Vec<u8>>>,
    total: usize,
    /// The byte cap (`MAX_MEDIA_BYTES`; smaller in tests).
    pub(crate) limit: usize,
    /// Anchors whose picture was dropped (only the first few are logged).
    dropped: usize,
}

impl Default for Pictures {
    fn default() -> Self {
        Pictures { by_fc: HashMap::new(), media: BTreeMap::new(), total: 0, limit: MAX_MEDIA_BYTES, dropped: 0 }
    }
}

impl Pictures {
    /// The media key and display size (points) of the picture at `fc_pic`, copying its
    /// bytes the first time. `None` when the header is malformed, the payload is a format
    /// we do not embed (metafiles), or a picture count/byte cap is reached.
    pub(crate) fn get(&mut self, data: &[u8], fc_pic: u32) -> Option<(String, f32, f32)> {
        if let Some(hit) = self.by_fc.get(&fc_pic) {
            return Some(hit.clone());
        }
        if self.by_fc.len() >= MAX_PICTURES {
            return None;
        }
        let pic = locate(data, fc_pic)?;
        let bytes = data.get(pic.range)?;
        let total = self.total.checked_add(bytes.len()).filter(|&t| t <= self.limit)?;
        let key = format!("image{}.{}", self.by_fc.len() + 1, pic.ext);
        self.media.insert(key.clone(), Arc::new(bytes.to_vec()));
        self.total = total;
        self.by_fc.insert(fc_pic, (key.clone(), pic.w, pic.h));
        Some((key, pic.w, pic.h))
    }

    /// Note a picture anchor that could not be materialised.
    pub(crate) fn dropped(&mut self, cp: u32) {
        if self.dropped < 10 {
            log::warn!("docbin: picture at CP {cp} could not be read (or a picture cap was reached) and was dropped");
        }
        self.dropped = self.dropped.saturating_add(1);
    }
}

/// Where a picture's image bytes sit in the Data stream, and its display size in points.
struct Located {
    w: f32,
    h: f32,
    range: Range<usize>,
    ext: &'static str,
}

/// Locate the picture at `fc_pic` in the Data stream without copying it.
fn locate(data: &[u8], fc_pic: u32) -> Option<Located> {
    let base = fc_pic as usize;
    let pic = data.get(base..)?;
    let lcb = u32::from_le_bytes(pic.get(0..4)?.try_into().ok()?) as usize;
    let cb_header = u16::from_le_bytes(pic.get(4..6)?.try_into().ok()?) as usize;
    let cb_header = if (0x20..=0x100).contains(&cb_header) { cb_header } else { CB_HEADER };
    let m = pic.get(OFF_PICMID..OFF_PICMID + 8)?;
    let dxa_goal = i16::from_le_bytes([m[0], m[1]]) as f32;
    let dya_goal = i16::from_le_bytes([m[2], m[3]]) as f32;
    let mx = u16::from_le_bytes([m[4], m[5]]) as f32;
    let my = u16::from_le_bytes([m[6], m[7]]) as f32;
    let scale = |r: f32| if r > 0.0 { (r / 1000.0).clamp(0.01, 100.0) } else { 1.0 };
    let w = (dxa_goal / 20.0 * scale(mx)).clamp(1.0, 1584.0);
    let h = (dya_goal / 20.0 * scale(my)).clamp(1.0, 1584.0);
    let end = lcb.min(pic.len());
    let body = pic.get(cb_header..end)?;
    let at = sniff(body)?;
    let image = body.get(at..)?;
    let start = base.checked_add(cb_header)?.checked_add(at)?;
    Some(Located { w, h, range: start..start.checked_add(image.len())?, ext: ext_of(image) })
}

/// Offset of the first embedded raster image inside the payload, skipping the small
/// metafile header some writers put in front of PNG/JPEG data. `None` for metafiles.
fn sniff(body: &[u8]) -> Option<usize> {
    for at in [0usize, 8, 12, 16, 20, 22, 24, 32] {
        if let Some(b) = body.get(at..)
            && (b.starts_with(b"\x89PNG") || b.starts_with(b"\xFF\xD8\xFF") || b.starts_with(b"GIF8") || b.starts_with(b"BM"))
        {
            return Some(at);
        }
    }
    None
}

fn ext_of(b: &[u8]) -> &'static str {
    if b.starts_with(b"\x89PNG") {
        "png"
    } else if b.starts_with(b"\xFF\xD8\xFF") {
        "jpeg"
    } else if b.starts_with(b"GIF8") {
        "gif"
    } else {
        "bmp"
    }
}
