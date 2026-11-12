//! Inline pictures: `sprmCPicLocation` → a `PICF` in the Data stream → embedded image bytes
//! ([MS-DOC] §2.9.190 PICF, §1.3.5 Pictures), and the shared store of every picture the
//! document reads (inline and floating).

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;

use crate::officeart;

/// `PICF.cbHeader` — the fixed size of the PICF structure.
const CB_HEADER: usize = 0x44;
/// Offsets inside PICF: `PICMID.dxaGoal/dyaGoal/mx/my` start at 28.
const OFF_PICMID: usize = 28;

/// Most distinct pictures one document materialises from the Data stream.
const MAX_PICTURES: usize = 5_000;
/// Most picture bytes one document copies out of the Data stream, all pictures together.
pub(crate) const MAX_MEDIA_BYTES: usize = 256 << 20;
/// Most bytes one document decodes pictures into (metafiles inflated, DIB pixels), charged
/// per attempt whether it succeeds or not: retained media is capped separately, by
/// `MAX_MEDIA_BYTES` (#3711, #3712).
const MAX_DECODE_WORK: usize = 512 << 20;
/// The decode budget grows with the input: up to this many bytes per byte of the WordDocument
/// and Data streams. An uncompressed DIB declares at most 32 bytes of pixels per file byte (1
/// bit per pixel, 4 bytes charged each), so 64 passes any such picture. RLE8/RLE4 bitmaps
/// reach about 510 times that and rely on the `MAX_DECODE_WORK` floor instead; a decompression
/// bomb (ratios around 1e6) still hits the cap.
const DECODE_WORK_PER_INPUT_BYTE: usize = 64;
/// The most the decode budget grows to, whatever the input.
const MAX_DECODE_WORK_CEILING: usize = 2 << 30;

/// The decode budget for a document whose pictures live in `input` stream bytes, never less
/// than `floor`.
pub(crate) fn decode_work(input: usize, floor: usize) -> usize {
    input.saturating_mul(DECODE_WORK_PER_INPUT_BYTE).clamp(floor, floor.max(MAX_DECODE_WORK_CEILING))
}

/// The document's pictures, keyed by their location in the Data stream: anchors that share a
/// `fc_pic` share one media entry (or one failure), and the total bytes copied are capped, so a
/// file with thousands of anchors on one huge picture costs one copy, not thousands. Pictures of
/// the drawing layer's BLIP store are shared the same way, by store index.
pub(crate) struct Pictures {
    /// fc_pic → the inline picture already materialised (`None`: unreadable or over the cap,
    /// not retried).
    by_fc: HashMap<u32, Option<Inline>>,
    /// BLIP store index → media key (`None`: unreadable, not retried).
    by_bse: HashMap<u32, Option<String>>,
    /// (stream, foDelay, record length) of a BLIP record outside the store → media key: store
    /// entries and inline pictures that alias one record share one decode (`None`: failed, not
    /// retried). The stream is told apart by its address.
    by_delay: HashMap<(usize, usize, usize), Option<String>>,
    /// The media entries, merged into the document at the end.
    pub(crate) media: BTreeMap<String, Arc<Vec<u8>>>,
    total: usize,
    /// The byte cap (`MAX_MEDIA_BYTES`; smaller in tests).
    pub(crate) limit: usize,
    /// Decode work left (`MAX_DECODE_WORK`; smaller in tests).
    pub(crate) work: officeart::Decode,
    /// Anchors whose picture was dropped (only the first few are logged).
    dropped: usize,
    /// Pictures kept as placeholders (formats we can't draw).
    placeholders: usize,
}

impl Default for Pictures {
    fn default() -> Self {
        Pictures {
            by_fc: HashMap::new(),
            by_bse: HashMap::new(),
            by_delay: HashMap::new(),
            media: BTreeMap::new(),
            total: 0,
            limit: MAX_MEDIA_BYTES,
            work: officeart::Decode::new(MAX_DECODE_WORK),
            dropped: 0,
            placeholders: 0,
        }
    }
}

/// An inline picture: its media key, display size (points), crop fractions and alt text.
#[derive(Clone, Debug)]
pub(crate) struct Inline {
    pub(crate) key: String,
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) crop: [f32; 4],
    pub(crate) alt: String,
}

