//! Picture Effects (Picture Format › Picture Styles › Picture Effects): Preset, Shadow,
//! Reflection, Glow, Soft Edges, Bevel and 3-D Rotation, our own drawings of what each one does,
//! made on the picture's bitmap.
//!
//! They follow the picture's outline (its shape, grown by its frames and border) and its alpha: a
//! picture whose background was removed casts the shadow of what is left and glows round it. What
//! reaches outside the picture (an outer shadow, a glow, a reflection, a turn) grows the bitmap,
//! and [`Canvas::room`] says by how much on each side, so the frame grows with it and the picture
//! keeps its size and place on the page.
//!
//! Lengths are points, drawn at the picture's pixels per point ([`Scale`]); angles are degrees and
//! transparencies percent. Every value is clamped, blurs and glows are worked out on a copy of at
//! most [`MASK_PIXELS`] pixels, and the bitmap never grows past [`MAX_CANVAS_PIXELS`], so the work
//! is bounded whatever the picture or the parameters.

use image::RgbaImage;
use serde_json::Value;
use wordcraft_doc::effects::Glow;
use wordcraft_doc::props::Rgb;

use super::super::effects::blur;
use super::super::picture::too_large;
use crate::{CmdError, p};

/// The longest blur, distance, glow, soft edge or bevel, points.
pub const MAX_LEN: f32 = 100.0;
/// The most pixels a blur or glow's mask is worked out on.
const MASK_PIXELS: u64 = 1_000_000;
/// The most pixels the drawn picture may have.
const MAX_CANVAS_PIXELS: u64 = 60_000_000;
/// The longest length drawn, pixels.
const MAX_FX_PX: f32 = 4000.0;
/// The steepest a 3-D rotation tilts or turns the picture, degrees: past it the picture is edge on.
pub const MAX_TURN: f32 = 85.0;

/// A shadow: outside the picture (cast behind it) or inside it (along its edges).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PicShadow {
    pub inner: bool,
    pub color: Rgb,
    pub transparency: f32,
    pub blur: f32,
    pub distance: f32,
    /// The direction it is cast, clockwise from the right (y down).
    pub angle: f32,
    /// An outer shadow's size, percent of the picture's.
    pub size: f32,
    /// A perspective shadow: its height as a share of the picture's from the picture's bottom
    /// (negative: laid on the floor in front of it) and its lean, degrees.
    pub perspective: Option<(f32, f32)>,
}

impl Default for PicShadow {
    /// Offset: Bottom Right.
    fn default() -> Self {
        let d = wordcraft_doc::effects::Shadow::default();
        PicShadow {
            inner: false,
            color: d.color,
            transparency: d.transparency,
            blur: d.blur,
            distance: d.distance,
            angle: d.angle,
            size: 100.0,
            perspective: None,
        }
    }
}

/// A reflection under the picture: it starts `transparency` see-through and fades out over
/// `size` percent of the picture's height, `distance` below it, blurred by `blur`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PicReflection {
    pub transparency: f32,
    pub size: f32,
    pub distance: f32,
    pub blur: f32,
}

impl Default for PicReflection {
    fn default() -> Self {
        PicReflection { transparency: 50.0, size: 35.0, distance: 0.0, blur: 0.5 }
    }
}

/// The profile of a bevel's edge (ECMA-376 `ST_BevelPresetType` names), drawn our own way: how its
/// height rises from the outline inward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BevelKind {
    Circle,
    RelaxedInset,
    Cross,
    CoolSlant,
    Angle,
    SoftRound,
    Convex,
    Slope,
    Divot,
    Riblet,
    HardEdge,
    ArtDeco,
}

/// A bevel: its profile, how far in from the outline it reaches and how high it rises (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PicBevel {
    pub kind: BevelKind,
    pub width: f32,
    pub height: f32,
}

/// A 3-D rotation: the picture turned about its vertical axis (`x`: positive turns its right side
/// away), tilted about its horizontal one (`y`: positive tips its top away), turned in its own
/// plane (`z`, clockwise), and seen with a field of view of `perspective` degrees (0: parallel).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PicRotation {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub perspective: f32,
}

/// Everything Picture Effects puts on a picture; each one on its own.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PictureEffects {
    pub shadow: Option<PicShadow>,
    pub reflection: Option<PicReflection>,
    pub glow: Option<Glow>,
    /// Soft edges: the picture fades out over this distance inside its outline (points).
    pub soft_edge: Option<f32>,
    pub bevel: Option<PicBevel>,
    pub rotation: Option<PicRotation>,
}

impl PictureEffects {
    pub fn is_empty(&self) -> bool {
        *self == PictureEffects::default()
    }
}

/// Shadow › Outer.
pub const OUTER_SHADOWS: [(&str, &str); 9] = wordcraft_doc::effects::SHADOW_PRESETS;
/// Shadow › Inner.
pub const INNER_SHADOWS: [(&str, &str); 9] = [
    ("insideTopLeft", "Inside: Top Left"),
    ("insideTop", "Inside: Top"),
    ("insideTopRight", "Inside: Top Right"),
    ("insideLeft", "Inside: Left"),
    ("insideCenter", "Inside: Center"),
    ("insideRight", "Inside: Right"),
    ("insideBottomLeft", "Inside: Bottom Left"),
    ("insideBottom", "Inside: Bottom"),
    ("insideBottomRight", "Inside: Bottom Right"),
];
/// Shadow › Perspective.
pub const PERSPECTIVE_SHADOWS: [(&str, &str); 5] = [
    ("perspectiveUpperLeft", "Perspective: Upper Left"),
    ("perspectiveUpperRight", "Perspective: Upper Right"),
    ("perspectiveBelow", "Perspective: Below"),
    ("perspectiveLowerLeft", "Perspective: Lower Left"),
    ("perspectiveLowerRight", "Perspective: Lower Right"),
];
/// Reflection › Reflection Variations.
pub const REFLECTIONS: [(&str, &str); 9] = [
    ("tightTouching", "Tight Reflection: Touching"),
    ("halfTouching", "Half Reflection: Touching"),
    ("fullTouching", "Full Reflection: Touching"),
    ("tight4pt", "Tight Reflection: 4 pt Offset"),
    ("half4pt", "Half Reflection: 4 pt Offset"),
    ("full4pt", "Full Reflection: 4 pt Offset"),
    ("tight8pt", "Tight Reflection: 8 pt Offset"),
    ("half8pt", "Half Reflection: 8 pt Offset"),
    ("full8pt", "Full Reflection: 8 pt Offset"),
];
/// Glow › Glow Variations: the sizes (points), each in the theme's accents.
pub const GLOW_SIZES: [f32; 4] = [5.0, 8.0, 11.0, 18.0];
/// Soft Edges: the radii offered (points).
pub const SOFT_EDGES: [f32; 6] = [1.0, 2.5, 5.0, 10.0, 25.0, 50.0];
/// Bevel.
pub const BEVELS: [(&str, &str); 12] = [
    ("circle", "Circle"),
    ("relaxedInset", "Relaxed Inset"),
    ("cross", "Cross"),
    ("coolSlant", "Cool Slant"),
    ("angle", "Angle"),
    ("softRound", "Soft Round"),
    ("convex", "Convex"),
    ("slope", "Slope"),
    ("divot", "Divot"),
    ("riblet", "Riblet"),
    ("hardEdge", "Hard Edge"),
    ("artDeco", "Art Deco"),
];
/// 3-D Rotation › Parallel (ECMA-376 `ST_PresetCameraType` names).
pub const PARALLEL: [(&str, &str); 10] = [
    ("isometricLeftDown", "Isometric: Left Down"),
    ("isometricRightUp", "Isometric: Right Up"),
    ("isometricTopUp", "Isometric: Top Up"),
    ("isometricBottomDown", "Isometric: Bottom Down"),
    ("isometricOffAxis1Left", "Off Axis 1: Left"),
    ("isometricOffAxis1Right", "Off Axis 1: Right"),
    ("isometricOffAxis1Top", "Off Axis 1: Top"),
    ("isometricOffAxis2Left", "Off Axis 2: Left"),
    ("isometricOffAxis2Right", "Off Axis 2: Right"),
    ("isometricOffAxis2Top", "Off Axis 2: Top"),
];
/// 3-D Rotation › Perspective.
pub const PERSPECTIVES: [(&str, &str); 10] = [
    ("perspectiveLeft", "Perspective: Left"),
    ("perspectiveRight", "Perspective: Right"),
    ("perspectiveAbove", "Perspective: Above"),
    ("perspectiveBelow", "Perspective: Below"),
    ("perspectiveAboveLeftFacing", "Perspective: Above, Facing Left"),
    ("perspectiveAboveRightFacing", "Perspective: Above, Facing Right"),
    ("perspectiveContrastingLeftFacing", "Perspective: Contrasting Left"),
    ("perspectiveContrastingRightFacing", "Perspective: Contrasting Right"),
    ("perspectiveHeroicExtremeLeftFacing", "Perspective: Heroic Left"),
    ("perspectiveRelaxed", "Perspective: Relaxed"),
];
/// How many Preset combinations there are (`preset1` to `preset12`).
pub const PRESETS: usize = 12;

