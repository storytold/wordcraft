//! Picture Format's adjustments as one stack (#2052, #3344): Remove Background, Set Transparent
//! Color, Corrections, Color, Artistic Effects, Transparency, Picture Styles, Picture Border and
//! Picture Effects each keep their own setting, and the picture is drawn again from the bitmap
//! under all of them whenever one changes. So each replaces only itself, as in Word: Color after
//! Remove Background keeps the background removed, Sharpen keeps the brightness, and Transparency
//! 0% after 50% is the picture as it was.
//!
//! The stack is kept in [`Session::picture_looks`] under the media key of the bitmap it drew, so
//! undo and redo find it again; a bitmap that no look drew (just inserted, opened from a file, or
//! changed by Icon Color, Compress or the one-colour fill) starts a new stack on itself.
//!
//! A style, a border or an effect is drawn round the picture's edge ([`PictureLook::draws_round`]):
//! then the bitmap holds only what the crop shows, with room round it for what reaches outside (a
//! frame, a shadow, a reflection), and the picture's frame on the page grows by that room so the
//! picture keeps its own size and place. [`Shown`] is the picture as the user sees it under such a
//! look: its crop, its own size and the room; Crop, Size and Reset Picture work on that.

pub mod fx;
pub mod styles;

use std::cell::Cell;
use std::sync::Arc;

use image::{DynamicImage, GrayImage, RgbaImage};
use serde_json::Value;
use wordcraft_doc::para::Float;
use wordcraft_doc::picfx::PicFx;

use self::fx::{Canvas, Dash, PictureEffects, Scale, Shape};
use super::arrange::MAX_OFFSET;
use super::cutout::{self, Mark};
use super::picture::{artistic_effect, draw_media, picture_style, recolor, visible};
use super::select::{selected, with_obj};
use super::{CmdError, CmdResult, InlineObject, Rgb, Session, Wrap, json, p};

/// Corrections: −100 to 100 each, 0 is none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Corrections {
    pub brightness: f32,
    pub contrast: f32,
    /// Positive sharpens, negative softens.
    pub sharpen: f32,
}

/// Color: saturation (percent, 100 is as it is), tone (a colour temperature in kelvin, 6500 is as
/// it is) and a recolour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PictureColor {
    pub saturation: f32,
    pub temperature: f32,
    pub recolor: Option<Recolor>,
}

impl Default for PictureColor {
    fn default() -> Self {
        PictureColor { saturation: 100.0, temperature: NEUTRAL_K, recolor: None }
    }
}

/// The colour temperature that leaves a picture's colours as they are.
pub const NEUTRAL_K: f32 = 6500.0;

/// Color › Recolor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Recolor {
    Grayscale,
    Sepia,
    Washout,
    /// Black and white, white above this share of full brightness (percent).
    BlackAndWhite(f32),
    /// The blue duotone scripts have used since before the theme-colour variations.
    Tint,
    /// The picture's shades in one colour: dark, from near black to the colour.
    Dark(Rgb),
    /// Light, from the colour to white.
    Light(Rgb),
}

impl Recolor {
    /// The `recolor` names [`change`] takes.
    pub const NAMES: [&'static str; 8] = ["none", "grayscale", "sepia", "washout", "blackAndWhite", "tint", "dark", "light"];
}

/// Remove Background: the marks it ran with and, once found, the mask of what stays on the
/// look's bitmap (shared between the looks drawn from it).
#[derive(Clone, Debug, Default)]
pub struct Cutout {
    pub marks: Vec<Mark>,
    pub mask: Option<Arc<GrayImage>>,
}

/// How wide a Picture Border is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BorderWidth {
    /// Pixels of the bitmap (`width`, as scripts have given it).
    Px(u32),
    /// Points (`weight`, the ribbon's Weight).
    Pt(f32),
}

/// Picture Border: its colour, width and dashes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub color: Rgb,
    pub width: BorderWidth,
    pub dash: Dash,
}

/// The weight a new border takes: three quarters of a point, as Word's.
pub const DEFAULT_WEIGHT: f32 = 0.75;

