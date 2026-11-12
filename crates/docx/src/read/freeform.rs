//! Freeform shapes (DrawingML custom geometry, `a:custGeom`; legacy VML paths) and ink strokes
//! drawn with them.

use wordcraft_doc::freeform::{FreePath, Freeform, InkTool, MAX_PATHS, MAX_POINTS};

use crate::units::int;
use crate::xml::El;

/// Straight pieces a Bézier curve is flattened into.
const CURVE_STEPS: usize = 8;
/// Points read from one path segment. A conformant `a:lnTo`/`a:moveTo` has one, `a:quadBezTo`
/// two and `a:cubicBezTo` three; the cap bounds the de Casteljau control polygon so a segment
/// stuffed with points can't make curve flattening quadratic in the point count.
const MAX_SEG_POINTS: usize = 8;
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
        // Subpaths count too: stop once the shape has as many paths as it may keep.
        if paths.len() >= MAX_PATHS {
            break;
        }
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
            let pts: Vec<[f64; 2]> = seg.children("a:pt").filter_map(pt).take(MAX_SEG_POINTS).collect();
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
            // A path drawn back to where it started is closed, though it has no `a:close` (#743:
            // Word's cover pages), so it's filled.
            let back = cur.len() > 2 && cur.first() == cur.last();
            done.push((cur, back));
        }
        for (pts, closed) in done.into_iter().take(MAX_PATHS.saturating_sub(paths.len())) {
            paths.push(FreePath { pts: pts.into_iter().take(MAX_POINTS).map(scale).collect(), closed: closed && !fill_none });
        }
    }
    let f = Freeform { w, h, paths, alpha: 1.0, ink: None, order: None }.sanitized();
    (!f.paths.is_empty()).then_some(f)
}

/// Largest VML path coordinate kept (coordinate-space units).
const MAX_VML: f64 = 1.0e12;
/// Characters of a VML `path` attribute read; the rest is ignored.
const MAX_VML_PATH: usize = 1 << 20;
/// Parameters of a VML path kept, all commands together.
const MAX_VML_PARAMS: usize = MAX_POINTS * 8;

/// A legacy VML freeform (`v:shape` with a `path`, ECMA-376 Part 4 §14.1.2.19).
pub struct VmlPath {
    pub freeform: Freeform,
    /// `nf` in the path: no fill.
    pub no_fill: bool,
    /// `ns` in the path: no outline.
    pub no_stroke: bool,
}

/// The VML path commands that are two letters long.
const VML_TWO: [&str; 11] = ["nf", "ns", "ae", "al", "at", "ar", "wa", "wr", "qx", "qy", "qb"];

