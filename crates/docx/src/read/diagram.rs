//! SmartArt diagrams: the ready-made drawing (`dsp:drawing`) Word keeps beside the diagram data,
//! converted to [`GraphicItem`]s. The diagram's layout isn't run; the saved drawing is the picture.

use wordcraft_doc::graphic::{GraphicItem, PathSeg, TextAlign};
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::Rgb;

use super::Reader;
use super::chart::text_w;
use super::drawing_color::{color, paint};
use crate::package::{Rel, Rels, rel_is, rt};
use crate::units::{MAX_LEN_PT, int};
use crate::xml::El;

/// Deepest `dsp:grpSp` nesting followed.
const MAX_GROUP_DEPTH: usize = 8;
/// Most shapes read from one drawing.
const MAX_SHAPES: usize = 10_000;
/// Most items emitted for one diagram.
const MAX_ITEMS: usize = 50_000;
/// EMU per point.
const EMU_PT: f64 = 12_700.0;
/// Largest coordinate or extent accepted, EMU (about 78,000 pt).
const MAX_EMU: f64 = 1.0e9;
/// Line height as a multiple of the font size.
const LINE: f32 = 1.2;

/// An affine map per axis (`x' = kx·x + ox`, `y' = ky·y + oy`).
#[derive(Clone, Copy, Debug)]
struct Xf {
    kx: f64,
    ox: f64,
    ky: f64,
    oy: f64,
}

impl Xf {
    const ID: Xf = Xf { kx: 1.0, ox: 0.0, ky: 1.0, oy: 0.0 };

    /// `self` applied after `inner`.
    fn then(self, inner: Xf) -> Xf {
        Xf { kx: self.kx * inner.kx, ox: self.kx * inner.ox + self.ox, ky: self.ky * inner.ky, oy: self.ky * inner.oy + self.oy }
    }

    /// The box `[x, y, w, h]` mapped to corners `[x0, y0, x1, y1]`.
    fn rect(self, b: [f64; 4]) -> [f64; 4] {
        let (x0, x1) = (self.kx * b[0] + self.ox, self.kx * (b[0] + b[2]) + self.ox);
        let (y0, y1) = (self.ky * b[1] + self.oy, self.ky * (b[1] + b[3]) + self.oy);
        [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
    }
}

/// A shape's outline in its own box: points and whether it's closed (a fill needs closed).
type Poly = (Vec<(f64, f64)>, bool);

enum Geo {
    Kind(ShapeKind),
    Poly(Vec<(f64, f64)>, bool),
}

/// Points, clamped into the range layout accepts (NaN, from a degenerate transform, is 0).
fn pt(v: f64) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(-MAX_LEN_PT as f64, MAX_LEN_PT as f64) as f32 }
}

/// An EMU attribute of `e`, finite and bounded.
fn emu(e: &El, attr: &str) -> Option<f64> {
    int(e.attr(attr)?).map(|v| (v as f64).clamp(-MAX_EMU, MAX_EMU))
}

/// `a:off` / `a:ext` of an element (a shape's `a:xfrm`, `dsp:txXfrm`) as `[x, y, w, h]` in EMU.
fn xfrm_box(x: &El) -> Option<[f64; 4]> {
    let (off, ext) = (x.child("a:off")?, x.child("a:ext")?);
    Some([emu(off, "x")?, emu(off, "y")?, emu(ext, "cx")?.max(0.0), emu(ext, "cy")?.max(0.0)])
}

/// Child-space to parent-space scale of a group, from its `a:xfrm` (`off`/`ext` against `chOff`/`chExt`).
fn group_xf(grp: &El) -> Xf {
    let Some(x) = grp.child("dsp:grpSpPr").and_then(|p| p.child("a:xfrm")) else { return Xf::ID };
    let v = |tag: &str, a: &str| x.child(tag).and_then(|e| emu(e, a)).unwrap_or(0.0);
    let scale = |e: f64, c: f64| if c > 0.0 && e >= 0.0 { e / c } else { 1.0 };
    let kx = scale(v("a:ext", "cx"), v("a:chExt", "cx"));
    let ky = scale(v("a:ext", "cy"), v("a:chExt", "cy"));
    Xf { kx, ox: v("a:off", "x") - v("a:chOff", "x") * kx, ky, oy: v("a:off", "y") - v("a:chOff", "y") * ky }
}

/// Shapes in `tree` (depth-first through groups), each with its transform into the drawing.
fn collect<'a>(tree: &'a El, xf: Xf, depth: usize, out: &mut Vec<(Xf, &'a El)>) {
    for e in tree.els() {
        match e.name.as_str() {
            "dsp:sp" if out.len() < MAX_SHAPES => out.push((xf, e)),
            "dsp:grpSp" if depth < MAX_GROUP_DEPTH => collect(e, xf.then(group_xf(e)), depth + 1, out),
            _ => {}
        }
    }
}