/// Everything Picture Format drew on a picture's bitmap, and the bitmap it started from. Kept in
/// [`Session::picture_looks`] under the media key of the drawn bitmap.
#[derive(Clone, Debug, Default)]
pub struct PictureLook {
    /// The media key of the bitmap under all of them.
    pub base: String,
    pub cutout: Option<Cutout>,
    /// Set Transparent Color.
    pub clear_color: Option<Rgb>,
    pub corrections: Corrections,
    pub color: PictureColor,
    pub effect: Option<String>,
    /// Transparency, percent.
    pub transparency: f32,
    pub style: Option<String>,
    pub border: Option<Border>,
    /// Picture Effects.
    pub effects: PictureEffects,
    /// Drawn round the picture: the crop it showed, the size of what that left (pixels of the
    /// drawn bitmap), and the room round it on each side (left, top, right, bottom; pixels).
    pub crop: [f32; 4],
    pub vis: (u32, u32),
    pub room: [u32; 4],
    /// The drawn bitmap: a media key that names other bytes later (undo then a new picture
    /// reusing the key) is not this look's.
    pub drawn: Arc<Vec<u8>>,
}

impl PictureLook {
    /// Nothing drawn: the base bitmap as it is.
    pub fn is_plain(&self) -> bool {
        self.cutout.is_none()
            && self.clear_color.is_none()
            && self.corrections == Corrections::default()
            && self.color == PictureColor::default()
            && self.effect.is_none()
            && self.transparency <= 0.0
            && !self.draws_round()
    }

    /// Whether it draws round the picture's edge: a style, a border or a Picture Effect.
    pub fn draws_round(&self) -> bool {
        self.style.is_some() || self.border.is_some() || !self.effects.is_empty()
    }
}

/// The artistic effects `picture.effects` takes, besides `none`.
pub const EFFECTS: [&str; 26] = [
    "marker",
    "pencilGrayscale",
    "pencilSketch",
    "lineDrawing",
    "chalkSketch",
    "paintStrokes",
    "paintBrush",
    "glowDiffused",
    "blur",
    "lightScreen",
    "watercolorSponge",
    "filmGrain",
    "mosaicBubbles",
    "glass",
    "cement",
    "texturizer",
    "crisscrossEtching",
    "pastelsSmooth",
    "plasticWrap",
    "cutout",
    "photocopy",
    "glowEdges",
    // Kept for scripts written before the gallery.
    "sharpen",
    "invert",
    "posterize",
    "pixelate",
];

/// How far (per channel) a colour may be from the one Set Transparent Color picked and still clear.
const CLEAR_COLOR_TOLERANCE: u8 = 8;

/// The selected picture's look: the one that drew its bitmap, or a new one on its bitmap.
pub fn current(s: &Session) -> Option<PictureLook> {
    let (_, InlineObject::Image { media, .. }) = selected(s)? else { return None };
    let cur = s.doc.media.get(&media).cloned();
    let drawn =
        s.picture_looks.get(&media).filter(|l| cur.as_ref().is_some_and(|c| Arc::ptr_eq(c, &l.drawn)) && s.doc.media.contains_key(&l.base)).cloned();
    Some(drawn.unwrap_or_else(|| PictureLook { base: media, ..Default::default() }))
}

/// A picture as its look shows it: the crop it shows, its own size (points), and the room its
/// look takes round it in its frame (points: the bitmap's left, top, right and bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub crop: [f32; 4],
    pub w: f32,
    pub h: f32,
    pub room: [f32; 4],
}

/// Picture `o` as `look` (the look that drew its bitmap, if any) shows it. None for anything but
/// a picture.
pub fn shown_of(o: &InlineObject, look: Option<&PictureLook>) -> Option<Shown> {
    let InlineObject::Image { w, h, crop, .. } = o else { return None };
    let plain = Shown { crop: *crop, w: *w, h: *h, room: [0.0; 4] };
    let Some(l) = look.filter(|l| l.draws_round() && l.vis.0 > 0 && l.vis.1 > 0) else { return Some(plain) };
    let total = |a: u32, b: u32, c: u32| a as f32 + b as f32 + c as f32;
    let (kx, ky) = (*w / total(l.vis.0, l.room[0], l.room[2]), *h / total(l.vis.1, l.room[1], l.room[3]));
    if !kx.is_finite() || !ky.is_finite() || kx <= 0.0 || ky <= 0.0 {
        return Some(plain);
    }
    let room = [l.room[0] as f32 * kx, l.room[1] as f32 * ky, l.room[2] as f32 * kx, l.room[3] as f32 * ky];
    Some(Shown { crop: l.crop, w: l.vis.0 as f32 * kx, h: l.vis.1 as f32 * ky, room })
}