fn shadow_preset(id: &str) -> Option<PicShadow> {
    let d = PicShadow::default();
    if let Some(s) = wordcraft_doc::effects::Shadow::preset(id) {
        return Some(PicShadow { color: s.color, transparency: s.transparency, blur: s.blur, distance: s.distance, angle: s.angle, ..d });
    }
    let inner = |angle: f32| Some(PicShadow { inner: true, transparency: 50.0, blur: 5.0, distance: 4.0, angle, ..d });
    let persp = |height: f32, lean: f32| Some(PicShadow { transparency: 80.0, blur: 6.0, distance: 0.0, perspective: Some((height, lean)), ..d });
    match id {
        "insideTopLeft" => inner(45.0),
        "insideTop" => inner(90.0),
        "insideTopRight" => inner(135.0),
        "insideLeft" => inner(0.0),
        "insideCenter" => Some(PicShadow { inner: true, transparency: 50.0, blur: 9.0, distance: 0.0, angle: 0.0, ..d }),
        "insideRight" => inner(180.0),
        "insideBottomLeft" => inner(315.0),
        "insideBottom" => inner(270.0),
        "insideBottomRight" => inner(225.0),
        "perspectiveUpperLeft" => persp(0.75, -35.0),
        "perspectiveUpperRight" => persp(0.75, 35.0),
        "perspectiveBelow" => persp(-0.25, 0.0),
        "perspectiveLowerLeft" => persp(-0.35, -40.0),
        "perspectiveLowerRight" => persp(-0.35, 40.0),
        _ => None,
    }
}

fn reflection_preset(id: &str) -> Option<PicReflection> {
    let size = match id.get(..4) {
        Some("tigh") => 35.0,
        Some("half") => 55.0,
        Some("full") => 92.0,
        _ => return None,
    };
    let distance = if id.ends_with("Touching") {
        0.0
    } else if id.ends_with("4pt") {
        4.0
    } else if id.ends_with("8pt") {
        8.0
    } else {
        return None;
    };
    REFLECTIONS.iter().any(|(i, _)| *i == id).then_some(PicReflection { size, distance, ..PicReflection::default() })
}

fn bevel_kind(id: &str) -> Option<BevelKind> {
    Some(match id {
        "circle" => BevelKind::Circle,
        "relaxedInset" => BevelKind::RelaxedInset,
        "cross" => BevelKind::Cross,
        "coolSlant" => BevelKind::CoolSlant,
        "angle" => BevelKind::Angle,
        "softRound" => BevelKind::SoftRound,
        "convex" => BevelKind::Convex,
        "slope" => BevelKind::Slope,
        "divot" => BevelKind::Divot,
        "riblet" => BevelKind::Riblet,
        "hardEdge" => BevelKind::HardEdge,
        "artDeco" => BevelKind::ArtDeco,
        _ => return None,
    })
}

impl BevelKind {
    /// The name `bevel` takes.
    pub fn id(self) -> &'static str {
        match self {
            BevelKind::Circle => "circle",
            BevelKind::RelaxedInset => "relaxedInset",
            BevelKind::Cross => "cross",
            BevelKind::CoolSlant => "coolSlant",
            BevelKind::Angle => "angle",
            BevelKind::SoftRound => "softRound",
            BevelKind::Convex => "convex",
            BevelKind::Slope => "slope",
            BevelKind::Divot => "divot",
            BevelKind::Riblet => "riblet",
            BevelKind::HardEdge => "hardEdge",
            BevelKind::ArtDeco => "artDeco",
        }
    }

    /// How steeply the edge rises at `t` (0 at the outline, 1 where the bevel ends), in heights
    /// per width.
    fn slope(self, t: f32) -> f32 {
        use std::f32::consts::PI;
        let t = t.clamp(0.0, 1.0);
        match self {
            BevelKind::Circle => (1.0 - t) / (1.0 - (1.0 - t) * (1.0 - t)).sqrt().max(0.2),
            BevelKind::RelaxedInset => 0.25 * PI * (PI * t).sin(),
            BevelKind::Cross => {
                if t < 0.5 {
                    1.6
                } else {
                    -1.6
                }
            }
            BevelKind::CoolSlant => {
                if t < 0.6 {
                    1.4
                } else {
                    0.0
                }
            }
            BevelKind::Angle => 1.0,
            BevelKind::SoftRound => 5.4 * t * (1.0 - t),
            BevelKind::Convex => 2.0 * (1.0 - t),
            BevelKind::Slope => 2.0 * t,
            BevelKind::Divot => -0.6 * PI * (PI * t).cos(),
            BevelKind::Riblet => 0.9 * PI * (3.0 * PI * t).cos(),
            BevelKind::HardEdge => {
                if t < 0.3 {
                    3.0
                } else {
                    0.0
                }
            }
            BevelKind::ArtDeco => {
                if ((t * 5.0) as u32).is_multiple_of(2) {
                    1.8
                } else {
                    0.0
                }
            }
        }
    }
}

fn rotation_preset(id: &str) -> Option<PicRotation> {
    let par = |x: f32, y: f32| Some(PicRotation { x, y, z: 0.0, perspective: 0.0 });
    let per = |x: f32, y: f32, z: f32, fov: f32| Some(PicRotation { x, y, z, perspective: fov });
    match id {
        "isometricLeftDown" => par(45.0, 35.0),
        "isometricRightUp" => par(-45.0, -35.0),
        "isometricTopUp" => par(-45.0, 35.0),
        "isometricBottomDown" => par(45.0, -35.0),
        "isometricOffAxis1Left" => par(60.0, 18.0),
        "isometricOffAxis1Right" => par(-60.0, 18.0),
        "isometricOffAxis1Top" => par(20.0, 55.0),
        "isometricOffAxis2Left" => par(60.0, -18.0),
        "isometricOffAxis2Right" => par(-60.0, -18.0),
        "isometricOffAxis2Top" => par(-20.0, 55.0),
        "perspectiveLeft" => per(25.0, 0.0, 0.0, 45.0),
        "perspectiveRight" => per(-25.0, 0.0, 0.0, 45.0),
        "perspectiveAbove" => per(0.0, 30.0, 0.0, 45.0),
        "perspectiveBelow" => per(0.0, -30.0, 0.0, 45.0),
        "perspectiveAboveLeftFacing" => per(25.0, 25.0, 0.0, 45.0),
        "perspectiveAboveRightFacing" => per(-25.0, 25.0, 0.0, 45.0),
        "perspectiveContrastingLeftFacing" => per(40.0, 10.0, -8.0, 45.0),
        "perspectiveContrastingRightFacing" => per(-40.0, 10.0, 8.0, 45.0),
        "perspectiveHeroicExtremeLeftFacing" => per(30.0, -15.0, 0.0, 80.0),
        "perspectiveRelaxed" => per(0.0, 40.0, 0.0, 45.0),
        _ => None,
    }
}

