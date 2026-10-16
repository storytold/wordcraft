//! How much memory resvg may take to draw a picture (#1910 review 10). resvg allocates as it
//! draws, and a failed allocation aborts the process: a layer for every group it draws apart
//! (clipped only to five times the raster each way), a filter's whole region for every step of
//! the filter (not clipped at all), a pattern's tile, a clip path's or mask's own raster, decoded
//! images, and the edges of every path it fills or strokes. This walks the tree the way resvg's
//! renderer does, with the same transforms and the same sizes, and adds up what is held at once
//! along the deepest way down: an upper bound on resvg's peak memory for one raster. The walk
//! uses an explicit stack; work shared by references is walked once per use, as resvg draws it,
//! up to [`MAX_JOBS`].

use resvg::tiny_skia::Transform;

/// Most pieces of the tree the walk looks at for one raster; past it the cost counts as
/// unbounded.
const MAX_JOBS: u64 = 1 << 24;
/// Bytes per pixel of a raster.
const PX: u64 = 4;
/// Most dashes tiny-skia makes of one path.
const MAX_DASHES: u64 = 1_000_000;

/// A part of a raster resvg draws into: its size, and the area layers are clipped to (resvg's
/// `max_bbox`, five times the root raster each way, in the coordinates of the raster drawn into).
#[derive(Clone, Copy)]
struct Target {
    w: u64,
    h: u64,
    max: (i64, i64, i64, i64),
}

impl Target {
    fn root(w: u32, h: u32) -> Target {
        let (w, h) = (u64::from(w), u64::from(h));
        Target { w, h, max: (-(w as i64) * 2, -(h as i64) * 2, w as i64 * 3, h as i64 * 3) }
    }

    fn bytes(&self) -> u64 {
        self.w.saturating_mul(self.h).saturating_mul(PX)
    }
}

