//! Picture Format's adjustments as one stack (#2052, #3344): Remove Background, Set Transparent
//! Color, Corrections, Color, Artistic Effects, Transparency, Picture Styles and Picture Border
//! each keep their own setting, and the picture is drawn again from the bitmap under all of them
//! whenever one changes. So each replaces only itself, as in Word: Color after Remove Background
//! keeps the background removed, Sharpen keeps the brightness, and Transparency 0% after 50% is
//! the picture as it was.
//!
//! The stack is kept in [`Session::picture_looks`] under the media key of the bitmap it drew, so
//! undo and redo find it again; a bitmap that no look drew (just inserted, opened from a file, or
//! changed by Icon Color, Compress or the one-colour fill) starts a new stack on itself.

use std::cell::Cell;
use std::sync::Arc;

use image::{DynamicImage, GrayImage, RgbaImage};
use serde_json::Value;
use wordcraft_doc::picfx::PicFx;

use super::cutout::{self, Mark};
use super::picture::{adjust_media, artistic_effect, frame, picture_style, recolor};
use super::select::{selected, with_obj};
use super::{CmdError, CmdResult, InlineObject, Rgb, Session, json, p};

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
    pub border: Option<(Rgb, u32)>,
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
            && self.style.is_none()
            && self.border.is_none()
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

/// The picture styles `picture.style` takes.
pub const STYLES: [&str; 5] = ["simpleFrame", "thickFrame", "rounded", "softEdge", "shadow"];

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
            // A style replaces the picture's style and border, as Word's gallery does.
            let st = p::str(v, "style").unwrap_or("simpleFrame");
            if st == "none" {
                look.style = None;
            } else if STYLES.contains(&st) {
                look.style = Some(st.to_string());
            } else {
                return Err(CmdError::Params(format!("unknown style `{st}`; use none or one of {}", STYLES.join(", "))));
            }
            look.border = None;
        }
        "picture.border" => {
            // Replaces the border, keeping the style. `color: null` is No Outline; a part left
            // out keeps the border's own (black, 4 px for a new one).
            if v.get("color").is_some_and(Value::is_null) {
                look.border = None;
            } else {
                let (c0, w0) = look.border.unwrap_or((Rgb::BLACK, 4));
                let c = p::str(v, "color").and_then(Rgb::parse).unwrap_or(c0);
                let w = p::u64(v, "width").map_or(w0, |w| w.clamp(1, 200) as u32);
                look.border = Some((c, w));
            }
        }
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
    let mut look = current(s).ok_or_else(|| CmdError::Disabled("select a picture first".into()))?;
    let v = if id == "picture.transparentColor" && v.get("color").is_none() { picked_color(s, &look, v)? } else { v.clone() };
    change(id, &v, &mut look)?;
    let fx = file_fx(id, &v);
    if look.is_plain() {
        // Nothing left on it: the bitmap as it was.
        let base = look.base.clone();
        return with_obj(s, |o| {
            if let InlineObject::Image { media, fx: pic_fx, .. } = o {
                *media = base.clone();
                fx(pic_fx);
            }
        });
    }
    let found = Cell::new(None);
    let l = look.clone();
    let mut out = adjust_media(s, &look.base, fx, |img| {
        let (img, mask) = render(img, &l)?;
        found.set(mask);
        Ok(img)
    })?;
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
    if let Some((_, InlineObject::Image { media, .. })) = selected(s)
        && let Some(drawn) = s.doc.media.get(&media).cloned()
    {
        s.picture_looks.insert(media, PictureLook { drawn, ..look });
    }
    Ok(out)
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

/// `img` (the look's base) drawn with everything on `look`, in Word's order. Returns the mask
/// Remove Background found, when it had to find one.
pub fn render(img: DynamicImage, look: &PictureLook) -> Result<(DynamicImage, Option<Arc<GrayImage>>), CmdError> {
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
    if let Some(st) = &look.style {
        img = picture_style(img, st)?;
    }
    if let Some((c, w)) = look.border {
        img = frame(img, w, [c.0, c.1, c.2, 255])?;
    }
    Ok((img, found))
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
    let mut look = current(s)?;
    change(id, v, &mut look).ok()?;
    preview_look(thumb, &look)
}

/// [`preview`] of a look already changed.
pub fn preview_look(thumb: &RgbaImage, look: &PictureLook) -> Option<RgbaImage> {
    let mut look = look.clone();
    if let Some(c) = look.cutout.as_mut() {
        // Shrink the mask the look has; with none yet, the preview leaves the background.
        match c.mask.as_ref() {
            Some(m) => c.mask = Some(Arc::new(image::imageops::resize(&**m, thumb.width(), thumb.height(), image::imageops::FilterType::Triangle))),
            None => look.cutout = None,
        }
    }
    render(DynamicImage::ImageRgba8(thumb.clone()), &look).ok().map(|(img, _)| img.into_rgba8())
}

#[cfg(test)]
mod tests;
