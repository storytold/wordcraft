use super::options::DisplayOptions;
use super::paint::{Paint, paint_for};
use super::{BezPath, Document, Draw, Graphic, GraphicItem, GraphicKind, InlineObject, PathSeg, Rect, Rgb, ShapeKind, Spin, TextAlign};

/// The draws of a chart or diagram's items inside `rect` (page coordinates). The items were built
/// for the size `g.w` × `g.h` (0: `rect`'s own size), so they are scaled to `rect`; strokes and text
/// by the mean of the two scales.
pub(super) fn graphic_draws(doc: &Document, g: &Graphic, rect: Rect, alpha: f32) -> Vec<Draw> {
    let (sx, sy) = (scale_of(rect.w, g.w), scale_of(rect.h, g.h));
    let k = (sx + sy) / 2.0;
    let inside = |r: &[f32; 4]| Rect::new(rect.x + r[0] * sx, rect.y + r[1] * sy, r[2] * sx, r[3] * sy);
    let mut out = Vec::with_capacity(g.items.len());
    for it in &g.items {
        match it {
            GraphicItem::Shape { rect: r, kind, fill, stroke, stroke_width } => out.push(Draw::Shape {
                rect: inside(r),
                kind: *kind,
                fill: fill.map(Paint::solid),
                stroke: *stroke,
                stroke_width: stroke_width * k,
                effects: Default::default(),
                corner: None,
                dash: Vec::new(),
                cap: Default::default(),
            }),
            GraphicItem::Path { segs, fill, stroke, stroke_width } => {
                let segs = page_segs(segs, |x, y| (rect.x + x * sx, rect.y + y * sy));
                out.push(Draw::Path {
                    segs,
                    fill: fill.map(Paint::solid),
                    stroke: *stroke,
                    stroke_width: stroke_width * k,
                    dash: Vec::new(),
                    cap: Default::default(),
                })
            }
            GraphicItem::Text { rect: r, .. } => out.extend(text_glyphs(doc, inside(r), it, alpha, k)),
            GraphicItem::Image { rect: r, media } => {
                out.push(Draw::Image { rect: inside(r), media: media.clone(), crop: [0.0; 4], alpha, alt: String::new(), decorative: false })
            }
        }
    }
    out
}

/// Scale from the size `built` points a graphic's items were made for to `len` points; 1 when
/// the built size is unknown (0).
fn scale_of(len: f32, built: f32) -> f32 {
    if built.is_finite() && built > 0.0 && len.is_finite() { (len / built).max(0.0) } else { 1.0 }
}

/// Path segments mapped to page coordinates by `map`, cleaned for the rasterizer and PDF: a segment
/// with a non-finite number is dropped, and so is anything before a move (or after a broken one).
fn page_segs(segs: &[PathSeg], map: impl Fn(f32, f32) -> (f32, f32)) -> Vec<PathSeg> {
    let fin = |p: (f32, f32)| p.0.is_finite() && p.1.is_finite();
    let mut out = Vec::with_capacity(segs.len());
    let mut open = false;
    for s in segs {
        match *s {
            PathSeg::Move(x, y) => {
                let p = map(x, y);
                open = fin(p);
                if open {
                    out.push(PathSeg::Move(p.0, p.1));
                }
            }
            PathSeg::Line(x, y) => {
                let p = map(x, y);
                if open && fin(p) {
                    out.push(PathSeg::Line(p.0, p.1));
                }
            }
            PathSeg::Cubic(a, b, c, d, e, f) => {
                let (p1, p2, p3) = (map(a, b), map(c, d), map(e, f));
                if open && fin(p1) && fin(p2) && fin(p3) {
                    out.push(PathSeg::Cubic(p1.0, p1.1, p2.0, p2.1, p3.0, p3.1));
                }
            }
            PathSeg::Close => {
                if open {
                    out.push(PathSeg::Close);
                }
                open = false;
            }
        }
    }
    out
}

/// A path of clean segments (see [`page_segs`]) as a kurbo path, for the rasteriser and PDF.
/// How a shape looks: its preset `kind`, fill, outline, effects, freeform paths and connector extras.
#[derive(Clone, Copy)]
pub(super) struct ShapeLook<'a> {
    pub kind: ShapeKind,
    pub fill: Option<&'a wordcraft_doc::fill::Fill>,
    pub stroke: Option<Rgb>,
    pub stroke_width: f32,
    pub effects: wordcraft_doc::effects::ShapeEffects,
    pub freeform: Option<&'a wordcraft_doc::freeform::Freeform>,
    pub extra: &'a wordcraft_doc::connector::ShapeExtra,
}

