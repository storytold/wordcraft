//! Remove Background (Picture Format › Adjust, #3344): finds the subject of a photo and clears
//! what is around it.
//!
//! The method is GrabCut (Rother, Kolmogorov and Blake, "GrabCut: interactive foreground
//! extraction using iterated graph cuts", SIGGRAPH 2004), written here from the paper:
//! - The picture is shrunk to at most [`WORK_PIXELS`] pixels.
//! - A thin band around its edge starts as background and the rest as the subject. Strokes drawn
//!   with Mark Areas to Keep and Mark Areas to Remove, and pixels that are already transparent,
//!   are fixed.
//! - Each side's colours are modelled by a mixture of [`K`] Gaussians. A pixel costs the negative
//!   log-likelihood of its colour under the side it is given, and two neighbours cost less to
//!   separate the more their colours differ (a contrast-sensitive Potts term).
//! - A minimum cut (Boykov and Kolmogorov's maximum flow) labels the pixels, and the models are learned again
//!   from the labels, [`ITERATIONS`] times. The band teaches the background's colours without
//!   pulling its neighbours along, and a last cut lets go of it, so a subject that runs off the
//!   picture's edge (a portrait's shoulders) is kept there too.
//! - Specks are dropped, and the mask is enlarged with its edge following the picture's own edges
//!   through a guided filter (He, Sun and Tang, "Guided image filtering", ECCV 2010).
//!
//! The work is bounded whatever the picture: the cut runs at the working size, marks have a
//! point budget, and the maximum flow has a work budget.

use image::{GrayImage, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::CmdError;

/// The most pixels the graph cut works on.
const WORK_PIXELS: u64 = 180 * 180;
/// The most pixels the mask's edge is refined on before it is enlarged to the picture's size.
const REFINE_PIXELS: u64 = 1024 * 1024;
/// Gaussians per colour model.
const K: usize = 5;
/// Rounds of learning the colour models and cutting, before the last cut.
const ITERATIONS: usize = 4;
/// Weight of the smoothness term (the paper's γ).
const GAMMA: f64 = 50.0;
/// Energies are cut in whole units of 1/SCALE.
const SCALE: f64 = 32.0;
/// The most one pixel's colour can favour a side (nats): a colour neither model has seen can't
/// outweigh every neighbour.
const MAX_DATA: f64 = 200.0;
/// Width of the band that starts as background: this share of the shorter side.
const BAND: f32 = 0.02;
/// How strongly the band pulls its neighbours to the background while the models are learned,
/// as a share of the smoothness term.
const BAND_PULL: f64 = 0.25;
/// How much (nats) a pixel's place favours the subject in its middle and the background at its
/// edges while the models are learned.
const CENTRE_PRIOR: f64 = 1.0;
/// Below this share of the picture kept, without marks, the cut found nothing to tell apart.
const TOO_LITTLE: f64 = 0.01;
/// Added to each colour variance so a flat area has a usable model.
const VARIANCE_FLOOR: f64 = 8.0;
/// Steps the maximum flow may take, over all the cuts, before each cut is taken as it is.
const FLOW_BUDGET: u64 = 200_000_000;

/// The most strokes, and points across them, a removal takes.
pub const MAX_MARKS: usize = 500;
pub const MAX_MARK_POINTS: usize = 4000;
/// A stroke's pen width, as a share of the picture's longer side.
pub const DEFAULT_PEN: f32 = 0.02;
const MIN_PEN: f32 = 0.002;
const MAX_PEN: f32 = 0.05;

/// A stroke drawn with Mark Areas to Keep (`keep`) or Mark Areas to Remove: its points across
/// and down the picture, 0 to 1 of its uncropped width and height, and its pen width as a share
/// of the picture's longer side. A later stroke wins where two cross.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub keep: bool,
    pub points: Vec<[f32; 2]>,
    pub width: f32,
}

/// The `marks` parameter: `[{"keep": bool, "points": [[x, y], …], "width"?: share}]`. Absent or
/// null is no marks.
pub fn marks_from(v: &Value) -> Result<Vec<Mark>, CmdError> {
    let list = match v.get("marks") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(CmdError::Params("`marks` is a list of {\"keep\": bool, \"points\": [[x, y], …], \"width\"?: n}".into())),
    };
    if list.len() > MAX_MARKS {
        return Err(CmdError::Params(format!("at most {MAX_MARKS} marks")));
    }
    let mut total = 0usize;
    let mut out = Vec::with_capacity(list.len());
    for m in list {
        let keep = m
            .get("keep")
            .and_then(Value::as_bool)
            .ok_or_else(|| CmdError::Params("each mark needs `keep`: true (Mark Areas to Keep) or false (Mark Areas to Remove)".into()))?;
        let pts = m
            .get("points")
            .and_then(Value::as_array)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| CmdError::Params("each mark needs `points`: [[x, y], …], 0 to 1 across and down the picture".into()))?;
        total = total.saturating_add(pts.len());
        if total > MAX_MARK_POINTS {
            return Err(CmdError::Params(format!("at most {MAX_MARK_POINTS} mark points in all")));
        }
        let mut points = Vec::with_capacity(pts.len());
        for pt in pts {
            let xy = pt.as_array().filter(|a| a.len() == 2).and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]));
            let Some([x, y]) = xy.filter(|[x, y]| x.is_finite() && y.is_finite()) else {
                return Err(CmdError::Params("a mark point is [x, y], two numbers".into()));
            };
            // Far outside the picture is as good as just outside it.
            points.push([x.clamp(-1.0, 2.0) as f32, y.clamp(-1.0, 2.0) as f32]);
        }
        let width = m.get("width").and_then(Value::as_f64).filter(|w| w.is_finite()).map_or(DEFAULT_PEN, |w| (w as f32).clamp(MIN_PEN, MAX_PEN));
        out.push(Mark { keep, points, width });
    }
    Ok(out)
}

