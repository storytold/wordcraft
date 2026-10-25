//! Floating shapes and pictures: the `PlcfSpa` anchors of the main document and the headers
//! ([MS-DOC] §2.8.27 PlcfSpa, §2.9.253 Spa) joined with the OfficeArt shapes they name, and the
//! text box stories (`PlcftxbxTxt`, §2.8.30) that fill text boxes.

use std::collections::HashMap;

use wordcraft_doc::fill::Fill;
use wordcraft_doc::para::{Anchor, Float, FloatAlign, GroupChild, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::props::Rgb;

use crate::media::Pictures;
use crate::officeart::{Node, Sp};

/// Most anchors (and text box stories) read per PLC.
const MAX_ANCHORS: usize = 10_000;
/// `Spa` and `FTXBXS` record sizes.
const CB_SPA: usize = 26;
const CB_FTXBXS: usize = 22;
/// Largest frame or offset kept (points; 22 inches, Word's own limit for sizes).
const MAX_PT: f32 = 1584.0;
/// Deepest group nesting flattened.
const MAX_GROUP_DEPTH: usize = 8;

/// The (CP, data) entries of a PLC with `cb`-byte data elements, the trailing CP dropped.
fn plc(table: &[u8], (fc, lcb): (u32, u32), cb: usize) -> Vec<(u32, &[u8])> {
    let Some(blob) = table.get(fc as usize..(fc as usize).saturating_add(lcb as usize)) else { return Vec::new() };
    let n = blob.len().saturating_sub(4) / (4 + cb);
    let n = n.min(MAX_ANCHORS);
    let data_at = (n + 1) * 4;
    (0..n)
        .filter_map(|i| {
            let cp = u32::from_le_bytes(blob.get(i * 4..i * 4 + 4)?.try_into().ok()?);
            let at = data_at + i * cb;
            Some((cp, blob.get(at..at + cb)?))
        })
        .collect()
}

/// The text box stories of a `PlcftxbxTxt` (or `PlcfHdrtxbxTxt`): (shape spid, story CP range
/// relative to the text box subdocument).
///
/// The CPs of a well-formed PLC ascend, so the stories are disjoint. A story starting before
/// the end of the last one kept (a hostile table pointing many text boxes at the same text,
/// each read into its own part) is dropped: every subdocument CP becomes at most one story.
pub(crate) fn textbox_stories(table: &[u8], pair: Option<(u32, u32)>) -> Vec<(u32, (u32, u32))> {
    let Some(pair) = pair else { return Vec::new() };
    let Some(blob) = table.get(pair.0 as usize..(pair.0 as usize).saturating_add(pair.1 as usize)) else { return Vec::new() };
    let mut out = Vec::new();
    let mut kept_to = 0u32;
    for (i, (cp, ftxbxs)) in plc(table, pair, CB_FTXBXS).iter().enumerate() {
        // The story runs to the next entry's CP (the PLC's last CP for the last entry).
        let Some(end) = blob.get((i + 1) * 4..(i + 1) * 4 + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes) else { continue };
        let Some(lid) = ftxbxs.get(14..18).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes) else { continue };
        if end > *cp && *cp >= kept_to && lid != 0 {
            out.push((lid, (*cp, end)));
            kept_to = end;
        }
    }
    out
}

/// What building the floating objects needs.
pub(crate) struct Build<'a> {
    /// The BLIP store and the stream holding BLIPs not embedded in it (WordDocument).
    pub(crate) bstore: &'a [&'a [u8]],
    pub(crate) delay: &'a [u8],
    /// Text box part ids by shape spid.
    pub(crate) stories: &'a HashMap<u32, u32>,
}

