use super::page::revision_color;
use super::paint::Stroke;
use super::{Document, Draw, HashMap, Rect, Rgb, ShapeKind, Spin, TextColor};

/// Each author's place among `authors` in order of first appearance (which colour marks their
/// changes and comments), found in one pass.
pub(crate) fn author_order<'a>(authors: impl Iterator<Item = &'a str>) -> HashMap<&'a str, u32> {
    let mut seen = HashMap::new();
    for a in authors {
        crate::work::steps(1);
        let n = u32::try_from(seen.len()).unwrap_or(u32::MAX);
        seen.entry(a).or_insert(n);
    }
    seen
}

/// The colours of the document's revision authors ([`author_order`]), worked out once for a
/// drawing, when its first tracked change is drawn, not once for every change.
pub(super) struct Authors<'a> {
    doc: &'a Document,
    order: std::cell::OnceCell<HashMap<&'a str, u32>>,
}

impl<'a> Authors<'a> {
    pub(super) fn new(doc: &'a Document) -> Self {
        Authors { doc, order: std::cell::OnceCell::new() }
    }

    /// The colour of revision `r`'s author.
    pub(super) fn color(&self, r: u32) -> Rgb {
        let order = self.order.get_or_init(|| author_order(self.doc.revisions.iter().map(|v| v.author.as_str())));
        revision_color(self.doc.revisions.get(r as usize).and_then(|v| order.get(v.author.as_str()).copied()).unwrap_or(0))
    }
}

/// How far a legacy check box's square sits inside the room it takes on its line, on every side.
pub const CHECK_BOX_INSET: f32 = 1.16;

/// A legacy form check box (`FORMCHECKBOX`) in `frame`, the room it takes on its line (as wide
/// as the text's line is tall): a square inset [`CHECK_BOX_INSET`] with a 0.72 pt border, and,
/// checked, an X from corner to corner in 0.48 pt lines, in the text's colour. Measured on Word's
/// PDF export: a 9.96 pt square for 10 pt Calibri, 22.08 pt for 20 pt.
pub fn check_box(frame: Rect, checked: bool, color: Rgb, alpha: f32, out: &mut Vec<Draw>) {
    let i = CHECK_BOX_INSET.min(frame.w / 4.0).min(frame.h / 4.0).max(0.0);
    let sq = Rect::new(frame.x + i, frame.y + i, (frame.w - 2.0 * i).max(0.5), (frame.h - 2.0 * i).max(0.5));
    let line =
        |x0: f32, y0: f32, x1: f32, y1: f32, width: f32| Draw::Line { x0, y0, x1, y1, width, color, stroke: Stroke::Solid, alpha, text_bg: None };
    let (l, t, r, b) = (sq.x, sq.y, sq.x + sq.w, sq.y + sq.h);
    out.push(Draw::Shape {
        rect: sq,
        kind: ShapeKind::Rectangle,
        fill: None,
        stroke: Some(color),
        stroke_width: 0.72,
        effects: Default::default(),
        corner: None,
        dash: Vec::new(),
        cap: Default::default(),
    });
    if checked {
        out.extend([line(l, t, r, b, 0.48), line(r, t, l, b, 0.48)]);
    }
}

/// Automatic colour is black, or white on a dark background.
pub fn text_color(c: &TextColor, background: Option<Rgb>) -> Rgb {
    match c {
        TextColor::Rgb(r) => *r,
        TextColor::Auto => {
            if background.is_some_and(|b| b.luma() < 100.0) {
                Rgb::WHITE
            } else {
                Rgb::BLACK
            }
        }
    }
}

