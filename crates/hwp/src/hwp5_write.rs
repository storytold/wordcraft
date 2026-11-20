//! Write a document edited in WordCraft back into its original HWP 5.0 file.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
//!
//! The HWP reader rebuilds each section as the element tree of an HWPX section, every paragraph
//! naming its PARA_HEADER record ([`hwp5::HWP_REC`]), and DocInfo as an HWPX header. Saving writes
//! both as XML, lets the HWPX write-back ([`crate::writer::plan`]) put the edits into them, and
//! reads the result back:
//!
//! - a paragraph whose text changed gets new PARA_TEXT and PARA_CHAR_SHAPE records; its controls
//!   (section and column settings, tables, pictures ...) keep their characters and records, its
//!   line layout cache (PARA_LINE_SEG) and range tags are dropped so Hancom lays it out again;
//! - a paragraph given another paragraph shape has it set in its header;
//! - a new paragraph copies the paragraph shape and style of the one it follows; a deleted one
//!   loses its records; list headers count their paragraphs again;
//! - a table written again by the write-back (rows or columns added, merged, resized) replaces
//!   the table's records, keeping its position settings; a new table becomes a paragraph holding
//!   it, its records modelled on a table of the document; a deleted table loses its control
//!   character and records;
//! - new character shapes, paragraph shapes, border fills and fonts go into DocInfo
//!   ([`crate::hwp5_docinfo`]);
//! - every other record keeps its bytes, and only the changed streams are replaced in the compound
//!   file.
//!
//! As for HWPX, paragraphs whose text sits between controls are reported, not written.
//!
//! From the files (not the document): the high bit of a paragraph's character count marks the
//! last paragraph of its list (section, cell, text box ...), a paragraph holding only its end mark
//! has no PARA_TEXT record, and a cell's list header is 47 bytes (the cell properties, then the
//! text width and reserved bytes).

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Write};

use crate::error::{Error, Result};
use crate::hwp5::{
    self, BIN_DATA, CTRL_HEADER, ID_MAPPINGS, LIST_HEADER, MEMO_LIST, PARA_CHAR_SHAPE, PARA_HEADER, PARA_LINE_SEG, PARA_RANGE_TAG, PARA_TEXT,
    Records, SC_PICTURE, SHAPE_COMPONENT, TABLE,
};
use crate::hwp5_enc::{
    CellShows, DEFAULT_TAB, Encoder, MAX_DEPTH, OBJECT_CHAR, OrigCell, Rec, TableModel, Text, Unit, counts_text, element_text, encode, first_rec,
    look_unit, paragraph_records, put_u16, put_u32, rec_of, run_shape, set_last_in_list, shape_at, shape_runs, t_units, tables_of, tokens,
    trim_cell_end, u16_at, u32_at, units_wchars,
};
use crate::writer::{SaveReport, plan};
use crate::xml::Element;

/// Namespaces of the section XML handed to the HWPX write-back.
const NS: &str = r#"xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core""#;
/// Namespaces of the header XML handed to the write-back.
const HEAD_NS: &str = r#"xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core""#;
/// A table of an edited paragraph: one of the paragraph's own (by its order there) or one the
/// write-back made.
enum Slot {
    Keep(usize),
    New(Element),
}

/// A paragraph's text runs in groups, the groups separated by runs of objects or controls
/// (`None`: a run holding both).
fn text_groups(p: &Element) -> Option<Vec<Text>> {
    let mut groups: Vec<Text> = Vec::new();
    let mut open = false;
    for run in p.children.iter().filter(|c| c.tag == "hp:run") {
        let has_text = run.children.iter().any(|c| c.tag == "hp:t");
        let objects = run.children.iter().any(|c| c.tag != "hp:t");
        if has_text && objects {
            return None;
        }
        if objects {
            open = false;
            continue;
        }
        if !open {
            groups.push(Vec::new());
            open = true;
        }
        let shape = run_shape(run);
        let g = groups.last_mut()?;
        for t in run.children.iter().filter(|c| c.tag == "hp:t") {
            t_units(t, shape, g)?;
        }
    }
    Some(groups)
}

/// What the edited section tree says about the original paragraphs.
#[derive(Default)]
struct Changes {
    /// Paragraphs still there.
    present: HashSet<usize>,
    /// Paragraphs whose text changed, with their new text.
    /// (each group of text runs, objects between them)
    modified: HashMap<usize, Vec<Text>>,
    /// Paragraphs given another paragraph shape.
    restyled: HashMap<usize, u16>,
    /// Paragraphs whose tables changed: their tables now.
    tables: HashMap<usize, Vec<Slot>>,
    /// Paragraphs whose pictures or drawings changed (added, removed, resized, replaced): the
    /// paragraph now, and how many pictures and drawings it had.
    pictures: HashMap<usize, (Element, (usize, usize))>,
    /// New paragraphs after (or before) an original one.
    after: HashMap<usize, Vec<Element>>,
    before: HashMap<usize, Vec<Element>>,
}

/// An original paragraph: its text, paragraph shape and the first source paragraph of each of its
/// tables.
struct Original {
    text: Option<Text>,
    shape: String,
    tables: Vec<Option<usize>>,
    pics: Vec<String>,
    drawings: Vec<String>,
}

/// A paragraph's drawings and groups (not pictures), each one whole: the write-back leaves an
/// unchanged drawing as it read it and replaces a changed one, so a new fill or outline at the
/// same size counts as a change too.
fn drawings_of(p: &Element) -> Vec<String> {
    const DRAWINGS: [&str; 9] =
        ["hp:rect", "hp:ellipse", "hp:line", "hp:arc", "hp:polygon", "hp:curve", "hp:container", "hp:connectLine", "hp:textart"];
    p.children
        .iter()
        .filter(|c| c.tag == "hp:run")
        .flat_map(|r| r.children.iter().filter(|c| DRAWINGS.contains(&c.tag.as_str())))
        .map(|d| d.to_xml(""))
        .collect()
}

/// A paragraph's pictures as the write-back leaves them: each one's image and shown size.
fn pictures_of(p: &Element) -> Vec<String> {
    p.children
        .iter()
        .filter(|c| c.tag == "hp:run")
        .flat_map(|r| r.children.iter().filter(|c| c.tag == "hp:pic"))
        .map(|pic| {
            let img = pic.find("hc:img").and_then(|i| i.get("binaryItemIDRef")).unwrap_or("");
            let sz = pic.find("hp:curSz");
            format!("{img} {} {}", sz.and_then(|s| s.get("width")).unwrap_or(""), sz.and_then(|s| s.get("height")).unwrap_or(""))
        })
        .collect()
}

/// Whether paragraph `p` holds only text and pictures (it can be written again whole).
fn text_and_pictures(p: &Element) -> bool {
    p.children.iter().filter(|c| c.tag == "hp:run").flat_map(|r| &r.children).all(|c| matches!(c.tag.as_str(), "hp:t" | "hp:pic" | "hp:linesegarray"))
}

fn originals(el: &Element, out: &mut HashMap<usize, Original>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    if el.tag == "hp:p"
        && let Some(r) = rec_of(el)
    {
        let tables = tables_of(el).into_iter().map(first_rec).collect();
        out.insert(
            r,
            Original {
                text: element_text(el),
                shape: el.get("paraPrIDRef").unwrap_or("").to_string(),
                tables,
                pics: pictures_of(el),
                drawings: drawings_of(el),
            },
        );
    }
    for c in &el.children {
        originals(c, out, depth + 1);
    }
}

impl Changes {
    fn walk(&mut self, el: &Element, orig: &HashMap<usize, Original>, report: &mut SaveReport, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        // the paragraphs of this element, in order: new ones go after the paragraph before them
        let mut last: Option<usize> = None;
        let mut waiting: Vec<Element> = Vec::new();
        for c in el.children.iter().filter(|c| c.tag == "hp:p") {
            let Some(r) = rec_of(c) else {
                match last {
                    Some(r) => self.after.entry(r).or_default().push(c.clone()),
                    None => waiting.push(c.clone()),
                }
                continue;
            };
            self.present.insert(r);
            let Some(o) = orig.get(&r) else { continue };
            if pictures_of(c) != o.pics || drawings_of(c) != o.drawings {
                self.pictures.insert(r, (c.clone(), (o.pics.len(), o.drawings.len())));
            }
            // (text the writer cannot hold was not edited by it either)
            if let Some(text) = element_text(c)
                && o.text.as_ref() != Some(&text)
            {
                match text_groups(c) {
                    Some(groups) => {
                        self.modified.insert(r, groups);
                    }
                    None => report.skipped.push("a paragraph with text and objects in one run: its edit is not written".into()),
                }
            }
            let shape = c.get("paraPrIDRef").unwrap_or("");
            if o.shape != shape
                && let Ok(id) = shape.parse()
            {
                self.restyled.insert(r, id);
            }
            let tables = tables_of(c);
            if tables.iter().map(|t| first_rec(t)).collect::<Vec<_>>() != o.tables {
                let slots = tables
                    .iter()
                    .map(|t| match first_rec(t).and_then(|f| o.tables.iter().position(|x| *x == Some(f))) {
                        Some(j) => Slot::Keep(j),
                        None => Slot::New((*t).clone()),
                    })
                    .collect();
                self.tables.insert(r, slots);
            }
            if !waiting.is_empty() {
                self.before.entry(r).or_default().append(&mut waiting);
            }
            last = Some(r);
        }
        if !waiting.is_empty() {
            report.skipped.push(format!("{} new paragraph(s) with no paragraph next to them to place them by", waiting.len()));
        }
        for c in &el.children {
            // new paragraphs and tables the write-back made are written whole, not walked
            let made = (c.tag == "hp:p" && rec_of(c).is_none()) || (c.tag == "hp:tbl" && first_rec(c).is_none());
            if !made {
                self.walk(c, orig, report, depth + 1);
            }
        }
    }
}

