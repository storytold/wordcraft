//! OfficeArt (Escher) drawing records, as Word 97-2003 stores them ([MS-ODRAW], public Microsoft
//! Open Specifications): the record header, the BLIP store and its pictures, shape containers with
//! their property tables, and the `OfficeArtContent` of the Table stream ([MS-DOC] §2.9.172).
//!
//! Everything here walks hostile bytes: record lengths are clamped to their parent, nesting depth
//! and record counts are capped, and decompressed metafiles are size-limited.

use std::collections::HashMap;
use std::io::Write;

use image::ImageEncoder;

use wordcraft_doc::props::Rgb;

/// Deepest container nesting walked (groups inside groups…).
const MAX_DEPTH: usize = 12;
/// Most records walked in one drawing tree or BLIP store.
const MAX_RECORDS: usize = 200_000;
/// Most BLIP store entries kept.
const MAX_BSE: usize = 10_000;
/// Most drawings (main document, headers) read from `OfficeArtContent`.
const MAX_DRAWINGS: usize = 8;
/// Largest decompressed metafile (bytes).
const MAX_METAFILE: usize = 64 << 20;
/// Largest DIB converted to PNG, per side (pixels).
const MAX_DIB_SIDE: u32 = 16_384;

/// Record types ([MS-ODRAW] §2.2, §2.3).
pub(crate) mod rt {
    pub(crate) const DGG_CONTAINER: u16 = 0xF000;
    pub(crate) const BSTORE_CONTAINER: u16 = 0xF001;
    pub(crate) const DG_CONTAINER: u16 = 0xF002;
    pub(crate) const SPGR_CONTAINER: u16 = 0xF003;
    pub(crate) const SP_CONTAINER: u16 = 0xF004;
    pub(crate) const FBSE: u16 = 0xF007;
    pub(crate) const FSPGR: u16 = 0xF009;
    pub(crate) const FSP: u16 = 0xF00A;
    pub(crate) const FOPT: u16 = 0xF00B;
    pub(crate) const CLIENT_TEXTBOX: u16 = 0xF00D;
    pub(crate) const CHILD_ANCHOR: u16 = 0xF00F;
    pub(crate) const SECONDARY_FOPT: u16 = 0xF121;
    pub(crate) const TERTIARY_FOPT: u16 = 0xF122;
    pub(crate) const BLIP_EMF: u16 = 0xF01A;
    pub(crate) const BLIP_WMF: u16 = 0xF01B;
    pub(crate) const BLIP_PICT: u16 = 0xF01C;
    pub(crate) const BLIP_JPEG: u16 = 0xF01D;
    pub(crate) const BLIP_PNG: u16 = 0xF01E;
    pub(crate) const BLIP_DIB: u16 = 0xF01F;
    pub(crate) const BLIP_TIFF: u16 = 0xF029;
    pub(crate) const BLIP_JPEG_CMYK: u16 = 0xF02A;
}

/// One record: header fields and its body (clamped to what the parent holds).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rec<'a> {
    pub(crate) inst: u16,
    pub(crate) typ: u16,
    pub(crate) body: &'a [u8],
}

/// The record at the start of `buf` and the bytes it spans (header included). A body longer
/// than `buf` is cut to what is there.
pub(crate) fn record(buf: &[u8]) -> Option<(Rec<'_>, usize)> {
    let h = buf.get(0..8)?;
    let ver_inst = u16::from_le_bytes([h[0], h[1]]);
    let typ = u16::from_le_bytes([h[2], h[3]]);
    let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
    let rest = buf.get(8..)?;
    let len = len.min(rest.len());
    let body = rest.get(..len)?;
    Some((Rec { inst: ver_inst >> 4, typ, body }, 8 + len))
}

/// The sibling records of a container body, at most `budget` of them (the budget is shared
/// across a whole walk, so a deep or wide tree cannot multiply the work).
pub(crate) fn children<'a>(body: &'a [u8], budget: &mut usize) -> Vec<Rec<'a>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while *budget > 0 {
        let Some((rec, used)) = body.get(at..).and_then(record) else { break };
        *budget -= 1;
        out.push(rec);
        at = at.saturating_add(used);
    }
    out
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    u32_at(b, at).map(|v| v as i32)
}