/// The selected picture as its look shows it.
pub fn shown(s: &Session) -> Option<Shown> {
    let (_, o) = selected(s)?;
    shown_of(&o, current(s).as_ref())
}

/// The selected object; a picture as its look shows it, with its crop and its own size (the
/// Size dialog and the ribbon's fields show these).
pub fn selected_shown(s: &Session) -> Option<(wordcraft_doc::Pos, InlineObject)> {
    let (pos, mut o) = selected(s)?;
    if let Some(sh) = shown(s)
        && let InlineObject::Image { w, h, crop, .. } = &mut o
    {
        (*w, *h, *crop) = (sh.w, sh.h, sh.crop);
    }
    Some((pos, o))
}

/// `room` (the bitmap's left, top, right, bottom) where it is on the page: a picture flipped
/// across shows its bitmap's left room on its right.
pub fn page_room(room: [f32; 4], flip_h: bool, flip_v: bool) -> [f32; 4] {
    let [l, t, r, b] = room;
    let (l, r) = if flip_h { (r, l) } else { (l, r) };
    let (t, b) = if flip_v { (b, t) } else { (t, b) };
    [l, t, r, b]
}

/// The selected picture's look when it draws round the picture, with the picture as it shows
/// and its placement.
pub(super) fn drawn_round(s: &Session) -> Option<(PictureLook, Shown, Float)> {
    let (_, o) = selected(s)?;
    let look = current(s).filter(|l| l.draws_round() && l.vis.0 > 0)?;
    let shown = shown_of(&o, Some(&look))?;
    let float = o.frame().map(|(_, _, f)| f.clone())?;
    Some((look, shown, float))
}

/// The selected picture's frame back to the picture alone, giving back the room its look took
/// (Reset Picture and Change Picture start from it). The bitmap is left for them to replace.
pub(super) fn unbake(s: &mut Session) -> Result<(), CmdError> {
    let Some((_, shown, _)) = drawn_round(s) else { return Ok(()) };
    let Some((_, InlineObject::Image { media, .. })) = selected(s) else { return Ok(()) };
    set(s, &media, shown.crop, (shown.w, shown.h), (shown.room, [0.0; 4]), (0.0, 0.0), |_| {}).map(|_| ())
}

/// What command `id` with params `v` changes on `look`. The command and the ribbon's gallery
/// previews both go through here.
pub fn change(id: &str, v: &Value, look: &mut PictureLook) -> Result<(), CmdError> {
    let num = |k: &str, lo: f32, hi: f32| p::f32(v, k).map(|x| x.clamp(lo, hi));
    match id {
        "picture.corrections" => {
            // A value left out keeps its setting: Sharpen keeps the brightness, as in Word.
            let c = &mut look.corrections;
            c.brightness = num("brightness", -100.0, 100.0).unwrap_or(c.brightness);
            c.contrast = num("contrast", -100.0, 100.0).unwrap_or(c.contrast);
            c.sharpen = num("sharpen", -100.0, 100.0).unwrap_or(c.sharpen);
        }
        "picture.color" => color_change(v, &mut look.color)?,
        "picture.effects" => {
            // One effect at a time, as in Word: a second one replaces the first.
            let e = p::str(v, "effect").unwrap_or("blur");
            if e != "none" && !EFFECTS.contains(&e) {
                return Err(CmdError::Params(format!("unknown effect `{e}`; use none or one of {}", EFFECTS.join(", "))));
            }
            look.effect = (e != "none").then(|| e.to_string());
        }
        "picture.transparency" => look.transparency = num("percent", 0.0, 100.0).unwrap_or(50.0),
        "picture.style" => {
            // A style replaces the picture's style, border and effects, as Word's gallery does.
            let st = p::str(v, "style").unwrap_or("simpleFrame");
            if st == "none" {
                look.style = None;
                look.effects = PictureEffects::default();
            } else if let Some(spec) = styles::spec(st) {
                look.style = Some(st.to_string());
                look.effects = (spec.effects)();
            } else if styles::LEGACY.iter().any(|(id, _)| *id == st) {
                look.style = Some(st.to_string());
                look.effects = PictureEffects::default();
            } else {
                let ids: Vec<&str> = styles::gallery().map(|(id, _)| id).collect();
                return Err(CmdError::Params(format!("unknown style `{st}`; use none or one of {}", ids.join(", "))));
            }
            look.border = None;
        }
        "picture.border" => {
            // Replaces the border, keeping the style. `color: null` is No Outline; a part left
            // out keeps the border's own (black, three quarters of a point, solid for a new one).
            if v.get("color").is_some_and(Value::is_null) {
                look.border = None;
            } else {
                let b = look.border.unwrap_or(Border { color: Rgb::BLACK, width: BorderWidth::Pt(DEFAULT_WEIGHT), dash: Dash::Solid });
                let width = match (p::f32(v, "weight").filter(|w| w.is_finite()), p::u64(v, "width")) {
                    (Some(pt), _) => BorderWidth::Pt(pt.clamp(0.25, fx::MAX_LEN)),
                    (None, Some(px)) => BorderWidth::Px(px.clamp(1, 200) as u32),
                    (None, None) => b.width,
                };
                let dash = match p::str(v, "dash") {
                    Some(d) => {
                        Dash::parse(d).ok_or_else(|| CmdError::Params(format!("unknown dash `{d}`; use one of {}", Dash::NAMES.join(", "))))?
                    }
                    None => b.dash,
                };
                look.border = Some(Border { color: p::str(v, "color").and_then(Rgb::parse).unwrap_or(b.color), width, dash });
            }
        }
        "picture.pictureEffects" => fx::change(v, &mut look.effects)?,
        "picture.removeBackground" => {
            look.cutout = if p::bool(v, "discard") == Some(true) { None } else { Some(Cutout { marks: cutout::marks_from(v)?, mask: None }) };
        }
        "picture.transparentColor" => {
            look.clear_color = match v.get("color") {
                Some(Value::Null) => None,
                Some(c) => Some(c.as_str().and_then(Rgb::parse).ok_or_else(|| CmdError::Params("`color` is RRGGBB, or null for none".into()))?),
                None => return Err(CmdError::Params("`color` (RRGGBB or null), or `x` and `y` on the picture, is required".into())),
            };
        }
        _ => return Err(CmdError::Params(format!("`{id}` doesn't change a picture's look"))),
    }
    Ok(())
}