/// The items of a SmartArt drawing, scaled so its extent fills the `w` × `h` (points) frame.
/// `image` maps a relationship id of the drawing part to a `Document::media` key.
pub(crate) fn diagram_items(drawing: &El, theme: &[Rgb], w: f32, h: f32, image: &mut dyn FnMut(&str) -> Option<String>) -> Vec<GraphicItem> {
    let Some(tree) = drawing.find("dsp:spTree") else { return Vec::new() };
    let mut shapes = Vec::new();
    collect(tree, Xf::ID, 0, &mut shapes);
    let placed: Vec<(Xf, &El, [f64; 4])> = shapes
        .into_iter()
        .filter_map(|(xf, sp)| {
            let b = sp.child("dsp:spPr").and_then(|p| p.child("a:xfrm")).and_then(xfrm_box)?;
            Some((xf, sp, b))
        })
        .collect();
    // The drawing's extent (from its origin) against the frame. One scale for both axes, never
    // above 1:1: Word draws the drawing at its own size when it fits the frame.
    let (ext_x, ext_y) = placed.iter().fold((0.0f64, 0.0f64), |(mx, my), (xf, _, b)| {
        let r = xf.rect(*b);
        (mx.max(r[2]), my.max(r[3]))
    });
    let frame = |f: f32| if f.is_finite() && f > 0.0 { f as f64 * EMU_PT } else { 0.0 };
    let fit_of = |frame: f64, ext: f64| if frame > 0.0 && ext > 0.0 { frame / ext } else { 1.0 };
    let fit = [1.0, fit_of(frame(w), ext_x), fit_of(frame(h), ext_y)].into_iter().fold(f64::INFINITY, f64::min);
    let root = Xf { kx: fit / EMU_PT, ox: 0.0, ky: fit / EMU_PT, oy: 0.0 };
    let mut out = Vec::new();
    for (xf, sp, b) in placed {
        if out.len() >= MAX_ITEMS {
            break;
        }
        shape(sp, root.then(xf), b, theme, fit as f32, image, &mut out);
    }
    out
}

/// One `dsp:sp`: its picture or outline, then its text.
/// `fit` is the drawing's scale into the frame, which text sizes follow.
fn shape(sp: &El, xf: Xf, b: [f64; 4], theme: &[Rgb], fit: f32, image: &mut dyn FnMut(&str) -> Option<String>, out: &mut Vec<GraphicItem>) {
    let [x0, y0, x1, y1] = xf.rect(b);
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let sppr = sp.child("dsp:spPr");
    let xfrm = sppr.and_then(|p| p.child("a:xfrm"));
    let attr = |n: &str| xfrm.and_then(|x| x.attr(n));
    let rot = attr("rot").and_then(int).unwrap_or(0).rem_euclid(21_600_000) as f64 / 60_000.0;
    let flip_h = attr("flipH").is_some_and(on);
    let flip_v = attr("flipV").is_some_and(on);
    let turned = rot != 0.0 || flip_h || flip_v;
    let rect = [pt(x0), pt(y0), pt(w), pt(h)];
    let filled;
    if let Some(blip) = sppr.and_then(|p| p.child("a:blipFill")).and_then(|f| f.find("a:blip")) {
        // A picture fill stretches into the shape's box; no clipping to the outline.
        if let Some(media) = blip.attr("r:embed").and_then(image) {
            out.push(GraphicItem::Image { rect, media });
        }
        filled = false;
    } else {
        let fill = fill_of(sp, sppr, theme);
        let (stroke, stroke_width) = line_of(sp, sppr, theme);
        let prst = sppr.and_then(|p| p.child("a:prstGeom")).and_then(|g| g.attr("prst")).unwrap_or("rect");
        let geom = sppr.and_then(|p| p.child("a:prstGeom"));
        let geo = match kind_of(prst) {
            Some(k) if !turned || prst == "ellipse" => Geo::Kind(k),
            _ => match outline(prst, w, h, geom) {
                Some((pts, closed)) => Geo::Poly(pts, closed),
                None => Geo::Kind(kind_of(prst).unwrap_or(ShapeKind::Rectangle)),
            },
        };
        match geo {
            Geo::Kind(kind) => out.push(GraphicItem::Shape { rect, kind, fill, stroke, stroke_width }),
            Geo::Poly(pts, closed) if !pts.is_empty() => {
                // Flip in the box, then turn about its centre (clockwise, y down).
                let (s, c) = rot.to_radians().sin_cos();
                let (cx, cy) = (w / 2.0, h / 2.0);
                let map = |(px, py): (f64, f64)| {
                    let px = if flip_h { w - px } else { px };
                    let py = if flip_v { h - py } else { py };
                    let (dx, dy) = (px - cx, py - cy);
                    (x0 + cx + dx * c - dy * s, y0 + cy + dx * s + dy * c)
                };
                let mut segs = Vec::with_capacity(pts.len() + 1);
                for (i, p) in pts.into_iter().enumerate() {
                    let (x, y) = map(p);
                    segs.push(if i == 0 { PathSeg::Move(pt(x), pt(y)) } else { PathSeg::Line(pt(x), pt(y)) });
                }
                if closed {
                    segs.push(PathSeg::Close);
                }
                let fill = if closed { fill } else { None };
                out.push(GraphicItem::Path { segs, fill, stroke, stroke_width });
            }
            Geo::Poly(..) => {}
        }
        filled = fill.is_some();
    }
    text(sp, xf, b, theme, filled, fit, out);
}