/// The draws of a shape at `rect`: a preset outline, or a freeform's paths (an ink stroke left out
/// under Review › Hide Ink).
pub(super) fn shape_draws(rect: Rect, look: &ShapeLook, opts: &DisplayOptions, out: &mut Vec<Draw>) {
    let ShapeLook { kind, fill, stroke, stroke_width, effects, freeform, extra } = *look;
    // Freeforms are painted across the shape's frame, not each path's bounds.
    let fill = fill.and_then(|f| paint_for(f, rect));
    if kind.is_open() {
        line_draws(rect, kind, stroke, stroke_width, effects, extra, out);
        return;
    }
    let Some(f) = freeform.filter(|_| kind == ShapeKind::Freeform) else {
        // A picture fill casts the shape's effects itself, under the picture.
        let effects = match &extra.picture {
            Some(media) => {
                let stroke_width = stroke.map(|_| stroke_width);
                let (corner, media, clip) = (extra.corner, media.clone(), Vec::new());
                out.push(Draw::ShapePicture { rect, kind, corner, media, clip, effects, stroke_width });
                Default::default()
            }
            None => effects,
        };
        out.push(Draw::Shape {
            rect,
            kind,
            fill,
            stroke,
            stroke_width,
            effects,
            corner: extra.corner,
            dash: extra.dash.pattern(stroke_width),
            cap: extra.cap,
        });
        return;
    };
    if f.is_ink() && opts.hide_ink {
        return;
    }
    let width = if stroke_width.is_finite() { stroke_width.clamp(0.0, 200.0) } else { 0.75 };
    // A picture fill inside the closed paths, under their outlines.
    if let Some(media) = extra.picture.as_ref().filter(|_| !f.is_ink()) {
        let mut clip = Vec::new();
        for (pts, _) in f.placed(rect.x, rect.y, rect.w, rect.h).into_iter().filter(|(_, closed)| *closed) {
            for (i, [x, y]) in pts.iter().enumerate() {
                clip.push(if i == 0 { PathSeg::Move(*x, *y) } else { PathSeg::Line(*x, *y) });
            }
            clip.push(PathSeg::Close);
        }
        if !clip.is_empty() {
            let effects = Default::default();
            out.push(Draw::ShapePicture { rect, kind, corner: None, media: media.clone(), clip, effects, stroke_width: None });
        }
    }
    for (pts, closed) in f.placed(rect.x, rect.y, rect.w, rect.h) {
        if f.is_ink() || (!closed && fill.is_none()) {
            if let Some(color) = stroke {
                let mut pts: Vec<(f32, f32)> = pts.iter().map(|[x, y]| (*x, *y)).collect();
                if closed && let Some(first) = pts.first().copied() {
                    pts.push(first);
                }
                if !closed
                    && extra.arrow_end
                    && let Some(arrow) = path_arrowhead(&pts, color, width)
                {
                    out.push(arrow);
                }
                out.push(Draw::Ink { pts, color, width: width.max(0.25), alpha: f.alpha });
            }
            continue;
        }
        let mut segs: Vec<PathSeg> = Vec::with_capacity(pts.len() + 1);
        for (i, [x, y]) in pts.iter().enumerate() {
            segs.push(if i == 0 { PathSeg::Move(*x, *y) } else { PathSeg::Line(*x, *y) });
        }
        if closed {
            segs.push(PathSeg::Close);
        }
        let width = path_stroke_width(width);
        out.push(Draw::Path {
            segs,
            fill: if closed { fill.clone() } else { None },
            stroke,
            stroke_width: width,
            dash: extra.dash.pattern(width),
            cap: extra.cap,
        });
        if !closed
            && extra.arrow_end
            && let Some(color) = stroke
        {
            let points: Vec<(f32, f32)> = pts.iter().map(|[x, y]| (*x, *y)).collect();
            if let Some(arrow) = path_arrowhead(&points, color, width) {
                out.push(arrow);
            }
        }
    }
}

// Dash lengths use the effective width used by both path painters.
fn path_stroke_width(width: f32) -> f32 {
    if width.is_finite() && width > 0.0 { width.clamp(0.25, 200.0) } else { 0.75 }
}

