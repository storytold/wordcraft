//! Where a picture's frame sits and how text wraps around it (ODF 1.3 §19.11 `text:anchor-type`,
//! §20.2xx `style:wrap`, `style:run-through`, `style:horizontal-pos`/`-rel`,
//! `style:vertical-pos`/`-rel`, `fo:margin-*` of `style:graphic-properties`), and WordCraft's
//! [`Float`] for it, both ways.

use super::strokes::Defs;
use super::styles::parse_border;
use super::writer::pt;
use super::xml::{Attrs, get, length};
use crate::model::{BoxLook, parse_color};
use wordcraft_doc::fill::Fill;
use wordcraft_doc::line_dash::LineDash;
use wordcraft_doc::para::{Anchor, Float, FloatAlign, ShapeKind, Wrap, WrapText};
use wordcraft_doc::props::BorderStyle;
use wordcraft_doc::props::Rgb;

/// Largest offset or distance read, points (22 inches).
const MAX_LEN: f32 = 1584.0;

/// The placement a graphic style states; each part `None` where it says nothing (its parent's
/// then counts).
#[derive(Clone, Debug, Default)]
pub(super) struct Graphic {
    wrap: Option<String>,
    run_through: Option<String>,
    hpos: Option<String>,
    hrel: Option<String>,
    vpos: Option<String>,
    vrel: Option<String>,
    /// `fo:margin-left`, `-right`, `-top`, `-bottom`.
    margins: [Option<f32>; 4],
    /// The frame's line (`fo:border`): colour and width, or `Some(None)` for none.
    line: Option<Option<(Rgb, f32)>>,
    /// `fo:padding-left`, `-top`, `-right`, `-bottom` (or `fo:padding` for each).
    padding: [Option<f32>; 4],
    /// Its fill style (`draw:fill`: none, solid, gradient, hatch, bitmap) and colour
    /// (`draw:fill-color`), and its background (`fo:background-color`, `Some(None)` when
    /// transparent).
    fill: Option<String>,
    fill_color: Option<Rgb>,
    background: Option<Option<Rgb>>,
    /// A drawing shape's line: its style (`draw:stroke`: none, solid, dash), colour and width.
    stroke: Option<String>,
    stroke_color: Option<Rgb>,
    stroke_width: Option<f32>,
    /// A shape grows to fit its text (`draw:auto-grow-height`).
    grow: Option<bool>,
    /// The names of its dash (`draw:stroke-dash`), gradient (`draw:fill-gradient-name`) and
    /// arrowheads (`draw:marker-start`, `draw:marker-end`; an empty name is none).
    dash: Option<String>,
    gradient: Option<String>,
    markers: [Option<String>; 2],
}

/// The colour LibreOffice draws a shape's line in when its styles give none (#3465a4), and fills
/// it with (#729fcf); its default graphic style states them too.
const SHAPE_LINE: Rgb = Rgb(0x34, 0x65, 0xa4);
const SHAPE_FILL: Rgb = Rgb(0x72, 0x9f, 0xcf);

/// The width a hairline (`svg:stroke-width` 0, or none stated) is drawn at, points.
const HAIRLINE: f32 = 0.5;

