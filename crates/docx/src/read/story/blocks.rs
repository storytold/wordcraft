use super::{
    Arc, Block, Blocks, Cell, CharProps, El, InlineObject, MAX_COLS, MAX_ROWS, MAX_TABLE_DEPTH, Node, NumChange, PB, Paragraph, Phase, PropChange,
    Reader, Rels, RevisionKind, Row, Run, RunCtx, StoryCtx, Table, children_of_choice, clean_text, collect, last_para_mut, para_block,
    placeholder_of, sectpr, sheet_from_page, tcpr, trpr,
};

impl Reader<'_> {
    /// Read a story's blocks from a container element (`w:body`, `w:hdr`, `w:tc`, `w:txbxContent`…).
    pub fn read_blocks(&mut self, sc: &mut StoryCtx, parent: &El, rels: &Rels, out: &mut Blocks, depth: usize) {
        if depth > crate::xml::MAX_DEPTH {
            return;
        }
        for e in parent.els() {
            match e.name.as_str() {
                "w:p" => {
                    let p = self.read_para(sc, e, rels, depth);
                    // A framed drop-cap paragraph merges into the paragraph it starts.
                    if let Some(Block::Para(prev)) = out.last().map(|b| &**b)
                        && prev.props.drop_cap.is_some()
                        && prev.section.is_none()
                        && prev.text.len() <= 16
                        && p.props.drop_cap.is_none()
                        && let Some(Block::Para(mut first)) = out.pop().map(|b| (*b).clone())
                    {
                        let mut props = p.props.clone();
                        props.drop_cap = first.props.drop_cap;
                        props.drop_cap_margin = first.props.drop_cap_margin;
                        props.drop_cap_distance = first.props.drop_cap_distance;
                        // A letter in a font of its own: the drop cap's font, the letter itself
                        // in the text's (so taking the drop cap away gives the letter its font back).
                        let cap_font = first.runs.first().and_then(|r| r.props.font.clone());
                        let text_font = p.runs.first().and_then(|r| r.props.font.clone());
                        if cap_font.is_some() && cap_font != text_font {
                            for r in &mut first.runs {
                                r.props.font = text_font.clone();
                            }
                            props.drop_cap_font = cap_font;
                        }
                        first.append(p);
                        first.props = props;
                        out.push(para_block(first));
                        continue;
                    }
                    out.push(para_block(p));
                }
                "w:tbl" => {
                    if depth / 4 >= MAX_TABLE_DEPTH {
                        out.push(para_block(Paragraph::with_text(&clean_text(&e.deep_text()), CharProps::default())));
                    } else if let Some(t) = self.read_table(sc, e, rels, depth) {
                        out.push(Arc::new(Block::Table(t)));
                    }
                }
                "w:sdt" => self.read_block_sdt(sc, e, rels, out, depth),
                "w:customXml" | "w:smartTag" | "w:ins" | "w:moveTo" => self.read_blocks(sc, e, rels, out, depth + 1),
                // Math is allowed straight in a story (EG_BlockLevelElts takes EG_RunLevelElements,
                // which takes EG_MathContent): it gets a paragraph of its own (#1273).
                "m:oMathPara" | "m:oMath" => {
                    let wrapped = El { name: "w:p".into(), attrs: Vec::new(), kids: vec![Node::El(e.clone())] };
                    let p = self.read_para(sc, &wrapped, rels, depth);
                    out.push(para_block(p));
                }
                "mc:AlternateContent" => {
                    if let Some(c) = children_of_choice(e) {
                        self.read_blocks(sc, c, rels, out, depth + 1);
                    }
                }
                "w:bookmarkStart" | "w:bookmarkEnd" | "w:commentRangeStart" | "w:commentRangeEnd" | "w:permStart" | "w:permEnd" => {
                    if let Some(o) = self.marker(e) {
                        sc.pending.push(o);
                    }
                }
                "w:moveFromRangeStart" | "w:moveFromRangeEnd" | "w:moveToRangeStart" | "w:moveToRangeEnd" => self.move_range(e),
                _ => {}
            }
        }
    }

    /// A block-level content control: its content's blocks, between a start marker at the start
    /// of the first paragraph and an end marker at the end of the last (a table at either end
    /// carries the control instead).
    fn read_block_sdt(&mut self, sc: &mut StoryCtx, e: &El, rels: &Rels, out: &mut Blocks, depth: usize) {
        let Some(content) = e.child("w:sdtContent") else { return };
        let mut ctl = crate::read::sdt::sdt_pr(e.child("w:sdtPr"), true);
        ctl.end_pr_xml = crate::read::sdt::sdt_end_pr(e.child("w:sdtEndPr"));
        ctl.run_props = e.child("w:sdtPr").and_then(|pr| pr.child("w:rPr")).map(|r| self.pc.rpr(r));
        let mut inner = Blocks::new();
        self.read_blocks(sc, content, rels, &mut inner, depth + 1);
        // A table of contents' control is handed to the TOC later, when the writer puts it back
        // (`toc_control_to_toc`).
        if inner.is_empty() {
            out.extend(inner);
            return;
        }
        if ctl.showing_placeholder
            && let Some(Block::Para(p)) = inner.first().map(|b| &**b)
        {
            ctl.placeholder_text = placeholder_of(&p.plain_text());
        }
        if let Some(first) = inner.first_mut() {
            match Arc::make_mut(first) {
                Block::Para(p) => {
                    let props = p.props_of_char(0).clone();
                    let _ = p.insert_object(0, InlineObject::ControlStart { control: Box::new(ctl) }, &props);
                }
                Block::Table(t) => t.controls.open.insert(0, ctl),
            }
        }
        if let Some(last) = inner.last_mut() {
            match Arc::make_mut(last) {
                Block::Para(p) => {
                    let (at, props) = (p.len(), p.runs.last().map(|r| r.props.clone()).unwrap_or_else(|| p.mark.clone()));
                    let _ = p.insert_object(at, InlineObject::ControlEnd, &props);
                }
                Block::Table(t) => t.controls.close = t.controls.close.saturating_add(1),
            }
        }
        out.extend(inner);
    }

    /// Markers left over at the end of a story go to its last paragraph (after a final table:
    /// the last paragraph of its last cell, so a bookmark around the table keeps its end).
    pub fn flush_pending(&mut self, sc: &mut StoryCtx, out: &mut Blocks) {
        if sc.pending.is_empty() {
            return;
        }
        if let Some(p) = last_para_mut(out, 0) {
            for o in sc.pending.drain(..) {
                let props = p.runs.last().map(|r| r.props.clone()).unwrap_or_default();
                p.text.push(wordcraft_doc::para::OBJ);
                p.objects.push(o);
                p.runs.push(Run { len: wordcraft_doc::para::OBJ.len_utf8(), props });
            }
            p.normalize();
        }
        sc.pending.clear();
    }

    pub(super) fn marker(&mut self, e: &El) -> Option<InlineObject> {
        let id = e.attr("w:id").map(str::trim).unwrap_or("").to_string();
        match e.name.as_str() {
            "w:bookmarkStart" => {
                if self.bookmarks.len() >= 100_000 {
                    return None;
                }
                let name = e.attr("w:name").map(str::trim).unwrap_or("").to_string();
                if name.is_empty() || name.len() > 256 {
                    return None;
                }
                self.bookmarks.insert(id, name.clone());
                Some(InlineObject::BookmarkStart { name })
            }
            "w:bookmarkEnd" => self
                .bookmarks
                .get(&id)
                .cloned()
                .or_else(|| e.attr("w:name").map(str::trim).filter(|n| !n.is_empty() && n.len() <= 256).map(str::to_string))
                .map(|name| InlineObject::BookmarkEnd { name }),
            "w:commentRangeStart" => self.comment_id(&id).map(|id| InlineObject::CommentStart { id }),
            "w:permStart" => {
                // Bounded: a hostile file can't grow the map without end.
                if self.perms.len() >= 100_000 {
                    return None;
                }
                let n = self.perms.len() as u32;
                let id = *self.perms.entry(id).or_insert(n);
                let text = |a: &str| e.attr(a).filter(|v| v.len() <= 256).unwrap_or("").to_string();
                let (group, editor) = (text("w:edGrp"), text("w:ed"));
                if group.is_empty() && editor.is_empty() {
                    return None;
                }
                Some(InlineObject::PermStart { id, group, editor })
            }
            "w:permEnd" => self.perms.get(&id).map(|id| InlineObject::PermEnd { id: *id }),
            "w:commentRangeEnd" => {
                let cid = self.comment_id(&id)?;
                self.comments_ended.insert(cid);
                Some(InlineObject::CommentEnd { id: cid })
            }
            _ => None,
        }
    }

    pub(super) fn comment_id(&self, file_id: &str) -> Option<u32> {
        self.comment_map.get(file_id).copied()
    }

    fn read_para(&mut self, sc: &mut StoryCtx, e: &El, rels: &Rels, depth: usize) -> Paragraph {
        let mut pb = PB::default();
        let mut para = Paragraph::new();
        let mut mark_moves: [Option<&El>; 2] = [None, None];
        self.moves.closed_here = [None, None];
        for o in std::mem::take(&mut sc.pending) {
            pb.push_obj(o, &CharProps::default());
        }
        if let Some(ppr) = e.child("w:pPr") {
            let (mut props, mark) = self.pc.ppr(ppr);
            crate::read::props::resolve_level_only(&self.doc.styles, &mut props, "Normal");
            para.props = props;
            para.mark = mark;
            // A tracked paragraph mark (ECMA-376 §17.13.5.16 / §17.13.5.15): `w:ins` / `w:del`
            // in the mark's `w:rPr`.
            if let Some(rpr) = ppr.child("w:rPr") {
                for k in rpr.els() {
                    match k.name.as_str() {
                        "w:ins" => para.mark.ins = Some(self.revision(RevisionKind::Insert, k)),
                        "w:del" => para.mark.del = Some(self.revision(RevisionKind::Delete, k)),
                        // A moved paragraph mark (§17.13.5.21, §17.13.5.24): its move range
                        // usually starts in the paragraph's content, so it is read after that.
                        "w:moveTo" => mark_moves[1] = Some(k),
                        "w:moveFrom" => mark_moves[0] = Some(k),
                        _ => {}
                    }
                }
                para.mark.fmt_change = self.rpr_change(rpr);
            }
            // Tracked formatting changes (§17.13.5.29, §17.13.5.19): the paragraph properties
            // before the change, and a change of the list numbering.
            if let Some(ch) = ppr.child("w:pPrChange") {
                let old = ch
                    .child("w:pPr")
                    .map(|o| {
                        let mut old = self.pc.ppr(o).0;
                        crate::read::props::resolve_level_only(&self.doc.styles, &mut old, "Normal");
                        old.formatting()
                    })
                    .unwrap_or_default();
                para.props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
            }
            if para.props.numbering.is_some()
                && let Some(ch) = ppr.child("w:numPr").and_then(|n| n.child("w:numberingChange"))
            {
                let original = ch.attr("w:original").unwrap_or("").chars().take(256).collect();
                para.props.num_change = Some(Box::new(NumChange { rev: self.revision(RevisionKind::Format, ch), original }));
            }
            if let Some(s) = ppr.child("w:sectPr") {
                para.section = Some(Box::new(self.read_section(s, rels)));
            }
        }
        let ctx = RunCtx::default();
        self.read_inline_children(sc, &mut pb, e, rels, &ctx, depth + 1);
        self.spill_fields(sc, &mut pb);
        if let Some(k) = mark_moves[0] {
            para.mark.del = Some(self.move_revision(false, k, true));
        }
        if let Some(k) = mark_moves[1] {
            para.mark.ins = Some(self.move_revision(true, k, true));
        }
        para.text = pb.text;
        para.runs = pb.runs;
        para.objects = pb.objects;
        // Paragraph boundaries inside an instruction are hidden with the field code.
        // The result paragraph still takes a line, even when its cached text is empty.
        if sc.fields.iter().any(|f| f.phase == Phase::Instr) && para.plain_text().is_empty() {
            para.mark.hidden = Some(true);
        }
        para.normalize();
        para
    }

    /// A tracked run formatting change in `rpr` (`w:rPrChange`, §17.13.5.31): the formatting
    /// before it. A change nested inside it is ignored.
    pub(super) fn rpr_change(&mut self, rpr: &El) -> Option<Box<PropChange<CharProps>>> {
        let ch = rpr.child("w:rPrChange")?;
        let old = ch.child("w:rPr").map(|o| self.pc.rpr(o).formatting()).unwrap_or_default();
        PropChange::boxed(self.revision(RevisionKind::Format, ch), old)
    }

    pub fn read_section(&mut self, s: &El, rels: &Rels) -> wordcraft_doc::SectionProps {
        let mode = self.doc.settings.multiple_pages();
        let (mut sp, refs) = sectpr(s);
        sheet_from_page(&mut sp, s, mode);
        // §17.13.5.32: the section's properties before a tracked change (no header references).
        if let Some(ch) = s.child("w:sectPrChange") {
            let old = ch
                .child("w:sectPr")
                .map(|o| {
                    let mut old = sectpr(o).0;
                    sheet_from_page(&mut old, o, mode);
                    old
                })
                .unwrap_or_default();
            sp.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
        }
        for r in refs {
            let Some(rel) = rels.by_id(&r.rid) else { continue };
            if rel.external {
                continue;
            }
            let target = rel.target.clone();
            let Some(id) = self.load_header_footer(&target, r.footer) else { continue };
            let set = if r.footer { &mut sp.footers } else { &mut sp.headers };
            match r.kind.as_str() {
                "first" => set.first = Some(id),
                "even" => set.even = Some(id),
                _ => set.default = Some(id),
            }
        }
        sp
    }

    // ---- tables ----

    fn read_table(&mut self, sc: &mut StoryCtx, t: &El, rels: &Rels, depth: usize) -> Option<Table> {
        let mut table = Table::default();
        if let Some(p) = t.child("w:tblPr") {
            table.props = self.pc.tblpr(p);
            if let Some(ch) = p.child("w:tblPrChange") {
                let old = ch.child("w:tblPr").map(|o| self.pc.tblpr(o)).unwrap_or_default();
                table.props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
            }
        }
        if let Some(g) = t.child("w:tblGrid") {
            table.grid = g
                .children("w:gridCol")
                .take(MAX_COLS)
                .map(|c| crate::units::tw(c, "w:w").unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT))
                .collect();
        }
        let mut rows = Vec::new();
        collect(t, "w:tr", &mut rows, &mut Vec::new(), 0);
        for (tr, row_controls) in rows.into_iter().take(MAX_ROWS) {
            let mut row = Row { props: tr.child("w:trPr").map(trpr).unwrap_or_default(), cells: Vec::new(), controls: row_controls };
            row.props.exceptions = tr.child("w:tblPrEx").map(|e| Box::new(self.pc.tblpr(e)));
            if let Some(ch) = tr.child("w:trPr").and_then(|p| p.child("w:trPrChange")) {
                let old = ch.child("w:trPr").map(trpr).unwrap_or_default();
                row.props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
            }
            let mut tcs = Vec::new();
            collect(tr, "w:tc", &mut tcs, &mut Vec::new(), 0);
            for (tc, cell_controls) in tcs.into_iter().take(MAX_COLS) {
                let (mut props, hcont) = tc
                    .child("w:tcPr")
                    .map(|p| tcpr(&self.pc, p))
                    .unwrap_or_else(|| (wordcraft_doc::props::CellProps { span: 1, ..Default::default() }, false));
                if let Some(ch) = tc.child("w:tcPr").and_then(|p| p.child("w:tcPrChange")) {
                    let old = ch.child("w:tcPr").map(|o| tcpr(&self.pc, o).0).unwrap_or_default();
                    props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
                }
                if hcont && let Some(prev) = row.cells.last_mut() {
                    prev.props.span = (prev.props.span + props.span.max(1)).min(63);
                    continue;
                }
                let mut blocks = Blocks::new();
                self.read_blocks(sc, tc, rels, &mut blocks, depth + 4);
                self.flush_pending(sc, &mut blocks);
                row.cells.push(Cell { props, blocks, controls: cell_controls });
            }
            if !row.cells.is_empty() {
                table.rows.push(row);
            }
        }
        if table.rows.is_empty() {
            return None;
        }
        if table.grid.is_empty() {
            let cols = table.cols().clamp(1, MAX_COLS);
            table.grid = vec![72.0; cols];
        }
        Some(table)
    }
}