/// The floating objects anchored by `PlcfSpa` (at `pair`), as (absolute anchor CP, object),
/// sorted by CP. `cp_base` turns the PLC's CPs into absolute ones (the header subdocument's
/// start for `PlcSpaHdr`).
pub(crate) fn anchors(
    table: &[u8],
    pair: Option<(u32, u32)>,
    cp_base: u32,
    shapes: &HashMap<u32, Node>,
    b: &Build,
    pics: &mut Pictures,
) -> Vec<(u32, InlineObject)> {
    let Some(pair) = pair else { return Vec::new() };
    let mut out = Vec::new();
    for (cp, spa) in plc(table, pair, CB_SPA) {
        let i32_at = |o: usize| spa.get(o..o + 4).and_then(|b| b.try_into().ok()).map(i32::from_le_bytes).unwrap_or(0);
        let spid = i32_at(0) as u32;
        let [xl, yt, xr, yb] = [4, 8, 12, 16].map(i32_at);
        let flags = spa.get(20..22).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
        let Some(node) = shapes.get(&spid) else { continue };
        let tw = |v: i64| (v as f32 / 20.0).clamp(-2.0 * MAX_PT, 2.0 * MAX_PT);
        let (mut x, mut y) = (tw(xl as i64), tw(yt as i64));
        let (mut w, mut h) = (tw(xr as i64 - xl as i64).clamp(0.0, MAX_PT), tw(yb as i64 - yt as i64).clamp(0.0, MAX_PT));
        let sp = node.sp();
        let rot = wordcraft_geom::normalize_degrees(if sp.props.rot.is_finite() { sp.props.rot } else { 0.0 });
        // A shape turned by 45°–135° (or 225°–315°) is anchored by its turned bounds: the
        // unrotated frame has the other aspect, about the same centre.
        if (45.0..135.0).contains(&rot) || (225.0..315.0).contains(&rot) {
            x += (w - h) / 2.0;
            y += (h - w) / 2.0;
            std::mem::swap(&mut w, &mut h);
        }
        let mut float = spa_float(flags, &sp.props);
        float.x = x;
        float.y = y;
        float.rot = rot;
        float.flip_h = sp.flip_h();
        float.flip_v = sp.flip_v();
        let obj = match node {
            Node::Shape(sp) => leaf(sp, w, h, float, b, pics),
            Node::Group(sp, members) => group(sp, members, w, h, float, b, pics),
        };
        match obj {
            Some(obj) => out.push((cp_base.saturating_add(cp), obj)),
            None => pics.dropped(cp_base.saturating_add(cp)),
        }
    }
    out.sort_by_key(|(cp, _)| *cp);
    out
}

/// Placement from a `Spa`'s flags (fHdr, bx, by, wr, wrk, fRcaSimple, fBelowText, fAnchorLock)
/// and the shape's position properties, which take precedence when present.
fn spa_float(flags: u16, p: &crate::officeart::Props) -> Float {
    let bx = (flags >> 1) & 3;
    let by = (flags >> 3) & 3;
    let wr = (flags >> 5) & 0xF;
    let below = flags & (1 << 14) != 0;
    let wrap = match wr {
        1 => Wrap::TopAndBottom,
        3 => {
            if below {
                Wrap::BehindText
            } else {
                Wrap::InFrontOfText
            }
        }
        4 => Wrap::Tight,
        5 => Wrap::Through,
        _ => Wrap::Square,
    };
    let h_rel = match p.posrelh.unwrap_or(bx as u32) {
        0 => Anchor::Margin,
        1 => Anchor::Page,
        3 => Anchor::Character,
        _ => Anchor::Column,
    };
    let v_rel = match p.posrelv.unwrap_or(by as u32) {
        0 => Anchor::Margin,
        1 => Anchor::Page,
        3 => Anchor::Line,
        _ => Anchor::Paragraph,
    };
    let align = |v: Option<u32>| match v {
        Some(1) => Some(FloatAlign::Start),
        Some(2) => Some(FloatAlign::Center),
        Some(3) => Some(FloatAlign::End),
        Some(4) => Some(FloatAlign::Inside),
        Some(5) => Some(FloatAlign::Outside),
        _ => None,
    };
    // Distances from text, EMUs; Word's defaults are 0.125" at the sides, none above and below.
    let emu = |v: Option<u32>, def: f32| v.map_or(def, |e| (e as f32 / 12_700.0).clamp(0.0, MAX_PT));
    let [dl, dt, dr, db] = p.wrap_dist;
    Float {
        wrap,
        h_rel,
        v_rel,
        h_align: align(p.posh),
        v_align: align(p.posv),
        dist: emu(dl, 9.0).max(emu(dr, 9.0)),
        dist_top: emu(dt, 0.0),
        dist_bottom: emu(db, 0.0),
        ..Float::default()
    }
}