/// The VML `path` of a shape, mapped onto its `w` × `h` frame (points) through its `coordsize`
/// (default `1000,1000`) and `coordorigin` (default `0,0`). Commands: `m`/`t` move (absolute /
/// relative), `l`/`r` line, `c`/`v` cubic Bézier (flattened), `x` close, `e` end, `nf`/`ns` no
/// fill / no outline; arcs (`at`, `ar`, `wa`, `wr`) and quadrants (`qx`, `qy`) are drawn straight
/// to their end, `qb` straight to its last point, and ellipses (`ae`, `al`) are skipped. An empty
/// parameter is 0. `None` when the path has no points, or uses formulas (`@n`, from `v:formulas`,
/// which aren't evaluated): such a shape isn't drawn rather than drawn wrong.
pub fn vml_path(path: &str, coordsize: Option<&str>, coordorigin: Option<&str>, w: f32, h: f32) -> Option<VmlPath> {
    if path.contains('@') {
        return None;
    }
    let pair = |v: Option<&str>| -> Option<(f64, f64)> {
        let v = v?;
        let (a, b) = v.split_once(',').unwrap_or((v, ""));
        let n = |s: &str| int(s).map(|i| (i as f64).clamp(-MAX_VML, MAX_VML));
        Some((n(a)?, n(b).unwrap_or(0.0)))
    };
    let size = |v: f64| if v > 0.0 { v } else { 1000.0 };
    let (cw, ch) = pair(coordsize).map_or((1000.0, 1000.0), |(a, b)| (size(a), size(b)));
    let (ox, oy) = pair(coordorigin).unwrap_or((0.0, 0.0));
    let (sx, sy) = (w as f64 / cw, h as f64 / ch);
    let scale = |[x, y]: [f64; 2]| -> [f32; 2] { [((x - ox) * sx) as f32, ((y - oy) * sy) as f32] };

    let mut d = Draw::default();
    let (mut no_fill, mut no_stroke) = (false, false);
    for (cmd, params) in vml_commands(path) {
        if d.full() {
            break;
        }
        let pts: Vec<[f64; 2]> = params.as_chunks::<2>().0.to_vec();
        match cmd.as_str() {
            "m" | "t" => {
                for (i, &p) in pts.iter().enumerate() {
                    let p = if cmd == "t" { d.rel(p) } else { p };
                    if i == 0 {
                        d.move_to(p);
                    } else {
                        d.line_to(p);
                    }
                }
            }
            "l" | "r" | "qx" | "qy" => {
                for &p in &pts {
                    let p = if cmd == "r" { d.rel(p) } else { p };
                    d.line_to(p);
                }
            }
            "c" | "v" => {
                for c in pts.as_chunks::<3>().0 {
                    let start = d.at;
                    let ctrl: Vec<[f64; 2]> = std::iter::once(start).chain(c.iter().map(|&p| if cmd == "v" { d.rel(p) } else { p })).collect();
                    for i in 1..=CURVE_STEPS {
                        d.line_to(bezier(&ctrl, i as f64 / CURVE_STEPS as f64));
                    }
                }
            }
            "qb" => {
                if let Some(&p) = pts.last() {
                    d.line_to(p);
                }
            }
            // Bounding box, then start and end: `at`/`wa` draw a line to the start, `ar`/`wr` move there.
            "at" | "ar" | "wa" | "wr" => {
                for [_, _, s, e] in pts.as_chunks::<4>().0 {
                    if matches!(cmd.as_str(), "ar" | "wr") {
                        d.move_to(*s);
                    } else {
                        d.line_to(*s);
                    }
                    d.line_to(*e);
                }
            }
            "x" => d.end(true),
            "e" => d.end(false),
            "nf" => no_fill = true,
            "ns" => no_stroke = true,
            _ => {}
        }
    }
    d.end(false);
    let paths = d.done.into_iter().map(|(pts, closed)| FreePath { pts: pts.into_iter().map(scale).collect(), closed }).collect();
    let freeform = Freeform { w, h, paths, alpha: 1.0, ink: None, order: None }.sanitized();
    (!freeform.paths.is_empty()).then_some(VmlPath { freeform, no_fill, no_stroke })
}

/// The pen while a VML path is drawn: where it is, where the subpath began, the subpath being
/// drawn and those done (closed or not).
#[derive(Default)]
struct Draw {
    at: [f64; 2],
    start: [f64; 2],
    cur: Vec<[f64; 2]>,
    done: Vec<(Vec<[f64; 2]>, bool)>,
}

impl Draw {
    fn full(&self) -> bool {
        self.done.len() >= MAX_PATHS
    }

    fn rel(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        [(self.at[0] + x).clamp(-MAX_VML, MAX_VML), (self.at[1] + y).clamp(-MAX_VML, MAX_VML)]
    }

    fn move_to(&mut self, p: [f64; 2]) {
        self.end(false);
        self.at = p;
        self.start = p;
        self.cur.push(p);
    }

    fn line_to(&mut self, p: [f64; 2]) {
        if self.cur.is_empty() {
            self.cur.push(self.at);
            self.start = self.at;
        }
        if self.cur.len() < MAX_POINTS {
            self.cur.push(p);
        }
        self.at = p;
    }

    /// Finish the subpath being drawn, closed back to its start or not.
    fn end(&mut self, closed: bool) {
        if !self.cur.is_empty() && !self.full() {
            self.done.push((std::mem::take(&mut self.cur), closed));
        }
        self.cur.clear();
        if closed {
            self.at = self.start;
        }
    }
}