/// What stays of `img`: 255 for its subject, 0 for its background, between them along the edge.
/// The size of `img`.
pub fn foreground(img: &RgbaImage, marks: &[Mark]) -> GrayImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return GrayImage::new(w, h);
    }
    let (ww, wh) = fit(w, h, WORK_PIXELS);
    let small = if (ww, wh) == (w, h) { img.clone() } else { image::imageops::thumbnail(img, ww, wh) };
    let mut work = Work::new(&small, marks);
    let models = work.cut();
    work.tidy();
    work.refine(img, models.as_ref())
}

/// `w` × `h` shrunk, keeping its proportions, to at most `max` pixels (never enlarged).
fn fit(w: u32, h: u32, max: u64) -> (u32, u32) {
    let n = u64::from(w) * u64::from(h);
    if n <= max {
        return (w, h);
    }
    let k = (max as f64 / n as f64).sqrt();
    let mut a = ((f64::from(w) * k).round() as u32).clamp(1, w);
    let mut b = ((f64::from(h) * k).round() as u32).clamp(1, h);
    // A very long, thin picture: its short side can't shrink below 1.
    let max = u32::try_from(max).unwrap_or(u32::MAX).max(1);
    b = b.min(max / a.max(1)).max(1);
    a = a.min(max / b).max(1);
    (a, b)
}

/// A pixel's side. The fixed ones are never changed by a cut.
const BG: u8 = 0;
const FG: u8 = 1;
const MAYBE_BG: u8 = 2;
const MAYBE_FG: u8 = 3;

fn is_fg(l: u8) -> bool {
    l == FG || l == MAYBE_FG
}

/// The picture at its working size and the side each pixel is on.
struct Work {
    w: usize,
    h: usize,
    rgb: Vec<[f32; 3]>,
    label: Vec<u8>,
    /// Background only because it is in the band around the edge (let go for the last cut).
    band: Vec<bool>,
    /// Whether any marks were drawn.
    marked: bool,
}

impl Work {
    fn new(img: &RgbaImage, marks: &[Mark]) -> Work {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let b = ((w.min(h) as f32 * BAND).round() as usize).max(1);
        let mut rgb = Vec::with_capacity(w * h);
        let mut label = Vec::with_capacity(w * h);
        let mut band = Vec::with_capacity(w * h);
        for (x, y, p) in img.enumerate_pixels() {
            let [r, g, bl, a] = p.0;
            rgb.push([f32::from(r), f32::from(g), f32::from(bl)]);
            let (x, y) = (x as usize, y as usize);
            let edge = x < b || y < b || x + b >= w || y + b >= h;
            label.push(if a < 128 || edge { BG } else { MAYBE_FG });
            band.push(a >= 128 && edge);
        }
        let mut work = Work { w, h, rgb, label, band, marked: !marks.is_empty() };
        let opaque: Vec<bool> = img.pixels().map(|p| p.0[3] >= 128).collect();
        for m in marks {
            let side = if m.keep { FG } else { BG };
            stamp(m, w, h, |i| {
                // A mark doesn't bring back what is already transparent.
                if opaque.get(i).copied().unwrap_or(false)
                    && let (Some(l), Some(b)) = (work.label.get_mut(i), work.band.get_mut(i))
                {
                    *l = side;
                    *b = false;
                }
            });
        }
        work
    }

    /// GrabCut: learn, cut, again; then the last cut without the band. Returns the colour models
    /// (subject, background) when both sides had pixels to learn from.
    fn cut(&mut self) -> Option<(Gmm, Gmm)> {
        let n = self.label.len();
        let (fg_seen, bg_seen) = (self.label.iter().any(|&l| is_fg(l)), self.label.iter().any(|&l| !is_fg(l)));
        if !fg_seen || !bg_seen {
            // Too small to have both a band and a middle, or all marked: keep what isn't removed.
            for (l, b) in self.label.iter_mut().zip(&self.band) {
                if *b || *l == MAYBE_BG {
                    *l = MAYBE_FG;
                }
            }
            return None;
        }
        let pairs = Pairs::new(self);
        let (mut fg, mut bg): (Option<Gmm>, Option<Gmm>) = (None, None);
        let mut budget = FLOW_BUDGET;
        for round in 0..=ITERATIONS {
            let release = round == ITERATIONS;
            if release {
                for (l, b) in self.label.iter_mut().zip(&self.band) {
                    if *b && *l == BG {
                        *l = MAYBE_BG;
                    }
                }
            }
            let f = Gmm::learn(&self.rgb, &self.label, true, fg.as_ref());
            let g = Gmm::learn(&self.rgb, &self.label, false, bg.as_ref());
            let (Some(f), Some(g)) = (f, g) else { break };
            let mut graph = Graph::new(n, pairs.count);
            let hard = ((GAMMA * 8.0 + MAX_DATA) * SCALE).round() as i64 + 1;
            for (i, (&l, z)) in self.label.iter().zip(&self.rgb).enumerate() {
                match l {
                    FG => graph.terminal(i, hard),
                    BG => graph.terminal(i, -hard),
                    _ => {
                        let prior = if release { 0.0 } else { self.centre_prior(i) };
                        let d = (g.nll(*z) - f.nll(*z) + prior).clamp(-MAX_DATA, MAX_DATA);
                        graph.terminal(i, (d * SCALE).round() as i64);
                    }
                }
            }
            // The band is only a guess at the background: it teaches the background's colours but
            // pulls its neighbours to its side only weakly, so a subject running into it isn't
            // dragged out with it; a mild pull to the middle (`centre_prior`) peels the background
            // in from the edges instead. The last cut decides the band itself, with all its links
            // and without the prior.
            let band = &self.band;
            let in_band = |i: u32| band.get(i as usize).copied().unwrap_or(false);
            pairs.add_to(&mut graph, |a, b| if release || !(in_band(a) || in_band(b)) { 1.0 } else { BAND_PULL });
            let source_side = graph.min_cut(&mut budget);
            for (l, s) in self.label.iter_mut().zip(source_side) {
                if *l == MAYBE_FG || *l == MAYBE_BG {
                    *l = if s { MAYBE_FG } else { MAYBE_BG };
                }
            }
            fg = Some(f);
            bg = Some(g);
        }
        let opaque = self.label.iter().zip(&self.band).filter(|(l, b)| **b || **l != BG).count();
        let kept = self.label.iter().filter(|&&l| is_fg(l)).count();
        if !self.marked && (kept as f64) < TOO_LITTLE * opaque as f64 {
            // Nothing told the subject from its background (a picture filled edge to edge):
            // keep it all, rather than clear the whole picture, and let marks say what goes.
            for l in &mut self.label {
                if *l == MAYBE_BG {
                    *l = MAYBE_FG;
                }
            }
            return None;
        }
        fg.zip(bg)
    }

