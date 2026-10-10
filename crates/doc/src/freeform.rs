//! Freeform shapes: the geometry of a shape drawn point by point (DrawingML `a:custGeom`), such as
//! an ink stroke drawn with a pen, pencil or highlighter (Draw tab).

use serde::{Deserialize, Serialize};

/// Most points one path keeps (hostile files and commands): a long pen stroke has a few hundred.
pub const MAX_POINTS: usize = 5_000;
/// Most paths one freeform keeps.
pub const MAX_PATHS: usize = 256;
/// Most ink strokes on one page: drawing more is refused.
pub const MAX_INK_PER_PAGE: usize = 2_000;
/// Largest coordinate kept, in the path's own coordinate space.
const MAX_COORD: f32 = 1.0e6;

/// The pen an ink stroke was drawn with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InkTool {
    #[default]
    Pen,
    Pencil,
    Highlighter,
}

impl InkTool {
    pub fn parse(s: &str) -> Option<InkTool> {
        match s.to_ascii_lowercase().as_str() {
            "pen" => Some(InkTool::Pen),
            "pencil" => Some(InkTool::Pencil),
            "highlighter" => Some(InkTool::Highlighter),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            InkTool::Pen => "pen",
            InkTool::Pencil => "pencil",
            InkTool::Highlighter => "highlighter",
        }
    }
    /// The colour a new stroke gets (black pen, grey pencil, yellow highlighter).
    pub fn default_color(self) -> crate::props::Rgb {
        match self {
            InkTool::Pen => crate::props::Rgb(0, 0, 0),
            InkTool::Pencil => crate::props::Rgb(0x59, 0x59, 0x59),
            InkTool::Highlighter => crate::props::Rgb(0xFF, 0xFF, 0x00),
        }
    }
    /// The width a new stroke gets, points.
    pub fn default_width(self) -> f32 {
        match self {
            InkTool::Pen => 1.5,
            InkTool::Pencil => 1.0,
            InkTool::Highlighter => 12.0,
        }
    }
    /// How opaque its strokes are: a highlighter's let the text show through, a pencil's are a
    /// little lighter than a pen's.
    pub fn alpha(self) -> f32 {
        match self {
            InkTool::Pen => 1.0,
            InkTool::Pencil => 0.85,
            InkTool::Highlighter => 0.5,
        }
    }
}

/// One path of a freeform: points joined by straight lines, closed back to the first or not.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FreePath {
    pub pts: Vec<[f32; 2]>,
    pub closed: bool,
}

/// A freeform shape's geometry: paths in a `w` × `h` coordinate space that is stretched over the
/// shape's frame (DrawingML's `a:path` `w` and `h`). Drawn with the shape's outline colour and
/// width (and its fill, for closed paths that aren't ink).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Freeform {
    pub w: f32,
    pub h: f32,
    pub paths: Vec<FreePath>,
    /// Opacity of the outline, 0–1.
    pub alpha: f32,
    /// Set when this is ink: drawn with a pen (Draw tab), hidden by Review › Hide Ink and removed
    /// by the eraser.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ink: Option<InkTool>,
}

impl Default for Freeform {
    fn default() -> Self {
        Freeform { w: 0.0, h: 0.0, paths: Vec::new(), alpha: 1.0, ink: None }
    }
}

impl Freeform {
    /// An ink stroke through `pts` (shape-local points, already in the `w` × `h` frame).
    pub fn ink(tool: InkTool, w: f32, h: f32, pts: Vec<[f32; 2]>) -> Freeform {
        Freeform { w, h, paths: vec![FreePath { pts, closed: false }], alpha: tool.alpha(), ink: Some(tool) }.sanitized()
    }