// ---------------------------------------------------------------------------------------------
// Pictures (BLIPs).

/// A decoded picture: bytes ready for `Document::media` and their file extension.
pub(crate) struct Blip {
    pub(crate) bytes: Vec<u8>,
    pub(crate) ext: &'static str,
    /// The format cannot be drawn (Macintosh PICT): the picture is kept as a placeholder of
    /// the right size.
    pub(crate) placeholder: bool,
}

/// The picture of an `OfficeArtFBSE` body: its embedded BLIP, or the BLIP stored at `foDelay`
/// in `delay` (the WordDocument stream for the document's BLIP store, the Data stream for
/// inline pictures). `limit` caps the bytes produced.
#[cfg(test)]
pub(crate) fn fbse_blip(fbse: &[u8], delay: &[u8], limit: usize, work: &mut Decode) -> Option<Blip> {
    let (rec, _) = fbse_record(fbse, delay)?;
    blip(&rec, limit, work)
}

/// The document's decode budget: bytes of inflating and pixel decoding left, and how many
/// attempts it has refused (only the first few are logged).
pub(crate) struct Decode {
    pub(crate) left: usize,
    refused: usize,
}

impl Decode {
    pub(crate) fn new(left: usize) -> Self {
        Decode { left, refused: 0 }
    }

    /// Take `cost` off the budget; `false` (and a log line) when it doesn't fit.
    fn charge(&mut self, cost: usize, what: &str) -> bool {
        match self.left.checked_sub(cost) {
            Some(left) if cost > 0 => {
                self.left = left;
                true
            }
            _ => {
                if self.refused < 10 {
                    log::warn!(
                        "docbin: a {what} needing {cost} bytes of decode work is over the decode budget ({} left) and was not decoded",
                        self.left
                    );
                }
                self.refused = self.refused.saturating_add(1);
                false
            }
        }
    }
}

/// The BLIP record of an `OfficeArtFBSE`: embedded after it, or in the delay stream, where
/// its (`foDelay`, record length) location is returned too (entries can share one record,
/// whatever sizes their FBSEs claim: the record's own header is what counts).
pub(crate) fn fbse_record<'a>(fbse: &'a [u8], delay: &'a [u8]) -> Option<(Rec<'a>, Option<(usize, usize)>)> {
    // btWin32, btMacOS, rgbUid[16], tag, size, cRef, foDelay, unused, cbName, unused×2.
    let fo_delay = u32_at(fbse, 28)? as usize;
    let cb_name = *fbse.get(33)? as usize;
    let at = 36usize.checked_add(cb_name)?;
    if let Some((rec, _)) = fbse.get(at..).and_then(record) {
        return Some((rec, None));
    }
    // Not embedded: the record sits in the delay stream.
    let (rec, used) = record(delay.get(fo_delay..)?)?;
    Some((rec, Some((fo_delay, used))))
}