fn color_change(v: &Value, c: &mut PictureColor) -> Result<(), CmdError> {
    let sat = p::f32(v, "saturation").map(|s| s.clamp(0.0, 400.0));
    let temp = p::f32(v, "temperature").map(|t| t.clamp(1500.0, 12000.0));
    let named = p::str(v, "recolor");
    let mode = p::str(v, "mode");
    // Nothing at all asks for Grayscale, as `picture.color` always has.
    let mode = mode.or((sat.is_none() && temp.is_none() && named.is_none()).then_some("grayscale"));
    if let Some(s) = sat {
        c.saturation = s;
    }
    if let Some(t) = temp {
        c.temperature = t;
    }
    // `mode` is how scripts asked before Color had its three parts: `saturation`, or a recolour.
    let recolor = match (named, mode) {
        (Some(n), _) => Some(n),
        (None, Some("saturation")) => {
            c.saturation = sat.unwrap_or(100.0);
            None
        }
        (None, Some("tone")) => None,
        (None, m) => m,
    };
    if let Some(name) = recolor {
        let color = || p::str(v, "color").and_then(Rgb::parse).ok_or_else(|| CmdError::Params(format!("`{name}` needs `color`: RRGGBB")));
        c.recolor = match name {
            "none" => None,
            "grayscale" => Some(Recolor::Grayscale),
            "sepia" => Some(Recolor::Sepia),
            "washout" => Some(Recolor::Washout),
            "blackAndWhite" => Some(Recolor::BlackAndWhite(p::f32(v, "threshold").map_or(50.0, |t| t.clamp(0.0, 100.0)))),
            "tint" => Some(Recolor::Tint),
            "dark" => Some(Recolor::Dark(color()?)),
            "light" => Some(Recolor::Light(color()?)),
            other => return Err(CmdError::Params(format!("unknown recolor `{other}`; use one of {}", Recolor::NAMES.join(", ")))),
        };
    }
    Ok(())
}

/// What command `id` with `v` does to the corrections the picture brought from its file (`fx`):
/// an edit that draws its own replaces them.
fn file_fx(id: &str, v: &Value) -> fn(&mut PicFx) {
    let washout = p::str(v, "recolor").or(p::str(v, "mode")) == Some("washout");
    match id {
        "picture.corrections" => clear_lum,
        // Word keeps Washout as brightness and contrast (`a:lum`).
        "picture.color" if washout => clear_lum,
        "picture.transparency" => |fx: &mut PicFx| fx.transparency = 0,
        _ => |_: &mut PicFx| {},
    }
}

