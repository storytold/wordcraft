//! Freeform shapes (DrawingML custom geometry, `a:custGeom`) and ink strokes drawn with them.

use wordcraft_doc::freeform::{FreePath, Freeform, InkTool, MAX_PATHS, MAX_POINTS};

use crate::units::int;
use crate::xml::El;

/// Straight pieces a Bézier curve is flattened into.
const CURVE_STEPS: usize = 8;
/// Largest path coordinate kept, in EMUs.
const MAX_EMU: f64 = 1.0e12;

/// The paths of the custom geometry in `sppr` (`wps:spPr`), mapped onto a `w` × `h` shape
/// (points): each `a:path` in its own `w` × `h` space (EMUs of the shape's size when it gives
/// none). Lines and Bézier curves (flattened) are kept; arcs are drawn straight to their end.
/// `None` when there is no `a:custGeom` or it has no points.
pub fn cust_geom(sppr: &El, w: f32, h: f32) -> Option<Freeform> {
    let list = sppr.child("a:custGeom")?.child("a:pathLst")?;
    let mut paths = Vec::new();
    for p in list.children("a:path").take(MAX_PATHS) {
        let dim = |n: &str, shape: f32| p.attr(n).and_then(int).filter(|v| *v > 0).map_or(shape as f64 * 12_700.0, |v| v as f64);
        let (pw, ph) = (dim("w", w), dim("h", h));
        let scale = |[x, y]: [f64; 2]| -> [f32; 2] {
            let sx = if pw > 0.0 { w as f64 / pw } else { 1.0 / 12_700.0 };
            let sy = if ph > 0.0 { h as f64 / ph } else { 1.0 / 12_700.0 };
            [(x * sx) as f32, (y * sy) as f32]
        };
        let pt = |e: &El| -> Option<[f64; 2]> {
            let c = |n: &str| e.attr(n).and_then(int).map(|v| (v as f64).clamp(-MAX_EMU, MAX_EMU));
            Some([c("x")?, c("y")?])
        };
        // Each subpath (from an `a:moveTo`) becomes a path of its own.
        let mut cur: Vec<[f64; 2]> = Vec::new();
        let mut done: Vec<(Vec<[f64; 2]>, bool)> = Vec::new();
        let fill_none = p.attr("fill") == Some("none");
        for seg in p.els() {
            if cur.len() >= MAX_POINTS || done.len() >= MAX_PATHS {
                break;
            }
            let pts: Vec<[f64; 2]> = seg.children("a:pt").filter_map(pt).collect();
            match seg.local() {
                "moveTo" => {
                    if !cur.is_empty() {
                        done.push((std::mem::take(&mut cur), false));
                    }
                    cur.extend(pts.first());
                }
                "lnTo" => cur.extend(pts.first()),
                "cubicBezTo" | "quadBezTo" => {
                    let Some(&start) = cur.last() else {
                        cur.extend(pts.last());
                        continue;
                    };
                    let ctrl: Vec<[f64; 2]> = std::iter::once(start).chain(pts.iter().copied()).collect();
                    for i in 1..=CURVE_STEPS {
                        cur.push(bezier(&ctrl, i as f64 / CURVE_STEPS as f64));
                    }
                }
                "close" if !cur.is_empty() => done.push((std::mem::take(&mut cur), true)),
                _ => {}
            }
        }
        if !cur.is_empty() {
            done.push((cur, false));
        }
        for (pts, closed) in done {
            paths.push(FreePath { pts: pts.into_iter().take(MAX_POINTS).map(scale).collect(), closed: closed && !fill_none });
        }
    }
    let f = Freeform { w, h, paths, alpha: 1.0, ink: None }.sanitized();
    (!f.paths.is_empty()).then_some(f)
}

/// The pen of an ink stroke written by WordCraft, from its drawing's name (`Ink Pen 3`,
/// `Ink Highlighter 4`).
pub fn ink_tool(name: &str) -> Option<InkTool> {
    let mut words = name.split_whitespace();
    if words.next() != Some("Ink") {
        return None;
    }
    words.next().and_then(InkTool::parse)
}

/// The outline opacity of `ln` (`a:ln`): its colour's `a:alpha`, 0–1.
pub fn line_alpha(ln: Option<&El>) -> f32 {
    ln.and_then(|l| l.child("a:solidFill"))
        .and_then(|f| f.els().next())
        .and_then(|c| c.child("a:alpha"))
        .and_then(|a| a.attr("val"))
        .and_then(int)
        .map_or(1.0, |v| (v.clamp(0, 100_000) as f32) / 100_000.0)
}

/// The point at `t` on the Bézier curve with control points `c` (de Casteljau).
fn bezier(c: &[[f64; 2]], t: f64) -> [f64; 2] {
    let mut p: Vec<[f64; 2]> = c.to_vec();
    while p.len() > 1 {
        p = p
            .windows(2)
            .filter_map(|w| match w {
                [a, b] => Some([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]),
                _ => None,
            })
            .collect();
    }
    p.first().copied().unwrap_or([0.0, 0.0])
}
