//! Text lines of a PDF from its glyphs' baselines (hayro's interpreter), and the comparison of
//! two PDFs of the same document line by line.
//!
//! Baselines rather than text boxes: a box's top depends on the font metrics each PDF writer
//! stores, so two PDFs with the same glyphs in the same places can differ by several points.

use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;
use kurbo::{Affine, BezPath, Point, Rect};
use serde_json::{Value, json};

/// One glyph: baseline origin (points, y down from the page top), font size, text.
#[derive(Clone, Debug)]
struct G {
    x: f64,
    /// Where the glyph's advance ends (its start when the font gives no width).
    end: f64,
    y: f64,
    size: f64,
    text: String,
}

/// One line: page (0-based), baseline y, left x, text without spaces (the match key) and with
/// approximate spaces (for reading).
#[derive(Clone, Debug)]
pub struct Line {
    pub page: usize,
    pub y: f64,
    pub x: f64,
    pub key: String,
    pub text: String,
}

struct Collect {
    glyphs: Vec<G>,
}

impl Device<'_> for Collect {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, g: &Glyph<'_>, transform: Affine, glyph_transform: Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
        // A synthetic bold glyph is filled and then stroked: count it once.
        if matches!(mode, GlyphDrawMode::Stroke(_)) {
            return;
        }
        let text = match g.as_unicode() {
            Some(hayro_cmap::BfString::Char(c)) => c.to_string(),
            Some(hayro_cmap::BfString::String(s)) => s,
            None => '\u{FFFD}'.to_string(),
        };
        if text.trim().is_empty() {
            return;
        }
        // `glyph_transform` places the glyph's outline (1000 units per em) in user space at its
        // text position; `transform` maps user space to the page. The outline's origin is the
        // baseline start; the size is the page length of a one-em vertical step.
        let t = transform * glyph_transform;
        let o = t * Point::ZERO;
        let size = (t * Point::new(0.0, 1000.0) - o).hypot();
        // The advance, in the outline's units (PDF widths: thousandths of an em).
        let adv = match g {
            Glyph::Outline(o) => o.advance_width().map_or(0.0, f64::from),
            Glyph::Type3(_) => 0.0,
        };
        let end = (t * Point::new(adv, 0.0)).x;
        self.glyphs.push(G { x: o.x, end, y: o.y, size, text });
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The lines of every page of `bytes`, top to bottom.
pub fn lines(bytes: Vec<u8>) -> Result<(Vec<Line>, usize), String> {
    let pdf = Pdf::new(bytes).map_err(|e| format!("not a readable PDF: {e:?}"))?;
    let cache = InterpreterCache::new();
    let mut out = Vec::new();
    let pages = pdf.pages();
    for (n, page) in pages.iter().enumerate() {
        // y down from the page's top edge, rotation and crop box applied.
        let init = Affine::new(page.initial_transform(true).as_coeffs());
        let mut ctx = Context::new(init, Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        let mut c = Collect { glyphs: Vec::new() };
        interpret_page(page, &mut ctx, &mut c);
        out.extend(group(n, c.glyphs));
    }
    Ok((out, pages.len()))
}

/// Groups glyphs into lines: sorted by baseline, a glyph joins the line above when it is within
/// half the larger font size of it, so superscripts and subscripts stay on their line.
fn group(page: usize, mut glyphs: Vec<G>) -> Vec<Line> {
    glyphs.sort_by(|a, b| a.y.total_cmp(&b.y));
    let mut clusters: Vec<Vec<G>> = Vec::new();
    for g in glyphs {
        let join = clusters.last().and_then(|c| c.last()).is_some_and(|last| g.y - last.y <= 0.5 * g.size.max(last.size));
        match clusters.last_mut() {
            Some(c) if join => c.push(g),
            _ => clusters.push(vec![g]),
        }
    }
    clusters
        .into_iter()
        .map(|mut c| {
            // The line's baseline: that of its largest glyphs (not a raised or lowered one).
            let big = c.iter().map(|g| g.size).fold(0.0, f64::max);
            let mut ys: Vec<f64> = c.iter().filter(|g| g.size >= big - 0.25).map(|g| g.y).collect();
            ys.sort_by(f64::total_cmp);
            let y = ys.get(ys.len() / 2).copied().unwrap_or(0.0);
            c.sort_by(|a, b| a.x.total_cmp(&b.x));
            // A line number: leading digits more than 1.5 em before the next glyph. It stays in
            // the text but not in the match key, so a numbering difference does not hide breaks.
            let is_digit = |g: &G| g.text.chars().all(|ch| ch.is_ascii_digit());
            let first = (1..c.len()).take_while(|&i| is_digit(&c[i - 1])).find(|&i| c[i].x - c[i - 1].x > 1.5 * c[i - 1].size.max(1.0)).unwrap_or(0);
            let numbered = first > 0;
            // Segments split at gaps wider than three ems (table cells, columns), for reading.
            let mut segs: Vec<&[G]> = Vec::new();
            let mut start = 0;
            for i in 1..=c.len() {
                if i == c.len() || i == first || c[i].x - c[i - 1].x > 3.0 * c[i - 1].size.max(1.0) {
                    segs.push(&c[start..i]);
                    start = i;
                }
            }
            let x = c.get(first).map_or(0.0, |g| g.x);
            let key: String = c[first..].iter().map(|g| g.text.as_str()).collect();
            let mut text = String::new();
            for (k, seg) in segs.iter().enumerate() {
                if k > 0 {
                    text.push_str(if k == 1 && numbered { " " } else { " | " });
                }
                for (i, g) in seg.iter().enumerate() {
                    // A space where the gap after the previous glyph's advance passes 0.15 em.
                    if i > 0 && g.x - seg[i - 1].end > 0.15 * seg[i - 1].size {
                        text.push(' ');
                    }
                    text.push_str(&g.text);
                }
            }
            Line { page, y, x, key, text }
        })
        .collect()
}

/// Matched index pairs of two key lists, in order (longest common subsequence).
fn lcs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (ma, mb) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let (n, m) = (ma.len(), mb.len());
    let mut out: Vec<(usize, usize)> = (0..pre).map(|i| (i, i)).collect();
    if (n + 1).saturating_mul(m + 1) <= 50_000_000 {
        let mut t = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i * (m + 1) + j] =
                    if ma[i] == mb[j] { t[(i + 1) * (m + 1) + j + 1] + 1 } else { t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if ma[i] == mb[j] {
                out.push((pre + i, pre + j));
                (i, j) = (i + 1, j + 1);
            } else if t[(i + 1) * (m + 1) + j] >= t[i * (m + 1) + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    out.extend((0..suf).map(|k| (a.len() - suf + k, b.len() - suf + k)));
    out
}

/// Tolerance for "same place", in points.
pub const TOL: f64 = 0.5;

/// Compares a reference (Word) and a test (WordCraft) PDF of the same document.
pub fn compare(r: &[Line], rp: usize, t: &[Line], tp: usize) -> Value {
    let ka: Vec<&str> = r.iter().map(|l| l.key.as_str()).collect();
    let kb: Vec<&str> = t.iter().map(|l| l.key.as_str()).collect();
    let pairs = lcs(&ka, &kb);
    let mut matched = vec![None; r.len()];
    for &(a, b) in &pairs {
        matched[a] = Some(b);
    }
    let mut dys: Vec<f64> = Vec::new();
    let (mut max_dx, mut max_dy) = (0.0f64, 0.0f64);
    let mut first: Option<Value> = None;
    for (i, l) in r.iter().enumerate() {
        let Some(j) = matched[i] else {
            if first.is_none() {
                // The test's line in the same place in the matching: after the previous match.
                let prev = (0..i).rev().find_map(|k| matched[k]).map_or(0, |j| j + 1);
                first = Some(
                    json!({"kind": "line break", "page": l.page + 1, "word": l.text, "wordcraft": t.get(prev).map(|m| m.text.clone()), "word_page": l.page + 1, "wordcraft_page": t.get(prev).map(|m| m.page + 1)}),
                );
            }
            continue;
        };
        let m = &t[j];
        if m.page != l.page {
            if first.is_none() {
                first = Some(json!({"kind": "page break", "word_page": l.page + 1, "wordcraft_page": m.page + 1, "word": l.text}));
            }
            continue;
        }
        let (dy, dx) = (m.y - l.y, m.x - l.x);
        dys.push(dy.abs());
        max_dy = max_dy.max(dy.abs());
        max_dx = max_dx.max(dx.abs());
        if first.is_none() && (dy.abs() > TOL || dx.abs() > TOL) {
            first = Some(
                json!({"kind": if dy.abs() > TOL { "vertical offset" } else { "horizontal offset" }, "page": l.page + 1, "word": l.text, "dy": round(dy), "dx": round(dx), "word_y": round(l.y), "wordcraft_y": round(m.y)}),
            );
        }
    }
    dys.sort_by(f64::total_cmp);
    let same = pairs.len();
    let status = if rp == tp && same == r.len() && same == t.len() && max_dy <= TOL && max_dx <= TOL {
        "same"
    } else if rp == tp && same == r.len() && same == t.len() {
        "offsets"
    } else if rp == tp {
        "breaks"
    } else {
        "pages"
    };
    json!({
        "status": status,
        "pages": [rp, tp],
        "lines": [r.len(), t.len()],
        "same_break_pct": round(100.0 * same as f64 / r.len().max(1) as f64),
        "dy_median": round(dys.get(dys.len() / 2).copied().unwrap_or(0.0)),
        "dy_max": round(max_dy),
        "dx_max": round(max_dx),
        "first_difference": first,
    })
}

fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(x: f64, y: f64, size: f64, t: &str) -> G {
        G { x, end: x + 0.5 * size, y, size, text: t.into() }
    }

    #[test]
    fn superscripts_stay_on_their_line_and_lines_split_at_line_gaps() {
        let l = group(0, vec![g(10.0, 100.0, 11.0, "a"), g(16.0, 100.0, 11.0, "b"), g(22.0, 96.0, 7.0, "1"), g(10.0, 113.0, 11.0, "c")]);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].key.as_str(), l[0].y), ("ab1", 100.0));
        assert_eq!(l[1].key, "c");
    }

    #[test]
    fn a_line_number_in_the_margin_is_not_part_of_the_key() {
        let l = group(0, vec![g(36.0, 100.0, 10.0, "1"), g(36.0 + 6.0, 100.0, 10.0, "2"), g(72.0, 100.0, 11.0, "a"), g(78.0, 100.0, 11.0, "b")]);
        assert_eq!((l[0].key.as_str(), l[0].x, l[0].text.as_str()), ("ab", 72.0, "12 ab"));
        // Text starting with digits keeps them: only the number before the wide gap goes.
        let l = group(0, vec![g(36.0, 100.0, 10.0, "6"), g(72.0, 100.0, 11.0, "2"), g(78.0, 100.0, 11.0, ")")]);
        assert_eq!((l[0].key.as_str(), l[0].x), ("2)", 72.0));
    }

    #[test]
    fn identical_lines_are_the_same_and_a_moved_word_is_a_break() {
        let a = vec![
            Line { page: 0, y: 100.0, x: 72.0, key: "onetwo".into(), text: "one two".into() },
            Line { page: 0, y: 114.0, x: 72.0, key: "three".into(), text: "three".into() },
        ];
        assert_eq!(compare(&a, 1, &a, 1)["status"], "same");
        let b = vec![
            Line { page: 0, y: 100.0, x: 72.0, key: "one".into(), text: "one".into() },
            Line { page: 0, y: 114.0, x: 72.0, key: "twothree".into(), text: "two three".into() },
        ];
        let c = compare(&a, 1, &b, 1);
        assert_eq!(c["status"], "breaks");
        assert_eq!(c["first_difference"]["kind"], "line break");
        let mut d = a.clone();
        d[1].y = 115.0;
        let c = compare(&a, 1, &d, 1);
        assert_eq!((c["status"].as_str(), c["dy_max"].as_f64()), (Some("offsets"), Some(1.0)));
    }
}
