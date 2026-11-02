use wordcraft_doc::bookmark::{BookmarkContainer, BookmarkPlace};
use wordcraft_doc::para::OBJ;

use super::toc::written_toc_span;
use super::{Block, Blocks, CharProps, InlineObject, MAX_DEPTH, Paragraph, PartRels, Table, W, Writer, tblpr, tcpr, trpr, twips};

/// Find an edge paragraph, counting tables entered so a marker is lifted only to its
/// source container, including when the bordering cell itself starts/ends with a table.
fn edge_para(bl: &[std::sync::Arc<Block>], after: bool, depth: usize) -> Option<(&Paragraph, u8)> {
    let b = if after { bl.last() } else { bl.first() }?;
    match &**b {
        Block::Para(p) => Some((p, 0)),
        Block::Table(t) if depth < MAX_DEPTH => {
            let row = if after { t.rows.last() } else { t.rows.first() }?;
            let cell = if after { row.cells.last() } else { row.cells.first() }?;
            edge_para(&cell.blocks, after, depth + 1).map(|(p, tables)| (p, tables.saturating_add(1)))
        }
        Block::Table(_) => None,
    }
}

fn edge_markers(p: &Paragraph, after: bool) -> &[InlineObject] {
    let count = if after { p.text.chars().rev().take_while(|c| *c == OBJ).count() } else { p.text.chars().take_while(|c| *c == OBJ).count() };
    let count = count.min(p.objects.len());
    // A visible object is content too: a marker moved behind a picture stays inline.
    if after {
        let suffix = p.objects.get(p.objects.len() - count..).unwrap_or(&[]);
        let n = suffix.iter().rev().take_while(|o| o.is_marker()).count();
        suffix.get(suffix.len() - n..).unwrap_or(&[])
    } else {
        let prefix = p.objects.get(..count).unwrap_or(&[]);
        let n = prefix.iter().take_while(|o| o.is_marker()).count();
        prefix.get(..n).unwrap_or(&[])
    }
}

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
            let one = std::slice::from_ref(b);
            let before = self.lift_bookmarks(edge_para(one, false, depth), BookmarkContainer::Block, false);
            let after = self.lift_bookmarks(edge_para(one, true, depth), BookmarkContainer::Block, true);
            self.write_bookmarks(w, &before);
            match &**b {
                Block::Para(p) => {
                    self.inline_controls = plan.inline_in(i);
                    self.para(w, p, rels, top, depth);
                }
                Block::Table(t) => self.table(w, t, rels, depth),
            }
            self.write_bookmarks(w, &after);
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

    fn lift_bookmarks(&mut self, edge: Option<(&Paragraph, u8)>, container: BookmarkContainer, after: bool) -> Vec<(String, bool)> {
        if self.doc.bookmark_attrs.is_empty() {
            return Vec::new();
        }
        let Some((p, tables)) = edge else { return Vec::new() };
        let place = Some(BookmarkPlace { container, after, tables });
        let mut out = Vec::new();
        for o in edge_markers(p, after) {
            let (name, end) = match o {
                InlineObject::BookmarkStart { name } => (name, false),
                InlineObject::BookmarkEnd { name } => (name, true),
                _ => continue,
            };
            let Some(attrs) = self.doc.bookmark_attrs.get(name) else { continue };
            if (if end { attrs.end } else { attrs.start }) == place {
                let n = self.lifted_bookmarks.entry((name.clone(), end)).or_insert(0);
                *n = n.saturating_add(1);
                out.push((name.clone(), end));
            }
        }
        out
    }

    fn write_bookmarks(&mut self, w: &mut W, marks: &[(String, bool)]) {
        for (name, end) in marks {
            self.bookmark(w, name, *end);
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
            let before = self.lift_bookmarks(r.cells.first().and_then(|c| edge_para(&c.blocks, false, depth)), BookmarkContainer::Row, false);
            let after = self.lift_bookmarks(r.cells.last().and_then(|c| edge_para(&c.blocks, true, depth)), BookmarkContainer::Row, true);
            self.write_bookmarks(w, &before);
            for c in &r.controls.open {
                crate::write::sdt::open(w, c);
                open_rows += 1;
            }
            w.open("w:tr", &[]);
            let chg = r.props.fmt_change.as_ref().map(|c| self.change_attrs(c.rev));
            trpr(w, &r.props, chg.as_ref());
            let mut open_cells = 0u32;
            for c in &r.cells {
                let before = self.lift_bookmarks(edge_para(&c.blocks, false, depth), BookmarkContainer::Cell, false);
                let after = self.lift_bookmarks(edge_para(&c.blocks, true, depth), BookmarkContainer::Cell, true);
                self.write_bookmarks(w, &before);
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
                self.write_bookmarks(w, &after);
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
            self.write_bookmarks(w, &after);
        }
        for _ in 0..open_rows {
            crate::write::sdt::close(w);
        }
        w.close("w:tbl");
    }
}
