//! Connectors and the shape properties beyond fill and outline: a shape's drawing id, its text
//! warp, a rounded rectangle's corner, a picture fill, and a connector's ends.
//!
//! A connector (DrawingML `straightConnector1`, `bentConnector3`, `curvedConnector3`) is a line
//! from its frame's top-left to its bottom-right corner (flips swap the corners). Each end may be
//! glued to a connection site of another shape (`a:stCxn` / `a:endCxn`, ECMA-376 §20.1.2.2.36 and
//! §20.1.2.2.13: the shape's drawing id and a site index). When a glued shape moves, [`route`]
//! gives the connector's new frame from its ends.

use serde::{Deserialize, Serialize};

use crate::graphic::PathSeg;
use crate::para::{InlineObject, ShapeKind};
use crate::wordart::TextWarp;

/// DrawingML outline ends (ST_LineCap); missing and unknown values use flat ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineCap {
    #[default]
    Flat,
    Square,
    Round,
}

impl LineCap {
    /// Read an ST_LineCap value; absent or unrecognized values use the flat default.
    pub fn from_ooxml(value: &str) -> Self {
        match value {
            "sq" => Self::Square,
            "rnd" => Self::Round,
            _ => Self::Flat,
        }
    }

    /// Return the canonical DrawingML spelling used when saving the outline.
    pub fn ooxml(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Square => "sq",
            Self::Round => "rnd",
        }
    }

    /// Whether the cap can be omitted from older-compatible serialized shape properties.
    pub fn is_flat(&self) -> bool {
        *self == Self::Flat
    }
}

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
    /// A rounded rectangle's corner radius (`roundRect`'s `adj`, ECMA-376 §20.1.10.56): in
    /// 100000ths of the shorter side, at most [`MAX_CORNER`]; `None` draws [`DEFAULT_CORNER`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner: Option<u32>,
    /// The outline dash pattern; absent in older saved documents means solid.
    #[serde(skip_serializing_if = "crate::line_dash::LineDash::is_solid")]
    pub dash: crate::line_dash::LineDash,
    /// A picture fill (Shape Fill › Picture, DrawingML's `a:blipFill`): the `Document::media`
    /// key of the picture stretched over the shape's box and clipped to its outline, in place
    /// of the colour fill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// The outline ends; older saved documents and absent DrawingML caps are flat.
    #[serde(skip_serializing_if = "LineCap::is_flat")]
    pub cap: LineCap,
    /// The outline's gradient (`a:ln/a:gradFill`), kept for saving; the outline is drawn in its
    /// main colour (the shape's `stroke`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_fill: Option<crate::fill::Fill>,
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

/// The largest rounded-rectangle corner: half the shorter side, which makes its ends round.
pub const MAX_CORNER: u32 = 50_000;

/// A rounded rectangle's corner radius without its own: 16% of the shorter side.
pub const DEFAULT_CORNER: f32 = 0.16;

/// The corner radius, in points, of a `w` × `h` rounded rectangle with `corner` (see
/// [`ShapeExtra::corner`]).
pub fn corner_radius(corner: Option<u32>, w: f32, h: f32) -> f32 {
    let side = wordcraft_geom::finite(w).min(wordcraft_geom::finite(h)).max(0.0);
    side * corner.map_or(DEFAULT_CORNER, |c| c.min(MAX_CORNER) as f32 / 100_000.0)
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
        // The corners, the legs' midpoints and the hypotenuse's.
        ShapeKind::RightTriangle => vec![(0.0, 0.0), (0.0, cy), (0.0, h), (cx, h), (w, h), (cx, cy)],
        ShapeKind::Line | ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => vec![(0.0, 0.0), (w, h)],
        // The corners, in the order of the presets' connection lists: a hexagon's and an
        // octagon's from the right side round through the bottom, a pentagon's from its top.
        ShapeKind::Hexagon | ShapeKind::Octagon | ShapeKind::Pentagon => {
            let pts = crate::preset::polygon(kind, w, h).unwrap_or_default();
            let at = |i: usize| pts.get(i).map(|(x, y)| (x * w, y * h));
            let order: &[usize] = match kind {
                ShapeKind::Hexagon => &[3, 4, 5, 0, 1, 2],
                ShapeKind::Octagon => &[3, 4, 5, 6, 7, 0, 1, 2],
                _ => &[1, 0, 4, 3, 2],
            };
            order.iter().filter_map(|i| at(*i)).collect()
        }
        // The other polygons: top, left, bottom and right, where the lines through the middle
        // meet the outline.
        ShapeKind::Parallelogram | ShapeKind::Trapezoid | ShapeKind::Plus | ShapeKind::Chevron | ShapeKind::Decagon => {
            let pts: Vec<(f32, f32)> = crate::preset::polygon(kind, w, h).unwrap_or_default().iter().map(|(x, y)| (x * w, y * h)).collect();
            let (top, bottom) = crossings(&pts, cx, true);
            let (left, right) = crossings(&pts, cy, false);
            vec![(cx, top), (left, cy), (cx, bottom), (right, cy)]
        }
        _ => vec![(cx, 0.0), (0.0, cy), (cx, h), (w, cy)],
    }
}

