//! Shape Format › Shape Fill (#1500): a solid colour with its transparency, or a gradient, for
//! every selected shape, text box and drawing canvas.

use serde_json::Value;
use std::sync::Arc;
use wordcraft_doc::Pos;
use wordcraft_doc::fill::{Fill, Gradient, GradientFlip, GradientKind, PathShape};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::Rgb;
use wordcraft_doc::wordart::GradStop;

use super::objects::{edit_obj, picked_objects, selected_all};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Most shapes one command fills (a selection over a whole document).
const MAX_FILL_SHAPES: usize = 10_000;
/// Most stops read from a command (the fill keeps at most `MAX_STOPS` after sanitizing).
const MAX_PARAM_STOPS: usize = 64;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("shape.fill", "Shape Fill", "Shape Format › Shape Styles", shape_fill)
            .params(
                r#"{"color": "RRGGBB" | null, "transparency"?: %} | {"picture": {"path"?: string, "data"?: base64}} (a picture is stretched over the shape and clipped to its outline; without a path or data the app asks for a file; one shape only) | {"gradient": {"stops": [{"pos": %, "color": "RRGGBB", "transparency"?: %}], "type"?: "linear|radial|rectangular|path", "angle"?: deg, "focus"?: [l, t, r, b], "tile"?: [l, t, r, b], "scaled"?: bool, "rotateWithShape"?: bool}}  (tile: percent insets of the gradient's area; scaled: a linear angle scaled with the shape; a colour alone keeps the shape's transparency; angle: linear only, 0 = left to right, 90 = top to bottom; focus: percent insets of where a radial, rectangular or path gradient starts; applies to every selected shape, text box and canvas background)"#,
            )
            .when(super::objects::has_shape_or_canvas),
    ]
}

/// What a `shape.fill` command sets.
enum Change {
    None,
    /// A colour, with its transparency or (`None`) keeping each shape's.
    Solid(Rgb, Option<f32>),
    Fill(Fill),
}

fn shape_fill(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(pic) = v.get("picture") {
        return super::objects::picture_fill(s, pic);
    }
    let change = parse(v)?;
    let targets = targets(s);
    if targets.is_empty() {
        return Err(CmdError::Disabled("no shape selected".into()));
    }
    let mut last = Value::Null;
    for pos in &targets {
        let o = edit_obj(s, pos, |o| {
            let slot = match o {
                InlineObject::Shape { fill, extra, .. } => {
                    // A colour or gradient takes a picture fill off.
                    extra.picture = None;
                    fill
                }
                o => match super::canvas::style_mut(o) {
                    Some(style) => &mut style.fill,
                    None => return,
                },
            };
            *slot = match &change {
                Change::None => None,
                Change::Fill(f) => Some(f.clone()),
                Change::Solid(color, t) => {
                    let kept = match slot {
                        Some(Fill::Solid { transparency, .. }) => *transparency,
                        _ => 0.0,
                    };
                    Fill::Solid { color: *color, transparency: t.unwrap_or(kept) }.sanitized()
                }
            };
        })?;
        last = serde_json::to_value(o).unwrap_or(Value::Null);
    }
    Ok(serde_json::json!({"object": last, "count": targets.len()}))
}

/// The shapes and canvases in the selection and those added to it (Shift+click).
fn targets(s: &Session) -> Vec<Pos> {
    let fillable = |o: &InlineObject| matches!(o, InlineObject::Shape { .. }) || o.is_canvas();
    let mut out: Vec<Pos> = selected_all(s, MAX_FILL_SHAPES).into_iter().filter(|(_, o)| fillable(o)).map(|(pos, _)| pos).collect();
    for pos in picked_objects(s) {
        if !out.contains(&pos) && s.doc.para_at(&pos).and_then(|q| q.object_at(pos.off)).is_some_and(fillable) {
            out.push(pos);
        }
    }
    out
}