fn on(v: &str) -> bool {
    v == "1" || v == "true"
}

/// The colour of a shape's style reference (`a:fillRef`, `a:lnRef`); index 0 means none.
fn style_colour(sp: &El, which: &str, theme: &[Rgb]) -> Option<Rgb> {
    let r = sp.child("dsp:style")?.child(which)?;
    if r.attr("idx") == Some("0") {
        return None;
    }
    color(r, theme)
}

/// A shape's fill: its own `a:*Fill`, else the style's fill colour.
fn fill_of(sp: &El, sppr: Option<&El>, theme: &[Rgb]) -> Option<Rgb> {
    let own = sppr.and_then(|p| p.els().find(|e| matches!(e.name.as_str(), "a:solidFill" | "a:noFill" | "a:gradFill" | "a:pattFill")));
    match own {
        Some(e) => paint(e, theme),
        None => style_colour(sp, "a:fillRef", theme),
    }
}

/// A shape's outline colour and width in points (`a:ln`, else the style's line at 0.75 pt).
fn line_of(sp: &El, sppr: Option<&El>, theme: &[Rgb]) -> (Option<Rgb>, f32) {
    let ln = sppr.and_then(|p| p.child("a:ln"));
    let width = ln.and_then(|l| l.attr("w")).and_then(int).map_or(0.75, |v| (v as f64 / EMU_PT).clamp(0.0, 100.0) as f32);
    let own = ln.and_then(|l| l.els().find(|e| matches!(e.name.as_str(), "a:solidFill" | "a:noFill" | "a:gradFill")));
    match own {
        Some(e) if e.name == "a:noFill" => (None, 0.0),
        Some(e) => (paint(e, theme), width),
        None => (style_colour(sp, "a:lnRef", theme), width),
    }
}

/// The preset's own [`ShapeKind`], for the presets that have one. `None` means a polygon.
fn kind_of(prst: &str) -> Option<ShapeKind> {
    Some(match prst {
        "roundRect" => ShapeKind::RoundedRectangle,
        "ellipse" => ShapeKind::Ellipse,
        "triangle" => ShapeKind::Triangle,
        "diamond" => ShapeKind::Diamond,
        "line" | "straightConnector1" => ShapeKind::Line,
        "rightArrow" => ShapeKind::Arrow,
        "star5" | "star4" | "star6" => ShapeKind::Star,
        "heart" => ShapeKind::Heart,
        "leftArrow" | "upArrow" | "downArrow" | "leftRightArrow" | "upDownArrow" | "chevron" | "homePlate" | "hexagon" | "pentagon" | "trapezoid"
        | "parallelogram" | "rtTriangle" | "pie" | "blockArc" => return None,
        _ => ShapeKind::Rectangle,
    })
}

/// A preset's adjust value (`a:avLst` `a:gd name`, `fmla="val N"`) in its raw units, `def` when absent.
fn raw_adj(geom: Option<&El>, name: &str, def: f64) -> f64 {
    geom.and_then(|g| g.child("a:avLst"))
        .and_then(|l| l.children("a:gd").find(|d| d.attr("name") == Some(name)))
        .and_then(|d| d.attr("fmla"))
        .and_then(|f| f.strip_prefix("val "))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .map_or(def, |v| v.clamp(-1.0e9, 1.0e9))
}

/// A preset's adjust value as a fraction of the box (raw / 100000), `def` (a fraction) when absent.
fn adj(geom: Option<&El>, name: &str, def: f64) -> f64 {
    raw_adj(geom, name, def * 100_000.0) / 100_000.0
}