/// Where the line `x = at` (`vertical`) or `y = at` crosses the closed polygon `pts`: the least
/// and greatest crossing along the line (0 and 0 when it misses).
fn crossings(pts: &[(f32, f32)], at: f32, vertical: bool) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for (i, a) in pts.iter().enumerate() {
        let Some(b) = pts.get((i + 1) % pts.len()) else { continue };
        // (along the line, across it) for each end.
        let ((a_on, a_across), (b_on, b_across)) = if vertical { ((a.1, a.0), (b.1, b.0)) } else { ((a.0, a.1), (b.0, b.1)) };
        let (lo_x, hi_x) = (a_across.min(b_across), a_across.max(b_across));
        if at < lo_x || at > hi_x {
            continue;
        }
        let v = if (b_across - a_across).abs() < 1e-6 {
            // The edge lies on the line: both its ends cross it.
            lo = lo.min(a_on.min(b_on));
            hi = hi.max(a_on.max(b_on));
            continue;
        } else {
            a_on + (b_on - a_on) * (at - a_across) / (b_across - a_across)
        };
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if lo.is_finite() && hi.is_finite() { (lo, hi) } else { (0.0, 0.0) }
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

/// Re-route a group's glued connectors. Resolved ends use the shapes' connection sites in child
/// coordinates; free or unresolved ends keep their displayed position after child scaling and spin.
/// Returns whether any moved. Non-groups and connectors needing a non-invertible or display-capped
/// fallback are unchanged.
pub fn reroute_group(group: &mut InlineObject) -> bool {
    let InlineObject::Group { w, h, ch_w, ch_h, children, .. } = group else { return false };
    let fin = wordcraft_geom::finite;
    // Match group_rects: invalid child extents use unit scale; invalid display extents become zero.
    let sx = if fin(*ch_w) > 0.0 { fin(*w) / *ch_w } else { 1.0 };
    let sy = if fin(*ch_h) > 0.0 { fin(*h) / *ch_h } else { 1.0 };
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
        let (s, e, fallback) = match (find(extra.start), find(extra.end)) {
            (Some(s), Some(e)) => (s, e, false),
            (s, e) => {
                if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
                    continue;
                }
                // Rendering scales the rectangle before spinning it. Undo only the group scale
                // so route can rebuild an unrotated child frame without moving fallback ends.
                let (x, y) = (fin(c.x) * sx, fin(c.y) * sy);
                let (cw, ch) = ((fin(*w) * sx).clamp(0.0, 4000.0), (fin(*h) * sy).clamp(0.0, 4000.0));
                let (cx, cy) = (x + cw / 2.0, y + ch / 2.0);
                let spin = float.spin();
                let s0 = spin.apply(cx, cy, x, y);
                let e0 = spin.apply(cx, cy, x + cw, y + ch);
                (s.unwrap_or((s0.0 / sx, s0.1 / sy)), e.unwrap_or((e0.0 / sx, e0.1 / sy)), true)
            }
        };
        if ![s.0, s.1, e.0, e.1].into_iter().all(f32::is_finite) {
            continue;
        }
        let f = route(s, e);
        if ![f.x, f.y, f.w, f.h].into_iter().all(f32::is_finite) {
            continue;
        }
        // group_rects caps displayed child sizes. A larger fallback span cannot be represented
        // by an unrotated frame, so keep the original instead of moving its free/unresolved end.
        if fallback && (f.w * sx > 4000.0 || f.h * sy > 4000.0) {
            continue;
        }
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
    use crate::para::{Float, GroupChild};

    /// Older JSON retains flat ends without adding a property; non-flat values round-trip.
    #[test]
    fn older_shape_json_defaults_to_flat_caps() {
        let extra: ShapeExtra = serde_json::from_str("{}").unwrap();
        assert_eq!(extra.cap, LineCap::Flat);
        assert_eq!(serde_json::to_value(&extra).unwrap(), serde_json::json!({}));
        for cap in [LineCap::Square, LineCap::Round] {
            let extra = ShapeExtra { cap, ..Default::default() };
            assert_eq!(serde_json::from_value::<ShapeExtra>(serde_json::to_value(&extra).unwrap()).unwrap(), extra);
        }
    }

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

    fn group(children: Vec<GroupChild>) -> InlineObject {
        InlineObject::Group { w: 200.0, h: 300.0, ch_w: 200.0, ch_h: 300.0, float: Float::default(), children, canvas: None }
    }

    #[test]
    fn route_and_ends_are_inverse() {
        for (s, e) in [((0.0, 0.0), (10.0, 5.0)), ((10.0, 5.0), (0.0, 0.0)), ((3.0, 9.0), (7.0, 1.0))] {
            let f = route(s, e);
            assert_eq!(ends(f.x, f.y, f.w, f.h, f.flip_h, f.flip_v), (s, e));
        }
    }

    #[test]
    fn corner_radius_follows_the_shorter_side() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(near(corner_radius(None, 200.0, 100.0), 16.0));
        assert!(near(corner_radius(Some(0), 200.0, 100.0), 0.0));
        assert!(near(corner_radius(Some(3_565), 100.0, 200.0), 3.565));
        assert!(near(corner_radius(Some(MAX_CORNER), 200.0, 100.0), 50.0));
        assert!(near(corner_radius(Some(u32::MAX), 200.0, 100.0), 50.0), "capped at a half-circle end");
        assert!(near(corner_radius(Some(10_000), f32::NAN, 100.0), 0.0));
        assert!(near(corner_radius(Some(10_000), -5.0, 100.0), 0.0));
    }

    /// One rerouting case: the free end settles and the attached end keeps its site.
    fn assert_transformed_reroute(kind: ShapeKind, rot: f32, flip_h: bool, flip_v: bool, free: Option<ConnEnd>, free_start: bool) {
        let attached = Some(ConnEnd { id: 7, site: 3 });
        let (start, end) = if free_start { (free, attached) } else { (attached, free) };
        let mut obj = shape(kind, 40.0, 20.0, ShapeExtra { start, end, ..Default::default() });
        let InlineObject::Shape { float, .. } = &mut obj else { panic!() };
        float.rot = rot;
        float.flip_h = flip_h;
        float.flip_v = flip_v;
        // Frame (10, 30, 40, 20), centre (30, 40): mirror, then rotate
        // clockwise in y-down group space. Do not use the routing transform
        // to compute the expectation.
        let sign = if free_start { -1.0 } else { 1.0 };
        let dx = sign * if flip_h { -20.0 } else { 20.0 };
        let dy = sign * if flip_v { -10.0 } else { 10.0 };
        let (sin, cos) = rot.to_radians().sin_cos();
        let expected = (30.0 + dx * cos - dy * sin, 40.0 + dx * sin + dy * cos);
        let kids = vec![
            GroupChild { x: 100.0, y: 200.0, obj: shape(ShapeKind::Rectangle, 40.0, 20.0, ShapeExtra { id: 7, ..Default::default() }) },
            GroupChild { x: 10.0, y: 30.0, obj },
        ];
        let mut group = group(kids);
        assert!(reroute_group(&mut group));
        let InlineObject::Group { children, .. } = &group else { panic!() };
        let c = &children[1];
        let InlineObject::Shape { w, h, float, .. } = &c.obj else { panic!() };
        let (start, end) = ends(c.x, c.y, *w, *h, float.flip_h, float.flip_v);
        let (free, attached) = if free_start { (start, end) } else { (end, start) };
        assert!(
            (free.0 - expected.0).abs() < 0.001 && (free.1 - expected.1).abs() < 0.001,
            "{kind:?}, {rot}, {flip_h}, {flip_v}, {free_start}: {free:?} != {expected:?}"
        );
        assert_eq!(attached, (140.0, 210.0));
        assert_eq!(float.rot, 0.0);
        assert!(!reroute_group(&mut group), "rerouting must settle");
    }

    #[test]
    fn rerouting_preserves_transformed_free_and_unresolved_ends() {
        for kind in [ShapeKind::StraightConnector, ShapeKind::ElbowConnector, ShapeKind::CurvedConnector] {
            for rot in [0.0_f32, 30.0, 90.0, 180.0, 270.0, -90.0, 450.0] {
                for (flip_h, flip_v) in [(false, false), (true, false), (false, true), (true, true)] {
                    for free in [None, Some(ConnEnd { id: 404, site: 0 }), Some(ConnEnd { id: 7, site: 99 })] {
                        for free_start in [false, true] {
                            assert_transformed_reroute(kind, rot, flip_h, flip_v, free, free_start);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rerouting_preserves_both_unresolved_ends() {
        let extra = ShapeExtra { start: Some(ConnEnd { id: 404, site: 0 }), end: Some(ConnEnd { id: 7, site: 99 }), ..Default::default() };
        let mut obj = shape(ShapeKind::StraightConnector, 40.0, 20.0, extra);
        let InlineObject::Shape { float, .. } = &mut obj else { panic!() };
        float.rot = 90.0;
        let kids = vec![
            GroupChild { x: 100.0, y: 200.0, obj: shape(ShapeKind::Rectangle, 40.0, 20.0, ShapeExtra { id: 7, ..Default::default() }) },
            GroupChild { x: 10.0, y: 30.0, obj },
        ];
        let mut group = group(kids);
        assert!(reroute_group(&mut group));
        let InlineObject::Group { children, .. } = &group else { panic!() };
        let c = &children[1];
        let InlineObject::Shape { w, h, float, .. } = &c.obj else { panic!() };
        let (start, end) = ends(c.x, c.y, *w, *h, float.flip_h, float.flip_v);
        assert!((start.0 - 40.0).abs() < 0.001 && (start.1 - 20.0).abs() < 0.001);
        assert!((end.0 - 20.0).abs() < 0.001 && (end.1 - 60.0).abs() < 0.001);
        assert_eq!(float.rot, 0.0);
        assert!(!reroute_group(&mut group));
    }

    #[test]
    fn rerouting_nonuniform_group_preserves_both_unresolved_ends() {
        let extra = ShapeExtra { start: Some(ConnEnd { id: 404, site: 0 }), end: Some(ConnEnd { id: 7, site: 99 }), ..Default::default() };
        let mut obj = shape(ShapeKind::StraightConnector, 40.0, 20.0, extra);
        let InlineObject::Shape { float, .. } = &mut obj else { panic!() };
        float.rot = 90.0;
        let mut group = group(vec![
            GroupChild { x: 100.0, y: 200.0, obj: shape(ShapeKind::Rectangle, 40.0, 20.0, ShapeExtra { id: 7, ..Default::default() }) },
            GroupChild { x: 10.0, y: 30.0, obj },
        ]);
        let InlineObject::Group { w, h, .. } = &mut group else { panic!() };
        (*w, *h) = (400.0, 150.0);
        assert!(reroute_group(&mut group));
        let InlineObject::Group { children, .. } = &group else { panic!() };
        let c = &children[1];
        let InlineObject::Shape { w, h, float, .. } = &c.obj else { panic!() };
        let (start, end) = ends(c.x, c.y, *w, *h, float.flip_h, float.flip_v);
        // Displayed rect (20, 15, 80, 10), spun 90 degrees: (65, -20) to (55, 60).
        assert!((start.0 - 32.5).abs() < 0.001 && (start.1 + 40.0).abs() < 0.001);
        assert!((end.0 - 27.5).abs() < 0.001 && (end.1 - 120.0).abs() < 0.001);
        assert!(!reroute_group(&mut group));
    }

    #[test]
    fn rerouting_preserves_fallback_when_group_display_cap_prevents_route() {
        for free_start in [false, true] {
            let attached = Some(ConnEnd { id: 7, site: 3 });
            let (start, end) = if free_start { (None, attached) } else { (attached, None) };
            let mut obj = shape(ShapeKind::StraightConnector, 40.0, 0.0, ShapeExtra { start, end, ..Default::default() });
            let InlineObject::Shape { float, .. } = &mut obj else { panic!() };
            float.flip_h = free_start;
            let mut group = group(vec![
                GroupChild { x: 0.0, y: 0.0, obj: shape(ShapeKind::Rectangle, 40.0, 20.0, ShapeExtra { id: 7, ..Default::default() }) },
                GroupChild { x: 3000.0, y: 10.0, obj },
            ]);
            let InlineObject::Group { w, .. } = &mut group else { panic!() };
            *w = 400.0;
            let visual_free = |group: &InlineObject| {
                let rects = group.group_rects(0.0, 0.0, 400.0, 300.0);
                let ([x, y, w, h], obj) = rects[1];
                let spin = obj.frame().unwrap().2.spin();
                let (px, py) = if free_start { (x, y) } else { (x + w, y + h) };
                spin.apply(x + w / 2.0, y + h / 2.0, px, py)
            };
            let before = visual_free(&group);
            assert_eq!(before, (6080.0, 10.0));
            let moved = reroute_group(&mut group);
            assert_eq!(visual_free(&group), before, "the rebuilt display frame cannot span both endpoints");
            assert!(!moved, "an unrepresentable fallback route must leave the connector unchanged");
            let InlineObject::Group { children, .. } = &group else { panic!() };
            let c = &children[1];
            let InlineObject::Shape { w, h, float, .. } = &c.obj else { panic!() };
            assert_eq!((c.x, c.y, *w, *h, float.rot, float.flip_h), (3000.0, 10.0, 40.0, 0.0, 0.0, free_start));
            assert!(!reroute_group(&mut group), "repeated rerouting must not shrink the connector");
        }
    }

    #[test]
    fn rerouting_group_handles_degenerate_dimensions_without_nonfinite_writes() {
        for dims in [
            [0.0, 300.0, 200.0, 300.0],
            [200.0, 0.0, 200.0, 300.0],
            [-200.0, 300.0, 200.0, 300.0],
            [f32::NAN, 300.0, 200.0, 300.0],
            [200.0, f32::INFINITY, 200.0, 300.0],
            [f32::MAX, 300.0, f32::MIN_POSITIVE, 300.0],
            [f32::MIN_POSITIVE, 300.0, f32::MAX, 300.0],
            [20000.0, f32::MIN_POSITIVE, 200.0, 300.0],
            [200.0, 300.0, 0.0, 0.0],
            [200.0, 300.0, f32::NAN, f32::INFINITY],
            [200.0, 300.0, -200.0, -300.0],
        ] {
            let mut obj =
                shape(ShapeKind::StraightConnector, 40.0, 20.0, ShapeExtra { start: Some(ConnEnd { id: 7, site: 3 }), ..Default::default() });
            let InlineObject::Shape { float, .. } = &mut obj else { panic!() };
            float.rot = 90.0;
            let mut group = group(vec![
                GroupChild { x: 100.0, y: 200.0, obj: shape(ShapeKind::Rectangle, 40.0, 20.0, ShapeExtra { id: 7, ..Default::default() }) },
                GroupChild { x: 10.0, y: 30.0, obj },
            ]);
            let InlineObject::Group { w, h, ch_w, ch_h, .. } = &mut group else { panic!() };
            (*w, *h, *ch_w, *ch_h) = (dims[0], dims[1], dims[2], dims[3]);
            let moved = reroute_group(&mut group);
            // group_rects treats invalid child extents as unit scale; display scales that
            // cannot be inverted (or whose inverse overflows) must not change the connector.
            assert_eq!(moved, dims[2] <= 0.0 || !dims[2].is_finite(), "{dims:?}");
            let InlineObject::Group { children, .. } = &group else { panic!() };
            let c = &children[1];
            let InlineObject::Shape { w, h, float, .. } = &c.obj else { panic!() };
            assert!([c.x, c.y, *w, *h, float.rot].into_iter().all(f32::is_finite), "{dims:?}");
            assert!(!reroute_group(&mut group), "rerouting must settle: {dims:?}");
        }
        assert!(!reroute_group(&mut shape(ShapeKind::Rectangle, 40.0, 20.0, ShapeExtra::default())));
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
        // Fully resolved endpoints must still come from their sites, regardless of the
        // connector's old rotation and flips.
        let InlineObject::Shape { float, .. } = &mut kids[2].obj else { panic!() };
        float.rot = 90.0;
        float.flip_h = true;
        float.flip_v = true;
        let mut group = group(kids);
        assert!(reroute_group(&mut group));
        // From the rectangle's right side (20, 5) to the ellipse's left side (100, 55).
        let InlineObject::Group { children, .. } = &group else { panic!() };
        let c = &children[2];
        let InlineObject::Shape { w, h, float, .. } = &c.obj else { panic!() };
        assert_eq!((c.x, c.y, *w, *h, float.flip_h, float.flip_v), (20.0, 5.0, 80.0, 50.0, false, false));
        assert!(!reroute_group(&mut group), "settled");
        // A fully resolved connector does not need an invertible group scale.
        for (width, height) in [(400.0, 150.0), (0.0, 0.0), (f32::NAN, f32::INFINITY)] {
            let mut scaled = group.clone();
            let InlineObject::Group { w, h, children, .. } = &mut scaled else { panic!() };
            (*w, *h) = (width, height);
            let InlineObject::Shape { float, .. } = &mut children[2].obj else { panic!() };
            float.rot = 90.0;
            assert!(reroute_group(&mut scaled));
            let InlineObject::Group { children, .. } = &scaled else { panic!() };
            let c = &children[2];
            let InlineObject::Shape { w, h, float, .. } = &c.obj else { panic!() };
            assert_eq!(ends(c.x, c.y, *w, *h, float.flip_h, float.flip_v), ((20.0, 5.0), (100.0, 55.0)));
            assert!(!reroute_group(&mut scaled));
        }
        // A connection to a missing id or site keeps that end where it was.
        let InlineObject::Group { children, .. } = &mut group else { panic!() };
        if let InlineObject::Shape { extra, .. } = &mut children[2].obj {
            extra.end = Some(ConnEnd { id: 404, site: 0 });
            extra.start = Some(ConnEnd { id: 7, site: 99 });
        }
        assert!(!reroute_group(&mut group));
    }

    /// #1534 review: the polygonal presets' connection sites lie on their outlines; a hexagon,
    /// octagon and pentagon have their DrawingML sites (the corners, from the right or top).
    #[test]
    fn polygon_presets_have_sites_on_their_outline() {
        let on_outline = |kind: ShapeKind, (x, y): (f32, f32)| {
            let pts = crate::preset::polygon(kind, 100.0, 60.0).unwrap();
            let n = pts.len();
            (0..n).any(|i| {
                let ((ax, ay), (bx, by)) = (pts[i], pts[(i + 1) % n]);
                let (ax, ay, bx, by) = (ax * 100.0, ay * 60.0, bx * 100.0, by * 60.0);
                let len = (bx - ax).hypot(by - ay);
                let cross = ((bx - ax) * (y - ay) - (by - ay) * (x - ax)).abs();
                let within = (x - ax) * (x - bx) <= 0.01 && (y - ay) * (y - by) <= 0.01;
                within && cross / len.max(1e-6) < 0.01
            })
        };
        for kind in [
            ShapeKind::Hexagon,
            ShapeKind::Octagon,
            ShapeKind::Pentagon,
            ShapeKind::Parallelogram,
            ShapeKind::Trapezoid,
            ShapeKind::Plus,
            ShapeKind::Chevron,
            ShapeKind::Decagon,
        ] {
            let s = sites(kind, 100.0, 60.0);
            assert!(s.len() >= 4, "{kind:?}");
            for p in &s {
                assert!(on_outline(kind, *p), "{kind:?}: {p:?} is off the outline");
            }
        }
        // A hexagon's six corners, from the right one round through the bottom.
        let hex = sites(ShapeKind::Hexagon, 100.0, 60.0);
        assert_eq!(hex.len(), 6);
        assert_eq!(hex[0], (100.0, 30.0));
        assert!(site_point(ShapeKind::Hexagon, [0.0, 0.0, 100.0, 60.0], Default::default(), 5).is_some(), "a glued site 5 resolves");
        assert_eq!(sites(ShapeKind::Octagon, 100.0, 60.0).len(), 8);
        assert_eq!(sites(ShapeKind::Pentagon, 100.0, 60.0)[0], (50.0, 0.0));
        // Junk sizes don't panic.
        for kind in [ShapeKind::Hexagon, ShapeKind::Plus] {
            let _ = sites(kind, f32::NAN, -5.0);
            let _ = sites(kind, 0.0, 0.0);
        }
    }
}