    /// The pull of pixel `i`'s place: up to [`CENTRE_PRIOR`] to the subject in the middle (along
    /// the shorter side), as much to the background at the edge.
    fn centre_prior(&self, i: usize) -> f64 {
        let (x, y) = ((i % self.w.max(1)) as f64 + 0.5, (i / self.w.max(1)) as f64 + 0.5);
        let half = 0.5 * self.w.min(self.h).max(1) as f64;
        let c = (x.min(self.w as f64 - x).min(y).min(self.h as f64 - y) / half).clamp(0.0, 1.0);
        CENTRE_PRIOR * (2.0 * c - 1.0)
    }

    /// Drops specks of subject and fills pinholes in it (unless a mark asked for them).
    fn tidy(&mut self) {
        let n = self.label.len();
        let speck = (n / 500).max(4);
        let pinhole = (n / 2000).max(2);
        let (w, h) = (self.w, self.h);
        let mut seen = vec![false; n];
        let mut stack = Vec::new();
        let mut members = Vec::new();
        for start in 0..n {
            if seen.get(start).copied().unwrap_or(true) {
                continue;
            }
            let fg = self.label.get(start).is_some_and(|&l| is_fg(l));
            // One region of like pixels, 4-connected.
            members.clear();
            stack.clear();
            stack.push(start);
            if let Some(s) = seen.get_mut(start) {
                *s = true;
            }
            let (mut fixed, mut edge) = (false, false);
            while let Some(i) = stack.pop() {
                members.push(i);
                fixed |= self.label.get(i).is_some_and(|&l| l == FG || l == BG) && !self.band.get(i).copied().unwrap_or(false);
                let (x, y) = (i % w, i / w);
                edge |= x == 0 || y == 0 || x + 1 == w || y + 1 == h;
                let around = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
                for j in around.into_iter().flatten() {
                    let like = self.label.get(j).is_some_and(|&l| is_fg(l) == fg);
                    if like && let Some(s) = seen.get_mut(j).filter(|s| !**s) {
                        *s = true;
                        stack.push(j);
                    }
                }
            }
            let flip = if fg { members.len() < speck && !fixed } else { members.len() <= pinhole && !fixed && !edge };
            if flip {
                for &i in &members {
                    if let Some(l) = self.label.get_mut(i) {
                        *l = if fg { MAYBE_BG } else { MAYBE_FG };
                    }
                }
            }
        }
    }

    /// The mask at `img`'s size: the working labels enlarged to at most [`REFINE_PIXELS`], the
    /// uncertain edge decided again by the colour models, smoothed along the picture's own edges
    /// with a guided filter, then enlarged to the picture.
    fn refine(&self, img: &RgbaImage, models: Option<&(Gmm, Gmm)>) -> GrayImage {
        let (w, h) = img.dimensions();
        let (rw, rh) = fit(w, h, REFINE_PIXELS);
        let guide = if (rw, rh) == (w, h) { None } else { Some(image::imageops::thumbnail(img, rw, rh)) };
        let guide = guide.as_ref().unwrap_or(img);
        let (rw, rh) = (rw as usize, rh as usize);
        let gray: Vec<f32> =
            guide.pixels().map(|p| (0.299 * f32::from(p.0[0]) + 0.587 * f32::from(p.0[1]) + 0.114 * f32::from(p.0[2])) / 255.0).collect();
        let (sx, sy) = (self.w as f32 / rw as f32, self.h as f32 / rh as f32);
        let at = |x: usize, y: usize| y.saturating_mul(self.w).saturating_add(x);
        let fg_at = |x: usize, y: usize| if self.label.get(at(x, y)).is_some_and(|&l| is_fg(l)) { 1.0f32 } else { 0.0 };
        // Whether the working pixel under (x, y) is fixed by a mark or by being see-through.
        let fixed_at = |x: usize, y: usize| {
            let (wx, wy) = (((x as f32 + 0.5) * sx) as usize, ((y as f32 + 0.5) * sy) as usize);
            let i = at(wx.min(self.w - 1), wy.min(self.h - 1));
            matches!((self.label.get(i).copied(), self.band.get(i).copied()), (Some(FG), _) | (Some(BG), Some(false)))
        };
        let mut p = Vec::with_capacity(rw * rh);
        for (i, px) in guide.pixels().enumerate() {
            let (x, y) = (i % rw, i / rw);
            // Bilinear, between the centres of the working pixels.
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (self.w - 1) as f32);
            let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (self.h - 1) as f32);
            let (x0, y0) = (fx as usize, fy as usize);
            let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let top = fg_at(x0, y0) * (1.0 - tx) + fg_at(x1, y0) * tx;
            let bottom = fg_at(x0, y1) * (1.0 - tx) + fg_at(x1, y1) * tx;
            let mut v = top * (1.0 - ty) + bottom * ty;
            if v > 0.0 && v < 1.0 && !fixed_at(x, y) {
                if let Some((f, g)) = models {
                    let z = [f32::from(px.0[0]), f32::from(px.0[1]), f32::from(px.0[2])];
                    let d = (g.nll(z) - f.nll(z)).clamp(-30.0, 30.0) as f32;
                    v = 0.5 * v + 0.5 / (1.0 + (-d).exp());
                }
            }
            p.push(v);
        }
        let r = ((rw as f32 / self.w as f32).round() as usize).max(1);
        let q = guided_filter(&gray, &p, rw, rh, r, 2e-3);
        let mut out = GrayImage::new(rw as u32, rh as u32);
        // Marked and see-through pixels are already 0 or 1 in `p` (not decided again above), so
        // their edges come out of the filter smooth rather than in steps of the working size.
        for (o, v) in out.pixels_mut().zip(q) {
            o.0[0] = (((v - 0.35) / 0.3).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        if (rw as u32, rh as u32) == (w, h) { out } else { image::imageops::resize(&out, w, h, image::imageops::FilterType::Triangle) }
    }
}

