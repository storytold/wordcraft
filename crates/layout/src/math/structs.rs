use super::bx::{Bx, Cls, St, hcat};
use super::state::wide_accent;
use super::*;

impl Ctx {
    /// Limits under and over `base` (centred), spaced as for big operators (`big`) or as
    /// accents-like limits.
    pub(super) fn limits(&self, base: Bx, under: Option<Bx>, over: Option<Bx>, ic: f32, st: St, big: bool) -> Bx {
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
    pub(super) fn attach_op(&self, base: Bx, sub: Option<Bx>, sup: Option<Bx>, st: St) -> Bx {
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
    pub(super) fn attach(&self, base: Bx, sub: Option<Bx>, sup: Option<Bx>, st: St, char_base: bool) -> Bx {
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

    pub(super) fn frac(&mut self, kind: FracKind, num: &[MNode], den: &[MNode], st: St, d: usize) -> Bx {
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

    pub(super) fn radical(&mut self, deg: &[MNode], deg_hide: bool, e: &[MNode], st: St, d: usize) -> Bx {
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

    pub(super) fn accent(&mut self, chr: char, e: &[MNode], st: St, d: usize) -> Bx {
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
            self.char_glyphs(chr, size, 0)
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

    pub(super) fn matrix(&mut self, rows: &[Vec<Arg>], col_jc: &[ColJc], st: St, d: usize) -> Bx {
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
        let ncols = cells.iter().map(Vec::len).max().unwrap_or(0);
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
