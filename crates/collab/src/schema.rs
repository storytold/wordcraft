//! Schema 1 in detail: stories, paragraphs and tables ↔ Yjs XML, and the minimal transaction that
//! turns one version of a story into the next.
//!
//! A story is an XmlFragment of `<p>` and `<table>` elements. A `<p>`'s attributes are its
//! properties (`pPr.<key>`), its mark's formatting (`rPr`) and section (`sectPr`); its one XmlText
//! child is its text: characters with formatting attributes, inline objects as embeds. A
//! `<table>` (`tblPr`, `grid`, `controls`) holds `<tr>` (`trPr`, `controls`) holding `<tc>`
//! (`tcPr`, `controls`) holding blocks.

use std::sync::Arc;

use serde_json::{Map as JsonMap, Value as Json};
use wordcraft_doc::para::{OBJ, Run};
use wordcraft_doc::table::{Cell, Row, Table};
use wordcraft_doc::{Block, Blocks, CharProps, InlineObject, Paragraph, PartKind};
use yrs::types::text::{Diff, YChange};
use yrs::{Any, Out, ReadTxn, Text, TransactionMut, Xml, XmlElementPrelim, XmlElementRef, XmlFragment, XmlOut, XmlTextPrelim, XmlTextRef};

use crate::json::{attrs_delta, attrs_exact, attrs_object, from_json, from_object, object_of, to_any, to_json};

/// Tables in tables are read and written this deep (the document model allows a few levels).
pub(crate) const MAX_DEPTH: usize = 16;
/// Prefix of the paragraph property attributes.
pub(crate) const PPR: &str = "pPr.";
/// The roots holding stories other than the body, by part kind.
pub(crate) const STORY_ROOTS: [&str; 4] = ["headers", "footers", "notes", "textBoxes"];
/// Middles of block lists aligned by content when no bigger than this (rows × columns of the
/// alignment table); larger ones pair blocks by position.
const MAX_ALIGN: usize = 40_000;

pub(crate) fn kind_name(k: PartKind) -> &'static str {
    match k {
        PartKind::Header => "header",
        PartKind::Footer => "footer",
        PartKind::Footnote => "footnote",
        PartKind::Endnote => "endnote",
        PartKind::Comment => "comment",
        PartKind::TextBox => "textBox",
    }
}

pub(crate) fn kind_of(name: &str) -> Option<PartKind> {
    Some(match name {
        "header" => PartKind::Header,
        "footer" => PartKind::Footer,
        "footnote" => PartKind::Footnote,
        "endnote" => PartKind::Endnote,
        "comment" => PartKind::Comment,
        "textBox" => PartKind::TextBox,
        _ => return None,
    })
}

/// The root a story of `kind` lives in (comments' text lives in their comment).
pub(crate) fn story_root(k: PartKind) -> Option<&'static str> {
    match k {
        PartKind::Header => Some("headers"),
        PartKind::Footer => Some("footers"),
        PartKind::Footnote | PartKind::Endnote => Some("notes"),
        PartKind::TextBox => Some("textBoxes"),
        PartKind::Comment => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Paragraph text as units

/// One character or inline object of a paragraph, with its formatting.
#[derive(Clone, Copy)]
pub(crate) struct Unit<'a> {
    pub ch: char,
    pub obj: Option<&'a InlineObject>,
    pub props: &'a CharProps,
    /// Its byte offset in the paragraph's text.
    pub at: usize,
}

impl Unit<'_> {
    /// Its length in the shared text (UTF-16 code units; an embed is 1).
    pub fn ylen(&self) -> u32 {
        if self.obj.is_some() { 1 } else { self.ch.len_utf16() as u32 }
    }
    fn same_content(&self, o: &Unit) -> bool {
        match (self.obj, o.obj) {
            (Some(a), Some(b)) => a == b,
            (None, None) => self.ch == o.ch,
            _ => false,
        }
    }
}

static NO_PROPS: std::sync::LazyLock<CharProps> = std::sync::LazyLock::new(CharProps::default);