/// Sets each pixel of a `w` × `h` grid under `mark`'s stroke.
fn stamp(mark: &Mark, w: usize, h: usize, mut set: impl FnMut(usize)) {
    let (fw, fh) = (w as f32, h as f32);
    let r = (mark.width * fw.max(fh) / 2.0).max(0.75);
    let pts: Vec<(f32, f32)> = mark.points.iter().map(|p| (p[0] * fw, p[1] * fh)).collect();
    let segments: Vec<((f32, f32), (f32, f32))> =
        if pts.len() == 1 { pts.iter().map(|&p| (p, p)).collect() } else { pts.windows(2).filter_map(|s| Some((*s.first()?, *s.get(1)?))).collect() };
    for (a, b) in segments {
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        // Short pieces, so each one's box is mostly stroke.
        let pieces = (len / r.max(1.0)).ceil().max(1.0) as usize;
        for k in 0..pieces {
            let t0 = k as f32 / pieces as f32;
            let t1 = (k + 1) as f32 / pieces as f32;
            let s0 = (a.0 + (b.0 - a.0) * t0, a.1 + (b.1 - a.1) * t0);
            let s1 = (a.0 + (b.0 - a.0) * t1, a.1 + (b.1 - a.1) * t1);
            let x0 = (s0.0.min(s1.0) - r).floor().max(0.0) as usize;
            let y0 = (s0.1.min(s1.1) - r).floor().max(0.0) as usize;
            let x1 = (s0.0.max(s1.0) + r).ceil().min(fw - 1.0);
            let y1 = (s0.1.max(s1.1) + r).ceil().min(fh - 1.0);
            if x1 < 0.0 || y1 < 0.0 {
                continue;
            }
            for y in y0..=(y1 as usize) {
                for x in x0..=(x1 as usize) {
                    if segment_distance((x as f32 + 0.5, y as f32 + 0.5), s0, s1) <= r {
                        set(y * w + x);
                    }
                }
            }
        }
    }
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// A Gaussian of a colour model: its mean, inverse covariance and log of its weight over its
/// normalising constant.
#[derive(Clone, Copy, Debug)]
struct Gauss {
    mean: [f64; 3],
    inv: [[f64; 3]; 3],
    ln_c: f64,
}

impl Gauss {
    /// Log of the weighted density at `z`.
    fn ln(&self, z: [f32; 3]) -> f64 {
        let d = [f64::from(z[0]) - self.mean[0], f64::from(z[1]) - self.mean[1], f64::from(z[2]) - self.mean[2]];
        let mut m = 0.0;
        for (row, di) in self.inv.iter().zip(d) {
            m += di * (row[0] * d[0] + row[1] * d[1] + row[2] * d[2]);
        }
        self.ln_c - 0.5 * m
    }
}

/// A Gaussian mixture over colours.
#[derive(Clone, Debug)]
struct Gmm(Vec<Gauss>);

/// Sums for a Gaussian's mean and covariance.
#[derive(Clone, Copy, Default)]
struct Sums {
    n: f64,
    s: [f64; 3],
    ss: [[f64; 3]; 3],
}

impl Sums {
    fn add(&mut self, z: [f32; 3]) {
        let z = z.map(f64::from);
        self.n += 1.0;
        for (a, row) in self.ss.iter_mut().enumerate() {
            self.s[a] += z[a];
            for (b, v) in row.iter_mut().enumerate() {
                *v += z[a] * z[b];
            }
        }
    }

    fn gauss(&self, total: f64) -> Option<Gauss> {
        if self.n < 1.0 || total <= 0.0 {
            return None;
        }
        let mean = self.s.map(|s| s / self.n);
        let mut c = [[0.0; 3]; 3];
        for (a, row) in c.iter_mut().enumerate() {
            for (b, v) in row.iter_mut().enumerate() {
                *v = self.ss[a][b] / self.n - mean[a] * mean[b];
            }
            row[a] += VARIANCE_FLOOR;
        }
        let det = c[0][0] * (c[1][1] * c[2][2] - c[1][2] * c[2][1]) - c[0][1] * (c[1][0] * c[2][2] - c[1][2] * c[2][0])
            + c[0][2] * (c[1][0] * c[2][1] - c[1][1] * c[2][0]);
        if !(det.is_finite() && det > 1e-9) {
            return None;
        }
        let inv = [
            [
                (c[1][1] * c[2][2] - c[1][2] * c[2][1]) / det,
                (c[0][2] * c[2][1] - c[0][1] * c[2][2]) / det,
                (c[0][1] * c[1][2] - c[0][2] * c[1][1]) / det,
            ],
            [
                (c[1][2] * c[2][0] - c[1][0] * c[2][2]) / det,
                (c[0][0] * c[2][2] - c[0][2] * c[2][0]) / det,
                (c[0][2] * c[1][0] - c[0][0] * c[1][2]) / det,
            ],
            [
                (c[1][0] * c[2][1] - c[1][1] * c[2][0]) / det,
                (c[0][1] * c[2][0] - c[0][0] * c[2][1]) / det,
                (c[0][0] * c[1][1] - c[0][1] * c[1][0]) / det,
            ],
        ];
        let ln_c = (self.n / total).ln() - 0.5 * det.ln() - 1.5 * (2.0 * std::f64::consts::PI).ln();
        Some(Gauss { mean, inv, ln_c })
    }
}

impl Gmm {
    /// The model of one side's pixels (`fg`: the subject's): each pixel goes to its likeliest
    /// Gaussian of `prev`, or the first time to its nearest k-means cluster.
    fn learn(rgb: &[[f32; 3]], label: &[u8], fg: bool, prev: Option<&Gmm>) -> Option<Gmm> {
        let side = || rgb.iter().zip(label).filter(move |(_, l)| is_fg(**l) == fg).map(|(z, _)| *z);
        let mut sums = [Sums::default(); K];
        match prev {
            Some(g) => side().for_each(|z| {
                if let Some(s) = sums.get_mut(g.likeliest(z)) {
                    s.add(z);
                }
            }),
            None => {
                let centres = kmeans(side());
                side().for_each(|z| {
                    if let Some(s) = sums.get_mut(nearest(&centres, z)) {
                        s.add(z);
                    }
                });
            }
        }
        let total: f64 = sums.iter().map(|s| s.n).sum();
        let g: Vec<Gauss> = sums.iter().filter_map(|s| s.gauss(total)).collect();
        (!g.is_empty()).then_some(Gmm(g))
    }

    fn likeliest(&self, z: [f32; 3]) -> usize {
        let mut best = (0, f64::NEG_INFINITY);
        for (i, g) in self.0.iter().enumerate() {
            let v = g.ln(z);
            if v > best.1 {
                best = (i, v);
            }
        }
        best.0
    }

    /// Negative log-likelihood of colour `z`.
    fn nll(&self, z: [f32; 3]) -> f64 {
        let lns: Vec<f64> = self.0.iter().map(|g| g.ln(z)).collect();
        let top = lns.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if !top.is_finite() {
            return MAX_DATA;
        }
        -(top + lns.iter().map(|v| (v - top).exp()).sum::<f64>().ln())
    }
}

/// [`K`] cluster centres of `colours` (Lloyd's k-means on at most 4096 of them, starting from
/// brightness quantiles, so the result is always the same).
fn kmeans(colours: impl Iterator<Item = [f32; 3]>) -> Vec<[f32; 3]> {
    let all: Vec<[f32; 3]> = colours.collect();
    let step = (all.len() / 4096).max(1);
    let mut sample: Vec<[f32; 3]> = all.into_iter().step_by(step).collect();
    if sample.is_empty() {
        return Vec::new();
    }
    let luma = |z: &[f32; 3]| 0.299 * z[0] + 0.587 * z[1] + 0.114 * z[2];
    sample.sort_by(|a, b| luma(a).total_cmp(&luma(b)));
    let k = K.min(sample.len());
    let mut centres: Vec<[f32; 3]> = sample.chunks(sample.len().div_ceil(k)).map(mean_of).collect();
    for _ in 0..8 {
        let mut sums = vec![([0.0f64; 3], 0u32); centres.len()];
        for z in &sample {
            if let Some((s, n)) = sums.get_mut(nearest(&centres, *z)) {
                for (a, v) in s.iter_mut().zip(z) {
                    *a += f64::from(*v);
                }
                *n += 1;
            }
        }
        for (c, (s, n)) in centres.iter_mut().zip(sums) {
            if n > 0 {
                *c = s.map(|v| (v / f64::from(n)) as f32);
            }
        }
    }
    centres
}

fn mean_of(zs: &[[f32; 3]]) -> [f32; 3] {
    let n = zs.len().max(1) as f32;
    let mut m = [0.0f32; 3];
    for z in zs {
        for (a, v) in m.iter_mut().zip(z) {
            *a += v / n;
        }
    }
    m
}

fn nearest(centres: &[[f32; 3]], z: [f32; 3]) -> usize {
    let d = |c: &[f32; 3]| (c[0] - z[0]).powi(2) + (c[1] - z[1]).powi(2) + (c[2] - z[2]).powi(2);
    centres.iter().enumerate().min_by(|a, b| d(a.1).total_cmp(&d(b.1))).map_or(0, |(i, _)| i)
}

/// The smoothness term: for each pixel, its neighbour to the right, below, below right and below
/// left, and what separating them costs.
struct Pairs {
    pairs: Vec<(u32, u32, i64)>,
    count: usize,
}

impl Pairs {
    fn new(work: &Work) -> Pairs {
        let (w, h) = (work.w, work.h);
        let at = |x: usize, y: usize| work.rgb.get(y * w + x).copied().unwrap_or_default();
        let d2 = |a: [f32; 3], b: [f32; 3]| f64::from((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2));
        let mut raw = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let z = at(x, y);
                let i = (y * w + x) as u32;
                let mut push = |nx: usize, ny: usize, dist: f64| raw.push((i, (ny * w + nx) as u32, d2(z, at(nx, ny)), dist));
                if x + 1 < w {
                    push(x + 1, y, 1.0);
                }
                if y + 1 < h {
                    push(x, y + 1, 1.0);
                    if x + 1 < w {
                        push(x + 1, y + 1, std::f64::consts::SQRT_2);
                    }
                    if x > 0 {
                        push(x - 1, y + 1, std::f64::consts::SQRT_2);
                    }
                }
            }
        }
        let mean = if raw.is_empty() { 0.0 } else { raw.iter().map(|r| r.2).sum::<f64>() / raw.len() as f64 };
        let beta = if mean > 0.0 { 1.0 / (2.0 * mean) } else { 0.0 };
        let pairs: Vec<(u32, u32, i64)> =
            raw.into_iter().map(|(a, b, d2, dist)| (a, b, (GAMMA * (-beta * d2).exp() / dist * SCALE).round() as i64)).filter(|p| p.2 > 0).collect();
        let count = pairs.len();
        Pairs { pairs, count }
    }

    /// Links each pair `keep` takes.
    fn add_to(&self, g: &mut Graph, keep: impl Fn(u32, u32) -> f64) {
        for &(a, b, c) in &self.pairs {
            let k = keep(a, b);
            let c = (c as f64 * k).round() as i64;
            if c > 0 {
                g.edge(a as usize, b as usize, c, c);
            }
        }
    }
}

