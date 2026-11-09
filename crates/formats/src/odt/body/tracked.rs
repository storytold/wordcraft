//! Tracked changes (ODF 1.3 §5.5): the changed regions, deleted content and the insertions open
//! around the text.

use super::{Attrs, Body, FBlock, HashMap, Inline, MAX_OPEN_INSERTIONS, MAX_REGIONS, Para, RevMark, RevisionKind, get, local};

/// A `text:changed-region` read from `text:tracked-changes`.
#[derive(Default)]
pub(in crate::odt) struct Region {
    pub(in crate::odt) kind: Option<RevisionKind>,
    /// Sequence number: tells regions by the same author at the same time apart.
    pub(in crate::odt) id: u32,
    pub(in crate::odt) author: String,
    pub(in crate::odt) date: String,
    /// A deletion's deleted paragraphs.
    pub(in crate::odt) blocks: Vec<FBlock>,
}

impl Region {
    pub(in crate::odt) fn mark(&self) -> RevMark {
        RevMark { author: self.author.trim().to_string(), date: self.date.trim().to_string(), id: Some(self.id) }
    }
}

/// Reader state saved while a deletion's content is read into its own container.
pub(in crate::odt) struct DelState {
    pub(in crate::odt) containers: usize,
    pub(in crate::odt) tables: usize,
    pub(in crate::odt) cells: usize,
    pub(in crate::odt) lists_saved: usize,
    pub(in crate::odt) fmt: usize,
    pub(in crate::odt) lists: Vec<(Option<String>, bool)>,
    pub(in crate::odt) item_used: Vec<bool>,
    /// Insertions open inside the deleted content (text inserted, then deleted).
    pub(in crate::odt) ins: Vec<(String, RevMark)>,
}

/// A `text:change-start` (`start`) or `text:change-end` (attributes `a`) among the changes of
/// `kind` open (insertions, or format changes): unknown ids, other kinds' marks, ends without a
/// start and starts past [`MAX_OPEN_INSERTIONS`] are ignored.
pub(in crate::odt) fn change_mark(regions: &HashMap<String, Region>, open: &mut Vec<(String, RevMark)>, kind: RevisionKind, start: bool, a: &Attrs) {
    let Some(id) = get(a, "text:change-id") else { return };
    if !start {
        if let Some(k) = open.iter().position(|(o, _)| o == id) {
            open.remove(k);
        }
        return;
    }
    if let Some(r) = regions.get(id).filter(|r| r.kind == Some(kind))
        && open.len() < MAX_OPEN_INSERTIONS
        && !open.iter().any(|(o, _)| o == id)
    {
        open.push((id.to_string(), r.mark()));
    }
}