/// A paragraph's units. A U+FFFC without its object (a damaged paragraph) isn't shared.
pub(crate) fn units(p: &Paragraph) -> Vec<Unit<'_>> {
    let mut out = Vec::with_capacity(p.text.len());
    let mut runs = p.runs.iter();
    let mut run: Option<&Run> = runs.next();
    let mut run_start = 0usize;
    let mut k = 0usize;
    for (i, c) in p.text.char_indices() {
        while let Some(r) = run {
            if i >= run_start + r.len {
                run_start += r.len;
                run = runs.next();
            } else {
                break;
            }
        }
        let props = run.map(|r| &r.props).unwrap_or(&NO_PROPS);
        if c == OBJ {
            let obj = p.objects.get(k);
            k += 1;
            if let Some(o) = obj {
                out.push(Unit { ch: c, obj: Some(o), props, at: i });
            }
            continue;
        }
        out.push(Unit { ch: c, obj: None, props, at: i });
    }
    out
}

/// The shared-text index of byte `off` of `p`'s text.
pub(crate) fn y_of_byte(p: &Paragraph, off: usize) -> u32 {
    units(p).iter().take_while(|u| u.at < off).map(Unit::ylen).sum()
}

/// The byte offset of shared-text index `y` in `p`'s text (its end past the last unit).
pub(crate) fn byte_of_y(p: &Paragraph, y: u32) -> usize {
    let mut at = 0u32;
    for u in units(p) {
        if at >= y {
            return u.at;
        }
        at += u.ylen();
    }
    p.text.len()
}

// ---------------------------------------------------------------------------------------------
// Reading

fn tag_of(e: &XmlElementRef) -> Option<&str> {
    e.try_tag().map(|t| t.as_ref())
}

/// The block elements of a story or cell (`<p>` and `<table>`) with their raw child indexes;
/// other children are skipped.
pub(crate) fn block_slots<T: ReadTxn, F: XmlFragment>(txn: &T, f: &F) -> Vec<(u32, XmlElementRef)> {
    f.children(txn)
        .enumerate()
        .filter_map(|(i, c)| match c {
            XmlOut::Element(e) if matches!(tag_of(&e), Some("p" | "table")) => Some((i as u32, e)),
            _ => None,
        })
        .collect()
}

pub(crate) fn children_tagged<T: ReadTxn>(txn: &T, e: &XmlElementRef, tag: &str) -> Vec<(u32, XmlElementRef)> {
    e.children(txn)
        .enumerate()
        .filter_map(|(i, c)| match c {
            XmlOut::Element(x) if tag_of(&x) == Some(tag) => Some((i as u32, x)),
            _ => None,
        })
        .collect()
}

/// A paragraph's text (its first XmlText child).
pub(crate) fn text_of<T: ReadTxn>(txn: &T, p: &XmlElementRef) -> Option<XmlTextRef> {
    p.children(txn).find_map(|c| match c {
        XmlOut::Text(t) => Some(t),
        _ => None,
    })
}

fn attr_json<T: ReadTxn>(txn: &T, e: &XmlElementRef, name: &str) -> Option<Json> {
    match e.get_attribute(txn, name)? {
        Out::Any(Any::Null | Any::Undefined) => None,
        Out::Any(a) => Some(to_json(&a)),
        _ => None,
    }
}

fn attr_object<T: ReadTxn>(txn: &T, e: &XmlElementRef, name: &str) -> JsonMap<String, Json> {
    match attr_json(txn, e, name) {
        Some(Json::Object(o)) => o,
        _ => JsonMap::new(),
    }
}

/// Reads a paragraph element.
pub(crate) fn read_para<T: ReadTxn>(txn: &T, el: &XmlElementRef) -> Paragraph {
    let mut p = Paragraph::new();
    let mut props = JsonMap::new();
    for (k, v) in el.attributes(txn) {
        if let Some(key) = k.strip_prefix(PPR)
            && let Out::Any(a) = v
            && !matches!(a, Any::Null | Any::Undefined)
        {
            props.insert(key.to_string(), to_json(&a));
        }
    }
    p.props = from_object(props);
    p.mark = from_object(attr_object(txn, el, "rPr"));
    p.section = attr_json(txn, el, "sectPr").and_then(from_json).map(Box::new);
    let mut cache: Vec<(JsonMap<String, Json>, CharProps)> = Vec::new();
    for c in el.children(txn) {
        let XmlOut::Text(t) = c else { continue };
        for d in t.diff(txn, YChange::identity) {
            let Diff { insert, attributes, .. } = d;
            let o = attrs_object(attributes.as_deref());
            let props = match cache.iter().find(|(k, _)| *k == o) {
                Some((_, p)) => p.clone(),
                None => {
                    let parsed: CharProps = from_object(o.clone());
                    cache.push((o, parsed.clone()));
                    parsed
                }
            };
            match insert {
                Out::Any(Any::String(s)) => {
                    let clean: String = s.chars().filter(|c| *c != OBJ).collect();
                    push_run(&mut p, &clean, props);
                }
                Out::Any(other) => {
                    if let Some(obj) = from_json::<InlineObject>(to_json(&other)) {
                        p.objects.push(obj);
                        push_run(&mut p, &OBJ.to_string(), props);
                    }
                }
                _ => {}
            }
        }
    }
    p
}