impl Graphic {
    /// A `style:graphic-properties` element.
    pub(super) fn read(a: &Attrs) -> Graphic {
        let s = |k: &str| get(a, k).map(|v| v.trim().to_string());
        let m = |k: &str| get(a, k).and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, MAX_LEN));
        Graphic {
            wrap: s("style:wrap"),
            run_through: s("style:run-through"),
            hpos: s("style:horizontal-pos"),
            hrel: s("style:horizontal-rel"),
            vpos: s("style:vertical-pos"),
            vrel: s("style:vertical-rel"),
            margins: [m("fo:margin-left"), m("fo:margin-right"), m("fo:margin-top"), m("fo:margin-bottom")],
            line: ["fo:border", "fo:border-top", "fo:border-left"]
                .iter()
                .find_map(|k| get(a, k))
                .and_then(parse_border)
                .map(|b| (b.style != BorderStyle::None && b.width > 0.0).then(|| (b.color.unwrap_or(Rgb::BLACK), b.width.clamp(0.0, 12.0)))),
            padding: ["left", "top", "right", "bottom"].map(|side| m(&format!("fo:padding-{side}")).or_else(|| m("fo:padding"))),
            fill: s("draw:fill"),
            fill_color: get(a, "draw:fill-color").and_then(parse_color),
            background: get(a, "fo:background-color").map(|v| parse_color(v).filter(|_| !v.trim().eq_ignore_ascii_case("transparent"))),
            stroke: s("draw:stroke"),
            stroke_color: get(a, "svg:stroke-color").and_then(parse_color),
            grow: get(a, "draw:auto-grow-height").map(|v| v.trim() == "true"),
            dash: s("draw:stroke-dash"),
            gradient: s("draw:fill-gradient-name"),
            markers: [s("draw:marker-start"), s("draw:marker-end")],
            stroke_width: get(a, "svg:stroke-width").and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 12.0)),
        }
    }

    /// This placement with `parent`'s filling what it leaves unsaid.
    pub(super) fn or(mut self, parent: &Graphic) -> Graphic {
        let fill = |a: &mut Option<String>, b: &Option<String>| {
            if a.is_none() {
                a.clone_from(b);
            }
        };
        fill(&mut self.wrap, &parent.wrap);
        fill(&mut self.run_through, &parent.run_through);
        fill(&mut self.hpos, &parent.hpos);
        fill(&mut self.hrel, &parent.hrel);
        fill(&mut self.vpos, &parent.vpos);
        fill(&mut self.vrel, &parent.vrel);
        for (m, p) in self.margins.iter_mut().zip(parent.margins) {
            *m = m.or(p);
        }
        for (m, p) in self.padding.iter_mut().zip(parent.padding) {
            *m = m.or(p);
        }
        self.line = self.line.or(parent.line);
        fill(&mut self.fill, &parent.fill);
        self.fill_color = self.fill_color.or(parent.fill_color);
        self.background = self.background.or(parent.background);
        fill(&mut self.stroke, &parent.stroke);
        self.stroke_color = self.stroke_color.or(parent.stroke_color);
        self.stroke_width = self.stroke_width.or(parent.stroke_width);
        self.grow = self.grow.or(parent.grow);
        fill(&mut self.dash, &parent.dash);
        fill(&mut self.gradient, &parent.gradient);
        for (m, p) in self.markers.iter_mut().zip(&parent.markers) {
            fill(m, p);
        }
        self
    }

    /// The fill a style of `draw:fill` `kind` gives, `default` when it states none: `None` is
    /// no fill; a gradient, hatch or bitmap stands as its colour.
    fn fill_with(&self, default: Option<&str>, defs: &Defs) -> Option<Fill> {
        let color = self.fill_color.or(self.background.flatten());
        let solid = |c: Option<Rgb>| c.map(Fill::solid);
        match self.fill.as_deref().or(default) {
            Some("none") => None,
            Some("solid") => solid(color),
            Some("gradient") => self.gradient.as_deref().and_then(|n| defs.gradient(n)).cloned().or_else(|| solid(color.or(Some(Rgb::WHITE)))),
            Some(_) => solid(color.or(Some(Rgb::WHITE))),
            None => solid(self.background.flatten()),
        }
    }

    /// The line's dash for a line `width` points wide, and its arrowheads (a named marker at
    /// either end).
    fn stroke_extra(&self, width: f32, defs: &Defs) -> (LineDash, (bool, bool)) {
        let dash = match self.stroke.as_deref() {
            Some("dash") => self.dash.as_deref().and_then(|n| defs.dash(n)).map_or(LineDash::Dash, |d| d.pattern(width)),
            _ => LineDash::Solid,
        };
        let marked = |m: &Option<String>| m.as_deref().is_some_and(|m| !m.trim().is_empty());
        (dash, (marked(&self.markers[0]), marked(&self.markers[1])))
    }

    /// The look of drawing shape `kind` in this style: LibreOffice's solid fill and hairline in
    /// its default colours where the styles say nothing; a line has no fill.
    pub(super) fn shape_look(&self, kind: ShapeKind, defs: &Defs) -> BoxLook {
        let fill = if kind == ShapeKind::Line {
            None
        } else {
            let mut g = self.clone();
            g.fill_color = g.fill_color.or(Some(SHAPE_FILL));
            g.fill_with(Some("solid"), defs)
        };
        let line = match self.stroke.as_deref() {
            Some("none") => None,
            _ => Some((self.stroke_color.unwrap_or(SHAPE_LINE), self.stroke_width.filter(|w| *w > 0.0).unwrap_or(HAIRLINE))),
        };
        // Only a shape that says it grows to its text does: WordCraft's fit shrinks it too.
        let (dash, arrows) = self.stroke_extra(line.map_or(HAIRLINE, |l| l.1), defs);
        BoxLook { fill, line, dash, arrows, inset: Some(self.padding.map(|p| p.unwrap_or(0.0))), auto_fit: self.grow == Some(true) }
    }

    /// The look of a text box in a frame of this style, growing to its text when `auto_fit`.
    /// What the styles leave unsaid is ODF's default: no line, no fill, no padding.
    pub(super) fn look(&self, auto_fit: bool, defs: &Defs) -> BoxLook {
        BoxLook {
            fill: self.fill_with(None, defs),
            line: self.line.flatten(),
            dash: LineDash::Solid,
            arrows: (false, false),
            inset: Some(self.padding.map(|p| p.unwrap_or(0.0))),
            auto_fit,
        }
    }
}

