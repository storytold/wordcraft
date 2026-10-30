//! How a Word shape (`wps:wsp`) looks: its preset geometry, and the fill and outline it gets from its own properties (`wps:spPr`) or, failing those, from the theme through its
//! style references (`wps:style`: `a:fillRef`, `a:lnRef`, ECMA-376 Part 1 §20.1.4.2).

use std::sync::Arc;

use wordcraft_doc::fill::{Fill, Gradient, GradientFlip, GradientKind, PathShape};
use wordcraft_doc::para::{PictureShape, ShapeKind};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::wordart::{GradStop, MAX_STOPS};

use super::drawing_color::{color, color_alpha, fill as read_fill, fill_colour};
use crate::units::{int, measure};
use crate::xml::El;

/// Outline widths of the theme's three line styles (`a:lnStyleLst`), points: what Word's
/// default themes give (6 350, 12 700 and 19 050 EMU).
const THEME_LINE_WIDTHS: [f32; 3] = [0.5, 1.0, 1.5];

/// The shape kind drawn for DrawingML preset geometry `prst` (ECMA-376 Part 1 §20.1.10.56).
/// Presets WordCraft has no shape for are drawn as the nearest one it has: callouts as the box,
/// round or cloud they point from, flowchart boxes as rectangles.
pub(crate) fn preset_kind(prst: &str) -> ShapeKind {
    match prst {
        "roundRect"
        | "round1Rect"
        | "round2SameRect"
        | "round2DiagRect"
        | "snipRoundRect"
        | "wedgeRoundRectCallout"
        | "flowChartAlternateProcess"
        | "flowChartTerminator"
        | "plaque" => ShapeKind::RoundedRectangle,
        "ellipse" | "wedgeEllipseCallout" | "cloudCallout" | "cloud" | "flowChartConnector" | "donut" | "flowChartOnlineStorage" => {
            ShapeKind::Ellipse
        }
        "flowChartOr" => ShapeKind::FlowchartOr,
        "triangle" | "flowChartExtract" | "flowChartMerge" => ShapeKind::Triangle,
        "rtTriangle" => ShapeKind::RightTriangle,
        "diamond" | "flowChartDecision" => ShapeKind::Diamond,
        "line" | "lineInv" => ShapeKind::Line,
        "straightConnector1" => ShapeKind::StraightConnector,
        "bentConnector2" | "bentConnector3" | "bentConnector4" | "bentConnector5" => ShapeKind::ElbowConnector,
        "curvedConnector2" | "curvedConnector3" | "curvedConnector4" | "curvedConnector5" => ShapeKind::CurvedConnector,
        "rightArrow" | "leftRightArrow" | "upDownArrow" | "notchedRightArrow" | "stripedRightArrow" => ShapeKind::Arrow,
        "leftArrow" => ShapeKind::ArrowLeft,
        "upArrow" => ShapeKind::ArrowUp,
        "downArrow" => ShapeKind::ArrowDown,
        "star4" | "star5" | "star6" | "star7" | "star8" | "star10" | "star12" | "star16" | "star24" | "star32" => ShapeKind::Star,
        "heart" => ShapeKind::Heart,
        _ => ShapeKind::Rectangle,
    }
}

/// The DrawingML preset geometry (ECMA-376 Part 1 §20.1.10.56) written for a shape kind.
pub(crate) fn preset_name(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::Freeform => "rect",
        ShapeKind::RoundedRectangle => "roundRect",
        ShapeKind::Ellipse => "ellipse",
        ShapeKind::FlowchartOr => "flowChartOr",
        ShapeKind::Triangle => "triangle",
        ShapeKind::RightTriangle => "rtTriangle",
        ShapeKind::Diamond => "diamond",
        ShapeKind::Line => "line",
        ShapeKind::StraightConnector => "straightConnector1",
        ShapeKind::ElbowConnector => "bentConnector3",
        ShapeKind::CurvedConnector => "curvedConnector3",
        ShapeKind::Arrow => "rightArrow",
        ShapeKind::ArrowLeft => "leftArrow",
        ShapeKind::ArrowUp => "upArrow",
        ShapeKind::ArrowDown => "downArrow",
        ShapeKind::Star => "star5",
        ShapeKind::Heart => "heart",
    }
}

/// The most adjust values kept from a picture's preset outline, and the longest name or formula.
const MAX_ADJUSTS: usize = 16;
const MAX_ADJUST_LEN: usize = 256;