/// Preset `n` (1 to [`PRESETS`]): our own combinations of the effects.
pub fn preset(n: usize) -> Option<PictureEffects> {
    let shadow = |id: &str| shadow_preset(id);
    let bevel = |kind: BevelKind, w: f32| Some(PicBevel { kind, width: w, height: w });
    let rot = |id: &str| rotation_preset(id);
    let refl = |id: &str| reflection_preset(id);
    let glow = Some(Glow { color: Rgb(0x15, 0x60, 0x82), size: 8.0, transparency: 60.0 });
    let none = PictureEffects::default();
    Some(match n {
        1 => PictureEffects { bevel: bevel(BevelKind::Circle, 6.0), ..none },
        2 => PictureEffects { bevel: bevel(BevelKind::SoftRound, 6.0), shadow: shadow("offsetBottomRight"), ..none },
        3 => PictureEffects { bevel: bevel(BevelKind::RelaxedInset, 8.0), shadow: shadow("offsetBottom"), ..none },
        4 => PictureEffects { bevel: bevel(BevelKind::Angle, 4.0), shadow: shadow("perspectiveBelow"), ..none },
        5 => PictureEffects { bevel: bevel(BevelKind::Convex, 6.0), reflection: refl("halfTouching"), ..none },
        6 => PictureEffects { bevel: bevel(BevelKind::SoftRound, 4.0), glow, ..none },
        7 => PictureEffects { rotation: rot("perspectiveRelaxed"), shadow: shadow("offsetBottom"), ..none },
        8 => PictureEffects { rotation: rot("perspectiveLeft"), bevel: bevel(BevelKind::Circle, 4.0), ..none },
        9 => PictureEffects { shadow: shadow("offsetCenter"), soft_edge: Some(5.0), ..none },
        10 => PictureEffects { rotation: rot("isometricOffAxis1Right"), shadow: shadow("offsetBottomRight"), ..none },
        11 => PictureEffects { bevel: bevel(BevelKind::ArtDeco, 8.0), reflection: refl("tightTouching"), ..none },
        12 => PictureEffects { rotation: rot("perspectiveRight"), reflection: refl("half4pt"), ..none },
        _ => return None,
    })
}

/// `v` (a finite number) clamped, or `d`.
fn num(v: &Value, k: &str, lo: f32, hi: f32, d: f32) -> f32 {
    p::f32(v, k).filter(|x| x.is_finite()).map_or(d, |x| x.clamp(lo, hi))
}

fn unknown(what: &str, id: &str, ids: &[&str]) -> CmdError {
    CmdError::Params(format!("unknown {what} `{id}`; use none or one of {}", ids.join(", ")))
}

/// What `picture.pictureEffects` with `v` does to `fx`: `preset` first (it replaces them all),
/// then each effect named: a preset name, an object of the parts to change (from the picture's
/// own, or the default), or `null` / `"none"` to take it off. An effect left out stays.
pub fn change(v: &Value, fx: &mut PictureEffects) -> Result<(), CmdError> {
    if let Some(pr) = v.get("preset") {
        match pr.as_str() {
            Some("none") => *fx = PictureEffects::default(),
            Some(id) => {
                let n = id.strip_prefix("preset").and_then(|n| n.parse::<usize>().ok());
                *fx = n.and_then(preset).ok_or_else(|| CmdError::Params(format!("unknown preset `{id}`; use none or preset1 to preset{PRESETS}")))?;
            }
            None if pr.is_null() => *fx = PictureEffects::default(),
            None => return Err(CmdError::Params("`preset`: none or preset1 to preset12".into())),
        }
    }
    let off = |e: &Value| e.is_null() || e.as_str() == Some("none") || e.as_bool() == Some(false);
    if let Some(e) = v.get("shadow") {
        fx.shadow = if off(e) {
            None
        } else if let Some(id) = e.as_str() {
            let ids: Vec<&str> = OUTER_SHADOWS.iter().chain(&INNER_SHADOWS).chain(&PERSPECTIVE_SHADOWS).map(|(i, _)| *i).collect();
            Some(shadow_preset(id).ok_or_else(|| unknown("shadow", id, &ids))?)
        } else if e.is_object() || e.as_bool() == Some(true) {
            let base = match p::str(e, "preset") {
                Some(id) => shadow_preset(id).ok_or_else(|| CmdError::Params(format!("unknown shadow preset `{id}`")))?,
                None => fx.shadow.unwrap_or_default(),
            };
            Some(shadow_parts(e, base))
        } else {
            return Err(CmdError::Params("`shadow`: a preset name, an object or null".into()));
        };
    }
    if let Some(e) = v.get("reflection") {
        fx.reflection = if off(e) {
            None
        } else if let Some(id) = e.as_str() {
            let ids: Vec<&str> = REFLECTIONS.iter().map(|(i, _)| *i).collect();
            Some(reflection_preset(id).ok_or_else(|| unknown("reflection", id, &ids))?)
        } else if e.is_object() || e.as_bool() == Some(true) {
            let b = match p::str(e, "preset") {
                Some(id) => reflection_preset(id).ok_or_else(|| CmdError::Params(format!("unknown reflection preset `{id}`")))?,
                None => fx.reflection.unwrap_or_default(),
            };
            Some(PicReflection {
                transparency: num(e, "transparency", 0.0, 100.0, b.transparency),
                size: num(e, "size", 1.0, 100.0, b.size),
                distance: num(e, "distance", 0.0, MAX_LEN, b.distance),
                blur: num(e, "blur", 0.0, MAX_LEN, b.blur),
            })
        } else {
            return Err(CmdError::Params("`reflection`: a preset name, an object or null".into()));
        };
    }
    if let Some(e) = v.get("glow") {
        let glow = if off(e) {
            None
        } else if let Some(n) = e.as_f64() {
            let b = fx.glow.unwrap_or_default();
            Some(Glow { size: if n.is_finite() { (n as f32).clamp(0.0, MAX_LEN) } else { b.size }, ..b })
        } else if e.is_object() || e.as_bool() == Some(true) {
            let b = fx.glow.unwrap_or_default();
            Some(Glow {
                color: p::str(e, "color").and_then(Rgb::parse).unwrap_or(b.color),
                size: num(e, "size", 0.0, MAX_LEN, b.size),
                transparency: num(e, "transparency", 0.0, 100.0, b.transparency),
            })
        } else {
            return Err(CmdError::Params("`glow`: a size in points, an object or null".into()));
        };
        fx.glow = glow.filter(|g| g.size > 0.0);
    }
    if let Some(e) = v.get("softEdge") {
        let soft = if off(e) {
            None
        } else if let Some(n) = e.as_f64() {
            Some(if n.is_finite() { (n as f32).clamp(0.0, MAX_LEN) } else { 0.0 })
        } else {
            return Err(CmdError::Params("`softEdge`: a radius in points or null".into()));
        };
        fx.soft_edge = soft.filter(|r| *r > 0.0);
    }
    if let Some(e) = v.get("bevel") {
        let ids: Vec<&str> = BEVELS.iter().map(|(i, _)| *i).collect();
        let bevel = if off(e) {
            None
        } else if let Some(id) = e.as_str() {
            Some(PicBevel { kind: bevel_kind(id).ok_or_else(|| unknown("bevel", id, &ids))?, width: 6.0, height: 6.0 })
        } else if e.is_object() || e.as_bool() == Some(true) {
            let b = fx.bevel.unwrap_or(PicBevel { kind: BevelKind::Circle, width: 6.0, height: 6.0 });
            let kind = match p::str(e, "preset") {
                Some(id) => bevel_kind(id).ok_or_else(|| unknown("bevel", id, &ids))?,
                None => b.kind,
            };
            Some(PicBevel { kind, width: num(e, "width", 0.0, MAX_LEN, b.width), height: num(e, "height", 0.0, MAX_LEN, b.height) })
        } else {
            return Err(CmdError::Params("`bevel`: a preset name, an object or null".into()));
        };
        fx.bevel = bevel.filter(|b| b.width > 0.0);
    }
    if let Some(e) = v.get("rotation") {
        let ids: Vec<&str> = PARALLEL.iter().chain(&PERSPECTIVES).map(|(i, _)| *i).collect();
        let rotation = if off(e) {
            None
        } else if let Some(id) = e.as_str() {
            Some(rotation_preset(id).ok_or_else(|| unknown("3-D rotation", id, &ids))?)
        } else if e.is_object() {
            let b = match p::str(e, "preset") {
                Some(id) => rotation_preset(id).ok_or_else(|| unknown("3-D rotation", id, &ids))?,
                None => fx.rotation.unwrap_or_default(),
            };
            let z = p::f32(e, "z").filter(|z| z.is_finite()).map_or(b.z, |z| z.clamp(-1.0e6, 1.0e6).rem_euclid(360.0));
            Some(PicRotation {
                x: num(e, "x", -MAX_TURN, MAX_TURN, b.x),
                y: num(e, "y", -MAX_TURN, MAX_TURN, b.y),
                z: if z.is_finite() && z < 360.0 { z } else { 0.0 },
                perspective: num(e, "perspective", 0.0, 120.0, b.perspective),
            })
        } else {
            return Err(CmdError::Params("`rotation`: a preset name, an object or null".into()));
        };
        fx.rotation = rotation.filter(|r| *r != PicRotation::default());
    }
    Ok(())
}