/// Whether `c` stands upright in top-to-bottom East Asian text (`tbRl`) rather than turning with
/// the line: ideographs, kana, Hangul, full-width letters and digits, and the ideographic comma
/// and full stop (an approximation of Unicode's vertical orientation, UAX #50). Brackets, dashes
/// and the prolonged sound mark turn with the line, as their vertical forms do.
pub fn upright_in_vertical(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{11FF}'
        | '\u{2E80}'..='\u{2FFF}'
        | '\u{3001}'..='\u{3007}'
        | '\u{3012}'..='\u{3013}'
        | '\u{3020}'..='\u{302F}'
        | '\u{3031}'..='\u{303F}'
        | '\u{3040}'..='\u{30FB}'
        | '\u{30FD}'..='\u{30FF}'
        | '\u{3100}'..='\u{31EF}'
        | '\u{31F0}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{A960}'..='\u{A97F}'
        | '\u{AC00}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF01}'..='\u{FF07}'
        | '\u{FF0A}'..='\u{FF0C}'
        | '\u{FF0E}'..='\u{FF3A}'
        | '\u{FF3C}'
        | '\u{FF3E}'..='\u{FF5A}'
        | '\u{FFE0}'..='\u{FFE6}'
        | '\u{20000}'..='\u{3FFFF}'
    )
}

/// Top-to-bottom text drawn turned 90° clockwise, with its East Asian characters turned back
/// upright, each about the centre of its em square (`tbRl`: ideographs stand, Latin lies).
/// Glyph runs are split where uprightness changes, so the text keeps its order.
pub(super) fn upright_east_asian(items: Vec<Draw>) -> Vec<Draw> {
    let mut out = Vec::with_capacity(items.len());
    for d in items {
        let Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges, text_bg, x_scale } = d else {
            out.push(d);
            continue;
        };
        let up = |k: usize| ranges.get(k).and_then(|r| text.get(r.clone())).and_then(|t| t.chars().next()).is_some_and(upright_in_vertical);
        if ranges.len() != glyphs.len() || !(0..glyphs.len()).any(up) {
            out.push(Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges, text_bg, x_scale });
            continue;
        }
        let piece = |k0: usize, k1: usize| -> Draw {
            let gs = glyphs.get(k0..k1).unwrap_or(&[]).to_vec();
            let rs = ranges.get(k0..k1).unwrap_or(&[]);
            let (b0, b1) = (rs.iter().map(|r| r.start).min().unwrap_or(0), rs.iter().map(|r| r.end).max().unwrap_or(0));
            let t = text.get(b0..b1).unwrap_or("").to_string();
            let rs = rs.iter().map(|r| r.start.saturating_sub(b0)..r.end.saturating_sub(b0)).collect();
            Draw::Glyphs { face, size, glyphs: gs, color, alpha, synth_bold, synth_italic, text: t, link: link.clone(), ranges: rs, text_bg, x_scale }
        };
        let mut k0 = 0;
        while k0 < glyphs.len() {
            let u = up(k0);
            let k1 = (k0 + 1..glyphs.len()).find(|&k| up(k) != u).unwrap_or(glyphs.len());
            if !u {
                out.push(piece(k0, k1));
            } else {
                for k in k0..k1 {
                    let Some(&(_, gx, base)) = glyphs.get(k) else { continue };
                    // The em square: one em along the line (the advance to the next glyph when
                    // known), from about 0.88 em above the baseline to 0.12 em below it.
                    let adv = glyphs.get(k + 1).map(|g| g.1 - gx).filter(|a| *a > 0.0 && *a < size * 2.0).unwrap_or(size);
                    let (cx, cy) = (gx + adv / 2.0, base - size * 0.38);
                    let mut g = piece(k, k + 1);
                    // The ideographic comma and full stop sit in the upper right of their square in
                    // vertical text (lower left in horizontal): moved there before standing up.
                    if let Draw::Glyphs { text, glyphs, .. } = &mut g
                        && matches!(text.chars().next(), Some('\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}'))
                    {
                        for (_, x, y) in glyphs.iter_mut() {
                            *x += size * 0.55;
                            *y -= size * 0.55;
                        }
                    }
                    out.push(Draw::Rotated { cx, cy, spin: Spin::new(-90.0, false, false), items: vec![g] });
                }
            }
            k0 = k1;
        }
    }
    out
}
