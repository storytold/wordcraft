use super::bx::{Bx, Cls, RunPart, St, classes, compose, finalize, gaps_between, hcat};
use super::state::op_char;
use super::{Arg, BrkBin, BrkBinSub, Ctx, MNode, MRun, MSty, MathProps, split_number};

impl Ctx {
    /// Rows stacked and centred on the math axis; `gap`: least space between rows, `min_skip`:
    /// least baseline-to-baseline distance.
    pub(super) fn stack_rows(&self, rows: Vec<(Bx, f32)>, st: St, gap: f32, min_skip: f32) -> (Bx, Vec<f32>) {
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

    pub(super) fn eq_arr(&mut self, rows: &[Arg], st: St, d: usize) -> Bx {
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
        let ncols = segs_rows.iter().map(Vec::len).max().unwrap_or(1);
        // Rows without `&` are centred below, so they don't widen the aligned columns.
        let mut col_w = vec![0.0f32; ncols];
        let mut lone_w = 0.0f32;
        for r in &segs_rows {
            if let [b] = r.as_slice() {
                lone_w = lone_w.max(b.w);
                continue;
            }
            for (i, b) in r.iter().enumerate() {
                if let Some(c) = col_w.get_mut(i) {
                    *c = c.max(b.w);
                }
            }
        }
        let cols: f32 = col_w.iter().sum();
        let total = cols.max(lone_w);
        let mut placed = Vec::new();
        for (r, path) in segs_rows.into_iter().zip(row_paths) {
            let mut row = Bx::default();
            let mut x = 0.0f32;
            // A row without `&` is centred in the array, as Word does (#2617).
            let single = r.len() == 1;
            for (i, b) in r.into_iter().enumerate() {
                let cw = col_w.get(i).copied().unwrap_or(0.0);
                let bw = b.w;
                let dx = if single || i % 2 == 1 { 0.0 } else { cw - bw };
                row.put(b, x + dx, 0.0);
                x += if single { bw } else { cw };
                row.w = row.w.max(x);
            }
            let rx = (total - if single { row.w } else { cols }) / 2.0;
            finalize(&mut row, &path, em);
            placed.push((row, rx.max(0.0)));
        }
        let (mut out, bases) = self.stack_rows(placed, st, em * 0.1, em * 1.1);
        out.w = out.w.max(total);
        (out, bases)
    }

    /// A display equation with numbers (`#`): the equation centred on the line, each row's
    /// number at the right margin. Automatic numbers (`#` alone) count up from `counter`.
    pub(super) fn numbered(&mut self, rows: &[Arg], st: St, avail: f32, counter: &mut u32) -> Bx {
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

    /// A display equation, broken into lines where it is wider than `avail` or a run asks for a
    /// manual break (`m:brk`). Breaks fall only at top-level binary operators and relations (never
    /// inside a fraction, radical, script or matrix), before or after the operator as `m:brkBin`
    /// says; lines are filled greedily and a term wider than the line overflows it. The first line
    /// starts at the left; the rest are indented by `m:wrapIndent`, set flush right with
    /// `m:wrapRight`, or aligned at an operator of the first line (`m:alnAt`).
    pub(super) fn wrapped(&mut self, a: &[MNode], st: St, avail: f32, mp: &MathProps) -> Bx {
        let em = self.em(st);
        let atoms: Vec<Bx> = self.row(a, st, 0, 0).into_iter().map(|(b, _)| b).collect();
        let gaps = gaps_between(&atoms, st, em);
        let total: f32 = atoms.iter().map(|b| b.w).sum::<f32>() + gaps.iter().sum::<f32>();
        let manual = atoms.iter().skip(1).any(|b| b.brk.is_some());
        if atoms.len() < 2 || (!manual && total <= avail + 0.01) {
            let mut b = compose(atoms, st, em);
            finalize(&mut b, &self.path, em);
            return b;
        }
        let cls = classes(&atoms);
        let n = atoms.len();
        let is_op = |i: usize| -> bool {
            atoms.get(i).is_some_and(|b| b.glyph.is_some() && !b.transparent)
                && cls.get(i).is_some_and(|c| matches!(c.0, Cls::Bin | Cls::Rel) && c.0 == c.1)
        };
        // Chunk starts (after the first): the atom index → forced (`Some(alnAt)`) and the
        // operator repeated at the start of the line it begins.
        let mut starts: std::collections::BTreeMap<usize, (Option<u8>, Option<usize>)> = std::collections::BTreeMap::new();
        for i in 1..n {
            let op = is_op(i);
            let at = if op && mp.brk_bin != BrkBin::Before { i + 1 } else { i };
            if at == 0 || at >= n {
                continue;
            }
            let repeat = (op && mp.brk_bin == BrkBin::Repeat).then_some(i);
            let forced = atoms.get(i).and_then(|b| b.brk);
            if op || forced.is_some() {
                let e = starts.entry(at).or_insert((None, None));
                if forced.is_some() {
                    e.0 = forced;
                }
                if repeat.is_some() {
                    e.1 = repeat;
                }
            }
        }
        // Chunks: [s, e) with their width inside and the gap before them.
        let mut bounds: Vec<usize> = vec![0];
        bounds.extend(starts.keys().copied());
        bounds.push(n);
        let width_of = |s: usize, e: usize| -> f32 {
            (s..e).map(|i| atoms.get(i).map_or(0.0, |b| b.w) + if i > s { gaps.get(i).copied().unwrap_or(0.0) } else { 0.0 }).sum()
        };
        // The operator glyphs a repeated operator shows at the end of one line and the start of
        // the next.
        let repeats: std::collections::HashMap<usize, (Option<Bx>, Bx)> = starts
            .values()
            .filter_map(|v| v.1)
            .filter_map(|i| {
                let b = atoms.get(i)?;
                let (end, start) = match op_char(b) {
                    Some('−' | '-') => match mp.brk_bin_sub {
                        BrkBinSub::MinusMinus => ('−', '−'),
                        BrkBinSub::MinusPlus => ('−', '+'),
                        BrkBinSub::PlusMinus => ('+', '−'),
                    },
                    Some(c) => (c, c),
                    None => return None,
                };
                let end = (Some(end) != op_char(b)).then(|| self.op_glyph(b, end, st)).flatten();
                Some((i, (end, self.op_glyph(b, start, st)?)))
            })
            .collect();
        let indent = if mp.wrap_indent.is_finite() { mp.wrap_indent.clamp(0.0, 1584.0) } else { 0.0 }.min(avail * 0.5);
        // Greedy fill: lines as lists of chunks (index into `bounds`).
        let mut lines: Vec<(usize, usize)> = Vec::new();
        let mut cur_w = 0.0f32;
        let mut auto = false;
        for c in 0..bounds.len().saturating_sub(1) {
            let (s, e) = (bounds.get(c).copied().unwrap_or(0), bounds.get(c + 1).copied().unwrap_or(n));
            let w = width_of(s, e);
            let lead = gaps.get(s).copied().unwrap_or(0.0);
            let info = starts.get(&s).copied();
            let copy_w = info.and_then(|v| v.1).and_then(|i| repeats.get(&i)).map_or(0.0, |r| r.1.w + lead);
            let line_avail = if lines.len() <= 1 || mp.wrap_right { avail } else { avail - indent };
            match lines.last_mut() {
                Some(l) if info.is_some_and(|v| v.0.is_none()) && cur_w + lead + w <= line_avail + 0.01 => {
                    l.1 = c + 1;
                    cur_w += lead + w;
                }
                Some(_) if info.is_some_and(|v| v.0.is_none()) => {
                    auto = true;
                    lines.push((c, c + 1));
                    cur_w = copy_w + w;
                }
                _ => {
                    lines.push((c, c + 1));
                    cur_w = copy_w + w;
                }
            }
        }
        // Build each line's box.
        let ops: Vec<bool> = (0..n).map(is_op).collect();
        let path = self.path.clone();
        let mut atoms: Vec<Option<Bx>> = atoms.into_iter().map(Some).collect();
        let mut boxes: Vec<(Bx, Option<u8>)> = Vec::with_capacity(lines.len());
        let mut ops_x: Vec<f32> = Vec::new();
        for (li, &(c0, c1)) in lines.iter().enumerate() {
            let (s, e) = (bounds.get(c0).copied().unwrap_or(0), bounds.get(c1).copied().unwrap_or(n));
            let info = starts.get(&s).copied();
            let next = lines.get(li + 1).and_then(|l| bounds.get(l.0)).and_then(|s| starts.get(s)).and_then(|v| v.1);
            let mut row = Bx::default();
            let mut x = 0.0f32;
            let copy = info.and_then(|v| v.1).and_then(|i| repeats.get(&i)).map(|r| r.1.clone());
            let has_copy = copy.is_some();
            if let Some(cb) = copy {
                let w = cb.w;
                row.put(cb, 0.0, 0.0);
                x = w;
            }
            for i in s..e {
                let Some(mut b) = atoms.get_mut(i).and_then(Option::take) else { continue };
                if i > s || has_copy {
                    x += gaps.get(i).copied().unwrap_or(0.0);
                }
                if next == Some(i)
                    && let Some((Some(end), _)) = repeats.get(&i)
                {
                    let mut r = end.clone();
                    let k = if b.w > 0.0 { r.w / b.w } else { 1.0 };
                    r.slots = std::mem::take(&mut b.slots)
                        .into_iter()
                        .map(|mut sl| {
                            sl.x *= k;
                            sl
                        })
                        .collect();
                    b = r;
                }
                if li == 0 && ops.get(i).copied().unwrap_or(false) {
                    ops_x.push(x);
                }
                let w = b.w;
                row.put(b, x, 0.0);
                x += w;
            }
            row.w = x;
            finalize(&mut row, &path, em);
            boxes.push((row, info.and_then(|v| v.0)));
        }
        let widest = boxes.iter().map(|(b, _)| b.w).fold(0.0f32, f32::max);
        let right = if auto { avail.max(widest) } else { widest };
        let mut placed = Vec::with_capacity(boxes.len());
        for (li, (b, aln)) in boxes.into_iter().enumerate() {
            let x = if li == 0 {
                0.0
            } else if let Some(ax) = aln.filter(|a| *a > 0).and_then(|a| ops_x.get(a as usize - 1)) {
                *ax
            } else if mp.wrap_right {
                (right - b.w).max(0.0)
            } else {
                indent
            };
            placed.push((b, x));
        }
        // Stack the lines down from the first one's baseline.
        let (gap, skip) = (em * 0.2, em * 1.25);
        let mut out = Bx::default();
        let mut y = 0.0f32;
        let mut prev_d: Option<f32> = None;
        for (b, x) in placed {
            if let Some(pd) = prev_d {
                y -= (pd + gap + b.a).max(skip);
            }
            prev_d = Some(b.d);
            out.w = out.w.max(x + b.w);
            out.put(b, x, y);
        }
        out.w = out.w.max(right);
        out
    }
}