fn clear_lum(fx: &mut PicFx) {
    fx.bright = 0;
    fx.contrast = 0;
}

/// Runs a look command on the selected picture: its look changes, and the picture is drawn again
/// from the look's base bitmap.
pub(super) fn apply(s: &mut Session, id: &str, v: &Value) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("select a picture first".into()))?;
    let mut look = current(s).ok_or_else(|| CmdError::Disabled("select a picture first".into()))?;
    let before = shown_of(&o, Some(&look)).ok_or_else(|| CmdError::Disabled("select a picture first".into()))?;
    let v = if id == "picture.transparentColor" && v.get("color").is_none() { picked_color(s, &look, v)? } else { v.clone() };
    change(id, &v, &mut look)?;
    place(s, look, Placement { crop: before.crop, w: before.w, h: before.h }, before.room, (0.0, 0.0), file_fx(id, &v))
}

/// Where a picture shows: its crop and its own size (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placement {
    pub crop: [f32; 4],
    pub w: f32,
    pub h: f32,
}

/// The selected picture drawn with `look` from its base bitmap, showing as `at`. `old_room` is the
/// room the previous look took in the frame (points); `shift` is how far the picture's centre
/// moves on the page (points, before its turn). `fx` changes the corrections it brought from its
/// file.
pub(super) fn place(s: &mut Session, look: PictureLook, at: Placement, old_room: [f32; 4], shift: (f32, f32), fx: impl Fn(&mut PicFx)) -> CmdResult {
    if look.is_plain() {
        // Nothing left on it: the bitmap as it was.
        let base = look.base.clone();
        return set(s, &base, at.crop, (at.w, at.h), (old_room, [0.0; 4]), shift, fx);
    }
    let frame = Frame { crop: at.crop, size: (at.w, at.h) };
    let found = Cell::new(None);
    let drawn = Cell::new(((0, 0), [0u32; 4]));
    let key = draw_media(s, &look.base, |img| {
        let d = render(img, &look, frame)?;
        found.set(d.mask);
        drawn.set((d.vis, d.room));
        Ok(d.img)
    })?;
    let (vis, room) = drawn.get();
    let round = look.draws_round() && vis.0 > 0 && vis.1 > 0;
    let (crop, size, room_pt) = if round {
        // Points per pixel of the drawn bitmap: the visible part fills the picture's own size.
        let (kx, ky) = (at.w / vis.0 as f32, at.h / vis.1 as f32);
        let r = [room[0] as f32 * kx, room[1] as f32 * ky, room[2] as f32 * kx, room[3] as f32 * ky];
        ([0.0; 4], (at.w + r[0] + r[2], at.h + r[1] + r[3]), r)
    } else {
        (at.crop, (at.w, at.h), [0.0; 4])
    };
    let mut out = set(s, &key, crop, size, (old_room, room_pt), shift, fx)?;
    let mut look = look;
    if let (Some(c), Some(mask)) = (look.cutout.as_mut(), found.take()) {
        c.mask = Some(mask);
    }
    if let Some(mask) = look.cutout.as_ref().and_then(|c| c.mask.as_ref())
        && let Some(o) = out.as_object_mut()
    {
        let all = mask.pixels().len().max(1) as f64;
        let kept = mask.pixels().map(|p| f64::from(p.0[0])).sum::<f64>() / 255.0 / all;
        o.insert("kept".into(), json!((kept * 1000.0).round() / 1000.0));
    }
    if let Some(drawn) = s.doc.media.get(&key).cloned() {
        s.picture_looks.insert(key, PictureLook { drawn, crop: at.crop, vis, room, ..look });
    }
    Ok(out)
}

/// The selected picture showing `media` cropped `crop` in a frame `size` points, its frame moved
/// so the picture keeps its place as the room its look takes goes from `rooms.0` to `rooms.1`
/// and its centre moves by `shift`.
fn set(
    s: &mut Session,
    media: &str,
    crop: [f32; 4],
    size: (f32, f32),
    rooms: ([f32; 4], [f32; 4]),
    shift: (f32, f32),
    fx: impl Fn(&mut PicFx),
) -> CmdResult {
    let (nw, nh) = size;
    if !(nw.is_finite() && nh.is_finite() && nw > 0.0 && nh > 0.0) {
        return Err(CmdError::Failed("the picture has no size".into()));
    }
    with_obj(s, |o| {
        if let InlineObject::Image { media: m, crop: c, w, h, float, fx: pic_fx, .. } = o {
            keep_place(float, (*w, *h, rooms.0), (nw, nh, rooms.1), shift);
            *m = media.to_string();
            *c = crop;
            (*w, *h) = (nw, nh);
            fx(pic_fx);
        }
    })
}