impl Body<'_> {
    /// An element inside `text:tracked-changes`, outside deleted content.
    pub(in crate::odt) fn tracked_start(&mut self, q: &str, a: &Attrs) {
        match local(q) {
            "changed-region" if self.region.is_none() && self.region_count < MAX_REGIONS => {
                // ODF 1.3 names it by `xml:id`; `text:id` is the older name, kept beside it.
                let id = get(a, "text:id").or_else(|| get(a, "xml:id")).unwrap_or_default().to_string();
                let r = Region { id: self.region_count as u32, ..Default::default() };
                self.region_count += 1;
                self.region = Some((id, r));
            }
            "insertion" | "deletion" | "format-change" if self.region.as_ref().is_some_and(|(_, r)| r.kind.is_none()) => {
                let kind = match local(q) {
                    "insertion" => RevisionKind::Insert,
                    "deletion" => RevisionKind::Delete,
                    _ => RevisionKind::Format,
                };
                if let Some((_, r)) = &mut self.region {
                    r.kind = Some(kind);
                }
                if kind == RevisionKind::Delete {
                    self.containers.push(Vec::new());
                    self.del = Some(DelState {
                        containers: self.containers.len(),
                        tables: self.tables.len(),
                        cells: self.cells.len(),
                        lists_saved: self.lists_saved.len(),
                        fmt: self.fmt.len(),
                        lists: std::mem::take(&mut self.lists),
                        item_used: std::mem::take(&mut self.item_used),
                        ins: Vec::new(),
                    });
                }
            }
            "change-info" if self.region.is_some() => self.info_at = Some(self.depth),
            _ => self.skip = 1,
        }
    }

    /// End a deletion's content: its paragraphs go to the region being read.
    pub(in crate::odt) fn close_deletion(&mut self) {
        if self.del.is_none() {
            return;
        }
        self.end_para();
        let Some(d) = self.del.take() else { return };
        let base = d.containers.saturating_sub(1).min(self.containers.len());
        let blocks: Vec<FBlock> = self.containers.split_off(base).into_iter().flatten().filter(|b| matches!(b, FBlock::Para(_))).collect();
        if self.containers.is_empty() {
            self.containers.push(Vec::new());
        }
        self.tables.truncate(d.tables);
        self.cells.truncate(d.cells);
        self.formulas.truncate(d.cells);
        self.cell_tops.truncate(d.cells);
        self.lists_saved.truncate(d.lists_saved);
        self.fmt.truncate(d.fmt);
        self.lists = d.lists;
        self.item_used = d.item_used;
        self.frame = None;
        self.in_desc = false;
        self.para_cont = false;
        if let Some((_, r)) = &mut self.region {
            r.blocks = blocks;
        }
    }

    /// Put a deletion's paragraphs back at a `text:change` point, as deleted text. Inside a
    /// paragraph, each deleted paragraph mark splits it; between paragraphs, each deleted
    /// paragraph stands alone with its mark deleted.
    pub(in crate::odt) fn place_deletion(&mut self, blocks: Vec<FBlock>, mark: RevMark) {
        let mut paras: Vec<Para> = Vec::new();
        for b in blocks {
            let FBlock::Para(mut p) = b else { continue };
            // Pictures and other objects carry no revision mark: leave them out.
            p.inlines.retain(|i| matches!(i, Inline::Text(..) | Inline::Anchor(_)));
            for i in &mut p.inlines {
                if let Inline::Text(_, f) = i {
                    f.del = Some(mark.clone());
                }
            }
            paras.push(p);
        }
        let append = |dst: &mut Para, src: Vec<Inline>| {
            for i in src {
                match i {
                    Inline::Text(t, f) => dst.push_text(&t, &f),
                    other => dst.inlines.push(other),
                }
            }
        };
        let Some(mut cur) = self.para.take() else {
            self.para_cont = false;
            for mut p in paras {
                p.mark_del = Some(Box::new(mark.clone()));
                self.finish_para(p);
            }
            return;
        };
        let n = paras.len();
        let mut it = paras.into_iter();
        let mut first_mark_ins = None;
        if let Some(first) = it.next() {
            first_mark_ins = first.mark_ins;
            append(&mut cur, first.inlines);
        }
        if n < 2 {
            self.para = Some(cur);
            return;
        }
        let mut rest = Para { kind: cur.kind, list: cur.list, align: cur.align, ..Default::default() };
        // The host paragraph now ends at the first deleted paragraph's mark.
        cur.mark_del = Some(Box::new(mark.clone()));
        cur.mark_ins = first_mark_ins;
        self.finish_para(cur);
        for (k, mut p) in it.enumerate() {
            if k + 2 == n {
                // The last deleted paragraph runs on into the rest of the body paragraph.
                rest.kind = p.kind;
                rest.align = p.align.or(rest.align);
                append(&mut rest, p.inlines);
            } else {
                p.mark_del = Some(Box::new(mark.clone()));
                self.finish_para(p);
            }
        }
        self.para = Some(rest);
    }

    pub(in crate::odt) fn tracked_context(&mut self, q: &str, a: &Attrs) -> bool {
        if !self.in_tc {
            return false;
        }
        if self.del.is_none() {
            self.tracked_start(q, a);
            return true;
        }
        // Inside deleted content: only insertions (text inserted, then deleted) nest.
        if matches!(local(q), "change-start" | "change-end") {
            let regions = &self.regions;
            if let Some(d) = &mut self.del {
                change_mark(regions, &mut d.ins, RevisionKind::Insert, local(q) == "change-start", a);
            }
            return true;
        }
        if matches!(local(q), "tracked-changes" | "changed-region" | "insertion" | "deletion" | "format-change" | "change") {
            self.skip = 1;
            return true;
        }
        if local(q) == "change-info" && self.region.is_some() {
            self.info_at = Some(self.depth);
            return true;
        }
        false
    }
}