fn color(v: &Value) -> Result<Rgb, CmdError> {
    p::str(v, "color").and_then(Rgb::parse).ok_or_else(|| CmdError::Params("`color`: \"RRGGBB\"".into()))
}

fn parse(v: &Value) -> Result<Change, CmdError> {
    let Some(g) = v.get("gradient") else {
        return match v.get("color") {
            None | Some(Value::Null) => Ok(Change::None),
            Some(_) => Ok(Change::Solid(color(v)?, p::f32(v, "transparency"))),
        };
    };
    let bad = |m: &str| CmdError::Params(format!("`gradient`: {m}"));
    if !g.is_object() {
        return Err(bad("an object"));
    }
    let stops = g
        .get("stops")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`stops` is a list"))?
        .iter()
        .take(MAX_PARAM_STOPS)
        .map(|st| Ok(GradStop { pos: p::f32(st, "pos").unwrap_or(0.0), color: color(st)?, transparency: p::f32(st, "transparency").unwrap_or(0.0) }))
        .collect::<Result<Vec<_>, CmdError>>()?;
    if stops.len() < 2 {
        return Err(bad("two or more stops"));
    }
    let rect = |key: &str, default: f32| -> Option<[f32; 4]> {
        let a = g.get(key)?.as_array()?;
        let mut r = [default; 4];
        for (slot, x) in r.iter_mut().zip(a) {
            *slot = x.as_f64().map_or(default, |x| x as f32);
        }
        Some(r)
    };
    let path = |shape| GradientKind::Path { shape, focus: rect("focus", 50.0).unwrap_or([50.0; 4]) };
    let kind = match p::str(g, "type").unwrap_or("linear") {
        "linear" => {
            GradientKind::Linear { angle: p::f32(g, "angle").unwrap_or(90.0), scaled: g.get("scaled").and_then(Value::as_bool).unwrap_or(false) }
        }
        "radial" => path(PathShape::Circle),
        "rectangular" => path(PathShape::Rect),
        "path" => path(PathShape::Shape),
        t => return Err(bad(&format!("unknown type `{t}`"))),
    };
    let rot_with_shape = g.get("rotateWithShape").and_then(Value::as_bool).unwrap_or(true);
    let fill = Fill::Gradient(Arc::new(Gradient { stops, kind, tile: rect("tile", 0.0), rot_with_shape, flip: GradientFlip::None }));
    fill.sanitized().map(Change::Fill).ok_or_else(|| bad("no stops"))
}

/// The `shape.fill` params that set `fill` (`None`: no fill).
pub fn params(fill: Option<&Fill>) -> Value {
    use serde_json::json;
    match fill {
        None => json!({"color": null}),
        Some(Fill::Solid { color, transparency }) => json!({"color": color.hex(), "transparency": transparency}),
        Some(Fill::Gradient(g)) => {
            let stops: Vec<Value> = g.stops.iter().map(|s| json!({"pos": s.pos, "color": s.color.hex(), "transparency": s.transparency})).collect();
            let mut out = json!({"stops": stops, "rotateWithShape": g.rot_with_shape});
            if let Some(t) = g.tile {
                out["tile"] = json!(t);
            }
            match g.kind {
                GradientKind::Linear { angle, scaled } => {
                    out["type"] = json!("linear");
                    out["angle"] = json!(angle);
                    out["scaled"] = json!(scaled);
                }
                GradientKind::Path { shape, focus } => {
                    out["type"] = json!(match shape {
                        PathShape::Circle => "radial",
                        PathShape::Rect => "rectangular",
                        PathShape::Shape => "path",
                    });
                    out["focus"] = json!(focus);
                }
            }
            json!({"gradient": out})
        }
    }
}

/// The directions Format Shape offers for a linear gradient, as angles (0 = left to right, 90 =
/// top to bottom).
pub const LINEAR_DIRECTIONS: [f32; 8] = [45.0, 90.0, 135.0, 0.0, 180.0, 315.0, 270.0, 225.0];