/// A table's records (control, TABLE, first cell's list header) from its control record `c`.
fn table_model(r: &Records, c: usize) -> TableModel {
    let kids = r.children(c);
    let t = kids.iter().position(|&x| r.tag(x) == TABLE);
    TableModel {
        ctrl: Some(r.data(c).to_vec()),
        table: t.and_then(|t| kids.get(t)).map(|&x| r.data(x).to_vec()),
        cell: t.and_then(|t| kids.get(t + 1..)).and_then(|rest| rest.iter().find(|&&x| r.tag(x) == LIST_HEADER)).map(|&x| r.data(x).to_vec()),
        ..Default::default()
    }
}

/// [`table_model`] with the table's cells as they are (to keep the ones that do not change).
fn table_model_with_cells(r: &Records, c: usize) -> TableModel {
    let mut model = table_model(r, c);
    let base = r.recs.get(c).map_or(0, |x| x.level + 1);
    let mut cells: Vec<OrigCell> = Vec::new();
    let mut caption = Vec::new();
    let mut seen_table = false;
    for &k in r.children(c) {
        match r.tag(k) {
            // the caption's list comes before the TABLE record, the cells' after it
            TABLE => seen_table = true,
            _ if !seen_table => subtree(r, k, base, &mut caption, 0),
            LIST_HEADER => cells.push(OrigCell { recs: Vec::new(), paras: 0, look: Some(CellShows::default()), controls: false }),
            PARA_HEADER => {
                let Some(cell) = cells.last_mut() else { continue };
                cell.paras += 1;
                let mut shows = cell.look.take().unwrap_or_default();
                let ok = para_look(r, k, &mut shows, &mut cell.controls, 0);
                cell.look = ok.then_some(shows);
                subtree(r, k, base, &mut cell.recs, 0);
            }
            _ => {}
        }
    }
    for c in &mut cells {
        if let Some(l) = c.look.as_mut() {
            trim_cell_end(&mut l.text, 0);
        }
    }
    model.cells = cells;
    model.caption = caption;
    model
}

