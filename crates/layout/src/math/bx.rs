use super::*;

/// Layout style: script level (0 = text size, 1 = script, 2 = script-script), display or inline,
/// cramped (superscripts lowered: inside radicals, denominators, subscripts).
#[derive(Clone, Copy, Debug)]
pub(super) struct St {
    pub(super) level: u8,
    pub(super) display: bool,
    pub(super) cramped: bool,
    /// Letters default to upright (function names).
    pub(super) upright: bool,
}

impl St {
    pub(super) fn script(self) -> St {
        St { level: (self.level + 1).min(2), display: false, ..self }
    }
    pub(super) fn cramp(self) -> St {
        St { cramped: true, ..self }
    }
    /// Numerator / denominator style.
    pub(super) fn frac(self) -> St {
        if self.display { St { display: false, ..self } } else { self.script() }
    }
}

/// TeX atom classes (spacing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum Cls {
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
pub(super) struct Bx {
    pub(super) w: f32,
    pub(super) a: f32,
    pub(super) d: f32,
    pub(super) items: Vec<MItem>,
    /// Italic correction after the box (superscripts move right by it).
    pub(super) ic: f32,
    pub(super) first: Cls,
    pub(super) last: Cls,
    /// The box is one glyph: (face, glyph, top-accent attachment x).
    pub(super) glyph: Option<(FaceRef, u32, f32)>,
    /// Takes no part in inter-atom spacing (spaces).
    pub(super) transparent: bool,
    /// Caret positions inside, relative to the box.
    pub(super) slots: Vec<Slot>,
    /// Text atoms: x of each character boundary (one more than the characters).
    pub(super) cuts: Vec<f32>,
    /// A manual line break before this atom (`m:brk`; the value is its `m:alnAt`).
    pub(super) brk: Option<u8>,
}

impl Bx {
    pub(super) fn cls(mut self, c: Cls) -> Bx {
        self.first = c;
        self.last = c;
        self
    }
    /// Draw `o` at (dx, dy) into this box (extents grow; width is the caller's business).
    pub(super) fn put(&mut self, o: Bx, dx: f32, dy: f32) {
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
    pub(super) fn slot(&mut self, path: &[(usize, usize)], off: usize, x: f32) {
        self.slots.push(Slot { path: path.to_vec(), off, x, y: 0.0, a: 0.0, d: 0.0 });
    }
    pub(super) fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgb) {
        if w > 0.0 && h > 0.0 {
            self.items.push(MItem::Rect { x, y, w, h, color });
            self.a = self.a.max(y + h);
            self.d = self.d.max(-y);
        }
    }
}

pub(super) enum RunPart {
    Atom(Bx),
    /// An `&` alignment mark (a zero-width atom holding its caret slots).
    Align(Bx),
}

/// Give the caret slots of argument `path` in `b` the argument's height.
pub(super) fn finalize(b: &mut Bx, path: &[(usize, usize)], em: f32) {
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
pub(super) fn gaps_between(atoms: &[Bx], st: St, em: f32) -> Vec<f32> {
    let cls = classes(atoms);
    let solid: Vec<usize> = (0..atoms.len()).filter(|i| atoms.get(*i).is_some_and(|b| !b.transparent)).collect();
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

/// Each atom's classes (first, last) in context: a binary operator where TeX makes it ordinary
/// (a unary minus) becomes ordinary.
pub(super) fn classes(atoms: &[Bx]) -> Vec<(Cls, Cls)> {
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
    cls
}

/// Atoms side by side with inter-atom spacing.
pub(super) fn compose(mut atoms: Vec<Bx>, st: St, em: f32) -> Bx {
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
pub(super) fn hcat(parts: Vec<Bx>) -> Bx {
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