/// The directions Format Shape offers for radial and rectangular gradients, as where they start
/// (percent insets, left, top, right, bottom): each corner, then the centre.
pub const PATH_DIRECTIONS: [[f32; 4]; 5] =
    [[100.0, 100.0, 0.0, 0.0], [0.0, 100.0, 100.0, 0.0], [50.0, 50.0, 50.0, 50.0], [100.0, 0.0, 0.0, 100.0], [0.0, 0.0, 100.0, 100.0]];

fn grad(stops: &[(f32, Rgb)], kind: GradientKind) -> Fill {
    let stops = stops.iter().map(|&(pos, color)| GradStop { pos, color, transparency: 0.0 }).collect();
    Fill::Gradient(Arc::new(Gradient { stops, kind, tile: None, rot_with_shape: true, flip: GradientFlip::None }))
}

/// The gradient Format Shape's Gradient fill starts from, from the theme's first accent
/// whatever the shape's colour (as Word 365 does): four stops, 0% at 95% lighter, 74% and 83%
/// at 55% lighter, 100% at 70% lighter, top to bottom, scaled with the shape.
pub fn default_gradient(accent1: Rgb) -> Fill {
    let lighter = |off: f64| {
        let c = accent1.modified("lumMod", 1.0 - off);
        c.modified("lumOff", off)
    };
    let stops = [(0.0, 0.95), (74.0, 0.55), (83.0, 0.55), (100.0, 0.7)]
        .into_iter()
        .map(|(pos, off)| GradStop { pos, color: lighter(off), transparency: 0.0 })
        .collect();
    let g = Gradient { stops, kind: GradientKind::Linear { angle: 90.0, scaled: true }, tile: None, rot_with_shape: true, flip: GradientFlip::None };
    Fill::Gradient(Arc::new(g))
}

/// Where each corner tile's gradient starts (percent insets) and the tile rectangle Word gives
/// it: from the bottom-right, bottom-left, top-right and top-left corners. Observed for the
/// bottom-right corner (`fillToRect l=t=100%`, `tileRect r=b=-100%`); the others mirror it.
const CORNERS: [([f32; 4], [f32; 4]); 4] = [
    ([100.0, 100.0, 0.0, 0.0], [0.0, 0.0, -100.0, -100.0]),
    ([0.0, 100.0, 100.0, 0.0], [-100.0, 0.0, 0.0, -100.0]),
    ([100.0, 0.0, 0.0, 100.0], [0.0, -100.0, -100.0, 0.0]),
    ([0.0, 0.0, 100.0, 100.0], [-100.0, -100.0, 0.0, 0.0]),
];

/// The tile rectangle Word gives a radial or rectangular gradient starting at `focus`: a corner's,
/// or none from the centre.
pub fn corner_tile(focus: [f32; 4]) -> Option<[f32; 4]> {
    CORNERS.iter().find(|(f, _)| *f == focus).map(|(_, t)| *t)
}