fn push_run(p: &mut Paragraph, text: &str, props: CharProps) {
    if text.is_empty() {
        return;
    }
    p.text.push_str(text);
    match p.runs.last_mut() {
        Some(r) if r.props == props => r.len += text.len(),
        _ => p.runs.push(Run { len: text.len(), props }),
    }
}

/// Reads a block. `hint`: the block this element was before (its unchanged paragraphs are
/// reused, so layout caches stay).
pub(crate) fn read_block<T: ReadTxn>(txn: &T, el: &XmlElementRef, hint: Option<&Arc<Block>>, depth: usize) -> Option<Arc<Block>> {
    let block = match tag_of(el)? {
        "p" => Block::Para(read_para(txn, el)),
        "table" if depth < MAX_DEPTH => {
            let old = hint.and_then(|h| h.as_table());
            let mut t = Table {
                props: from_object(attr_object(txn, el, "tblPr")),
                grid: attr_json(txn, el, "grid").and_then(from_json).unwrap_or_default(),
                rows: Vec::new(),
                controls: attr_json(txn, el, "controls").and_then(from_json).unwrap_or_default(),
            };
            for (ri, (_, tr)) in children_tagged(txn, el, "tr").into_iter().enumerate() {
                let old_row = old.and_then(|t| t.rows.get(ri));
                let mut row = Row {
                    props: from_object(attr_object(txn, &tr, "trPr")),
                    cells: Vec::new(),
                    controls: attr_json(txn, &tr, "controls").and_then(from_json).unwrap_or_default(),
                };
                for (ci, (_, tc)) in children_tagged(txn, &tr, "tc").into_iter().enumerate() {
                    let old_cell = old_row.and_then(|r| r.cells.get(ci));
                    let mut blocks = read_blocks(txn, &tc, old_cell.map(|c| &c.blocks), depth + 1);
                    if !matches!(blocks.last().map(|b| &**b), Some(Block::Para(_))) {
                        blocks.push(Arc::new(Block::Para(Paragraph::new())));
                    }
                    row.cells.push(Cell {
                        props: from_object(attr_object(txn, &tc, "tcPr")),
                        blocks,
                        controls: attr_json(txn, &tc, "controls").and_then(from_json).unwrap_or_default(),
                    });
                }
                if !row.cells.is_empty() {
                    t.rows.push(row);
                }
            }
            if t.rows.is_empty() {
                return None;
            }
            fix_grid(&mut t);
            Block::Table(t)
        }
        _ => return None,
    };
    match hint {
        Some(h) if **h == block => Some(h.clone()),
        _ => Some(Arc::new(block)),
    }
}

/// A grid with a width for every column the rows use.
fn fix_grid(t: &mut Table) {
    let cols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span()).sum::<usize>()).max().unwrap_or(0).min(wordcraft_doc::table::MAX_COLS);
    t.grid.retain(|w| w.is_finite() && *w >= 0.0);
    while t.grid.len() < cols {
        let w = t.grid.last().copied().filter(|w| *w > 0.0).unwrap_or(72.0);
        t.grid.push(w);
    }
}

/// Reads the blocks of a story or cell; `hint`: what they were (equal blocks are reused, by
/// position).
pub(crate) fn read_blocks<T: ReadTxn, F: XmlFragment>(txn: &T, f: &F, hint: Option<&Blocks>, depth: usize) -> Blocks {
    block_slots(txn, f).iter().enumerate().filter_map(|(i, (_, el))| read_block(txn, el, hint.and_then(|h| h.get(i)), depth)).collect()
}

// ---------------------------------------------------------------------------------------------
// Writing

fn set_attr(txn: &mut TransactionMut, el: &XmlElementRef, name: &str, v: Option<Json>) {
    let have = attr_json(txn, el, name);
    match v {
        Some(v) if have.as_ref() != Some(&v) => {
            el.insert_attribute(txn, name, to_any(&v));
        }
        None if have.is_some() => el.remove_attribute(txn, &name),
        _ => {}
    }
}