/// Make a filled triangular arrowhead at the last point, in the points' coordinate space.
/// The last distinct segment supplies its direction and the bounded stroke width sets its size.
/// Returns `None` if there is no usable final segment. The triangle has no outline, so the path's
/// cap setting does not affect its appearance.
fn path_arrowhead(points: &[(f32, f32)], color: Rgb, stroke_width: f32) -> Option<Draw> {
    let tip = *points.last()?;
    let (prev_x, prev_y) = points.iter().rev().skip(1).copied().find(|p| *p != tip)?;
    let dx = tip.0 - prev_x;
    let dy = tip.1 - prev_y;
    let length = dx.hypot(dy);
    if !length.is_finite() || length <= f32::EPSILON {
        return None;
    }
    let (dx, dy) = (dx / length, dy / length);
    let width = if stroke_width.is_finite() { stroke_width.clamp(0.25, 200.0) } else { 0.75 };
    let len = (width * 3.5).max(7.0);
    let (bx, by) = (tip.0 - dx * len, tip.1 - dy * len);
    let (nx, ny) = (-dy * len * 0.45, dx * len * 0.45);
    let segs = vec![PathSeg::Move(tip.0, tip.1), PathSeg::Line(bx + nx, by + ny), PathSeg::Line(bx - nx, by - ny), PathSeg::Close];
    Some(Draw::Path { segs, fill: Some(Paint::solid(color)), stroke: None, stroke_width: 0.0, dash: Vec::new(), cap: Default::default() })
}

/// A line or connector at `rect` (its unflipped frame: flips are the frame's turn), with its
/// arrowheads.
fn line_draws(
    rect: Rect,
    kind: ShapeKind,
    stroke: Option<Rgb>,
    stroke_width: f32,
    effects: wordcraft_doc::effects::ShapeEffects,
    extra: &wordcraft_doc::connector::ShapeExtra,
    out: &mut Vec<Draw>,
) {
    use wordcraft_doc::connector::{connector_segs, end_direction};
    match connector_segs(kind, rect.w, rect.h) {
        Some(segs) => {
            let segs = segs
                .into_iter()
                .map(|s| match s {
                    PathSeg::Move(x, y) => PathSeg::Move(rect.x + x, rect.y + y),
                    PathSeg::Line(x, y) => PathSeg::Line(rect.x + x, rect.y + y),
                    PathSeg::Cubic(a, b, c, d, e, f) => PathSeg::Cubic(rect.x + a, rect.y + b, rect.x + c, rect.y + d, rect.x + e, rect.y + f),
                    PathSeg::Close => PathSeg::Close,
                })
                .collect();
            let width = path_stroke_width(stroke_width);
            out.push(Draw::Path { segs, fill: None, stroke, stroke_width: width, dash: extra.dash.pattern(width), cap: extra.cap });
        }
        None => out.push(Draw::Shape {
            rect,
            kind,
            fill: None,
            stroke,
            stroke_width,
            effects,
            corner: None,
            dash: extra.dash.pattern(stroke_width),
            cap: extra.cap,
        }),
    }
    let Some(color) = stroke else { return };
    // Our line runs from the bottom-left corner to the top-right; connectors from the top-left
    // to the bottom-right.
    let (start, end) =
        if kind == ShapeKind::Line { ((rect.x, rect.bottom()), (rect.right(), rect.y)) } else { ((rect.x, rect.y), (rect.right(), rect.bottom())) };
    let width = if stroke_width.is_finite() { stroke_width.clamp(0.25, 200.0) } else { 0.75 };
    let len = (width * 3.5).max(7.0);
    for (on, tip, at_end) in [(extra.arrow_start, start, false), (extra.arrow_end, end, true)] {
        if !on {
            continue;
        }
        let (dx, dy) = end_direction(kind, rect.w, rect.h, at_end);
        let (bx, by) = (tip.0 - dx * len, tip.1 - dy * len);
        let (nx, ny) = (-dy * len * 0.45, dx * len * 0.45);
        let segs = vec![PathSeg::Move(tip.0, tip.1), PathSeg::Line(bx + nx, by + ny), PathSeg::Line(bx - nx, by - ny), PathSeg::Close];
        out.push(Draw::Path { segs, fill: Some(Paint::solid(color)), stroke: None, stroke_width: 0.0, dash: Vec::new(), cap: Default::default() });
    }
}

