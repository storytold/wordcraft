//! Equation layout: the [`wordcraft_doc::math`] tree → positioned glyphs and rules.
//!
//! Follows the OpenType `MATH` model (the one Word's own math engine and Cambria Math were built
//! around): font constants for script shifts, fraction and radical gaps, limits and rules; size
//! variants and glyph assemblies for stretchy delimiters, radicals, big operators and wide
//! accents; atom classes with TeX/Word inter-atom spacing. Without a math font the layout uses
//! fixed proportions of the em.
//!
//! Coordinates: x right from the equation's left edge, y *up* from its baseline.

use std::sync::Arc;

use wordcraft_doc::math::{
    Arg, BrkBin, BrkBinSub, ColJc, FracKind, LimLoc, MAX_DEPTH, MClass, MNode, MRun, MScr, MSty, MathProps, ScriptKind, is_integral, math_alnum,
    parse_linear,
};
use wordcraft_doc::props::{Rgb, TextColor};
use wordcraft_doc::resolve::ResolvedChar;
use wordcraft_fonts::math::{C, Construction, MathTable, glyph_bounds};
use wordcraft_fonts::{FaceRef, FontDb};

mod bx;
mod glyphs;
mod nodes;
mod rows;
mod state;
mod structs;

use bx::St;
use state::Ctx;

/// Something to draw.
#[derive(Clone, Debug)]
pub enum MItem {
    Glyphs {
        face: FaceRef,
        size: f32,
        color: Rgb,
        synth_bold: bool,
        synth_italic: bool,
        /// Glyph id, x, y (baseline, up).
        glyphs: Vec<(u32, f32, f32)>,
        text: String,
    },
    /// A filled rectangle; `y` is its bottom edge.
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Rgb,
    },
    Line {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: Rgb,
        dotted: bool,
    },
    /// Drawn on screen only (placeholders for empty slots, the empty-equation hint).
    ScreenOnly(Box<MItem>),
}

impl MItem {
    fn shift(&mut self, dx: f32, dy: f32) {
        match self {
            MItem::Glyphs { glyphs, .. } => {
                for g in glyphs {
                    g.1 += dx;
                    g.2 += dy;
                }
            }
            MItem::Rect { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            MItem::Line { x0, y0, x1, y1, .. } => {
                *x0 += dx;
                *x1 += dx;
                *y0 += dy;
                *y1 += dy;
            }
            MItem::ScreenOnly(i) => i.shift(dx, dy),
        }
    }
}

/// A caret position inside an equation, where it is drawn: `x`, the baseline `y` (up from the
/// equation's baseline) and the caret's extent above (`a`) and below (`d`) it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub path: Vec<(usize, usize)>,
    pub off: usize,
    pub x: f32,
    pub y: f32,
    pub a: f32,
    pub d: f32,
}

/// A laid-out equation.
#[derive(Clone, Debug, Default)]
pub struct MathLayout {
    pub width: f32,
    /// Extent above and below the baseline, points.
    pub ascent: f32,
    pub descent: f32,
    pub items: Vec<MItem>,
    /// Every caret position, for drawing the caret and hit testing.
    pub slots: Vec<Slot>,
}

impl MathLayout {
    /// Where the caret at (`path`, `off`) is drawn.
    pub fn slot(&self, path: &[(usize, usize)], off: usize) -> Option<&Slot> {
        self.slots
            .iter()
            .find(|s| s.path == path && s.off == off)
            .or_else(|| self.slots.iter().filter(|s| s.path == path).min_by_key(|s| s.off.abs_diff(off)))
    }
    /// The caret position nearest to (`x`, `y`) (y up from the baseline).
    pub fn hit(&self, x: f32, y: f32) -> Option<&Slot> {
        self.slots.iter().min_by(|a, b| slot_dist(a, x, y).total_cmp(&slot_dist(b, x, y)))
    }
    /// The extent of the argument at `path` (a numerator, a radicand…): left and right x, and
    /// how far it reaches above and below the equation's baseline. Spans its caret positions,
    /// within the equation.
    pub fn arg_extent(&self, path: &[(usize, usize)]) -> Option<(f32, f32, f32, f32)> {
        let mut slots = self.slots.iter().filter(|s| s.path == path);
        let first = slots.next()?;
        let init = (first.x, first.x, first.y + first.a, first.d - first.y);
        let (x0, x1, up, down) = slots.fold(init, |(x0, x1, up, down), s| (x0.min(s.x), x1.max(s.x), up.max(s.y + s.a), down.max(s.d - s.y)));
        Some((x0, x1, up.min(self.ascent), down.min(self.descent)))
    }
}