/// Shape Fill › Gradient's Light and Dark Variations of `c`, each with its name, in Word's
/// order (a grid five wide): three stops at 0, 50 and 100%, light ones `c` tinted to 66, 44.5
/// and 23.5% with saturation ×1.6, dark ones shaded to 30, 67.5 and 100% with saturation
/// ×1.15 (as Word writes them); linear ones scaled with the shape, the rest circular.
pub fn variations(c: Rgb) -> [Vec<(Fill, String)>; 2] {
    let tiles = |m: &str, steps: [f64; 3], sat: f64| -> Vec<(Fill, String)> {
        let stop = |i: usize| {
            let color = c.modified(m, steps[i]).modified("satMod", sat);
            GradStop { pos: i as f32 * 50.0, color, transparency: 0.0 }
        };
        let fill = |kind, tile| {
            let g = Gradient { stops: (0..3).map(stop).collect(), kind, tile, rot_with_shape: true, flip: GradientFlip::None };
            Fill::Gradient(Arc::new(g))
        };
        let lin = |angle: f32| (fill(GradientKind::Linear { angle, scaled: true }, None), linear_direction_name(angle));
        let corner = |i: usize| {
            let (focus, tile) = CORNERS[i];
            (fill(GradientKind::Path { shape: PathShape::Circle, focus }, Some(tile)), path_direction_name(focus))
        };
        let centre = (fill(GradientKind::Path { shape: PathShape::Circle, focus: [50.0; 4] }, None), path_direction_name([50.0; 4]));
        vec![
            lin(45.0),
            lin(90.0),
            lin(135.0),
            corner(0),
            corner(1),
            lin(0.0),
            centre,
            lin(180.0),
            corner(2),
            corner(3),
            lin(315.0),
            lin(270.0),
            lin(225.0),
        ]
    };
    [tiles("tint", [0.66, 0.445, 0.235], 1.6), tiles("shade", [0.3, 0.675, 1.0], 1.15)]
}

/// The 30 preset gradients from the theme's six accents: per accent, soft, lit from the top,
/// deep, lit from the bottom and centre glow. Our own design (the repo's clean-room rule: no
/// copying Word's presets), on the same accents-by-styles grid as Word's.
pub fn presets(theme: &[Rgb]) -> Vec<Fill> {
    let accent = |i: usize| theme.get(4 + i).copied().unwrap_or(Rgb(68, 114, 196));
    let down = GradientKind::Linear { angle: 90.0, scaled: false };
    let mut out = Vec::with_capacity(30);
    for style in 0..5 {
        for i in 0..6 {
            let c = accent(i);
            out.push(match style {
                0 => grad(&[(0.0, c.modified("tint", 0.2)), (100.0, c.modified("tint", 0.6))], down),
                1 => grad(&[(0.0, c.modified("tint", 0.4)), (50.0, c), (100.0, c.modified("shade", 0.75))], down),
                2 => grad(&[(0.0, c), (100.0, c.modified("shade", 0.5))], down),
                3 => grad(&[(0.0, c.modified("shade", 0.75)), (50.0, c), (100.0, c.modified("tint", 0.4))], down),
                _ => grad(
                    &[(0.0, c.modified("tint", 0.4)), (100.0, c.modified("shade", 0.75))],
                    GradientKind::Path { shape: PathShape::Circle, focus: [50.0; 4] },
                ),
            });
        }
    }
    out
}

/// The preset styles' names, in [`presets`]'s order (each for accents 1 to 6).
pub const PRESET_STYLES: [&str; 5] = ["Soft", "Lit from Top", "Deep", "Lit from Bottom", "Center Glow"];

/// The name of the linear direction at `angle`.
pub fn linear_direction_name(angle: f32) -> String {
    match angle.round() as i32 {
        0 => "Linear Right",
        45 => "Linear Diagonal - Top Left to Bottom Right",
        90 => "Linear Down",
        135 => "Linear Diagonal - Top Right to Bottom Left",
        180 => "Linear Left",
        225 => "Linear Diagonal - Bottom Right to Top Left",
        270 => "Linear Up",
        315 => "Linear Diagonal - Bottom Left to Top Right",
        _ => "Linear",
    }
    .into()
}

/// The name of the radial or rectangular direction starting at `focus`.
pub fn path_direction_name(focus: [f32; 4]) -> String {
    match focus.map(|v| v.round() as i32) {
        [0, 0, 100, 100] => "From Top Left Corner",
        [100, 0, 0, 100] => "From Top Right Corner",
        [0, 100, 100, 0] => "From Bottom Left Corner",
        [100, 100, 0, 0] => "From Bottom Right Corner",
        _ => "From Center",
    }
    .into()
}

