//! Tracked formatting changes (#41): recording them while Track Changes is on, describing them
//! ("Formatted: Bold, Size: 14 pt") and accepting or rejecting them.
//!
//! Recording isn't done by each formatting command: after any command that edits the document,
//! [`record`] compares the paragraphs it changed with the document before it. Where a paragraph
//! kept its text but its runs, mark or properties were formatted differently, the change is kept
//! as a formatting revision holding the formatting before it (`w:rPrChange` / `w:pPrChange`).
//! Like Word, an author's later changes to the same text keep the formatting before their first
//! one, and formatting changed back to what it was stops being a change.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::props::{CellProps, CharProps, ParaProps, PropChange, RowProps, TableProps};
use wordcraft_doc::section::SectionProps;
use wordcraft_doc::table::{Cell, Row, Table};
use wordcraft_doc::{Block, Blocks, Document, Paragraph, Revision, RevisionKind};

use crate::Session;

/// Commands whose formatting edits aren't the author's own changes: accepting or rejecting
/// changes, comparing documents, undo and redo, and opening or replacing the document.
fn records(id: &str) -> bool {
    !(id.starts_with("review.accept")
        || id.starts_with("review.reject")
        || matches!(id, "review.compare" | "review.combine" | "edit.undo" | "edit.redo")
        || id.starts_with("file."))
}

/// Tables nested deeper than this aren't compared (hostile documents).
const MAX_DEPTH: usize = 16;

struct Recorder<'a> {
    author: String,
    date: String,
    before: &'a [Revision],
    revisions: &'a mut Vec<Revision>,
    rid: Option<u32>,
}