/// A graph of pixels plus a source (the subject) and a sink (the background), for one minimum
/// cut: each pixel's pull to a terminal, and the links between pixels.
struct Graph {
    /// Positive: the pixel's link from the source; negative: its link to the sink.
    tr: Vec<i64>,
    edges: Vec<(u32, u32, i64, i64)>,
}

/// A tree node's parent link: none (free or orphaned), or straight to its terminal.
const NO_PARENT: u32 = u32::MAX;
const TERMINAL: u32 = u32::MAX - 1;
/// Which search tree a node is in.
const FREE: u8 = 0;
const S_TREE: u8 = 1;
const T_TREE: u8 = 2;

impl Graph {
    fn new(pixels: usize, pairs: usize) -> Graph {
        Graph { tr: vec![0; pixels], edges: Vec::with_capacity(pairs) }
    }

    /// Pixel `i`'s pull to a side: positive to the subject (what putting it in the background
    /// costs), negative to the background.
    fn terminal(&mut self, i: usize, pull: i64) {
        if let Some(t) = self.tr.get_mut(i) {
            *t = t.saturating_add(pull);
        }
    }

    fn edge(&mut self, a: usize, b: usize, ab: i64, ba: i64) {
        if a != b && a < self.tr.len() && b < self.tr.len() {
            self.edges.push((a as u32, b as u32, ab, ba));
        }
    }