/// A pie slice (`pie`) or block arc (`blockArc`) from `adj1` to `adj2` (60000ths of a degree,
/// clockwise from 3 o'clock); the block arc's band is `adj3` of the short side thick.
fn sector(block: bool, w: f64, h: f64, geom: Option<&El>) -> Poly {
    let deg = |n: &str, def: f64| raw_adj(geom, n, def) / 60_000.0;
    let (st, en) = if block { (deg("adj1", 10_800_000.0), deg("adj2", 0.0)) } else { (deg("adj1", 0.0), deg("adj2", 16_200_000.0)) };
    let sweep = (en - st).rem_euclid(360.0);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let arc = move |rx: f64, ry: f64| {
        (0..=36).map(move |k| {
            let t = (st + sweep * k as f64 / 36.0).to_radians();
            (hw + rx * t.cos(), hh + ry * t.sin())
        })
    };
    if block {
        let band = lim(w.min(h) * adj(geom, "adj3", 0.25), w.min(h) / 2.0);
        let pts = arc(hw, hh).chain(arc(hw - band, hh - band).rev()).collect();
        (pts, true)
    } else {
        let mut pts = vec![(hw, hh)];
        pts.extend(arc(hw, hh));
        (pts, true)
    }
}

/// `v` limited to `[0, hi]` (never panics, unlike `clamp` with `hi < 0`).
fn lim(v: f64, hi: f64) -> f64 {
    v.max(0.0).min(hi.max(0.0))
}

/// The outline of a polygon preset in a `w` × `h` box (points), or `None` for presets with no
/// polygon (ellipse, heart, stars). Unknown presets are rectangles.
fn outline(prst: &str, w: f64, h: f64, geom: Option<&El>) -> Option<Poly> {
    let ss = w.min(h);
    let a = |n: &str, d: f64| adj(geom, n, d);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let pts = match prst {
        "line" | "straightConnector1" => return Some((vec![(0.0, 0.0), (w, h)], false)),
        "pie" | "blockArc" => return Some(sector(prst == "blockArc", w, h, geom)),
        "ellipse" | "heart" | "star5" | "star4" | "star6" => return None,
        "roundRect" => return Some((rounded(w, h, lim(ss * a("adj", 0.16667), ss / 2.0)), true)),
        "triangle" => {
            let apex = lim(w * a("adj", 0.5), w);
            vec![(apex, 0.0), (w, h), (0.0, h)]
        }
        "rtTriangle" => vec![(0.0, 0.0), (0.0, h), (w, h)],
        "diamond" => vec![(hw, 0.0), (w, hh), (hw, h), (0.0, hh)],
        "rightArrow" | "leftArrow" | "upArrow" | "downArrow" => {
            let (len, thick) = if prst == "upArrow" || prst == "downArrow" { (h, w) } else { (w, h) };
            arrow(len, thick, a("adj1", 0.5), a("adj2", 0.5), false)
                .into_iter()
                .map(|(u, v)| match prst {
                    "leftArrow" => (w - u, v),
                    "upArrow" => (v, h - u),
                    "downArrow" => (v, u),
                    _ => (u, v),
                })
                .collect()
        }
        "leftRightArrow" | "upDownArrow" => {
            let (len, thick) = if prst == "upDownArrow" { (h, w) } else { (w, h) };
            arrow(len, thick, a("adj1", 0.5), a("adj2", 0.5), true)
                .into_iter()
                .map(|(u, v)| if prst == "upDownArrow" { (v, u) } else { (u, v) })
                .collect()
        }
        "chevron" => {
            let x1 = lim(ss * a("adj", 0.5), w);
            vec![(0.0, 0.0), (w - x1, 0.0), (w, hh), (w - x1, h), (0.0, h), (x1, hh)]
        }
        "homePlate" => {
            let x1 = w - lim(ss * a("adj", 0.5), w);
            vec![(0.0, 0.0), (x1, 0.0), (w, hh), (x1, h), (0.0, h)]
        }
        "hexagon" => {
            let x1 = lim(ss * a("adj", 0.25), w / 2.0);
            vec![(0.0, hh), (x1, 0.0), (w - x1, 0.0), (w, hh), (w - x1, h), (x1, h)]
        }
        "pentagon" => (0..5)
            .map(|k| {
                let t = (-90.0 + 72.0 * k as f64).to_radians();
                (hw + hw * t.cos(), hh + hh * t.sin())
            })
            .collect(),
        "trapezoid" => {
            let x2 = lim(ss * a("adj", 0.25), w / 2.0);
            vec![(0.0, h), (x2, 0.0), (w - x2, 0.0), (w, h)]
        }
        "parallelogram" => {
            let x2 = lim(ss * a("adj", 0.25), w);
            vec![(0.0, h), (x2, 0.0), (w, 0.0), (w - x2, h)]
        }
        _ => vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)],
    };
    Some((pts, true))
}