/// The [`Float`] of a frame anchored as `anchor` (`text:anchor-type`) at `x`, `y` (`svg:x`,
/// `svg:y`) with graphic style placement `g`. A frame in line with the text (`as-char`, or no
/// anchor stated) doesn't float.
pub(super) fn float_of(anchor: Option<&str>, x: Option<&str>, y: Option<&str>, g: &Graphic) -> Float {
    let anchor = anchor.map(str::trim);
    if matches!(anchor, None | Some("as-char")) {
        return Float::default();
    }
    let page = anchor == Some("page");
    let (wrap, wrap_text) = match g.wrap.as_deref() {
        Some("left") => (Wrap::Square, WrapText::Left),
        Some("right") => (Wrap::Square, WrapText::Right),
        Some("parallel") => (Wrap::Square, WrapText::BothSides),
        Some("dynamic") | Some("biggest") => (Wrap::Square, WrapText::Largest),
        Some("run-through") if g.run_through.as_deref() == Some("background") => (Wrap::BehindText, WrapText::BothSides),
        Some("run-through") => (Wrap::InFrontOfText, WrapText::BothSides),
        Some("none") => (Wrap::TopAndBottom, WrapText::BothSides),
        // None stated (§20.400 gives no default): both sides, as LibreOffice wraps.
        _ => (Wrap::Square, WrapText::BothSides),
    };
    let h_rel = match g.hrel.as_deref() {
        Some("page") => Anchor::Page,
        Some("page-start-margin") => Anchor::LeftMargin,
        Some("page-end-margin") => Anchor::RightMargin,
        Some("page-content") => Anchor::Margin,
        Some("paragraph-content") => Anchor::Column,
        Some("char") => Anchor::Character,
        Some(_) => Anchor::Paragraph,
        None if page => Anchor::Page,
        None => Anchor::Paragraph,
    };
    let v_rel = match g.vrel.as_deref() {
        Some("page") => Anchor::Page,
        Some("page-content") => Anchor::Margin,
        Some("line") | Some("char") | Some("text") | Some("baseline") => Anchor::Line,
        Some(_) => Anchor::Paragraph,
        None if page => Anchor::Page,
        None => Anchor::Paragraph,
    };
    let h_align = match g.hpos.as_deref() {
        Some("left") => Some(FloatAlign::Start),
        Some("center") => Some(FloatAlign::Center),
        Some("right") => Some(FloatAlign::End),
        Some("inside") => Some(FloatAlign::Inside),
        Some("outside") => Some(FloatAlign::Outside),
        _ => None,
    };
    let v_align = match g.vpos.as_deref() {
        Some("top") => Some(FloatAlign::Start),
        Some("middle") => Some(FloatAlign::Center),
        Some("bottom") => Some(FloatAlign::End),
        _ => None,
    };
    let off = |v: Option<&str>| v.and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(-MAX_LEN, MAX_LEN)).unwrap_or(0.0);
    let [left, right, top, bottom] = g.margins;
    Float {
        wrap,
        wrap_text,
        h_rel,
        v_rel,
        x: if h_align.is_none() { off(x) } else { 0.0 },
        y: if v_align.is_none() { off(y) } else { 0.0 },
        h_align,
        v_align,
        dist: left.unwrap_or(0.0).max(right.unwrap_or(0.0)),
        dist_top: top.unwrap_or(0.0),
        dist_bottom: bottom.unwrap_or(0.0),
        ..Float::default()
    }
}

