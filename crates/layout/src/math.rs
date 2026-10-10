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
    Arg, ColJc, FracKind, LimLoc, MAX_DEPTH, MClass, MNode, MRun, MScr, MSty, ScriptKind, is_integral, math_alnum, parse_linear,
};
use wordcraft_doc::props::{Rgb, TextColor};
use wordcraft_doc::resolve::ResolvedChar;
use wordcraft_fonts::math::{C, Construction, MathTable, glyph_bounds};
use wordcraft_fonts::{FaceRef, FontDb};

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
/// `avail` is the width of the line (numbered display equations fill it); `counter` counts the
/// document's automatic equation numbers so far and advances past this equation's.
pub fn layout_equation(
    math: &wordcraft_doc::math::Math,
    linear: &str,
    rc: &ResolvedChar,
    display: bool,
    avail: f32,
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

/// Layout style: script level (0 = text size, 1 = script, 2 = script-script), display or inline,
/// cramped (superscripts lowered: inside radicals, denominators, subscripts).
#[derive(Clone, Copy, Debug)]
struct St {
    level: u8,
    display: bool,
    cramped: bool,
    /// Letters default to upright (function names).
    upright: bool,
}

impl St {
    fn script(self) -> St {
        St { level: (self.level + 1).min(2), display: false, ..self }
    }
    fn cramp(self) -> St {
        St { cramped: true, ..self }
    }
    /// Numerator / denominator style.
    fn frac(self) -> St {
        if self.display { St { display: false, ..self } } else { self.script() }
    }
}

/// TeX atom classes (spacing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Cls {
    #[default]
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

impl Cls {
    fn idx(self) -> usize {
        self as usize
    }
}

/// Inter-atom space in mu (1/18 em): TeX's table; negative = only outside scripts.
const SPACING: [[i8; 8]; 8] = [
    // Ord  Op  Bin  Rel Open Close Punct Inner
    [0, 3, -4, -5, 0, 0, 0, -3],     // Ord
    [3, 3, 0, -5, 0, 0, 0, -3],      // Op
    [-4, -4, 0, 0, -4, 0, 0, -4],    // Bin
    [-5, -5, 0, 0, -5, 0, 0, -5],    // Rel
    [0, 0, 0, 0, 0, 0, 0, 0],        // Open
    [0, 3, -4, -5, 0, 0, 0, -3],     // Close
    [-3, -3, 0, -3, -3, -3, -3, -3], // Punct
    [-3, 3, -4, -5, -3, 0, -3, -3],  // Inner
];

fn spacing_mu(l: Cls, r: Cls, st: St) -> f32 {
    let v = SPACING.get(l.idx()).and_then(|row| row.get(r.idx())).copied().unwrap_or(0);
    if v < 0 && st.level > 0 { 0.0 } else { v.unsigned_abs() as f32 }
}

/// A box under construction.
#[derive(Clone, Debug, Default)]
struct Bx {
    w: f32,
    a: f32,
    d: f32,
    items: Vec<MItem>,
    /// Italic correction after the box (superscripts move right by it).
    ic: f32,
    first: Cls,
    last: Cls,
    /// The box is one glyph: (face, glyph, top-accent attachment x).
    glyph: Option<(FaceRef, u32, f32)>,
    /// Takes no part in inter-atom spacing (spaces).
    transparent: bool,
    /// Caret positions inside, relative to the box.
    slots: Vec<Slot>,
    /// Text atoms: x of each character boundary (one more than the characters).
    cuts: Vec<f32>,
}

impl Bx {
    fn cls(mut self, c: Cls) -> Bx {
        self.first = c;
        self.last = c;
        self
    }
    /// Draw `o` at (dx, dy) into this box (extents grow; width is the caller's business).
    fn put(&mut self, o: Bx, dx: f32, dy: f32) {
        self.a = self.a.max(o.a + dy);
        self.d = self.d.max(o.d - dy);
        for mut it in o.items {
            it.shift(dx, dy);
            self.items.push(it);
        }
        for mut sl in o.slots {
            sl.x += dx;
            sl.y += dy;
            self.slots.push(sl);
        }
    }
    fn slot(&mut self, path: &[(usize, usize)], off: usize, x: f32) {
        self.slots.push(Slot { path: path.to_vec(), off, x, y: 0.0, a: 0.0, d: 0.0 });
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgb) {
        if w > 0.0 && h > 0.0 {
            self.items.push(MItem::Rect { x, y, w, h, color });
            self.a = self.a.max(y + h);
            self.d = self.d.max(-y);
        }
    }
}

struct Ctx {
    face: FaceRef,
    t: Option<Arc<MathTable>>,
    upem: f32,
    base: f32,
    color: Rgb,
    text_font: String,
    /// Nodes laid out so far (caps work on hostile input).
    nodes: usize,
    /// The argument being laid out (for caret slots).
    path: Vec<(usize, usize)>,
    /// Index of the node being laid out in its argument, innermost last.
    node_stack: Vec<usize>,
}

const MAX_NODES: usize = 200_000;

/// Constants as fractions of the em when there's no `MATH` table.
fn fallback(c: C) -> f32 {
    match c {
        C::MathLeading => 0.15,
        C::AxisHeight => 0.25,
        C::AccentBaseHeight => 0.45,
        C::FlattenedAccentBaseHeight => 0.66,
        C::SubscriptShiftDown => 0.2,
        C::SubscriptTopMax => 0.36,
        C::SubscriptBaselineDropMin => 0.05,
        C::SuperscriptShiftUp => 0.36,
        C::SuperscriptShiftUpCramped => 0.29,
        C::SuperscriptBottomMin => 0.11,
        C::SuperscriptBaselineDropMax => 0.25,
        C::SubSuperscriptGapMin => 0.2,
        C::SuperscriptBottomMaxWithSubscript => 0.36,
        C::SpaceAfterScript => 0.05,
        C::UpperLimitGapMin => 0.1,
        C::UpperLimitBaselineRiseMin => 0.2,
        C::LowerLimitGapMin => 0.1,
        C::LowerLimitBaselineDropMin => 0.6,
        C::StackTopShiftUp => 0.44,
        C::StackTopDisplayStyleShiftUp => 0.68,
        C::StackBottomShiftDown => 0.34,
        C::StackBottomDisplayStyleShiftDown => 0.69,
        C::StackGapMin => 0.12,
        C::StackDisplayStyleGapMin => 0.28,
        C::StretchStackTopShiftUp => 0.2,
        C::StretchStackBottomShiftDown => 0.6,
        C::StretchStackGapAboveMin => 0.1,
        C::StretchStackGapBelowMin => 0.1,
        C::FractionNumeratorShiftUp => 0.39,
        C::FractionNumeratorDisplayStyleShiftUp => 0.68,
        C::FractionDenominatorShiftDown => 0.34,
        C::FractionDenominatorDisplayStyleShiftDown => 0.69,
        C::FractionNumeratorGapMin => 0.04,
        C::FractionNumDisplayStyleGapMin => 0.12,
        C::FractionRuleThickness => 0.05,
        C::FractionDenominatorGapMin => 0.04,
        C::FractionDenomDisplayStyleGapMin => 0.12,
        C::SkewedFractionHorizontalGap => 0.35,
        C::SkewedFractionVerticalGap => 0.1,
        C::OverbarVerticalGap => 0.15,
        C::OverbarRuleThickness => 0.05,
        C::OverbarExtraAscender => 0.05,
        C::UnderbarVerticalGap => 0.15,
        C::UnderbarRuleThickness => 0.05,
        C::UnderbarExtraDescender => 0.05,
        C::RadicalVerticalGap => 0.05,
        C::RadicalDisplayStyleVerticalGap => 0.15,
        C::RadicalRuleThickness => 0.05,
        C::RadicalExtraAscender => 0.05,
        C::RadicalKernBeforeDegree => 0.28,
        C::RadicalKernAfterDegree => -0.55,
    }
}

/// Characters drawn as operators with their own spacing class.
fn class_of(c: char) -> Cls {
    match wordcraft_doc::math::math_class(c) {
        MClass::Ord => Cls::Ord,
        MClass::Op => Cls::Op,
        MClass::Bin => Cls::Bin,
        MClass::Rel => Cls::Rel,
        MClass::Open => Cls::Open,
        MClass::Close => Cls::Close,
        MClass::Punct => Cls::Punct,
        MClass::Inner => Cls::Inner,
    }
}

/// Accents drawn across the whole base (stretched horizontally).
fn wide_accent(c: char) -> bool {
    matches!(
        c,
        '\u{20D6}'
            | '\u{20D7}'
            | '\u{20E1}'
            | '\u{20D0}'
            | '\u{20D1}'
            | '\u{2190}'
            | '\u{2192}'
            | '\u{2194}'
            | '\u{23DC}'
            | '\u{23DD}'
            | '\u{23DE}'
            | '\u{23DF}'
            | '\u{23B4}'
            | '\u{23B5}'
    )
}

impl Ctx {
    fn scale(&self, level: u8) -> f32 {
        let (s1, s2) = match &self.t {
            Some(t) => (t.script_percent, t.script_script_percent),
            None => (70.0, 50.0),
        };
        let s1 = if (30.0..=100.0).contains(&s1) { s1 } else { 70.0 };
        let s2 = if (20.0..=100.0).contains(&s2) { s2 } else { 50.0 };
        match level {
            0 => 1.0,
            1 => s1 / 100.0,
            _ => s2 / 100.0,
        }
    }
    fn size(&self, st: St) -> f32 {
        self.base * self.scale(st.level)
    }
    /// A constant in points at the style's size.
    fn k(&self, c: C, st: St) -> f32 {
        let size = self.size(st);
        match &self.t {
            Some(t) => t.get(c) * size / self.upem,
            None => fallback(c) * size,
        }
    }
    fn axis(&self, st: St) -> f32 {
        self.k(C::AxisHeight, st)
    }
    fn em(&self, st: St) -> f32 {
        self.size(st)
    }

    fn arg(&mut self, a: &[MNode], st: St, depth: usize) -> Bx {
        let atoms = self.row(a, st, depth, 0);
        let em = self.em(st);
        let mut b = compose(atoms.into_iter().map(|(b, _)| b).collect(), st, em);
        finalize(&mut b, &self.path, em);
        b
    }

    /// Child argument `c` of the node being laid out; an empty one shows a placeholder.
    fn sub(&mut self, c: usize, a: &[MNode], st: St, depth: usize) -> Bx {
        let me = self.node_stack.last().copied().unwrap_or(0);
        self.path.push((me, c));
        let b = if a.is_empty() { self.placeholder(st) } else { self.arg(a, st, depth) };
        self.path.pop();
        b
    }

    /// The dotted box an empty argument shows on screen.
    fn placeholder(&mut self, st: St) -> Bx {
        let em = self.em(st);
        let (w, a, d, pad) = (em * 0.56, em * 0.7, em * 0.06, em * 0.05);
        let mut b = Bx { w: w + 2.0 * pad, a, d, ..Default::default() };
        let (x0, x1, y0, y1) = (pad, pad + w, -d, a);
        let width = (em * 0.035).max(0.4);
        for ((ax, ay), (bx, by)) in [((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))] {
            let line = MItem::Line { x0: ax, y0: ay, x1: bx, y1: by, width, color: Rgb(0x60, 0x60, 0x60), dotted: true };
            b.items.push(MItem::ScreenOnly(Box::new(line)));
        }
        let path = self.path.clone();
        b.slot(&path, 0, pad);
        finalize(&mut b, &path, em);
        b
    }

    /// An empty equation: a grey prompt (screen only).
    fn hint(&mut self, st: St) -> Bx {
        let size = self.size(st);
        let rs = wordcraft_fonts::word::resolve(&self.text_font, false, false);
        let mut g = self.glyphs(rs.face, "Type an equation", size, 0, Rgb(0x80, 0x80, 0x80), (rs.synth_bold, rs.synth_italic));
        g.items = std::mem::take(&mut g.items).into_iter().map(|i| MItem::ScreenOnly(Box::new(i))).collect();
        g.glyph = None;
        g.cuts.clear();
        g.slots.clear();
        g.slot(&[], 0, 0.0);
        finalize(&mut g, &[], size);
        g
    }

    /// The atoms of a row with their alignment segment (`&` starts a new segment). `u0`: the
    /// unit offset of the row's first node (for caret slots).
    fn row(&mut self, a: &[MNode], st: St, depth: usize, u0: usize) -> Vec<(Bx, usize)> {
        let mut out = Vec::new();
        if depth > MAX_DEPTH {
            return out;
        }
        let mut seg = 0usize;
        let mut u = u0;
        for (i, n) in a.iter().enumerate() {
            self.nodes += 1;
            if self.nodes > MAX_NODES {
                break;
            }
            let path = self.path.clone();
            match n {
                MNode::Run(r) => {
                    for part in self.run_atoms(r, st, u, &path) {
                        match part {
                            RunPart::Atom(b) => out.push((b, seg)),
                            RunPart::Align(b) => {
                                seg += 1;
                                out.push((b, seg));
                            }
                        }
                    }
                    u += r.text.chars().count();
                }
                MNode::Nary { .. } | MNode::Func { .. } => {
                    self.node_stack.push(i);
                    let mut atoms = self.op_atoms(n, st, depth + 1);
                    self.node_stack.pop();
                    if let Some(f) = atoms.first_mut() {
                        f.slot(&path, u, 0.0);
                    }
                    if let Some(l) = atoms.last_mut() {
                        let w = l.w;
                        l.slot(&path, u + 1, w);
                    }
                    for b in atoms {
                        out.push((b, seg));
                    }
                    u += 1;
                }
                _ => {
                    self.node_stack.push(i);
                    let mut b = self.node(n, st, depth + 1);
                    self.node_stack.pop();
                    let w = b.w;
                    b.slot(&path, u, 0.0);
                    b.slot(&path, u + 1, w);
                    out.push((b, seg));
                    u += 1;
                }
            }
        }
        out
    }

    /// Glyph run for `text` in `face` at `size`: a box with ink extents.
    fn glyphs(&self, face: FaceRef, text: &str, size: f32, level: u8, color: Rgb, synth: (bool, bool)) -> Bx {
        let feats = if level > 0 { vec![wordcraft_fonts::feature_value(b"ssty", level as u32)] } else { Vec::new() };
        let shaped = wordcraft_fonts::shape(&face, text, &feats, |c| c);
        let k = size / face.upem.max(1.0) as f32;
        let mut x = 0.0f32;
        let mut b = Bx::default();
        let mut glyphs = Vec::with_capacity(shaped.len());
        let mut last_gid = None;
        for g in &shaped {
            let gx = x + g.x_offset as f32 * k;
            let gy = g.y_offset as f32 * k;
            if let Some((x0, y0, x1, y1)) = glyph_bounds(&face, g.gid) {
                b.a = b.a.max(y1 * k + gy);
                b.d = b.d.max(-y0 * k - gy);
                let _ = (x0, x1);
            }
            glyphs.push((g.gid, gx, gy));
            x += g.x_advance as f32 * k;
            last_gid = Some(g.gid);
        }
        b.w = x;
        // Character boundaries: where the first glyph of each character starts.
        let mut cuts = Vec::with_capacity(text.len() + 1);
        for (cb, _) in text.char_indices() {
            let mut gx = 0.0f32;
            let mut found = None;
            for g in &shaped {
                if g.cluster >= cb {
                    found = Some(gx);
                    break;
                }
                gx += g.x_advance as f32 * k;
            }
            cuts.push(found.unwrap_or(x));
        }
        cuts.push(x);
        b.cuts = cuts;
        if let (Some(t), Some(g)) = (&self.t, last_gid)
            && face.id() == self.face.id()
        {
            b.ic = t.italics.get(&g).copied().unwrap_or(0.0) * k;
        }
        if shaped.len() == 1
            && let Some(g) = last_gid
        {
            let attach = match (&self.t, glyph_bounds(&face, g)) {
                (Some(t), _) if face.id() == self.face.id() && t.top_accent.contains_key(&g) => t.top_accent.get(&g).copied().unwrap_or(0.0) * k,
                (_, Some((x0, _, x1, _))) => (x0 + x1) / 2.0 * k,
                _ => b.w / 2.0,
            };
            b.glyph = Some((face, g, attach));
        }
        if !glyphs.is_empty() {
            b.items.push(MItem::Glyphs { face, size, color, synth_bold: synth.0, synth_italic: synth.1, glyphs, text: text.to_string() });
        }
        b
    }

    /// A single glyph by id (size variants, assembly parts).
    fn glyph_id(&self, gid: u32, size: f32, color: Rgb) -> Bx {
        let face = self.face;
        let k = size / face.upem.max(1.0) as f32;
        let mut b = Bx { w: face.advance(gid) as f32 * k, ..Default::default() };
        if let Some((_, y0, _, y1)) = glyph_bounds(&face, gid) {
            b.a = y1 * k;
            b.d = -y0 * k;
        }
        if let Some(t) = &self.t {
            b.ic = t.italics.get(&gid).copied().unwrap_or(0.0) * k;
        }
        b.items.push(MItem::Glyphs { face, size, color, synth_bold: false, synth_italic: false, glyphs: vec![(gid, 0.0, 0.0)], text: String::new() });
        b
    }

    /// A text atom with caret slots at its character boundaries (`k0`: the unit offset of its
    /// first character).
    fn text_atom(&self, face: FaceRef, text: &str, size: f32, level: u8, color: Rgb, synth: (bool, bool), path: &[(usize, usize)], k0: usize) -> Bx {
        let mut b = self.glyphs(face, text, size, level, color, synth);
        let cuts = std::mem::take(&mut b.cuts);
        for (j, x) in cuts.iter().enumerate() {
            b.slot(path, k0 + j, *x);
        }
        b
    }

    fn run_atoms(&self, r: &MRun, st: St, u0: usize, path: &[(usize, usize)]) -> Vec<RunPart> {
        let mut out = Vec::new();
        let size = r.size.filter(|s| s.is_finite()).unwrap_or(self.base).clamp(1.0, 1638.0) * self.scale(st.level);
        let color = r.color.unwrap_or(self.color);
        if r.nor {
            // Normal text: the text font, upright unless asked.
            let fam = r.font.clone().filter(|f| !f.eq_ignore_ascii_case("Cambria Math")).unwrap_or_else(|| self.text_font.clone());
            let (bold, italic) = match r.sty {
                Some(MSty::Bold) => (true, false),
                Some(MSty::Italic) => (false, true),
                Some(MSty::BoldItalic) => (true, true),
                _ => (false, false),
            };
            let rs = wordcraft_fonts::word::resolve(&fam, bold, italic);
            let mut b = self.text_atom(rs.face, &r.text, size, 0, color, (rs.synth_bold, rs.synth_italic), path, u0);
            b.glyph = None;
            out.push(RunPart::Atom(b.cls(Cls::Ord)));
            return out;
        }
        // Map characters (math alphabets, operators) and split into atoms by class.
        let mut cur = String::new();
        let mut cur_k0 = 0usize;
        let mut cur_face: Option<FaceRef> = None;
        let mut synth_italic = false;
        let flush = |cur: &mut String, k0: usize, face: Option<FaceRef>, synth_italic: bool, out: &mut Vec<RunPart>| {
            if !cur.is_empty() {
                let f = face.unwrap_or(self.face);
                let b = self.text_atom(f, cur, size, st.level, color, (false, synth_italic), path, u0 + k0);
                out.push(RunPart::Atom(b.cls(Cls::Ord)));
                cur.clear();
            }
        };
        // A zero-width (or space) atom that takes no part in spacing, with its two slots.
        let blank = |w: f32, k: usize| {
            let mut b = Bx { w, transparent: true, ..Default::default() };
            b.slot(path, u0 + k, 0.0);
            b.slot(path, u0 + k + 1, w);
            b
        };
        for (k, c) in r.text.chars().enumerate() {
            if c == '&' && !r.lit {
                flush(&mut cur, cur_k0, cur_face, synth_italic, &mut out);
                out.push(RunPart::Align(blank(0.0, k)));
                continue;
            }
            let c = if r.lit {
                c
            } else {
                match c {
                    '-' => '−',
                    '*' => '∗',
                    '\'' => '′',
                    _ => c,
                }
            };
            let is_letter = c.is_alphabetic();
            let greek_cap = ('\u{391}'..='\u{3A9}').contains(&c);
            let (bold, italic) = match r.sty {
                Some(MSty::Plain) => (false, false),
                Some(MSty::Bold) => (true, false),
                Some(MSty::Italic) => (false, true),
                Some(MSty::BoldItalic) => (true, true),
                None => (false, is_letter && !greek_cap && !st.upright && r.scr == MScr::Roman && !r.lit),
            };
            let mapped = math_alnum(c, bold, italic, r.scr);
            // Fall back to the plain letter (slanted) when the face lacks the math alphabet.
            let (ch, slant) = if mapped != c && !self.face.covers(mapped) { (c, italic) } else { (mapped, false) };
            let face = if self.face.covers(ch) || ch.is_whitespace() {
                None
            } else {
                FontDb::global().fallback_for(ch, self.face.id()).map(|f| FaceRef::of(&f))
            };
            let cls = if r.lit { Cls::Ord } else { class_of(ch) };
            if ch == ' ' || ch == '\u{2061}' || ch == '\u{2062}' || ch == '\u{2063}' || ch == '\u{200B}' {
                flush(&mut cur, cur_k0, cur_face, synth_italic, &mut out);
                // Word sets a typed space in an equation about 0.4 em wide.
                out.push(RunPart::Atom(blank(if ch == ' ' { size * 0.4 } else { 0.0 }, k)));
                continue;
            }
            let same_face = cur_face.map(|f| f.id()) == face.map(|f| f.id());
            if cls != Cls::Ord || !same_face || slant != synth_italic {
                flush(&mut cur, cur_k0, cur_face, synth_italic, &mut out);
            }
            if cls != Cls::Ord {
                let f = face.unwrap_or(self.face);
                let b = self.text_atom(f, &ch.to_string(), size, st.level, color, (false, slant), path, u0 + k);
                out.push(RunPart::Atom(b.cls(cls)));
                continue;
            }
            if cur.is_empty() {
                cur_k0 = k;
            }
            cur_face = face;
            synth_italic = slant;
            cur.push(ch);
        }
        flush(&mut cur, cur_k0, cur_face, synth_italic, &mut out);
        out
    }

    /// N-ary operators and functions: the operator atom (class Op) then the operand.
    fn op_atoms(&mut self, n: &MNode, st: St, depth: usize) -> Vec<Bx> {
        match n {
            MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub, sup, e } => {
                let e = self.sub(2, e, st, depth);
                let loc = lim_loc.unwrap_or(if is_integral(*chr) { LimLoc::SubSup } else { LimLoc::UndOvr });
                let loc = if st.display { loc } else { LimLoc::SubSup };
                let size = self.size(st);
                let base_gid = self.face.glyph_for(*chr);
                let mut target = 0.0f32;
                if st.display {
                    target = match &self.t {
                        Some(t) => t.display_operator_min_height * size / self.upem,
                        None => size * 1.5,
                    };
                }
                if *grow {
                    target = target.max(e.a + e.d);
                }
                let op = if *grow && target > 0.0 && self.t.is_some() {
                    self.stretch_v(base_gid, target, size).unwrap_or_else(|| self.glyph_id(base_gid, size, self.color))
                } else if target > 0.0 && self.t.is_some() {
                    // Word sets display operators about 1.2 em tall: the size variant nearest that.
                    let want = target.min(size * 1.2) * self.upem / size;
                    let (g, adv) = self
                        .t
                        .as_ref()
                        .and_then(|t| t.vert.get(&base_gid))
                        .and_then(|c| c.variants.iter().min_by(|a, b| (a.1 - want).abs().total_cmp(&(b.1 - want).abs())))
                        .copied()
                        .unwrap_or((base_gid, want));
                    // A font without a variant near that size: scale the nearest one.
                    let f = if adv > 0.0 { (want / adv).clamp(0.6, 1.5) } else { 1.0 };
                    let f = if (0.9..=1.1).contains(&f) { 1.0 } else { f };
                    self.glyph_id(g, size * f, self.color)
                } else if base_gid != 0 {
                    self.glyph_id(base_gid, size, self.color)
                } else {
                    let s = if st.display { size * 1.4 } else { size };
                    self.glyphs(self.face, &chr.to_string(), s, 0, self.color, (false, false))
                };
                // Centre the operator on the math axis.
                let dy = self.axis(st) - (op.a - op.d) / 2.0;
                let mut ob = Bx { w: op.w, ..Default::default() };
                let ic = op.ic;
                ob.put(op, 0.0, dy);
                let sub = if *sub_hide { None } else { Some(self.sub(0, sub, st.script().cramp(), depth)) };
                let sup = if *sup_hide { None } else { Some(self.sub(1, sup, st.script(), depth)) };
                let opb = match loc {
                    LimLoc::UndOvr => self.limits(ob, sub, sup, ic, st, true),
                    LimLoc::SubSup => {
                        ob.ic = ic;
                        self.attach_op(ob, sub, sup, st)
                    }
                };
                vec![opb.cls(Cls::Op), e]
            }
            MNode::Func { name, e } => {
                let name = self.sub(0, name, St { upright: true, ..st }, depth);
                let e = self.sub(1, e, St { upright: false, ..st }, depth);
                vec![name.cls(Cls::Op), e]
            }
            _ => Vec::new(),
        }
    }

    /// Limits under and over `base` (centred), spaced as for big operators (`big`) or as
    /// accents-like limits.
    fn limits(&self, base: Bx, under: Option<Bx>, over: Option<Bx>, ic: f32, st: St, big: bool) -> Bx {
        let w = base.w.max(under.as_ref().map_or(0.0, |b| b.w)).max(over.as_ref().map_or(0.0, |b| b.w));
        let mut out = Bx { w, ..Default::default() };
        let (ba, bd) = (base.a, base.d);
        let bw = base.w;
        out.put(base, (w - bw) / 2.0, 0.0);
        if let Some(o) = over {
            let rise = if big {
                self.k(C::UpperLimitBaselineRiseMin, st).max(self.k(C::UpperLimitGapMin, st) + o.d)
            } else {
                self.k(C::UpperLimitGapMin, st) + o.d
            };
            let x = (w - o.w) / 2.0 + ic / 2.0;
            out.put(o, x, ba + rise);
        }
        if let Some(u) = under {
            let drop = if big {
                self.k(C::LowerLimitBaselineDropMin, st).max(self.k(C::LowerLimitGapMin, st) + u.a)
            } else {
                self.k(C::LowerLimitGapMin, st) + u.a
            };
            let x = (w - u.w) / 2.0 - ic / 2.0;
            out.put(u, x, -bd - drop);
        }
        out
    }

    /// Limits beside a big operator: the superscript at its advance, the subscript tucked left
    /// under the slant by the italic correction.
    fn attach_op(&self, base: Bx, sub: Option<Bx>, sup: Option<Bx>, st: St) -> Bx {
        let ic = base.ic;
        let mut b = base;
        b.ic = 0.0;
        let (bw, bd) = (b.w, b.d);
        let mut out = self.attach(b, None, sup, st, false);
        if let Some(s) = sub {
            let v = self.k(C::SubscriptShiftDown, st).max(s.a - self.k(C::SubscriptTopMax, st)).max(bd + self.k(C::SubscriptBaselineDropMin, st));
            let x = (bw - ic).max(0.0);
            let sw = s.w;
            out.put(s, x, -v);
            out.w = out.w.max(x + sw + self.k(C::SpaceAfterScript, st));
        }
        out
    }

    /// Sub/superscripts after `base`. `char_base`: the base is a single character (no baseline drop).
    fn attach(&self, base: Bx, sub: Option<Bx>, sup: Option<Bx>, st: St, char_base: bool) -> Bx {
        let (bw, ba, bd, ic) = (base.w, base.a, base.d, base.ic);
        let (first, last) = (base.first, base.last);
        let mut out = Bx { w: bw, ..Default::default() };
        out.put(base, 0.0, 0.0);
        let mut u = 0.0f32;
        let mut v = 0.0f32;
        if let Some(p) = &sup {
            let shift = if st.cramped { self.k(C::SuperscriptShiftUpCramped, st) } else { self.k(C::SuperscriptShiftUp, st) };
            u = shift.max(p.d + self.k(C::SuperscriptBottomMin, st));
            if !char_base {
                u = u.max(ba - self.k(C::SuperscriptBaselineDropMax, st));
            }
        }
        if let Some(b) = &sub {
            v = self.k(C::SubscriptShiftDown, st).max(b.a - self.k(C::SubscriptTopMax, st));
            if !char_base {
                v = v.max(bd + self.k(C::SubscriptBaselineDropMin, st));
            }
        }
        if let (Some(p), Some(b)) = (&sup, &sub) {
            let gap = (u - p.d) - (b.a - v);
            let min = self.k(C::SubSuperscriptGapMin, st);
            if gap < min {
                v += min - gap;
                let lift = self.k(C::SuperscriptBottomMaxWithSubscript, st) - (u - p.d);
                if lift > 0.0 {
                    u += lift;
                    v -= lift;
                }
            }
        }
        let space = self.k(C::SpaceAfterScript, st);
        let mut w = bw;
        if let Some(p) = sup {
            let x = bw + ic;
            w = w.max(x + p.w);
            out.put(p, x, u);
        }
        if let Some(b) = sub {
            let x = bw;
            w = w.max(x + b.w);
            out.put(b, x, -v);
        }
        out.w = w + if w > bw { space } else { 0.0 };
        out.first = first;
        out.last = last;
        out
    }

    /// A vertical size variant or assembly of `gid` at least `target` tall (`None` without a
    /// `MATH` table or construction for the glyph).
    fn stretch_v(&self, gid: u32, target: f32, size: f32) -> Option<Bx> {
        let t = self.t.as_ref()?;
        let con = t.vert.get(&gid)?;
        self.stretch(con, t, target, size, true)
    }

    fn stretch_h(&self, gid: u32, target: f32, size: f32) -> Option<Bx> {
        let t = self.t.as_ref()?;
        let con = t.horiz.get(&gid)?;
        self.stretch(con, t, target, size, false)
    }

    fn stretch(&self, con: &Construction, t: &MathTable, target: f32, size: f32, vertical: bool) -> Option<Bx> {
        let k = size / self.upem;
        let target_fu = target / k;
        // The first variant big enough.
        if let Some((g, _)) = con.variants.iter().find(|(_, adv)| *adv >= target_fu - 1.0) {
            return Some(self.glyph_id(*g, size, self.color));
        }
        if con.parts.is_empty() {
            return con.variants.last().map(|(g, _)| self.glyph_id(*g, size, self.color));
        }
        // Assembly: repeat extenders until the parts span the target.
        let o = t.min_connector_overlap;
        let fixed: f32 = con.parts.iter().filter(|p| !p.extender).map(|p| p.full_advance).sum();
        let ext: f32 = con.parts.iter().filter(|p| p.extender).map(|p| p.full_advance).sum();
        let n_fixed = con.parts.iter().filter(|p| !p.extender).count();
        let n_ext = con.parts.len() - n_fixed;
        let mut reps = 0usize;
        if n_ext > 0 {
            while reps < 100 {
                let n = n_fixed + n_ext * reps;
                let len = fixed + ext * reps as f32 - o * n.saturating_sub(1) as f32;
                if len >= target_fu {
                    break;
                }
                reps += 1;
            }
        }
        let seq: Vec<_> = con.parts.iter().flat_map(|p| std::iter::repeat_n(*p, if p.extender { reps } else { 1 })).collect();
        if seq.is_empty() {
            return None;
        }
        let joints = seq.len().saturating_sub(1);
        let max_len: f32 = seq.iter().map(|p| p.full_advance).sum::<f32>() - o * joints as f32;
        // Spread the excess over the joints as extra overlap (within each connector's length).
        let excess = (max_len - target_fu).max(0.0);
        let per = if joints > 0 { excess / joints as f32 } else { 0.0 };
        let mut b = Bx::default();
        let mut pos = 0.0f32;
        let mut glyphs = Vec::with_capacity(seq.len());
        for (i, p) in seq.iter().enumerate() {
            if vertical {
                let y0 = glyph_bounds(&self.face, p.gid).map(|bb| bb.1).unwrap_or(0.0);
                glyphs.push((p.gid, 0.0, (pos - y0) * k));
                b.w = b.w.max(self.face.advance(p.gid) as f32 * k);
            } else {
                glyphs.push((p.gid, pos * k, 0.0));
                if let Some((_, y0, _, y1)) = glyph_bounds(&self.face, p.gid) {
                    b.a = b.a.max(y1 * k);
                    b.d = b.d.max(-y0 * k);
                }
            }
            if let Some(next) = seq.get(i + 1) {
                let room = p.end_connector.min(next.start_connector).max(o);
                let ov = (o + per).min(room);
                pos += p.full_advance - ov;
            } else {
                pos += p.full_advance;
            }
        }
        if vertical {
            b.a = pos * k;
            b.d = 0.0;
        } else {
            b.w = pos * k;
        }
        b.items.push(MItem::Glyphs { face: self.face, size, color: self.color, synth_bold: false, synth_italic: false, glyphs, text: String::new() });
        Some(b)
    }

    /// A delimiter (or separator) glyph covering `a`/`d` around the axis (or the content exactly
    /// with `exact`).
    fn delimiter(&self, c: char, a: f32, d: f32, st: St, grow: bool, exact: bool) -> Bx {
        let size = self.size(st);
        let axis = self.axis(st);
        let mut glyph = self.glyphs(self.face, &c.to_string(), size, 0, self.color, (false, false));
        glyph.glyph = None;
        if !grow {
            return glyph;
        }
        let target = if exact { a + d } else { 2.0 * (a - axis).max(d + axis) };
        // Small content keeps the plain glyph where it sits.
        if glyph.a + glyph.d >= target - size * 0.02 {
            return glyph;
        }
        let gid = self.face.glyph_for(c);
        let Some(big) = (if gid != 0 { self.stretch_v(gid, target, size) } else { None }) else { return glyph };
        let dy = if exact { (a - d) / 2.0 - (big.a - big.d) / 2.0 } else { axis - (big.a - big.d) / 2.0 };
        let mut out = Bx { w: big.w, ..Default::default() };
        out.put(big, 0.0, dy);
        out
    }

    fn node(&mut self, n: &MNode, st: St, depth: usize) -> Bx {
        if depth > MAX_DEPTH {
            return Bx::default();
        }
        let d = depth + 1;
        let em = self.em(st);
        match n {
            MNode::Run(r) => {
                let atoms: Vec<Bx> = self
                    .run_atoms(r, st, 0, &self.path.clone())
                    .into_iter()
                    .map(|p| match p {
                        RunPart::Atom(b) | RunPart::Align(b) => b,
                    })
                    .collect();
                compose(atoms, st, em)
            }
            MNode::Frac { kind, num, den } => self.frac(*kind, num, den, st, d),
            MNode::Script { kind, base, sub, sup } => {
                let b = self.sub(0, base, st, d);
                let char_base = b.glyph.is_some();
                let sub_b = if matches!(kind, ScriptKind::Sub | ScriptKind::SubSup | ScriptKind::Pre) {
                    Some(self.sub(1, sub, st.script().cramp(), d))
                } else {
                    None
                };
                let sup_b = if matches!(kind, ScriptKind::Sup | ScriptKind::SubSup | ScriptKind::Pre) {
                    Some(self.sub(2, sup, st.script(), d))
                } else {
                    None
                };
                if *kind == ScriptKind::Pre {
                    // Pre-scripts: attach to an empty base, then put the base after them.
                    let empty = Bx::default();
                    let mut pre = self.attach(empty, sub_b, sup_b, st, false);
                    let pw = pre.w;
                    let (f, l) = (b.first, b.last);
                    pre.w = pw + b.w;
                    pre.put(b, pw, 0.0);
                    pre.first = f;
                    pre.last = l;
                    return pre;
                }
                self.attach(b, sub_b, sup_b, st, char_base)
            }
            MNode::Rad { deg, deg_hide, e } => self.radical(deg, *deg_hide, e, st, d),
            MNode::Nary { .. } | MNode::Func { .. } => {
                let atoms = self.op_atoms(n, st, d);
                compose(atoms, st, em)
            }
            MNode::Delim { beg, end, sep, grow, shp_match, elems } => {
                let parts: Vec<Bx> = elems.iter().enumerate().map(|(k, e)| self.sub(k, e, st, d)).collect();
                let a = parts.iter().map(|b| b.a).fold(0.0f32, f32::max);
                let dd = parts.iter().map(|b| b.d).fold(0.0f32, f32::max);
                let mut row: Vec<Bx> = Vec::new();
                if let Some(c) = beg {
                    row.push(self.delimiter(*c, a, dd, st, *grow, *shp_match).cls(Cls::Open));
                }
                let n = parts.len();
                for (i, p) in parts.into_iter().enumerate() {
                    row.push(p.cls(Cls::Inner));
                    if i + 1 < n
                        && let Some(c) = sep
                    {
                        row.push(self.delimiter(*c, a, dd, st, *grow, *shp_match).cls(Cls::Ord));
                    }
                }
                if let Some(c) = end {
                    row.push(self.delimiter(*c, a, dd, st, *grow, *shp_match).cls(Cls::Close));
                }
                // No spacing inside the brackets.
                let mut out = hcat(row);
                out.first = if beg.is_some() { Cls::Open } else { Cls::Inner };
                out.last = if end.is_some() { Cls::Close } else { Cls::Inner };
                out
            }
            MNode::Lim { upper, e, lim } => {
                let base = self.sub(0, e, st, d);
                let ls = St { upright: false, ..st.script() };
                let l = self.sub(1, lim, if *upper { ls } else { ls.cramp() }, d);
                let (f, la) = (base.first, base.last);
                let mut out =
                    if *upper { self.limits(base, None, Some(l), 0.0, st, false) } else { self.limits(base, Some(l), None, 0.0, st, false) };
                out.first = f;
                out.last = la;
                out
            }
            MNode::Acc { chr, e } => self.accent(*chr, e, st, d),
            MNode::Bar { top, e } => {
                let b = self.sub(0, e, st, d);
                let (w, a, dd) = (b.w, b.a, b.d);
                let mut out = Bx { w, ..Default::default() };
                out.put(b, 0.0, 0.0);
                let color = self.color;
                if *top {
                    let t = self.k(C::OverbarRuleThickness, st).max(0.4);
                    let y = a + self.k(C::OverbarVerticalGap, st);
                    out.rect(0.0, y, w, t, color);
                    out.a = out.a.max(y + t + self.k(C::OverbarExtraAscender, st));
                } else {
                    let t = self.k(C::UnderbarRuleThickness, st).max(0.4);
                    let y = -dd - self.k(C::UnderbarVerticalGap, st) - t;
                    out.rect(0.0, y, w, t, color);
                    out.d = out.d.max(-y + self.k(C::UnderbarExtraDescender, st));
                }
                out
            }
            MNode::BorderBox { hide, strike, e } => {
                let b = self.sub(0, e, st, d);
                let t = self.k(C::FractionRuleThickness, st).max(0.4);
                let pad = em * 0.12;
                let (w, a, dd) = (b.w + 2.0 * pad, b.a + pad, b.d + pad);
                let mut out = Bx { w: w + 2.0 * t, ..Default::default() };
                out.put(b, pad + t, 0.0);
                let color = self.color;
                let [ht, hb, hl, hr] = *hide;
                let (x0, x1, y0, y1) = (0.0, w + 2.0 * t, -dd - t, a + t);
                if !ht {
                    out.rect(x0, a, x1 - x0, t, color);
                }
                if !hb {
                    out.rect(x0, y0, x1 - x0, t, color);
                }
                if !hl {
                    out.rect(x0, y0, t, y1 - y0, color);
                }
                if !hr {
                    out.rect(x1 - t, y0, t, y1 - y0, color);
                }
                let [sh, sv, bltr, tlbr] = *strike;
                let mid = (a - dd) / 2.0;
                let line = |out: &mut Bx, p: (f32, f32, f32, f32)| {
                    out.items.push(MItem::Line { x0: p.0, y0: p.1, x1: p.2, y1: p.3, width: t, color, dotted: false })
                };
                if sh {
                    line(&mut out, (x0, mid, x1, mid));
                }
                if sv {
                    line(&mut out, ((x0 + x1) / 2.0, y0, (x0 + x1) / 2.0, y1));
                }
                if bltr {
                    line(&mut out, (x0, y0, x1, y1));
                }
                if tlbr {
                    line(&mut out, (x0, y1, x1, y0));
                }
                out.a = out.a.max(y1);
                out.d = out.d.max(-y0);
                out
            }
            MNode::Boxed { e } => self.sub(0, e, st, d),
            MNode::GroupChr { chr, top, e } => {
                let b = self.sub(0, e, st, d);
                let size = self.size(st);
                let gid = self.face.glyph_for(*chr);
                let g =
                    self.stretch_h(gid, b.w, size).unwrap_or_else(|| self.glyphs(self.face, &chr.to_string(), size, 0, self.color, (false, false)));
                let w = b.w.max(g.w);
                let (ba, bd, bw) = (b.a, b.d, b.w);
                let (ga, gd, gw) = (g.a, g.d, g.w);
                let mut out = Bx { w, ..Default::default() };
                out.put(b, (w - bw) / 2.0, 0.0);
                if *top {
                    let y = ba + self.k(C::StretchStackGapAboveMin, st) + gd;
                    out.put(g, (w - gw) / 2.0, y);
                } else {
                    let y = -bd - self.k(C::StretchStackGapBelowMin, st) - ga;
                    out.put(g, (w - gw) / 2.0, y);
                }
                out
            }
            MNode::EqArr { rows } => self.eq_arr(rows, st, d),
            MNode::Matrix { rows, col_jc } => self.matrix(rows, col_jc, st, d),
            MNode::Phant { show, zero_wid, zero_asc, zero_desc, e } => {
                let mut b = self.sub(0, e, st, d);
                if !show {
                    b.items.clear();
                }
                if *zero_wid {
                    b.w = 0.0;
                }
                if *zero_asc {
                    b.a = 0.0;
                }
                if *zero_desc {
                    b.d = 0.0;
                }
                b.glyph = None;
                b
            }
        }
    }

    fn frac(&mut self, kind: FracKind, num: &[MNode], den: &[MNode], st: St, d: usize) -> Bx {
        let em = self.em(st);
        match kind {
            FracKind::Linear => {
                let n = self.sub(0, num, st, d);
                let dn = self.sub(1, den, st, d);
                let slash = self.glyphs(self.face, "/", self.size(st), st.level, self.color, (false, false)).cls(Cls::Ord);
                hcat(vec![n, slash, dn])
            }
            FracKind::Skewed => {
                let n = self.sub(0, num, st.script(), d);
                let dn = self.sub(1, den, st.script().cramp(), d);
                let size = self.size(st);
                let slash = self.glyphs(self.face, "\u{2044}", size, 0, self.color, (false, false));
                let gap = self.k(C::SkewedFractionHorizontalGap, st) / 2.0;
                let (nw, sw) = (n.w, slash.w);
                let mut out = Bx { w: n.w + gap + slash.w + gap + dn.w, ..Default::default() };
                let up = self.axis(st) + self.k(C::SkewedFractionVerticalGap, st) / 2.0;
                out.put(n, 0.0, up.max(0.0));
                out.put(slash, nw + gap, 0.0);
                out.put(dn, nw + gap + sw + gap, -self.k(C::SkewedFractionVerticalGap, st) / 2.0);
                out
            }
            FracKind::Bar | FracKind::NoBar => {
                let cs = st.frac();
                let n = self.sub(0, num, St { cramped: st.cramped, ..cs }, d);
                let dn = self.sub(1, den, cs.cramp(), d);
                let axis = self.axis(st);
                let (u, v) = if kind == FracKind::Bar {
                    let t = self.k(C::FractionRuleThickness, st);
                    let (su, sd, gn, gd) = if st.display {
                        (
                            self.k(C::FractionNumeratorDisplayStyleShiftUp, st),
                            self.k(C::FractionDenominatorDisplayStyleShiftDown, st),
                            self.k(C::FractionNumDisplayStyleGapMin, st),
                            self.k(C::FractionDenomDisplayStyleGapMin, st),
                        )
                    } else {
                        (
                            self.k(C::FractionNumeratorShiftUp, st),
                            self.k(C::FractionDenominatorShiftDown, st),
                            self.k(C::FractionNumeratorGapMin, st),
                            self.k(C::FractionDenominatorGapMin, st),
                        )
                    };
                    (su.max(gn + t / 2.0 + axis + n.d), sd.max(gd + t / 2.0 - axis + dn.a))
                } else {
                    let (mut su, mut sd, gap) = if st.display {
                        (
                            self.k(C::StackTopDisplayStyleShiftUp, st),
                            self.k(C::StackBottomDisplayStyleShiftDown, st),
                            self.k(C::StackDisplayStyleGapMin, st),
                        )
                    } else {
                        (self.k(C::StackTopShiftUp, st), self.k(C::StackBottomShiftDown, st), self.k(C::StackGapMin, st))
                    };
                    let g = (su - n.d) - (dn.a - sd);
                    if g < gap {
                        su += (gap - g) / 2.0;
                        sd += (gap - g) / 2.0;
                    }
                    (su, sd)
                };
                let pad = em * 0.08;
                let w = n.w.max(dn.w) + 2.0 * pad;
                let (nw, dw) = (n.w, dn.w);
                let mut out = Bx { w, ..Default::default() };
                out.put(n, (w - nw) / 2.0, u);
                out.put(dn, (w - dw) / 2.0, -v);
                if kind == FracKind::Bar {
                    let t = self.k(C::FractionRuleThickness, st).max(0.4);
                    out.rect(pad * 0.5, axis - t / 2.0, w - pad, t, self.color);
                }
                out
            }
        }
    }

    fn radical(&mut self, deg: &[MNode], deg_hide: bool, e: &[MNode], st: St, d: usize) -> Bx {
        let b = self.sub(1, e, st.cramp(), d);
        let size = self.size(st);
        let t = self.k(C::RadicalRuleThickness, st).max(0.4);
        let mut gap = if st.display { self.k(C::RadicalDisplayStyleVerticalGap, st) } else { self.k(C::RadicalVerticalGap, st) };
        let target = b.a + b.d + gap + t;
        let gid = self.face.glyph_for('√');
        let g = self.stretch_v(gid, target, size).unwrap_or_else(|| {
            let mut g = self.glyphs(self.face, "√", size, 0, self.color, (false, false));
            g.glyph = None;
            g
        });
        let extra = (g.a + g.d) - target;
        if extra > 0.0 {
            gap += extra / 2.0;
        }
        // The sign's top meets the rule over the base.
        let top = b.a + gap + t;
        let dy = top - g.a;
        let (ga, gd, gw) = (g.a, g.d, g.w);
        let mut sign = Bx { w: gw, ..Default::default() };
        sign.put(g, 0.0, dy);
        // Degree.
        let mut x0 = 0.0f32;
        let mut out = Bx::default();
        if !deg_hide {
            let db = self.sub(0, deg, St { level: (st.level + 2).min(2), display: false, cramped: true, upright: st.upright }, d);
            let kb = self.k(C::RadicalKernBeforeDegree, st);
            let ka = self.k(C::RadicalKernAfterDegree, st);
            let raise = self.t.as_ref().map_or(60.0, |t| t.radical_degree_bottom_raise_percent) / 100.0 * (ga + gd);
            let y = (dy - gd) + raise;
            let dw = db.w;
            out.put(db, kb, y);
            x0 = (kb + dw + ka).max(0.0);
        }
        let bw = b.w;
        out.put(sign, x0, 0.0);
        out.rect(x0 + gw, top - t, bw, t, self.color);
        out.put(b, x0 + gw, 0.0);
        out.w = x0 + gw + bw;
        out.a = out.a.max(top + self.k(C::RadicalExtraAscender, st));
        out
    }

    fn accent(&mut self, chr: char, e: &[MNode], st: St, d: usize) -> Bx {
        let b = self.sub(0, e, st.cramp(), d);
        let size = self.size(st);
        let color = self.color;
        // Overline-style accents are rules.
        if matches!(chr, '\u{305}' | '\u{AF}' | '\u{304}' | '\u{203E}') {
            let (w, a) = (b.w, b.a);
            let mut out = Bx { w, ..Default::default() };
            let (f, l) = (b.first, b.last);
            out.put(b, 0.0, 0.0);
            let t = self.k(C::OverbarRuleThickness, st).max(0.4);
            let y = a + self.k(C::OverbarVerticalGap, st);
            out.rect(0.0, y, w, t, color);
            out.first = f;
            out.last = l;
            return out;
        }
        let gid = self.face.glyph_for(chr);
        // Combining marks have no advance: compare the base with the accent's ink.
        let ink = glyph_bounds(&self.face, gid).map(|(x0, _, x1, _)| (x1 - x0) * size / self.upem).unwrap_or(0.0);
        let narrow = gid != 0 && b.w <= ink * 1.2;
        let mut acc = if wide_accent(chr) && gid != 0 && !narrow {
            self.stretch_h(gid, b.w, size).unwrap_or_else(|| self.glyph_id(gid, size, color))
        } else if gid != 0 {
            self.glyph_id(gid, size, color)
        } else {
            self.glyphs(self.face, &chr.to_string(), size, 0, color, (false, false))
        };
        acc.glyph = None;
        // Horizontal: the base's attachment point under the accent's.
        let base_x = b.glyph.map(|(_, _, ax)| ax).unwrap_or(b.w / 2.0);
        let acc_x = {
            let single = acc.items.first().and_then(|it| match it {
                MItem::Glyphs { glyphs, .. } if glyphs.len() == 1 => glyphs.first().map(|g| g.0),
                _ => None,
            });
            let k = size / self.upem;
            match single {
                Some(g) => match self.t.as_ref().and_then(|t| t.top_accent.get(&g).copied()) {
                    Some(v) => v * k,
                    None => glyph_bounds(&self.face, g).map(|(x0, _, x1, _)| (x0 + x1) / 2.0 * k).unwrap_or(acc.w / 2.0),
                },
                None => acc.w / 2.0,
            }
        };
        // Vertical: accents sit on an x-height base; taller bases push them up.
        let abh = self.k(C::AccentBaseHeight, st);
        let raise = (b.a - abh).max(0.0);
        let (f, l, w, ic) = (b.first, b.last, b.w, b.ic);
        let glyph = b.glyph;
        let mut out = Bx { w, ..Default::default() };
        out.put(b, 0.0, 0.0);
        let mut x = base_x - acc_x;
        if x < 0.0 && acc.w > w {
            x = (w - acc.w) / 2.0;
        }
        out.put(acc, x, raise);
        out.first = f;
        out.last = l;
        out.ic = ic;
        out.glyph = glyph.filter(|_| false);
        out
    }

    /// Rows stacked and centred on the math axis; `gap`: least space between rows, `min_skip`:
    /// least baseline-to-baseline distance.
    fn stack_rows(&self, rows: Vec<(Bx, f32)>, st: St, gap: f32, min_skip: f32) -> (Bx, Vec<f32>) {
        // (row box, x) → baselines going down.
        let mut ys = Vec::with_capacity(rows.len());
        let mut y = 0.0f32;
        let mut prev_d: Option<f32> = None;
        for (b, _) in &rows {
            if let Some(pd) = prev_d {
                y -= (pd + gap + b.a).max(min_skip);
            }
            ys.push(y);
            prev_d = Some(b.d);
        }
        let top = rows.first().map_or(0.0, |(b, _)| b.a);
        let bottom = rows.last().map_or(0.0, |(b, _)| b.d) - ys.last().copied().unwrap_or(0.0);
        // Centre on the axis.
        let shift = self.axis(st) + (bottom - top) / 2.0;
        let mut out = Bx::default();
        let mut bases = Vec::with_capacity(ys.len());
        for ((b, x), yy) in rows.into_iter().zip(ys) {
            out.w = out.w.max(x + b.w);
            out.put(b, x, yy + shift);
            bases.push(yy + shift);
        }
        (out, bases)
    }

    fn eq_arr(&mut self, rows: &[Arg], st: St, d: usize) -> Bx {
        self.eq_rows(rows, st, d).0
    }

    /// Equation-array rows aligned at `&`, and each row's baseline.
    fn eq_rows(&mut self, rows: &[Arg], st: St, d: usize) -> (Bx, Vec<f32>) {
        let em = self.em(st);
        let me = self.node_stack.last().copied().unwrap_or(0);
        // Each row: segments split at `&`.
        let mut segs_rows: Vec<Vec<Bx>> = Vec::new();
        let mut row_paths = Vec::new();
        for (ri, r) in rows.iter().take(1000).enumerate() {
            self.path.push((me, ri));
            row_paths.push(self.path.clone());
            let atoms = if r.is_empty() { vec![(self.placeholder(st), 0)] } else { self.row(r, st, d, 0) };
            self.path.pop();
            let n = atoms.iter().map(|(_, s)| *s + 1).max().unwrap_or(1).min(64);
            let all: Vec<Bx> = atoms.iter().map(|(b, _)| b.clone()).collect();
            let gaps = gaps_between(&all, st, em);
            let mut segs: Vec<Vec<Bx>> = vec![Vec::new(); n];
            for (i, (mut b, s)) in atoms.into_iter().enumerate() {
                // Keep the spacing before an atom with the atom.
                let g = gaps.get(i).copied().unwrap_or(0.0);
                if g > 0.0 {
                    let mut wrapped = Bx { w: g + b.w, a: b.a, d: b.d, first: b.first, last: b.last, ..Default::default() };
                    wrapped.put(b, g, 0.0);
                    b = wrapped;
                }
                if let Some(seg) = segs.get_mut(s.min(n - 1)) {
                    seg.push(b);
                }
            }
            segs_rows.push(segs.into_iter().map(hcat).collect());
        }
        let ncols = segs_rows.iter().map(|r| r.len()).max().unwrap_or(1);
        let mut col_w = vec![0.0f32; ncols];
        for r in &segs_rows {
            for (i, b) in r.iter().enumerate() {
                if let Some(c) = col_w.get_mut(i) {
                    *c = c.max(b.w);
                }
            }
        }
        let total: f32 = col_w.iter().sum();
        let mut placed = Vec::new();
        for (r, path) in segs_rows.into_iter().zip(row_paths) {
            let mut row = Bx::default();
            let mut x = 0.0f32;
            let single = r.len() == 1 && ncols > 1;
            for (i, b) in r.into_iter().enumerate() {
                let cw = col_w.get(i).copied().unwrap_or(0.0);
                let bw = b.w;
                let dx = if ncols == 1 {
                    0.0
                } else if single {
                    // A row without `&` aligns with the first column's right edge.
                    cw - bw
                } else if i % 2 == 0 {
                    cw - bw
                } else {
                    0.0
                };
                row.put(b, x + dx, 0.0);
                x += cw;
                row.w = row.w.max(x);
            }
            let rx = if ncols == 1 { (total - row.w) / 2.0 } else { 0.0 };
            finalize(&mut row, &path, em);
            placed.push((row, rx.max(0.0)));
        }
        let (mut out, bases) = self.stack_rows(placed, st, em * 0.1, em * 1.1);
        out.w = out.w.max(total);
        (out, bases)
    }

    /// A display equation with numbers (`#`): the equation centred on the line, each row's
    /// number at the right margin. Automatic numbers (`#` alone) count up from `counter`.
    fn numbered(&mut self, rows: &[Arg], st: St, avail: f32, counter: &mut u32) -> Bx {
        let em = self.em(st);
        let parts: Vec<(Arg, Arg, usize)> =
            rows.iter().take(1000).map(|r| split_number(r).unwrap_or_else(|| (r.clone(), Vec::new(), usize::MAX))).collect();
        self.node_stack.push(0);
        let eq_rows: Vec<Arg> = parts.iter().map(|(e, _, _)| e.clone()).collect();
        let (eq, bases) = self.eq_rows(&eq_rows, st, 1);
        // Numbers.
        let mut nums = Vec::new();
        for (ri, (_, num, hash)) in parts.iter().enumerate() {
            if *hash == usize::MAX {
                nums.push(None);
                continue;
            }
            self.path.push((0, ri));
            let path = self.path.clone();
            // Every numbered equation counts; `#` alone shows the count.
            *counter = counter.saturating_add(1);
            let b = if num.is_empty() {
                let r = MRun { text: format!("({counter})"), sty: Some(MSty::Plain), ..Default::default() };
                let mut b = compose(
                    self.run_atoms(&r, St { upright: true, ..st }, 0, &path)
                        .into_iter()
                        .map(|p| match p {
                            RunPart::Atom(b) | RunPart::Align(b) => b,
                        })
                        .collect(),
                    st,
                    em,
                );
                b.slots.clear();
                b.slot(&path, hash + 1, 0.0);
                b
            } else {
                let atoms = self.row(num, st, 2, hash + 1);
                let mut b = compose(atoms.into_iter().map(|(b, _)| b).collect(), st, em);
                b.slot(&path, hash + 1, 0.0);
                b
            };
            let mut b = b;
            finalize(&mut b, &path, em);
            self.path.pop();
            nums.push(Some(b));
        }
        self.node_stack.pop();
        let num_w = nums.iter().flatten().map(|b| b.w).fold(0.0f32, f32::max);
        let gap = em;
        let mut x_eq = ((avail - eq.w) / 2.0).max(0.0);
        if x_eq + eq.w + gap + num_w > avail {
            x_eq = (avail - num_w - gap - eq.w).max(0.0);
        }
        let width = avail.max(x_eq + eq.w + gap + num_w);
        let mut out = Bx { w: width, ..Default::default() };
        out.put(eq, x_eq, 0.0);
        for (b, y) in nums.into_iter().zip(bases) {
            if let Some(b) = b {
                let bw = b.w;
                out.put(b, width - bw, y);
            }
        }
        out
    }

    fn matrix(&mut self, rows: &[Vec<Arg>], col_jc: &[ColJc], st: St, d: usize) -> Bx {
        let em = self.em(st);
        let mut cells: Vec<Vec<Bx>> = Vec::new();
        let mut flat = 0usize;
        for r in rows.iter().take(1000) {
            let mut row = Vec::new();
            for (ci, c) in r.iter().enumerate() {
                if ci < 64 {
                    row.push(self.sub(flat, c, st, d));
                }
                flat += 1;
            }
            cells.push(row);
        }
        let ncols = cells.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut col_w = vec![0.0f32; ncols];
        for r in &cells {
            for (i, b) in r.iter().enumerate() {
                if let Some(c) = col_w.get_mut(i) {
                    *c = c.max(b.w);
                }
            }
        }
        let gap = em * 0.8;
        let mut placed = Vec::new();
        for r in cells {
            let mut row = Bx::default();
            let mut x = 0.0f32;
            for (i, b) in r.into_iter().enumerate() {
                let cw = col_w.get(i).copied().unwrap_or(0.0);
                let bw = b.w;
                let dx = match col_jc.get(i).copied().unwrap_or_default() {
                    ColJc::Center => (cw - bw) / 2.0,
                    ColJc::Left => 0.0,
                    ColJc::Right => cw - bw,
                };
                row.put(b, x + dx, 0.0);
                x += cw + gap;
            }
            row.w = (x - gap).max(0.0);
            placed.push((row, 0.0));
        }
        let (mut out, _) = self.stack_rows(placed, st, em * 0.25, em * 1.2);
        out.w = col_w.iter().sum::<f32>() + gap * ncols.saturating_sub(1) as f32;
        out
    }
}