fn shadow_parts(e: &Value, b: PicShadow) -> PicShadow {
    let angle = p::f32(e, "angle").filter(|a| a.is_finite()).map_or(b.angle, |a| a.clamp(-1.0e6, 1.0e6).rem_euclid(360.0));
    PicShadow {
        inner: p::bool(e, "inner").unwrap_or(b.inner),
        color: p::str(e, "color").and_then(Rgb::parse).unwrap_or(b.color),
        transparency: num(e, "transparency", 0.0, 100.0, b.transparency),
        blur: num(e, "blur", 0.0, MAX_LEN, b.blur),
        distance: num(e, "distance", 0.0, MAX_LEN * 2.0, b.distance),
        angle: if angle.is_finite() && angle < 360.0 { angle } else { 0.0 },
        size: num(e, "size", 1.0, 200.0, b.size),
        perspective: b.perspective,
    }
}

/// The picture's outline shape (Picture Styles').
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Shape {
    #[default]
    Rect,
    /// Rounded corners, their radius a share of the shorter side.
    Rounded(f32),
    Oval,
    /// The top-left and bottom-right corners cut off, a share of the shorter side.
    SnipDiagonal(f32),
    /// The top-left and bottom-right corners rounded.
    RoundDiagonal(f32),
}

impl Shape {
    /// How far inside the `w` × `h` outline point (`x`, `y`) is (negative outside), pixels.
    pub fn depth(self, x: f32, y: f32, w: f32, h: f32) -> f32 {
        let (hw, hh) = ((w / 2.0).max(0.0), (h / 2.0).max(0.0));
        let (px, py) = (x - hw, y - hh);
        let short = w.min(h).max(0.0);
        match self {
            Shape::Rect => rounded_box(px, py, hw, hh, [0.0; 4]),
            Shape::Rounded(k) => rounded_box(px, py, hw, hh, [k * short; 4]),
            Shape::RoundDiagonal(k) => rounded_box(px, py, hw, hh, [k * short, 0.0, k * short, 0.0]),
            Shape::SnipDiagonal(k) => {
                let s = k * short;
                let tl = (x + y - s) / std::f32::consts::SQRT_2;
                let br = ((w - x) + (h - y) - s) / std::f32::consts::SQRT_2;
                rounded_box(px, py, hw, hh, [0.0; 4]).min(tl).min(br)
            }
            Shape::Oval => {
                let (a, b) = (hw.max(0.5), hh.max(0.5));
                let k0 = ((px / a).powi(2) + (py / b).powi(2)).sqrt();
                let k1 = ((px / (a * a)).powi(2) + (py / (b * b)).powi(2)).sqrt();
                if k1 <= 1e-6 { a.min(b) } else { -(k0 * (k0 - 1.0) / k1) }
            }
        }
    }
}

/// How far inside a box of half sizes `hw`, `hh` (centred on 0) point (`px`, `py`) is, with the
/// corners rounded by `r` (top left, top right, bottom right, bottom left).
fn rounded_box(px: f32, py: f32, hw: f32, hh: f32, r: [f32; 4]) -> f32 {
    let r = match (px >= 0.0, py >= 0.0) {
        (false, false) => r[0],
        (true, false) => r[1],
        (true, true) => r[2],
        (false, true) => r[3],
    };
    let r = r.min(hw).min(hh).max(0.0);
    let (qx, qy) = (px.abs() - hw + r, py.abs() - hh + r);
    // A square corner stays square outside too, so a frame round it has square corners.
    let outside = if r > 0.0 { qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r } else { qx.max(qy) };
    -outside
}

/// A line pattern for a border, in multiples of its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Dash {
    #[default]
    Solid,
    RoundDot,
    SquareDot,
    Dash,
    DashDot,
    LongDash,
    LongDashDot,
    LongDashDotDot,
}

impl Dash {
    /// The names `dash` takes.
    pub const NAMES: [&'static str; 8] = ["solid", "roundDot", "squareDot", "dash", "dashDot", "longDash", "longDashDot", "longDashDotDot"];

    pub fn parse(s: &str) -> Option<Dash> {
        Some(match s {
            "solid" => Dash::Solid,
            "roundDot" => Dash::RoundDot,
            "squareDot" => Dash::SquareDot,
            "dash" => Dash::Dash,
            "dashDot" => Dash::DashDot,
            "longDash" => Dash::LongDash,
            "longDashDot" => Dash::LongDashDot,
            "longDashDotDot" => Dash::LongDashDotDot,
            _ => return None,
        })
    }

    /// On and off lengths, in widths.
    fn pattern(self) -> &'static [f32] {
        match self {
            Dash::Solid => &[],
            Dash::RoundDot => &[1.0, 2.0],
            Dash::SquareDot => &[1.0, 1.0],
            Dash::Dash => &[4.0, 3.0],
            Dash::DashDot => &[4.0, 3.0, 1.0, 3.0],
            Dash::LongDash => &[8.0, 3.0],
            Dash::LongDashDot => &[8.0, 3.0, 1.0, 3.0],
            Dash::LongDashDotDot => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        }
    }

    /// How much of a line `w` wide shows at `s` along it and `u` across it (0 inside to 1
    /// outside).
    fn coverage(self, s: f32, u: f32, w: f32) -> f32 {
        let pat = self.pattern();
        let period: f32 = pat.iter().sum::<f32>() * w;
        if pat.is_empty() || !period.is_finite() || period <= 0.0 {
            return 1.0;
        }
        let mut at = s.rem_euclid(period);
        for (i, len) in pat.iter().enumerate() {
            let len = len * w;
            if at < len {
                if i % 2 == 1 {
                    return 0.0;
                }
                if self == Dash::RoundDot {
                    // A round dot: a disc the line's width across.
                    let (along, across) = (at - len / 2.0, (u - 0.5) * w);
                    return (w / 2.0 - along.hypot(across) + 0.5).clamp(0.0, 1.0);
                }
                return 1.0;
            }
            at -= len;
        }
        0.0
    }
}

/// How lengths in points are drawn: pixels per point, and the longest length worth drawing.
#[derive(Clone, Copy, Debug)]
pub struct Scale {
    pub ppp: f32,
    cap: f32,
}

impl Scale {
    /// For a picture whose visible part is `vis` pixels and shows `size` points.
    pub fn new(vis: (u32, u32), size: (f32, f32)) -> Scale {
        let per = |px: u32, pt: f32| if pt.is_finite() && pt > 0.0 { px as f32 / pt } else { 0.0 };
        let (kx, ky) = (per(vis.0, size.0), per(vis.1, size.1));
        let ppp = match (kx > 0.0, ky > 0.0) {
            (true, true) => (kx + ky) / 2.0,
            (true, false) => kx,
            (false, true) => ky,
            _ => 4.0 / 3.0,
        };
        let long = vis.0.max(vis.1) as f32;
        Scale { ppp: ppp.clamp(0.001, 10_000.0), cap: MAX_FX_PX.min(long * 4.0 + 64.0) }
    }

    /// `pt` points in pixels.
    pub fn px(&self, pt: f32) -> f32 {
        if pt.is_finite() { (pt * self.ppp).clamp(0.0, self.cap) } else { 0.0 }
    }
}

/// A picture being drawn: its bitmap, where the part its crop shows (the window) is in it, and
/// the outline drawn so far.
pub struct Canvas {
    pub img: RgbaImage,
    /// The window's top-left corner in the bitmap.
    pub at: (u32, u32),
    /// The window's size.
    pub win: (u32, u32),
    /// The outline's shape (round the window), grown by this many pixels by frames and borders.
    pub shape: Shape,
    pub grow: f32,
}

impl Canvas {
    pub fn new(img: RgbaImage) -> Canvas {
        let win = img.dimensions();
        Canvas { img, at: (0, 0), win, shape: Shape::Rect, grow: 0.0 }
    }