/// A block arrow along `len` (tip at `len`) across `thick`. `a1` is the shaft thickness and `a2`
/// the head length, as fractions of the box; `double` puts a head on both ends.
fn arrow(len: f64, thick: f64, a1: f64, a2: f64, double: bool) -> Vec<(f64, f64)> {
    let head = lim(len.min(thick) * a2, len / 2.0);
    let half = lim(thick * a1 / 2.0, thick / 2.0);
    let c = thick / 2.0;
    if double {
        vec![
            (0.0, c),
            (head, 0.0),
            (head, c - half),
            (len - head, c - half),
            (len - head, 0.0),
            (len, c),
            (len - head, thick),
            (len - head, c + half),
            (head, c + half),
            (head, thick),
        ]
    } else {
        vec![(0.0, c - half), (len - head, c - half), (len - head, 0.0), (len, c), (len - head, thick), (len - head, c + half), (0.0, c + half)]
    }
}

/// A `w` × `h` rectangle with corners of radius `r`, each corner in six steps.
fn rounded(w: f64, h: f64, r: f64) -> Vec<(f64, f64)> {
    let mut pts = Vec::with_capacity(28);
    for (cx, cy, start) in [(w - r, r, -90.0), (w - r, h - r, 0.0), (r, h - r, 90.0), (r, r, 180.0)] {
        for k in 0..=6 {
            let t = (start + 15.0 * k as f64).to_radians();
            pts.push((cx + r * t.cos(), cy + r * t.sin()));
        }
    }
    pts
}

/// One line of a text item before it's placed.
struct Line {
    text: String,
    size: f32,
    color: Rgb,
    bold: bool,
    align: TextAlign,
    font: Option<String>,
}

/// The text of a `dsp:txBody`: one item per (wrapped) line, stacked and centred in the text box.
fn text(sp: &El, xf: Xf, b: [f64; 4], theme: &[Rgb], filled: bool, fit: f32, out: &mut Vec<GraphicItem>) {
    let Some(tb) = sp.child("dsp:txBody") else { return };
    let body = tb.child("a:bodyPr");
    let scale = body
        .and_then(|p| p.child("a:normAutofit"))
        .and_then(|n| n.attr("fontScale"))
        .and_then(int)
        .map_or(1.0, |v| (v as f32 / 100_000.0).clamp(0.1, 1.0));
    let inset = |n: &str, def: f64| body.and_then(|p| p.attr(n)).and_then(int).map_or(def, |v| (v as f64).clamp(0.0, MAX_EMU));
    let text_box = match sp.child("dsp:txXfrm").and_then(xfrm_box) {
        Some(t) => t,
        None => {
            let (l, t, r, bo) = (inset("lIns", 91_440.0), inset("tIns", 45_720.0), inset("rIns", 91_440.0), inset("bIns", 45_720.0));
            [b[0] + l, b[1] + t, (b[2] - l - r).max(0.0), (b[3] - t - bo).max(0.0)]
        }
    };
    let [x0, y0, x1, y1] = xf.rect(text_box);
    let (x0, y0, tw, th) = (pt(x0), pt(y0), pt(x1 - x0), pt(y1 - y0));
    // Each line is one item: stop at the diagram's item limit, however much text there is.
    let room = MAX_ITEMS.saturating_sub(out.len());
    let fallback = theme.get(if filled { 1 } else { 0 }).copied().unwrap_or(if filled { Rgb::WHITE } else { Rgb::BLACK });
    let font_ref = sp.child("dsp:style").and_then(|s| s.child("a:fontRef")).and_then(|f| color(f, theme));
    let mut lines: Vec<Line> = Vec::new();
    for p in tb.children("a:p") {
        if lines.len() >= room {
            break;
        }
        let text: String = p.children("a:r").filter_map(|r| r.child("a:t")).map(|t| t.text()).collect();
        let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let run = p.children("a:r").find(|r| r.child("a:t").is_some());
        let rpr = run.and_then(|r| r.child("a:rPr"));
        // The file's size (18 pt when unset), reduced by the autofit and scaled with the drawing.
        let base = rpr.and_then(|r| r.attr("sz")).and_then(int).map_or(18.0, |v| (v as f32 / 100.0).clamp(1.0, 400.0));
        let size = base * scale * fit;
        let color = rpr.and_then(|r| r.child("a:solidFill")).and_then(|f| color(f, theme)).or(font_ref).unwrap_or(fallback);
        let bold = rpr.and_then(|r| r.attr("b")).is_some_and(on);
        let align = match p.child("a:pPr").and_then(|q| q.attr("algn")) {
            Some("l" | "just" | "dist") => TextAlign::Left,
            Some("r") => TextAlign::Right,
            _ => TextAlign::Center,
        };
        let font = rpr
            .and_then(|r| r.child("a:latin"))
            .and_then(|l| l.attr("typeface"))
            .filter(|t| !t.is_empty() && !t.starts_with('+'))
            .map(str::to_string);
        for line in wrap(text, size, tw, room - lines.len()) {
            lines.push(Line { text: line, size, color, bold, align, font: font.clone() });
        }
    }
    let total: f32 = lines.iter().map(|l| l.size * LINE).sum();
    let mut y = y0 + (th - total) / 2.0;
    for l in lines {
        let lh = l.size * LINE;
        out.push(GraphicItem::Text { rect: [x0, y, tw, lh], text: l.text, size: l.size, color: l.color, bold: l.bold, align: l.align, font: l.font });
        y += lh;
    }
}