/// One shape as a picture or a drawn shape, `w` × `h` points.
fn leaf(sp: &Sp, w: f32, h: f32, float: Float, b: &Build, pics: &mut Pictures) -> Option<InlineObject> {
    let p = &sp.props;
    if let Some(pib) = p.pib {
        let media = pics.bse(pib, b.bstore, b.delay)?;
        return Some(InlineObject::Image { media, w, h, alt: p.alt.clone(), float, crop: p.crop, ole: None });
    }
    let story = b.stories.get(&sp.spid).copied().filter(|_| sp.textbox || sp.spt == 202);
    let mut kind = match sp.spt {
        1 => ShapeKind::Rectangle,
        2 => ShapeKind::RoundedRectangle,
        3 => ShapeKind::Ellipse,
        4 => ShapeKind::Diamond,
        5 => ShapeKind::Triangle,
        6 => ShapeKind::RightTriangle,
        12 | 187 => ShapeKind::Star,
        13 | 69 => ShapeKind::Arrow,
        66 => ShapeKind::ArrowLeft,
        67 => ShapeKind::ArrowDown,
        68 => ShapeKind::ArrowUp,
        20 | 32 => ShapeKind::Line,
        74 => ShapeKind::Heart,
        202 => ShapeKind::TextBox,
        // A freeform's path is not read: better left out than drawn as a box.
        0 => return None,
        _ => ShapeKind::Rectangle,
    };
    if story.is_some() && kind == ShapeKind::Rectangle {
        kind = ShapeKind::TextBox;
    }
    let line = kind == ShapeKind::Line;
    let fill = if line || p.filled == Some(false) { None } else { Some(Fill::solid(p.fill_color.unwrap_or(Rgb::WHITE))) };
    let stroke = if p.line == Some(false) { None } else { Some(p.line_color.unwrap_or(Rgb::BLACK)) };
    // Line widths are EMUs; Word's default is 0.75 pt.
    let stroke_width = p.line_width.map_or(0.75, |e| (e as f32 / 12_700.0).clamp(0.0, 100.0));
    Some(InlineObject::Shape {
        kind,
        w,
        h,
        fill,
        stroke,
        stroke_width,
        float,
        story,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    })
}

/// A group: its members flattened into the group's own coordinate space (nested groups mapped
/// through their child anchors).
fn group(sp: &Sp, members: &[Node], w: f32, h: f32, float: Float, b: &Build, pics: &mut Pictures) -> Option<InlineObject> {
    let [l, t, r, btm] = sp.group_space.unwrap_or([0, 0, 0, 0]);
    let (cw, ch) = ((r as f64 - l as f64) as f32, (btm as f64 - t as f64) as f32);
    let (ch_w, ch_h) = if cw > 0.0 && ch > 0.0 { (cw, ch) } else { (w.max(1.0), h.max(1.0)) };
    let mut children = Vec::new();
    // Map: outer = x0 + inner × s; the top group's space starts at its FSPGR origin.
    members_into(members, (-(l as f32), -(t as f32), 1.0, 1.0), 0, b, pics, &mut children);
    if children.is_empty() {
        return None;
    }
    Some(InlineObject::Group { w, h, float, ch_w, ch_h, children, canvas: None })
}

fn members_into(nodes: &[Node], t: (f32, f32, f32, f32), depth: usize, b: &Build, pics: &mut Pictures, out: &mut Vec<GroupChild>) {
    let (x0, y0, sx, sy) = t;
    let fit = |v: f32| if v.is_finite() { v.clamp(-1e7, 1e7) } else { 0.0 };
    for n in nodes {
        if out.len() >= wordcraft_doc::para::MAX_GROUP_CHILDREN {
            return;
        }
        let sp = n.sp();
        let Some([l, t, r, btm]) = sp.child else { continue };
        let (ox, oy) = (l as f32, t as f32);
        let (ew, eh) = ((r as f64 - l as f64) as f32, (btm as f64 - t as f64) as f32);
        match n {
            Node::Group(_, members) => {
                if depth >= MAX_GROUP_DEPTH {
                    continue;
                }
                let [gl, gt, gr, gb] = sp.group_space.unwrap_or([l, t, r, btm]);
                let (gw, gh) = ((gr as f64 - gl as f64) as f32, (gb as f64 - gt as f64) as f32);
                let (kx, ky) = if gw > 0.0 && gh > 0.0 { (ew / gw, eh / gh) } else { (1.0, 1.0) };
                let nt = (fit(x0 + (ox - gl as f32 * kx) * sx), fit(y0 + (oy - gt as f32 * ky) * sy), fit(sx * kx), fit(sy * ky));
                members_into(members, nt, depth + 1, b, pics, out);
            }
            Node::Shape(sp) => {
                let float = Float { rot: sp.props.rot, flip_h: sp.flip_h(), flip_v: sp.flip_v(), ..Float::default() };
                let (w, h) = (fit(ew * sx).max(0.0), fit(eh * sy).max(0.0));
                if let Some(obj) = leaf(sp, w, h, float, b, pics) {
                    out.push(GroupChild { x: fit(x0 + ox * sx), y: fit(y0 + oy * sy), obj });
                }
            }
        }
    }
}