/// Where a frame or shape `w` × `h` (points) sits: anchored as its `text:anchor-type` says, at
/// its `svg:x`/`svg:y`, or turned and moved by its `draw:transform`, in graphic style `g`.
pub(super) fn place(a: &Attrs, w: f32, h: f32, g: &Graphic) -> Float {
    let turned = get(a, "draw:transform").and_then(|t| transform(t, w, h));
    let (x, y) = match turned {
        Some((x, y, _)) => (Some(pt(x)), Some(pt(y))),
        None => (get(a, "svg:x").map(String::from), get(a, "svg:y").map(String::from)),
    };
    let mut f = float_of(get(a, "text:anchor-type"), x.as_deref(), y.as_deref(), g);
    if let Some((.., rot)) = turned {
        f.rot = rot;
    }
    if f.wrap == Wrap::Inline {
        f.y = inline_raise(g, y.as_deref().and_then(length), h);
    }
    f
}

/// How far a frame `h` points tall in line with the text is raised from standing on the
/// baseline (points, negative lowered; WordCraft keeps it in an inline [`Float`]'s `y`): its
/// `style:vertical-pos` relative to the baseline (§20.387, §20.388; LibreOffice's placement
/// when no `style:vertical-rel` is given) — `top` stands on it, `middle` is centred on it,
/// `bottom` hangs from it, `from-top` has its top `svg:y` below it. Placement relative to the
/// character or line stays on the baseline.
fn inline_raise(g: &Graphic, y: Option<f32>, h: f32) -> f32 {
    if !matches!(g.vrel.as_deref(), None | Some("baseline")) || !h.is_finite() {
        return 0.0;
    }
    let raise = match g.vpos.as_deref() {
        Some("middle") => -h / 2.0,
        Some("bottom") => -h,
        Some("from-top") => -(h + y.filter(|v| v.is_finite()).unwrap_or(0.0)),
        _ => 0.0,
    };
    raise.clamp(-MAX_RAISE, MAX_RAISE)
}

/// Most an inline frame is raised or lowered, points (22 inches).
const MAX_RAISE: f32 = 1584.0;

/// Most operations of a `draw:transform` read.
const MAX_TRANSFORMS: usize = 16;