    /// The minimum cut: whether each pixel ends on the source's side. This is Boykov and
    /// Kolmogorov's maximum flow ("An experimental comparison of min-cut/max-flow algorithms for
    /// energy minimization in vision", 2004): a search tree grows from each terminal until they
    /// touch, the path where they touch is filled, and the nodes it cut off are re-attached or
    /// freed. If `budget` runs out, the source's tree so far is taken.
    fn min_cut(self, budget: &mut u64) -> Vec<bool> {
        let mut f = Flow::new(self);
        f.run(budget);
        f.tree.iter().map(|&t| t == S_TREE).collect()
    }
}

/// The state of [`Graph::min_cut`]. A node's parent link is the arc from it to its parent.
struct Flow {
    tr: Vec<i64>,
    /// Compressed rows: node u's arcs are head[u]..head[u + 1].
    head: Vec<u32>,
    to: Vec<u32>,
    rev: Vec<u32>,
    cap: Vec<i64>,
    tree: Vec<u8>,
    parent: Vec<u32>,
    /// When a node's distance to its terminal was last known to be right, and that distance.
    stamp: Vec<u64>,
    dist: Vec<u32>,
    active: std::collections::VecDeque<u32>,
    queued: Vec<bool>,
    orphans: std::collections::VecDeque<u32>,
    time: u64,
}

impl Flow {
    fn new(g: Graph) -> Flow {
        let n = g.tr.len();
        let mut head = vec![0u32; n + 1];
        for &(a, b, ..) in &g.edges {
            for u in [a, b] {
                if let Some(h) = head.get_mut(u as usize + 1) {
                    *h += 1;
                }
            }
        }
        for u in 0..n {
            let prev = head.get(u).copied().unwrap_or(0);
            if let Some(h) = head.get_mut(u + 1) {
                *h += prev;
            }
        }
        let arcs = head.get(n).copied().unwrap_or(0) as usize;
        let (mut to, mut cap, mut rev) = (vec![0u32; arcs], vec![0i64; arcs], vec![0u32; arcs]);
        let mut next: Vec<u32> = head.clone();
        for (a, b, ab, ba) in g.edges {
            let (Some(&i), Some(&j)) = (next.get(a as usize), next.get(b as usize)) else { continue };
            for u in [a, b] {
                if let Some(x) = next.get_mut(u as usize) {
                    *x += 1;
                }
            }
            let (i, j) = (i as usize, j as usize);
            if let (Some(t), Some(c), Some(r)) = (to.get_mut(i), cap.get_mut(i), rev.get_mut(i)) {
                (*t, *c, *r) = (b, ab, j as u32);
            }
            if let (Some(t), Some(c), Some(r)) = (to.get_mut(j), cap.get_mut(j), rev.get_mut(j)) {
                (*t, *c, *r) = (a, ba, i as u32);
            }
        }
        let mut f = Flow {
            tree: vec![FREE; n],
            parent: vec![NO_PARENT; n],
            stamp: vec![0; n],
            dist: vec![0; n],
            active: std::collections::VecDeque::new(),
            queued: vec![false; n],
            orphans: std::collections::VecDeque::new(),
            time: 0,
            tr: g.tr,
            head,
            to,
            rev,
            cap,
        };
        for u in 0..n {
            let t = f.tr.get(u).copied().unwrap_or(0);
            if t != 0 {
                f.set(u, if t > 0 { S_TREE } else { T_TREE }, TERMINAL, 0, 1);
                f.activate(u);
            }
        }
        f
    }

