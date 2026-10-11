//! Connectors and the shape properties beyond fill and outline: a shape's drawing id, its text
//! warp, and a connector's ends.
//!
//! A connector (DrawingML `straightConnector1`, `bentConnector3`, `curvedConnector3`) is a line
//! from its frame's top-left to its bottom-right corner (flips swap the corners). Each end may be
//! glued to a connection site of another shape (`a:stCxn` / `a:endCxn`, ECMA-376 §20.1.2.2.36 and
//! §20.1.2.2.13: the shape's drawing id and a site index). When a glued shape moves, [`route`]
//! gives the connector's new frame from its ends.

use serde::{Deserialize, Serialize};

use crate::graphic::PathSeg;
use crate::para::{GroupChild, InlineObject, ShapeKind};
use crate::wordart::TextWarp;

/// The rest of a shape's properties. Empty (the default) for most shapes.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShapeExtra {
    /// The drawing id connectors refer to (`wp:docPr/@id`, `wps:cNvPr/@id`); 0 = none. Kept only
    /// for shapes a connector is glued to.
    #[serde(skip_serializing_if = "is_zero")]
    pub id: u32,
    /// The text's warp (Text Effects › Transform).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warp: Option<TextWarp>,
    /// What a connector's start and end are glued to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<ConnEnd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<ConnEnd>,
    /// Arrowheads at a line's or connector's start (`a:headEnd`) and end (`a:tailEnd`).
    #[serde(skip_serializing_if = "is_false")]
    pub arrow_start: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub arrow_end: bool,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}

impl ShapeExtra {
    pub fn is_empty(&self) -> bool {
        *self == ShapeExtra::default()
    }
}

/// One glued end of a connector: the drawing id of the shape and its connection site index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnEnd {
    pub id: u32,
    pub site: u32,
}

/// Largest connection site index read from a file.
pub const MAX_SITE: u32 = 64;

impl ShapeKind {
    /// A connector (straight, elbow or curved): a line that can be glued to shapes.
    pub fn is_connector(self) -> bool {
        matches!(self, ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector)
    }
    /// A line or connector: drawn as an open path, never filled.
    pub fn is_open(self) -> bool {
        self == ShapeKind::Line || self.is_connector()
    }
}

/// The connection sites of a `kind` shape `w` × `h`, in its own unrotated frame, by index. The
/// order follows the preset shapes' connection lists (for a rectangle: top, left, bottom, right).
pub fn sites(kind: ShapeKind, w: f32, h: f32) -> Vec<(f32, f32)> {
    let (w, h) = (wordcraft_geom::finite(w), wordcraft_geom::finite(h));
    let (cx, cy) = (w / 2.0, h / 2.0);
    match kind {
        ShapeKind::Ellipse => {
            // The four sides, and the points of the ellipse at 45° between them.
            let (dx, dy) = (cx * std::f32::consts::FRAC_1_SQRT_2, cy * std::f32::consts::FRAC_1_SQRT_2);
            vec![(cx, 0.0), (cx - dx, cy - dy), (0.0, cy), (cx - dx, cy + dy), (cx, h), (cx + dx, cy + dy), (w, cy), (cx + dx, cy - dy)]
        }
        ShapeKind::Triangle => vec![(cx, 0.0), (cx / 2.0, cy), (0.0, h), (cx, h), (w, h), (cx + cx / 2.0, cy)],
        ShapeKind::Line | ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => vec![(0.0, 0.0), (w, h)],
        _ => vec![(cx, 0.0), (0.0, cy), (cx, h), (w, cy)],
    }
}

/// Site `site` of a `kind` shape with frame `rect` (x, y, w, h) on the page, turned and flipped by
/// `spin` about its centre. `None` for a site the shape doesn't have.
pub fn site_point(kind: ShapeKind, rect: [f32; 4], spin: wordcraft_geom::Spin, site: u32) -> Option<(f32, f32)> {
    let [x, y, w, h] = rect;
    let (sx, sy) = *sites(kind, w, h).get(site as usize)?;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (px, py) = spin.apply(cx, cy, x + sx, y + sy);
    (px.is_finite() && py.is_finite()).then_some((px, py))
}