/// A `draw:transform` (ODF 1.3 §19.228) of an object `w` × `h`, as WordCraft places it: the
/// offset of its unrotated box and its clockwise turn about the box's centre, in degrees
/// (0–360). The file's operations apply in turn to the object's own coordinates, whose origin
/// is its top left corner: `rotate` turns it anticlockwise (radians), `translate` moves it and
/// `scale` scales it; `skewX`, `skewY` and `matrix` are left out. `None` when none reads.
pub(super) fn transform(v: &str, w: f32, h: f32) -> Option<(f32, f32, f32)> {
    // x' = m0·x + m2·y + m4, y' = m1·x + m3·y + m5.
    let mut m = [1.0f64, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut angle = 0.0f64;
    let mut any = false;
    let mut rest = v;
    for _ in 0..MAX_TRANSFORMS {
        let (Some(open), Some(close)) = (rest.find('('), rest.find(')')) else { break };
        let (Some(name), Some(args)) = (rest.get(..open), rest.get(open + 1..close)) else { break };
        let args: Vec<&str> = args.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).collect();
        rest = rest.get(close + 1..).unwrap_or("");
        let num = |i: usize| args.get(i).and_then(|a| a.parse::<f64>().ok()).filter(|v| v.is_finite());
        let len = |i: usize| args.get(i).and_then(|a| length(a)).filter(|v| v.is_finite()).map(|v| f64::from(v.clamp(-MAX_LEN, MAX_LEN)));
        match name.trim() {
            "rotate" => {
                let Some(a) = num(0) else { continue };
                let a = a % std::f64::consts::TAU;
                let (sin, cos) = a.sin_cos();
                m = [
                    cos * m[0] + sin * m[1],
                    -sin * m[0] + cos * m[1],
                    cos * m[2] + sin * m[3],
                    -sin * m[2] + cos * m[3],
                    cos * m[4] + sin * m[5],
                    -sin * m[4] + cos * m[5],
                ];
                angle += a;
            }
            "translate" => {
                let Some(tx) = len(0) else { continue };
                m[4] += tx;
                m[5] += len(1).unwrap_or(0.0);
            }
            "scale" => {
                let Some(sx) = num(0).map(|v| v.clamp(-100.0, 100.0)) else { continue };
                let sy = num(1).map_or(sx, |v| v.clamp(-100.0, 100.0));
                m = [m[0] * sx, m[1] * sy, m[2] * sx, m[3] * sy, m[4] * sx, m[5] * sy];
            }
            _ => continue,
        }
        any = true;
    }
    if !any {
        return None;
    }
    let (hw, hh) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let centre = (m[0] * hw + m[2] * hh + m[4], m[1] * hw + m[3] * hh + m[5]);
    let fit = |v: f64| if v.is_finite() { v.clamp(-f64::from(MAX_LEN), f64::from(MAX_LEN)) as f32 } else { 0.0 };
    let rot = (-angle.to_degrees()).rem_euclid(360.0);
    // Rounding can leave a whole turn.
    let rot = if rot.is_finite() && rot < 359.9999 { rot as f32 } else { 0.0 };
    Some((fit(centre.0 - hw), fit(centre.1 - hh), rot))
}

/// The `draw:transform` of an object `w` × `h` whose unrotated box is at `x`, `y` and turned `rot`
/// degrees clockwise about its centre (the reverse of [`transform`]).
pub(super) fn transform_attr(x: f32, y: f32, w: f32, h: f32, rot: f32) -> String {
    let fin = |v: f32| if v.is_finite() { f64::from(v.clamp(-MAX_LEN, MAX_LEN)) } else { 0.0 };
    let a = -fin(rot).to_radians();
    let (sin, cos) = a.sin_cos();
    let (hw, hh) = (fin(w) / 2.0, fin(h) / 2.0);
    // The corner that ends up where the centre stays put.
    let tx = fin(x) + hw - (cos * hw + sin * hh);
    let ty = fin(y) + hh - (-sin * hw + cos * hh);
    format!(" draw:transform=\"rotate ({a}) translate ({} {})\"", pt(tx as f32), pt(ty as f32))
}