    fn arcs(&self, u: usize) -> std::ops::Range<usize> {
        self.head.get(u).copied().unwrap_or(0) as usize..self.head.get(u + 1).copied().unwrap_or(0) as usize
    }

    fn cap(&self, a: usize) -> i64 {
        self.cap.get(a).copied().unwrap_or(0)
    }

    fn to(&self, a: usize) -> usize {
        self.to.get(a).copied().unwrap_or(0) as usize
    }

    fn rev(&self, a: usize) -> usize {
        self.rev.get(a).copied().unwrap_or(0) as usize
    }

    fn tree(&self, u: usize) -> u8 {
        self.tree.get(u).copied().unwrap_or(FREE)
    }

    fn parent(&self, u: usize) -> u32 {
        self.parent.get(u).copied().unwrap_or(NO_PARENT)
    }

    fn push(&mut self, a: usize, f: i64) {
        let r = self.rev(a);
        if let Some(c) = self.cap.get_mut(a) {
            *c -= f;
        }
        if let Some(c) = self.cap.get_mut(r) {
            *c += f;
        }
    }

    fn set(&mut self, u: usize, tree: u8, parent: u32, stamp: u64, dist: u32) {
        if let (Some(t), Some(p), Some(s), Some(d)) = (self.tree.get_mut(u), self.parent.get_mut(u), self.stamp.get_mut(u), self.dist.get_mut(u)) {
            (*t, *p, *s, *d) = (tree, parent, stamp, dist);
        }
    }

    fn activate(&mut self, u: usize) {
        if let Some(q) = self.queued.get_mut(u).filter(|q| !**q) {
            *q = true;
            self.active.push_back(u as u32);
        }
    }

    fn orphan(&mut self, u: usize) {
        if let Some(p) = self.parent.get_mut(u) {
            *p = NO_PARENT;
        }
        self.orphans.push_front(u as u32);
    }

    fn run(&mut self, budget: &mut u64) {
        let mut current: Option<usize> = None;
        loop {
            // The next active node: the one that just found a path, while it's still in a tree.
            let p = match current.take().filter(|&c| self.tree(c) != FREE) {
                Some(c) => c,
                None => loop {
                    let Some(u) = self.active.pop_front() else { return };
                    let u = u as usize;
                    if let Some(q) = self.queued.get_mut(u) {
                        *q = false;
                    }
                    if self.tree(u) != FREE {
                        break u;
                    }
                },
            };
            // Growth: claim free neighbours; a neighbour in the other tree is a path.
            let side = self.tree(p);
            let mut meet = None;
            for a in self.arcs(p) {
                *budget = budget.saturating_sub(1);
                let q = self.to(a);
                // The arc the flow would take: out of p in the source's tree, into it in the sink's.
                let room = if side == S_TREE { self.cap(a) } else { self.cap(self.rev(a)) };
                if room <= 0 {
                    continue;
                }
                match self.tree(q) {
                    FREE => {
                        let (stamp, dist) = (self.stamp.get(p).copied().unwrap_or(0), self.dist.get(p).copied().unwrap_or(0));
                        self.set(q, side, self.rev(a) as u32, stamp, dist.saturating_add(1));
                        self.activate(q);
                    }
                    t if t != side => {
                        // The arc from the source's side to the sink's.
                        meet = Some(if side == S_TREE { a } else { self.rev(a) });
                        break;
                    }
                    _ => {}
                }
            }
            if *budget == 0 {
                return;
            }
            if let Some(a) = meet {
                current = Some(p);
                self.time += 1;
                self.augment(a);
                self.adopt(budget);
            }
        }
    }

    /// Fills the path through arc `a` (from the source's tree to the sink's) and orphans the
    /// nodes whose link to their parent it filled.
    fn augment(&mut self, a: usize) {
        let (x, y) = (self.to(self.rev(a)), self.to(a));
        let mut f = self.cap(a);
        // The bottleneck: up the source's tree from x, up the sink's tree from y.
        for (start, source) in [(x, true), (y, false)] {
            let mut u = start;
            for _ in 0..=self.tr.len() {
                let pa = self.parent(u);
                if pa == TERMINAL {
                    let t = self.tr.get(u).copied().unwrap_or(0);
                    f = f.min(if source { t } else { -t });
                    break;
                }
                if pa == NO_PARENT {
                    return;
                }
                let pa = pa as usize;
                f = f.min(if source { self.cap(self.rev(pa)) } else { self.cap(pa) });
                u = self.to(pa);
            }
        }
        if f <= 0 {
            return;
        }
        self.push(a, f);
        for (start, source) in [(x, true), (y, false)] {
            let mut u = start;
            for _ in 0..=self.tr.len() {
                let pa = self.parent(u);
                if pa == TERMINAL {
                    if let Some(t) = self.tr.get_mut(u) {
                        *t -= if source { f } else { -f };
                        if *t == 0 {
                            self.orphan(u);
                        }
                    }
                    break;
                }
                if pa == NO_PARENT {
                    break;
                }
                let pa = pa as usize;
                // Flow goes down the source's tree (parent to u) and up the sink's (u to parent).
                let arc = if source { self.rev(pa) } else { pa };
                self.push(arc, f);
                let next = self.to(pa);
                if self.cap(arc) == 0 {
                    self.orphan(u);
                }
                u = next;
            }
        }
    }