/// Decode one `OfficeArtBlip*` record ([MS-ODRAW] §2.2.23-§2.2.31). `work` is the
/// document's decode budget: inflating a metafile or decoding a DIB is charged to it before
/// it runs, whether it succeeds or not, and nothing is decoded once it is spent.
pub(crate) fn blip(rec: &Rec, limit: usize, work: &mut Decode) -> Option<Blip> {
    // Each type has two instance values; the odd one carries a second 16-byte UID.
    let uids = if rec.inst & 1 == 1 { 32 } else { 16 };
    match rec.typ {
        rt::BLIP_PNG | rt::BLIP_JPEG | rt::BLIP_JPEG_CMYK | rt::BLIP_DIB | rt::BLIP_TIFF => {
            let data = rec.body.get(uids + 1..)?; // + tag byte
            if data.is_empty() || data.len() > limit {
                return None;
            }
            match rec.typ {
                rt::BLIP_PNG => Some(Blip { bytes: data.to_vec(), ext: "png", placeholder: false }),
                rt::BLIP_JPEG | rt::BLIP_JPEG_CMYK => Some(Blip { bytes: data.to_vec(), ext: "jpeg", placeholder: false }),
                rt::BLIP_TIFF => Some(Blip { bytes: data.to_vec(), ext: "tiff", placeholder: false }),
                _ => dib_to_png(data, limit, work),
            }
        }
        rt::BLIP_EMF | rt::BLIP_WMF | rt::BLIP_PICT => {
            // OfficeArtMetafileHeader: cbSize, rcBounds, ptSize, cbSave, compression, filter.
            let hdr = rec.body.get(uids..uids + 34)?;
            let cb_size = u32_at(hdr, 0)? as usize;
            let cb_save = u32_at(hdr, 28)? as usize;
            let compression = *hdr.get(32)?;
            let raw = rec.body.get(uids + 34..)?;
            let raw = raw.get(..cb_save.min(raw.len()))?;
            let cap = limit.min(MAX_METAFILE);
            let bytes = if compression == 0 {
                let want = if cb_size > 0 { cb_size.min(cap) } else { cap };
                // The compressed stream is read whole whatever it inflates to: a record that
                // claims to inflate to nothing still costs its input (#3871).
                if !work.charge(want.max(raw.len()), "metafile") {
                    return None;
                }
                miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(raw, want).ok()?
            } else {
                if raw.len() > cap {
                    return None;
                }
                raw.to_vec()
            };
            if bytes.is_empty() {
                return None;
            }
            match rec.typ {
                rt::BLIP_EMF => Some(Blip { bytes, ext: "emf", placeholder: false }),
                rt::BLIP_WMF => Some(Blip { bytes, ext: "wmf", placeholder: false }),
                _ => Some(Blip { bytes, ext: "pict", placeholder: true }),
            }
        }
        _ => None,
    }
}

/// A device-independent bitmap (BITMAPINFOHEADER + palette + pixels) as a PNG: the DIB gets
/// the 14-byte BMP file header it lacks, is decoded with size limits and re-encoded. When the
/// bitmap can't be decoded, or its declared pixels (4 bytes each) don't fit in `work`, the
/// BMP bytes are kept (and render as a grey placeholder).
fn dib_to_png(dib: &[u8], limit: usize, work: &mut Decode) -> Option<Blip> {
    let header = u32_at(dib, 0)? as usize;
    if !(12..=124).contains(&header) {
        return None;
    }
    let (bits, used, compression) =
        if header == 12 { (u16_at(dib, 10)? as usize, 0, 0) } else { (u16_at(dib, 14)? as usize, u32_at(dib, 32)? as usize, u32_at(dib, 16)?) };
    let entry = if header == 12 { 3 } else { 4 };
    let colors = if used > 0 {
        used.min(256)
    } else if bits <= 8 {
        1usize << bits
    } else {
        0
    };
    // BI_BITFIELDS masks follow a 40-byte header.
    let masks = if header == 40 && (compression == 3 || compression == 6) { if compression == 3 { 12 } else { 16 } } else { 0 };
    let off_bits = 14usize.checked_add(header)?.checked_add(colors * entry)?.checked_add(masks)?;
    let file_size = 14usize.checked_add(dib.len()).filter(|&size| size <= limit)?;
    // The declared size is what decoding allocates, however small the (RLE) data is.
    let (w, h) = if header == 12 {
        (u64::from(u16_at(dib, 4)?), u64::from(u16_at(dib, 6)?))
    } else {
        (u64::from(u32_at(dib, 4)?), u64::from((u32_at(dib, 8)? as i32).unsigned_abs()))
    };
    let cost = w.saturating_mul(h).saturating_mul(4).try_into().unwrap_or(usize::MAX);
    // Copying the DIB into a BMP is work too, however few pixels it declares: it is charged
    // before the copy is made (#3871). A bitmap whose pixels don't fit still keeps its BMP
    // bytes as a placeholder, if the copy fits.
    let decode = work.charge(cost.max(dib.len()), "bitmap");
    if !decode && !work.charge(dib.len(), "bitmap") {
        return None;
    }
    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&u32::try_from(file_size).ok()?.to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&u32::try_from(off_bits).ok()?.to_le_bytes());
    bmp.extend_from_slice(dib);
    let placeholder = |bmp| Some(Blip { bytes: bmp, ext: "bmp", placeholder: true });
    if !decode {
        return placeholder(bmp);
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bmp));
    reader.set_format(image::ImageFormat::Bmp);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIB_SIDE);
    limits.max_image_height = Some(MAX_DIB_SIDE);
    limits.max_alloc = Some((256 << 20).min(limit as u64));
    reader.limits(limits);
    let mut png = PictureWriter { bytes: Vec::new(), limit };
    match reader.decode() {
        Ok(img)
            if image::codecs::png::PngEncoder::new(&mut png).write_image(img.as_bytes(), img.width(), img.height(), img.color().into()).is_ok() =>
        {
            Some(Blip { bytes: png.bytes, ext: "png", placeholder: false })
        }
        _ => placeholder(bmp),
    }
}