fn slot_dist(s: &Slot, x: f32, y: f32) -> f32 {
    let (top, bottom) = (s.y + s.a, s.y - s.d);
    let dy = if y > top {
        y - top
    } else if y < bottom {
        bottom - y
    } else {
        0.0
    };
    // Inside a slot's height, prefer the innermost (shortest) argument.
    let h = (s.a + s.d).max(0.1);
    (s.x - x).abs() + dy * 3.0 + if dy == 0.0 { h * 0.05 } else { 0.0 }
}

/// Numbered rows (`#`) in an equation: they advance the automatic numbering.
pub fn auto_numbers(math: &wordcraft_doc::math::Math, display: bool) -> u32 {
    if !display {
        return 0;
    }
    match math.nodes.as_slice() {
        [MNode::EqArr { rows }] => rows.iter().filter(|r| split_number(r).is_some()).count() as u32,
        _ => 0,
    }
}

/// A row's equation and number parts around its top-level `#`, and the units before the `#`.
fn split_number(row: &[MNode]) -> Option<(Arg, Arg, usize)> {
    let mut eq = Vec::new();
    let mut units = 0usize;
    for (i, n) in row.iter().enumerate() {
        if let MNode::Run(r) = n
            && !r.lit
            && let Some(b) = r.text.find('#')
        {
            let mut left = r.clone();
            left.text = r.text.get(..b).unwrap_or("").to_string();
            let mut right = r.clone();
            right.text = r.text.get(b + 1..).unwrap_or("").to_string();
            units += left.text.chars().count();
            if !left.text.is_empty() {
                eq.push(MNode::Run(left));
            }
            let mut num = Vec::new();
            if !right.text.is_empty() {
                num.push(MNode::Run(right));
            }
            num.extend(row.get(i + 1..).unwrap_or(&[]).iter().cloned());
            return Some((eq, num, units));
        }
        units += match n {
            MNode::Run(r) => r.text.chars().count(),
            _ => 1,
        };
        eq.push(n.clone());
    }
    None
}

/// Lay out an equation set at the size and colour of `rc`. `display`: an equation on a line of
/// its own (bigger operators, limits above and below, full-size fractions).
/// `avail` is the width of the line (numbered display equations fill it, wider display equations
/// wrap to it as `props` says); `counter` counts the document's automatic equation numbers so far
/// and advances past this equation's.
pub fn layout_equation(
    math: &wordcraft_doc::math::Math,
    linear: &str,
    rc: &ResolvedChar,
    display: bool,
    avail: f32,
    props: &MathProps,
    counter: &mut u32,
) -> MathLayout {
    let parsed;
    let nodes: &Arg = if math.nodes.is_empty() && !math.linear {
        parsed = parse_linear(linear);
        &parsed
    } else {
        &math.nodes
    };
    let (face, table) = wordcraft_fonts::math::resolve_math("Cambria Math");
    let color = match rc.color {
        TextColor::Rgb(c) => c,
        TextColor::Auto => Rgb::BLACK,
    };
    let base = rc.draw_size().clamp(1.0, 1638.0);
    let mut cx = Ctx {
        face,
        t: table,
        upem: face.upem.max(1.0) as f32,
        base,
        color,
        text_font: rc.font.clone(),
        nodes: 0,
        path: Vec::new(),
        node_stack: Vec::new(),
    };
    let st = St { level: 0, display, cramped: false, upright: false };
    let avail = if avail.is_finite() { avail.clamp(0.0, 1e5) } else { 0.0 };
    let b = match nodes.as_slice() {
        [] => cx.hint(st),
        [MNode::EqArr { rows }] if display && rows.iter().any(|r| split_number(r).is_some()) => cx.numbered(rows, st, avail, counter),
        _ if display => cx.wrapped(nodes, st, avail, props),
        _ => cx.arg(nodes, st, 0),
    };
    let mut slots = b.slots;
    let em = base;
    for s in &mut slots {
        if s.a <= 0.0 && s.d <= 0.0 {
            s.a = em * 0.75;
            s.d = em * 0.25;
        }
    }
    MathLayout { width: b.w.max(0.0), ascent: b.a.max(0.0), descent: b.d.max(0.0), items: b.items, slots }
}

#[cfg(test)]
mod tests;