    /// Re-attaches each orphan to a neighbour in its tree still linked to the terminal, nearest
    /// first, or frees it (orphaning its children, waking its neighbours).
    fn adopt(&mut self, budget: &mut u64) {
        while let Some(p) = self.orphans.pop_front() {
            let p = p as usize;
            let side = self.tree(p);
            let t = self.tr.get(p).copied().unwrap_or(0);
            if (side == S_TREE && t > 0) || (side == T_TREE && t < 0) {
                self.set(p, side, TERMINAL, self.time, 1);
                continue;
            }
            let mut best: Option<(usize, u32)> = None;
            for a in self.arcs(p) {
                *budget = budget.saturating_sub(1);
                let q = self.to(a);
                // Flow from q into p in the source's tree, from p into q in the sink's.
                let room = if side == S_TREE { self.cap(self.rev(a)) } else { self.cap(a) };
                if room > 0
                    && self.tree(q) == side
                    && let Some(d) = self.rooted(q)
                {
                    if best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((a, d));
                    }
                }
            }
            if let Some((a, d)) = best {
                self.set(p, side, a as u32, self.time, d.saturating_add(1));
                continue;
            }
            for a in self.arcs(p) {
                let q = self.to(a);
                if self.tree(q) != side {
                    continue;
                }
                let room = if side == S_TREE { self.cap(self.rev(a)) } else { self.cap(a) };
                if room > 0 {
                    self.activate(q);
                }
                let pq = self.parent(q);
                if pq != TERMINAL && pq != NO_PARENT && self.to(pq as usize) == p {
                    self.orphan(q);
                }
            }
            self.set(p, FREE, NO_PARENT, 0, 0);
        }
    }

    /// The distance from `q` to its tree's terminal, if its chain of parents reaches it; the
    /// chain's nodes are stamped with their distances, so the next walk stops at them.
    fn rooted(&mut self, q: usize) -> Option<u32> {
        let mut d = 0u32;
        let mut u = q;
        let mut steps = 0usize;
        loop {
            steps += 1;
            if steps > self.tr.len() + 1 {
                return None;
            }
            if self.stamp.get(u).copied() == Some(self.time) {
                d = d.saturating_add(self.dist.get(u).copied().unwrap_or(0));
                break;
            }
            let pa = self.parent(u);
            d = d.saturating_add(1);
            if pa == TERMINAL {
                if let (Some(s), Some(di)) = (self.stamp.get_mut(u), self.dist.get_mut(u)) {
                    (*s, *di) = (self.time, 1);
                }
                break;
            }
            if pa == NO_PARENT {
                return None;
            }
            u = self.to(pa as usize);
        }
        let (mut u, mut dd) = (q, d);
        while self.stamp.get(u).copied() != Some(self.time) {
            if let (Some(s), Some(di)) = (self.stamp.get_mut(u), self.dist.get_mut(u)) {
                (*s, *di) = (self.time, dd);
            }
            dd = dd.saturating_sub(1);
            let pa = self.parent(u);
            if pa == TERMINAL || pa == NO_PARENT {
                break;
            }
            u = self.to(pa as usize);
        }
        Some(d)
    }
}

/// He, Sun and Tang's guided filter of `p` by `guide` (both `w` × `h`, 0 to 1), radius `r`.
fn guided_filter(guide: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mean_i = box_mean(guide, w, h, r);
    let mean_p = box_mean(p, w, h, r);
    let ii: Vec<f32> = guide.iter().map(|v| v * v).collect();
    let ip: Vec<f32> = guide.iter().zip(p).map(|(a, b)| a * b).collect();
    let (corr_i, corr_ip) = (box_mean(&ii, w, h, r), box_mean(&ip, w, h, r));
    let mut a = Vec::with_capacity(w * h);
    let mut b = Vec::with_capacity(w * h);
    for (((mi, mp), ci), cip) in mean_i.iter().zip(&mean_p).zip(&corr_i).zip(&corr_ip) {
        let var = ci - mi * mi;
        let cov = cip - mi * mp;
        let ak = cov / (var + eps);
        a.push(ak);
        b.push(mp - ak * mi);
    }
    let (mean_a, mean_b) = (box_mean(&a, w, h, r), box_mean(&b, w, h, r));
    guide.iter().zip(mean_a).zip(mean_b).map(|((i, a), b)| a * i + b).collect()
}

/// The mean of each pixel's (2r + 1)² box (what of it is inside the picture).
fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let pass = |src: &[f32], len: usize, lines: usize, stride: usize, step: usize| {
        let mut out = vec![0.0f32; src.len()];
        let mut prefix = vec![0.0f64; len + 1];
        for line in 0..lines {
            let at = |k: usize| line * stride + k * step;
            for k in 0..len {
                let v = f64::from(src.get(at(k)).copied().unwrap_or(0.0));
                let before = prefix.get(k).copied().unwrap_or(0.0);
                if let Some(p) = prefix.get_mut(k + 1) {
                    *p = before + v;
                }
            }
            for k in 0..len {
                let (lo, hi) = (k.saturating_sub(r), (k + r + 1).min(len));
                let sum = prefix.get(hi).copied().unwrap_or(0.0) - prefix.get(lo).copied().unwrap_or(0.0);
                if let Some(o) = out.get_mut(at(k)) {
                    *o = (sum / (hi - lo).max(1) as f64) as f32;
                }
            }
        }
        out
    };
    let rows = pass(src, w, h, w, 1);
    pass(&rows, h, w, 1, w)
}

#[cfg(test)]
mod tests;