/// Cap encoded bytes while the PNG encoder writes, before the buffer can grow.
struct PictureWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for PictureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let Some(end) = self.bytes.len().checked_add(buf.len()).filter(|&end| end <= self.limit) else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "picture exceeds remaining media budget"));
        };
        if end > self.bytes.capacity() {
            let new_capacity = self.bytes.capacity().saturating_mul(2).max(end).min(self.limit);
            self.bytes.try_reserve_exact(new_capacity - self.bytes.len()).map_err(std::io::Error::other)?;
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Shapes.

/// The shape properties we use, from `OfficeArtFOPT` and its secondary/tertiary tables
/// ([MS-ODRAW] §2.3).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Props {
    /// `pib`: 1-based index of the picture in the BLIP store.
    pub(crate) pib: Option<u32>,
    /// Rotation, degrees clockwise.
    pub(crate) rot: f32,
    /// Crop fractions (left, top, right, bottom).
    pub(crate) crop: [f32; 4],
    pub(crate) fill_color: Option<Rgb>,
    pub(crate) filled: Option<bool>,
    pub(crate) line_color: Option<Rgb>,
    /// Line width in EMUs.
    pub(crate) line_width: Option<u32>,
    pub(crate) line: Option<bool>,
    /// An arrowhead at either end of a line.
    pub(crate) arrowhead: bool,
    /// `posh`/`posrelh`/`posv`/`posrelv`: alignment and reference frame (Word 2000 and later).
    pub(crate) posh: Option<u32>,
    pub(crate) posrelh: Option<u32>,
    pub(crate) posv: Option<u32>,
    pub(crate) posrelv: Option<u32>,
    /// Distance from wrapped text (left, top, right, bottom), EMUs.
    pub(crate) wrap_dist: [Option<u32>; 4],
    /// `wzDescription`: the alternative text.
    pub(crate) alt: String,
}