impl Pictures {
    /// An empty store whose decode budget scales with the `input` stream bytes (WordDocument
    /// plus Data) the pictures can come from.
    pub(crate) fn for_input(input: usize) -> Self {
        Pictures { work: officeart::Decode::new(decode_work(input, MAX_DECODE_WORK)), ..Pictures::default() }
    }

    /// The inline picture at `fc_pic` in the Data stream, copying its bytes the first time.
    /// `None` when the header is malformed, the picture can't be read, or a picture
    /// count/byte cap is reached.
    pub(crate) fn get(&mut self, data: &[u8], fc_pic: u32) -> Option<&Inline> {
        if !self.by_fc.contains_key(&fc_pic) {
            if self.by_fc.len().saturating_add(self.by_bse.len()) >= MAX_PICTURES {
                return None;
            }
            let pic = self.read(data, fc_pic);
            self.by_fc.insert(fc_pic, pic);
        }
        self.by_fc.get(&fc_pic)?.as_ref()
    }

    /// Read the inline picture at `fc_pic`: nothing is decoded or copied beyond what is left of
    /// the byte cap, and nothing at all once it is used up.
    fn read(&mut self, data: &[u8], fc_pic: u32) -> Option<Inline> {
        let room = self.limit.saturating_sub(self.total);
        if room == 0 {
            return None;
        }
        match locate(data, fc_pic, self)? {
            Found::Range(pic) => {
                let bytes = data.get(pic.range.clone()).filter(|b| b.len() <= room)?.to_vec();
                let key = self.insert(bytes, pic.ext, false)?;
                Some(Inline { key, w: pic.w, h: pic.h, crop: [0.0; 4], alt: String::new() })
            }
            Found::Shape { w, h, key, props } => Some(Inline { key: key?, w, h, crop: props.crop, alt: props.alt }),
        }
    }

    /// The media key of BLIP store entry `pib` (1-based), decoding it the first time; `delay`
    /// is the stream that holds BLIPs not embedded in the store (the WordDocument stream).
    pub(crate) fn bse(&mut self, pib: u32, bstore: &[&[u8]], delay: &[u8]) -> Option<String> {
        if let Some(hit) = self.by_bse.get(&pib) {
            return hit.clone();
        }
        if self.by_fc.len().saturating_add(self.by_bse.len()) >= MAX_PICTURES {
            return None;
        }
        let fbse = bstore.get((pib as usize).checked_sub(1)?)?;
        let key = self.decode_bse(fbse, delay);
        self.by_bse.insert(pib, key.clone());
        key
    }

    /// Decode a store entry's BLIP, once per delay-stream record however many entries name it.
    fn decode_bse(&mut self, fbse: &[u8], delay: &[u8]) -> Option<String> {
        let (rec, at) = officeart::fbse_record(fbse, delay)?;
        let at = at.map(|(fo, used)| (delay.as_ptr() as usize, fo, used));
        if let Some(hit) = at.and_then(|at| self.by_delay.get(&at)) {
            return hit.clone();
        }
        let room = self.limit.saturating_sub(self.total);
        let key = officeart::blip(&rec, room, &mut self.work).and_then(|b| self.insert(b.bytes, b.ext, b.placeholder));
        if let Some(at) = at {
            self.by_delay.insert(at, key.clone());
        }
        key
    }