pub fn seg_path(segs: &[PathSeg]) -> BezPath {
    let mut p = BezPath::new();
    for s in segs {
        match *s {
            PathSeg::Move(x, y) => p.move_to((x as f64, y as f64)),
            PathSeg::Line(x, y) => p.line_to((x as f64, y as f64)),
            PathSeg::Cubic(a, b, c, d, e, f) => p.curve_to((a as f64, b as f64), (c as f64, d as f64), (e as f64, f as f64)),
            PathSeg::Close => p.close_path(),
        }
    }
    p
}

/// One line of a `GraphicItem::Text` in `r`, shaped in its font (the document's body font when
/// unset) and aligned horizontally, centred vertically, at its size times `k`. `None` when there is
/// nothing to draw, or the item's rectangle or size isn't a finite number (a degenerate scale:
/// `r` itself is always finite, as [`Rect::new`] makes it).
fn text_glyphs(doc: &Document, r: Rect, it: &GraphicItem, alpha: f32, k: f32) -> Option<Draw> {
    let GraphicItem::Text { rect, text, size, color, bold, align, font } = it else { return None };
    let size = *size * k;
    if !rect.iter().chain([&size]).all(|v| v.is_finite()) {
        return None;
    }
    let family = font.as_deref().filter(|f| !f.is_empty()).unwrap_or(&doc.settings.minor_font);
    let size = size.clamp(1.0, 400.0);
    let resolved = wordcraft_fonts::word::resolve(family, *bold, false);
    let face = resolved.face.get();
    let shaped = wordcraft_fonts::shape(face, text, &[], |c| c);
    if shaped.is_empty() {
        return None;
    }
    let k = size / face.upem.max(1.0) as f32;
    let width: f32 = shaped.iter().map(|g| g.x_advance as f32 * k).sum();
    let (ascent, descent) = wordcraft_fonts::word::line_metrics(face);
    let baseline = r.y + r.h / 2.0 + (ascent - descent) as f32 * k / 2.0;
    let mut pen = match align {
        TextAlign::Left => r.x,
        TextAlign::Center => r.x + (r.w - width) / 2.0,
        TextAlign::Right => r.x + r.w - width,
    };
    let mut glyphs = Vec::with_capacity(shaped.len());
    for g in &shaped {
        glyphs.push((g.gid, pen + g.x_offset as f32 * k, baseline - g.y_offset as f32 * k));
        pen += g.x_advance as f32 * k;
    }
    Some(Draw::Glyphs {
        face: resolved.face,
        size,
        glyphs,
        color: *color,
        alpha,
        synth_bold: resolved.synth_bold,
        synth_italic: false,
        text: text.clone(),
        link: None,
        ranges: Vec::new(),
        text_bg: None,
        x_scale: 1.0,
    })
}

/// `items` turned by `spin` about the centre of `rect` (as they are, unturned).
pub(super) fn spun(spin: Spin, rect: Rect, items: Vec<Draw>, out: &mut Vec<Draw>) {
    if spin.is_identity() {
        out.extend(items);
    } else {
        out.push(Draw::Rotated { cx: rect.x + rect.w / 2.0, cy: rect.y + rect.h / 2.0, spin, items });
    }
}