    /// How far the bitmap reaches past the window on each side (left, top, right, bottom), pixels.
    pub fn room(&self) -> [u32; 4] {
        let (w, h) = self.img.dimensions();
        [self.at.0, self.at.1, w.saturating_sub(self.at.0 + self.win.0), h.saturating_sub(self.at.1 + self.win.1)]
    }

    /// How far inside the outline the centre of pixel (`x`, `y`) is (negative outside).
    pub fn depth(&self, x: u32, y: u32) -> f32 {
        self.depth_at(x as f32 + 0.5, y as f32 + 0.5)
    }

    fn depth_at(&self, x: f32, y: f32) -> f32 {
        let (ox, oy) = (self.at.0 as f32, self.at.1 as f32);
        self.shape.depth(x - ox, y - oy, self.win.0 as f32, self.win.1 as f32) + self.grow
    }

    /// The bitmap grown by `pad` pixels (left, top, right, bottom), new pixels clear.
    pub fn pad(&mut self, pad: [u32; 4]) -> Result<(), CmdError> {
        if pad == [0; 4] {
            return Ok(());
        }
        let (w, h) = self.img.dimensions();
        let nw = w.checked_add(pad[0]).and_then(|v| v.checked_add(pad[2])).ok_or_else(too_large)?;
        let nh = h.checked_add(pad[1]).and_then(|v| v.checked_add(pad[3])).ok_or_else(too_large)?;
        fits(nw, nh)?;
        let mut out = RgbaImage::new(nw, nh);
        image::imageops::replace(&mut out, &self.img, i64::from(pad[0]), i64::from(pad[1]));
        self.img = out;
        self.at = (self.at.0 + pad[0], self.at.1 + pad[1]);
        Ok(())
    }

    /// The window's pixels kept inside `shape` (Picture Styles' rounded or oval pictures).
    pub fn mask_shape(&mut self, shape: Shape) {
        self.shape = shape;
        if shape == Shape::Rect {
            return;
        }
        let (x0, y0, x1, y1) = (self.at.0, self.at.1, self.at.0 + self.win.0, self.at.1 + self.win.1);
        let me = &*self;
        let k: Vec<(u32, u32, f32)> =
            (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y, (me.depth(x, y) + 0.5).clamp(0.0, 1.0)))).filter(|(_, _, k)| *k < 1.0).collect();
        for (x, y, k) in k {
            if let Some(p) = self.img.get_pixel_mut_checked(x, y) {
                p.0[3] = (f32::from(p.0[3]) * k).round() as u8;
            }
        }
    }

    /// A line `w` pixels wide of `color` round the outline, outside it, in `dash`; `metal` shades
    /// it like a polished frame. The outline grows by `w`.
    pub fn ring(&mut self, color: Rgb, w: f32, dash: Dash, metal: bool) -> Result<(), CmdError> {
        if !w.is_finite() || w < 0.25 {
            return Ok(());
        }
        let pad = w.ceil() as u32;
        self.pad([pad; 4])?;
        let (ox, oy) = (self.at.0 as f32 - self.grow, self.at.1 as f32 - self.grow);
        let (bw, bh) = (self.win.0 as f32 + 2.0 * self.grow, self.win.1 as f32 + 2.0 * self.grow);
        let oval = self.shape == Shape::Oval;
        let perimeter = if oval {
            // Ramanujan's.
            let (a, b) = ((bw + w) / 2.0, (bh + w) / 2.0);
            std::f32::consts::PI * (3.0 * (a + b) - ((3.0 * a + b) * (a + 3.0 * b)).max(0.0).sqrt())
        } else {
            2.0 * (bw + bh)
        };
        let (w_img, h_img) = self.img.dimensions();
        let mut paint = Vec::new();
        for y in 0..h_img {
            for x in 0..w_img {
                let d = self.depth(x, y);
                let cov = (d + w + 0.5).clamp(0.0, 1.0) - (d + 0.5).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                // Where along the outline the pixel is.
                let s = if oval {
                    let (cx, cy) = (ox + bw / 2.0, oy + bh / 2.0);
                    ((fy - cy).atan2(fx - cx) / std::f32::consts::TAU).rem_euclid(1.0) * perimeter
                } else {
                    let (qx, qy) = (fx.clamp(ox, ox + bw) - ox, fy.clamp(oy, oy + bh) - oy);
                    if qy <= 0.0 {
                        qx
                    } else if qx >= bw {
                        bw + qy
                    } else if qy >= bh {
                        bw + bh + (bw - qx)
                    } else {
                        2.0 * bw + bh + (bh - qy)
                    }
                };
                let u = (-d / w).clamp(0.0, 1.0);
                let cov = cov * dash.coverage(s, u, w);
                if cov <= 0.0 {
                    continue;
                }
                let c = if metal { self.metal(x, y, u) } else { [color.0, color.1, color.2] };
                paint.push((x, y, [c[0], c[1], c[2], (cov * 255.0).round() as u8]));
            }
        }
        for (x, y, ring) in paint {
            if let Some(p) = self.img.get_pixel_mut_checked(x, y) {
                p.0 = over(p.0, ring);
            }
        }
        self.grow += w;
        Ok(())
    }

    /// A polished frame's grey at `u` across it (0 inside, 1 outside), lit from the top left.
    fn metal(&self, x: u32, y: u32, u: f32) -> [u8; 3] {
        let ridge = (1.0 - (2.0 * u - 1.0).abs()).powf(0.6);
        let (gx, gy) = self.normal(x as f32 + 0.5, y as f32 + 0.5);
        // The outward side facing the light (top left) is lit, the other in shade.
        let side = (gx * 0.7 + gy * 0.7).clamp(-1.0, 1.0);
        let v = (95.0 + 125.0 * ridge + 45.0 * side).clamp(0.0, 255.0);
        [v as u8, (v * 1.01).min(255.0) as u8, (v * 1.05).min(255.0) as u8]
    }

    /// The unit direction into the outline at a point (the depth's gradient), or zero.
    fn normal(&self, x: f32, y: f32) -> (f32, f32) {
        let gx = (self.depth_at(x + 1.0, y) - self.depth_at(x - 1.0, y)) / 2.0;
        let gy = (self.depth_at(x, y + 1.0) - self.depth_at(x, y - 1.0)) / 2.0;
        let len = gx.hypot(gy);
        if len.is_finite() && len > 1e-4 { (gx / len, gy / len) } else { (0.0, 0.0) }
    }
}

/// Whether a `w` × `h` picture may be drawn.
fn fits(w: u32, h: u32) -> Result<(), CmdError> {
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_CANVAS_PIXELS || !wordcraft_geom::decode_fits(w, h, wordcraft_geom::MAX_DECODED_BYTES) {
        return Err(too_large());
    }
    Ok(())
}

/// `top` over `bottom` (straight alpha).
fn over(bottom: [u8; 4], top: [u8; 4]) -> [u8; 4] {
    let ta = f32::from(top[3]) / 255.0;
    if ta >= 1.0 {
        return top;
    }
    let ba = f32::from(bottom[3]) / 255.0;
    let a = ta + ba * (1.0 - ta);
    if a <= 0.0 {
        return [0; 4];
    }
    let mix = |t: u8, b: u8| ((f32::from(t) * ta + f32::from(b) * ba * (1.0 - ta)) / a).round().clamp(0.0, 255.0) as u8;
    [mix(top[0], bottom[0]), mix(top[1], bottom[1]), mix(top[2], bottom[2]), (a * 255.0).round().clamp(0.0, 255.0) as u8]
}

/// Everything in `fx` on the canvas, in order: soft edges, bevel, an inner shadow, the 3-D
/// rotation, then what is cast outside (shadow, glow, reflection).
pub fn draw(cv: &mut Canvas, fx: &PictureEffects, sc: Scale) -> Result<(), CmdError> {
    if let Some(r) = fx.soft_edge {
        soft_edge(cv, sc.px(r));
    }
    if let Some(b) = fx.bevel {
        bevel(cv, b, sc);
    }
    if let Some(sh) = fx.shadow.filter(|s| s.inner) {
        inner_shadow(cv, sh, sc);
    }
    if let Some(r) = fx.rotation {
        rotate(cv, r)?;
    }
    outer(cv, fx, sc)
}

