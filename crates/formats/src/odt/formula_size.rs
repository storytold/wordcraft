//! A rough size for an equation written as a formula object: LibreOffice scales a formula to its
//! frame, so the frame should be about the size the formula takes. A box per node (width, height
//! above and below the baseline, in ems of 12 pt text), sized by its structure.

use wordcraft_doc::math::{MAX_DEPTH, MNode};

/// Width, ascent and descent, in ems.
#[derive(Clone, Copy, Default)]
struct Box3 {
    w: f32,
    asc: f32,
    desc: f32,
}

impl Box3 {
    fn h(self) -> f32 {
        self.asc + self.desc
    }
    fn scaled(self, k: f32) -> Box3 {
        Box3 { w: self.w * k, asc: self.asc * k, desc: self.desc * k }
    }
}

/// Script and limit size relative to the base.
const SMALL: f32 = 0.7;

fn row(nodes: &[MNode], depth: usize) -> Box3 {
    if depth > MAX_DEPTH {
        return Box3::default();
    }
    nodes.iter().map(|n| node(n, depth + 1)).fold(Box3 { w: 0.0, asc: 0.75, desc: 0.25 }, |a, b| Box3 {
        w: a.w + b.w,
        asc: a.asc.max(b.asc),
        desc: a.desc.max(b.desc),
    })
}

fn text(s: &str) -> Box3 {
    // Operators take room on both sides.
    let w: f32 = s
        .chars()
        .map(|c| {
            if "+-−=<>≤≥±×÷→←≠≈".contains(c) {
                1.0
            } else if c.is_whitespace() {
                0.25
            } else {
                0.55
            }
        })
        .sum();
    Box3 { w, asc: 0.75, desc: 0.25 }
}

fn node(n: &MNode, depth: usize) -> Box3 {
    let r = |a: &[MNode]| row(a, depth);
    match n {
        MNode::Run(run) => text(&run.text),
        MNode::Frac { num, den, .. } => {
            let (a, b) = (r(num), r(den));
            Box3 { w: a.w.max(b.w) + 0.3, asc: a.h() + 0.35, desc: b.h() - 0.15 }
        }
        MNode::Script { base, sub, sup, .. } => {
            let b = r(base);
            let (sb, sp) = (r(sub).scaled(SMALL), r(sup).scaled(SMALL));
            let asc = if sup.is_empty() { b.asc } else { b.asc.max(sp.h() + 0.35) };
            let desc = if sub.is_empty() { b.desc } else { b.desc.max(sb.h() - 0.1) };
            Box3 { w: b.w + sb.w.max(sp.w) + 0.1, asc, desc }
        }
        MNode::Rad { deg, e, .. } => {
            let b = r(e);
            Box3 { w: b.w + 0.8 + r(deg).scaled(0.5).w, asc: b.asc + 0.2, desc: b.desc }
        }
        MNode::Nary { sub, sup, e, .. } => {
            let b = r(e);
            let (sb, sp) = (r(sub).scaled(SMALL), r(sup).scaled(SMALL));
            Box3 {
                w: 1.0 + sb.w.max(sp.w) + b.w + 0.2,
                asc: b.asc.max(1.0 + if sup.is_empty() { 0.0 } else { sp.h() }),
                desc: b.desc.max(0.4 + if sub.is_empty() { 0.0 } else { sb.h() }),
            }
        }
        MNode::Delim { elems, .. } => {
            let inner = elems.iter().map(|a| r(a)).fold(Box3 { w: 0.0, asc: 0.75, desc: 0.25 }, |a, b| Box3 {
                w: a.w + b.w + 0.3,
                asc: a.asc.max(b.asc),
                desc: a.desc.max(b.desc),
            });
            Box3 { w: inner.w + 0.6, ..inner }
        }
        MNode::Func { name, e } => {
            let (a, b) = (r(name), r(e));
            Box3 { w: a.w + b.w + 0.2, asc: a.asc.max(b.asc), desc: a.desc.max(b.desc) }
        }
        MNode::Lim { upper, e, lim } => {
            let (b, l) = (r(e), r(lim).scaled(SMALL));
            let w = b.w.max(l.w);
            if *upper { Box3 { w, asc: b.asc + l.h(), desc: b.desc } } else { Box3 { w, asc: b.asc, desc: b.desc + l.h() } }
        }
        MNode::Acc { e, .. } => {
            let b = r(e);
            Box3 { asc: b.asc + 0.3, ..b }
        }
        MNode::Bar { top, e } | MNode::GroupChr { top, e, .. } => {
            let b = r(e);
            if *top { Box3 { asc: b.asc + 0.3, ..b } } else { Box3 { desc: b.desc + 0.3, ..b } }
        }
        MNode::BorderBox { e, .. } | MNode::Boxed { e } => {
            let b = r(e);
            Box3 { w: b.w + 0.4, asc: b.asc + 0.2, desc: b.desc + 0.2 }
        }
        MNode::EqArr { rows } => stack(rows.iter().map(|a| r(a))),
        MNode::Matrix { rows, .. } => {
            let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
            let w: f32 = (0..cols).map(|c| rows.iter().filter_map(|row| row.get(c)).map(|a| r(a).w).fold(0.0, f32::max)).sum::<f32>()
                + 0.6 * cols.saturating_sub(1) as f32;
            let s =
                stack(rows.iter().map(|row| {
                    row.iter().map(|a| r(a)).fold(Box3::default(), |a, b| Box3 { w: 0.0, asc: a.asc.max(b.asc), desc: a.desc.max(b.desc) })
                }));
            Box3 { w, ..s }
        }
        MNode::Phant { show, zero_wid, zero_asc, zero_desc, e } => {
            let b = r(e);
            if !*show && *zero_wid && *zero_asc && *zero_desc {
                return Box3::default();
            }
            Box3 { w: if *zero_wid { 0.0 } else { b.w }, asc: if *zero_asc { 0.0 } else { b.asc }, desc: if *zero_desc { 0.0 } else { b.desc } }
        }
    }
}