enum RunPart {
    Atom(Bx),
    /// An `&` alignment mark (a zero-width atom holding its caret slots).
    Align(Bx),
}

/// Give the caret slots of argument `path` in `b` the argument's height.
fn finalize(b: &mut Bx, path: &[(usize, usize)], em: f32) {
    let (a, d) = (b.a.max(em * 0.72), b.d.max(em * 0.22));
    for s in &mut b.slots {
        if s.path == path && s.a <= 0.0 && s.d <= 0.0 {
            s.a = a - s.y;
            s.d = d + s.y;
        }
    }
}

/// Spacing before each atom (TeX rules: a binary operator after nothing, an operator, a
/// relation, an opening bracket or punctuation is ordinary, as is one before a relation,
/// closing bracket or punctuation).
fn gaps_between(atoms: &[Bx], st: St, em: f32) -> Vec<f32> {
    let mut cls: Vec<(Cls, Cls)> = atoms.iter().map(|b| (b.first, b.last)).collect();
    let solid: Vec<usize> = (0..atoms.len()).filter(|i| atoms.get(*i).is_some_and(|b| !b.transparent)).collect();
    for (k, &i) in solid.iter().enumerate() {
        let prev = k.checked_sub(1).and_then(|p| solid.get(p)).and_then(|p| cls.get(*p)).map(|c| c.1);
        let next = solid.get(k + 1).and_then(|n| cls.get(*n)).map(|c| c.0);
        if let Some(c) = cls.get_mut(i) {
            if c.0 == Cls::Bin && matches!(prev, None | Some(Cls::Bin | Cls::Op | Cls::Rel | Cls::Open | Cls::Punct)) {
                c.0 = Cls::Ord;
                if c.1 == Cls::Bin {
                    c.1 = Cls::Ord;
                }
            }
            if c.1 == Cls::Bin && matches!(next, None | Some(Cls::Rel | Cls::Close | Cls::Punct)) {
                c.1 = Cls::Ord;
                if c.0 == Cls::Bin {
                    c.0 = Cls::Ord;
                }
            }
        }
    }
    let mut gaps = vec![0.0f32; atoms.len()];
    for k in 1..solid.len() {
        let (Some(&p), Some(&i)) = (solid.get(k - 1), solid.get(k)) else { continue };
        let (Some(l), Some(r)) = (cls.get(p), cls.get(i)) else { continue };
        if let Some(g) = gaps.get_mut(i) {
            *g = spacing_mu(l.1, r.0, st) * em / 18.0;
        }
    }
    gaps
}

