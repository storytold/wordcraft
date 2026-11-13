//! Inline pictures in the Data stream: a `PICF` header ([MS-DOC] §2.9.193) followed by an
//! `OfficeArtInlineSpContainer` ([MS-ODRAW]): the picture-frame shape, its properties (the
//! picture's place in the BLIP store, crop, alt text) and the `OfficeArtFBSE` with the BLIP
//! embedded in it. The character 0x01 in the text points here through `sprmCPicLocation`.

use sha1::Digest;

/// `PICF.cbHeader`.
const CB_HEADER: u16 = 0x44;
/// `MM_SHAPE`: the picture is the OfficeArt shape that follows the header.
const MM_SHAPE: u16 = 0x0064;
/// Largest picture written (bytes of its BLIP): Word's own limit for one picture is far above
/// what the Data stream's 32-bit offsets allow many of.
const MAX_PICTURE: usize = 64 << 20;
/// All pictures together: `sprmCPicLocation` offsets are 32-bit, keep well inside them.
const MAX_DATA: usize = 1 << 30;

/// OfficeArt record types and BLIP instances ([MS-ODRAW] §2.2).
const SP_CONTAINER: u16 = 0xF004;
const FSP: u16 = 0xF00A;
const FOPT: u16 = 0xF00B;
const FBSE: u16 = 0xF007;
const BLIP_EMF: (u16, u16) = (0xF01A, 0x3D4);
const BLIP_WMF: (u16, u16) = (0xF01B, 0x216);
const BLIP_JPEG: (u16, u16) = (0xF01D, 0x46A);
const BLIP_PNG: (u16, u16) = (0xF01E, 0x6E0);
/// `msosptPictureFrame`.
const SPT_PICTURE_FRAME: u16 = 75;

/// The Data stream being built.
#[derive(Default)]
pub(crate) struct Pictures {
    pub(crate) data: Vec<u8>,
}

/// A picture in a form the BLIP store holds, with its `btWin32` type.
enum Blip {
    Png(Vec<u8>),
    Jpeg(Vec<u8>),
    /// EMF (true) or WMF metafile bytes.
    Metafile(Vec<u8>, bool),
}

impl Blip {
    /// Sniff `bytes`: PNG and JPEG go in as they are, metafiles compressed, other raster
    /// formats (GIF, BMP, TIFF, WebP) re-encoded as PNG.
    fn of(bytes: &[u8]) -> Option<Blip> {
        if bytes.starts_with(b"\x89PNG") {
            return Some(Blip::Png(bytes.to_vec()));
        }
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some(Blip::Jpeg(bytes.to_vec()));
        }
        // EMF: an EMR_HEADER record (type 1) with " EMF" at offset 40.
        if bytes.get(0..4) == Some(&[1, 0, 0, 0]) && bytes.get(40..44) == Some(b" EMF") {
            return Some(Blip::Metafile(bytes.to_vec(), true));
        }
        // WMF: the placeable header's key, or a plain header (type 1 or 2, header size 9).
        if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || (matches!(bytes.get(0..4), Some([1 | 2, 0, 9, 0]))) {
            // The BLIP holds the metafile without the placeable header.
            let body = if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) { bytes.get(22..)? } else { bytes };
            return Some(Blip::Metafile(body.to_vec(), false));
        }
        let img = image::load_from_memory(bytes).ok()?;
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(Blip::Png(png))
    }

    /// `btWin32` ([MS-ODRAW] §2.4.1 MSOBLIPTYPE).
    fn bt(&self) -> u8 {
        match self {
            Blip::Png(_) => 6,
            Blip::Jpeg(_) => 5,
            Blip::Metafile(_, true) => 2,
            Blip::Metafile(_, false) => 3,
        }
    }

    /// The `OfficeArtBlip*` record.
    fn record(&self, w_emu: i32, h_emu: i32) -> Vec<u8> {
        let (raw, (typ, inst)) = match self {
            Blip::Png(b) => (b, BLIP_PNG),
            Blip::Jpeg(b) => (b, BLIP_JPEG),
            Blip::Metafile(b, emf) => (b, if *emf { BLIP_EMF } else { BLIP_WMF }),
        };
        let mut body = uid(raw).to_vec();
        match self {
            Blip::Png(b) | Blip::Jpeg(b) => {
                body.push(0xFF); // tag
                body.extend_from_slice(b);
            }
            Blip::Metafile(b, _) => {
                let packed = miniz_oxide::deflate::compress_to_vec_zlib(b, 6);
                // OfficeArtMetafileHeader: cbSize, rcBounds, ptSize (EMUs), cbSave, compression
                // (0 = deflate), filter (0xFE = none).
                body.extend_from_slice(&(b.len() as u32).to_le_bytes());
                for v in [0i32, 0, w_emu / 12_700, h_emu / 12_700] {
                    body.extend_from_slice(&v.to_le_bytes());
                }
                body.extend_from_slice(&w_emu.to_le_bytes());
                body.extend_from_slice(&h_emu.to_le_bytes());
                body.extend_from_slice(&(packed.len() as u32).to_le_bytes());
                body.push(0);
                body.push(0xFE);
                body.extend_from_slice(&packed);
            }
        }
        rec(0, inst, typ, &body)
    }
}