/// Soft edges: everything inside the outline fades out toward it over `r` pixels.
fn soft_edge(cv: &mut Canvas, r: f32) {
    if r < 0.5 {
        return;
    }
    let (w, h) = cv.img.dimensions();
    let k: Vec<f32> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| cv.depth(x, y)).collect();
    for (p, d) in cv.img.pixels_mut().zip(k) {
        if d < 0.0 {
            continue;
        }
        let t = (d / r).clamp(0.0, 1.0);
        let k = t * t * (3.0 - 2.0 * t);
        p.0[3] = (f32::from(p.0[3]) * k).round() as u8;
    }
}

/// A bevel: the picture near its outline lit from the top left as if its edge rose in `b`'s
/// profile.
fn bevel(cv: &mut Canvas, b: PicBevel, sc: Scale) {
    let (bw, bh) = (sc.px(b.width), sc.px(b.height));
    if bw < 0.5 {
        return;
    }
    let l = {
        let (x, y, z) = (-0.55f32, -0.65f32, 0.52f32);
        let n = (x * x + y * y + z * z).sqrt();
        (x / n, y / n, z / n)
    };
    let (w, h) = cv.img.dimensions();
    let mut shade = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let d = cv.depth(x, y);
            if !(0.0..bw).contains(&d) {
                continue;
            }
            let s = b.kind.slope(d / bw) * bh / bw;
            let (gx, gy) = cv.normal(x as f32 + 0.5, y as f32 + 0.5);
            // The surface rises inward along (gx, gy): its normal leans outward.
            let (nx, ny, nz) = (-s * gx, -s * gy, 1.0f32);
            let n = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            let k = ((nx * l.0 + ny * l.1 + nz * l.2) / n - l.2) * 1.4;
            if k.is_finite() && k.abs() > 1e-3 {
                shade.push((x, y, k.clamp(-0.85, 0.85)));
            }
        }
    }
    for (x, y, k) in shade {
        if let Some(p) = cv.img.get_pixel_mut_checked(x, y) {
            for c in p.0.iter_mut().take(3) {
                let v = f32::from(*c);
                *c = if k > 0.0 { v + (255.0 - v) * k } else { v * (1.0 + k) }.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// A plane of values (alpha, a mask) over the canvas at `k` of its pixels per pixel, its first
/// cell's top-left at canvas point (`ox`, `oy`).
struct Plane {
    w: usize,
    h: usize,
    k: f32,
    ox: f32,
    oy: f32,
    v: Vec<f32>,
}

impl Plane {
    /// A plane over canvas rect (`ox`, `oy`) `cw` × `ch` at scale `k`, every cell `fill`.
    fn new(ox: f32, oy: f32, cw: f32, ch: f32, k: f32, fill: f32) -> Plane {
        let w = ((cw * k).ceil().max(1.0) as usize).min(1 << 15);
        let h = ((ch * k).ceil().max(1.0) as usize).min(1 << 15);
        Plane { w, h, k, ox, oy, v: vec![fill; w.saturating_mul(h).min(64 << 20)] }
    }

    /// The canvas point at the centre of cell (`i`, `j`).
    fn centre(&self, i: usize, j: usize) -> (f32, f32) {
        (self.ox + (i as f32 + 0.5) / self.k, self.oy + (j as f32 + 0.5) / self.k)
    }

    /// The value at canvas point (`x`, `y`), between the cells; `outside` past the plane.
    fn at(&self, x: f32, y: f32, outside: f32) -> f32 {
        let fx = (x - self.ox) * self.k - 0.5;
        let fy = (y - self.oy) * self.k - 0.5;
        if !fx.is_finite() || !fy.is_finite() {
            return outside;
        }
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let cell = |xi: f32, yi: f32| {
            if xi < 0.0 || yi < 0.0 || xi >= self.w as f32 || yi >= self.h as f32 {
                outside
            } else {
                self.v.get(yi as usize * self.w + xi as usize).copied().unwrap_or(outside)
            }
        };
        let top = cell(x0, y0) * (1.0 - tx) + cell(x0 + 1.0, y0) * tx;
        let bottom = cell(x0, y0 + 1.0) * (1.0 - tx) + cell(x0 + 1.0, y0 + 1.0) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    /// Blurred with a box of `r` canvas pixels, twice.
    fn blurred(mut self, r: f32) -> Plane {
        let r = (r * self.k).round();
        if r >= 1.0 && self.w > 0 && self.h > 0 {
            self.v = blur(&self.v, self.w as u32, self.h as u32, r.min(4096.0) as usize);
        }
        self
    }
}

/// The scale masks are worked out at for a `w` × `h` canvas.
fn mask_scale(w: f32, h: f32) -> f32 {
    let n = (w * h).max(1.0);
    (MASK_PIXELS as f32 / n).sqrt().min(1.0)
}

/// The canvas's alpha (0 to 1) on a plane at scale `k`, `pad` canvas pixels round it of `fill`;
/// `invert`: one minus it.
fn alpha_plane(img: &RgbaImage, k: f32, pad: f32, fill: f32, invert: bool) -> Plane {
    let (w, h) = img.dimensions();
    let mut plane = Plane::new(-pad, -pad, w as f32 + 2.0 * pad, h as f32 + 2.0 * pad, k, fill);
    let mut count = vec![0u32; plane.v.len()];
    let mut sum = vec![0.0f32; plane.v.len()];
    for (x, y, p) in img.enumerate_pixels() {
        let i = ((x as f32 + 0.5 + pad) * k) as usize;
        let j = ((y as f32 + 0.5 + pad) * k) as usize;
        if i < plane.w && j < plane.h {
            let at = j * plane.w + i;
            let a = f32::from(p.0[3]) / 255.0;
            if let (Some(c), Some(s)) = (count.get_mut(at), sum.get_mut(at)) {
                *c += 1;
                *s += if invert { 1.0 - a } else { a };
            }
        }
    }
    for ((v, c), s) in plane.v.iter_mut().zip(count).zip(sum) {
        if c > 0 {
            *v = s / c as f32;
        }
    }
    plane
}

/// An inner shadow: the outside of the picture cast inward along its edges, inside it only.
fn inner_shadow(cv: &mut Canvas, sh: PicShadow, sc: Scale) {
    let (blur_px, dist) = (sc.px(sh.blur), sc.px(sh.distance));
    let opacity = 1.0 - sh.transparency.clamp(0.0, 100.0) / 100.0;
    if opacity <= 0.0 {
        return;
    }
    let (w, h) = cv.img.dimensions();
    let k = mask_scale(w as f32, h as f32);
    let a = sh.angle.to_radians();
    let (dx, dy) = (dist * a.cos(), dist * a.sin());
    let pad = blur_px * 1.5 + dist + 2.0;
    let outside = alpha_plane(&cv.img, k, pad, 1.0, true).blurred(blur_px / 2.0);
    let c = [f32::from(sh.color.0), f32::from(sh.color.1), f32::from(sh.color.2)];
    for (x, y, p) in cv.img.enumerate_pixels_mut() {
        if p.0[3] == 0 {
            continue;
        }
        let s = outside.at(x as f32 + 0.5 - dx, y as f32 + 0.5 - dy, 1.0).clamp(0.0, 1.0) * opacity;
        if s > 0.0 {
            for (ch, col) in p.0.iter_mut().take(3).zip(c) {
                *ch = (f32::from(*ch) * (1.0 - s) + col * s).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// The picture turned in three dimensions about its window's centre, drawn into a bitmap holding
/// what it covers and the window.
fn rotate(cv: &mut Canvas, rot: PicRotation) -> Result<(), CmdError> {
    let (w, h) = cv.img.dimensions();
    let (cx, cy) = (cv.at.0 as f32 + cv.win.0 as f32 / 2.0, cv.at.1 as f32 + cv.win.1 as f32 / 2.0);
    let (ax, ay, az) = (rot.x.clamp(-MAX_TURN, MAX_TURN).to_radians(), rot.y.clamp(-MAX_TURN, MAX_TURN).to_radians(), rot.z.to_radians());
    let diag = (w as f32).hypot(h as f32).max(1.0);
    let fov = rot.perspective.clamp(0.0, 120.0);
    // The camera: far enough that the whole picture stays in front of it.
    let dist = if fov > 0.5 { ((diag / 2.0) / (fov.to_radians() / 2.0).tan()).max(diag * 0.75) } else { 0.0 };
    let project = |x: f32, y: f32| {
        let (x, y) = (x - cx, y - cy);
        // In its plane, then tilted (top away), then turned (right side away).
        let (x, y) = (x * az.cos() - y * az.sin(), x * az.sin() + y * az.cos());
        let (y, z) = (y * ay.cos(), y * ay.sin());
        let (x, z) = (x * ax.cos() + z * ax.sin(), z * ax.cos() - x * ax.sin());
        let k = if dist > 0.0 { dist / (dist - z).max(dist * 0.05) } else { 1.0 };
        (cx + x * k, cy + y * k)
    };
    let quad = [project(0.0, 0.0), project(w as f32, 0.0), project(w as f32, h as f32), project(0.0, h as f32)];
    if quad.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return Ok(());
    }
    let Some(inv) = square_to_quad(quad).and_then(invert3) else { return Ok(()) };
    // The new bitmap: what the turned picture covers, and the window.
    let (win_x0, win_y0) = (cv.at.0 as f32, cv.at.1 as f32);
    let (win_x1, win_y1) = (win_x0 + cv.win.0 as f32, win_y0 + cv.win.1 as f32);
    let x0 = quad.iter().map(|q| q.0).fold(win_x0, f32::min).floor();
    let y0 = quad.iter().map(|q| q.1).fold(win_y0, f32::min).floor();
    let x1 = quad.iter().map(|q| q.0).fold(win_x1, f32::max).ceil();
    let y1 = quad.iter().map(|q| q.1).fold(win_y1, f32::max).ceil();
    let (nw, nh) = ((x1 - x0).clamp(1.0, 1.0e6) as u32, (y1 - y0).clamp(1.0, 1.0e6) as u32);
    fits(nw, nh)?;
    let src = &cv.img;
    let out = RgbaImage::from_fn(nw, nh, |i, j| {
        let (px, py) = (x0 + i as f32 + 0.5, y0 + j as f32 + 0.5);
        let hx = inv[0] * px + inv[1] * py + inv[2];
        let hy = inv[3] * px + inv[4] * py + inv[5];
        let hw = inv[6] * px + inv[7] * py + inv[8];
        if hw.abs() < 1e-9 {
            return image::Rgba([0; 4]);
        }
        let (u, v) = (hx / hw, hy / hw);
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return image::Rgba([0; 4]);
        }
        image::Rgba(sample(src, u * w as f32 - 0.5, v * h as f32 - 0.5))
    });
    let new_at = ((win_x0 - x0).max(0.0) as u32, (win_y0 - y0).max(0.0) as u32);
    cv.img = out;
    cv.at = new_at;
    Ok(())
}

/// `img` at (`x`, `y`), between pixels (premultiplied while mixing), clear outside.
fn sample(img: &RgbaImage, x: f32, y: f32) -> [u8; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let px = |xi: f32, yi: f32| -> [f32; 4] {
        if xi < 0.0 || yi < 0.0 {
            return [0.0; 4];
        }
        match img.get_pixel_checked(xi as u32, yi as u32) {
            Some(p) => {
                let a = f32::from(p.0[3]) / 255.0;
                [f32::from(p.0[0]) * a, f32::from(p.0[1]) * a, f32::from(p.0[2]) * a, a]
            }
            None => [0.0; 4],
        }
    };
    let (a, b, c, d) = (px(x0, y0), px(x0 + 1.0, y0), px(x0, y0 + 1.0), px(x0 + 1.0, y0 + 1.0));
    let mut m = [0.0f32; 4];
    for (i, v) in m.iter_mut().enumerate() {
        let g = |q: [f32; 4]| q.get(i).copied().unwrap_or(0.0);
        *v = (g(a) * (1.0 - tx) + g(b) * tx) * (1.0 - ty) + (g(c) * (1.0 - tx) + g(d) * tx) * ty;
    }
    let alpha = m[3];
    if alpha <= 1e-4 {
        return [0; 4];
    }
    let un = |v: f32| (v / alpha).round().clamp(0.0, 255.0) as u8;
    [un(m[0]), un(m[1]), un(m[2]), (alpha * 255.0).round().clamp(0.0, 255.0) as u8]
}

/// The projective map taking the unit square's corners (0,0), (1,0), (1,1), (0,1) to `q`'s
/// (Heckbert's square-to-quad), as a row-major 3 × 3 matrix.
fn square_to_quad(q: [(f32, f32); 4]) -> Option<[f32; 9]> {
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = q;
    let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
    let (g, h) = if sx.abs() < 1e-6 && sy.abs() < 1e-6 {
        (0.0, 0.0)
    } else {
        let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-9 {
            return None;
        }
        ((sx * dy2 - dx2 * sy) / den, (dx1 * sy - sx * dy1) / den)
    };
    Some([x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h, 1.0])
}

/// The inverse of a row-major 3 × 3 matrix, None when it has none.
fn invert3(m: [f32; 9]) -> Option<[f32; 9]> {
    let [a, b, c, d, e, f, g, h, i] = m.map(f64::from);
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let r = [e * i - f * h, c * h - b * i, b * f - c * e, f * g - d * i, a * i - c * g, c * d - a * f, d * h - e * g, b * g - a * h, a * e - b * d];
    Some(r.map(|v| (v / det) as f32))
}

/// The rows and columns of `img` holding anything (alpha above 0): x0, y0, x1, y1 (exclusive).
fn content(img: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

/// An affine map, row-major 2 × 3: x' = a·x + b·y + c, y' = d·x + e·y + f.
type Affine = [f32; 6];

fn apply(m: &Affine, x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5])
}

fn invert(m: &Affine) -> Option<Affine> {
    let det = m[0] * m[4] - m[1] * m[3];
    if !det.is_finite() || det.abs() < 1e-9 {
        return None;
    }
    let (a, b, d, e) = (m[4] / det, -m[1] / det, -m[3] / det, m[0] / det);
    Some([a, b, -(a * m[2] + b * m[5]), d, e, -(d * m[2] + e * m[5])])
}

/// What is cast outside the picture: its shadow (behind), its glow (behind), and its reflection
/// (below). The bitmap grows to hold them.
fn outer(cv: &mut Canvas, fx: &PictureEffects, sc: Scale) -> Result<(), CmdError> {
    let shadow = fx.shadow.filter(|s| !s.inner && s.transparency < 100.0);
    let glow = fx.glow.filter(|g| g.size > 0.0 && g.transparency < 100.0);
    let refl = fx.reflection.filter(|r| r.transparency < 100.0);
    if shadow.is_none() && glow.is_none() && refl.is_none() {
        return Ok(());
    }
    let Some((cx0, cy0, cx1, cy1)) = content(&cv.img) else { return Ok(()) };
    let (w, h) = cv.img.dimensions();
    let (wf, hf) = (w as f32, h as f32);
    let k = mask_scale(wf, hf);
    let alpha = alpha_plane(&cv.img, k, 0.0, 0.0, false);
    // Each layer's extent in canvas coordinates: x0, y0, x1, y1.
    let mut bounds = (0.0f32, 0.0f32, wf, hf);
    let mut grow = |b: (f32, f32, f32, f32)| {
        bounds = (bounds.0.min(b.0), bounds.1.min(b.1), bounds.2.max(b.2), bounds.3.max(b.3));
    };
    let shadow_layer = shadow.and_then(|sh| {
        let blur_px = sc.px(sh.blur);
        let dist = sc.px(sh.distance);
        let a = sh.angle.to_radians();
        let (dx, dy) = (dist * a.cos(), dist * a.sin());
        let s = sh.size.clamp(1.0, 200.0) / 100.0;
        let (mx, my) = ((cx0 + cx1) as f32 / 2.0, (cy0 + cy1) as f32 / 2.0);
        let m: Affine = match sh.perspective {
            // From the content's bottom edge: up the wall behind (positive) or along the floor.
            Some((height, lean)) => {
                let sy = if height.abs() < 0.05 { 0.05f32.copysign(height) } else { height.clamp(-1.0, 1.0) };
                let lean = lean.clamp(-80.0, 80.0).to_radians().tan() * sy.abs();
                let yb = cy1 as f32;
                [s, lean, mx - s * mx - lean * yb + dx, 0.0, sy, yb * (1.0 - sy) + dy]
            }
            None => [s, 0.0, mx - s * mx + dx, 0.0, s, my - s * my + dy],
        };
        let inv = invert(&m)?;
        let corners = [apply(&m, 0.0, 0.0), apply(&m, wf, 0.0), apply(&m, wf, hf), apply(&m, 0.0, hf)];
        let reach = blur_px * 1.5 + 2.0;
        let x0 = corners.iter().map(|c| c.0).fold(f32::MAX, f32::min) - reach;
        let y0 = corners.iter().map(|c| c.1).fold(f32::MAX, f32::min) - reach;
        let x1 = corners.iter().map(|c| c.0).fold(f32::MIN, f32::max) + reach;
        let y1 = corners.iter().map(|c| c.1).fold(f32::MIN, f32::max) + reach;
        if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (x0, y0, x1, y1) = (x0.max(-wf * 3.0 - 4096.0), y0.max(-hf * 3.0 - 4096.0), x1.min(wf * 4.0 + 4096.0), y1.min(hf * 4.0 + 4096.0));
        let mut plane = Plane::new(x0, y0, x1 - x0, y1 - y0, k, 0.0);
        for j in 0..plane.h {
            for i in 0..plane.w {
                let (px, py) = plane.centre(i, j);
                let (sx, sy) = apply(&inv, px, py);
                if let Some(v) = plane.v.get_mut(j * plane.w + i) {
                    *v = alpha.at(sx, sy, 0.0);
                }
            }
        }
        let plane = plane.blurred(blur_px / 2.0);
        grow((x0, y0, x1, y1));
        Some((plane, sh.color, 1.0 - sh.transparency.clamp(0.0, 100.0) / 100.0))
    });
    let glow_layer = glow.map(|g| {
        let size = sc.px(g.size).max(1.0);
        let pad = size + 2.0;
        let mut plane = alpha_plane(&cv.img, k, pad, 0.0, false);
        let d = distance_out(&plane);
        let size_k = size * plane.k;
        for (v, d) in plane.v.iter_mut().zip(d) {
            let t = (1.0 - d / size_k).clamp(0.0, 1.0);
            *v = t.powf(1.6);
        }
        grow((-pad, -pad, wf + pad, hf + pad));
        (plane, g.color, 1.0 - g.transparency.clamp(0.0, 100.0) / 100.0)
    });
    let refl_layer = match refl {
        Some(r) => {
            let rh = ((cy1 - cy0) as f32 * r.size.clamp(1.0, 100.0) / 100.0).ceil().max(1.0) as u32;
            let top = cy1 as f32 + sc.px(r.distance).round();
            let blur_px = sc.px(r.blur);
            let start = 1.0 - r.transparency.clamp(0.0, 100.0) / 100.0;
            fits(w, rh)?;
            let mut img = RgbaImage::from_fn(w, rh, |x, yy| {
                let src_y = cy1.saturating_sub(1).saturating_sub(yy);
                let mut p = cv.img.get_pixel_checked(x, src_y).map_or([0; 4], |p| p.0);
                if src_y < cy0 {
                    p = [0; 4];
                }
                let fade = start * (1.0 - (yy as f32 + 0.5) / rh as f32);
                p[3] = (f32::from(p[3]) * fade.clamp(0.0, 1.0)).round() as u8;
                image::Rgba(p)
            });
            if blur_px >= 1.0 {
                img = blur_rgba(&img, (blur_px / 2.0).round() as usize);
            }
            grow((0.0, top, wf, top + rh as f32));
            Some((img, top))
        }
        None => None,
    };
    // The new bitmap holds the picture and every layer.
    let pad = [(-bounds.0).ceil().max(0.0), (-bounds.1).ceil().max(0.0), (bounds.2 - wf).ceil().max(0.0), (bounds.3 - hf).ceil().max(0.0)];
    let pad = pad.map(|v| v.min(u32::MAX as f32) as u32);
    cv.pad(pad)?;
    let (ox, oy) = (pad[0] as f32, pad[1] as f32);
    let (nw, nh) = cv.img.dimensions();
    let layered = |x: u32, y: u32, body: [u8; 4]| {
        // Canvas coordinates before the padding.
        let (bx, by) = (x as f32 + 0.5 - ox, y as f32 + 0.5 - oy);
        let mut out = [0u8; 4];
        for (plane, c, op) in shadow_layer.iter().chain(glow_layer.iter()) {
            let a = plane.at(bx, by, 0.0).clamp(0.0, 1.0) * op;
            if a > 0.0 {
                out = over(out, [c.0, c.1, c.2, (a * 255.0).round() as u8]);
            }
        }
        out = over(out, body);
        if let Some((img, top)) = &refl_layer {
            let yy = by - 0.5 - top;
            if yy >= 0.0
                && let Some(p) = img.get_pixel_checked((bx - 0.5) as u32, yy as u32).filter(|_| bx >= 0.5)
            {
                out = over(out, p.0);
            }
        }
        out
    };
    let mut drawn = RgbaImage::new(nw, nh);
    for (x, y, p) in drawn.enumerate_pixels_mut() {
        let body = cv.img.get_pixel_checked(x, y).map_or([0; 4], |p| p.0);
        p.0 = layered(x, y, body);
    }
    cv.img = drawn;
    Ok(())
}

/// How far each cell of `plane` is from the nearest cell at least half full, in cells (two-pass
/// chamfer distance, 0 on those cells).
fn distance_out(plane: &Plane) -> Vec<f32> {
    let (w, h) = (plane.w, plane.h);
    let big = (w + h) as f32 * 2.0 + 1.0;
    let mut d: Vec<f32> = plane.v.iter().map(|v| if *v >= 0.5 { 0.0 } else { big }).collect();
    let at = |d: &[f32], x: isize, y: isize| -> f32 {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h { big } else { d.get(y as usize * w + x as usize).copied().unwrap_or(big) }
    };
    let diag = std::f32::consts::SQRT_2;
    for y in 0..h as isize {
        for x in 0..w as isize {
            let i = y as usize * w + x as usize;
            let cur = d.get(i).copied().unwrap_or(big);
            let best = cur.min(at(&d, x - 1, y) + 1.0).min(at(&d, x, y - 1) + 1.0).min(at(&d, x - 1, y - 1) + diag).min(at(&d, x + 1, y - 1) + diag);
            if let Some(v) = d.get_mut(i) {
                *v = best;
            }
        }
    }
    for y in (0..h as isize).rev() {
        for x in (0..w as isize).rev() {
            let i = y as usize * w + x as usize;
            let cur = d.get(i).copied().unwrap_or(big);
            let best = cur.min(at(&d, x + 1, y) + 1.0).min(at(&d, x, y + 1) + 1.0).min(at(&d, x + 1, y + 1) + diag).min(at(&d, x - 1, y + 1) + diag);
            if let Some(v) = d.get_mut(i) {
                *v = best;
            }
        }
    }
    d
}

/// `img` blurred with a box of `r` pixels, twice, premultiplied so clear pixels don't darken it.
fn blur_rgba(img: &RgbaImage, r: usize) -> RgbaImage {
    let (w, h) = img.dimensions();
    if r == 0 || w == 0 || h == 0 {
        return img.clone();
    }
    let a: Vec<f32> = img.pixels().map(|p| f32::from(p.0[3]) / 255.0).collect();
    let planes: Vec<Vec<f32>> = (0..3)
        .map(|ch| {
            let v: Vec<f32> = img.pixels().zip(&a).map(|(p, a)| f32::from(p.0.get(ch).copied().unwrap_or(0)) * a).collect();
            blur(&v, w, h, r)
        })
        .collect();
    let ab = blur(&a, w, h, r);
    let mut out = img.clone();
    for (i, p) in out.pixels_mut().enumerate() {
        let alpha = ab.get(i).copied().unwrap_or(0.0);
        if alpha <= 1e-4 {
            p.0 = [0; 4];
            continue;
        }
        let ch = |c: usize| (planes.get(c).and_then(|pl| pl.get(i)).copied().unwrap_or(0.0) / alpha).round().clamp(0.0, 255.0) as u8;
        p.0 = [ch(0), ch(1), ch(2), (alpha * 255.0).round().clamp(0.0, 255.0) as u8];
    }
    out
}

#[cfg(test)]
mod tests;
