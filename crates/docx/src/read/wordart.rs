//! WordArt text effects in `w:rPr` ([MS-DOCX] §2.6.1: `w14:textFill`, `w14:textOutline`,
//! `w14:shadow`, `w14:glow`, `w14:reflection`). Lengths are EMUs, angles 60000ths of a degree,
//! percentages 1000ths of a percent.

use wordcraft_doc::effects::{Glow, Shadow};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::wordart::{GradStop, MAX_STOPS, Reflection, TextEffects, TextFill, TextOutline};

use crate::units::{int, measure};
use crate::xml::El;

/// Read one `w14:` run property into `fx`; false when `e` isn't one.
pub(crate) fn read(e: &El, theme: &[Rgb], fx: &mut TextEffects) -> bool {
    match e.name.as_str() {
        "w14:textFill" => fx.fill = fill(e, theme),
        "w14:textOutline" => {
            let (color, transparency) = match e.els().find(|c| matches!(c.name.as_str(), "w14:solidFill" | "w14:gradFill" | "w14:noFill")) {
                Some(f) if f.name == "w14:noFill" => (None, 0.0),
                Some(f) => match fill(f, theme) {
                    Some(TextFill::Solid { color, transparency }) => (Some(color), transparency),
                    Some(t) => (t.main_color(), 0.0),
                    None => (None, 0.0),
                },
                None => (Some(Rgb::BLACK), 0.0),
            };
            fx.outline = Some(TextOutline { color, width: emu(e, "w14:w"), transparency });
        }
        "w14:shadow" => {
            let (color, transparency) = color(e, theme).unwrap_or((Rgb::BLACK, 60.0));
            fx.shadow = Some(
                Shadow {
                    color,
                    transparency,
                    blur: emu(e, "w14:blurRad"),
                    distance: emu(e, "w14:dist"),
                    angle: angle(e, "w14:dir"),
                    rot_with_shape: false,
                }
                .sanitized(),
            );
        }
        "w14:glow" => {
            let (color, transparency) = color(e, theme).unwrap_or((Glow::default().color, 60.0));
            fx.glow = Some(Glow { color, size: emu(e, "w14:rad"), transparency }.sanitized()).filter(|g| g.size > 0.0);
        }
        "w14:reflection" => {
            // Start opacity and the end position of the fade (both 1000ths of a percent).
            let pct = |n: &str, d: f32| e.attr(n).and_then(int).map_or(d, |v| v.clamp(0, 100_000) as f32 / 1000.0);
            fx.reflection = Some(Reflection {
                transparency: 100.0 - pct("w14:stA", 50.0),
                size: pct("w14:endPos", 50.0),
                distance: emu(e, "w14:dist"),
                blur: emu(e, "w14:blurRad"),
            });
        }
        _ => return false,
    }
    true
}

fn emu(e: &El, n: &str) -> f32 {
    e.attr(n).and_then(|v| measure(v, 12_700.0)).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, 2000.0)
}

fn angle(e: &El, n: &str) -> f32 {
    e.attr(n).and_then(int).map_or(0.0, |v| v.rem_euclid(21_600_000) as f32 / 60_000.0)
}

/// A fill element (`w14:noFill`, `w14:solidFill`, `w14:gradFill`), or the one inside `e`.
fn fill(e: &El, theme: &[Rgb]) -> Option<TextFill> {
    let f = if matches!(e.name.as_str(), "w14:noFill" | "w14:solidFill" | "w14:gradFill") {
        e
    } else {
        e.els().find(|c| matches!(c.name.as_str(), "w14:noFill" | "w14:solidFill" | "w14:gradFill"))?
    };
    match f.name.as_str() {
        "w14:noFill" => Some(TextFill::None),
        "w14:solidFill" => color(f, theme).map(|(color, transparency)| TextFill::Solid { color, transparency }),
        _ => {
            let stops: Vec<GradStop> = f
                .child("w14:gsLst")
                .into_iter()
                .flat_map(|l| l.children("w14:gs"))
                .take(MAX_STOPS)
                .filter_map(|gs| {
                    let (color, transparency) = color(gs, theme)?;
                    let pos = gs.attr("w14:pos").and_then(int).map_or(0.0, |v| v.clamp(0, 100_000) as f32 / 1000.0);
                    Some(GradStop { pos, color, transparency })
                })
                .collect();
            let angle = f.child("w14:lin").map_or(90.0, |l| angle(l, "w14:ang"));
            TextFill::Gradient { stops, angle }.sanitized()
        }
    }
}

/// The colour element inside `e` (`w14:srgbClr`, `w14:schemeClr`) with its modifiers, and its
/// transparency (percent, from `w14:alpha`).
fn color(e: &El, theme: &[Rgb]) -> Option<(Rgb, f32)> {
    let c = e.els().find(|c| matches!(c.name.as_str(), "w14:srgbClr" | "w14:schemeClr"))?;
    let val = c.attr("w14:val")?;
    let mut rgb = if c.name == "w14:srgbClr" { Rgb::parse(val)? } else { super::drawing_color::scheme(val, theme)? };
    let mut transparency = 0.0;
    for m in c.els() {
        let v = m.attr("w14:val").and_then(int).map_or(0.0, |v| (v as f64 / 100_000.0).clamp(-10.0, 10.0));
        match m.name.as_str() {
            "w14:lumMod" => rgb = super::drawing_color::lum(rgb, |l| l * v),
            "w14:lumOff" => rgb = super::drawing_color::lum(rgb, |l| l + v),
            // Unlike DrawingML's `a:alpha`, `w14:alpha` is the transparency.
            "w14:alpha" => transparency = (v.clamp(0.0, 1.0) * 100.0) as f32,
            _ => {}
        }
    }
    Some((rgb, transparency))
}
