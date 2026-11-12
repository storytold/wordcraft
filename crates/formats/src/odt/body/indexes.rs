//! Tables of contents and other generated indexes read (ODF 1.3 §8): their entries as last
//! generated, with the Word field they stand for — in the title paragraph when there is one,
//! as WordCraft inserts a table, or around the entries — so Update Table finds them.

use super::{Attrs, Body, FBlock, Fmt, Inline, get};
use crate::odt::writer::{IndexKind, index_instr};

/// An index being read.
pub(in crate::odt) struct OpenIndex {
    pub(in crate::odt) depth: usize,
    /// What it stands for; `None` for indexes Word has no field for (user and object indexes),
    /// read as their paragraphs.
    pub(in crate::odt) kind: Option<IndexKind>,
    /// Where its blocks start in the container it is in, and its title's (start, end) there.
    pub(in crate::odt) start: usize,
    pub(in crate::odt) title: (Option<usize>, Option<usize>),
}

/// Whether `name` (local, in the text namespace) is an index element.
pub(in crate::odt) fn is_index(name: &str) -> bool {
    matches!(name, "table-of-content" | "illustration-index" | "table-index" | "alphabetical-index" | "bibliography" | "user-index" | "object-index")
}

impl Body<'_> {
    /// An index element, its source or its title (`q`, attributes `a`); false for anything else.
    pub(in crate::odt) fn start_index_part(&mut self, q: &str, a: &Attrs) -> bool {
        let Some(name) = q.strip_prefix("text:") else { return false };
        if is_index(name) {
            // A table of contents or another index (§8), read as its entries and the field it
            // stands for.
            if self.index.is_none() && self.note.is_none() && self.del.is_none() && !self.in_tc && self.frame.is_none() {
                self.start_index(name);
            }
        } else if name.ends_with("-source") {
            // What it is generated from: its levels or caption name; its templates aren't read.
            self.index_source(a);
            self.skip = 1;
        } else if name == "index-title" {
            self.index_title(false);
        } else {
            return false;
        }
        true
    }

    /// An index element begins: its blocks are read into the container it is in.
    pub(in crate::odt) fn start_index(&mut self, name: &str) {
        self.end_para();
        let kind = match name {
            "table-of-content" => Some(IndexKind::Contents(10)),
            "illustration-index" => Some(IndexKind::Captions("Figure".into())),
            "table-index" => Some(IndexKind::Captions("Table".into())),
            "alphabetical-index" => Some(IndexKind::Alphabetical),
            "bibliography" => Some(IndexKind::Bibliography),
            _ => None,
        };
        let start = self.container().len();
        self.index = Some(OpenIndex { depth: self.depth, kind, start, title: (None, None) });
    }

    /// An index's source (what it is generated from): its outline levels or caption name.
    pub(in crate::odt) fn index_source(&mut self, a: &Attrs) {
        let Some(ix) = &mut self.index else { return };
        match &mut ix.kind {
            Some(IndexKind::Contents(levels)) => {
                if let Some(n) = get(a, "text:outline-level").and_then(|v| v.trim().parse::<u32>().ok()) {
                    *levels = n.clamp(1, 10) as u8;
                }
            }
            Some(IndexKind::Captions(name)) => {
                if let Some(c) = get(a, "text:caption-sequence-name").map(str::trim).filter(|c| !c.is_empty()) {
                    *name = c.chars().take(64).collect();
                }
            }
            _ => {}
        }
    }

    /// An index title begins (`end` false) or ends.
    pub(in crate::odt) fn index_title(&mut self, end: bool) {
        self.end_para();
        let len = self.container().len();
        if let Some(ix) = &mut self.index {
            if end {
                ix.title.1.get_or_insert(len);
            } else {
                ix.title.0.get_or_insert(len);
            }
        }
    }

    /// The index being read ends: its field goes into its title's first paragraph, or around
    /// its entries.
    pub(in crate::odt) fn end_index(&mut self) {
        self.end_para();
        let Some(ix) = self.index.take() else { return };
        let Some(kind) = ix.kind else { return };
        let instr = index_instr(&kind);
        let blocks = self.container();
        let start = ix.start.min(blocks.len());
        let titled = match ix.title {
            (Some(s), Some(e)) if s == start && e > s => blocks.get_mut(s),
            _ => None,
        };
        if let Some(FBlock::Para(p)) = titled {
            p.inlines.push(Inline::Field { instr, result: String::new(), fmt: Fmt::default(), locked: false });
            return;
        }
        let Some(entries) = blocks.get_mut(start..) else { return };
        let first = entries.iter().position(|b| matches!(b, FBlock::Para(_)));
        let last = entries.iter().rposition(|b| matches!(b, FBlock::Para(_)));
        let (Some(first), Some(last)) = (first, last) else { return };
        if let Some(FBlock::Para(p)) = entries.get_mut(first) {
            p.inlines.insert(0, Inline::FieldStart(instr));
        }
        if let Some(FBlock::Para(p)) = entries.get_mut(last) {
            p.inlines.push(Inline::FieldEnd);
        }
    }
}
