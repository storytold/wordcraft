//! WordArt text effects as `w14:` run properties ([MS-DOCX] §2.6.1), in the schema's order:
//! glow, shadow, reflection, text outline, text fill. Lengths are EMUs, angles 60000ths of a
//! degree, percentages 1000ths of a percent.

use wordcraft_doc::props::Rgb;
use wordcraft_doc::wordart::{TextEffects, TextFill};

use crate::units::{emu, n};
use crate::xml::W;

fn pct(v: f32) -> String {
    n((wordcraft_geom::finite(v).clamp(0.0, 100.0) * 1000.0).round() as i64)
}

fn ang(deg: f32) -> String {
    n((wordcraft_geom::finite(deg).rem_euclid(360.0) * 60_000.0).round().clamp(0.0, 21_599_999.0) as i64)
}

fn len(pt: f32) -> String {
    emu(wordcraft_geom::finite(pt).clamp(0.0, 2000.0))
}

/// `w14:srgbClr` with its transparency (`w14:alpha` is the transparency, not the opacity).
fn color(w: &mut W, c: Rgb, transparency: f32) {
    if transparency > 0.0 {
        w.open("w14:srgbClr", &[("w14:val", &c.hex())]);
        w.empty("w14:alpha", &[("w14:val", &pct(transparency))]);
        w.close("w14:srgbClr");
    } else {
        w.empty("w14:srgbClr", &[("w14:val", &c.hex())]);
    }
}

fn fill(w: &mut W, f: &TextFill) {
    match f {
        TextFill::None => w.empty("w14:noFill", &[]),
        TextFill::Solid { color: c, transparency } => {
            w.open("w14:solidFill", &[]);
            color(w, *c, *transparency);
            w.close("w14:solidFill");
        }
        TextFill::Gradient { stops, angle } => {
            w.open("w14:gradFill", &[]);
            w.open("w14:gsLst", &[]);
            for s in stops {
                w.open("w14:gs", &[("w14:pos", &pct(s.pos))]);
                color(w, s.color, s.transparency);
                w.close("w14:gs");
            }
            w.close("w14:gsLst");
            w.empty("w14:lin", &[("w14:ang", &ang(*angle)), ("w14:scaled", "0")]);
            w.close("w14:gradFill");
        }
    }
}

/// The effects of `fx` (sanitized first).
pub fn rpr_effects(w: &mut W, fx: &TextEffects) {
    let fx = fx.sanitized();
    if let Some(g) = fx.glow {
        w.open("w14:glow", &[("w14:rad", &len(g.size))]);
        color(w, g.color, g.transparency);
        w.close("w14:glow");
    }
    if let Some(s) = fx.shadow {
        w.open(
            "w14:shadow",
            &[
                ("w14:blurRad", &len(s.blur)),
                ("w14:dist", &len(s.distance)),
                ("w14:dir", &ang(s.angle)),
                ("w14:sx", "100000"),
                ("w14:sy", "100000"),
                ("w14:kx", "0"),
                ("w14:ky", "0"),
                ("w14:algn", "ctr"),
            ],
        );
        color(w, s.color, s.transparency);
        w.close("w14:shadow");
    }
    if let Some(r) = fx.reflection {
        w.empty(
            "w14:reflection",
            &[
                ("w14:blurRad", &len(r.blur)),
                ("w14:stA", &pct(100.0 - r.transparency)),
                ("w14:stPos", "0"),
                ("w14:endA", "0"),
                ("w14:endPos", &pct(r.size)),
                ("w14:dist", &len(r.distance)),
                ("w14:dir", "5400000"),
                ("w14:fadeDir", "5400000"),
                ("w14:sx", "100000"),
                ("w14:sy", "-100000"),
                ("w14:kx", "0"),
                ("w14:ky", "0"),
                ("w14:algn", "bl"),
            ],
        );
    }
    if let Some(o) = fx.outline {
        w.open("w14:textOutline", &[("w14:w", &len(o.width)), ("w14:cap", "flat"), ("w14:cmpd", "sng"), ("w14:algn", "ctr")]);
        match o.color {
            Some(c) => {
                w.open("w14:solidFill", &[]);
                color(w, c, o.transparency);
                w.close("w14:solidFill");
            }
            None => w.empty("w14:noFill", &[]),
        }
        w.empty("w14:prstDash", &[("w14:val", "solid")]);
        w.empty("w14:round", &[]);
        w.close("w14:textOutline");
    }
    if let Some(f) = &fx.fill {
        w.open("w14:textFill", &[]);
        fill(w, f);
        w.close("w14:textFill");
    }
}