fn opt_json<T: serde::Serialize>(v: &T, empty: bool) -> Option<Json> {
    if empty { None } else { serde_json::to_value(v).ok() }
}

/// Writes a paragraph's attributes (only those that differ from `old`'s).
fn write_para_attrs(txn: &mut TransactionMut, el: &XmlElementRef, old: Option<&Paragraph>, p: &Paragraph) {
    let new = object_of(&p.props);
    let before = old.map(|o| object_of(&o.props)).unwrap_or_default();
    for (k, v) in &new {
        if before.get(k) != Some(v) {
            el.insert_attribute(txn, format!("{PPR}{k}"), to_any(v));
        }
    }
    for k in before.keys() {
        if !new.contains_key(k) {
            el.remove_attribute(txn, &format!("{PPR}{k}"));
        }
    }
    if old.is_none_or(|o| o.mark != p.mark) {
        let m = object_of(&p.mark);
        set_attr(txn, el, "rPr", (!m.is_empty()).then_some(Json::Object(m)));
    }
    if old.is_none_or(|o| o.section != p.section) {
        set_attr(txn, el, "sectPr", p.section.as_ref().and_then(|s| serde_json::to_value(s).ok()));
    }
}

/// Every character property key a paragraph uses (to insert text with exactly its attributes).
fn keys_of(ps: &[&Paragraph]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for p in ps {
        for r in &p.runs {
            for k in object_of(&r.props).keys() {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
    }
    keys
}

/// Inserts `us` into `t` at shared index `at`, each with exactly its formatting.
fn insert_units(txn: &mut TransactionMut, t: &XmlTextRef, mut at: u32, us: &[Unit], keys: &[String]) {
    let mut i = 0;
    while i < us.len() {
        let Some(u) = us.get(i) else { break };
        let attrs = attrs_exact(&object_of(u.props), keys);
        if let Some(o) = u.obj {
            let v = serde_json::to_value(o).map(|j| to_any(&j)).unwrap_or(Any::Null);
            t.insert_embed_with_attributes(txn, at, v, attrs);
            at += 1;
            i += 1;
            continue;
        }
        let mut s = String::new();
        let mut j = i;
        while let Some(x) = us.get(j) {
            if x.obj.is_some() || !std::ptr::eq(x.props, u.props) && x.props != u.props {
                break;
            }
            s.push(x.ch);
            j += 1;
        }
        let n = s.encode_utf16().count() as u32;
        t.insert_with_attributes(txn, at, &s, attrs);
        at += n;
        i = j;
    }
}

/// A new `<p>` at raw index `raw` of `f`.
pub(crate) fn write_para<F: XmlFragment>(txn: &mut TransactionMut, f: &F, raw: u32, p: &Paragraph) {
    let el = f.insert(txn, raw, XmlElementPrelim::empty("p"));
    write_para_attrs(txn, &el, None, p);
    let t = el.push_back(txn, XmlTextPrelim::new(""));
    let us = units(p);
    insert_units(txn, &t, 0, &us, &keys_of(&[p]));
}

/// A new block at raw index `raw` of `f`.
pub(crate) fn write_block<F: XmlFragment>(txn: &mut TransactionMut, f: &F, raw: u32, b: &Block, depth: usize) {
    match b {
        Block::Para(p) => write_para(txn, f, raw, p),
        Block::Table(t) if depth < MAX_DEPTH => {
            let el = f.insert(txn, raw, XmlElementPrelim::empty("table"));
            write_table_attrs(txn, &el, None, t);
            for row in &t.rows {
                let n = el.len(txn);
                write_row(txn, &el, n, row, depth);
            }
        }
        // Deeper tables aren't shared (the model doesn't make them).
        Block::Table(_) => {}
    }
}

fn write_table_attrs(txn: &mut TransactionMut, el: &XmlElementRef, old: Option<&Table>, t: &Table) {
    if old.is_none_or(|o| o.props != t.props) {
        set_attr(txn, el, "tblPr", serde_json::to_value(&t.props).ok());
    }
    if old.is_none_or(|o| o.grid != t.grid) {
        set_attr(txn, el, "grid", serde_json::to_value(&t.grid).ok());
    }
    if old.is_none_or(|o| o.controls != t.controls) {
        set_attr(txn, el, "controls", opt_json(&t.controls, t.controls.is_empty()));
    }
}

fn write_row(txn: &mut TransactionMut, table: &XmlElementRef, raw: u32, row: &Row, depth: usize) {
    let tr = table.insert(txn, raw, XmlElementPrelim::empty("tr"));
    write_row_attrs(txn, &tr, None, row);
    for cell in &row.cells {
        let n = tr.len(txn);
        write_cell(txn, &tr, n, cell, depth);
    }
}

fn write_row_attrs(txn: &mut TransactionMut, tr: &XmlElementRef, old: Option<&Row>, row: &Row) {
    if old.is_none_or(|o| o.props != row.props) {
        set_attr(txn, tr, "trPr", serde_json::to_value(&row.props).ok());
    }
    if old.is_none_or(|o| o.controls != row.controls) {
        set_attr(txn, tr, "controls", opt_json(&row.controls, row.controls.is_empty()));
    }
}

fn write_cell(txn: &mut TransactionMut, tr: &XmlElementRef, raw: u32, cell: &Cell, depth: usize) {
    let tc = tr.insert(txn, raw, XmlElementPrelim::empty("tc"));
    write_cell_attrs(txn, &tc, None, cell);
    for (i, b) in cell.blocks.iter().enumerate() {
        write_block(txn, &tc, i as u32, b, depth + 1);
    }
}

fn write_cell_attrs(txn: &mut TransactionMut, tc: &XmlElementRef, old: Option<&Cell>, cell: &Cell) {
    if old.is_none_or(|o| o.props != cell.props) {
        set_attr(txn, tc, "tcPr", serde_json::to_value(&cell.props).ok());
    }
    if old.is_none_or(|o| o.controls != cell.controls) {
        set_attr(txn, tc, "controls", opt_json(&cell.controls, cell.controls.is_empty()));
    }
}

/// Writes a whole story into an empty fragment.
pub(crate) fn write_blocks<F: XmlFragment>(txn: &mut TransactionMut, f: &F, blocks: &Blocks) {
    for b in blocks {
        let n = f.len(txn);
        write_block(txn, f, n, b, 0);
    }
}

// ---------------------------------------------------------------------------------------------
// Diffing: one version of a story → the next, as few Yjs operations as possible

/// Pairs old and new items in order: `(Some(i), Some(j))` updates old `i` into new `j`, `(Some(i),
/// None)` deletes it, `(None, Some(j))` inserts new `j`. Equal items at both ends pair up; the
/// middle pairs by `score` (0: never), the highest total, ties to the earlier.
pub(crate) fn align<T>(old: &[T], new: &[T], same: impl Fn(&T, &T) -> bool, score: impl Fn(&T, &T) -> u64) -> Vec<(Option<usize>, Option<usize>)> {
    let (m, n) = (old.len(), new.len());
    let mut pre = 0;
    while pre < m && pre < n && old.get(pre).zip(new.get(pre)).is_some_and(|(a, b)| same(a, b)) {
        pre += 1;
    }
    let mut suf = 0;
    while suf < m - pre && suf < n - pre && old.get(m - 1 - suf).zip(new.get(n - 1 - suf)).is_some_and(|(a, b)| same(a, b)) {
        suf += 1;
    }
    let mut out: Vec<(Option<usize>, Option<usize>)> = (0..pre).map(|i| (Some(i), Some(i))).collect();
    let (om, nm) = (m - pre - suf, n - pre - suf);
    if om == 0 || nm == 0 || om.saturating_mul(nm) > MAX_ALIGN {
        // By position: pair what pairs, then delete or insert the rest.
        for k in 0..om.max(nm) {
            let o = (k < om).then_some(pre + k);
            let nn = (k < nm).then_some(pre + k);
            match (o, nn) {
                (Some(a), Some(b)) if score(&old[a], &new[b]) > 0 => out.push((Some(a), Some(b))),
                (Some(a), Some(b)) => {
                    out.push((Some(a), None));
                    out.push((None, Some(b)));
                }
                (a, b) => out.push((a, b)),
            }
        }
    } else {
        // Weighted LCS over the middle.
        let w = nm + 1;
        let mut best = vec![0u64; (om + 1) * w];
        for i in (0..om).rev() {
            for j in (0..nm).rev() {
                let s = score(&old[pre + i], &new[pre + j]);
                let take = if s > 0 { s + best[(i + 1) * w + j + 1] } else { 0 };
                let skip_old = best[(i + 1) * w + j];
                let skip_new = best[i * w + j + 1];
                best[i * w + j] = take.max(skip_old).max(skip_new);
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < om || j < nm {
            if i == om {
                out.push((None, Some(pre + j)));
                j += 1;
            } else if j == nm {
                out.push((Some(pre + i), None));
                i += 1;
            } else {
                let s = score(&old[pre + i], &new[pre + j]);
                let here = best[i * w + j];
                if s > 0 && here == s + best[(i + 1) * w + j + 1] {
                    out.push((Some(pre + i), Some(pre + j)));
                    i += 1;
                    j += 1;
                } else if here == best[i * w + j + 1] {
                    out.push((None, Some(pre + j)));
                    j += 1;
                } else {
                    out.push((Some(pre + i), None));
                    i += 1;
                }
            }
        }
    }
    out.extend((0..suf).map(|k| (Some(m - suf + k), Some(n - suf + k))));
    out
}

fn same_block(a: &Arc<Block>, b: &Arc<Block>) -> bool {
    Arc::ptr_eq(a, b) || **a == **b
}

/// How alike two paragraph texts are: the common start counts most (Enter keeps the paragraph
/// with the text before the caret), then the common end.
fn para_score(a: &Paragraph, b: &Paragraph) -> u64 {
    let pre = a.text.bytes().zip(b.text.bytes()).take_while(|(x, y)| x == y).count();
    let rest = a.text.len().min(b.text.len()) - pre;
    let suf = a.text.bytes().rev().zip(b.text.bytes().rev()).take(rest).take_while(|(x, y)| x == y).count();
    1 + 4 * pre as u64 + suf as u64
}

fn block_score(a: &Arc<Block>, b: &Arc<Block>) -> u64 {
    match (&**a, &**b) {
        (Block::Para(x), Block::Para(y)) => para_score(x, y),
        (Block::Table(x), Block::Table(y)) => 1 + x.rows.iter().zip(&y.rows).filter(|(r, s)| r == s).count() as u64,
        _ => 0,
    }
}

/// Turns the story or cell `f`, which holds `old`, into `new`.
pub(crate) fn update_blocks<F: XmlFragment>(txn: &mut TransactionMut, f: &F, old: &Blocks, new: &Blocks, depth: usize) {
    let slots = block_slots(txn, f);
    if slots.len() != old.len() {
        // The shared story isn't what we last synced (it shouldn't happen): write it afresh.
        let n = f.len(txn);
        f.remove_range(txn, 0, n);
        write_blocks(txn, f, new);
        return;
    }
    let plan = align(old, new, same_block, block_score);
    let mut shift: i64 = 0;
    for (k, step) in plan.iter().enumerate() {
        match *step {
            (Some(i), Some(j)) => {
                let (Some(o), Some(n), Some((_, el))) = (old.get(i), new.get(j), slots.get(i)) else { continue };
                if !same_block(o, n) {
                    update_block(txn, f, el, o, n, depth);
                }
            }
            (Some(i), None) => {
                let Some((raw, _)) = slots.get(i) else { continue };
                let at = (*raw as i64 + shift).max(0) as u32;
                f.remove_range(txn, at, 1);
                shift -= 1;
            }
            (None, Some(j)) => {
                let Some(n) = new.get(j) else { continue };
                // Before the next old block still there, else at the end.
                let next_old = plan.iter().skip(k + 1).find_map(|(o, _)| *o);
                let at = match next_old.and_then(|i| slots.get(i)) {
                    Some((raw, _)) => (*raw as i64 + shift).max(0) as u32,
                    None => f.len(txn),
                };
                write_block(txn, f, at.min(f.len(txn)), n, depth);
                shift += 1;
            }
            (None, None) => {}
        }
    }
}

/// Updates one block element in place (or replaces it when its kind changed).
fn update_block<F: XmlFragment>(txn: &mut TransactionMut, f: &F, el: &XmlElementRef, old: &Arc<Block>, new: &Arc<Block>, depth: usize) {
    match (&**old, &**new) {
        (Block::Para(o), Block::Para(n)) => update_para(txn, el, o, n),
        (Block::Table(o), Block::Table(n)) if depth < MAX_DEPTH => update_table(txn, el, o, n, depth),
        _ => {
            // Replace: insert the new one where the old one is, then remove the old one.
            let id = AsRef::<yrs::branch::Branch>::as_ref(el).id();
            let raw = f.children(txn).position(|c| matches!(&c, XmlOut::Element(e) if AsRef::<yrs::branch::Branch>::as_ref(e).id() == id));
            if let Some(raw) = raw {
                write_block(txn, f, raw as u32, new, depth);
                f.remove_range(txn, raw as u32 + 1, 1);
            }
        }
    }
}

/// Turns paragraph element `el`, which holds `old`, into `new`: its attributes, then the text's
/// changed middle (deleted and inserted), then formatting changes on the rest.
pub(crate) fn update_para(txn: &mut TransactionMut, el: &XmlElementRef, old: &Paragraph, new: &Paragraph) {
    write_para_attrs(txn, el, Some(old), new);
    if old.text == new.text && old.runs == new.runs && old.objects == new.objects {
        return;
    }
    let t = match text_of(txn, el) {
        Some(t) => t,
        None => el.push_back(txn, XmlTextPrelim::new("")),
    };
    let ou = units(old);
    let nu = units(new);
    let (m, n) = (ou.len(), nu.len());
    let mut pre = 0;
    while pre < m && pre < n && ou.get(pre).zip(nu.get(pre)).is_some_and(|(a, b)| a.same_content(b)) {
        pre += 1;
    }
    let mut suf = 0;
    while suf < m - pre && suf < n - pre && ou.get(m - 1 - suf).zip(nu.get(n - 1 - suf)).is_some_and(|(a, b)| a.same_content(b)) {
        suf += 1;
    }
    let y = |us: &[Unit], k: usize| -> u32 { us.iter().take(k).map(Unit::ylen).sum() };
    let at = y(&ou, pre);
    let del = y(&ou, m - suf) - at;
    if del > 0 {
        t.remove_range(txn, at, del);
    }
    let keys = keys_of(&[old, new]);
    if let Some(mid) = nu.get(pre..n - suf)
        && !mid.is_empty()
    {
        insert_units(txn, &t, at, mid, &keys);
    }
    // Formatting changed on text that stayed: `format` it, run by run.
    let mut pairs: Vec<(u32, &CharProps, &CharProps, u32)> = Vec::new();
    let mut ystart = 0u32;
    for (k, u) in nu.iter().enumerate() {
        let len = u.ylen();
        let old_u = if k < pre {
            ou.get(k)
        } else if k >= n - suf {
            ou.get(m - (n - k))
        } else {
            None
        };
        if let Some(o) = old_u
            && o.props != u.props
        {
            match pairs.last_mut() {
                Some(last) if last.0 + last.3 == ystart && (std::ptr::eq(last.1, o.props) || last.1 == o.props) && last.2 == u.props => last.3 += len,
                _ => pairs.push((ystart, o.props, u.props, len)),
            }
        }
        ystart += len;
    }
    for (start, o, nw, len) in pairs {
        let d = attrs_delta(&object_of(o), &object_of(nw));
        if !d.is_empty() {
            t.format(txn, start, len, d);
        }
    }
}

/// Turns table element `el`, which holds `old`, into `new`: attributes, rows (aligned like
/// blocks), cells, and the cells' blocks.
fn update_table(txn: &mut TransactionMut, el: &XmlElementRef, old: &Table, new: &Table, depth: usize) {
    write_table_attrs(txn, el, Some(old), new);
    let trs = children_tagged(txn, el, "tr");
    if trs.len() != old.rows.len() {
        let n = el.len(txn);
        el.remove_range(txn, 0, n);
        for row in &new.rows {
            let k = el.len(txn);
            write_row(txn, el, k, row, depth);
        }
        return;
    }
    let row_score = |a: &Row, b: &Row| 1 + a.cells.iter().zip(&b.cells).filter(|(x, y)| x == y).count() as u64;
    let plan = align(&old.rows, &new.rows, |a, b| a == b, row_score);
    let mut shift: i64 = 0;
    for (k, step) in plan.iter().enumerate() {
        match *step {
            (Some(i), Some(j)) => {
                let (Some(o), Some(n), Some((_, tr))) = (old.rows.get(i), new.rows.get(j), trs.get(i)) else { continue };
                if o != n {
                    update_row(txn, tr, o, n, depth);
                }
            }
            (Some(i), None) => {
                let Some((raw, _)) = trs.get(i) else { continue };
                el.remove_range(txn, (*raw as i64 + shift).max(0) as u32, 1);
                shift -= 1;
            }
            (None, Some(j)) => {
                let Some(n) = new.rows.get(j) else { continue };
                let next_old = plan.iter().skip(k + 1).find_map(|(o, _)| *o);
                let at = match next_old.and_then(|i| trs.get(i)) {
                    Some((raw, _)) => (*raw as i64 + shift).max(0) as u32,
                    None => el.len(txn),
                };
                let at = at.min(el.len(txn));
                write_row(txn, el, at, n, depth);
                shift += 1;
            }
            (None, None) => {}
        }
    }
}

fn update_row(txn: &mut TransactionMut, tr: &XmlElementRef, old: &Row, new: &Row, depth: usize) {
    write_row_attrs(txn, tr, Some(old), new);
    let tcs = children_tagged(txn, tr, "tc");
    if tcs.len() != old.cells.len() {
        let n = tr.len(txn);
        tr.remove_range(txn, 0, n);
        for c in &new.cells {
            let k = tr.len(txn);
            write_cell(txn, tr, k, c, depth);
        }
        return;
    }
    let cell_score = |a: &Cell, b: &Cell| 1 + a.blocks.iter().zip(&b.blocks).filter(|(x, y)| same_block(x, y)).count() as u64;
    let plan = align(&old.cells, &new.cells, |a, b| a == b, cell_score);
    let mut shift: i64 = 0;
    for (k, step) in plan.iter().enumerate() {
        match *step {
            (Some(i), Some(j)) => {
                let (Some(o), Some(n), Some((_, tc))) = (old.cells.get(i), new.cells.get(j), tcs.get(i)) else { continue };
                if o != n {
                    write_cell_attrs(txn, tc, Some(o), n);
                    update_blocks(txn, tc, &o.blocks, &n.blocks, depth + 1);
                }
            }
            (Some(i), None) => {
                let Some((raw, _)) = tcs.get(i) else { continue };
                tr.remove_range(txn, (*raw as i64 + shift).max(0) as u32, 1);
                shift -= 1;
            }
            (None, Some(j)) => {
                let Some(n) = new.cells.get(j) else { continue };
                let next_old = plan.iter().skip(k + 1).find_map(|(o, _)| *o);
                let at = match next_old.and_then(|i| tcs.get(i)) {
                    Some((raw, _)) => (*raw as i64 + shift).max(0) as u32,
                    None => tr.len(txn),
                };
                let at = at.min(tr.len(txn));
                write_cell(txn, tr, at, n, depth);
                shift += 1;
            }
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_keeps_the_paragraph_with_the_text_before_the_caret() {
        let p = |t: &str| Arc::new(Block::Para(Paragraph::with_text(t, CharProps::default())));
        let old = vec![p("HelloWorld")];
        // Enter in the middle: the old paragraph keeps "Hello".
        let plan = align(&old, &[p("Hello"), p("World")], same_block, block_score);
        assert_eq!(plan, vec![(Some(0), Some(0)), (None, Some(1))]);
        // Enter at the start: an empty paragraph is inserted before.
        let plan = align(&old, &[p(""), p("HelloWorld")], same_block, block_score);
        assert_eq!(plan, vec![(None, Some(0)), (Some(0), Some(1))]);
        // Unchanged ends pair up, a table never pairs with a paragraph.
        let t = Arc::new(Block::Table(Table::new(1, 1, 100.0)));
        let plan = align(&[p("a"), p("b"), p("c")], &[p("a"), t.clone(), p("c")], same_block, block_score);
        assert_eq!(plan, vec![(Some(0), Some(0)), (None, Some(1)), (Some(1), None), (Some(2), Some(2))]);
    }

    #[test]
    fn shared_indexes_count_utf16_and_embeds() {
        let mut p = Paragraph::with_text("aé😀b", CharProps::default());
        assert_eq!(y_of_byte(&p, 0), 0);
        assert_eq!(y_of_byte(&p, 1), 1);
        assert_eq!(y_of_byte(&p, 3), 2);
        assert_eq!(y_of_byte(&p, 7), 4);
        assert_eq!(byte_of_y(&p, 4), 7);
        assert_eq!(byte_of_y(&p, 99), p.text.len());
        // A damaged object mark counts nothing.
        p.text.push(OBJ);
        if let Some(r) = p.runs.last_mut() {
            r.len += OBJ.len_utf8();
        }
        assert_eq!(units(&p).len(), 4);
    }
}