    /// The same geometry with non-finite points dropped, coordinates clamped, the counts capped
    /// and the opacity in 0–1: safe to draw whatever file or command it came from.
    pub fn sanitized(mut self) -> Freeform {
        let fin = |v: f32| if v.is_finite() { v.clamp(-MAX_COORD, MAX_COORD) } else { 0.0 };
        self.w = fin(self.w).max(0.0);
        self.h = fin(self.h).max(0.0);
        self.alpha = if self.alpha.is_finite() { self.alpha.clamp(0.0, 1.0) } else { 1.0 };
        self.paths.truncate(MAX_PATHS);
        for p in &mut self.paths {
            p.pts.retain(|[x, y]| x.is_finite() && y.is_finite());
            p.pts.truncate(MAX_POINTS);
            for pt in &mut p.pts {
                *pt = pt.map(fin);
            }
        }
        self.paths.retain(|p| !p.pts.is_empty());
        self
    }

    pub fn is_ink(&self) -> bool {
        self.ink.is_some()
    }

    /// The paths mapped onto a frame at (`x`, `y`) of size `fw` × `fh` (page points). A zero
    /// coordinate space takes the frame's own size.
    pub fn placed(&self, x: f32, y: f32, fw: f32, fh: f32) -> Vec<(Vec<[f32; 2]>, bool)> {
        let sx = if self.w > 0.0 { fw / self.w } else { 1.0 };
        let sy = if self.h > 0.0 { fh / self.h } else { 1.0 };
        let (sx, sy) = (if sx.is_finite() { sx } else { 1.0 }, if sy.is_finite() { sy } else { 1.0 });
        self.paths
            .iter()
            .take(MAX_PATHS)
            .map(|p| (p.pts.iter().take(MAX_POINTS).map(|[px, py]| [x + px * sx, y + py * sy]).collect(), p.closed))
            .collect()
    }

    /// Distance from (`px`, `py`) to the nearest path of this freeform placed on a frame at
    /// (`x`, `y`), `fw` × `fh` (page points); `None` when it has no points.
    pub fn distance(&self, x: f32, y: f32, fw: f32, fh: f32, px: f32, py: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for (pts, closed) in self.placed(x, y, fw, fh) {
            let n = pts.len();
            let segs = if closed && n > 2 { n } else { n.saturating_sub(1) };
            let mut consider = |a: [f32; 2], b: [f32; 2]| {
                let d = seg_distance(a, b, [px, py]);
                if d.is_finite() && best.is_none_or(|bd| d < bd) {
                    best = Some(d);
                }
            };
            if n == 1
                && let Some(a) = pts.first()
            {
                consider(*a, *a);
            }
            for i in 0..segs {
                if let (Some(a), Some(b)) = (pts.get(i), pts.get((i + 1) % n)) {
                    consider(*a, *b);
                }
            }
        }
        best
    }
}

/// Distance from `p` to the segment `a`–`b`.
fn seg_distance(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_geometry_is_sanitized() {
        let mut pts = vec![[f32::NAN, 1.0], [2.0, f32::INFINITY], [1.0, 2.0], [1e30, -1e30]];
        pts.extend(std::iter::repeat_n([3.0, 3.0], MAX_POINTS * 2));
        let f = Freeform { w: f32::NAN, h: -4.0, paths: vec![FreePath { pts, closed: false }], alpha: 7.0, ink: None }.sanitized();
        assert_eq!((f.w, f.h, f.alpha), (0.0, 0.0, 1.0));
        let p = &f.paths[0].pts;
        assert_eq!(p.len(), MAX_POINTS);
        assert_eq!(p[0], [1.0, 2.0]);
        assert_eq!(p[1], [MAX_COORD, -MAX_COORD]);
    }

    #[test]
    fn distance_to_a_stroke_scales_with_its_frame() {
        let f = Freeform::ink(InkTool::Pen, 10.0, 10.0, vec![[0.0, 0.0], [10.0, 0.0]]);
        // Frame twice the size at (100, 100): the stroke runs from (100,100) to (120,100).
        assert_eq!(f.distance(100.0, 100.0, 20.0, 20.0, 110.0, 103.0), Some(3.0));
        assert_eq!(Freeform::default().distance(0.0, 0.0, 1.0, 1.0, 0.0, 0.0), None);
    }
}