/// Rows one under another, centred on the math axis.
fn stack(rows: impl Iterator<Item = Box3>) -> Box3 {
    let (mut w, mut h, mut n) = (0.0f32, 0.0f32, 0usize);
    for b in rows {
        w = w.max(b.w);
        h += b.h();
        n += 1;
    }
    let h = h + 0.2 * n.saturating_sub(1) as f32;
    Box3 { w, asc: h / 2.0 + 0.25, desc: h / 2.0 - 0.25 }
}

/// Width and height in points of the equation `linear` set in 12 pt text.
pub(super) fn size(linear: &str) -> (f32, f32) {
    let b = row(&wordcraft_doc::math::parse_linear(linear), 0);
    let fit = |v: f32| if v.is_finite() { v } else { 1.0 };
    ((fit(b.w) * 12.0 + 2.0).clamp(4.0, 1584.0), (fit(b.h()) * 12.0 + 2.0).clamp(8.0, 1584.0))
}

#[cfg(test)]
mod tests {
    use super::size;

    #[test]
    fn structure_makes_room() {
        let (xw, xh) = size("x");
        let (fw, fh) = size("(a+b)/c");
        assert!(fh > xh * 1.6, "a fraction is taller: {fh} vs {xh}");
        assert!(fw > xw * 2.0);
        let (sw, sh) = size("x^2");
        assert!(sw > xw && sh > xh);
        let (_, mh) = size("■(a&b@c&d)");
        assert!(mh > xh * 1.6, "{mh}");
        // Hostile: deep nesting and long text stay bounded.
        let deep = "(".repeat(5000) + &")".repeat(5000);
        let (dw, dh) = size(&deep);
        assert!(dw.is_finite() && dh.is_finite() && dw <= 1584.0 && dh <= 1584.0);
        assert!(size(&"x".repeat(100_000)).0 <= 1584.0);
    }
}