impl Props {
    /// Apply one property table (`OfficeArtFOPT` or a secondary/tertiary one): `count` 6-byte
    /// entries, then the complex data of the entries flagged complex, in order.
    pub(crate) fn apply(&mut self, body: &[u8], count: u16) {
        let n = (count as usize).min(body.len() / 6);
        let mut complex_at = n * 6;
        for i in 0..n {
            let Some(opid) = u16_at(body, i * 6) else { break };
            let Some(op) = u32_at(body, i * 6 + 2) else { break };
            let pid = opid & 0x3FFF;
            if opid & 0x8000 != 0 {
                let start = complex_at;
                complex_at = complex_at.saturating_add(op as usize);
                if pid == 0x0381 {
                    let end = complex_at.min(body.len());
                    if let Some(data) = body.get(start..end) {
                        let units: Vec<u16> =
                            data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|u| *u != 0).take(4096).collect();
                        self.alt = String::from_utf16_lossy(&units);
                    }
                }
                continue;
            }
            let fixed = |v: u32| (v as i32) as f32 / 65_536.0;
            match pid {
                0x0004 => self.rot = fixed(op),
                0x0100..=0x0103 => {
                    // cropFromTop, cropFromBottom, cropFromLeft, cropFromRight.
                    let slot = [1, 3, 0, 2][(pid - 0x0100) as usize];
                    let v = fixed(op);
                    self.crop[slot] = if v.is_finite() { v.clamp(0.0, 0.99) } else { 0.0 };
                }
                0x0104 => self.pib = Some(op),
                0x0181 => self.fill_color = color(op),
                // fFilled (bit 4) is valid when fUsefFilled (bit 20) is set.
                0x01BF if op & (1 << 20) != 0 => self.filled = Some(op & (1 << 4) != 0),
                0x01BF => {}
                0x01C0 => self.line_color = color(op),
                0x01CB => self.line_width = Some(op),
                0x01D0 | 0x01D1 => self.arrowhead |= op != 0,
                // fLine (bit 3) is valid when fUsefLine (bit 19) is set.
                0x01FF if op & (1 << 19) != 0 => self.line = Some(op & (1 << 3) != 0),
                0x01FF => {}
                0x0384..=0x0387 => self.wrap_dist[(pid - 0x0384) as usize] = Some(op),
                0x038F => self.posh = Some(op),
                0x0390 => self.posrelh = Some(op),
                0x0391 => self.posv = Some(op),
                0x0392 => self.posrelv = Some(op),
                _ => {}
            }
        }
    }
}

/// An `OfficeArtCOLORREF` holding a literal RGB value (palette, scheme and system colour
/// references are not resolved).
fn color(v: u32) -> Option<Rgb> {
    let [r, g, b, flags] = v.to_le_bytes();
    // fPaletteIndex, fSchemeIndex, fSysIndex.
    if flags & 0x19 != 0 {
        return None;
    }
    Some(Rgb(r, g, b))
}

/// One shape (`OfficeArtSpContainer`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Sp {
    pub(crate) spid: u32,
    /// Shape type (`MSOSPT`), from the FSP's instance.
    pub(crate) spt: u16,
    /// FSP flags: fGroup, fChild, fPatriarch, fDeleted, fOleShape, fHaveMaster, fFlipH, fFlipV…
    pub(crate) flags: u32,
    pub(crate) props: Props,
    /// `OfficeArtChildAnchor` (left, top, right, bottom) in the parent group's space.
    pub(crate) child: Option<[i32; 4]>,
    /// `OfficeArtFSPGR`: a group's own coordinate space.
    pub(crate) group_space: Option<[i32; 4]>,
    /// Has an `OfficeArtClientTextbox` (its text is in the text box story).
    pub(crate) textbox: bool,
}

impl Sp {
    pub(crate) fn flip_h(&self) -> bool {
        self.flags & (1 << 6) != 0
    }
    pub(crate) fn flip_v(&self) -> bool {
        self.flags & (1 << 7) != 0
    }
    fn deleted(&self) -> bool {
        self.flags & (1 << 3) != 0
    }
}

/// A shape or a group of shapes.
#[derive(Clone, Debug)]
pub(crate) enum Node {
    Shape(Sp),
    /// A group: its own shape record (FSPGR, anchor) and members.
    Group(Sp, Vec<Node>),
}

impl Node {
    pub(crate) fn sp(&self) -> &Sp {
        match self {
            Node::Shape(sp) | Node::Group(sp, _) => sp,
        }
    }
}