    /// Add picture bytes as a new media entry, within the byte cap.
    fn insert(&mut self, bytes: Vec<u8>, ext: &str, placeholder: bool) -> Option<String> {
        let total = self.total.checked_add(bytes.len()).filter(|&t| t <= self.limit)?;
        let key = format!("image{}.{ext}", self.media.len() + 1);
        if placeholder {
            if self.placeholders < 10 {
                log::warn!("docbin: picture `{key}` is in a format WordCraft can't draw ({ext}); it shows as a placeholder");
            }
            self.placeholders = self.placeholders.saturating_add(1);
        }
        self.media.insert(key.clone(), Arc::new(bytes));
        self.total = total;
        Some(key)
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

/// What a `PICFAndOfficeArtData` holds: raw image bytes (older writers), or an
/// `OfficeArtInlineSpContainer` (Word 97 and later) whose BLIP is decoded.
enum Found {
    Range(Located),
    Shape { w: f32, h: f32, key: Option<String>, props: officeart::Props },
}

/// `PICF.mfpf.mm` values of pictures stored as a shape (`MM_SHAPE`) or a shape with a file
/// name (`MM_SHAPEFILE`).
const MM_SHAPE: u16 = 0x0064;
const MM_SHAPEFILE: u16 = 0x0066;

/// Locate the picture at `fc_pic` in the Data stream ([MS-DOC] §2.9.192 PICFAndOfficeArtData);
/// the BLIP is decoded once per record and charged to the decode budget.
fn locate(data: &[u8], fc_pic: u32, pics: &mut Pictures) -> Option<Found> {
    let base = fc_pic as usize;
    let pic = data.get(base..)?;
    let lcb = u32::from_le_bytes(pic.get(0..4)?.try_into().ok()?) as usize;
    let cb_header = u16::from_le_bytes(pic.get(4..6)?.try_into().ok()?) as usize;
    let cb_header = if (0x20..=0x100).contains(&cb_header) { cb_header } else { CB_HEADER };
    let mm = u16::from_le_bytes(pic.get(6..8)?.try_into().ok()?);
    let m = pic.get(OFF_PICMID..OFF_PICMID + 8)?;
    let dxa_goal = i16::from_le_bytes([m[0], m[1]]) as f32;
    let dya_goal = i16::from_le_bytes([m[2], m[3]]) as f32;
    let mx = u16::from_le_bytes([m[4], m[5]]) as f32;
    let my = u16::from_le_bytes([m[6], m[7]]) as f32;
    let scale = |r: f32| if r > 0.0 { (r / 1000.0).clamp(0.01, 100.0) } else { 1.0 };
    let end = lcb.min(pic.len());
    if mm == MM_SHAPE || mm == MM_SHAPEFILE {
        let mut at = cb_header;
        if mm == MM_SHAPEFILE {
            at = at.checked_add(1 + *pic.get(at)? as usize)?;
        }
        let (key, props) = inline_shape(pic.get(at..end)?, data, pics)?;
        // The goal size is before cropping; the crop fractions take their share off it.
        let [cl, ct, cr, cb] = props.crop;
        let w = (dxa_goal / 20.0 * (1.0 - cl - cr).max(0.01) * scale(mx)).clamp(1.0, 1584.0);
        let h = (dya_goal / 20.0 * (1.0 - ct - cb).max(0.01) * scale(my)).clamp(1.0, 1584.0);
        return Some(Found::Shape { w, h, key, props });
    }
    let w = (dxa_goal / 20.0 * scale(mx)).clamp(1.0, 1584.0);
    let h = (dya_goal / 20.0 * scale(my)).clamp(1.0, 1584.0);
    let body = pic.get(cb_header..end)?;
    let at = sniff(body)?;
    let image = body.get(at..)?;
    let start = base.checked_add(cb_header)?.checked_add(at)?;
    Some(Found::Range(Located { w, h, range: start..start.checked_add(image.len())?, ext: ext_of(image) }))
}

/// An `OfficeArtInlineSpContainer`: the picture shape, then the `OfficeArtFBSE` records its
/// `pib` indexes (BLIPs embedded, or at `foDelay` in the Data stream). The BLIP is decoded
/// once per record, however many pictures (and FBSE sizes) name it.
fn inline_shape(buf: &[u8], data: &[u8], pics: &mut Pictures) -> Option<(Option<String>, officeart::Props)> {
    let mut budget = 10_000usize;
    let recs = officeart::children(buf, &mut budget);
    let sp = recs.iter().find(|r| r.typ == officeart::rt::SP_CONTAINER).map(|r| officeart::shape(r.body, &mut budget))?;
    let fbses: Vec<&[u8]> = recs.iter().filter(|r| r.typ == officeart::rt::FBSE).map(|r| r.body).collect();
    let fbse = sp.props.pib.and_then(|p| fbses.get((p as usize).checked_sub(1)?)).or(fbses.first())?;
    let key = pics.decode_bse(fbse, data);
    Some((key, sp.props))
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
