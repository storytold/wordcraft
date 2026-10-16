//! DrawingML colours shared by charts and SmartArt: a colour element with its modifiers, and the
//! paint of a fill or line (solid, the first stop of a gradient, a pattern's foreground).

use wordcraft_doc::THEME_COLORS;
use wordcraft_doc::props::Rgb;

use crate::units::int;
use crate::xml::El;

/// Theme colour `i` (0 dk1, 1 lt1, 2 dk2, 3 lt2, 4..9 accent1..6, 10 hlink, 11 folHlink), falling
/// back to the default theme.
pub(crate) fn slot(theme: &[Rgb], i: usize) -> Rgb {
    theme.get(i).or_else(|| THEME_COLORS.get(i)).copied().unwrap_or(Rgb::BLACK)
}

/// The colour of a fill or line element: a solid colour, the first stop of a gradient, or the
/// foreground of a pattern. `None` for no fill and picture fills.
pub(crate) fn paint(e: &El, theme: &[Rgb]) -> Option<Rgb> {
    match e.name.as_str() {
        "a:gradFill" => e.child("a:gsLst").and_then(|l| l.child("a:gs")).and_then(|g| color(g, theme)),
        "a:pattFill" => e.child("a:fgClr").and_then(|c| color(c, theme)),
        "a:noFill" | "a:blipFill" => None,
        _ => color(e, theme),
    }
}

/// The fill of a `…Pr` or `a:ln` element (its first solid, gradient or pattern fill), as [`paint`].
pub(crate) fn fill_colour(e: &El, theme: &[Rgb]) -> Option<Rgb> {
    let f = e.els().find(|c| matches!(c.name.as_str(), "a:solidFill" | "a:gradFill" | "a:pattFill"))?;
    paint(f, theme)
}

fn is_color(e: &El) -> bool {
    matches!(e.name.as_str(), "a:srgbClr" | "a:schemeClr" | "a:sysClr" | "a:prstClr" | "a:scrgbClr" | "a:hslClr")
}

/// A colour element (`a:srgbClr`, `a:schemeClr`, `a:sysClr`), or the one inside `e`, with its
/// `lumMod` / `lumOff` / `tint` / `shade` applied in document order.
pub(crate) fn color(e: &El, theme: &[Rgb]) -> Option<Rgb> {
    let c = if is_color(e) { e } else { e.els().find(|c| is_color(c))? };
    let base = match c.name.as_str() {
        "a:srgbClr" => Rgb::parse(c.attr("val")?)?,
        "a:schemeClr" => *theme.get(scheme_slot(c.attr("val")?)?)?,
        "a:sysClr" => match c.attr("lastClr").and_then(Rgb::parse) {
            Some(rgb) => rgb,
            None => sys_color(c.attr("val")?)?,
        },
        "a:prstClr" => preset_color(c.attr("val")?)?,
        "a:scrgbClr" => {
            // Linear 0–100 000 per channel, to sRGB.
            let ch = |n: &str| -> u8 {
                let l = c.attr(n).and_then(int).map_or(0.0, |v| (v as f64 / 100_000.0).clamp(0.0, 1.0));
                let s = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
                (s * 255.0).round().clamp(0.0, 255.0) as u8
            };
            Rgb(ch("r"), ch("g"), ch("b"))
        }
        "a:hslClr" => {
            let v = |n: &str| c.attr(n).and_then(int).unwrap_or(0) as f64;
            let hue = (v("hue") / 60_000.0).rem_euclid(360.0) / 360.0;
            from_hsl(hue, (v("sat") / 100_000.0).clamp(0.0, 1.0), (v("lum") / 100_000.0).clamp(0.0, 1.0))
        }
        _ => return None,
    };
    let val = |m: &El| m.attr("val").and_then(int).map_or(0.0, |v| (v as f64 / 100_000.0).clamp(-10.0, 10.0));
    let mut rgb = base;
    for m in c.els() {
        rgb = match m.name.as_str() {
            "a:lumMod" => adjust_lum(rgb, |l| l * val(m)),
            "a:lumOff" => adjust_lum(rgb, |l| l + val(m)),
            "a:tint" => mix(rgb, 255.0, val(m)),
            "a:shade" => mix(rgb, 0.0, val(m)),
            _ => rgb,
        };
    }
    Some(rgb)
}

/// A preset colour (`a:prstClr/@val`, ECMA-376 Part 1 §20.1.10.48): the common ones; others are
/// unknown (`None`).
pub(crate) fn preset_color(name: &str) -> Option<Rgb> {
    Some(match name {
        "black" => Rgb(0, 0, 0),
        "white" => Rgb(0xFF, 0xFF, 0xFF),
        "red" => Rgb(0xFF, 0, 0),
        "green" => Rgb(0, 0x80, 0),
        "lime" => Rgb(0, 0xFF, 0),
        "blue" => Rgb(0, 0, 0xFF),
        "yellow" => Rgb(0xFF, 0xFF, 0),
        "cyan" | "aqua" => Rgb(0, 0xFF, 0xFF),
        "magenta" | "fuchsia" => Rgb(0xFF, 0, 0xFF),
        "gray" | "grey" => Rgb(0x80, 0x80, 0x80),
        "silver" => Rgb(0xC0, 0xC0, 0xC0),
        "ltGray" | "ltGrey" => Rgb(0xD3, 0xD3, 0xD3),
        "dkGray" | "dkGrey" => Rgb(0xA9, 0xA9, 0xA9),
        "maroon" => Rgb(0x80, 0, 0),
        "navy" => Rgb(0, 0, 0x80),
        "olive" => Rgb(0x80, 0x80, 0),
        "purple" => Rgb(0x80, 0, 0x80),
        "teal" => Rgb(0, 0x80, 0x80),
        "orange" => Rgb(0xFF, 0xA5, 0),
        "brown" => Rgb(0xA5, 0x2A, 0x2A),
        "pink" => Rgb(0xFF, 0xC0, 0xCB),
        _ => return None,
    })
}