/// Whether `f` turns its object.
pub(super) fn turned(f: &Float) -> bool {
    f.rot.is_finite() && f.rot.rem_euclid(360.0) > 0.001 && f.rot.rem_euclid(360.0) < 359.999
}

/// How a floating picture is written: its `text:anchor-type`, its graphic style's
/// `style:graphic-properties` attributes, and the frame's `svg:x`/`svg:y` attributes, or its
/// `draw:transform` when it is turned (`w` × `h` its size).
pub(super) fn frame_of(f: &Float, w: f32, h: f32) -> (&'static str, String, String) {
    let anchor = if f.h_rel == Anchor::Page && f.v_rel == Anchor::Page {
        "page"
    } else if f.v_rel == Anchor::Line {
        "char"
    } else {
        "paragraph"
    };
    let wrap = match (f.wrap, f.wrap_text) {
        (Wrap::BehindText, _) => "run-through\" style:run-through=\"background",
        (Wrap::InFrontOfText, _) => "run-through\" style:run-through=\"foreground",
        (Wrap::TopAndBottom, _) => "none",
        (_, WrapText::Left) => "left",
        (_, WrapText::Right) => "right",
        (_, WrapText::Largest) => "dynamic",
        _ => "parallel",
    };
    let hrel = match f.h_rel {
        Anchor::Page => "page",
        Anchor::Margin | Anchor::InsideMargin | Anchor::OutsideMargin => "page-content",
        Anchor::LeftMargin => "page-start-margin",
        Anchor::RightMargin => "page-end-margin",
        Anchor::Column => "paragraph-content",
        Anchor::Character => "char",
        _ => "paragraph",
    };
    let vrel = match f.v_rel {
        Anchor::Page | Anchor::TopMargin | Anchor::BottomMargin => "page",
        Anchor::Margin => "page-content",
        Anchor::Line => "line",
        _ => "paragraph",
    };
    let hpos = match f.h_align {
        Some(FloatAlign::Start) => "left",
        Some(FloatAlign::Center) => "center",
        Some(FloatAlign::End) => "right",
        Some(FloatAlign::Inside) => "inside",
        Some(FloatAlign::Outside) => "outside",
        None => "from-left",
    };
    let vpos = match f.v_align {
        Some(FloatAlign::Start) | Some(FloatAlign::Inside) => "top",
        Some(FloatAlign::Center) => "middle",
        Some(FloatAlign::End) | Some(FloatAlign::Outside) => "bottom",
        None => "from-top",
    };
    let d = |v: f32| pt(if v.is_finite() { v.clamp(0.0, MAX_LEN) } else { 0.0 });
    let props = format!(
        " style:wrap=\"{wrap}\" style:horizontal-pos=\"{hpos}\" style:horizontal-rel=\"{hrel}\" style:vertical-pos=\"{vpos}\" style:vertical-rel=\"{vrel}\" fo:margin-left=\"{}\" fo:margin-right=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\"",
        d(f.dist),
        d(f.dist),
        d(f.dist_top),
        d(f.dist_bottom)
    );
    let mut xy = String::new();
    let p = |v: f32| pt(if v.is_finite() { v.clamp(-MAX_LEN, MAX_LEN) } else { 0.0 });
    if turned(f) {
        let x = if f.h_align.is_none() { f.x } else { 0.0 };
        let y = if f.v_align.is_none() { f.y } else { 0.0 };
        return (anchor, props, transform_attr(x, y, w, h, f.rot));
    }
    if f.h_align.is_none() {
        xy.push_str(&format!(" svg:x=\"{}\"", p(f.x)));
    }
    if f.v_align.is_none() {
        xy.push_str(&format!(" svg:y=\"{}\"", p(f.y)));
    }
    (anchor, props, xy)
}