/// Stop editing for the Format Shape pane's gradient stops bar. Stops stay in position order; a
/// gradient keeps 2 to `MAX_STOPS` of them.
pub mod stops {
    use wordcraft_doc::props::Rgb;
    use wordcraft_doc::wordart::{GradStop, MAX_STOPS};

    /// The gradient's colour and transparency at `pos` (percent).
    pub fn at(stops: &[GradStop], pos: f32) -> (Rgb, f32) {
        let after = stops.iter().position(|s| s.pos >= pos);
        let (a, b) = match after {
            Some(0) | None => {
                let s = if after.is_none() { stops.last() } else { stops.first() };
                return s.map_or((Rgb::BLACK, 0.0), |s| (s.color, s.transparency));
            }
            Some(i) => (stops[i - 1], stops[i]),
        };
        let t = if b.pos > a.pos { (pos - a.pos) / (b.pos - a.pos) } else { 0.0 };
        let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round().clamp(0.0, 255.0) as u8;
        (Rgb(mix(a.color.0, b.color.0), mix(a.color.1, b.color.1), mix(a.color.2, b.color.2)), a.transparency + (b.transparency - a.transparency) * t)
    }

    /// Adds a stop at `pos` in the gradient's colour there; its index, or `None` when full.
    pub fn add(stops: &mut Vec<GradStop>, pos: f32) -> Option<usize> {
        if stops.len() >= MAX_STOPS {
            return None;
        }
        let pos = if pos.is_finite() { pos.clamp(0.0, 100.0) } else { 50.0 };
        let (color, transparency) = at(stops, pos);
        let i = stops.iter().position(|s| s.pos > pos).unwrap_or(stops.len());
        stops.insert(i, GradStop { pos, color, transparency });
        Some(i)
    }

    /// Removes stop `i` unless only two are left; whether it did.
    pub fn remove(stops: &mut Vec<GradStop>, i: usize) -> bool {
        if stops.len() <= 2 || i >= stops.len() {
            return false;
        }
        stops.remove(i);
        true
    }

    /// Moves stop `i` to `pos`, keeping the stops in order; its new index.
    pub fn move_to(stops: &mut Vec<GradStop>, i: usize, pos: f32) -> usize {
        if i >= stops.len() || !pos.is_finite() {
            return i;
        }
        let mut s = stops.remove(i);
        s.pos = pos.clamp(0.0, 100.0);
        let j = stops.iter().position(|o| o.pos > s.pos).unwrap_or(stops.len());
        stops.insert(j, s);
        j
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, params, parse, stops};
    use std::sync::Arc;
    use wordcraft_doc::fill::{Fill, Gradient, GradientFlip, GradientKind, PathShape};

    /// The variations Word writes for accent 1 (`tint`/`shade` with `satMod`, observed in Word 365).
    #[test]
    fn variations_follow_words() {
        use super::variations;
        let c = Rgb(0x15, 0x60, 0x82);
        let [light, dark] = variations(c);
        assert_eq!((light.len(), dark.len()), (13, 13));
        assert_eq!(light[1].1, "Linear Down");
        assert_eq!(light[6].1, "From Center");
        let Fill::Gradient(g) = &light[0].0 else { panic!() };
        assert_eq!(g.kind, GradientKind::Linear { angle: 45.0, scaled: true });
        assert_eq!(g.stops.iter().map(|s| s.pos).collect::<Vec<_>>(), [0.0, 50.0, 100.0]);
        assert!(g.stops[0].color.0 > g.stops[2].color.0 || g.stops[0].color.2 < g.stops[2].color.2, "lighter towards the end: {:?}", g.stops);
        let Fill::Gradient(g) = &light[3].0 else { panic!() };
        assert_eq!(
            (g.kind, g.tile),
            (GradientKind::Path { shape: PathShape::Circle, focus: [100.0, 100.0, 0.0, 0.0] }, Some([0.0, 0.0, -100.0, -100.0]))
        );
        let Fill::Gradient(g) = super::default_gradient(c) else { panic!() };
        assert_eq!(g.stops.iter().map(|s| s.pos).collect::<Vec<_>>(), [0.0, 74.0, 83.0, 100.0], "Word's starting gradient");
        assert_eq!(g.stops[1].color, g.stops[2].color);
        assert!(g.stops.iter().all(|s| s.color.0 > c.0 && s.color.1 > c.1), "lighter than the accent");
        let Fill::Gradient(g) = &dark[1].0 else { panic!() };
        assert_eq!(g.stops[2].color, c.modified("satMod", 1.15), "the last dark stop is the colour itself");
    }

