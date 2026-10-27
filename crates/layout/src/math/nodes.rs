use super::bx::{Bx, Cls, RunPart, St, compose, finalize, hcat};
use super::state::{MAX_NODES, class_of};
use super::*;

impl Ctx {
    pub(super) fn arg(&mut self, a: &[MNode], st: St, depth: usize) -> Bx {
        let atoms = self.row(a, st, depth, 0);
        let em = self.em(st);
        let mut b = compose(atoms.into_iter().map(|(b, _)| b).collect(), st, em);
        finalize(&mut b, &self.path, em);
        b
    }

    /// Child argument `c` of the node being laid out; an empty one shows a placeholder.
    pub(super) fn sub(&mut self, c: usize, a: &[MNode], st: St, depth: usize) -> Bx {
        let me = self.node_stack.last().copied().unwrap_or(0);
        self.path.push((me, c));
        let b = if a.is_empty() { self.placeholder(st) } else { self.arg(a, st, depth) };
        self.path.pop();
        b
    }

    /// The dotted box an empty argument shows on screen.
    pub(super) fn placeholder(&mut self, st: St) -> Bx {
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
    pub(super) fn hint(&mut self, st: St) -> Bx {
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
    pub(super) fn row(&mut self, a: &[MNode], st: St, depth: usize, u0: usize) -> Vec<(Bx, usize)> {
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
                    let first = out.len();
                    for part in self.run_atoms(r, st, u, &path) {
                        match part {
                            RunPart::Atom(b) => out.push((b, seg)),
                            RunPart::Align(b) => {
                                seg += 1;
                                out.push((b, seg));
                            }
                        }
                    }
                    if let Some((b, _)) = out.get_mut(first) {
                        b.brk = r.brk;
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

    pub(super) fn run_atoms(&self, r: &MRun, st: St, u0: usize, path: &[(usize, usize)]) -> Vec<RunPart> {
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
                let op = if base_gid == 0 {
                    // The math font lacks the operator: a fallback font's glyph, display-sized.
                    let s = if st.display { size * 1.4 } else { size };
                    self.char_glyphs(*chr, s, 0)
                } else if *grow && target > 0.0 && self.t.is_some() {
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
                } else {
                    self.glyph_id(base_gid, size, self.color)
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
                let g = self.stretch_h(gid, b.w, size).unwrap_or_else(|| self.char_glyphs(*chr, size, 0));
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
}