/// A 16-byte id for a BLIP (the format wants an MD4; any digest identifies it).
fn uid(bytes: &[u8]) -> [u8; 16] {
    let d = sha1::Sha1::digest(bytes);
    let mut u = [0u8; 16];
    u.copy_from_slice(d.get(..16).unwrap_or(&[0; 16]));
    u
}

/// An OfficeArt record: header (version, instance, type, length) then the body.
fn rec(ver: u16, inst: u16, typ: u16, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + body.len());
    v.extend_from_slice(&((ver & 0xF) | (inst << 4)).to_le_bytes());
    v.extend_from_slice(&typ.to_le_bytes());
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    v
}

/// A 16.16 fixed-point fraction.
fn fixed(f: f32) -> u32 {
    ((f.clamp(0.0, 0.99) * 65_536.0).round() as i32) as u32
}

impl Pictures {
    /// Add a picture shown `w` × `h` points with `crop` (left, top, right, bottom fractions):
    /// the offset of its `PICF` in the Data stream, `None` when the picture can't be stored.
    pub(crate) fn add(&mut self, bytes: &[u8], w: f32, h: f32, crop: [f32; 4], alt: &str) -> Option<u32> {
        if bytes.len() > MAX_PICTURE {
            return None;
        }
        let blip = Blip::of(bytes)?;
        let [cl, ct, cr, cb] = crop.map(|c| if c.is_finite() { c.clamp(0.0, 0.99) } else { 0.0 });
        // The goal size is the uncropped picture's: the crop takes its share off it.
        let gw = (w.max(1.0) / (1.0 - cl - cr).max(0.01)).min(1584.0);
        let gh = (h.max(1.0) / (1.0 - ct - cb).max(0.01)).min(1584.0);
        let emu = |pt: f32| (pt * 12_700.0).round() as i32;

        let blip_rec = blip.record(emu(gw), emu(gh));
        let mut fbse = vec![blip.bt(), blip.bt()];
        fbse.extend_from_slice(&uid(bytes));
        fbse.extend_from_slice(&0x00FFu16.to_le_bytes()); // tag
        fbse.extend_from_slice(&(blip_rec.len() as u32).to_le_bytes());
        fbse.extend_from_slice(&1u32.to_le_bytes()); // cRef
        fbse.extend_from_slice(&0u32.to_le_bytes()); // foDelay: embedded
        fbse.extend_from_slice(&[0, 0, 0, 0]); // unused1, cbName, unused2, unused3
        fbse.extend_from_slice(&blip_rec);
        let fbse = rec(2, u16::from(blip.bt()), FBSE, &fbse);

        // The shape: an FSP (spid, fHaveAnchor | fHaveSpt) and its properties.
        let mut fsp = 1025u32.to_le_bytes().to_vec();
        fsp.extend_from_slice(&0x0A00u32.to_le_bytes());
        let fsp = rec(2, SPT_PICTURE_FRAME, FSP, &fsp);
        let mut props: Vec<(u16, u32)> = Vec::new();
        for (pid, f) in [(0x0100u16, ct), (0x0101, cb), (0x0102, cl), (0x0103, cr)] {
            if f > 0.0 {
                props.push((pid, fixed(f)));
            }
        }
        props.push((0x4104, 1)); // pib, fBid: the first (only) store entry
        let alt_units: Vec<u16> = alt.encode_utf16().take(4095).chain(std::iter::once(0)).collect();
        let has_alt = alt_units.len() > 1;
        if has_alt {
            props.push((0x8381, (alt_units.len() * 2) as u32)); // wzDescription, complex
        }
        let mut fopt = Vec::new();
        for (pid, v) in &props {
            fopt.extend_from_slice(&pid.to_le_bytes());
            fopt.extend_from_slice(&v.to_le_bytes());
        }
        if has_alt {
            for u in &alt_units {
                fopt.extend_from_slice(&u.to_le_bytes());
            }
        }
        let fopt = rec(3, props.len() as u16, FOPT, &fopt);
        let mut sp = fsp;
        sp.extend_from_slice(&fopt);
        let sp = rec(0xF, 0, SP_CONTAINER, &sp);

        let lcb = usize::from(CB_HEADER) + sp.len() + fbse.len();
        let at = self.data.len();
        if at.checked_add(lcb)? > MAX_DATA {
            return None;
        }
        let mut picf = vec![0u8; usize::from(CB_HEADER)];
        picf[0..4].copy_from_slice(&(lcb as u32).to_le_bytes());
        picf[4..6].copy_from_slice(&CB_HEADER.to_le_bytes());
        picf[6..8].copy_from_slice(&MM_SHAPE.to_le_bytes());
        // PICMID: dxaGoal, dyaGoal, mx, my (1000 = 100 %).
        picf[28..30].copy_from_slice(&super::grpprl::twips(gw).to_le_bytes());
        picf[30..32].copy_from_slice(&super::grpprl::twips(gh).to_le_bytes());
        picf[32..34].copy_from_slice(&1000u16.to_le_bytes());
        picf[34..36].copy_from_slice(&1000u16.to_le_bytes());
        self.data.extend_from_slice(&picf);
        self.data.extend_from_slice(&sp);
        self.data.extend_from_slice(&fbse);
        u32::try_from(at).ok()
    }
}