/// A picture, shape or group drawn in `rect` (its unturned frame), turned by its own spin about
/// the rect's centre; a group's members are turned inside it. `outer` is the spin it is already
/// drawn inside (its group's). Nothing for anything else.
pub(super) fn object_draws(o: &InlineObject, rect: Rect, outer: Spin, alpha: f32, opts: &DisplayOptions, out: &mut Vec<Draw>) {
    let own = o.frame().map(|(_, _, f)| f.spin()).unwrap_or_default();
    match o {
        InlineObject::Image { media, crop, alt, .. } => {
            let decorative = o.is_decorative();
            spun(own, rect, vec![Draw::Image { rect, media: media.clone(), crop: *crop, alpha, alt: alt.clone(), decorative }], out)
        }
        InlineObject::Shape { kind, fill, stroke, stroke_width, effects, freeform, extra, .. } => {
            let effects = effects_in(*effects, own.within(outer));
            let mut items = Vec::new();
            shape_draws(
                rect,
                &ShapeLook {
                    kind: *kind,
                    fill: fill.as_ref(),
                    stroke: *stroke,
                    stroke_width: *stroke_width,
                    effects,
                    freeform: freeform.as_deref(),
                    extra,
                },
                opts,
                &mut items,
            );
            match crate::shape_figure_alt(o) {
                // A described shape is one figure with its description (tagged PDF).
                Some(alt) => {
                    let mut draws = Vec::new();
                    spun(own, rect, items, &mut draws);
                    out.push(Draw::Figure { alt: alt.to_string(), kind: None, draws })
                }
                None => spun(own, rect, items, out),
            }
        }
        InlineObject::Group { .. } => {
            let mut members = Vec::new();
            for ([x, y, w, h], c) in o.group_rects(rect.x, rect.y, rect.w, rect.h) {
                if c.frame().is_some_and(|(_, _, f)| f.hidden) {
                    continue;
                }
                object_draws(c, Rect::new(x, y, w, h), own.within(outer), alpha, opts, &mut members);
            }
            match o.canvas_style() {
                // A Drawing Canvas: its background and outline, then its members clipped to its
                // frame, all inside its turn.
                Some(c) => spun(own, rect, vec![canvas_background(rect, c), Draw::Clip { rect, items: members }], out),
                None => spun(own, rect, members, out),
            }
        }
        // Native ink in a group (kept opaque): drawn as drawing-level ink is, from its pictures.
        InlineObject::Graphic { graphic, .. } if graphic.kind == GraphicKind::Ink => {
            let (sx, sy) = (scale_of(rect.w, graphic.w), scale_of(rect.h, graphic.h));
            let items = graphic
                .items
                .iter()
                .filter_map(|it| match it {
                    GraphicItem::Image { rect: r, media } => Some(Draw::Image {
                        rect: Rect::new(rect.x + r[0] * sx, rect.y + r[1] * sy, r[2] * sx, r[3] * sy),
                        media: media.clone(),
                        crop: [0.0; 4],
                        alpha,
                        alt: String::new(),
                        decorative: false,
                    }),
                    _ => None,
                })
                .collect();
            spun(own, rect, items, out)
        }
        _ => {}
    }
}

/// A shape's `effects` as drawn turned by `spin` (all the turns it is drawn inside): glows and
/// soft edges turn with the shape, and so does a shadow's offset when it rotates with the shape
/// (`rotWithShape`); one that doesn't keeps its direction on the page, so its angle is turned back.
pub(crate) fn effects_in(effects: wordcraft_doc::effects::ShapeEffects, spin: Spin) -> wordcraft_doc::effects::ShapeEffects {
    let mut e = effects;
    if let Some(s) = e.shadow.as_mut().filter(|s| !s.rot_with_shape && !spin.is_identity()) {
        let a = s.angle.to_radians();
        let (dx, dy) = spin.unapply(0.0, 0.0, a.cos(), a.sin());
        s.angle = wordcraft_geom::normalize_degrees(dy.atan2(dx).to_degrees());
    }
    e
}

/// Where inline object `obj` is drawn, given its cluster's box (`adv` × `obj_h` standing on the
/// baseline at `cx`): inside the room kept for its effects.
/// A rotated one's unrotated frame, centred in that room.
pub(crate) fn inline_rect(obj: Option<&InlineObject>, cx: f32, base: f32, adv: f32, obj_h: f32) -> Rect {
    let frame = obj.and_then(InlineObject::frame);
    let [l, t, r, b] = frame.map_or([0.0; 4], |(_, _, float)| float.effect_extent());
    let room = Rect::new(cx + l, base - obj_h + t, (adv - l - r).max(0.0), (obj_h - t - b).max(0.0));
    let Some((w, h, float)) = frame.filter(|(_, _, f)| f.rot != 0.0) else { return room };
    // The room is the rotated bounds of the frame: undo the rotation.
    let (w, h) = (wordcraft_geom::finite(w).clamp(1.0, 4000.0), wordcraft_geom::finite(h).clamp(1.0, 4000.0));
    let (bw, _) = float.spin().extent(w, h);
    let k = if bw > 0.0 { room.w / bw } else { 1.0 };
    let (fw, fh) = (w * k, h * k);
    Rect::new(room.x + (room.w - fw) / 2.0, room.y + (room.h - fh) / 2.0, fw, fh)
}

/// A Drawing Canvas's background and outline over its frame `rect`.
fn canvas_background(rect: Rect, c: &wordcraft_doc::para::CanvasStyle) -> Draw {
    Draw::Shape {
        rect,
        kind: ShapeKind::Rectangle,
        fill: c.fill.as_ref().and_then(|f| paint_for(f, rect)),
        stroke: c.stroke,
        stroke_width: c.stroke_width,
        effects: Default::default(),
        corner: None,
        dash: Vec::new(),
        cap: Default::default(),
    }
}