/// A system colour without its `lastClr`: the usual light-theme values of the common ones.
fn sys_color(name: &str) -> Option<Rgb> {
    Some(match name {
        "windowText" | "btnText" | "menuText" | "captionText" | "infoText" => Rgb(0, 0, 0),
        "window" | "menu" | "highlightText" => Rgb(0xFF, 0xFF, 0xFF),
        "btnFace" | "3dLight" => Rgb(0xF0, 0xF0, 0xF0),
        "grayText" | "btnShadow" => Rgb(0xA0, 0xA0, 0xA0),
        "highlight" => Rgb(0x00, 0x78, 0xD7),
        _ => return None,
    })
}

/// Index into `Document::settings.theme_colors` (dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink).
fn scheme_slot(name: &str) -> Option<usize> {
    Some(match name {
        "dk1" | "tx1" => 0,
        "lt1" | "bg1" => 1,
        "dk2" | "tx2" => 2,
        "lt2" | "bg2" => 3,
        "accent1" => 4,
        "accent2" => 5,
        "accent3" => 6,
        "accent4" => 7,
        "accent5" => 8,
        "accent6" => 9,
        "hlink" => 10,
        "folHlink" => 11,
        _ => return None,
    })
}

/// Theme colour `name` (`accent1`, `tx1`…) of `theme`.
pub(crate) fn scheme(name: &str, theme: &[Rgb]) -> Option<Rgb> {
    scheme_slot(name).map(|i| slot(theme, i))
}

/// `c` with its luminance changed by `f`.
pub(crate) fn lum(c: Rgb, f: impl Fn(f64) -> f64) -> Rgb {
    adjust_lum(c, f)
}

/// `tint` (towards `to` = 255) and `shade` (towards 0): `v` is the share of the original colour kept.
pub(crate) fn mix(c: Rgb, to: f64, v: f64) -> Rgb {
    let f = |x: u8| (x as f64 * v + to * (1.0 - v)).round().clamp(0.0, 255.0) as u8;
    Rgb(f(c.0), f(c.1), f(c.2))
}

/// `c` with its HSL luminance changed by `f`.
pub(crate) fn adjust_lum(c: Rgb, f: impl Fn(f64) -> f64) -> Rgb {
    let (h, s, l) = to_hsl(c);
    from_hsl(h, s, f(l).clamp(0.0, 1.0))
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    if s == 0.0 {
        return Rgb(byte(l), byte(l), byte(l));
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |t: f64| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        byte(v)
    };
    Rgb(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use wordcraft_doc::THEME_COLORS;

    use super::*;

    const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

    /// The element `name` with `attrs` and `body`, parsed.
    fn el(name: &str, attrs: &str, body: &str) -> El {
        crate::xml::parse(format!(r#"<{name} {NS} {attrs}>{body}</{name}>"#).as_bytes()).unwrap()
    }

    #[test]
    fn gradient_and_pattern_fills_take_their_first_colour() {
        let theme = THEME_COLORS.to_vec();
        let grad = el("a:gradFill", "", r#"<a:gsLst><a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs></a:gsLst>"#);
        assert_eq!(paint(&grad, &theme), Some(Rgb(255, 0, 0)));
        let patt =
            el("a:pattFill", r#"prst="dkDnDiag""#, r#"<a:fgClr><a:srgbClr val="00FF00"/></a:fgClr><a:bgClr><a:srgbClr val="0000FF"/></a:bgClr>"#);
        assert_eq!(paint(&patt, &theme), Some(Rgb(0, 255, 0)));
        let ln = el("a:ln", "", r#"<a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs></a:gsLst></a:gradFill>"#);
        assert_eq!(fill_colour(&ln, &theme), Some(Rgb(255, 0, 0)));
        assert_eq!(fill_colour(&el("a:ln", "", "<a:noFill/>"), &theme), None);
    }

    #[test]
    fn tint_and_shade_move_towards_white_and_black() {
        let theme = THEME_COLORS.to_vec();
        let near = |c: Option<Rgb>| c.is_some_and(|Rgb(r, g, b)| [r, g, b].iter().all(|x| x.abs_diff(128) <= 1));
        assert!(near(color(&el("a:srgbClr", r#"val="000000""#, r#"<a:tint val="50000"/>"#), &theme)));
        assert!(near(color(&el("a:srgbClr", r#"val="FFFFFF""#, r#"<a:shade val="50000"/>"#), &theme)));
    }

    #[test]
    fn modifiers_apply_in_the_order_written() {
        let theme = THEME_COLORS.to_vec();
        let a = el("a:srgbClr", r#"val="808080""#, r#"<a:lumMod val="50000"/><a:lumOff val="50000"/>"#);
        let b = el("a:srgbClr", r#"val="808080""#, r#"<a:lumOff val="50000"/><a:lumMod val="50000"/>"#);
        assert_ne!(color(&a, &theme), color(&b, &theme));
    }
}