/// The commands of a VML path with their parameters: a command is a letter (or one of
/// [`VML_TWO`]) followed by numbers separated by commas or spaces. An empty parameter (nothing
/// before a comma) is 0, and so is a formula reference (`@n`, not expected here: see
/// [`vml_path`]); fractions are dropped. Anything else is skipped.
fn vml_commands(path: &str) -> Vec<(String, Vec<f64>)> {
    let mut out: Vec<(String, Vec<f64>)> = Vec::new();
    let mut chars = path.chars().take(MAX_VML_PATH).peekable();
    // Whether a number was read since the command or the last comma.
    let mut has_num = true;
    let mut kept = 0usize;
    let mut push = |out: &mut Vec<(String, Vec<f64>)>, v: f64| {
        if kept < MAX_VML_PARAMS
            && let Some((_, p)) = out.last_mut()
        {
            p.push(v);
            kept += 1;
        }
    };
    while let Some(c) = chars.next() {
        if c.is_ascii_alphabetic() {
            // Cap the commands too, so a path of bare letters can't build a million entries.
            if out.len() >= MAX_VML_PARAMS {
                break;
            }
            if !has_num {
                push(&mut out, 0.0);
            }
            has_num = false;
            let lower = c.to_ascii_lowercase();
            let mut cmd = lower.to_string();
            if let Some(&n) = chars.peek() {
                let two: String = [lower, n.to_ascii_lowercase()].iter().collect();
                if VML_TWO.contains(&two.as_str()) {
                    chars.next();
                    cmd = two;
                }
            }
            out.push((cmd, Vec::new()));
        } else if c == ',' {
            if !has_num {
                push(&mut out, 0.0);
            }
            has_num = false;
        } else if matches!(c, '@' | '-' | '+' | '.') || c.is_ascii_digit() {
            let mut v = c.to_digit(10).map_or(0.0, f64::from);
            let mut frac = c == '.';
            while let Some(&n) = chars.peek() {
                if n == '.' {
                    frac = true;
                } else if n.is_ascii_digit() {
                    if !frac {
                        v = (v * 10.0 + n.to_digit(10).map_or(0.0, f64::from)).min(MAX_VML);
                    }
                } else {
                    break;
                }
                chars.next();
            }
            let v = match c {
                '@' => 0.0,
                '-' => -v,
                _ => v,
            };
            has_num = true;
            push(&mut out, v);
        }
        // Whitespace and anything else only separate numbers.
    }
    if !has_num {
        push(&mut out, 0.0);
    }
    out
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

/// The drawing order of an ink stroke written by WordCraft, from its drawing's name
/// (`Ink Pen 3 #12`: the twelfth stroke drawn), for Ink Replay.
pub fn ink_order(name: &str) -> Option<u32> {
    name.split_whitespace().skip(3).find_map(|w| w.strip_prefix('#')?.parse().ok())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sppr(seg: &str) -> crate::xml::El {
        let xml = format!(
            "<wps:spPr xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
             xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\">\
             <a:custGeom><a:pathLst><a:path w=\"100\" h=\"100\">\
             <a:moveTo><a:pt x=\"0\" y=\"0\"/></a:moveTo>{seg}</a:path></a:pathLst></a:custGeom></wps:spPr>"
        );
        crate::xml::parse(xml.as_bytes()).expect("parse spPr")
    }

    #[test]
    fn a_vml_path_of_bare_letters_builds_a_bounded_command_list() {
        let path = "x".repeat(MAX_VML_PATH);
        let n = vml_commands(&path).len();
        assert!(n <= MAX_VML_PARAMS, "{n} commands");
    }

    #[test]
    fn a_bezier_segment_with_many_points_is_bounded() {
        // One cubicBezTo stuffed with 50 000 points must not make de Casteljau quadratic: the
        // control polygon is capped, so flattening stays a handful of points and returns at once.
        let pts = "<a:pt x=\"1\" y=\"1\"/>".repeat(50_000);
        let sppr = sppr(&format!("<a:cubicBezTo>{pts}</a:cubicBezTo>"));
        let f = cust_geom(&sppr, 100.0, 100.0).expect("a path");
        assert!(f.paths.iter().all(|p| p.pts.len() <= 1 + CURVE_STEPS), "{:?}", f.paths.iter().map(|p| p.pts.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_conformant_cubic_still_flattens_to_a_curve() {
        let sppr = sppr("<a:cubicBezTo><a:pt x=\"0\" y=\"100\"/><a:pt x=\"100\" y=\"100\"/><a:pt x=\"100\" y=\"0\"/></a:cubicBezTo>");
        let f = cust_geom(&sppr, 100.0, 100.0).expect("a path");
        assert_eq!(f.paths.len(), 1);
        assert!(f.paths[0].pts.len() >= 2 && f.paths[0].pts.len() <= 1 + CURVE_STEPS);
    }
}