/// Moves a floating frame (`w0` × `h0`, its picture's room `old`) that becomes `w1` × `h1` with
/// room `new`, so the picture's centre stays where it was on the page (turned with the frame),
/// moved by `shift`. A side positioned by alignment, or a picture in line with text, stays put.
fn keep_place(f: &mut Float, (w0, h0, old): (f32, f32, [f32; 4]), (w1, h1, new): (f32, f32, [f32; 4]), shift: (f32, f32)) {
    if f.wrap == Wrap::Inline {
        return;
    }
    let (o, n) = (page_room(old, f.flip_h, f.flip_v), page_room(new, f.flip_h, f.flip_v));
    // The picture's centre from its frame's, before and after, and the move asked for.
    let d = ((o[0] - o[2]) / 2.0 - (n[0] - n[2]) / 2.0 + shift.0, (o[1] - o[3]) / 2.0 - (n[1] - n[3]) / 2.0 + shift.1);
    let a = f.rot.to_radians();
    let (c, sn) = if a.is_finite() { (a.cos(), a.sin()) } else { (1.0, 0.0) };
    let (rx, ry) = (d.0 * c - d.1 * sn, d.0 * sn + d.1 * c);
    let (dx, dy) = (w0 / 2.0 + rx - w1 / 2.0, h0 / 2.0 + ry - h1 / 2.0);
    if f.h_align.is_none() && dx.is_finite() {
        f.x = (f.x + dx).clamp(-MAX_OFFSET, MAX_OFFSET);
    }
    if f.v_align.is_none() && dy.is_finite() {
        f.y = (f.y + dy).clamp(-MAX_OFFSET, MAX_OFFSET);
    }
}

/// `picture.transparentColor` at `x`, `y` (0 to 1 across and down the uncropped picture): the
/// colour of the look's base bitmap there.
fn picked_color(s: &Session, look: &PictureLook, v: &Value) -> Result<Value, CmdError> {
    let (Some(x), Some(y)) = (p::f32(v, "x"), p::f32(v, "y")) else {
        return Err(CmdError::Params("`color` (RRGGBB or null), or `x` and `y` on the picture, is required".into()));
    };
    let bytes = s.doc.media.get(&look.base).ok_or_else(|| CmdError::Failed("picture data missing".into()))?;
    let img = wordcraft_geom::picture::decode(bytes, wordcraft_geom::MAX_DECODED_BYTES)
        .map_err(|e| CmdError::Failed(format!("can't decode the picture: {e}")))?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err(CmdError::Failed("the picture has no pixels".into()));
    }
    let px = |f: f32, n: u32| ((f.clamp(0.0, 1.0) * n as f32) as u32).min(n - 1);
    let image::Rgba([r, g, b, _]) = image::GenericImageView::get_pixel(&img, px(x, w), px(y, h));
    Ok(json!({"color": Rgb(r, g, b).hex()}))
}

/// How a look draws round the picture: the crop the picture shows and its own size (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub crop: [f32; 4],
    pub size: (f32, f32),
}

/// What [`render`] drew: the bitmap, the mask Remove Background found (when it had to find one),
/// and for a look drawn round the picture the size of the picture in it and the room round it
/// (pixels; left, top, right, bottom).
pub struct Drawn {
    pub img: DynamicImage,
    pub mask: Option<Arc<GrayImage>>,
    pub vis: (u32, u32),
    pub room: [u32; 4],
}

