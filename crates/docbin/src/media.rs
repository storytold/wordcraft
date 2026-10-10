//! Inline pictures: `sprmCPicLocation` → a `PICF` in the Data stream → embedded image bytes
//! ([MS-DOC] §2.9.190 PICF, §1.3.5 Pictures).

/// `PICF.cbHeader` — the fixed size of the PICF structure.
const CB_HEADER: usize = 0x44;
/// Offsets inside PICF: `PICMID.dxaGoal/dyaGoal/mx/my` start at 28.
const OFF_PICMID: usize = 28;

/// One decoded picture: display size in points and the image bytes (format-sniffed).
pub(crate) struct Picture {
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) bytes: Vec<u8>,
    pub(crate) ext: &'static str,
}

/// Read the picture at `fc_pic` in the Data stream. `None` when the header is malformed or
/// the payload is a format we do not embed (metafiles), which the caller drops with a warn.
pub(crate) fn read(data: &[u8], fc_pic: u32) -> Option<Picture> {
    let pic = data.get(fc_pic as usize..)?;
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
    Some(Picture { w, h, bytes: body[at..].to_vec(), ext: ext_of(&body[at..]) })
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
