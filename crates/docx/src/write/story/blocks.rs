use super::toc::written_toc_span;
use super::*;

impl Writer<'_> {
    fn write_trailing_para(&mut self, w: &mut W, depth: usize) {
        // A header with no paragraph of its own before: the watermark goes in this one.
        if depth == 0
            && let Some(run) = self.pending_watermark.take()
        {
            w.raw("<w:p>");
            w.raw(&run);
            w.raw("</w:p>");
        } else {
            w.raw("<w:p/>");
        }
    }

    /// Write a block list. `top` = body level (sections allowed).
    pub fn blocks(&mut self, w: &mut W, bl: &Blocks, rels: &mut PartRels, top: bool, depth: usize) {
        if depth > MAX_DEPTH {
            w.raw("<w:p/>");
            return;
        }
        let plan = crate::write::sdt::plan(bl);
        let toc = if top { written_toc_span(bl) } else { None };
        // Whether the heading's TOC field was written and still needs its end.
        let mut toc_field_open = false;
        for (i, b) in bl.iter().enumerate() {
            for c in plan.opens.get(i).into_iter().flatten() {
                crate::write::sdt::open(w, c);
            }
            self.update_toc_before_block(i, toc, toc_field_open, w);
            match &**b {
                Block::Para(p) => {
                    self.inline_controls = plan.inline_in(i);
                    self.para(w, p, rels, top, depth);
                }
                Block::Table(t) => self.table(w, t, rels, depth),
            }
            self.update_toc_after_block(i, toc, &mut toc_field_open, w);
            for _ in 0..plan.closes.get(i).copied().unwrap_or(0) {
                crate::write::sdt::close(w);
            }
        }
        if depth == 0 {
            self.move_ranges(w, &[]);
        }
        if bl.last().is_none_or(|b| matches!(**b, Block::Table(_))) {
            self.write_trailing_para(w, depth);
        }
    }

    /// The moves (revision, destination?) in `props`.
    pub(super) fn moves_of(&self, props: &CharProps) -> Vec<(u32, bool)> {
        [(props.del, false), (props.ins, true)]
            .into_iter()
            .filter_map(|(r, to)| r.filter(|r| self.doc.move_name(Some(*r)).is_some()).map(|r| (r, to)))
            .collect()
    }

    /// Bring the open move ranges to `want` (ECMA-376 §17.13.5.22–27): end the ranges of moves
    /// that stop here, start those that begin. A range is written around each stretch of a
    /// move's text (and paragraph marks), with the move's name, author and date.
    pub(super) fn move_ranges(&mut self, w: &mut W, want: &[(u32, bool)]) {
        let mut k = 0;
        while k < self.open_moves.len() {
            match self.open_moves.get(k) {
                Some((rev, id, to)) if !want.contains(&(*rev, *to)) => {
                    w.empty(if *to { "w:moveToRangeEnd" } else { "w:moveFromRangeEnd" }, &[("w:id", id)]);
                    self.open_moves.remove(k);
                }
                _ => k += 1,
            }
        }
        for &(rev, to) in want {
            if self.open_moves.iter().any(|(r, _, t)| *r == rev && *t == to) {
                continue;
            }
            let name = self.doc.move_name(Some(rev)).unwrap_or("").to_string();
            let (id, author, date) = self.rev_attrs(rev);
            let tag = if to { "w:moveToRangeStart" } else { "w:moveFromRangeStart" };
            if date.is_empty() {
                w.empty(tag, &[("w:id", &id), ("w:name", &name), ("w:author", &author)]);
            } else {
                w.empty(tag, &[("w:id", &id), ("w:name", &name), ("w:author", &author), ("w:date", &date)]);
            }
            self.open_moves.push((rev, id, to));
        }
    }

    fn table(&mut self, w: &mut W, t: &Table, rels: &mut PartRels, depth: usize) {
        let rows: Vec<_> = t.rows.iter().filter(|r| !r.cells.is_empty()).collect();
        if rows.is_empty() {
            return;
        }
        w.open("w:tbl", &[]);
        let chg = t.props.fmt_change.as_ref().map(|c| self.change_attrs(c.rev));
        tblpr(w, &t.props, chg.as_ref());
        w.open("w:tblGrid", &[]);
        let cols = t.cols().clamp(1, 63);
        for g in 0..cols {
            let gw = t.grid.get(g).copied().unwrap_or(72.0);
            w.empty("w:gridCol", &[("w:w", &twips(gw.max(0.0)))]);
        }
        w.close("w:tblGrid");
        // Controls around rows and cells (repeating sections): opens and closes balanced
        // whatever editing left, the ones still open closed at the end.
        let mut open_rows = 0u32;
        for r in rows {
            for c in &r.controls.open {
                crate::write::sdt::open(w, c);
                open_rows += 1;
            }
            w.open("w:tr", &[]);
            let chg = r.props.fmt_change.as_ref().map(|c| self.change_attrs(c.rev));
            trpr(w, &r.props, chg.as_ref());
            let mut open_cells = 0u32;
            for c in &r.cells {
                for ctl in &c.controls.open {
                    crate::write::sdt::open(w, ctl);
                    open_cells += 1;
                }
                w.open("w:tc", &[]);
                let chg = c.props.fmt_change.as_ref().map(|c| self.change_attrs(c.rev));
                tcpr(w, &c.props, chg.as_ref());
                self.blocks(w, &c.blocks, rels, false, depth + 1);
                w.close("w:tc");
                let n = c.controls.close.min(open_cells);
                for _ in 0..n {
                    crate::write::sdt::close(w);
                }
                open_cells -= n;
            }
            for _ in 0..open_cells {
                crate::write::sdt::close(w);
            }
            w.close("w:tr");
            let n = r.controls.close.min(open_rows);
            for _ in 0..n {
                crate::write::sdt::close(w);
            }
            open_rows -= n;
        }
        for _ in 0..open_rows {
            crate::write::sdt::close(w);
        }
        w.close("w:tbl");
    }
}