/// The site of a `kind` shape (frame `rect`, `spin`) nearest to page point `at`: its index, point
/// and distance.
pub fn nearest_site(kind: ShapeKind, rect: [f32; 4], spin: wordcraft_geom::Spin, at: (f32, f32)) -> Option<(u32, (f32, f32), f32)> {
    let n = sites(kind, rect[2], rect[3]).len() as u32;
    (0..n)
        .filter_map(|i| site_point(kind, rect, spin, i).map(|p| (i, p, ((p.0 - at.0).powi(2) + (p.1 - at.1).powi(2)).sqrt())))
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

/// A connector's frame: top-left corner, size and flips.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// The frame of a connector from `start` to `end`.
pub fn route(start: (f32, f32), end: (f32, f32)) -> Frame {
    let f = wordcraft_geom::finite;
    let (x0, y0, x1, y1) = (f(start.0), f(start.1), f(end.0), f(end.1));
    Frame { x: x0.min(x1), y: y0.min(y1), w: (x1 - x0).abs(), h: (y1 - y0).abs(), flip_h: x1 < x0, flip_v: y1 < y0 }
}

/// A connector's start and end from its frame (the inverse of [`route`]).
pub fn ends(x: f32, y: f32, w: f32, h: f32, flip_h: bool, flip_v: bool) -> ((f32, f32), (f32, f32)) {
    let (xa, xb) = if flip_h { (x + w, x) } else { (x, x + w) };
    let (ya, yb) = if flip_v { (y + h, y) } else { (y, y + h) };
    ((xa, ya), (xb, yb))
}

/// The path of a connector `kind` in a `w` × `h` frame at the origin, from its top-left to its
/// bottom-right corner (flips are the frame's turn): straight, an elbow through the middle (out
/// horizontally, across, in horizontally), or an S-curve. `None` for other kinds.
pub fn connector_segs(kind: ShapeKind, w: f32, h: f32) -> Option<Vec<PathSeg>> {
    let (w, h) = (wordcraft_geom::finite(w), wordcraft_geom::finite(h));
    let mx = w / 2.0;
    Some(match kind {
        ShapeKind::StraightConnector => vec![PathSeg::Move(0.0, 0.0), PathSeg::Line(w, h)],
        ShapeKind::ElbowConnector => vec![PathSeg::Move(0.0, 0.0), PathSeg::Line(mx, 0.0), PathSeg::Line(mx, h), PathSeg::Line(w, h)],
        ShapeKind::CurvedConnector => vec![PathSeg::Move(0.0, 0.0), PathSeg::Cubic(mx, 0.0, mx, h, w, h)],
        _ => return None,
    })
}

/// The direction (unit vector) a line or connector `kind` arrives at its end (`at_end`) or
/// leaves its start, in its unflipped `w` × `h` frame: where an arrowhead points.
pub fn end_direction(kind: ShapeKind, w: f32, h: f32, at_end: bool) -> (f32, f32) {
    let (dx, dy) = match kind {
        ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => {
            if w.abs() > 0.01 || h.abs() < 0.01 {
                (1.0, 0.0)
            } else {
                (0.0, 1.0)
            }
        }
        ShapeKind::Line => (w, -h),
        _ => (w, h),
    };
    let len = (dx * dx + dy * dy).sqrt();
    let (ux, uy) = if len > 1e-6 && len.is_finite() { (dx / len, dy / len) } else { (1.0, 0.0) };
    if at_end { (ux, uy) } else { (-ux, -uy) }
}

/// Re-route the glued connectors among a group's members (they share the group's coordinate
/// space). Returns whether any moved.
pub fn reroute_group(children: &mut [GroupChild]) -> bool {
    let shapes: Vec<(u32, ShapeKind, [f32; 4], wordcraft_geom::Spin)> = children
        .iter()
        .filter_map(|c| match &c.obj {
            InlineObject::Shape { kind, w, h, float, extra, .. } if extra.id != 0 => Some((extra.id, *kind, [c.x, c.y, *w, *h], float.spin())),
            _ => None,
        })
        .collect();
    if shapes.is_empty() {
        return false;
    }
    let find = |e: Option<ConnEnd>| {
        let e = e?;
        let (_, kind, rect, spin) = shapes.iter().find(|s| s.0 == e.id)?;
        site_point(*kind, *rect, *spin, e.site)
    };
    let mut moved = false;
    for c in children.iter_mut() {
        let InlineObject::Shape { kind, w, h, float, extra, .. } = &mut c.obj else { continue };
        if !kind.is_connector() || (extra.start.is_none() && extra.end.is_none()) {
            continue;
        }
        let (s0, e0) = ends(c.x, c.y, *w, *h, float.flip_h, float.flip_v);
        let (s, e) = (find(extra.start).unwrap_or(s0), find(extra.end).unwrap_or(e0));
        let f = route(s, e);
        let same = (f.x - c.x).abs() < 0.01 && (f.y - c.y).abs() < 0.01 && (f.w - *w).abs() < 0.01 && (f.h - *h).abs() < 0.01;
        if same && f.flip_h == float.flip_h && f.flip_v == float.flip_v && float.rot == 0.0 {
            continue;
        }
        c.x = f.x;
        c.y = f.y;
        *w = f.w;
        *h = f.h;
        float.rot = 0.0;
        float.flip_h = f.flip_h;
        float.flip_v = f.flip_v;
        moved = true;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::para::Float;

    fn shape(kind: ShapeKind, w: f32, h: f32, extra: ShapeExtra) -> InlineObject {
        InlineObject::Shape {
            kind,
            w,
            h,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float: Float::default(),
            story: None,
            freeform: None,
            effects: Default::default(),
            extra,
        }
    }

    #[test]
    fn route_and_ends_are_inverse() {
        for (s, e) in [((0.0, 0.0), (10.0, 5.0)), ((10.0, 5.0), (0.0, 0.0)), ((3.0, 9.0), (7.0, 1.0))] {
            let f = route(s, e);
            assert_eq!(ends(f.x, f.y, f.w, f.h, f.flip_h, f.flip_v), (s, e));
        }
    }

    #[test]
    fn group_connectors_follow_their_shapes() {
        let a = ShapeExtra { id: 7, ..Default::default() };
        let b = ShapeExtra { id: 9, ..Default::default() };
        let link = ShapeExtra { start: Some(ConnEnd { id: 7, site: 3 }), end: Some(ConnEnd { id: 9, site: 2 }), ..Default::default() };
        let mut kids = vec![
            GroupChild { x: 0.0, y: 0.0, obj: shape(ShapeKind::Rectangle, 20.0, 10.0, a) },
            GroupChild { x: 100.0, y: 50.0, obj: shape(ShapeKind::Ellipse, 20.0, 10.0, b) },
            GroupChild { x: 0.0, y: 0.0, obj: shape(ShapeKind::ElbowConnector, 1.0, 1.0, link) },
        ];
        assert!(reroute_group(&mut kids));
        // From the rectangle's right side (20, 5) to the ellipse's left side (100, 55).
        let c = &kids[2];
        let InlineObject::Shape { w, h, float, .. } = &c.obj else { panic!() };
        assert_eq!((c.x, c.y, *w, *h, float.flip_h, float.flip_v), (20.0, 5.0, 80.0, 50.0, false, false));
        assert!(!reroute_group(&mut kids), "settled");
        // A connection to a missing id or site keeps that end where it was.
        if let InlineObject::Shape { extra, .. } = &mut kids[2].obj {
            extra.end = Some(ConnEnd { id: 404, site: 0 });
            extra.start = Some(ConnEnd { id: 7, site: 99 });
        }
        assert!(!reroute_group(&mut kids));
    }
}
