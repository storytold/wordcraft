use super::bx::{Bx, St};
use super::state::fallback;
use super::*;

impl Ctx {
    pub(super) fn scale(&self, level: u8) -> f32 {
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

    pub(super) fn size(&self, st: St) -> f32 {
        self.base * self.scale(st.level)
    }

    /// A constant in points at the style's size.
    pub(super) fn k(&self, c: C, st: St) -> f32 {
        let size = self.size(st);
        match &self.t {
            Some(t) => t.get(c) * size / self.upem,
            None => fallback(c) * size,
        }
    }

    pub(super) fn axis(&self, st: St) -> f32 {
        self.k(C::AxisHeight, st)
    }

    pub(super) fn em(&self, st: St) -> f32 {
        self.size(st)
    }

    /// Glyph run for `text` in `face` at `size`: a box with ink extents.
    pub(super) fn glyphs(&self, face: FaceRef, text: &str, size: f32, level: u8, color: Rgb, synth: (bool, bool)) -> Bx {
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
        // Character boundaries: where the first glyph of each character starts. The glyph for a
        // boundary is never before the previous boundary's, so one walk of the glyphs finds all.
        let mut cuts = Vec::with_capacity(text.len() + 1);
        let (mut j, mut gx) = (0usize, 0.0f32);
        for (cb, _) in text.char_indices() {
            while let Some(g) = shaped.get(j)
                && g.cluster < cb
            {
                crate::work::steps(1);
                gx += g.x_advance as f32 * k;
                j += 1;
            }
            cuts.push(if j < shaped.len() { gx } else { x });
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

    /// `c` in the math font, or in a fallback font when the math font lacks it (an n-ary
    /// operator, delimiter or accent the font doesn't cover would otherwise draw as a missing
    /// glyph box, #1273).
    pub(super) fn char_glyphs(&self, c: char, size: f32, level: u8) -> Bx {
        let face = if self.face.covers(c) {
            self.face
        } else {
            FontDb::global().fallback_for(c, self.face.id()).map(|f| FaceRef::of(&f)).unwrap_or(self.face)
        };
        self.glyphs(face, &c.to_string(), size, level, self.color, (false, false))
    }

    /// A single glyph by id (size variants, assembly parts).
    pub(super) fn glyph_id(&self, gid: u32, size: f32, color: Rgb) -> Bx {
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
    pub(super) fn text_atom(
        &self,
        face: FaceRef,
        text: &str,
        size: f32,
        level: u8,
        color: Rgb,
        synth: (bool, bool),
        path: &[(usize, usize)],
        k0: usize,
    ) -> Bx {
        let mut b = self.glyphs(face, text, size, level, color, synth);
        let cuts = std::mem::take(&mut b.cuts);
        for (j, x) in cuts.iter().enumerate() {
            b.slot(path, k0 + j, *x);
        }
        b
    }

    /// A vertical size variant or assembly of `gid` at least `target` tall (`None` without a
    /// `MATH` table or construction for the glyph).
    pub(super) fn stretch_v(&self, gid: u32, target: f32, size: f32) -> Option<Bx> {
        let t = self.t.as_ref()?;
        let con = t.vert.get(&gid)?;
        self.stretch(con, t, target, size, true)
    }

    pub(super) fn stretch_h(&self, gid: u32, target: f32, size: f32) -> Option<Bx> {
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
    pub(super) fn delimiter(&self, c: char, a: f32, d: f32, st: St, grow: bool, exact: bool) -> Bx {
        let size = self.size(st);
        let axis = self.axis(st);
        let mut glyph = self.char_glyphs(c, size, 0);
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

    /// Operator `c` drawn like the operator atom `like` (its face, size and colour), without
    /// caret slots.
    pub(super) fn op_glyph(&self, like: &Bx, c: char, st: St) -> Option<Bx> {
        let MItem::Glyphs { face, size, color, synth_bold, synth_italic, .. } = like.items.iter().find(|i| matches!(i, MItem::Glyphs { .. }))? else {
            return None;
        };
        let face = if face.covers(c) { *face } else { self.face };
        let mut b = self.glyphs(face, &c.to_string(), *size, st.level, *color, (*synth_bold, *synth_italic));
        b.cuts.clear();
        Some(b.cls(like.first))
    }
}