    #[test]
    fn params_set_the_fill_they_were_made_from() {
        let g = |kind| {
            Fill::Gradient(Arc::new(Gradient {
                stops: vec![st(0.0, 10, 0.0), st(40.0, 90, 25.0), st(100.0, 250, 100.0)],
                kind,
                tile: None,
                rot_with_shape: false,
                flip: GradientFlip::None,
            }))
        };
        let fills = [
            Fill::Solid { color: Rgb(1, 2, 3), transparency: 35.0 },
            g(GradientKind::Linear { angle: 135.0, scaled: false }),
            g(GradientKind::Linear { angle: 45.0, scaled: true }),
            g(GradientKind::Path { shape: PathShape::Circle, focus: [100.0, 0.0, 0.0, 100.0] }),
            g(GradientKind::Path { shape: PathShape::Rect, focus: [50.0; 4] }),
            g(GradientKind::Path { shape: PathShape::Shape, focus: [50.0; 4] }),
        ];
        let tiled = super::variations(Rgb(9, 9, 9))[0][3].0.clone();
        for f in fills.into_iter().chain([tiled]) {
            let v = params(Some(&f));
            let back = match parse(&v) {
                Ok(Change::Fill(f)) => f,
                Ok(Change::Solid(c, t)) => Fill::Solid { color: c, transparency: t.unwrap_or(0.0) },
                _ => panic!("{v}"),
            };
            assert_eq!(back, f, "{v}");
        }
        assert!(matches!(parse(&params(None)), Ok(Change::None)));
    }
    use wordcraft_doc::props::Rgb;
    use wordcraft_doc::wordart::{GradStop, MAX_STOPS};

    fn st(pos: f32, c: u8, transparency: f32) -> GradStop {
        GradStop { pos, color: Rgb(c, c, c), transparency }
    }

    #[test]
    fn stops_are_added_in_the_gradients_colour_moved_in_order_and_kept_two_to_ten() {
        let mut g = vec![st(0.0, 0, 0.0), st(100.0, 200, 50.0)];
        assert_eq!(stops::add(&mut g, 25.0), Some(1));
        assert_eq!((g[1].color, g[1].transparency), (Rgb(50, 50, 50), 12.5));
        assert_eq!(stops::add(&mut g, f32::NAN), Some(2), "a junk position goes in the middle");
        assert_eq!(stops::move_to(&mut g, 1, 300.0), 3, "past the end: last");
        assert_eq!(g.iter().map(|s| s.pos).collect::<Vec<_>>(), [0.0, 50.0, 100.0, 100.0]);
        assert_eq!(stops::move_to(&mut g, 3, -1.0), 1, "after the stop already at 0%");
        assert!(stops::remove(&mut g, 0) && stops::remove(&mut g, 0));
        assert!(!stops::remove(&mut g, 0), "two stops stay");
        assert!(!stops::remove(&mut vec![st(0.0, 0, 0.0); 3], 7));
        while stops::add(&mut g, 10.0).is_some() {}
        assert_eq!(g.len(), MAX_STOPS);
        assert_eq!(stops::at(&g, 200.0).0, g[g.len() - 1].color);
        assert_eq!(stops::at(&[], 5.0), (Rgb::BLACK, 0.0));
    }
}