/// `img` (the look's base) drawn with everything on `look`, in Word's order, showing as `frame`.
pub fn render(img: DynamicImage, look: &PictureLook, frame: Frame) -> Result<Drawn, CmdError> {
    let mut found = None;
    let mut img = img;
    if let Some(c) = &look.cutout {
        let mut rgba = img.into_rgba8();
        let mask = match &c.mask {
            Some(m) if m.dimensions() == rgba.dimensions() => m.clone(),
            _ => {
                let m = Arc::new(cutout::foreground(&rgba, &c.marks));
                found = Some(m.clone());
                m
            }
        };
        apply_mask(&mut rgba, &mask);
        img = DynamicImage::ImageRgba8(rgba);
    }
    // Drawn round the picture: only what the crop shows, its edges where the crop puts them.
    let round = look.draws_round();
    if round {
        img = visible(img, frame.crop);
    }
    if let Some(c) = look.clear_color {
        let mut rgba = img.into_rgba8();
        let near = |a: u8, b: u8| a.abs_diff(b) <= CLEAR_COLOR_TOLERANCE;
        for px in rgba.pixels_mut() {
            if near(px.0[0], c.0) && near(px.0[1], c.1) && near(px.0[2], c.2) {
                px.0[3] = 0;
            }
        }
        img = DynamicImage::ImageRgba8(rgba);
    }
    let c = look.corrections;
    if c.brightness != 0.0 {
        img = DynamicImage::ImageRgba8(image::imageops::colorops::brighten(&img, (c.brightness * 2.55) as i32));
    }
    if c.contrast != 0.0 {
        img = DynamicImage::ImageRgba8(image::imageops::colorops::contrast(&img, c.contrast));
    }
    if c.sharpen > 0.0 {
        img = img.unsharpen(1.0 + c.sharpen / 50.0, 2);
    } else if c.sharpen < 0.0 && (-c.sharpen / 25.0).is_normal() {
        // `blur` asserts on a subnormal width.
        img = img.blur(-c.sharpen / 25.0);
    }
    let col = look.color;
    if col.saturation != 100.0 {
        img = recolor(img, "saturation", col.saturation);
    }
    if col.temperature != NEUTRAL_K {
        img = tone(img, col.temperature);
    }
    if let Some(r) = col.recolor {
        img = match r {
            Recolor::Grayscale => recolor(img, "grayscale", 100.0),
            Recolor::Sepia => recolor(img, "sepia", 100.0),
            Recolor::Washout => recolor(img, "washout", 100.0),
            Recolor::Tint => recolor(img, "tint", 100.0),
            Recolor::BlackAndWhite(t) => duotone(img, |l| if l * 100.0 > t { [255.0; 3] } else { [0.0; 3] }),
            Recolor::Dark(c) => {
                let c = [f32::from(c.0), f32::from(c.1), f32::from(c.2)];
                duotone(img, move |l| c.map(|v| v * (0.15 + 0.85 * l)))
            }
            Recolor::Light(c) => {
                let c = [f32::from(c.0), f32::from(c.1), f32::from(c.2)];
                duotone(img, move |l| c.map(|v| v + (255.0 - v) * l))
            }
        };
    }
    if let Some(e) = &look.effect {
        img = artistic_effect(img, e);
    }
    if look.transparency > 0.0 {
        let t = look.transparency;
        let mut rgba = img.into_rgba8();
        for px in rgba.pixels_mut() {
            px.0[3] = (px.0[3] as f32 * (1.0 - t / 100.0)) as u8;
        }
        img = DynamicImage::ImageRgba8(rgba);
    }
    let vis = (img.width(), img.height());
    if !round {
        return Ok(Drawn { img, mask: found, vis, room: [0; 4] });
    }
    let sc = Scale::new(vis, frame.size);
    let mut cv = match look.style.as_deref() {
        Some(st) if styles::LEGACY.iter().any(|(id, _)| *id == st) => legacy(img, st)?,
        Some(st) => {
            let mut cv = Canvas::new(img.into_rgba8());
            if let Some(spec) = styles::spec(st) {
                cv.mask_shape(spec.shape);
                for r in spec.rings {
                    cv.ring(r.color, sc.px(r.width).max(1.0), Dash::Solid, r.metal)?;
                }
            }
            cv
        }
        None => Canvas::new(img.into_rgba8()),
    };
    if let Some(b) = look.border {
        let w = match b.width {
            BorderWidth::Px(px) => px as f32,
            BorderWidth::Pt(pt) => sc.px(pt).max(1.0),
        };
        cv.ring(b.color, w, b.dash, false)?;
    }
    fx::draw(&mut cv, &look.effects, sc)?;
    let room = cv.room();
    Ok(Drawn { img: DynamicImage::ImageRgba8(cv.img), mask: found, vis, room })
}