/// A picture's Crop to Shape outline from its `a:prstGeom` (ECMA-376 Part 1 §20.1.9.18): drawn as
/// the nearest shape WordCraft has (a rectangle for presets it doesn't know, or open ones), and
/// the file's own preset name and adjust values kept when that isn't exactly the shape's own, so
/// saving writes them back unchanged. None for a plain rectangle or a malformed name.
pub(crate) fn picture_shape(geom: &El) -> Option<PictureShape> {
    let prst = geom.attr("prst")?;
    if prst.is_empty() || prst.len() > 64 || !prst.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let av: Vec<(String, String)> = geom
        .child("a:avLst")
        .map(|l| {
            l.children("a:gd")
                .filter_map(|gd| Some((gd.attr("name")?, gd.attr("fmla")?)))
                .filter(|(n, f)| n.len() <= MAX_ADJUST_LEN && f.len() <= MAX_ADJUST_LEN)
                .take(MAX_ADJUSTS)
                .map(|(n, f)| (n.to_string(), f.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if prst == "rect" && av.is_empty() {
        return None;
    }
    let kind = match preset_kind(prst) {
        k if k.is_open() => ShapeKind::Rectangle,
        k => k,
    };
    let prst = if preset_name(kind) == prst && av.is_empty() { String::new() } else { prst.to_string() };
    Some(PictureShape { kind, prst, av })
}

/// The shape kind drawn for a legacy VML preset shape type `spt` (`o:spt`, the MSOSPT numbers of
/// [MS-ODRAW] §2.4.24): that of the DrawingML preset of the same shape, through [`preset_kind`],
/// so a preset WordCraft has no shape for is drawn as the nearest one; None for shapes with no
/// preset geometry (freeforms, WordArt text, picture frames) and numbers MSOSPT doesn't define.
pub(crate) fn vml_preset_kind(spt: u32) -> Option<ShapeKind> {
    let prst = match spt {
        // A text box's shapetype: a text box even without text, as DrawingML's `txBox`.
        202 => return Some(ShapeKind::TextBox),
        // Not primitive (freeform), WordArt text shapes, picture frame, host control.
        0 | 24..=31 | 75 | 136..=175 | 201 => return None,
        // Line and straight connector: corner to corner of the shape's box.
        20 | 32 => "line",
        2 => "roundRect",
        3 => "ellipse",
        4 => "diamond",
        5 => "triangle",
        6 => "rtTriangle",
        12 => "star5",
        13 => "rightArrow",
        21 => "plaque",
        23 => "donut",
        33 => "bentConnector2",
        34 => "bentConnector3",
        35 => "bentConnector4",
        36 => "bentConnector5",
        37 => "curvedConnector2",
        38 => "curvedConnector3",
        39 => "curvedConnector4",
        40 => "curvedConnector5",
        58 => "star8",
        59 => "star16",
        60 => "star32",
        62 => "wedgeRoundRectCallout",
        63 => "wedgeEllipseCallout",
        66 => "leftArrow",
        67 => "downArrow",
        68 => "upArrow",
        69 => "leftRightArrow",
        70 => "upDownArrow",
        74 => "heart",
        92 => "star24",
        93 => "stripedRightArrow",
        94 => "notchedRightArrow",
        106 => "cloudCallout",
        110 => "flowChartDecision",
        116 => "flowChartTerminator",
        120 => "flowChartConnector",
        127 => "flowChartExtract",
        128 => "flowChartMerge",
        130 => "flowChartOnlineStorage",
        176 => "flowChartAlternateProcess",
        187 => "star4",
        // Every other preset is drawn as its box, as `preset_kind` does.
        1..=201 => "rect",
        _ => return None,
    };
    Some(preset_kind(prst))
}

/// A shape's fill and outline.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ShapePaint {
    pub fill: Option<Fill>,
    pub stroke: Option<Rgb>,
    /// Points.
    pub stroke_width: f32,
    /// The outline's gradient (drawn in its main colour, `stroke`).
    pub outline_fill: Option<Fill>,
}

/// The theme's fill styles (`a:fmtScheme`): `a:fillStyleLst` and `a:bgFillStyleLst`, which a
/// shape's `a:fillRef` 1–999 and 1001+ point to.
#[derive(Debug, Default)]
pub(crate) struct ThemeFills {
    pub fill: Vec<El>,
    pub bg: Vec<El>,
}

impl ThemeFills {
    pub fn read(fmt: Option<&El>) -> ThemeFills {
        let list = |n: &str| fmt.and_then(|f| f.child(n)).map(|l| l.els().take(MAX_THEME_FILLS).cloned().collect()).unwrap_or_default();
        ThemeFills { fill: list("a:fillStyleLst"), bg: list("a:bgFillStyleLst") }
    }
    /// Style `idx` (1-based; 1001+ the background styles; 1000 names none).
    fn get(&self, idx: i64) -> Option<&El> {
        let (list, i) = if idx > 1000 { (&self.bg, idx - 1001) } else { (&self.fill, idx - 1) };
        list.get(usize::try_from(i).ok()?)
    }
}

/// Most fill styles kept per theme list: every one a `a:fillRef` can reach (1–999 into
/// `a:fillStyleLst`, 1001–1999 into `a:bgFillStyleLst`, ECMA-376 Part 1 §20.1.4.2.10), and no
/// more, so a hostile theme with millions of entries keeps a bounded list.
const MAX_THEME_FILLS: usize = 999;

/// The fill a `…spPr` element states itself: `Some(None)` for none (`a:noFill`, picture fills),
/// `group` for `a:grpFill`, `None` when it leaves the fill to the shape's style.
pub(crate) fn own_fill(sppr: Option<&El>, theme: &[Rgb], group: Option<&Fill>) -> Option<Option<Fill>> {
    let f = sppr?.els().find(|c| matches!(c.name.as_str(), "a:noFill" | "a:blipFill" | "a:grpFill" | "a:solidFill" | "a:gradFill" | "a:pattFill"))?;
    Some(match f.name.as_str() {
        "a:grpFill" => group.cloned(),
        _ => read_fill(f, theme, None),
    })
}

/// The paint of a shape from its properties `sppr`, falling back on its style `style` (theme
/// colours `theme`, fill styles `styles`) for what they leave out: an explicit `a:noFill` is no
/// fill or outline, an explicit colour wins, `a:grpFill` is the enclosing group's fill `group`,
/// and a style reference with index 0 is none.
pub(crate) fn shape_paint(sppr: Option<&El>, style: Option<&El>, theme: &[Rgb], styles: &ThemeFills, group: Option<&Fill>) -> ShapePaint {
    let style_ref = |name: &str| -> Option<(i64, &El)> {
        let r = style?.child(name)?;
        Some((r.attr("idx").and_then(int).unwrap_or(0), r))
    };
    let fill = own_fill(sppr, theme, group).unwrap_or_else(|| {
        let (idx, r) = style_ref("a:fillRef").filter(|(idx, _)| *idx > 0)?;
        // The theme style in the reference's colour (`phClr`), else that colour: also for a style
        // the model can't hold (a picture fill), so the shape doesn't vanish.
        let ph = color_alpha(r, theme, None)?;
        let solid = Fill::Solid { color: ph.0, transparency: ph.1 };
        match styles.get(idx) {
            Some(st) if st.name == "a:noFill" => None,
            Some(st) => read_fill(st, theme, Some(ph)).or(Some(solid)),
            None => Some(solid),
        }
    });
    let ln = sppr.and_then(|s| s.child("a:ln"));
    let line_ref = style_ref("a:lnRef").filter(|(idx, _)| *idx > 0);
    let outline_fill = ln.and_then(|l| l.child("a:gradFill")).and_then(|g| read_fill(g, theme, None)).filter(|f| matches!(f, Fill::Gradient(_)));
    let stroke = match ln {
        Some(l) if l.child("a:noFill").is_some() => None,
        _ if outline_fill.is_some() => outline_fill.as_ref().map(Fill::main_color),
        Some(l) if l.els().any(|c| matches!(c.name.as_str(), "a:solidFill" | "a:gradFill" | "a:pattFill")) => fill_colour(l, theme),
        _ => line_ref.and_then(|(_, r)| color(r, theme)),
    };
    let themed_width = line_ref.and_then(|(idx, _)| THEME_LINE_WIDTHS.get(usize::try_from(idx.clamp(1, 3) - 1).unwrap_or(0)).copied());
    let stroke_width = ln
        .and_then(|l| l.attr("w"))
        .and_then(|v| measure(v, 12_700.0))
        .or(themed_width)
        // Word's default outline when the file names none (no outline: no width).
        .unwrap_or(if stroke.is_some() { 0.75 } else { 0.0 })
        .clamp(0.0, 100.0);
    ShapePaint { fill, stroke, stroke_width, outline_fill }
}

/// A VML shape's fill: `fillcolor` (white by default) and its `v:fill`: opacity, and a linear
/// (`type="gradient"`) or circular (`gradientRadial`) gradient to `color2`, or through `colors`.
/// A gradient's `focus` (−100%–100%) and `angle` are approximated: 0 and 100% run `color` to
/// `color2`, −100% the other way, and a focus between mirrors the gradient there (`color2`
/// there for a positive focus, `color` for a negative one). `None` when not filled.
pub(crate) fn vml_fill(shape: &El, shapetype: Option<&El>) -> Option<Fill> {
    // What the shape leaves out comes from its shapetype (#1377).
    let own = |n: &str| shape.attr(n).or_else(|| shapetype.and_then(|t| t.attr(n)));
    let f = shape.child("v:fill").or_else(|| shapetype.and_then(|t| t.child("v:fill")));
    let attr = |n: &str| f.and_then(|f| f.attr(n));
    let off = |v: Option<&str>| {
        v.is_some_and(|v| {
            let t = v.trim();
            t.eq_ignore_ascii_case("f") || t.eq_ignore_ascii_case("false") || t == "0"
        })
    };
    if off(own("filled")) || off(attr("on")) {
        return None;
    }
    let c1 = attr("color").or(own("fillcolor")).map_or(Some(Rgb::WHITE), vml_color)?;
    let t1 = 100.0 - attr("opacity").and_then(vml_fraction).unwrap_or(1.0) * 100.0;
    let radial = match attr("type") {
        Some("gradient") => false,
        Some("gradientRadial") => true,
        _ => return Fill::Solid { color: c1, transparency: t1 }.sanitized(),
    };
    let c2 = attr("color2").and_then(vml_color).unwrap_or(Rgb::WHITE);
    let t2 = 100.0 - attr("o:opacity2").and_then(vml_fraction).unwrap_or(1.0) * 100.0;
    let stop = |pos: f32, (color, transparency): (Rgb, f32)| GradStop { pos, color, transparency };
    let (a, b) = ((c1, t1), (c2, t2));
    let focus = attr("focus").and_then(vml_fraction).unwrap_or(0.0).clamp(-1.0, 1.0) * 100.0;
    let colors: Vec<GradStop> = attr("colors")
        .into_iter()
        .flat_map(|v| v.split(';'))
        .take(MAX_STOPS)
        .filter_map(|s| {
            let (pos, c) = s.trim().split_once(' ')?;
            Some(stop(vml_fraction(pos)? * 100.0, (vml_color(c)?, t1)))
        })
        .collect();
    let stops = if colors.len() >= 2 {
        colors
    } else if focus <= -100.0 {
        vec![stop(0.0, b), stop(100.0, a)]
    } else if focus >= 100.0 || focus == 0.0 {
        vec![stop(0.0, a), stop(100.0, b)]
    } else if focus > 0.0 {
        vec![stop(0.0, a), stop(focus, b), stop(100.0, a)]
    } else {
        vec![stop(0.0, b), stop(-focus, a), stop(100.0, b)]
    };
    let kind = if radial {
        // `focusposition` "x,y" (fractions of the shape) is the centre.
        let fp = attr("focusposition").unwrap_or("0.5,0.5");
        let (x, y) = fp.split_once(',').unwrap_or((fp, fp));
        let (x, y) = (vml_fraction(x).unwrap_or(0.5) * 100.0, vml_fraction(y).unwrap_or(0.5) * 100.0);
        GradientKind::Path { shape: PathShape::Circle, focus: [x, y, 100.0 - x, 100.0 - y] }
    } else {
        // VML's angle is counter-clockwise from a top-to-bottom gradient.
        let ang = attr("angle").and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0);
        GradientKind::Linear { angle: 90.0 - ang, scaled: false }
    };
    Fill::Gradient(Arc::new(Gradient { stops, kind, tile: None, rot_with_shape: true, flip: GradientFlip::None })).sanitized()
}

/// A VML fraction (`opacity`, `focus`, a `colors` position): `0.5`, `50%`, or 65536ths
/// (`32768f`). Not clamped: a focus runs from -1 to 1.
pub(crate) fn vml_fraction(v: &str) -> Option<f32> {
    let v = v.trim();
    let n = if let Some(f) = v.strip_suffix('f') {
        f.trim().parse::<f32>().ok()? / 65_536.0
    } else if let Some(p) = v.strip_suffix('%') {
        p.trim().parse::<f32>().ok()? / 100.0
    } else {
        v.parse::<f32>().ok()?
    };
    n.is_finite().then_some(n)
}

/// Explicit DrawingML dash stops override the solid default.
pub(crate) fn line_dash(ln: Option<&El>) -> wordcraft_doc::line_dash::LineDash {
    use wordcraft_doc::line_dash::{LineDash, MAX_DASH_STOPS};
    let Some(ln) = ln else { return LineDash::Solid };
    if let Some(custom) = ln.child("a:custDash") {
        let stops: Vec<_> =
            custom.children("a:ds").take(MAX_DASH_STOPS).filter_map(|e| Some((dash_ratio(e.attr("d")?)?, dash_ratio(e.attr("sp")?)?))).collect();
        return if stops.is_empty() { LineDash::Solid } else { LineDash::Custom(stops) };
    }
    ln.child("a:prstDash").and_then(|e| e.attr("val")).map(LineDash::from_ooxml).unwrap_or_default()
}

/// Strict OOXML may spell percentages with a percent sign instead of integer units.
fn dash_ratio(value: &str) -> Option<u32> {
    const PERCENT_UNITS: f64 = 1000.0;
    let value = value.trim();
    let Some(percent) = value.strip_suffix('%') else { return value.parse().ok() };
    let units = percent.parse::<f64>().ok()? * PERCENT_UNITS;
    if !units.is_finite() || !(0.0..=f64::from(u32::MAX)).contains(&units) {
        return None;
    }
    Some(units.round() as u32)
}

/// A VML colour (`strokecolor`, `fillcolor`): `#rrggbb`, `rrggbb`, `#rgb` or `rgb` (Word follows it
/// with the theme slot, as in `#4472c4 [3204]` or `4472c4 [3204]`), an `rgb(...)` function, or a
/// colour name.
pub(crate) fn vml_color(v: &str) -> Option<Rgb> {
    let s = v.trim();
    if let Some(inner) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")) {
        let nums: Vec<u8> = inner
            .trim_end_matches(')')
            .split(',')
            .take(3)
            .filter_map(|x| {
                let x = x.trim();
                if let Some(p) = x.strip_suffix('%') {
                    p.trim().parse::<f32>().ok().map(|f| (f.clamp(0.0, 100.0) * 2.55).round() as u8)
                } else {
                    x.parse::<f32>().ok().map(|f| f.clamp(0.0, 255.0).round() as u8)
                }
            })
            .collect();
        if let [r, g, b] = nums[..] {
            return Some(Rgb(r, g, b));
        }
    }
    let tok = s.split_whitespace().next()?;
    let hex = tok.strip_prefix('#').unwrap_or(tok);
    if hex.len() == 3 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let long: String = hex.chars().flat_map(|c| [c, c]).collect();
        return Rgb::parse(&long);
    }
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Rgb::parse(hex);
    }
    super::drawing_color::preset_color(tok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::THEME_COLORS;

    fn el(xml: &str) -> El {
        let ns = r#"xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
        crate::xml::parse(xml.replace(r#"xmlns:wps="w" xmlns:a="a""#, ns).as_bytes()).unwrap()
    }

    #[test]
    fn theme_styled_connector_gets_its_line_from_the_style() {
        // A connector with no colour of its own (what Word writes for Insert › Shapes › Line):
        // its outline comes from `a:lnRef`, its width from the theme's first line style.
        let sp = el(r#"<wps:spPr xmlns:wps="w" xmlns:a="a"><a:prstGeom prst="line"/><a:ln><a:tailEnd type="triangle"/></a:ln></wps:spPr>"#);
        let st = el(
            r#"<wps:style xmlns:wps="w" xmlns:a="a"><a:lnRef idx="1"><a:schemeClr val="accent4"/></a:lnRef><a:fillRef idx="0"><a:schemeClr val="accent4"/></a:fillRef></wps:style>"#,
        );
        let p = shape_paint(Some(&sp), Some(&st), &THEME_COLORS, &ThemeFills::default(), None);
        assert_eq!(p.stroke, Some(THEME_COLORS[7]));
        assert_eq!(p.stroke_width, 0.5);
        assert_eq!(p.fill, None);
    }

    #[test]
    fn flowchart_or_keeps_its_spokes() {
        assert_eq!(preset_kind("flowChartOr"), ShapeKind::FlowchartOr);
    }

    #[test]
    fn explicit_paint_wins_over_the_style() {
        let sp = el(r#"<wps:spPr xmlns:wps="w" xmlns:a="a"><a:noFill/><a:ln><a:solidFill><a:srgbClr val="EE0000"/></a:solidFill></a:ln></wps:spPr>"#);
        let st = el(
            r#"<wps:style xmlns:wps="w" xmlns:a="a"><a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="15000"/></a:schemeClr></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef></wps:style>"#,
        );
        let p = shape_paint(Some(&sp), Some(&st), &THEME_COLORS, &ThemeFills::default(), None);
        assert_eq!((p.fill, p.stroke, p.stroke_width), (None, Some(Rgb(0xEE, 0, 0)), 1.0));
        // Preset and scheme colours as the box's own fill and outline.
        let sp = el(
            r#"<wps:spPr xmlns:wps="w" xmlns:a="a"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill><a:ln w="6350"><a:solidFill><a:prstClr val="black"/></a:solidFill></a:ln></wps:spPr>"#,
        );
        let p = shape_paint(Some(&sp), None, &THEME_COLORS, &ThemeFills::default(), None);
        assert_eq!((p.fill, p.stroke, p.stroke_width), (Some(Fill::solid(THEME_COLORS[1])), Some(Rgb(0, 0, 0)), 0.5));
        // A style fill with index 0 is none; an outline set to no fill is none.
        let sp = el(r#"<wps:spPr xmlns:wps="w" xmlns:a="a"><a:ln><a:noFill/></a:ln></wps:spPr>"#);
        let st = el(
            r#"<wps:style xmlns:wps="w" xmlns:a="a"><a:lnRef idx="2"><a:schemeClr val="accent1"/></a:lnRef><a:fillRef idx="0"><a:schemeClr val="accent1"/></a:fillRef></wps:style>"#,
        );
        let p = shape_paint(Some(&sp), Some(&st), &THEME_COLORS, &ThemeFills::default(), None);
        assert_eq!((p.fill, p.stroke), (None, None));
    }

    #[test]
    fn a_theme_picture_fill_falls_back_on_the_reference_colour() {
        let fmt = el(
            r#"<a:fmtScheme xmlns:wps="w" xmlns:a="a"><a:fillStyleLst><a:noFill/><a:blipFill><a:blip/></a:blipFill></a:fillStyleLst></a:fmtScheme>"#,
        );
        let styles = ThemeFills::read(Some(&fmt));
        let st = |idx: u8| {
            el(&format!(r#"<wps:style xmlns:wps="w" xmlns:a="a"><a:fillRef idx="{idx}"><a:srgbClr val="112233"/></a:fillRef></wps:style>"#))
        };
        assert_eq!(shape_paint(None, Some(&st(2)), &THEME_COLORS, &styles, None).fill, Some(Fill::solid(Rgb(0x11, 0x22, 0x33))));
        // An explicit theme `a:noFill` stays none.
        assert_eq!(shape_paint(None, Some(&st(1)), &THEME_COLORS, &styles, None).fill, None);
    }

    #[test]
    fn every_reachable_theme_fill_style_is_kept() {
        // 20 fill styles and 20 background styles: the 17th of each a gradient / none, the rest
        // solid, so a reference past Word's usual three still finds its style.
        let entries = |odd: &str| {
            (1..=20)
                .map(|i| if i == 17 { odd.to_string() } else { r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#.to_string() })
                .collect::<String>()
        };
        let grad = r#"<a:gradFill><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"/></a:gs><a:gs pos="100000"><a:srgbClr val="FFFFFF"/></a:gs></a:gsLst><a:lin ang="0"/></a:gradFill>"#;
        let fmt = el(&format!(
            r#"<a:fmtScheme xmlns:wps="w" xmlns:a="a"><a:fillStyleLst>{}</a:fillStyleLst><a:bgFillStyleLst>{}</a:bgFillStyleLst></a:fmtScheme>"#,
            entries(grad),
            entries("<a:noFill/>")
        ));
        let styles = ThemeFills::read(Some(&fmt));
        let st = |idx: u32| {
            el(&format!(r#"<wps:style xmlns:wps="w" xmlns:a="a"><a:fillRef idx="{idx}"><a:srgbClr val="112233"/></a:fillRef></wps:style>"#))
        };
        let fill = |idx| shape_paint(None, Some(&st(idx)), &THEME_COLORS, &styles, None).fill;
        assert!(matches!(fill(17), Some(Fill::Gradient(_))), "{:?}", fill(17));
        assert_eq!(fill(1017), None);
        assert_eq!(fill(16), Some(Fill::solid(Rgb(0x11, 0x22, 0x33))));
    }

    #[test]
    fn a_theme_with_absurdly_many_fill_styles_stays_bounded() {
        let many = "<a:noFill/>".repeat(100_000);
        let fmt = el(&format!(
            r#"<a:fmtScheme xmlns:wps="w" xmlns:a="a"><a:fillStyleLst>{many}</a:fillStyleLst><a:bgFillStyleLst>{many}</a:bgFillStyleLst></a:fmtScheme>"#
        ));
        let styles = ThemeFills::read(Some(&fmt));
        assert_eq!((styles.fill.len(), styles.bg.len()), (MAX_THEME_FILLS, MAX_THEME_FILLS));
        let st = |idx: &str| {
            el(&format!(r#"<wps:style xmlns:wps="w" xmlns:a="a"><a:fillRef idx="{idx}"><a:srgbClr val="112233"/></a:fillRef></wps:style>"#))
        };
        // The last reachable style of each list is kept; past them (and 1000) the reference colour.
        for (idx, want) in [
            ("999", None),
            ("1999", None),
            ("1000", Some(Fill::solid(Rgb(0x11, 0x22, 0x33)))),
            ("2000", Some(Fill::solid(Rgb(0x11, 0x22, 0x33)))),
            ("99999999999", Some(Fill::solid(Rgb(0x11, 0x22, 0x33)))),
        ] {
            assert_eq!(shape_paint(None, Some(&st(idx)), &THEME_COLORS, &styles, None).fill, want, "idx {idx}");
        }
    }

    #[test]
    fn hostile_style_references_do_not_panic() {
        let st = el(
            r#"<wps:style xmlns:wps="w" xmlns:a="a"><a:lnRef idx="-99999999999"><a:schemeClr val="nope"/></a:lnRef><a:fillRef idx="99999999999"/></wps:style>"#,
        );
        let p = shape_paint(None, Some(&st), &[], &ThemeFills::default(), None);
        assert_eq!((p.fill, p.stroke), (None, None));
        let st = el(r#"<wps:style xmlns:wps="w" xmlns:a="a"><a:lnRef idx="77"><a:srgbClr val="010203"/></a:lnRef></wps:style>"#);
        assert_eq!(shape_paint(None, Some(&st), &[], &ThemeFills::default(), None).stroke_width, 1.5);
    }

    #[test]
    fn presets_map_to_the_nearest_shape() {
        assert_eq!(preset_kind("straightConnector1"), ShapeKind::StraightConnector);
        assert_eq!(preset_kind("bentConnector3"), ShapeKind::ElbowConnector);
        assert_eq!(preset_kind("wedgeRoundRectCallout"), ShapeKind::RoundedRectangle);
        assert_eq!(preset_kind("wedgeEllipseCallout"), ShapeKind::Ellipse);
        assert_eq!(preset_kind("wedgeRectCallout"), ShapeKind::Rectangle);
        assert_eq!(preset_kind("flowChartDecision"), ShapeKind::Diamond);
    }

    /// #3919: Word's up, down and left block arrows were all read as the right-pointing arrow.
    #[test]
    fn block_arrows_keep_their_direction() {
        assert_eq!(preset_kind("rightArrow"), ShapeKind::Arrow);
        assert_eq!(preset_kind("leftArrow"), ShapeKind::ArrowLeft);
        assert_eq!(preset_kind("upArrow"), ShapeKind::ArrowUp);
        assert_eq!(preset_kind("downArrow"), ShapeKind::ArrowDown);
    }

    #[test]
    fn vml_presets_map_like_drawingml_presets() {
        assert_eq!(vml_preset_kind(13), Some(ShapeKind::Arrow));
        assert_eq!(vml_preset_kind(12), Some(ShapeKind::Star));
        assert_eq!(vml_preset_kind(32), Some(ShapeKind::Line));
        assert_eq!(vml_preset_kind(202), Some(ShapeKind::TextBox));
        // The nearest shape, as `preset_kind` gives it: left arrow, right triangle, four-point
        // star, rounded callout, elbow connector, a bevel (its box).
        assert_eq!(vml_preset_kind(66), Some(ShapeKind::ArrowLeft));
        assert_eq!(vml_preset_kind(6), Some(ShapeKind::RightTriangle));
        assert_eq!(vml_preset_kind(187), Some(ShapeKind::Star));
        assert_eq!(vml_preset_kind(62), Some(ShapeKind::RoundedRectangle));
        assert_eq!(vml_preset_kind(34), Some(ShapeKind::ElbowConnector));
        assert_eq!(vml_preset_kind(84), Some(ShapeKind::Rectangle));
        // No preset geometry: freeform, WordArt text, picture frame, host control, out of range.
        for spt in [0, 24, 31, 136, 175, 75, 201, 203, u32::MAX] {
            assert_eq!(vml_preset_kind(spt), None, "{spt}");
        }
    }

    #[test]
    fn vml_gradients_focus_and_colours() {
        let v = |attrs: &str, fill: &str| {
            let ns = r#"xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#;
            vml_fill(&crate::xml::parse(format!(r#"<v:rect {ns} {attrs}>{fill}</v:rect>"#).as_bytes()).unwrap(), None)
        };
        let stops = |f: Option<Fill>| match f {
            Some(Fill::Gradient(g)) => g.stops.iter().map(|s| (s.pos, s.color)).collect::<Vec<_>>(),
            o => panic!("{o:?}"),
        };
        let (red, blue) = (Rgb(255, 0, 0), Rgb(0, 0, 255));
        // A focus of 50% mirrors the gradient in the middle; 100% runs as 0%, −100% reverses it.
        assert_eq!(
            stops(v(r##"fillcolor="red""##, r##"<v:fill type="gradient" color2="blue" focus="50%"/>"##)),
            [(0.0, red), (50.0, blue), (100.0, red)]
        );
        assert_eq!(stops(v(r##"fillcolor="red""##, r##"<v:fill type="gradient" color2="blue" focus="100%"/>"##)), [(0.0, red), (100.0, blue)]);
        assert_eq!(stops(v(r##"fillcolor="red""##, r##"<v:fill type="gradient" color2="blue" focus="-100%"/>"##)), [(0.0, blue), (100.0, red)]);
        assert_eq!(v("", r##"<v:fill opacity="-2"/>"##), Some(Fill::Solid { color: Rgb::WHITE, transparency: 100.0 }));
        // `colors` lists the stops (fractions or 65536ths).
        assert_eq!(stops(v("", r##"<v:fill type="gradient" colors="0 red;32768f blue;1 red"/>"##)), [(0.0, red), (50.0, blue), (100.0, red)]);
        let Some(Fill::Gradient(g)) = v(r##"fillcolor="red""##, r##"<v:fill type="gradientRadial" color2="blue" focusposition=".25,.75"/>"##) else {
            panic!()
        };
        assert_eq!(g.kind, GradientKind::Path { shape: PathShape::Circle, focus: [25.0, 75.0, 75.0, 25.0] });
        assert_eq!(v(r##"filled="f""##, ""), None);
        assert_eq!(v("", r##"<v:fill opacity="32768f"/>"##), Some(Fill::Solid { color: Rgb::WHITE, transparency: 50.0 }));
        // Hostile values don't panic.
        assert!(v("", r##"<v:fill type="gradient" colors="x;;1 #zz;NaN red" focus="NaN" angle="inf" opacity="1e99f"/>"##).is_some());
    }

    #[test]
    fn vml_colours() {
        assert_eq!(vml_color("#ffc000 [3207]"), Some(Rgb(0xFF, 0xC0, 0)));
        assert_eq!(vml_color("ffc000 [3207]"), Some(Rgb(0xFF, 0xC0, 0)));
        assert_eq!(vml_color("#f00"), Some(Rgb(0xFF, 0, 0)));
        assert_eq!(vml_color("f00"), Some(Rgb(0xFF, 0, 0)));
        assert_eq!(vml_color("black"), Some(Rgb(0, 0, 0)));
        assert_eq!(vml_color("Red"), Some(Rgb(0xFF, 0, 0)));
        assert_eq!(vml_color("rgb(255, 128, 0)"), Some(Rgb(255, 128, 0)));
        assert_eq!(vml_color("rgb(100%, 0%, 0%)"), Some(Rgb(255, 0, 0)));
        assert_eq!(vml_color("#é"), None);
        assert_eq!(vml_color(""), None);
    }
}