/// What resvg draws, as it walks.
enum Job<'a> {
    /// A group's children, one after another.
    Nodes(&'a usvg::Group, Transform, Target),
    /// One node.
    Node(&'a usvg::Node, Transform, Target),
    /// A group (drawn apart if it must be).
    Group(&'a usvg::Group, Transform, Target),
    /// A path filled (`false`) or stroked (`true`).
    Paint(&'a usvg::Path, Transform, Target, bool),
    /// A clip path applied to a layer of the size given.
    Clip(&'a usvg::ClipPath, Transform, Target),
    /// What a clip path holds, drawn into its raster.
    ClipKids(&'a usvg::Group, Transform, Target),
    /// A group with a clip path of its own inside a clip path.
    ClipGroup(&'a usvg::Group, &'a usvg::ClipPath, Transform, Target),
    /// A mask applied to a layer.
    Mask(&'a usvg::Mask, Transform, Target),
    /// A filter applied to a layer.
    Filter(&'a usvg::filter::Filter, Transform, Target),
}

/// An integer rectangle `(x0, y0, x1, y1)` holding `r`.
fn int_rect(r: usvg::NonZeroRect) -> Option<(i64, i64, i64, i64)> {
    let f = |v: f32| if v.is_finite() { v.clamp(-1e15, 1e15) as f64 } else { f64::NAN };
    let (x0, y0, x1, y1) = (f(r.left()).floor(), f(r.top()).floor(), f(r.right()).ceil(), f(r.bottom()).ceil());
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return None;
    }
    Some((x0 as i64, y0 as i64, x1 as i64, y1 as i64))
}

/// The pixels of `(x0, y0, x1, y1)`, as `(w, h)`.
fn dims(r: (i64, i64, i64, i64)) -> (u64, u64) {
    (r.2.saturating_sub(r.0).max(0) as u64, r.3.saturating_sub(r.1).max(0) as u64)
}

/// Bytes of a decoded raster image (from its header; the decoders' own limits apply too).
fn image_bytes(kind: &usvg::ImageKind) -> u64 {
    let data = match kind {
        usvg::ImageKind::JPEG(d) | usvg::ImageKind::PNG(d) | usvg::ImageKind::GIF(d) | usvg::ImageKind::WEBP(d) => d,
        usvg::ImageKind::SVG(_) => return 0,
    };
    match imagesize::blob_size(data) {
        Ok(s) => (s.width as u64).saturating_mul(s.height as u64).saturating_mul(PX),
        // Not readable: resvg won't draw it either.
        Err(_) => 0,
    }
}

/// What filling or stroking `p` with `ts` holds: its edges, and for a stroke its outline (and
/// dashes, and round joins and caps). Measured on tiny-skia, with a margin: a line costs about the
/// same at any size, a curve's stroke more the longer it is in pixels (the stroker splits it).
fn path_bytes(p: &usvg::Path, ts: Transform, stroke: bool) -> u64 {
    use usvg::tiny_skia_path::{PathSegment, Point};
    let (sx, sy) = ts.get_scale();
    let scale = f64::from(sx.max(sy));
    let scale = if scale.is_finite() { scale } else { 1.0 };
    let (mut lines, mut curves, mut curve_len, mut all_len, mut subpaths) = (0u64, 0u64, 0f64, 0f64, 0u64);
    let mut last = Point::zero();
    let dist = |a: Point, b: Point| {
        let d = f64::from(b.x - a.x).hypot(f64::from(b.y - a.y));
        if d.is_finite() { d } else { 0.0 }
    };
    for seg in p.data().segments() {
        match seg {
            PathSegment::MoveTo(q) => {
                subpaths += 1;
                last = q;
            }
            PathSegment::LineTo(q) => {
                lines += 1;
                all_len += dist(last, q);
                last = q;
            }
            PathSegment::QuadTo(a, q) => {
                curves += 1;
                let l = dist(last, a) + dist(a, q);
                (curve_len, all_len, last) = (curve_len + l, all_len + l, q);
            }
            PathSegment::CubicTo(a, b, q) => {
                curves += 1;
                let l = dist(last, a) + dist(a, b) + dist(b, q);
                (curve_len, all_len, last) = (curve_len + l, all_len + l, q);
            }
            PathSegment::Close => lines += 1,
        }
    }
    let px = |len: f64, per_px: f64| -> u64 { (len * scale * per_px).clamp(0.0, 1e18) as u64 };
    if !stroke {
        return lines.saturating_mul(200).saturating_add(curves.saturating_mul(400)).saturating_add(px(curve_len, 1.0));
    }
    let Some(s) = p.stroke() else { return 0 };
    let mut bytes = lines.saturating_mul(400).saturating_add(curves.saturating_mul(600)).saturating_add(px(curve_len, 4.0));
    let width = f64::from(s.width().get());
    // A round join or cap is a curve around a vertex; a square cap a few lines.
    let round = 600u64.saturating_add(px(width * 4.0, 4.0));
    let cap = match s.linecap() {
        usvg::LineCap::Round => round,
        usvg::LineCap::Square => 400,
        usvg::LineCap::Butt => 0,
    };
    if matches!(s.linejoin(), usvg::LineJoin::Round) {
        bytes = bytes.saturating_add(lines.saturating_add(curves).saturating_mul(round));
    }
    bytes = bytes.saturating_add(cap.saturating_mul(2).saturating_mul(subpaths));
    if let Some(dash) = s.dasharray() {
        // tiny-skia's own count, from the path's length (it measures the curves themselves,
        // which are never longer than the lines through their points) times the dashes per
        // interval; past a million it gives up, so at most a million are made. Each dash is a
        // piece of the outline, with caps at both ends (review 11 I-7).
        let period: f64 = dash.iter().map(|d| f64::from(*d)).sum();
        let dashes = if period > 0.0 && period.is_finite() {
            (all_len / period * (dash.len() as f64 / 2.0).max(1.0)).ceil().min(MAX_DASHES as f64) as u64
        } else {
            MAX_DASHES
        };
        let one = 800u64.saturating_add(cap.saturating_mul(2));
        bytes = bytes.saturating_add(dashes.max(1).saturating_mul(one)).saturating_add(px(all_len, 4.0));
    }
    bytes
}

/// What doing `job` holds itself, and the jobs it does while holding it (one after another).
fn expand<'a>(job: Job<'a>) -> (u64, Vec<Job<'a>>) {
    match job {
        Job::Nodes(g, ts, t) => (0, g.children().iter().map(|n| Job::Node(n, ts, t)).collect()),
        Job::Node(n, ts, t) => match n {
            usvg::Node::Group(g) => (0, vec![Job::Group(g, ts, t)]),
            usvg::Node::Text(text) => (0, vec![Job::Group(text.flattened(), ts, t)]),
            usvg::Node::Path(p) => {
                if !p.is_visible() {
                    return (0, Vec::new());
                }
                let mut subs = Vec::new();
                if p.fill().is_some() {
                    subs.push(Job::Paint(p, ts, t, false));
                }
                if p.stroke().is_some() {
                    subs.push(Job::Paint(p, ts, t, true));
                }
                (0, subs)
            }
            usvg::Node::Image(img) => {
                if !img.is_visible() {
                    return (0, Vec::new());
                }
                match img.kind() {
                    // Drawn into a raster of the target's size, as a picture of its own.
                    usvg::ImageKind::SVG(tree) => {
                        let inner = Target::root(t.w.min(u32::MAX as u64) as u32, t.h.min(u32::MAX as u64) as u32);
                        (t.bytes(), vec![Job::Nodes(tree.root(), ts, inner)])
                    }
                    // Decoded, and for some formats copied once.
                    kind => (image_bytes(kind).saturating_mul(2), Vec::new()),
                }
            }
        },
        Job::Group(g, ts, t) => {
            let ts = ts.pre_concat(g.transform());
            if !g.should_isolate() {
                return (0, vec![Job::Nodes(g, ts, t)]);
            }
            // The layer: the group's bounds, two pixels more each way (unless it has filters),
            // clipped to the area layers may take.
            let Some(r) = g.layer_bounding_box().transform(ts).and_then(int_rect) else { return (0, Vec::new()) };
            let pad = if g.filters().is_empty() { 2 } else { 0 };
            let r = (r.0 - pad, r.1 - pad, r.2 + pad, r.3 + pad);
            let m = t.max;
            let r = (r.0.max(m.0), r.1.max(m.1), r.2.min(m.2), r.3.min(m.3));
            let (w, h) = dims(r);
            if w == 0 || h == 0 {
                return (0, Vec::new());
            }
            let layer = Target { w, h, max: t.max };
            let ts = Transform::from_translate(-(r.0 as f32), -(r.1 as f32)).pre_concat(ts);
            let mut subs = vec![Job::Nodes(g, ts, layer)];
            subs.extend(g.filters().iter().map(|f| Job::Filter(f, ts, layer)));
            if let Some(c) = g.clip_path() {
                subs.push(Job::Clip(c, ts, layer));
            }
            if let Some(m) = g.mask() {
                subs.push(Job::Mask(m, ts, layer));
            }
            (layer.bytes(), subs)
        }
        Job::Paint(p, ts, t, stroke) => {
            let mut bytes = path_bytes(p, ts, stroke);
            let paint = if stroke { p.stroke().map(|s| s.paint()) } else { p.fill().map(|f| f.paint()) };
            let mut subs = Vec::new();
            if let Some(usvg::Paint::Pattern(pat)) = paint {
                // The tile, drawn at the scale it's used at, not clipped.
                let (sx, sy) = ts.pre_concat(pat.transform()).get_scale();
                let r = pat.rect();
                let (pw, ph) = ((r.width() * sx).round(), (r.height() * sy).round());
                if pw.is_finite() && ph.is_finite() && pw >= 1.0 && ph >= 1.0 {
                    let tile = Target { w: pw.min(1e12) as u64, h: ph.min(1e12) as u64, max: t.max };
                    bytes = bytes.saturating_add(tile.bytes());
                    subs.push(Job::Nodes(pat.root(), Transform::from_scale(sx, sy), tile));
                }
            }
            (bytes, subs)
        }
        Job::Clip(c, ts, t) => {
            // Its raster, then the mask made of it; what it holds, then its own clip path.
            let mut subs = vec![Job::ClipKids(c.root(), ts.pre_concat(c.transform()), t)];
            if let Some(sub) = c.clip_path() {
                subs.push(Job::Clip(sub, ts, t));
            }
            (t.bytes().saturating_add(t.w.saturating_mul(t.h)), subs)
        }
        Job::ClipKids(g, ts, t) => {
            let mut subs = Vec::new();
            for n in g.children() {
                match n {
                    usvg::Node::Path(p) if p.is_visible() => subs.push(Job::Paint(p, ts, Target { max: (0, 0, 1, 1), ..t }, false)),
                    usvg::Node::Text(text) => subs.push(Job::ClipKids(text.flattened(), ts, t)),
                    usvg::Node::Group(cg) => {
                        let ts = ts.pre_concat(cg.transform());
                        match cg.clip_path() {
                            Some(c) => subs.push(Job::ClipGroup(cg, c, ts, t)),
                            None => subs.push(Job::ClipKids(cg, ts, t)),
                        }
                    }
                    _ => {}
                }
            }
            (0, subs)
        }
        Job::ClipGroup(g, c, ts, t) => (t.bytes(), vec![Job::ClipKids(g, ts, t), Job::Clip(c, ts, t)]),
        Job::Mask(m, ts, t) => {
            if m.root().children().is_empty() {
                return (0, Vec::new());
            }
            // Its raster, a coverage mask of the same size, and the mask made of it.
            let mut subs = vec![Job::Nodes(m.root(), ts, t)];
            if let Some(sub) = m.mask() {
                subs.push(Job::Mask(sub, ts, t));
            }
            (t.bytes().saturating_add(t.w.saturating_mul(t.h).saturating_mul(2)), subs)
        }
        Job::Filter(f, ts, t) => {
            // The filter's region is not clipped. Every step makes a raster of it (some make
            // several, and blurs work in floating point), and every result is kept until the
            // filter ends.
            let Some(r) = f.rect().transform(ts).and_then(int_rect) else { return (0, Vec::new()) };
            let (rw, rh) = dims(r);
            let region = Target { w: rw, h: rh, max: (0, 0, rw as i64, rh as i64) };
            let one = region.bytes().max(t.bytes());
            let mut steps = 0u64;
            let mut subs = Vec::new();
            for p in f.primitives() {
                steps = steps.saturating_add(match p.kind() {
                    usvg::filter::Kind::GaussianBlur(_)
                    | usvg::filter::Kind::DropShadow(_)
                    | usvg::filter::Kind::Morphology(_)
                    | usvg::filter::Kind::ConvolveMatrix(_)
                    | usvg::filter::Kind::DiffuseLighting(_)
                    | usvg::filter::Kind::SpecularLighting(_) => 12,
                    _ => 4,
                });
                if let usvg::filter::Kind::Image(img) = p.kind() {
                    let (sx, sy) = ts.get_scale();
                    subs.push(Job::Nodes(img.root(), Transform::from_scale(sx, sy), region));
                }
            }
            (one.saturating_mul(steps.saturating_add(2)), subs)
        }
    }
}

/// An upper bound on the memory resvg takes to draw `root` with `ts` into a `w` × `h` raster
/// (the raster itself not included). `u64::MAX` when the walk is too long to finish.
pub(crate) fn draw_bytes(root: &usvg::Group, ts: Transform, w: u32, h: u32) -> u64 {
    group_bytes(Job::Nodes(root, ts, Target::root(w, h)))
}

/// [`draw_bytes`] for resvg's `render_node` of a group: it draws the group itself (so a layer of
/// its own if it needs one) with `max_bbox` set from the raster given.
pub(crate) fn node_bytes(group: &usvg::Group, ts: Transform, w: u32, h: u32) -> u64 {
    group_bytes(Job::Group(group, ts, Target::root(w, h)))
}

/// What `job` holds at most at once: what it holds itself, plus the most any one of the jobs it
/// does one after another holds. Walked with an explicit stack.
fn group_bytes(job: Job<'_>) -> u64 {
    struct Frame<'a> {
        own: u64,
        best: u64,
        todo: Vec<Job<'a>>,
    }
    let mut jobs = 0u64;
    let (own, mut todo) = expand(job);
    todo.reverse();
    let mut stack = vec![Frame { own, best: 0, todo }];
    loop {
        let next = match stack.last_mut() {
            Some(top) => top.todo.pop(),
            None => return 0,
        };
        match next {
            Some(job) => {
                jobs += 1;
                if jobs > MAX_JOBS {
                    return u64::MAX;
                }
                let (own, mut todo) = expand(job);
                todo.reverse();
                stack.push(Frame { own, best: 0, todo });
            }
            None => {
                let Some(done) = stack.pop() else { return 0 };
                let value = done.own.saturating_add(done.best);
                match stack.last_mut() {
                    Some(parent) => parent.best = parent.best.max(value),
                    None => return value,
                }
            }
        }
    }
}