/// One of the five styles drawn before the gallery grew, as it drew them: the picture in the new
/// bitmap and the outline its frame leaves.
fn legacy(img: DynamicImage, st: &str) -> Result<Canvas, CmdError> {
    let (w, h) = (img.width(), img.height());
    let m = (w.max(h) / 40).max(4);
    let drawn = picture_style(img, st)?.into_rgba8();
    let (at, shape) = match st {
        "thickFrame" => (m * 2 + m / 2, Shape::Rect),
        "rounded" => (0, Shape::Rounded(0.12)),
        "softEdge" | "shadow" => (0, Shape::Rect),
        _ => (m, Shape::Rect),
    };
    Ok(Canvas { img: drawn, at: (at, at), win: (w, h), shape, grow: at as f32 })
}

/// `rgba`'s alpha times `mask` (the same size).
fn apply_mask(rgba: &mut RgbaImage, mask: &GrayImage) {
    for (p, m) in rgba.pixels_mut().zip(mask.pixels()) {
        p.0[3] = ((u16::from(p.0[3]) * u16::from(m.0[0]) + 127) / 255) as u8;
    }
}

/// Each pixel's colour from its brightness (0 to 1).
fn duotone(img: DynamicImage, f: impl Fn(f32) -> [f32; 3]) -> DynamicImage {
    let mut rgba = img.into_rgba8();
    for px in rgba.pixels_mut() {
        let [r, g, b, a] = px.0;
        let l = (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)) / 255.0;
        let c = f(l);
        px.0 = [c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8, a];
    }
    DynamicImage::ImageRgba8(rgba)
}

/// Color Tone: the picture as if its white balance were set for `kelvin`. Above 6500 K it warms
/// (more red, less blue), below it cools; by the change in mireds, our own simple model.
fn tone(img: DynamicImage, kelvin: f32) -> DynamicImage {
    let m = 1.0e6 / NEUTRAL_K - 1.0e6 / kelvin.max(1.0);
    let gain = [1.0 + 0.0035 * m, 1.0 + 0.0008 * m, 1.0 - 0.0035 * m];
    let mut rgba = img.into_rgba8();
    for px in rgba.pixels_mut() {
        for (c, g) in px.0.iter_mut().zip(gain) {
            *c = (f32::from(*c) * g).clamp(0.0, 255.0) as u8;
        }
    }
    DynamicImage::ImageRgba8(rgba)
}

/// The selected picture as `id` with `v` would draw it, from `thumb` (a small copy of its look's
/// base bitmap): for the ribbon galleries' previews. None when the command isn't a look command
/// or its params are refused. Remove Background's mask, when the look has one, is shrunk to fit.
pub fn preview(s: &Session, thumb: &RgbaImage, id: &str, v: &Value) -> Option<RgbaImage> {
    let (_, o) = selected(s)?;
    let mut look = current(s)?;
    let shown = shown_of(&o, Some(&look))?;
    change(id, v, &mut look).ok()?;
    // A small copy of a large picture would draw a 3 pt shadow a pixel wide: the tile draws the
    // effects at three quarters of a pixel a point or more, so they show.
    let [_, _, fw, _] = super::picture::crop_window(shown.crop);
    let vis_w = (f64::from(thumb.width()) * fw) as f32;
    let k = if shown.w > 0.0 && vis_w / shown.w < PREVIEW_PPP { vis_w / PREVIEW_PPP / shown.w } else { 1.0 };
    preview_look(thumb, &look, Frame { crop: shown.crop, size: (shown.w * k, shown.h * k) })
}

/// The fewest pixels a point that gallery previews draw effects at.
const PREVIEW_PPP: f32 = 0.75;

/// [`preview`] of a look already changed, showing as `frame`.
pub fn preview_look(thumb: &RgbaImage, look: &PictureLook, frame: Frame) -> Option<RgbaImage> {
    let mut look = look.clone();
    if let Some(c) = look.cutout.as_mut() {
        // Shrink the mask the look has; with none yet, the preview leaves the background.
        match c.mask.as_ref() {
            Some(m) => c.mask = Some(Arc::new(image::imageops::resize(&**m, thumb.width(), thumb.height(), image::imageops::FilterType::Triangle))),
            None => look.cutout = None,
        }
    }
    render(DynamicImage::ImageRgba8(thumb.clone()), &look, frame).ok().map(|d| d.img.into_rgba8())
}

#[cfg(test)]
mod tests;