/// Splits `text` into at most `max` lines of whole words that fit `width` points (by the shared
/// text width estimate); a word wider than that keeps its own line.
fn wrap(text: &str, size: f32, width: f32, max: usize) -> Vec<String> {
    let limit = if width > 0.0 && size > 0.0 { width } else { f32::INFINITY };
    let space = text_w(" ", size);
    let mut lines = Vec::new();
    let (mut cur, mut cur_w) = (String::new(), 0.0f32);
    for word in text.split_whitespace() {
        let w = text_w(word, size);
        if !cur.is_empty() && cur_w + space + w > limit {
            if lines.len() + 1 >= max {
                break;
            }
            lines.push(std::mem::take(&mut cur));
            cur_w = 0.0;
        }
        if !cur.is_empty() {
            cur.push(' ');
            cur_w += space;
        }
        cur.push_str(word);
        cur_w += w;
    }
    if !cur.is_empty() && lines.len() < max {
        lines.push(cur);
    }
    lines
}

impl Reader<'_> {
    /// The drawing part of a SmartArt diagram (`a:graphicData` with `dgm:relIds`): the one its
    /// data part names, else the drawing numbered like the data part. `None` when there's none.
    pub(super) fn diagram_part(&mut self, gd: &El, rels: &Rels) -> Option<String> {
        let dm = gd.child("dgm:relIds")?.attr("r:dm")?;
        let data = super::part_of(rels, dm, rt::DIAGRAM_DATA)?;
        let ext_id = self.graphic_part(&data).and_then(|d| d.find("dsp:dataModelExt").and_then(|e| e.attr("relId")).map(str::to_string));
        let is_drawing = |r: &&Rel| !r.external && rel_is(&r.kind, rt::DIAGRAM_DRAWING);
        let drawing = ext_id.as_deref().and_then(|id| rels.by_id(id)).filter(is_drawing).or_else(|| {
            // No usable relationship id: the drawing whose number matches the data part's.
            let num = |t: &str| t.rsplit('/').next().unwrap_or(t).chars().filter(char::is_ascii_digit).collect::<String>();
            let n = num(&data);
            rels.list.iter().filter(is_drawing).find(|r| !n.is_empty() && num(&r.target) == n)
        });
        drawing.map(|r| r.target.clone())
    }

    /// The id of the relationship to a SmartArt diagram's drawing part, as its data part names it
    /// (`dsp:dataModelExt relId`): the id the parts use, not the markup, so it's kept with them.
    pub(super) fn diagram_drawing_rel(&mut self, gd: &El, rels: &Rels) -> Option<String> {
        let dm = gd.child("dgm:relIds")?.attr("r:dm")?;
        let data = super::part_of(rels, dm, rt::DIAGRAM_DATA)?;
        let id = self.graphic_part(&data)?.find("dsp:dataModelExt")?.attr("relId")?.to_string();
        rels.by_id(&id).is_some_and(|r| !r.external && rel_is(&r.kind, rt::DIAGRAM_DRAWING)).then_some(id)
    }

    /// The items of the SmartArt drawing part at `path`, fitted to `w` × `h` points.
    pub(super) fn diagram_drawing(&mut self, path: &str, w: f32, h: f32) -> Vec<GraphicItem> {
        let Some(root) = self.graphic_part(path) else { return Vec::new() };
        let drels = self.pkg.rels(path);
        let theme = self.doc.settings.theme_colors.clone();
        let mut image = |id: &str| self.media_for(&drels, id);
        diagram_items(&root, &theme, w, h, &mut image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::THEME_COLORS;

    const NS: &str = r#"xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

    fn drawing(body: &str) -> El {
        crate::xml::parse(format!("<dsp:drawing {NS}><dsp:spTree>{body}</dsp:spTree></dsp:drawing>").as_bytes()).unwrap()
    }

    /// Items for a drawing whose only image relationship is `rId1`.
    fn items(body: &str, w: f32, h: f32) -> Vec<GraphicItem> {
        let root = drawing(body);
        diagram_items(&root, &THEME_COLORS, w, h, &mut |id| (id == "rId1").then(|| "image1.png".to_string()))
    }

    /// A `dsp:sp` with the given geometry (`prst`, `xfrm` attributes) and extra spPr/txBody XML.
    fn sp(xfrm: &str, prst: &str, rest: &str) -> String {
        format!(
            r#"<dsp:sp><dsp:spPr><a:xfrm {xfrm}><a:off x="0" y="0"/><a:ext cx="1270000" cy="1270000"/></a:xfrm><a:prstGeom prst="{prst}"><a:avLst/></a:prstGeom>{rest}</dsp:spPr></dsp:sp>"#
        )
    }

    #[test]
    fn round_rect_takes_theme_colour_and_text() {
        let body = r#"<dsp:sp><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="635000"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="accent1"/></a:solidFill><a:ln><a:noFill/></a:ln></dsp:spPr><dsp:style><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></dsp:style><dsp:txBody><a:bodyPr/><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="2000" b="1"/><a:t>Foo</a:t></a:r></a:p></dsp:txBody></dsp:sp>"#;
        let out = items(body, 100.0, 50.0);
        let GraphicItem::Shape { kind, fill, stroke, rect, .. } = &out[0] else { panic!("shape first: {out:?}") };
        assert_eq!(*kind, ShapeKind::RoundedRectangle);
        assert_eq!(*fill, Some(Rgb(0x15, 0x60, 0x82)));
        assert_eq!(*stroke, None);
        assert_eq!(*rect, [0.0, 0.0, 100.0, 50.0]);
        let GraphicItem::Text { text, size, color, bold, align, .. } = &out[1] else { panic!("text second: {out:?}") };
        assert_eq!(text, "Foo");
        assert_eq!((*size, *color, *bold, *align), (20.0, Rgb::WHITE, true, TextAlign::Center));
    }

    #[test]
    fn chevron_is_a_path() {
        let out = items(&sp("", "chevron", ""), 100.0, 100.0);
        let GraphicItem::Path { segs, .. } = &out[0] else { panic!("path: {out:?}") };
        assert_eq!(segs.len(), 7);
        assert_eq!(segs.last(), Some(&PathSeg::Close));
    }

    #[test]
    fn rotated_rect_is_a_rotated_path() {
        let out = items(&sp(r#"rot="2700000""#, "rect", r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#), 100.0, 100.0);
        let GraphicItem::Path { segs, fill, .. } = &out[0] else { panic!("path: {out:?}") };
        assert_eq!(*fill, Some(Rgb(255, 0, 0)));
        let PathSeg::Move(x, y) = segs[0] else { panic!("move") };
        // A 100 pt square turned 45 degrees about its centre: the top-left corner moves to the
        // middle of the top edge's extension.
        assert!((x - 50.0).abs() < 0.01 && (y + 20.71).abs() < 0.01, "got {x} {y}");
    }

    #[test]
    fn lum_mod_darkens_the_theme_colour() {
        let rest = r#"<a:solidFill><a:schemeClr val="accent1"><a:lumMod val="50000"/></a:schemeClr></a:solidFill>"#;
        let out = items(&sp("", "rect", rest), 10.0, 10.0);
        let GraphicItem::Shape { fill, .. } = &out[0] else { panic!("shape") };
        let Rgb(r, g, b) = fill.expect("fill");
        // accent1 (156082) at half its luminance.
        assert!(r < 0x15 && g < 0x60 && b < 0x82, "got {r:02x}{g:02x}{b:02x}");
    }

    #[test]
    fn picture_fill_is_an_image() {
        let rest = r#"<a:blipFill><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>"#;
        let out = items(&sp("", "ellipse", rest), 10.0, 10.0);
        assert!(matches!(&out[0], GraphicItem::Image { media, .. } if media == "image1.png"), "{out:?}");
    }

    #[test]
    fn frame_scales_the_drawing_and_groups_transform_children() {
        // A 2 x 2 inch drawing in a 72 pt frame: a group at the origin holds a half-size child.
        let grp = r#"<dsp:grpSp><dsp:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="1270000"/><a:chOff x="0" y="0"/><a:chExt cx="2540000" cy="2540000"/></a:xfrm></dsp:grpSpPr><dsp:sp><dsp:spPr><a:xfrm><a:off x="1270000" y="1270000"/><a:ext cx="1270000" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></dsp:spPr></dsp:sp></dsp:grpSp>"#;
        let out = items(grp, 72.0, 72.0);
        let GraphicItem::Shape { rect, .. } = &out[0] else { panic!("shape: {out:?}") };
        // The group's child sits at half of the 1 in group extent (x = 0.5 in): 36 pt of the 72 pt frame.
        assert_eq!(*rect, [36.0, 36.0, 36.0, 36.0]);
    }

    #[test]
    fn long_text_wraps_into_several_lines() {
        let body = r#"<dsp:sp><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></dsp:spPr><dsp:txBody><a:bodyPr lIns="0" rIns="0"/><a:p><a:r><a:rPr sz="1000"/><a:t>one two three four five six</a:t></a:r></a:p></dsp:txBody></dsp:sp>"#;
        let out = items(body, 30.0, 30.0);
        let texts: Vec<&str> = out.iter().filter_map(|i| if let GraphicItem::Text { text, .. } = i { Some(text.as_str()) } else { None }).collect();
        assert!(texts.len() > 1, "{texts:?}");
        assert_eq!(texts.join(" "), "one two three four five six");
    }

    #[test]
    fn hostile_numbers_and_nesting_do_not_panic() {
        let huge = format!(
            r#"<dsp:sp><dsp:spPr><a:xfrm rot="zzz" flipH="1"><a:off x="99999999999999999999999" y="-1e400"/><a:ext cx="-5" cy="NaN"/></a:xfrm><a:prstGeom prst="leftRightArrow"><a:avLst><a:gd name="adj1" fmla="val 1e999"/><a:gd name="adj2" fmla="val -7"/></a:avLst></a:prstGeom></dsp:spPr><dsp:txBody><a:bodyPr lIns="99999999999999999999" tIns="-9"/><a:p><a:r><a:rPr sz="99999999999999"/><a:t>x</a:t></a:r></a:p></dsp:txBody></dsp:sp>{}"#,
            sp(r#"rot="2700000""#, "chevron", "")
        );
        let mut nested = String::new();
        for _ in 0..40 {
            nested = format!(
                r#"<dsp:grpSp><dsp:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></dsp:grpSpPr>{nested}{}</dsp:grpSp>"#,
                sp("", "rect", "")
            );
        }
        let _ = items(&huge, f32::NAN, -3.0);
        let _ = items(&huge, f32::INFINITY, 1.0e30);
        let _ = items(&nested, 100.0, 100.0);
        let root = drawing(&huge);
        assert!(diagram_items(&root, &[], 10.0, 10.0, &mut |_| None).len() <= MAX_ITEMS);
    }

    #[test]
    fn text_follows_autofit_and_the_drawing_fit() {
        // A 100 pt drawing: `fontScale` 50% halves its 20 pt text and the 18 pt default alike.
        let body = r#"<dsp:sp><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></dsp:spPr><dsp:txBody><a:bodyPr><a:normAutofit fontScale="50000"/></a:bodyPr><a:p><a:r><a:rPr sz="2000"/><a:t>Big</a:t></a:r></a:p><a:p><a:r><a:rPr/><a:t>Small</a:t></a:r></a:p></dsp:txBody></dsp:sp>"#;
        let sizes = |out: Vec<GraphicItem>| -> Vec<(String, f32)> {
            out.into_iter().filter_map(|i| if let GraphicItem::Text { text, size, .. } = i { Some((text, size)) } else { None }).collect()
        };
        assert_eq!(sizes(items(body, 100.0, 100.0)), [("Big".to_string(), 10.0), ("Small".to_string(), 9.0)]);
        // In a 50 pt frame the whole drawing is at half size, so its text is too.
        assert_eq!(sizes(items(body, 50.0, 50.0)), [("Big".to_string(), 5.0), ("Small".to_string(), 4.5)]);
    }

    #[test]
    fn one_huge_text_body_stops_at_the_item_limit() {
        let words = "word ".repeat(MAX_ITEMS + 100);
        let body = format!(
            r#"<dsp:sp><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="12700" cy="12700"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></dsp:spPr><dsp:txBody><a:bodyPr lIns="0" rIns="0"/><a:p><a:r><a:t>{words}</a:t></a:r></a:p><a:p><a:r><a:t>{words}</a:t></a:r></a:p></dsp:txBody></dsp:sp>"#
        );
        let out = items(&body, 1.0, 1.0);
        assert_eq!(out.len(), MAX_ITEMS);
    }

    #[test]
    fn text_far_outside_nested_groups_stays_finite() {
        // Eight groups each scaling 1e9: a shape at -1e9 EMU ends up beyond f32 before clamping.
        let mut tree = r#"<dsp:sp><dsp:spPr><a:xfrm><a:off x="-1000000000" y="0"/><a:ext cx="1000000000" cy="0"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></dsp:spPr><dsp:txBody><a:bodyPr/><a:p><a:r><a:t>Far</a:t></a:r></a:p></dsp:txBody></dsp:sp>"#.to_string();
        for _ in 0..MAX_GROUP_DEPTH {
            tree = format!(
                r#"<dsp:grpSp><dsp:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1000000000" cy="1"/><a:chOff x="0" y="0"/><a:chExt cx="1" cy="1"/></a:xfrm></dsp:grpSpPr>{tree}</dsp:grpSp>"#
            );
        }
        let out = items(&tree, 100.0, 100.0);
        let rects: Vec<[f32; 4]> = out.iter().filter_map(|i| if let GraphicItem::Text { rect, .. } = i { Some(*rect) } else { None }).collect();
        assert!(!rects.is_empty() && rects.iter().flatten().all(|v| v.is_finite()), "{rects:?}");
    }
}