/// What paragraph `k` shows (see [`CellShows`]), tables in it included; whether it holds
/// controls goes into `controls`. `false` when too deep to tell.
fn para_look(r: &Records, k: usize, shows: &mut CellShows, controls: &mut bool, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let mut units = Vec::new();
    let mut tables = Vec::new();
    for &x in r.children(k) {
        match r.tag(x) {
            PARA_TEXT => units.extend(tokens(r.data(x)).iter().filter_map(|t| t.unit).map(look_unit)),
            CTRL_HEADER => {
                *controls = true;
                match hwp5::ctrl_id(r.data(x)).as_str() {
                    // a drawing object or a form control (both shapes in the edited document); a
                    // picture's stored image as the reader gives it (`binaryItemIDRef`)
                    "gso " | "form" => {
                        shows.objects += 1;
                        // its shown size, as the edited document's hp:curSz gives it
                        if let Some(&s) = r.children(x).iter().find(|&&s| r.tag(s) == SHAPE_COMPONENT) {
                            shows.sizes.push((u32_at(r.data(s), 28), u32_at(r.data(s), 32)));
                        }
                        let pic = r
                            .children(x)
                            .iter()
                            .filter(|&&s| r.tag(s) == SHAPE_COMPONENT)
                            .flat_map(|&s| r.children(s))
                            .find(|&&p| r.tag(p) == SC_PICTURE);
                        if let Some(&p) = pic {
                            let d = r.data(p);
                            shows.pictures.push(d.get(71..73).map_or(0, |b| u16::from_le_bytes([b[0], b[1]])).to_string());
                        }
                    }
                    "tbl " => tables.push(x),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if counts_text(&units, !tables.is_empty()) {
        shows.text.push(units);
    }
    for t in tables {
        let mut from = shows.text.len();
        for &y in r.children(t) {
            match r.tag(y) {
                LIST_HEADER => {
                    trim_cell_end(&mut shows.text, from);
                    from = shows.text.len();
                    shows.cells += 1;
                }
                PARA_HEADER if !para_look(r, y, shows, controls, depth + 1) => return false,
                _ => {}
            }
        }
        trim_cell_end(&mut shows.text, from);
    }
    true
}

/// Record `k` and everything under it, levels counted from `base`.
fn subtree(r: &Records, k: usize, base: u32, out: &mut Vec<Rec>, depth: usize) {
    let Some(rec) = r.recs.get(k).filter(|_| depth <= MAX_DEPTH) else { return };
    out.push((rec.tag, rec.level.saturating_sub(base), r.data(k).to_vec()));
    for &c in r.children(k) {
        subtree(r, c, base, out, depth + 1);
    }
}

fn is_table(r: &Records, c: usize) -> bool {
    r.tag(c) == CTRL_HEADER && hwp5::ctrl_id(r.data(c)) == "tbl "
}

/// Rebuilds one section's record stream.
struct Out<'r, 'a> {
    r: &'r Records<'a>,
    ch: Changes,
    deleted: HashSet<usize>,
    /// Paragraph count changes of list headers.
    delta: HashMap<usize, i64>,
    /// Where in `recs` the list headers whose count `delta` changed are: their counts are set
    /// from the paragraphs really written under them once all is written (a new paragraph the
    /// encoder refuses is not there).
    counted: Vec<usize>,
    enc: Encoder,
    recs: Vec<Rec>,
    /// Edits not written, and how many paragraph edits and tables that undoes.
    skipped: Vec<String>,
    refused: usize,
    tables: usize,
}

impl Out<'_, '_> {
    fn push_raw(&mut self, k: usize) {
        let Some(rec) = self.r.recs.get(k) else { return };
        self.recs.push((rec.tag, rec.level, self.r.data(k).to_vec()));
    }

    /// Record `k` and everything under it as they were (a table whose paragraphs the write-back
    /// replaced, kept when the new one is not written).
    fn emit_verbatim(&mut self, k: usize, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        self.push_raw(k);
        for &c in self.r.children(k) {
            self.emit_verbatim(c, depth + 1);
        }
    }

    fn emit(&mut self, k: usize, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let r = self.r;
        let Some(level) = r.recs.get(k).map(|x| x.level) else { return };
        let tag = r.tag(k);
        if tag == PARA_HEADER {
            for p in self.ch.before.remove(&k).unwrap_or_default() {
                self.new_para(&p, k, level);
            }
            if self.deleted.contains(&k) {
                return;
            }
            let after = self.ch.after.remove(&k).unwrap_or_default();
            // The section's memos hang under its last paragraph, after its own records: they are
            // kept apart from the paragraph whichever way it is written (a picture edit writes only
            // its controls again), and move to a paragraph added after it (a file with them
            // elsewhere does not open in Hancom's viewer).
            let kids = r.children(k);
            let memo_at = kids.iter().position(|&c| r.tag(c) == MEMO_LIST).unwrap_or(kids.len());
            let (own, memos) = kids.split_at(memo_at);
            let at = self.recs.len();
            let text = self.ch.modified.remove(&k);
            let slots = self.ch.tables.remove(&k);
            let pictures = self.ch.pictures.remove(&k);
            // a paragraph with no controls of its own (plain text) is written again whole
            let plain = own.iter().all(|&c| r.tag(c) != CTRL_HEADER);
            if let Some((p, counts)) = pictures.as_ref().filter(|_| slots.is_none() && !plain)
                && let Some((wchars, controls)) = self.splice_pictures(own, p, *counts, level, text.as_deref())
            {
                // pictures edited beside other controls: those keep their records
                let (head, para_text, char_shape, merged) = paragraph_records(r.data(k), &wchars);
                if merged {
                    self.enc.merged_shapes.set(self.enc.merged_shapes.get() + 1);
                }
                self.recs.push((PARA_HEADER, level, head));
                if let Some(t) = para_text {
                    self.recs.push((PARA_TEXT, level + 1, t));
                }
                self.recs.push((PARA_CHAR_SHAPE, level + 1, char_shape));
                for c in controls {
                    match c {
                        Control::Kept(c) => self.emit(c, depth + 1),
                        Control::Resized(c, w, h) => self.emit_resized(c, w, h),
                        Control::New(recs) => self.recs.extend(recs),
                    }
                }
            } else if let Some((p, _)) = pictures.as_ref().filter(|p| plain || (text_and_pictures(&p.0) && own.iter().all(|&c| !is_table(r, c))))
                && let Some(recs) = self.enc.paragraph(p, r.data(k), level, 0)
            {
                // pictures and text edited together: the paragraph written again from its text and
                // pictures (it holds nothing else)
                self.recs.extend(recs);
            } else if text.is_some() || slots.is_some() {
                if pictures.is_some() {
                    self.skipped.push(format!("paragraph record {k}: its picture edits are not written (it holds more than text and pictures)"));
                }
                self.rebuild(k, own, text.as_deref(), slots, depth);
            } else {
                if pictures.is_some() {
                    self.skipped.push(format!("paragraph record {k}: its picture edits are not written (it holds more than text and pictures)"));
                }
                self.push_raw(k);
                for &c in own {
                    self.emit(c, depth + 1);
                }
            }
            if let Some(shape) = self.ch.restyled.remove(&k)
                && let Some(head) = self.recs.get_mut(at).filter(|x| x.0 == PARA_HEADER)
            {
                put_u16(&mut head.2, 8, shape);
            }
            for p in &after {
                self.new_para(p, k, level);
            }
            for &c in memos {
                self.emit(c, depth + 1);
            }
            return;
        }
        match self.delta.get(&k) {
            Some(&d) if tag == LIST_HEADER && d != 0 => {
                let mut data = r.data(k).to_vec();
                let n = i64::from(u16_at(&data, 0)) + d;
                put_u16(&mut data, 0, u16::try_from(n.max(1)).unwrap_or(1));
                self.counted.push(self.recs.len());
                self.recs.push((tag, level, data));
            }
            _ => self.push_raw(k),
        }
        for &c in r.children(k) {
            self.emit(c, depth + 1);
        }
    }

    /// What becomes of paragraph `k`'s tables (its table controls `tables`) given the write-back's
    /// `slots`: a new table in place of one, or tables removed. `None` when the change is not one
    /// of those.
    fn table_plan(tables: &[usize], slots: Vec<Slot>) -> Option<HashMap<usize, Option<Element>>> {
        let mut plan = HashMap::new();
        if slots.len() == tables.len() {
            for (i, s) in slots.into_iter().enumerate() {
                match s {
                    Slot::Keep(j) if j == i => {}
                    Slot::New(el) => {
                        plan.insert(*tables.get(i)?, Some(el));
                    }
                    Slot::Keep(_) => return None,
                }
            }
            return Some(plan);
        }
        let kept: Vec<usize> = slots.iter().map(|s| if let Slot::Keep(j) = s { Some(*j) } else { None }).collect::<Option<_>>()?;
        if slots.len() > tables.len() || kept.windows(2).any(|w| w[0] >= w[1]) {
            return None;
        }
        for (i, &t) in tables.iter().enumerate() {
            if !kept.contains(&i) {
                plan.insert(t, None);
            }
        }
        Some(plan)
    }

    /// Paragraph `k` (with its child records `kids`) with new text and/or changed tables. What
    /// cannot be placed is reported and written as it was.
    fn rebuild(&mut self, k: usize, kids: &[usize], text: Option<&[Text]>, slots: Option<Vec<Slot>>, depth: usize) {
        let r = self.r;
        let toks = kids.iter().find(|&&c| r.tag(c) == PARA_TEXT).map(|&c| tokens(r.data(c))).unwrap_or_default();
        let shapes = kids.iter().find(|&&c| r.tag(c) == PARA_CHAR_SHAPE).map(|&c| shape_runs(r.data(c))).unwrap_or_default();
        // each extended control character and its CTRL_HEADER, in order
        let ctrls: Vec<usize> = kids.iter().copied().filter(|&c| r.tag(c) == CTRL_HEADER).collect();
        let mut next = ctrls.iter();
        let tok_ctrl: Vec<Option<usize>> = toks.iter().map(|t| if t.extended() { next.next().copied() } else { None }).collect();
        let tables: Vec<usize> = ctrls.iter().copied().filter(|&c| is_table(r, c)).collect();
        // tables the edited paragraph still has as they were (their own edits are written)
        let kept: HashSet<usize> = match &slots {
            Some(s) => s.iter().filter_map(|x| if let Slot::Keep(j) = x { tables.get(*j).copied() } else { None }).collect(),
            None => tables.iter().copied().collect(),
        };
        let plan = match slots.map(|s| Self::table_plan(&tables, s)) {
            Some(None) => {
                self.skipped.push(format!("paragraph record {k}: its tables changed in a way not written yet"));
                HashMap::new()
            }
            Some(Some(p)) => p,
            None => HashMap::new(),
        };
        // where new text goes: each group of text characters (between controls) takes the
        // edited group at its place; a paragraph without text takes it at its end
        let mut groups: Vec<(usize, usize)> = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            match groups.last_mut() {
                Some(g) if t.unit.is_some() && g.1 == i => g.1 = i + 1,
                _ if t.unit.is_some() => groups.push((i, i + 1)),
                _ => {}
            }
        }
        let mut text = text;
        let placed = match text {
            Some(new) if new.len() == groups.len() && !groups.is_empty() => true,
            Some(new) if groups.is_empty() && new.len() <= 1 => true,
            Some(new) => {
                self.skipped.push(format!(
                    "paragraph record {k}: text between controls that do not match ({} pieces, {} edited); its edit is not written",
                    groups.len(),
                    new.len()
                ));
                self.refused += 1;
                false
            }
            None => false,
        };
        if !placed {
            text = None;
        }
        if text.is_none() && plan.is_empty() {
            self.push_raw(k);
            for &c in kids {
                if is_table(r, c) && !kept.contains(&c) { self.emit_verbatim(c, depth + 1) } else { self.emit(c, depth + 1) }
            }
            return;
        }
        let tab = toks.iter().find(|t| t.unit == Some(Unit::Tab)).map(|t| t.w.clone()).unwrap_or_else(|| self.enc.tab.clone());
        let mut wchars: Vec<(u16, u32)> = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            if let Some(new) = text
                && let Some(g) = groups.iter().position(|g| g.0 <= i && i < g.1)
            {
                if groups.get(g).is_some_and(|x| x.0 == i)
                    && let Some(units) = new.get(g)
                {
                    wchars.extend(units_wchars(units, &tab));
                }
                continue;
            }
            if tok_ctrl.get(i).copied().flatten().is_some_and(|c| matches!(plan.get(&c), Some(None))) {
                continue; // a deleted table's character
            }
            wchars.extend(t.w.iter().enumerate().map(|(j, &w)| (w, shape_at(&shapes, t.pos + j))));
        }
        if let Some(new) = text
            && groups.is_empty()
            && let Some(units) = new.first()
        {
            wchars.extend(units_wchars(units, &tab));
        }
        let end_pos = toks.last().map(|t| t.pos + t.w.len()).unwrap_or(0);
        wchars.push((13, shape_at(&shapes, end_pos)));
        let Some(level) = r.recs.get(k).map(|x| x.level) else { return };
        let (head, para_text, char_shape, merged) = paragraph_records(r.data(k), &wchars);
        if merged {
            self.enc.merged_shapes.set(self.enc.merged_shapes.get() + 1);
        }
        self.recs.push((PARA_HEADER, level, head));
        if let Some(t) = para_text {
            self.recs.push((PARA_TEXT, level + 1, t));
        }
        self.recs.push((PARA_CHAR_SHAPE, level + 1, char_shape));
        for &c in kids {
            if [PARA_TEXT, PARA_CHAR_SHAPE, PARA_LINE_SEG, PARA_RANGE_TAG].contains(&r.tag(c)) {
                continue;
            }
            match plan.get(&c) {
                Some(None) => self.tables += 1,
                Some(Some(el)) => {
                    let model = table_model_with_cells(r, c);
                    match self.enc.table_records(el, level + 1, &model, true, 0) {
                        Some(recs) => {
                            self.recs.extend(recs);
                            self.tables += 1;
                            let lost = model.lost.get();
                            if lost > 0 {
                                self.skipped.push(format!(
                                    "paragraph record {k}: {lost} table cell(s) holding fields or objects were changed or removed; what they held is not kept"
                                ));
                            }
                            let numbers = model.numbers_lost.get();
                            if numbers > 0 {
                                self.skipped.push(format!(
                                    "paragraph record {k}: {numbers} table cell(s) written again lose their {}",
                                    crate::hwpx_out::NUMBERS_LEFT_OUT
                                ));
                            }
                        }
                        None => {
                            self.skipped
                                .push(format!("paragraph record {k}: a table holding fields or objects could not be written; it is kept as it was"));
                            self.emit_verbatim(c, depth + 1);
                        }
                    }
                }
                None if is_table(r, c) && !kept.contains(&c) => self.emit_verbatim(c, depth + 1),
                None => self.emit(c, depth + 1),
            }
        }
    }

    /// The text and controls of a paragraph (its child records `kids`, at `level`) whose pictures
    /// or drawings were edited, the paragraph now being `p` (which had `counts` pictures and
    /// drawings): its other controls and text as they were; pictures and drawings removed,
    /// resized in place, or re-made when replaced; new ones put after the object before them in
    /// `p`, else before its text (when they come first) or at its end. With `text`, the
    /// paragraph's edited text groups take the place of its own (groups that only removed
    /// objects stood between count as one). `None` when the objects cannot be told apart or the
    /// text does not fall into the same groups.
    fn splice_pictures(
        &self,
        kids: &[usize],
        p: &Element,
        counts: (usize, usize),
        level: u32,
        text: Option<&[Text]>,
    ) -> Option<(Vec<(u16, u32)>, Vec<Control>)> {
        let r = self.r;
        let toks = kids.iter().find(|&&c| r.tag(c) == PARA_TEXT).map(|&c| tokens(r.data(c))).unwrap_or_default();
        let shapes = kids.iter().find(|&&c| r.tag(c) == PARA_CHAR_SHAPE).map(|&c| shape_runs(r.data(c))).unwrap_or_default();
        let ctrls: Vec<usize> = kids.iter().copied().filter(|&c| r.tag(c) == CTRL_HEADER).collect();
        if toks.iter().filter(|t| t.extended()).count() != ctrls.len() {
            return None;
        }
        // the original objects: (control, element kind, BinData reference of a picture, shown size)
        let object = |c: usize| -> Option<(&'static str, Option<u16>, u32, u32)> {
            let sc = r.children(c).iter().copied().find(|&x| r.tag(x) == SHAPE_COMPONENT)?;
            let d = r.data(sc);
            let kind = match d.get(..4)? {
                b"cip$" => "hp:pic",
                b"cer$" => "hp:rect",
                b"lle$" => "hp:ellipse",
                b"nil$" => "hp:line",
                b"cra$" => "hp:arc",
                b"lop$" => "hp:polygon",
                b"ruc$" => "hp:curve",
                b"noc$" => "hp:container",
                _ => return None,
            };
            let bin = if kind == "hp:pic" {
                Some(r.children(sc).iter().copied().find(|&x| r.tag(x) == SC_PICTURE).map(|pd| u16_at(r.data(pd), 71))?)
            } else {
                None
            };
            Some((kind, bin, u32_at(d, 28), u32_at(d, 32)))
        };
        let originals: Vec<(usize, (&str, Option<u16>, u32, u32))> = ctrls.iter().filter_map(|&c| object(c).map(|x| (c, x))).collect();
        let pictures = originals.iter().filter(|o| o.1.0 == "hp:pic").count();
        if (pictures, originals.len() - pictures) != counts {
            return None;
        }
        let (kept, after, first, last) = match_objects(p, &originals)?;
        let obj_index: HashMap<usize, usize> = originals.iter().enumerate().map(|(j, o)| (o.0, j)).collect();
        let removed = |c: usize| obj_index.get(&c).is_some_and(|j| !kept.contains_key(j));
        let new = |el: &Element| {
            if el.tag == "hp:pic" { self.enc.picture_records(el, level + 1) } else { self.enc.drawing_records(el, level + 1, 0) }.map(Control::New)
        };
        let mut wchars: Vec<(u16, u32)> = Vec::new();
        let mut controls: Vec<Control> = Vec::new();
        let first_text = toks.iter().position(|t| t.unit.is_some());
        // edited text: each group of text characters (between controls) takes the edited group
        // at its place, as in `rebuild`; groups with only removed objects between them are one;
        // a paragraph without text takes it at its end
        let mut ctrl_at: HashMap<usize, usize> = HashMap::new(); // token -> its control
        let mut it = ctrls.iter();
        for (i, t) in toks.iter().enumerate() {
            if t.extended() {
                ctrl_at.insert(i, *it.next()?);
            }
        }
        let groups = text_groups_of(&toks, |k| text.is_some() && ctrl_at.get(&k).is_some_and(|&c| removed(c)));
        if let Some(new) = text
            && !(new.len() == groups.len() || (groups.is_empty() && new.len() <= 1))
        {
            return None;
        }
        let tab = toks.iter().find(|t| t.unit == Some(Unit::Tab)).map(|t| t.w.clone()).unwrap_or_else(|| self.enc.tab.clone());
        let push_new = |els: &[&Element], shape: u32, wchars: &mut Vec<(u16, u32)>, controls: &mut Vec<Control>| -> Option<()> {
            for el in els {
                wchars.extend(OBJECT_CHAR.iter().map(|&w| (w, shape)));
                controls.push(new(el)?);
            }
            Some(())
        };
        for (i, t) in toks.iter().enumerate() {
            let shape = shape_at(&shapes, t.pos);
            if Some(i) == first_text {
                push_new(&first, shape, &mut wchars, &mut controls)?;
            }
            if let Some(new) = text
                && let Some(g) = groups.iter().position(|g| g.0 <= i && i < g.1)
            {
                if groups.get(g).is_some_and(|x| x.0 == i)
                    && let Some(units) = new.get(g)
                {
                    wchars.extend(units_wchars(units, &tab));
                }
                continue;
            }
            if !t.extended() {
                wchars.extend(t.w.iter().enumerate().map(|(k, &w)| (w, shape_at(&shapes, t.pos + k))));
                continue;
            }
            let c = *ctrl_at.get(&i)?;
            match obj_index.get(&c) {
                Some(j) => {
                    match kept.get(j) {
                        None => {} // removed
                        Some((el, true)) => {
                            let size = |a: &str| el.find("hp:curSz").and_then(|s| s.get(a)).and_then(|v| v.parse::<u32>().ok());
                            wchars.extend(t.w.iter().map(|&w| (w, shape)));
                            controls.push(Control::Resized(c, size("width")?, size("height")?));
                        }
                        Some((_, false)) => {
                            wchars.extend(t.w.iter().map(|&w| (w, shape)));
                            controls.push(Control::Kept(c));
                        }
                    }
                    push_new(after.get(j).map(Vec::as_slice).unwrap_or(&[]), shape, &mut wchars, &mut controls)?;
                }
                None => {
                    wchars.extend(t.w.iter().map(|&w| (w, shape)));
                    controls.push(Control::Kept(c));
                }
            }
        }
        let end_pos = toks.last().map(|t| t.pos + t.w.len()).unwrap_or(0);
        let shape = shape_at(&shapes, end_pos);
        if first_text.is_none() {
            push_new(&first, shape, &mut wchars, &mut controls)?;
            if let Some(units) = text.and_then(|t| t.first()) {
                wchars.extend(units_wchars(units, &tab));
            }
        }
        push_new(&last, shape, &mut wchars, &mut controls)?;
        wchars.push((13, shape));
        Some((wchars, controls))
    }

    /// Picture control `c` (and everything under it) as it was, shown `w` x `h` HWPUNIT: the
    /// control's size, its component's current size and scale matrix changed, nothing else (a
    /// picture's records re-made from the section tree lose what the viewer needs: measured, a
    /// page with a re-made background picture showed nothing).
    fn emit_resized(&mut self, c: usize, w: u32, h: u32) {
        let r = self.r;
        let Some(level) = r.recs.get(c).map(|x| x.level) else { return };
        let mut ctrl = r.data(c).to_vec();
        // the size may be relative (a share of the page in 1/100 %: a background picture 4267 high
        // is 42.67 % of the page); the new one is absolute (width bits 15-17 = 4, height bits
        // 18-19 = 2, public specification table 70)
        let attr = (u32_at(&ctrl, 4) & !(0b111 << 15) & !(0b11 << 18)) | (4 << 15) | (2 << 18);
        put_u32(&mut ctrl, 4, attr);
        put_u32(&mut ctrl, 16, w);
        put_u32(&mut ctrl, 20, h);
        self.recs.push((CTRL_HEADER, level, ctrl));
        for &k in r.children(c) {
            if r.tag(k) != SHAPE_COMPONENT {
                self.emit_verbatim(k, 0);
                continue;
            }
            // component: the id twice (8), offset (8), group level and version (4), original
            // size (8), current size (8), flips (4), rotation and centre (10), matrix count (2),
            // the translation (48), then the first scale matrix (e1 at +0, e5 at +32)
            let mut sc = r.data(k).to_vec();
            let (ow, oh) = (u32_at(&sc, 20).max(1), u32_at(&sc, 24).max(1));
            put_u32(&mut sc, 28, w);
            put_u32(&mut sc, 32, h);
            put_u32(&mut sc, 42, w / 2);
            put_u32(&mut sc, 46, h / 2);
            for (at, v) in [(100, f64::from(w) / f64::from(ow)), (132, f64::from(h) / f64::from(oh))] {
                if let Some(slot) = sc.get_mut(at..at + 8) {
                    slot.copy_from_slice(&v.to_le_bytes());
                }
            }
            self.recs.push((SHAPE_COMPONENT, r.recs.get(k).map_or(level + 1, |x| x.level), sc));
            for &x in r.children(k) {
                self.emit(x, 1); // a text box's own edits too
            }
        }
    }

    /// A new paragraph after (or before) paragraph `anchor`, at `level`.
    fn new_para(&mut self, p: &Element, anchor: usize, level: u32) {
        let header = self.r.data(anchor).to_vec();
        match self.enc.paragraph(p, &header, level, 0) {
            Some(recs) => {
                self.tables += recs.iter().filter(|x| x.0 == TABLE).count();
                self.recs.extend(recs);
            }
            None => self.skipped.push("a new paragraph holding objects other than text and tables is not written yet".into()),
        }
    }
}

/// Sets the paragraph count of each list header at `at` (indexes into `recs`) to the paragraphs
/// written under it: the PARA_HEADER records at its level that follow it, up to the first other
/// record at that level or above.
fn set_list_counts(recs: &mut [Rec], at: &[usize]) {
    for &i in at {
        let Some(level) = recs.get(i).filter(|x| x.0 == LIST_HEADER).map(|x| x.1) else { continue };
        let n = recs
            .get(i + 1..)
            .unwrap_or(&[])
            .iter()
            .take_while(|x| x.1 > level || (x.1 == level && x.0 == PARA_HEADER))
            .filter(|x| x.1 == level)
            .count();
        if let Some(lh) = recs.get_mut(i) {
            put_u16(&mut lh.2, 0, u16::try_from(n.max(1)).unwrap_or(u16::MAX));
        }
    }
}

/// A control of a paragraph written again: one of its own (record index, written as it was, its
/// own edits included), one resized, or new records.
enum Control {
    Kept(usize),
    /// A picture of its own shown at another size (width, height HWPUNIT).
    Resized(usize, u32, u32),
    New(Vec<Rec>),
}

/// What became of a paragraph's original objects (`originals`: control, kind, BinData reference,
/// shown size) in its edited form `p`: the ones kept (original index -> element, whether
/// resized), and new ones after a kept one, before the text, or at the end. A picture still in
/// the file names its BinData by number; a drawing read from the file has no instance id (the
/// writer gives new ones one). `None` when an object cannot be matched.
#[allow(clippy::type_complexity)]
fn match_objects<'e>(
    p: &'e Element,
    originals: &[(usize, (&str, Option<u16>, u32, u32))],
) -> Option<(HashMap<usize, (&'e Element, bool)>, HashMap<usize, Vec<&'e Element>>, Vec<&'e Element>, Vec<&'e Element>)> {
    const KINDS: [&str; 8] = ["hp:pic", "hp:rect", "hp:ellipse", "hp:line", "hp:arc", "hp:polygon", "hp:curve", "hp:container"];
    let mut edited: Vec<(&Element, bool)> = Vec::new();
    let mut text_seen = false;
    for run in p.children.iter().filter(|c| c.tag == "hp:run") {
        for c in &run.children {
            match c.tag.as_str() {
                "hp:t" if !c.text.is_empty() || !c.children.is_empty() => text_seen = true,
                tag if KINDS.contains(&tag) => edited.push((c, text_seen)),
                _ => {}
            }
        }
    }
    let mut next = 0;
    let mut kept: HashMap<usize, (&Element, bool)> = HashMap::new();
    let mut after: HashMap<usize, Vec<&Element>> = HashMap::new();
    let (mut first, mut last): (Vec<&Element>, Vec<&Element>) = (Vec::new(), Vec::new());
    let mut prev: Option<usize> = None;
    for (el, text_before) in edited {
        let existing = match el.tag.as_str() {
            "hp:pic" => el.find("hc:img").and_then(|i| i.get("binaryItemIDRef")).and_then(|v| v.parse::<u16>().ok()).map(Some),
            _ => el.get("instid").is_none().then_some(None),
        };
        let Some(bin) = existing else {
            match prev {
                Some(j) => after.entry(j).or_default().push(el),
                None if !text_before => first.push(el),
                None => last.push(el),
            }
            continue;
        };
        // An object read from the file names its control record: two drawings of a kind are
        // told apart (the second of two, its first deleted, is not taken for the first).
        let rec = el.get(hwp5::HWP_REC).and_then(|v| v.parse::<usize>().ok());
        let j = next
            + originals.get(next..)?.iter().position(|o| match rec {
                Some(rec) => o.0 == rec,
                None => o.1.0 == el.tag && (bin.is_none() || o.1.1 == bin),
            })?;
        next = j + 1;
        let o = originals.get(j)?.1;
        let size = |a: &str| el.find("hp:curSz").and_then(|s| s.get(a)).and_then(|v| v.parse::<u32>().ok());
        kept.insert(j, (el, size("width") != Some(o.2) || size("height") != Some(o.3)));
        prev = Some(j);
    }
    Some((kept, after, first, last))
}

/// The groups of text characters among `toks` (start, end token): runs of text, two runs with
/// only tokens `gap` says may go (removed objects) between them counting as one.
fn text_groups_of(toks: &[crate::hwp5_enc::Tok], gap: impl Fn(usize) -> bool) -> Vec<(usize, usize)> {
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if t.unit.is_none() {
            continue;
        }
        match groups.last_mut() {
            Some(g) if g.1 == i => g.1 = i + 1,
            Some(g) if (g.1..i).all(&gap) => g.1 = i + 1,
            _ => groups.push((i, i + 1)),
        }
    }
    groups
}

/// The records as a stream, each paragraph's last-in-list bit set from where it now stands.
fn serialize(recs: &mut [Rec]) -> Vec<u8> {
    set_last_in_list(recs);
    encode(recs)
}

/// The first tab of a section, to copy into new text.
fn section_tab(r: &Records) -> Vec<u16> {
    r.recs
        .iter()
        .enumerate()
        .filter(|(_, x)| x.tag == PARA_TEXT)
        .flat_map(|(k, _)| tokens(r.data(k)))
        .find(|t| t.unit == Some(Unit::Tab))
        .map(|t| t.w)
        .unwrap_or_else(|| DEFAULT_TAB.to_vec())
}

/// Each paragraph record's list header, for lists that count their paragraphs.
fn list_headers(r: &Records) -> HashMap<usize, usize> {
    let mut out = HashMap::new();
    let mut groups: Vec<&[usize]> = vec![r.roots.as_slice()];
    groups.extend(r.recs.iter().map(|x| x.children.as_slice()));
    for g in groups {
        for (lh, paras) in r.lists(g) {
            for p in paras {
                out.insert(p, lh);
            }
        }
    }
    out
}

/// Does paragraph `k` hold the section definition (the section's first paragraph) or the
/// section's memos (its last)?
fn holds_section(r: &Records, k: usize) -> bool {
    r.children(k).iter().any(|&c| r.tag(c) == MEMO_LIST || (r.tag(c) == CTRL_HEADER && hwp5::ctrl_id(r.data(c)) == "secd"))
}

/// The paragraph records inside paragraph `k` (in its tables, text boxes and notes, at any depth).
fn paragraphs_under(r: &Records, k: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = r.children(k).iter().map(|&c| (c, 1)).collect();
    while let Some((c, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        if r.tag(c) == PARA_HEADER {
            out.push(c);
        }
        stack.extend(r.children(c).iter().map(|&x| (x, depth + 1)));
    }
    out
}

/// The paragraph header new paragraphs in new cells copy: the first of a cell, else the first.
fn para_model(r: &Records) -> Vec<u8> {
    let first = (0..r.recs.len()).find(|&k| r.tag(k) == PARA_HEADER);
    let in_cell = (0..r.recs.len()).find(|&k| r.tag(k) == PARA_HEADER && r.recs.get(k).is_some_and(|x| x.level > 0));
    in_cell.or(first).map(|k| r.data(k).to_vec()).unwrap_or_else(|| vec![0; 24])
}

/// One section's stream with the edits of `edited` (the section XML after write-back); `None`
/// when nothing changed.
fn rewrite_section(
    data: &[u8],
    original: &Element,
    edited: &str,
    bins: &HashMap<String, u16>,
    picture_keys: &HashMap<String, u64>,
    report: &mut SaveReport,
) -> Result<Option<Vec<u8>>> {
    let edited = crate::xml::parse(edited.as_bytes(), false)?;
    let r = Records::parse(data);
    // Records nested deeper than the write-back follows would be left out of what it writes:
    // such a section is kept as it was, its edits reported, rather than written in part.
    if r.recs.iter().any(|x| usize::try_from(x.level).map_or(true, |l| l > MAX_DEPTH)) {
        report.skipped.push(format!("a section nests its records more than {MAX_DEPTH} deep; its edits are not written (it is kept as it was)"));
        return Ok(None);
    }
    let mut orig = HashMap::new();
    originals(original, &mut orig, 0);
    let mut ch = Changes::default();
    ch.walk(&edited, &orig, report, 0);
    let mut deleted: HashSet<usize> = orig.keys().copied().filter(|k| !ch.present.contains(k)).collect();
    for k in deleted.clone() {
        if holds_section(&r, k) {
            // Kept as it was, with the paragraphs of its tables and text boxes: left out, they
            // would leave its cells without paragraphs, which Hancom's viewer can't open.
            deleted.remove(&k);
            for p in paragraphs_under(&r, k) {
                deleted.remove(&p);
            }
            report.skipped.push("a paragraph holding the section's page settings or memos was deleted; it is kept".into());
        }
    }
    let unchanged = ch.modified.is_empty()
        && ch.restyled.is_empty()
        && ch.tables.is_empty()
        && ch.pictures.is_empty()
        && ch.after.is_empty()
        && ch.before.is_empty();
    if deleted.is_empty() && unchanged {
        return Ok(None);
    }
    let lists = list_headers(&r);
    let mut delta: HashMap<usize, i64> = HashMap::new();
    for k in &deleted {
        if let Some(&lh) = lists.get(k) {
            *delta.entry(lh).or_insert(0) -= 1;
        }
    }
    for (k, v) in ch.after.iter().chain(ch.before.iter()) {
        if let Some(&lh) = lists.get(k) {
            *delta.entry(lh).or_insert(0) += v.len() as i64;
        }
    }
    let tab = section_tab(&r);
    let model = (0..r.recs.len()).find(|&c| is_table(&r, c)).map(|c| table_model(&r, c)).unwrap_or_default();
    let para_model = para_model(&r);
    let enc = Encoder {
        tab,
        model,
        para_model,
        left_out: Default::default(),
        bins: bins.clone(),
        picture_keys: picture_keys.clone(),
        strict: true,
        memos: Default::default(),
        memo_fields: crate::hwp5_enc::memo_fields(&edited),
        merged_shapes: Default::default(),
    };
    let mut out = Out { r: &r, ch, deleted, delta, counted: Vec::new(), enc, recs: Vec::new(), skipped: Vec::new(), refused: 0, tables: 0 };
    for &k in &r.roots {
        out.emit(k, 0);
    }
    set_list_counts(&mut out.recs, &out.counted);
    report.skipped.append(&mut out.skipped);
    let objects = out.enc.left_out.replace(0);
    if objects > 0 {
        report.skipped.push(format!("{objects} picture(s) or shape(s) in new paragraphs are not written yet"));
    }
    let merged = out.enc.merged_shapes.replace(0);
    if merged > 0 {
        report.skipped.push(format!("{merged} paragraph(s) change character shape more than 65,535 times; the rest of each keeps the last shape"));
    }
    report.modified = report.modified.saturating_sub(out.refused);
    report.tables = report.tables.max(out.tables);
    let mut recs = out.recs;
    Ok(Some(serialize(&mut recs)))
}

/// Write the edits of `edited` (DOCX bytes of the document opened from the HWP 5.0 file
/// `original`) into a copy of it. With `docs` (the document before and after editing) formatting
/// and table edits are written too.
pub fn write_back_hwp(
    original: &[u8],
    edited: &[u8],
    docs: Option<(&wordcraft_doc::Document, &wordcraft_doc::Document)>,
) -> Result<(Vec<u8>, SaveReport)> {
    let pkg = hwp5::open(original)?;
    let mut trees = Vec::new();
    let mut texts = Vec::new();
    for name in &pkg.section_names {
        let data = pkg.read(name).ok_or_else(|| Error::format(format!("missing stream {name}")))?;
        let tree = hwp5::section(data);
        texts.push(tree.to_xml(NS));
        trees.push(tree);
    }
    let head = pkg.hwp5_header.as_ref().filter(|_| docs.is_some());
    let mut plan = plan(&pkg, texts.clone(), head.map(|h| h.to_xml(HEAD_NS)), edited, docs, true, true)?;
    let mut streams: Vec<(String, Vec<u8>)> = Vec::new();
    // new shapes go into DocInfo; when they cannot, only the text is written
    if let (Some(new_head), Some(old_head)) = (plan.header.as_deref(), head) {
        let added = crate::xml::parse(new_head.as_bytes(), false)
            .ok()
            .and_then(|after| read_stream(original, "DocInfo").ok().map(|d| (after, d)))
            .and_then(|(after, d)| crate::hwp5_docinfo::add_shapes(&d, old_head, &after, &plan.sources));
        match added {
            Some(recs) => streams.push(("DocInfo".into(), encode(&recs))),
            None => {
                plan = crate::writer::plan(&pkg, texts, None, edited, None, false, false)?;
                plan.report.skipped.push("formatting and table changes could not be written to this .hwp".into());
            }
        }
    }
    // new pictures: BIN_DATA records after the file's, their streams; pictures the sections hold
    // already keep their BinData ids
    let mut bins: HashMap<String, u16> = HashMap::new();
    if !plan.media.is_empty() {
        let info = match streams.iter().position(|s| s.0 == "DocInfo") {
            Some(i) => streams.remove(i).1,
            None => read_stream(original, "DocInfo")?,
        };
        let (info, count, ids) = add_pictures(&info, &plan.media)?;
        for i in 1..=count {
            bins.insert(i.to_string(), i);
        }
        for ((part, item, _, data), (index, id)) in plan.media.iter().zip(ids) {
            let ext = part.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_else(|| "png".into());
            bins.insert(item.clone(), index);
            streams.push((format!("BinData/BIN{id:04X}.{ext}"), data.clone()));
        }
        streams.push(("DocInfo".into(), info));
    } else if let Ok(info) = read_stream(original, "DocInfo") {
        let r = Records::parse(&info);
        let count = (0..r.recs.len()).filter(|&k| r.tag(k) == BIN_DATA).count().min(usize::from(u16::MAX)) as u16;
        for i in 1..=count {
            bins.insert(i.to_string(), i);
        }
    }
    // what each picture item holds (the original's by number, the new ones by name), so a
    // table written again knows its unchanged pictures (see `match_cells`)
    let image = |d: &[u8]| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        d.hash(&mut h);
        h.finish()
    };
    let mut picture_keys: HashMap<String, u64> = HashMap::new();
    for it in pkg.items.iter().filter(|it| it.id.parse::<u16>().is_ok()) {
        if let Some((_, d)) = pkg.binary(&it.id) {
            picture_keys.insert(it.id.clone(), image(d));
        }
    }
    for (_, item, _, data) in &plan.media {
        picture_keys.insert(item.clone(), image(data));
    }
    let mut report = plan.report;
    report.tables = 0;
    for ((name, tree), text) in pkg.section_names.iter().zip(&trees).zip(&plan.sections) {
        let (Some(text), Some(data)) = (text, pkg.read(name)) else { continue };
        if let Some(new) = rewrite_section(data, tree, text, &bins, &picture_keys, &mut report)? {
            streams.push((name.to_string(), new));
        }
    }
    if streams.is_empty() {
        return Ok((original.to_vec(), report));
    }
    Ok((replace_streams(original, &streams)?, report))
}

/// DocInfo `data` with a BIN_DATA record for each of `media` (embedded, compressed as the file
/// is, its extension) after the file's own, and the picture count of ID_MAPPINGS raised; with
/// how many BIN_DATA records the file had and, for each new one, its place in the list (what a
/// picture refers to: records are counted, their stored ids need not follow) and its stored id
/// (the stream's name, after the highest the file uses).
fn add_pictures(data: &[u8], media: &[crate::writer::NewMedia]) -> Result<(Vec<u8>, u16, Vec<(u16, u16)>)> {
    let r = Records::parse(data);
    let mut recs: Vec<Rec> = (0..r.recs.len()).map(|k| (r.tag(k), r.recs.get(k).map(|x| x.level).unwrap_or(0), r.data(k).to_vec())).collect();
    let count = u16::try_from(recs.iter().filter(|x| x.0 == BIN_DATA).count()).map_err(|_| Error::TooLarge("pictures"))?;
    let used = recs.iter().filter(|x| x.0 == BIN_DATA).map(|x| u16_at(&x.2, 2)).max().unwrap_or(0);
    // record numbers and stored ids are 16-bit: past 65,535 a new picture would take an
    // existing one's number or stream, so the save stops instead
    let step = |base: u16, k: usize| u16::try_from(k + 1).ok().and_then(|n| base.checked_add(n)).ok_or(Error::TooLarge("pictures"));
    let ids: Vec<(u16, u16)> = (0..media.len()).map(|k| Ok((step(count, k)?, step(used, k)?))).collect::<Result<_>>()?;
    let at = recs.iter().rposition(|x| x.0 == BIN_DATA).or_else(|| recs.iter().position(|x| x.0 == ID_MAPPINGS)).map_or(recs.len(), |i| i + 1);
    let new: Vec<Rec> = media
        .iter()
        .zip(&ids)
        .map(|((part, ..), (_, id))| {
            let ext = part.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_else(|| "png".into());
            let mut d = 1u16.to_le_bytes().to_vec();
            d.extend_from_slice(&id.to_le_bytes());
            d.extend_from_slice(&(ext.encode_utf16().count() as u16).to_le_bytes());
            d.extend(ext.encode_utf16().flat_map(u16::to_le_bytes));
            (BIN_DATA, 1, d)
        })
        .collect();
    recs.splice(at..at, new);
    if let Some(m) = recs.iter_mut().find(|x| x.0 == ID_MAPPINGS)
        && let Some(count) = m.2.get(0..4)
    {
        let n = i32::from_le_bytes([count[0], count[1], count[2], count[3]]).saturating_add(media.len() as i32);
        m.2.splice(0..4, n.to_le_bytes());
    }
    Ok((encode(&recs), count, ids))
}

fn damaged(e: std::io::Error) -> Error {
    Error::format(format!("damaged HWP file ({e})"))
}

/// The compound file, and whether its streams are compressed (FileHeader flag).
fn open_cfb(original: &[u8]) -> Result<(cfb::CompoundFile<Cursor<Vec<u8>>>, bool)> {
    let mut c = cfb::CompoundFile::open(Cursor::new(original.to_vec())).map_err(damaged)?;
    let mut head = Vec::new();
    Read::take(c.open_stream("/FileHeader").map_err(damaged)?, 256).read_to_end(&mut head).map_err(damaged)?;
    Ok((c, u32_at(&head, 36) & 1 != 0))
}

/// A stream of the compound file, decompressed when the file is compressed.
fn read_stream(original: &[u8], name: &str) -> Result<Vec<u8>> {
    let (mut c, compressed) = open_cfb(original)?;
    let mut raw = Vec::new();
    Read::take(c.open_stream(format!("/{name}")).map_err(damaged)?, 256 * 1024 * 1024).read_to_end(&mut raw).map_err(damaged)?;
    if compressed { hwp5::inflate(&raw) } else { Ok(raw) }
}

/// A copy of the compound file with streams replaced (compressed when the file is).
fn replace_streams(original: &[u8], streams: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let (mut c, compressed) = open_cfb(original)?;
    for (name, data) in streams {
        let bytes = if compressed { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.clone() };
        // a file without pictures has no BinData storage yet
        if let Some((parent, _)) = name.rsplit_once('/') {
            c.create_storage_all(format!("/{parent}")).map_err(damaged)?;
        }
        let mut s = c.create_stream(format!("/{name}")).map_err(damaged)?;
        s.write_all(&bytes).map_err(damaged)?;
    }
    c.flush().map_err(damaged)?;
    Ok(c.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new picture takes the next record number and stored id; when an original picture
    /// already has id 65,535 there is none left, and the save stops instead of writing the new
    /// image over that picture's stream (`BINFFFF`).
    #[test]
    fn new_pictures_never_reuse_a_stored_id() {
        let bin = |id: u16| {
            let mut d = 1u16.to_le_bytes().to_vec();
            d.extend(id.to_le_bytes());
            d.extend(3u16.to_le_bytes());
            d.extend("png".encode_utf16().flat_map(u16::to_le_bytes));
            encode(&[(BIN_DATA, 1, d)])
        };
        let media: Vec<crate::writer::NewMedia> = vec![("BinData/image9.png".into(), "image9".into(), "image/png", vec![1, 2, 3])];
        let (_, count, ids) = add_pictures(&bin(7), &media).unwrap();
        assert_eq!((count, ids), (1, vec![(2, 8)]));
        assert_eq!(add_pictures(&bin(u16::MAX), &media).unwrap_err(), Error::TooLarge("pictures"));
    }

    /// A drawing whose fill changed at the same size is a change too (the write-back replaces
    /// it); before, only the kind and size were compared, and the old fill was kept silently.
    #[test]
    fn a_drawing_with_a_new_fill_is_changed() {
        let para = |fill: &str| {
            crate::xml::parse(
                format!(r#"<hp:p><hp:run><hp:rect><hp:curSz width="100" height="50"/><hc:fillBrush><hc:winBrush faceColor="{fill}"/></hc:fillBrush></hp:rect></hp:run></hp:p>"#).as_bytes(),
                false,
            )
            .unwrap()
        };
        assert_eq!(drawings_of(&para("#FF0000")), drawings_of(&para("#FF0000")));
        assert_ne!(drawings_of(&para("#FF0000")), drawings_of(&para("#0000FF")));
    }

    /// Every record kept as it is and serialised again gives the stream back byte for byte (the
    /// last-in-list bits recomputed included). Over real files: `HWP_SAMPLES=<folder> cargo test
    /// -p wordcraft-hwp records_round_trip -- --ignored`.
    #[test]
    #[ignore = "reads the HWP samples named by HWP_SAMPLES"]
    fn records_round_trip() {
        let Ok(dir) = std::env::var("HWP_SAMPLES") else { return };
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut checked = 0;
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "hwp") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(pkg) = hwp5::open(&bytes) else { continue };
            for name in &pkg.section_names {
                let Some(data) = pkg.read(name) else { continue };
                let r = Records::parse(data);
                let mut out = Out {
                    r: &r,
                    ch: Changes::default(),
                    deleted: HashSet::new(),
                    delta: HashMap::new(),
                    counted: Vec::new(),
                    enc: Encoder {
                        tab: DEFAULT_TAB.to_vec(),
                        model: TableModel::default(),
                        para_model: Vec::new(),
                        left_out: Default::default(),
                        bins: Default::default(),
                        picture_keys: Default::default(),
                        strict: true,
                        memos: Default::default(),
                        memo_fields: Default::default(),
                        merged_shapes: Default::default(),
                    },
                    recs: Vec::new(),
                    skipped: Vec::new(),
                    refused: 0,
                    tables: 0,
                };
                for &k in &r.roots {
                    out.emit(k, 0);
                }
                let again = serialize(&mut out.recs);
                assert!(again == data, "{} {name}: {} bytes vs {}", path.display(), again.len(), data.len());
                checked += 1;
            }
        }
        assert!(checked > 0, "no HWP sections checked");
    }

    /// One record's bytes: header (tag, level, size) and data.
    fn record(tag: u32, level: u32, data: &[u8]) -> Vec<u8> {
        let mut out = (tag | (level << 10) | ((data.len() as u32) << 20)).to_le_bytes().to_vec();
        out.extend_from_slice(data);
        out
    }

    /// PARA_TEXT of `text`, with a table's control character after it when `table`.
    fn para_text(text: &str, table: bool) -> Vec<u8> {
        let mut w: Vec<u16> = text.encode_utf16().collect();
        if table {
            w.extend([11, 0, 0, 0, 0, 0, 0, 11]);
        }
        w.push(13);
        w.iter().flat_map(|c| c.to_le_bytes()).collect()
    }

    /// A deleted paragraph holding the section's settings is kept, and so are the paragraphs of
    /// the table in it: without them its cell had no paragraph left (a document trimmed from
    /// the top that way didn't open in Hancom's viewer).
    #[test]
    fn a_kept_section_paragraph_keeps_its_cell_paragraphs() {
        let mut secd = b"dces".to_vec();
        secd.resize(40, 0);
        let mut tbl = crate::hwp5_enc::TABLE_ID.to_vec();
        tbl.resize(46, 0);
        let mut first: Vec<u16> = vec![2, 0, 0, 0, 0, 0, 0, 2, 11, 0, 0, 0, 0, 0, 0, 11, 13];
        first.insert(16, u16::from(b'x'));
        let first: Vec<u8> = first.iter().flat_map(|c| c.to_le_bytes()).collect();
        let mut sec = record(PARA_HEADER, 0, &[0; 24]);
        sec.extend(record(PARA_TEXT, 1, &first));
        sec.extend(record(CTRL_HEADER, 1, &secd));
        sec.extend(record(CTRL_HEADER, 1, &tbl));
        sec.extend(record(TABLE, 2, &[0; 22]));
        let mut cell = vec![0u8; 47];
        cell[0] = 1; // one paragraph
        sec.extend(record(LIST_HEADER, 2, &cell));
        sec.extend(record(PARA_HEADER, 2, &[0; 24]));
        sec.extend(record(PARA_TEXT, 3, &para_text("in", false)));
        sec.extend(record(PARA_HEADER, 0, &[0; 24]));
        sec.extend(record(PARA_TEXT, 1, &para_text("end", false)));
        let original = hwp5::section(&sec);
        assert_eq!(original.children.iter().filter(|c| c.tag == "hp:p").count(), 2, "{}", original.to_xml(""));
        // the edit: the first paragraph (and its table) deleted, the last one's text changed
        let edited = original.to_xml("").replacen(&original.children[0].to_xml(""), "", 1).replace(">end<", ">end!<");
        let mut report = SaveReport::default();
        let out = rewrite_section(&sec, &original, &edited, &HashMap::new(), &HashMap::new(), &mut report).unwrap().unwrap();
        assert!(report.skipped.iter().any(|s| s.contains("it is kept")), "{report:?}");
        let r = Records::parse(&out);
        let lh = (0..r.recs.len()).find(|&k| r.tag(k) == LIST_HEADER).unwrap();
        assert_eq!(r.recs.get(lh + 1).map(|x| (x.tag, x.level)), Some((PARA_HEADER, 2)), "the cell keeps its paragraph");
    }

    /// A cell paragraph holding text and a table reads as two paragraphs (the text, then the
    /// table alone); when the outer table is written again, the cell must still look as it did,
    /// or the fields and objects only its original records hold are lost.
    #[test]
    fn cell_with_text_and_table_matches_its_edited_form() {
        let mut tbl = crate::hwp5_enc::TABLE_ID.to_vec();
        tbl.resize(46, 0);
        let mut sec = record(CTRL_HEADER, 0, &tbl);
        sec.extend(record(TABLE, 1, &[0; 22]));
        sec.extend(record(LIST_HEADER, 1, &[0; 47]));
        sec.extend(record(PARA_HEADER, 1, &[0; 24]));
        sec.extend(record(PARA_TEXT, 2, &para_text("x", true)));
        sec.extend(record(CTRL_HEADER, 2, &tbl));
        sec.extend(record(TABLE, 3, &[0; 22]));
        sec.extend(record(LIST_HEADER, 3, &[0; 47]));
        sec.extend(record(PARA_HEADER, 3, &[0; 24]));
        sec.extend(record(PARA_TEXT, 4, &para_text("in", false)));
        let r = Records::parse(&sec);
        let model = table_model_with_cells(&r, 0);
        let orig = model.cells.first().and_then(|c| c.look.clone());
        let edited = br#"<hp:subList xmlns:hp="h"><hp:p><hp:run><hp:t>x</hp:t></hp:run></hp:p><hp:p><hp:run><hp:tbl><hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>in</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl></hp:run></hp:p><hp:p><hp:run/></hp:p></hp:subList>"#;
        let sub = crate::xml::parse(edited, false).unwrap();
        let paras: Vec<&Element> = sub.children.iter().filter(|c| c.tag == "hp:p").collect();
        let new = crate::hwp5_enc::cell_look(&paras);
        assert_eq!(orig.as_ref().map(|s| (s.text.len(), s.cells)), Some((2, 1)), "{orig:?}");
        assert_eq!(new, orig);
    }

    /// A table on a text line reads as an invisible text box holding it (no outline, no fill,
    /// no margins), and a table written from the edited document has that box: the cell still
    /// looks as it did. A box with an outline is a drawn object, as before.
    #[test]
    fn cell_with_a_table_on_its_text_line_matches_its_edited_form() {
        let mut tbl = crate::hwp5_enc::TABLE_ID.to_vec();
        tbl.resize(46, 0);
        let mut sec = record(CTRL_HEADER, 0, &tbl);
        sec.extend(record(TABLE, 1, &[0; 22]));
        sec.extend(record(LIST_HEADER, 1, &[0; 47]));
        sec.extend(record(PARA_HEADER, 1, &[0; 24]));
        sec.extend(record(PARA_TEXT, 2, &para_text("x", true)));
        sec.extend(record(CTRL_HEADER, 2, &tbl));
        sec.extend(record(TABLE, 3, &[0; 22]));
        sec.extend(record(LIST_HEADER, 3, &[0; 47]));
        sec.extend(record(PARA_HEADER, 3, &[0; 24]));
        sec.extend(record(PARA_TEXT, 4, &para_text("in", false)));
        let r = Records::parse(&sec);
        let orig = table_model_with_cells(&r, 0).cells.first().and_then(|c| c.look.clone());
        let edited = |style: &str| {
            format!(
                r#"<hp:subList xmlns:hp="h" xmlns:hc="c"><hp:p><hp:run><hp:t>x</hp:t><hp:rect><hp:lineShape style="{style}"/><hp:drawText><hp:textMargin left="0" right="0" top="0" bottom="0"/><hp:subList><hp:p><hp:run><hp:tbl><hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>in</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl></hp:run></hp:p><hp:p><hp:run/></hp:p></hp:subList></hp:drawText></hp:rect></hp:run></hp:p></hp:subList>"#
            )
        };
        let look = |xml: String| {
            let sub = crate::xml::parse(xml.as_bytes(), false).unwrap();
            let paras: Vec<&Element> = sub.children.iter().filter(|c| c.tag == "hp:p").collect();
            crate::hwp5_enc::cell_look(&paras)
        };
        assert!(orig.is_some());
        assert_eq!(look(edited("NONE")), orig);
        assert_eq!(look(edited("SOLID")).map(|s| (s.objects, s.cells)), Some((1, 0)));
    }

    /// A fixed-width space (character control 31) reads as U+3000, and a table written from the
    /// edited document has that character: the cell still looks as it did.
    #[test]
    fn cell_with_a_fixed_width_space_matches_its_edited_form() {
        let mut tbl = crate::hwp5_enc::TABLE_ID.to_vec();
        tbl.resize(46, 0);
        let text: Vec<u8> = ['가' as u16, 31, '나' as u16, 13].iter().flat_map(|c| c.to_le_bytes()).collect();
        let mut sec = record(CTRL_HEADER, 0, &tbl);
        sec.extend(record(TABLE, 1, &[0; 22]));
        sec.extend(record(LIST_HEADER, 1, &[0; 47]));
        sec.extend(record(PARA_HEADER, 1, &[0; 24]));
        sec.extend(record(PARA_TEXT, 2, &text));
        let r = Records::parse(&sec);
        let orig = table_model_with_cells(&r, 0).cells.first().and_then(|c| c.look.clone());
        let sub =
            crate::xml::parse("<hp:subList xmlns:hp=\"h\"><hp:p><hp:run><hp:t>가\u{3000}나</hp:t></hp:run></hp:p></hp:subList>".as_bytes(), false)
                .unwrap();
        let paras: Vec<&Element> = sub.children.iter().filter(|c| c.tag == "hp:p").collect();
        assert!(orig.is_some());
        assert_eq!(crate::hwp5_enc::cell_look(&paras), orig);
    }
    /// A section whose records nest deeper than the write-back follows is kept as it was and
    /// its edits reported, not written without the records past that depth. Before, an edit to
    /// another paragraph wrote the section and dropped them without a notice.
    #[test]
    fn a_section_nested_too_deep_is_kept_whole() {
        let mut sec = record(PARA_HEADER, 0, &[0; 24]);
        sec.extend(record(PARA_TEXT, 1, &para_text("deep", false)));
        for level in 1..=300u32 {
            sec.extend(record(CTRL_HEADER, level, b"    "));
        }
        sec.extend(record(PARA_HEADER, 0, &[0; 24]));
        sec.extend(record(PARA_TEXT, 1, &para_text("end", false)));
        let original = hwp5::section(&sec);
        let edited = original.to_xml("").replace(">end<", ">end!<");
        let mut report = SaveReport::default();
        let out = rewrite_section(&sec, &original, &edited, &HashMap::new(), &HashMap::new(), &mut report).unwrap();
        assert!(out.is_none(), "kept as it was");
        assert!(report.skipped.iter().any(|s| s.contains("deep")), "{report:?}");
    }
    /// A new paragraph in a table cell that the encoder refuses (an equation, not written yet)
    /// doesn't count in the cell's paragraph count: the count is the paragraphs written. Before,
    /// the cell said two paragraphs and held one.
    #[test]
    fn a_refused_new_cell_paragraph_is_not_counted() {
        let mut tbl = crate::hwp5_enc::TABLE_ID.to_vec();
        tbl.resize(46, 0);
        let mut sec = record(PARA_HEADER, 0, &[0; 24]);
        sec.extend(record(PARA_TEXT, 1, &para_text("", true)));
        sec.extend(record(CTRL_HEADER, 1, &tbl));
        sec.extend(record(TABLE, 2, &[0; 22]));
        let mut cell = vec![0u8; 47];
        cell[0] = 1; // one paragraph
        sec.extend(record(LIST_HEADER, 2, &cell));
        sec.extend(record(PARA_HEADER, 2, &[0; 24]));
        sec.extend(record(PARA_TEXT, 3, &para_text("in", false)));
        sec.extend(record(PARA_HEADER, 0, &[0; 24]));
        sec.extend(record(PARA_TEXT, 1, &para_text("end", false)));
        let original = hwp5::section(&sec);
        let xml = original.to_xml("");
        let cell_end = xml.find("</hp:p>").expect("the cell paragraph") + "</hp:p>".len();
        let added = r#"<hp:p paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:equation/></hp:run></hp:p>"#;
        let edited = format!("{}{added}{}", &xml[..cell_end], &xml[cell_end..]);
        let mut report = SaveReport::default();
        let out = rewrite_section(&sec, &original, &edited, &HashMap::new(), &HashMap::new(), &mut report).unwrap().expect("written");
        assert!(report.skipped.iter().any(|s| s.contains("new paragraph")), "{report:?}");
        let r = Records::parse(&out);
        let lh = (0..r.recs.len()).find(|&k| r.tag(k) == LIST_HEADER).unwrap();
        let level = r.recs[lh].level;
        let paras = r.recs[lh + 1..]
            .iter()
            .take_while(|x| x.level > level || (x.level == level && x.tag == PARA_HEADER))
            .filter(|x| x.level == level)
            .count();
        assert_eq!((u16_at(r.data(lh), 0), paras), (1, 1));
    }
}