/// Parse one `OfficeArtSpContainer`.
pub(crate) fn shape(body: &[u8], budget: &mut usize) -> Sp {
    let mut sp = Sp::default();
    for rec in children(body, budget) {
        match rec.typ {
            rt::FSP => {
                sp.spt = rec.inst;
                sp.spid = u32_at(rec.body, 0).unwrap_or(0);
                sp.flags = u32_at(rec.body, 4).unwrap_or(0);
            }
            rt::FOPT | rt::SECONDARY_FOPT | rt::TERTIARY_FOPT => sp.props.apply(rec.body, rec.inst),
            rt::FSPGR | rt::CHILD_ANCHOR => {
                let r = [0, 4, 8, 12].map(|o| i32_at(rec.body, o));
                if let [Some(l), Some(t), Some(r), Some(b)] = r {
                    if rec.typ == rt::FSPGR {
                        sp.group_space = Some([l, t, r, b]);
                    } else {
                        sp.child = Some([l, t, r, b]);
                    }
                }
            }
            rt::CLIENT_TEXTBOX => sp.textbox = true,
            _ => {}
        }
    }
    sp
}

/// Parse an `OfficeArtSpgrContainer`: the first shape is the group itself, the rest its members.
fn group(body: &[u8], depth: usize, budget: &mut usize) -> Option<Node> {
    let recs = children(body, budget);
    let mut it = recs.iter();
    let head = it.find(|r| r.typ == rt::SP_CONTAINER).map(|r| shape(r.body, budget))?;
    let mut members = Vec::new();
    for rec in it {
        if let Some(n) = node(rec, depth + 1, budget) {
            members.push(n);
        }
    }
    Some(Node::Group(head, members))
}

/// A shape or group record as a node (`None` for other records, deleted shapes or past the
/// depth cap).
fn node(rec: &Rec, depth: usize, budget: &mut usize) -> Option<Node> {
    match rec.typ {
        rt::SP_CONTAINER => Some(shape(rec.body, budget)).filter(|s| !s.deleted()).map(Node::Shape),
        rt::SPGR_CONTAINER if depth < MAX_DEPTH => group(rec.body, depth, budget),
        _ => None,
    }
}

/// The document's drawings: the BLIP store and the top-level shapes of each drawing by spid.
#[derive(Default)]
pub(crate) struct Content<'a> {
    /// `OfficeArtFBSE` bodies, in BLIP store order (`pib` 1 = the first).
    pub(crate) bstore: Vec<&'a [u8]>,
    /// The main document's shapes (`dgglbl` 0) and the headers' (`dgglbl` 1), by spid.
    pub(crate) main: HashMap<u32, Node>,
    pub(crate) header: HashMap<u32, Node>,
}

/// Parse `OfficeArtContent` (the `fcDggInfo` blob of the Table stream): an
/// `OfficeArtDggContainer` followed by (`dgglbl`, `OfficeArtDgContainer`) drawings.
pub(crate) fn content(blob: &[u8]) -> Content<'_> {
    let mut out = Content::default();
    let mut budget = MAX_RECORDS;
    let Some((dgg, mut at)) = record(blob) else { return out };
    if dgg.typ == rt::DGG_CONTAINER {
        for rec in children(dgg.body, &mut budget) {
            if rec.typ == rt::BSTORE_CONTAINER {
                out.bstore = children(rec.body, &mut budget).into_iter().filter(|r| r.typ == rt::FBSE).map(|r| r.body).take(MAX_BSE).collect();
            }
        }
    }
    for _ in 0..MAX_DRAWINGS {
        let Some(&label) = blob.get(at) else { break };
        let Some((dg, used)) = blob.get(at + 1..).and_then(record) else { break };
        at = at.saturating_add(1 + used);
        if dg.typ != rt::DG_CONTAINER {
            continue;
        }
        let into = if label == 0 { &mut out.main } else { &mut out.header };
        for rec in children(dg.body, &mut budget) {
            if rec.typ != rt::SPGR_CONTAINER {
                continue;
            }
            // The top group: its first shape is the patriarch; the members are the drawing's shapes.
            if let Some(Node::Group(_, members)) = group(rec.body, 0, &mut budget) {
                for n in members {
                    into.entry(n.sp().spid).or_insert(n);
                }
            }
        }
    }
    out
}