/// Atoms side by side with inter-atom spacing.
fn compose(mut atoms: Vec<Bx>, st: St, em: f32) -> Bx {
    if atoms.len() == 1
        && let Some(b) = atoms.pop()
    {
        return b;
    }
    let gaps = gaps_between(&atoms, st, em);
    let first = atoms.iter().find(|b| !b.transparent).map(|b| b.first).unwrap_or_default();
    let last = atoms.iter().rev().find(|b| !b.transparent).map(|b| b.last).unwrap_or_default();
    let ic = atoms.last().map(|b| b.ic).unwrap_or(0.0);
    let mut out = Bx::default();
    let mut x = 0.0f32;
    for (b, g) in atoms.into_iter().zip(gaps) {
        x += g;
        let w = b.w;
        out.put(b, x, 0.0);
        x += w;
    }
    out.w = x;
    out.first = first;
    out.last = last;
    out.ic = ic;
    out
}

/// Boxes side by side, no spacing.
fn hcat(parts: Vec<Bx>) -> Bx {
    let first = parts.first().map(|b| b.first).unwrap_or_default();
    let last = parts.last().map(|b| b.last).unwrap_or_default();
    let mut out = Bx::default();
    let mut x = 0.0f32;
    for b in parts {
        let w = b.w;
        out.put(b, x, 0.0);
        x += w;
    }
    out.w = x;
    out.first = first;
    out.last = last;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::math::Math;

    fn rc() -> ResolvedChar {
        wordcraft_doc::styles::StyleSheet::default().resolve_char(None, &Default::default())
    }

    fn lay(lin: &str, display: bool) -> MathLayout {
        layout_equation(&Math::default(), lin, &rc(), display, 400.0, &mut 0)
    }

    #[test]
    fn fraction_is_stacked_with_a_rule() {
        let flat = lay("ab", true);
        let f = lay("a/b", true);
        assert!(f.ascent > flat.ascent + 2.0 && f.descent > flat.descent + 2.0, "{f:?}");
        assert!(f.items.iter().any(|i| matches!(i, MItem::Rect { .. })), "fraction rule");
        assert!(f.width < flat.width * 1.2, "stacked, not side by side");
    }

    #[test]
    fn superscript_is_raised_and_smaller() {
        let m = lay("x^2", false);
        let sizes: Vec<(f32, f32)> = m
            .items
            .iter()
            .filter_map(|i| match i {
                MItem::Glyphs { size, glyphs, .. } => glyphs.first().map(|g| (*size, g.2)),
                _ => None,
            })
            .collect();
        assert_eq!(sizes.len(), 2, "{m:?}");
        assert!(sizes[1].0 < sizes[0].0 && sizes[1].1 > 2.0, "{sizes:?}");
    }

    #[test]
    fn operators_get_spaces_and_unary_minus_does_not() {
        let tight = lay("ab", false);
        let spaced = lay("a+b", false);
        let unary = lay("−b", false);
        let plus_w = lay("+", false).width;
        assert!(spaced.width > tight.width + plus_w + 2.0, "medium spaces around +");
        assert!(unary.width < lay("b", false).width + plus_w + 1.0, "no space after a unary minus");
    }

    #[test]
    fn display_operators_are_big_with_limits_over_and_under() {
        let d = lay("∑_(i=1)^n▒i", true);
        let t = lay("∑_(i=1)^n▒i", false);
        assert!(d.ascent + d.descent > t.ascent + t.descent, "display sum taller");
        assert!(d.width < t.width + 4.0, "limits stacked in display");
    }

    #[test]
    fn delimiters_grow_around_tall_content() {
        let small = lay("(x)", true);
        let big = lay("((a)/(b))", true);
        let h = |m: &MathLayout| m.ascent + m.descent;
        assert!(h(&big) > h(&small) * 1.4, "{} vs {}", h(&big), h(&small));
    }

    #[test]
    fn every_structure_lays_out() {
        use wordcraft_doc::math::*;
        let x = || vec![MNode::Run(MRun::new("x"))];
        let nodes = vec![
            MNode::Acc { chr: '\u{302}', e: x() },
            MNode::Acc { chr: '\u{305}', e: x() },
            MNode::Bar { top: false, e: x() },
            MNode::BorderBox { hide: [false; 4], strike: [true; 4], e: x() },
            MNode::Boxed { e: x() },
            MNode::GroupChr { chr: '⏟', top: false, e: x() },
            MNode::EqArr { rows: vec![vec![MNode::Run(MRun::new("x&=1"))], vec![MNode::Run(MRun::new("y&=22"))]] },
            MNode::Matrix { rows: vec![vec![x(), x()], vec![x(), Vec::new()]], col_jc: vec![] },
            MNode::Phant { show: false, zero_wid: false, zero_asc: false, zero_desc: true, e: x() },
            MNode::Lim { upper: false, e: vec![MNode::Run(MRun::plain("lim"))], lim: x() },
            MNode::Rad { deg: x(), deg_hide: false, e: x() },
            MNode::Script { kind: ScriptKind::Pre, base: x(), sub: x(), sup: x() },
            MNode::Frac { kind: FracKind::Skewed, num: x(), den: x() },
            MNode::Frac { kind: FracKind::Linear, num: x(), den: x() },
            MNode::Frac { kind: FracKind::NoBar, num: x(), den: x() },
            MNode::Delim { beg: None, end: Some('|'), sep: Some('|'), grow: true, shp_match: true, elems: vec![x(), x()] },
            MNode::Run(MRun { text: "text".into(), nor: true, ..Default::default() }),
            MNode::Run(MRun { text: "R".into(), scr: MScr::DoubleStruck, ..Default::default() }),
        ];
        for n in nodes {
            let m = layout_equation(&Math { nodes: vec![n.clone()], ..Default::default() }, "", &rc(), true, 400.0, &mut 0);
            assert!(m.width > 0.0 && m.ascent + m.descent > 0.0, "{n:?} → {m:?}");
            for it in &m.items {
                if let MItem::Glyphs { glyphs, .. } = it {
                    assert!(glyphs.iter().all(|g| g.1.is_finite() && g.2.is_finite()));
                }
            }
        }
    }

    #[test]
    fn caret_slots_cover_every_position() {
        use wordcraft_doc::math_edit::{MathPos, move_right, units};
        let math = Math { nodes: wordcraft_doc::math::parse_linear("x=(-b±√(b^2-4ac))/2a+■(a&b@c&d)"), ..Default::default() };
        let m = layout_equation(&math, "", &rc(), true, 400.0, &mut 0);
        // Walking the caret through the equation, every position has a slot.
        let mut p = MathPos::default();
        let mut seen = 0;
        loop {
            let s = m.slots.iter().find(|s| s.path == p.path && s.off == p.off);
            assert!(s.is_some(), "no slot for {p:?}");
            seen += 1;
            match move_right(&math.nodes, &p) {
                Some(n) => p = n,
                None => break,
            }
            assert!(seen < 500);
        }
        assert_eq!(p.off, units(&math.nodes));
        // Hit testing finds the denominator below the bar.
        let den = m.slots.iter().find(|s| s.path == vec![(1, 1)]).unwrap();
        let hit = m.hit(den.x + 1.0, den.y).unwrap();
        assert_eq!(hit.path, vec![(1, 1)]);
    }

    #[test]
    fn empty_slots_show_placeholders_on_screen_only() {
        let math = Math { nodes: wordcraft_doc::math::parse_linear("⬚/⬚"), ..Default::default() };
        let m = layout_equation(&math, "", &rc(), true, 400.0, &mut 0);
        assert!(m.items.iter().all(|i| matches!(i, MItem::ScreenOnly(_) | MItem::Rect { .. })), "{:?}", m.items);
        assert!(m.slots.iter().any(|s| s.path == vec![(0, 0)] && s.off == 0));
        let empty = layout_equation(&Math::default(), "", &rc(), true, 400.0, &mut 0);
        assert!(empty.width > 10.0 && empty.items.iter().all(|i| matches!(i, MItem::ScreenOnly(_))));
    }

    #[test]
    fn numbered_equations_fill_the_line_with_the_number_at_the_right() {
        let math = Math { nodes: wordcraft_doc::math::parse_linear("E=mc^2#(1)"), ..Default::default() };
        let m = layout_equation(&math, "", &rc(), true, 400.0, &mut 0);
        assert!((m.width - 400.0).abs() < 0.01, "{}", m.width);
        let xs: Vec<f32> =
            m.items.iter().filter_map(|i| if let MItem::Glyphs { glyphs, .. } = i { glyphs.first().map(|g| g.1) } else { None }).collect();
        let max = xs.iter().cloned().fold(0.0f32, f32::max);
        assert!(max > 350.0, "number at the right margin: {xs:?}");
        let min = xs.iter().cloned().fold(f32::MAX, f32::min);
        assert!(min > 120.0 && min < 220.0, "equation centred: {xs:?}");
        // Automatic numbers count.
        let auto = Math { nodes: wordcraft_doc::math::parse_linear("a=b#"), ..Default::default() };
        let mut n = 4;
        let _ = layout_equation(&auto, "", &rc(), true, 400.0, &mut n);
        assert_eq!(n, 5);
        assert_eq!(auto_numbers(&auto, true), 1);
    }

    #[test]
    fn math_alphabets() {
        assert_eq!(math_alnum('x', false, true, MScr::Roman), '𝑥');
        assert_eq!(math_alnum('h', false, true, MScr::Roman), 'ℎ');
        assert_eq!(math_alnum('R', false, false, MScr::DoubleStruck), 'ℝ');
        assert_eq!(math_alnum('α', false, true, MScr::Roman), '𝛼');
        assert_eq!(math_alnum('2', false, true, MScr::Roman), '2');
        assert_eq!(math_alnum('A', true, false, MScr::Roman), '𝐀');
    }

    #[test]
    fn hostile_trees_never_panic() {
        use wordcraft_doc::math::*;
        // Deep nesting.
        let mut n = vec![MNode::Run(MRun::new("x"))];
        for _ in 0..500 {
            n = vec![MNode::Frac { kind: FracKind::Bar, num: n.clone(), den: Vec::new() }];
        }
        let _ = layout_equation(&Math { nodes: n, ..Default::default() }, "", &rc(), true, 400.0, &mut 0);
        // Empty everything.
        let empty = vec![
            MNode::Nary { chr: '\0', lim_loc: None, grow: true, sub_hide: false, sup_hide: false, sub: vec![], sup: vec![], e: vec![] },
            MNode::Delim { beg: Some('\u{10FFFF}'), end: None, sep: None, grow: true, shp_match: false, elems: vec![] },
            MNode::Matrix { rows: vec![vec![]], col_jc: vec![] },
            MNode::EqArr { rows: vec![] },
            MNode::Run(MRun { text: String::new(), size: Some(f32::NAN), ..Default::default() }),
        ];
        let m = layout_equation(&Math { nodes: empty, ..Default::default() }, "", &rc(), true, 400.0, &mut 0);
        assert!(m.width.is_finite() && m.ascent.is_finite());
    }
}