impl Recorder<'_> {
    /// The author's formatting revision for this command (made on first use).
    fn rid(&mut self) -> u32 {
        if let Some(r) = self.rid {
            return r;
        }
        let found = self.revisions.iter().rposition(|r| r.kind == RevisionKind::Format && r.author == self.author && r.date == self.date);
        let r = match found {
            Some(i) => i as u32,
            None => {
                self.revisions.push(Revision { kind: RevisionKind::Format, author: self.author.clone(), date: self.date.clone() });
                self.revisions.len().saturating_sub(1) as u32
            }
        };
        self.rid = Some(r);
        r
    }

    fn own(&self, rev: u32) -> bool {
        self.before.get(rev as usize).is_some_and(|r| r.author == self.author)
    }

    /// The tracked change `after` should carry, given the props `before` the command (`f` strips
    /// everything that isn't formatting); `None` when it already carries the right one.
    #[allow(clippy::type_complexity, clippy::option_option)]
    fn change<T: Clone + PartialEq>(
        &mut self,
        before: &T,
        after: &T,
        f: fn(&T) -> T,
        get: fn(&T) -> &Option<Box<PropChange<T>>>,
    ) -> Option<Option<Box<PropChange<T>>>> {
        let (fb, fa) = (f(before), f(after));
        let (had, has) = (get(before), get(after));
        let want = if fb == fa {
            // Not formatted by this command: keep the change it carries (or carried).
            if has.is_some() {
                return None;
            }
            had.clone()
        } else {
            // The author's own earlier change keeps the formatting before it; another author's
            // is superseded by this one.
            let own = had.as_ref().filter(|c| self.own(c.rev));
            let old = own.map(|c| c.old.clone()).unwrap_or(fb);
            if old == fa {
                None
            } else {
                let rev = own.map(|c| c.rev).unwrap_or_else(|| self.rid());
                PropChange::boxed(rev, old)
            }
        };
        (want != *has).then_some(want)
    }

    /// Record the formatting changes between `b` and `a` (the same text) into `a`.
    fn para(&mut self, b: &Paragraph, a: &mut Paragraph) {
        let mut runs = Vec::new();
        let (rb, ra): (Vec<_>, Vec<_>) = (b.run_ranges().collect(), a.run_ranges().collect());
        let (mut i, mut j) = (0, 0);
        while let (Some((x, pb)), Some((y, pa))) = (rb.get(i), ra.get(j)) {
            let (from, to) = (x.start.max(y.start), x.end.min(y.end));
            // Formatting text the author is inserting themselves changes the insertion, as in Word.
            let own_insert = pa.ins.and_then(|i| self.revisions.get(i as usize)).is_some_and(|r| r.author == self.author);
            if from < to
                && !own_insert
                && let Some(want) = self.change(*pb, *pa, CharProps::formatting, |c| &c.fmt_change)
            {
                runs.push((from, to, want));
            }
            if x.end <= y.end {
                i += 1;
            }
            if y.end <= x.end {
                j += 1;
            }
        }
        let mark = self.change(&b.mark, &a.mark, CharProps::formatting, |c| &c.fmt_change);
        let props = self.change(&b.props, &a.props, ParaProps::formatting, |p| &p.fmt_change);
        if runs.is_empty() && mark.is_none() && props.is_none() {
            return;
        }
        for (from, to, want) in runs {
            let _ = a.format(from, to, &|c| c.fmt_change = want.clone());
        }
        if let Some(m) = mark {
            a.mark.fmt_change = m;
        }
        if let Some(p) = props {
            a.props.fmt_change = p;
        }
        a.normalize();
        a.touch();
    }

    fn blocks(&mut self, before: &Blocks, after: &mut Blocks, depth: usize) {
        if before.len() != after.len() || depth > MAX_DEPTH {
            return;
        }
        for (b, a) in before.iter().zip(after.iter_mut()) {
            if Arc::ptr_eq(b, a) {
                continue;
            }
            match (&**b, &**a) {
                (Block::Para(pb), Block::Para(pa)) if pb.text == pa.text => {
                    if let Block::Para(pa) = Arc::make_mut(a) {
                        self.para(pb, pa);
                    }
                }
                (Block::Table(tb), Block::Table(ta)) if tb.rows.len() == ta.rows.len() => {
                    let Block::Table(ta) = Arc::make_mut(a) else { continue };
                    for (rb, ra) in tb.rows.iter().zip(ta.rows.iter_mut()) {
                        if rb.cells.len() != ra.cells.len() {
                            continue;
                        }
                        for (cb, ca) in rb.cells.iter().zip(ra.cells.iter_mut()) {
                            self.blocks(&cb.blocks, &mut ca.blocks, depth + 1);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Keep the formatting changes the command `id` made to `s.doc` (compared with `before`) as
/// tracked formatting changes, when Track Changes and Track Formatting are on.
pub(crate) fn record(s: &mut Session, id: &str, before: &Document) {
    if !s.doc.settings.track_changes || !s.prefs.markup.track_formatting || !records(id) {
        return;
    }
    let author = s.author.clone();
    let doc = &mut s.doc;
    let mut r = Recorder { author, date: super::now_iso(), before: &before.revisions, revisions: &mut doc.revisions, rid: None };
    r.blocks(&before.body, &mut doc.body, 0);
    for (k, pb) in &before.parts {
        if let Some(pa) = doc.parts.get_mut(k) {
            r.blocks(&pb.blocks, &mut pa.blocks, 0);
        }
    }
}

/// Compare Documents: record how paragraph `new` (the revised version, same text) is formatted
/// differently from `old` as tracked formatting changes by `author`.
pub(crate) fn compare_formatting(old: &Paragraph, new: &mut Paragraph, revisions: &mut Vec<Revision>, author: &str, date: &str) {
    let mut r = Recorder { author: author.to_string(), date: date.to_string(), before: &[], revisions, rid: None };
    r.para(old, new);
}

/// The properties a change sets, as the Style Inspector lists them: `[{"prop", "value"}]` in
/// field order; a switch turned off is `false`, a value removed is `null`.
pub fn diff<T: Serialize>(old: &T, new: &T) -> Vec<Value> {
    let (Ok(Value::Object(o)), Ok(Value::Object(n))) = (serde_json::to_value(old), serde_json::to_value(new)) else { return Vec::new() };
    let mut out = Vec::new();
    for (k, v) in &n {
        if o.get(k) != Some(v) {
            out.push(json!({"prop": k, "value": v}));
        }
    }
    for (k, v) in &o {
        if !n.contains_key(k) {
            out.push(json!({"prop": k, "value": if v.is_boolean() { json!(false) } else { Value::Null }}));
        }
    }
    out
}

/// The properties a run's (or paragraph mark's) change set.
pub fn char_diff(c: &CharProps) -> Vec<Value> {
    c.fmt_change.as_ref().map(|ch| diff(&ch.old.formatting(), &c.formatting())).unwrap_or_default()
}

/// The properties a paragraph's change set.
pub fn para_diff(p: &ParaProps) -> Vec<Value> {
    p.fmt_change.as_ref().map(|ch| diff(&ch.old.formatting(), &p.formatting())).unwrap_or_default()
}

/// A section's change: the properties it set.
pub fn section_diff(s: &SectionProps) -> Vec<Value> {
    let strip = |s: &SectionProps| SectionProps { fmt_change: None, headers: Default::default(), footers: Default::default(), ..s.clone() };
    s.fmt_change.as_ref().map(|ch| diff(&strip(&ch.old), &strip(s))).unwrap_or_default()
}

/// "Formatted: Bold, Not italic, Size: 14" — the change in plain English, for agents.
pub fn describe(props: &[Value]) -> String {
    let items: Vec<String> = props
        .iter()
        .map(|d| {
            let label = words(d["prop"].as_str().unwrap_or(""));
            match &d["value"] {
                Value::Bool(true) => label,
                Value::Bool(false) => format!("Not {}", label.to_lowercase()),
                Value::Null => format!("{label}: (default)"),
                Value::String(s) => format!("{label}: {s}"),
                Value::Number(n) => {
                    let x = n.as_f64().filter(|x| x.is_finite()).unwrap_or(0.0);
                    let t = format!("{x:.2}");
                    format!("{label}: {}", t.trim_end_matches('0').trim_end_matches('.'))
                }
                v => format!("{label}: {v}"),
            }
        })
        .collect();
    if items.is_empty() { "Formatted".into() } else { format!("Formatted: {}", items.join(", ")) }
}

/// `indentLeft` → `Indent left`.
fn words(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().take(64).enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Accept (drop the record) or reject (restore the old formatting) a run's change.
pub fn resolve_char(c: &mut CharProps, accept: bool) {
    let Some(ch) = c.fmt_change.take() else { return };
    if !accept {
        *c = CharProps { link: c.link.take(), ins: c.ins, del: c.del, ..ch.old.formatting() };
    }
}

/// Accept or reject a paragraph's change (its list numbering change goes with it).
pub fn resolve_para(p: &mut ParaProps, accept: bool) {
    p.num_change = None;
    let Some(ch) = p.fmt_change.take() else { return };
    if !accept {
        *p = ch.old.formatting();
    }
}

/// Accept or reject every table, row, cell and section change in `blocks` (paragraph changes
/// are resolved with their text).
pub fn resolve_containers(blocks: &mut Blocks, accept: bool, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for b in blocks.iter_mut() {
        let needs = match &**b {
            Block::Para(p) => p.section.as_ref().is_some_and(|s| s.fmt_change.is_some()),
            Block::Table(_) => true,
        };
        if !needs {
            continue;
        }
        match Arc::make_mut(b) {
            Block::Para(p) => {
                if let Some(s) = p.section.as_deref_mut() {
                    resolve_section(s, accept);
                    p.touch();
                }
            }
            Block::Table(t) => {
                resolve_table(t, accept);
                for r in &mut t.rows {
                    resolve_row(r, accept);
                    for c in &mut r.cells {
                        resolve_cell(c, accept);
                        resolve_containers(&mut c.blocks, accept, depth + 1);
                    }
                }
            }
        }
    }
}

/// Accept or reject a table's own change (not its rows' or cells').
pub fn resolve_table(t: &mut Table, accept: bool) {
    if let Some(ch) = t.props.fmt_change.take().filter(|_| !accept) {
        t.props = TableProps { fmt_change: None, ..ch.old };
    }
}

/// Accept or reject a row's change.
pub fn resolve_row(r: &mut Row, accept: bool) {
    if let Some(ch) = r.props.fmt_change.take().filter(|_| !accept) {
        r.props = RowProps { fmt_change: None, ..ch.old };
    }
}

/// Accept or reject a cell's change. Merges are the table's structure, not formatting: they stay.
pub fn resolve_cell(c: &mut Cell, accept: bool) {
    if let Some(ch) = c.props.fmt_change.take().filter(|_| !accept) {
        c.props = CellProps { fmt_change: None, span: c.props.span, vmerge: c.props.vmerge, ..ch.old };
    }
}

/// Accept or reject a section's change (its headers and footers stay).
pub fn resolve_section(s: &mut SectionProps, accept: bool) {
    let Some(ch) = s.fmt_change.take() else { return };
    if !accept {
        let (headers, footers) = (std::mem::take(&mut s.headers), std::mem::take(&mut s.footers));
        *s = SectionProps { headers, footers, fmt_change: None, ..ch.old };
    }
}

/// A table's, row's or cell's change: the properties it set.
pub fn table_diff(t: &TableProps) -> Vec<Value> {
    t.fmt_change
        .as_ref()
        .map(|ch| diff(&TableProps { fmt_change: None, ..ch.old.clone() }, &TableProps { fmt_change: None, ..t.clone() }))
        .unwrap_or_default()
}
pub fn row_diff(r: &RowProps) -> Vec<Value> {
    r.fmt_change
        .as_ref()
        .map(|ch| diff(&RowProps { fmt_change: None, ..ch.old.clone() }, &RowProps { fmt_change: None, ..r.clone() }))
        .unwrap_or_default()
}
pub fn cell_diff(c: &CellProps) -> Vec<Value> {
    c.fmt_change
        .as_ref()
        .map(|ch| diff(&CellProps { fmt_change: None, ..ch.old.clone() }, &CellProps { fmt_change: None, ..c.clone() }))
        .unwrap_or_default()
}
