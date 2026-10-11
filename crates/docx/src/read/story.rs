//! Stories: blocks, paragraphs, runs, fields, drawings, tables.

use std::sync::Arc;

use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
use wordcraft_doc::graphic::{Embedded, Graphic, GraphicItem, GraphicKind};
use wordcraft_doc::para::{Anchor, Float, FloatAlign, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{CharProps, NumChange, PropChange, Rgb};
use wordcraft_doc::table::{Cell, MAX_COLS, MAX_ROWS, Row, Table};
use wordcraft_doc::{Block, Blocks, InlineObject, Paragraph, PartKind, RevisionKind, Run, para_block};

use super::Reader;
use super::props::{sectpr, tcpr, trpr};
use crate::package::{Rels, rt};
use crate::units::{int, measure};
use crate::xml::El;

/// Deepest table nesting we build (deeper tables become plain paragraphs).
const MAX_TABLE_DEPTH: usize = 24;
/// Deepest field nesting we track.
const MAX_FIELD_DEPTH: usize = 32;
/// Deepest text-box-in-text-box nesting.
const MAX_STORY_DEPTH: usize = 4;
/// Longest field code we keep (Zotero's citation codes embed item data as JSON).
const MAX_INSTR: usize = 2 * 1024 * 1024;

/// Inherited run context (from `w:hyperlink`, `w:ins`, `w:del` wrappers).
#[derive(Clone, Default)]
pub struct RunCtx {
    link: Option<String>,
    ins: Option<u32>,
    del: Option<u32>,
}

#[derive(PartialEq)]
enum Phase {
    Instr,
    Result,
}

enum Item {
    Text(String),
    Obj(InlineObject),
}

struct FieldState {
    instr: String,
    phase: Phase,
    spilled: bool,
    buf: Vec<(Item, CharProps)>,
    props: CharProps,
    locked: bool,
    /// Kept as a range field: its result flows into the document as ordinary content between
    /// `FieldStart` and `FieldEnd` markers instead of being buffered.
    range: bool,
}

/// Fields kept as ranges: those whose result another program writes, with formatting and often
/// several paragraphs (citation managers' `ADDIN` fields: Zotero, Mendeley, EndNote).
fn is_range_instr(instr: &str) -> bool {
    instr.trim_start().get(..5).is_some_and(|k| k.eq_ignore_ascii_case("ADDIN"))
}

/// The last paragraph of a block list: its last block, or inside a final table the last
/// paragraph of its last cell (nested tables to [`MAX_TABLE_DEPTH`]).
fn last_para_mut(out: &mut Blocks, depth: usize) -> Option<&mut Paragraph> {
    match Arc::make_mut(out.last_mut()?) {
        Block::Para(p) => Some(p),
        Block::Table(t) if depth < MAX_TABLE_DEPTH => last_para_mut(&mut t.rows.last_mut()?.cells.last_mut()?.blocks, depth + 1),
        Block::Table(_) => None,
    }
}

/// The field that buffers content now: the innermost one that isn't a range field, unless it
/// has spilled (then content goes straight into the paragraph).
fn sink(sc: &mut StoryCtx) -> Option<&mut FieldState> {
    match sc.fields.iter_mut().rev().find(|f| !f.range) {
        Some(f) if !f.spilled => Some(f),
        _ => None,
    }
}

/// Per-story parsing state (fields may span paragraphs; block-level markers wait for the next
/// paragraph).
#[derive(Default)]
pub struct StoryCtx {
    fields: Vec<FieldState>,
    ignored_begins: usize,
    pending: Vec<InlineObject>,
    /// Text-box nesting depth.
    pub story_depth: usize,
}

/// Paragraph content under construction.
#[derive(Default)]
struct PB {
    text: String,
    runs: Vec<Run>,
    objects: Vec<InlineObject>,
}

impl PB {
    fn push_text(&mut self, s: &str, props: &CharProps) {
        if s.is_empty() {
            return;
        }
        self.text.push_str(s);
        self.push_run(s.len(), props);
    }
    fn push_obj(&mut self, o: InlineObject, props: &CharProps) {
        self.text.push(wordcraft_doc::para::OBJ);
        self.objects.push(o);
        self.push_run(wordcraft_doc::para::OBJ.len_utf8(), props);
    }
    fn push_run(&mut self, len: usize, props: &CharProps) {
        match self.runs.last_mut() {
            Some(r) if r.props == *props => r.len += len,
            _ => self.runs.push(Run { len, props: props.clone() }),
        }
    }
}

/// Clean text from `w:t`: drop characters with special meaning in the model and control
/// characters XML producers shouldn't have put there.
fn clean_text(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\n' | '\r' => Some(' '),
            '\t' => Some('\t'),
            wordcraft_doc::para::OBJ => None,
            c if (c as u32) < 0x20 => None,
            c => Some(c),
        })
        .collect()
}

fn children_of_choice(ac: &El) -> Option<&El> {
    // Prefer a Choice whose requirements we understand; else the Fallback; else any Choice.
    const KNOWN: &[&str] = &["wps", "wpg", "w14", "w15", "wp14", "a14", "w16se", "w16cid", "w16", "w16cex", "w16sdtdh", "v"];
    let ok = |c: &El| c.attr("Requires").unwrap_or("").split_whitespace().all(|r| KNOWN.contains(&r));
    ac.children("mc:Choice").find(|c| ok(c)).or_else(|| ac.child("mc:Fallback")).or_else(|| ac.child("mc:Choice"))
}

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
                        let lines = first.props.drop_cap;
                        let mut props = p.props.clone();
                        first.append(p);
                        props.drop_cap = lines;
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
                "w:sdt" => {
                    if let Some(c) = e.child("w:sdtContent") {
                        self.read_blocks(sc, c, rels, out, depth + 1);
                    }
                }
                "w:customXml" | "w:smartTag" | "w:ins" | "w:moveTo" => self.read_blocks(sc, e, rels, out, depth + 1),
                "mc:AlternateContent" => {
                    if let Some(c) = children_of_choice(e) {
                        self.read_blocks(sc, c, rels, out, depth + 1);
                    }
                }
                "w:bookmarkStart" | "w:bookmarkEnd" | "w:commentRangeStart" | "w:commentRangeEnd" => {
                    if let Some(o) = self.marker(e) {
                        sc.pending.push(o);
                    }
                }
                _ => {}
            }
        }
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

    fn marker(&mut self, e: &El) -> Option<InlineObject> {
        let id = e.attr("w:id").unwrap_or("").to_string();
        match e.name.as_str() {
            "w:bookmarkStart" => {
                let name = e.attr("w:name").unwrap_or("").to_string();
                if name.is_empty() || name.len() > 256 {
                    return None;
                }
                self.bookmarks.insert(id, name.clone());
                Some(InlineObject::BookmarkStart { name })
            }
            "w:bookmarkEnd" => self.bookmarks.get(&id).map(|n| InlineObject::BookmarkEnd { name: n.clone() }),
            "w:commentRangeStart" => self.comment_id(&id).map(|id| InlineObject::CommentStart { id }),
            "w:commentRangeEnd" => {
                let cid = self.comment_id(&id)?;
                self.comments_ended.insert(cid);
                Some(InlineObject::CommentEnd { id: cid })
            }
            _ => None,
        }
    }

    fn comment_id(&self, file_id: &str) -> Option<u32> {
        self.comment_map.get(file_id).copied()
    }

    fn read_para(&mut self, sc: &mut StoryCtx, e: &El, rels: &Rels, depth: usize) -> Paragraph {
        let mut pb = PB::default();
        let mut para = Paragraph::new();
        for o in std::mem::take(&mut sc.pending) {
            pb.push_obj(o, &CharProps::default());
        }
        if let Some(ppr) = e.child("w:pPr") {
            let (props, mark) = self.pc.ppr(ppr);
            para.props = props;
            para.mark = mark;
            // A tracked paragraph mark (ECMA-376 §17.13.5.16 / §17.13.5.15): `w:ins` / `w:del`
            // in the mark's `w:rPr`.
            if let Some(rpr) = ppr.child("w:rPr") {
                for k in rpr.els() {
                    match k.name.as_str() {
                        "w:ins" | "w:moveTo" => para.mark.ins = Some(self.revision(RevisionKind::Insert, k)),
                        "w:del" | "w:moveFrom" => para.mark.del = Some(self.revision(RevisionKind::Delete, k)),
                        _ => {}
                    }
                }
                para.mark.fmt_change = self.rpr_change(rpr);
            }
            // Tracked formatting changes (§17.13.5.29, §17.13.5.19): the paragraph properties
            // before the change, and a change of the list numbering.
            if let Some(ch) = ppr.child("w:pPrChange") {
                let old = ch.child("w:pPr").map(|o| self.pc.ppr(o).0.formatting()).unwrap_or_default();
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
        para.text = pb.text;
        para.runs = pb.runs;
        para.objects = pb.objects;
        para.normalize();
        para
    }

    /// A tracked run formatting change in `rpr` (`w:rPrChange`, §17.13.5.31): the formatting
    /// before it. A change nested inside it is ignored.
    fn rpr_change(&mut self, rpr: &El) -> Option<Box<PropChange<CharProps>>> {
        let ch = rpr.child("w:rPrChange")?;
        let old = ch.child("w:rPr").map(|o| self.pc.rpr(o).formatting()).unwrap_or_default();
        PropChange::boxed(self.revision(RevisionKind::Format, ch), old)
    }

    pub fn read_section(&mut self, s: &El, rels: &Rels) -> wordcraft_doc::SectionProps {
        let (mut sp, refs) = sectpr(s);
        // §17.13.5.32: the section's properties before a tracked change (no header references).
        if let Some(ch) = s.child("w:sectPrChange") {
            let old = ch.child("w:sectPr").map(|o| sectpr(o).0).unwrap_or_default();
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

    fn read_inline_children(&mut self, sc: &mut StoryCtx, pb: &mut PB, e: &El, rels: &Rels, ctx: &RunCtx, depth: usize) {
        if depth > crate::xml::MAX_DEPTH {
            return;
        }
        for k in e.els() {
            self.read_inline(sc, pb, k, rels, ctx, depth);
        }
    }

    fn read_inline(&mut self, sc: &mut StoryCtx, pb: &mut PB, k: &El, rels: &Rels, ctx: &RunCtx, depth: usize) {
        match k.name.as_str() {
            "w:r" => self.read_run(sc, pb, k, rels, ctx, depth),
            "w:hyperlink" => {
                let mut c = ctx.clone();
                let url = k.attr("r:id").and_then(|id| rels.by_id(id)).map(|r| r.target.clone());
                let anchor = k.attr("w:anchor").filter(|a| !a.is_empty());
                c.link = match (url, anchor) {
                    (Some(u), Some(a)) => Some(format!("{u}#{a}")),
                    (Some(u), None) => Some(u),
                    (None, Some(a)) => Some(format!("#{a}")),
                    (None, None) => ctx.link.clone(),
                };
                self.read_inline_children(sc, pb, k, rels, &c, depth + 1);
            }
            "w:ins" | "w:moveTo" => {
                let mut c = ctx.clone();
                c.ins = Some(self.revision(RevisionKind::Insert, k));
                self.read_inline_children(sc, pb, k, rels, &c, depth + 1);
            }
            "w:del" | "w:moveFrom" => {
                let mut c = ctx.clone();
                c.del = Some(self.revision(RevisionKind::Delete, k));
                self.read_inline_children(sc, pb, k, rels, &c, depth + 1);
            }
            "w:fldSimple" => {
                let instr = k.attr("w:instr").unwrap_or("").to_string();
                let props = k.child("w:r").and_then(|r| r.child("w:rPr")).map(|p| self.run_props(p, ctx)).unwrap_or_else(|| self.run_props_none(ctx));
                self.begin_field(sc, props, instr, on_off_attr(k, "w:fldLock"));
                self.separate_field(sc, pb);
                self.read_inline_children(sc, pb, k, rels, ctx, depth + 1);
                self.end_field(sc, pb);
            }
            "w:bookmarkStart" | "w:bookmarkEnd" | "w:commentRangeStart" | "w:commentRangeEnd" => {
                if let Some(o) = self.marker(k) {
                    let props = self.run_props_none(ctx);
                    // Markers go straight into the paragraph (never into a field's cached result).
                    pb.push_obj(o, &props);
                }
            }
            "w:sdt" => {
                if let Some(c) = k.child("w:sdtContent") {
                    self.read_inline_children(sc, pb, c, rels, ctx, depth + 1);
                }
            }
            "w:smartTag" | "w:customXml" | "w:dir" | "w:bdo" => self.read_inline_children(sc, pb, k, rels, ctx, depth + 1),
            "m:oMath" => {
                let props = self.run_props_none(ctx);
                let math = super::math::read_omath(k, Default::default());
                let linear = wordcraft_doc::math::to_linear(&math.nodes);
                self.emit_obj(sc, pb, InlineObject::Equation { linear, display: false, math }, &props);
            }
            "m:oMathPara" => {
                let props = self.run_props_none(ctx);
                let jc = super::math::para_jc(k);
                for m in k.children("m:oMath") {
                    let math = super::math::read_omath(m, jc);
                    let linear = wordcraft_doc::math::to_linear(&math.nodes);
                    self.emit_obj(sc, pb, InlineObject::Equation { linear, display: true, math }, &props);
                }
            }
            "mc:AlternateContent" => {
                if let Some(c) = children_of_choice(k) {
                    self.read_inline_children(sc, pb, c, rels, ctx, depth + 1);
                }
            }
            // Some generators write a break directly under the paragraph instead of inside a run.
            "w:br" | "w:cr" => {
                let props = self.run_props_none(ctx);
                self.read_run_child(sc, pb, k, &props, rels, &mut None, depth + 1);
            }
            _ => {}
        }
    }

    fn run_props(&mut self, rpr: &El, ctx: &RunCtx) -> CharProps {
        let mut p = self.pc.rpr(rpr);
        p.link = ctx.link.clone();
        p.ins = ctx.ins;
        p.del = ctx.del;
        p.fmt_change = self.rpr_change(rpr);
        p
    }
    fn run_props_none(&self, ctx: &RunCtx) -> CharProps {
        CharProps { link: ctx.link.clone(), ins: ctx.ins, del: ctx.del, ..Default::default() }
    }

    fn read_run(&mut self, sc: &mut StoryCtx, pb: &mut PB, r: &El, rels: &Rels, ctx: &RunCtx, depth: usize) {
        let props = match r.child("w:rPr") {
            Some(p) => self.run_props(p, ctx),
            None => self.run_props_none(ctx),
        };
        let mut note: Option<(NoteKind, u32, Option<String>)> = None;
        for k in r.els() {
            self.read_run_child(sc, pb, k, &props, rels, &mut note, depth + 1);
        }
        if let Some((kind, id, custom)) = note.take() {
            self.emit_obj(sc, pb, InlineObject::NoteRef { kind, id, custom: custom.unwrap_or_default() }, &props);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn read_run_child(
        &mut self,
        sc: &mut StoryCtx,
        pb: &mut PB,
        k: &El,
        props: &CharProps,
        rels: &Rels,
        note: &mut Option<(NoteKind, u32, Option<String>)>,
        depth: usize,
    ) {
        if depth > crate::xml::MAX_DEPTH {
            return;
        }
        // A custom footnote mark: the text right after the reference is the mark.
        if let Some((_, _, Some(custom))) = note.as_mut()
            && k.name == "w:t"
        {
            custom.push_str(&clean_text(&k.text()));
            return;
        }
        if k.name != "w:rPr"
            && let Some((kind, id, custom)) = note.take()
        {
            self.emit_obj(sc, pb, InlineObject::NoteRef { kind, id, custom: custom.unwrap_or_default() }, props);
        }
        match k.name.as_str() {
            "w:t" | "w:delText" => {
                let t = clean_text(&k.text());
                self.emit_text(sc, pb, &t, props);
            }
            "w:instrText" | "w:delInstrText" => {
                if let Some(f) = sc.fields.last_mut()
                    && f.phase == Phase::Instr
                    && !f.spilled
                    && f.instr.len() < MAX_INSTR
                {
                    f.instr.push_str(&k.text());
                }
            }
            "w:tab" | "w:ptab" => self.emit_text(sc, pb, "\t", props),
            "w:br" => {
                let c = match k.attr("w:type") {
                    Some("page") => "\u{000C}",
                    Some("column") => "\u{000E}",
                    _ => "\n",
                };
                self.emit_text(sc, pb, c, props);
            }
            "w:cr" => self.emit_text(sc, pb, "\n", props),
            "w:noBreakHyphen" => self.emit_text(sc, pb, "\u{2011}", props),
            "w:softHyphen" => self.emit_text(sc, pb, "\u{00AD}", props),
            "w:sym" => {
                let ch = k.attr("w:char").and_then(|h| u32::from_str_radix(h.trim(), 16).ok()).and_then(char::from_u32).filter(|c| *c as u32 >= 0x20);
                if let Some(ch) = ch {
                    let mut p = props.clone();
                    if let Some(f) = k.attr("w:font").filter(|f| !f.is_empty()) {
                        p.font = Some(f.to_string());
                    }
                    self.emit_text(sc, pb, &ch.to_string(), &p);
                }
            }
            "w:fldChar" => match k.attr("w:fldCharType") {
                Some("begin") => self.begin_field(sc, props.clone(), String::new(), on_off_attr(k, "w:fldLock")),
                Some("separate") => self.separate_field(sc, pb),
                Some("end") => self.end_field(sc, pb),
                _ => {}
            },
            "w:drawing" => {
                if let Some(o) = self.read_drawing(sc, k, rels) {
                    self.emit_obj(sc, pb, o, props);
                }
            }
            "w:pict" => {
                if let Some(o) = self.read_vml(sc, k, rels) {
                    self.emit_obj(sc, pb, o, props);
                }
            }
            "w:object" => {
                if let Some(o) = self.read_object(sc, k, rels) {
                    self.emit_obj(sc, pb, o, props);
                }
            }
            "w:footnoteReference" | "w:endnoteReference" => {
                let foot = k.name == "w:footnoteReference";
                let fid = k.attr("w:id").and_then(int).unwrap_or(i64::MIN);
                let map = if foot { &self.footnotes } else { &self.endnotes };
                if let Some(id) = map.get(&fid).copied() {
                    let kind = if foot { NoteKind::Footnote } else { NoteKind::Endnote };
                    let custom = on_off_attr(k, "w:customMarkFollows").then(String::new);
                    *note = Some((kind, id, custom));
                }
            }
            // The note's own number at the start of its text: a reference to the note being read.
            "w:footnoteRef" | "w:endnoteRef" => {
                let kind = if k.name == "w:footnoteRef" { NoteKind::Footnote } else { NoteKind::Endnote };
                if let Some((nk, id)) = self.current_note
                    && nk == kind
                {
                    *note = Some((kind, id, None));
                }
            }
            "w:commentReference" => {
                if let Some(cid) = k.attr("w:id").and_then(|i| self.comment_id(i))
                    && !self.comments_ended.contains(&cid)
                {
                    self.comments_ended.insert(cid);
                    pb.push_obj(InlineObject::CommentEnd { id: cid }, props);
                }
            }
            "w:pgNum" => self.emit_obj(sc, pb, InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false }, props),
            "w:ruby" => {
                if let Some(base) = k.child("w:rubyBase") {
                    for r in base.children("w:r") {
                        for c in r.els() {
                            self.read_run_child(sc, pb, c, props, rels, note, depth + 1);
                        }
                    }
                }
            }
            "mc:AlternateContent" => {
                if let Some(c) = children_of_choice(k) {
                    for x in c.els() {
                        self.read_run_child(sc, pb, x, props, rels, note, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    // ---- fields ----

    fn begin_field(&mut self, sc: &mut StoryCtx, props: CharProps, instr: String, locked: bool) {
        if sc.fields.len() >= MAX_FIELD_DEPTH || sc.ignored_begins > 0 {
            sc.ignored_begins += 1;
            return;
        }
        // Fields nested deeper than the cap are ignored (their content flows to the enclosing field).
        sc.fields.push(FieldState { instr, phase: Phase::Instr, spilled: false, buf: Vec::new(), props, locked, range: false });
    }

    /// Whether the field below the innermost one is still collecting its code (a field nested
    /// in another's code contributes text to that code, never content).
    fn parent_in_instr(sc: &StoryCtx) -> bool {
        let n = sc.fields.len();
        n >= 2 && sc.fields.get(n - 2).is_some_and(|p| !p.spilled && !p.range && p.phase == Phase::Instr)
    }

    /// `separate`: the code is complete; a range field starts its result here.
    fn separate_field(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        if sc.ignored_begins > 0 {
            return;
        }
        let nested_in_instr = Self::parent_in_instr(sc);
        let Some(f) = sc.fields.last_mut() else { return };
        if f.range {
            return;
        }
        f.phase = Phase::Result;
        if !f.spilled && !nested_in_instr && is_range_instr(&f.instr) {
            f.range = true;
            let (obj, props) = (InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked }, f.props.clone());
            self.emit_obj(sc, pb, obj, &props);
        }
    }

    fn end_field(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        if sc.ignored_begins > 0 {
            sc.ignored_begins -= 1;
            return;
        }
        let nested_in_instr = Self::parent_in_instr(sc);
        let Some(f) = sc.fields.pop() else { return };
        if f.range {
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        if f.spilled {
            return;
        }
        if f.phase == Phase::Instr && !nested_in_instr && is_range_instr(&f.instr) {
            // A range field with no result yet.
            self.emit_obj(sc, pb, InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked }, &f.props);
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        // A cached result holding a picture, chart or shape (`INCLUDEPICTURE`'s embedded
        // picture) can't be a text result: keep the field as a range around it so the picture
        // stays in the document and is saved again. The linked source is never read.
        if !nested_in_instr && f.buf.iter().any(|(it, _)| matches!(it, Item::Obj(o) if o.frame().is_some())) {
            self.emit_obj(sc, pb, InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked }, &f.props);
            for (it, p) in f.buf {
                match it {
                    Item::Text(t) => self.emit_text(sc, pb, &t, &p),
                    Item::Obj(o) => self.emit_obj(sc, pb, o, &p),
                }
            }
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        let mut result = String::new();
        for (it, _) in &f.buf {
            match it {
                Item::Text(t) => result.push_str(t),
                Item::Obj(o) => result.push_str(o.plain_text()),
            }
        }
        let obj = InlineObject::Field { instr: f.instr.trim().to_string(), result, locked: f.locked };
        if let Some(parent) = sc.fields.last_mut()
            && !parent.spilled
            && parent.phase == Phase::Instr
        {
            parent.instr.push_str(obj.plain_text());
            return;
        }
        self.emit_obj(sc, pb, obj, &f.props);
    }

    /// At a paragraph end, open fields can't keep buffering: emit them (empty result) and
    /// let the rest of their result flow as ordinary text.
    fn spill_fields(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        for f in sc.fields.iter_mut() {
            if f.spilled || f.range {
                continue;
            }
            f.spilled = true;
            pb.push_obj(InlineObject::Field { instr: f.instr.trim().to_string(), result: String::new(), locked: f.locked }, &f.props);
            for (it, p) in f.buf.drain(..) {
                match it {
                    Item::Text(t) => pb.push_text(&t, &p),
                    Item::Obj(o) => pb.push_obj(o, &p),
                }
            }
        }
    }

    fn emit_text(&mut self, sc: &mut StoryCtx, pb: &mut PB, s: &str, props: &CharProps) {
        match sink(sc) {
            Some(f) => {
                if f.phase == Phase::Result && f.buf.len() < 100_000 {
                    f.buf.push((Item::Text(s.to_string()), props.clone()));
                }
            }
            _ => pb.push_text(s, props),
        }
    }

    fn emit_obj(&mut self, sc: &mut StoryCtx, pb: &mut PB, o: InlineObject, props: &CharProps) {
        match sink(sc) {
            Some(f) => {
                if f.phase == Phase::Result && f.buf.len() < 100_000 {
                    f.buf.push((Item::Obj(o), props.clone()));
                }
            }
            _ => pb.push_obj(o, props),
        }
    }

    // ---- drawings ----

    fn read_drawing(&mut self, sc: &mut StoryCtx, d: &El, rels: &Rels) -> Option<InlineObject> {
        let (c, anchored) = match d.child("wp:inline") {
            Some(c) => (c, false),
            None => (d.child("wp:anchor")?, true),
        };
        let ext = c.child("wp:extent");
        let dim = |n: &str| ext.and_then(|e| e.attr(n)).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT);
        let (w, h) = (dim("cx"), dim("cy"));
        let alt = c.child("wp:docPr").and_then(|p| p.attr("descr").filter(|s| !s.is_empty()).or_else(|| p.attr("title"))).unwrap_or("").to_string();
        let mut float = if anchored { anchor_float(c) } else { Float::default() };
        if let Some(e) = c.child("wp:effectExtent") {
            for (slot, n) in float.effect.iter_mut().zip(["l", "t", "r", "b"]) {
                // Negative for a rotated object narrower than its frame; settled below.
                *slot = e.attr(n).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(-1584.0, 1584.0);
            }
        }
        let gd = c.child("a:graphic").and_then(|g| g.child("a:graphicData"))?;
        // Checked by URI before the blip search: a diagram's data can hold a blip further down.
        let uri = gd.attr("uri").unwrap_or("");
        let graphic_kind = if uri.ends_with("/chart") {
            Some(GraphicKind::Chart)
        } else if uri.ends_with("/diagram") {
            Some(GraphicKind::Diagram)
        } else {
            None
        };
        if let Some(kind) = graphic_kind {
            // Kept so saving writes the chart or diagram back: the frame around it is written
            // from the object's (possibly moved or resized) size and position.
            let extra: Vec<String> = match kind {
                GraphicKind::Chart => Vec::new(),
                GraphicKind::Diagram => self.diagram_drawing_rel(gd, rels).into_iter().collect(),
            };
            let source = c.child("a:graphic").and_then(|g| self.keep_embedded(g, rels, &extra));
            let graphic = self.graphic(kind, gd, rels, w, h, source);
            float.effect = float.effect.map(|e| e.max(0.0));
            return Some(InlineObject::Graphic { w, h, alt, float, graphic });
        }
        let mut obj = if let Some(g) = gd.child("wpg:wgp") {
            self.read_group(sc, g, rels, w, h, float)?
        } else if gd.find("a:blip").is_some() {
            self.read_pic(gd, rels, w, h, alt, float)?
        } else {
            let wsp = gd.find("wps:wsp")?;
            let mut obj = self.read_wsp(sc, wsp, rels, w, h, float);
            // A freeform: its custom geometry, and whether it's ink WordCraft wrote.
            if let InlineObject::Shape { kind, freeform, .. } = &mut obj
                && let Some(sppr) = wsp.child("wps:spPr")
                && let Some(mut f) = super::freeform::cust_geom(sppr, w, h)
            {
                f.alpha = super::freeform::line_alpha(sppr.child("a:ln"));
                f.ink = c.child("wp:docPr").and_then(|p| p.attr("name")).and_then(super::freeform::ink_tool);
                *kind = ShapeKind::Freeform;
                *freeform = Some(Arc::new(f));
            }
            obj
        };
        // Word's effect extent also covers a rotated object's overhang; the model keeps that
        // apart (it follows from the angle), so take it back out.
        if let Some(f) = obj.float_mut() {
            let (px, py) = f.spin_pad(w, h);
            for (e, p) in f.effect.iter_mut().zip([px, py, px, py]) {
                *e = (*e - p).clamp(0.0, 1584.0);
            }
        }
        Some(obj)
    }

    /// The chart or SmartArt diagram in an `a:graphicData`, as items in points from its top-left
    /// corner. Charts are drawn from their cached data, SmartArt diagrams from the drawing Word
    /// stores beside them. Built once per part and size; once the file's graphic budget is spent,
    /// later ones are empty.
    fn graphic(&mut self, kind: GraphicKind, gd: &El, rels: &Rels, w: f32, h: f32, source: Option<Arc<Embedded>>) -> Arc<Graphic> {
        let part = match kind {
            GraphicKind::Chart => gd.child("c:chart").and_then(|c| c.attr("r:id")).and_then(|id| super::part_of(rels, id, rt::CHART)),
            GraphicKind::Diagram => self.diagram_part(gd, rels),
        };
        let Some(path) = part else { return Arc::new(Graphic { kind, items: Vec::new(), w, h, source }) };
        let key = (kind, path, w.to_bits(), h.to_bits());
        if let Some(g) = self.graphics.get(&key) {
            return g.clone();
        }
        let items = if self.graphic_budget == 0 { Vec::new() } else { self.graphic_items(kind, &key.1, w, h) };
        self.graphic_budget = self.graphic_budget.saturating_sub(graphic_work(&items));
        let g = Arc::new(Graphic { kind, items, w, h, source });
        self.graphics.insert(key, g.clone());
        g
    }

    /// The items of the chart part or diagram drawing part at `path`.
    fn graphic_items(&mut self, kind: GraphicKind, path: &str, w: f32, h: f32) -> Vec<GraphicItem> {
        match kind {
            GraphicKind::Chart => match self.graphic_part(path) {
                Some(space) => super::chart::chart_items(&space, &self.doc.settings.theme_colors, w, h),
                None => Vec::new(),
            },
            GraphicKind::Diagram => self.diagram_drawing(path, w, h),
        }
    }

    /// A picture: the first `a:blip` in `pic` and its crop.
    fn read_pic(&mut self, pic: &El, rels: &Rels, w: f32, h: f32, alt: String, mut float: Float) -> Option<InlineObject> {
        let blip = pic.find("a:blip")?;
        float.set_spin(xfrm_spin(pic.find("pic:spPr")));
        let media = blip.attr("r:embed").and_then(|id| self.media_for(rels, id))?;
        let mut crop = [0.0f32; 4];
        if let Some(sr) = pic.find("a:srcRect") {
            for (i, n) in ["l", "t", "r", "b"].iter().enumerate() {
                if let Some(v) = sr.attr(n).and_then(int)
                    && let Some(slot) = crop.get_mut(i)
                {
                    *slot = (v.clamp(0, 100_000) as f32 / 100_000.0).clamp(0.0, 1.0);
                }
            }
        }
        Some(InlineObject::Image { media, w, h, alt, float, crop, ole: None })
    }

    /// A `wpg:wgp` group (`w` × `h`): its pictures, shapes and text boxes, nested groups
    /// flattened into it. `None` when it has no member we can show.
    fn read_group(&mut self, sc: &mut StoryCtx, g: &El, rels: &Rels, w: f32, h: f32, mut float: Float) -> Option<InlineObject> {
        let x = group_xfrm(g.child("wpg:grpSpPr"));
        float.set_spin(xfrm_spin(g.child("wpg:grpSpPr")));
        // The members' space; without `a:chExt` it is the group's own size.
        let (ch_w, ch_h) = match x.ch_ext {
            Some((cw, ch)) if cw > 0.0 && ch > 0.0 => (cw, ch),
            _ => (w.max(1.0), h.max(1.0)),
        };
        let (cx, cy) = x.ch_off.unwrap_or((0.0, 0.0));
        let mut children = Vec::new();
        self.group_members(sc, g, rels, (-cx, -cy, 1.0, 1.0), 0, &mut children);
        if children.is_empty() {
            return None;
        }
        Some(InlineObject::Group { w, h, float, ch_w, ch_h, children })
    }

    /// The members of group `g` into `out`, mapped into the outermost group's space by `t`
    /// (x0, y0, sx, sy: outer = x0 + inner × sx).
    fn group_members(
        &mut self,
        sc: &mut StoryCtx,
        g: &El,
        rels: &Rels,
        t: (f32, f32, f32, f32),
        depth: usize,
        out: &mut Vec<wordcraft_doc::para::GroupChild>,
    ) {
        let (x0, y0, sx, sy) = t;
        for e in g.els() {
            if out.len() >= wordcraft_doc::para::MAX_GROUP_CHILDREN {
                return;
            }
            let e = match e.name.as_str() {
                "mc:AlternateContent" => match children_of_choice(e).and_then(|c| c.els().next()) {
                    Some(c) => c,
                    None => continue,
                },
                _ => e,
            };
            let sppr = match e.name.as_str() {
                "wps:wsp" => e.child("wps:spPr"),
                "pic:pic" => e.child("pic:spPr"),
                "wpg:grpSp" => e.child("wpg:grpSpPr"),
                _ => continue,
            };
            let x = group_xfrm(sppr);
            let ((ox, oy), (ew, eh)) = (x.off, x.ext);
            if e.name == "wpg:grpSp" {
                if depth >= 8 {
                    continue;
                }
                // Its members' space maps onto its own box.
                let (cox, coy) = x.ch_off.unwrap_or((0.0, 0.0));
                let (cw, ch) = x.ch_ext.filter(|(cw, ch)| *cw > 0.0 && *ch > 0.0).unwrap_or((ew.max(1.0), eh.max(1.0)));
                let (kx, ky) = (ew / cw, eh / ch);
                let nt = (x0 + (ox - cox * kx) * sx, y0 + (oy - coy * ky) * sy, sx * kx, sy * ky);
                self.group_members(sc, e, rels, nt, depth + 1, out);
                continue;
            }
            let max = crate::units::MAX_LEN_PT;
            let fit = |v: f32, lo: f32| wordcraft_geom::finite(v).clamp(lo, max);
            let (w, h) = (fit(ew * sx, 0.0), fit(eh * sy, 0.0));
            let obj = if e.name == "pic:pic" {
                let alt = e.find("pic:cNvPr").and_then(|p| p.attr("descr")).unwrap_or("").to_string();
                self.read_pic(e, rels, w, h, alt, Float::default())
            } else {
                Some(self.read_wsp(sc, e, rels, w, h, Float::default()))
            };
            if let Some(obj) = obj {
                out.push(wordcraft_doc::para::GroupChild { x: fit(x0 + ox * sx, -max), y: fit(y0 + oy * sy, -max), obj });
            }
        }
    }

    fn read_wsp(&mut self, sc: &mut StoryCtx, wsp: &El, rels: &Rels, w: f32, h: f32, mut float: Float) -> InlineObject {
        let sppr = wsp.child("wps:spPr");
        float.set_spin(xfrm_spin(sppr));
        let prst = sppr.and_then(|s| s.child("a:prstGeom")).and_then(|g| g.attr("prst")).unwrap_or("rect");
        let txbx = wsp.child("wps:txbx").and_then(|t| t.child("w:txbxContent"));
        let mut kind = match prst {
            "roundRect" => ShapeKind::RoundedRectangle,
            "ellipse" => ShapeKind::Ellipse,
            "triangle" | "rtTriangle" => ShapeKind::Triangle,
            "diamond" => ShapeKind::Diamond,
            "line" | "straightConnector1" => ShapeKind::Line,
            "rightArrow" | "leftArrow" | "upArrow" | "downArrow" => ShapeKind::Arrow,
            "star5" | "star4" | "star6" => ShapeKind::Star,
            "heart" => ShapeKind::Heart,
            _ => ShapeKind::Rectangle,
        };
        let is_tb = wsp.child("wps:cNvSpPr").and_then(|c| c.attr("txBox")).is_some_and(|v| v == "1" || v == "true");
        if is_tb || (txbx.is_some() && kind == ShapeKind::Rectangle) {
            kind = ShapeKind::TextBox;
        }
        let solid = |e: Option<&El>| {
            e.and_then(|f| f.child("a:solidFill")).and_then(|f| f.child("a:srgbClr")).and_then(|c| c.attr("val")).and_then(Rgb::parse)
        };
        let fill = solid(sppr);
        let ln = sppr.and_then(|s| s.child("a:ln"));
        let stroke = solid(ln);
        let stroke_width = ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(0.0, 100.0);
        let effects = sppr.and_then(|s| s.child("a:effectLst")).map(effect_list).unwrap_or_default();
        let story = match txbx {
            Some(t) if sc.story_depth < MAX_STORY_DEPTH => Some(self.read_textbox(sc, t, rels)),
            _ => None,
        };
        InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, story, freeform: None, effects }
    }

    fn read_textbox(&mut self, sc: &StoryCtx, content: &El, rels: &Rels) -> u32 {
        let mut inner = StoryCtx { story_depth: sc.story_depth + 1, ..Default::default() };
        let mut blocks = Blocks::new();
        self.read_blocks(&mut inner, content, rels, &mut blocks, 0);
        self.flush_pending(&mut inner, &mut blocks);
        self.doc.add_part(PartKind::TextBox, blocks)
    }

    /// Legacy VML pictures and text boxes (best effort).
    fn read_vml(&mut self, sc: &mut StoryCtx, pict: &El, rels: &Rels) -> Option<InlineObject> {
        let shape = pict.els().find(|e| e.name.starts_with("v:") && e.local() != "shapetype")?;
        let style = shape.attr("style").unwrap_or("");
        let len = |k: &str| vml_style(style, k).and_then(|v| measure(v, 1.0)).unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT);
        let (w, h) = (len("width"), len("height"));
        let float = vml_float(shape, style);
        if let Some(img) = shape.find("v:imagedata") {
            let id = img.attr("r:id").or_else(|| img.attr("r:pict"))?;
            let media = self.media_for(rels, id)?;
            let alt = shape.attr("alt").or_else(|| img.attr("o:title")).unwrap_or("").to_string();
            return Some(InlineObject::Image { media, w, h, alt, float, crop: [0.0; 4], ole: None });
        }
        if let Some(t) = shape.find("w:txbxContent")
            && sc.story_depth < MAX_STORY_DEPTH
        {
            let story = Some(self.read_textbox(sc, t, rels));
            let fill = shape.attr("fillcolor").and_then(Rgb::parse);
            let stroke = shape.attr("strokecolor").and_then(Rgb::parse);
            return Some(InlineObject::Shape {
                kind: ShapeKind::TextBox,
                w,
                h,
                fill,
                stroke,
                stroke_width: 0.75,
                float,
                story,
                freeform: None,
                effects: Default::default(),
            });
        }
        None
    }

    /// An OLE object (`w:object`, ECMA-376 §17.3.3.19): shown as its picture (VML, or a DrawingML
    /// picture in `w:drawing`), and kept whole so saving writes the object back.
    fn read_object(&mut self, sc: &mut StoryCtx, obj: &El, rels: &Rels) -> Option<InlineObject> {
        let mut o = match self.read_vml(sc, obj, rels) {
            Some(o) => o,
            None => {
                let d = obj.child("w:drawing")?;
                self.read_drawing(sc, d, rels)?
            }
        };
        if let InlineObject::Image { ole, .. } = &mut o {
            *ole = self.keep_embedded(obj, rels, &[]);
        }
        Some(o)
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
        collect(t, "w:tr", &mut rows, 0);
        for tr in rows.into_iter().take(MAX_ROWS) {
            let mut row = Row { props: tr.child("w:trPr").map(trpr).unwrap_or_default(), cells: Vec::new() };
            if let Some(ch) = tr.child("w:trPr").and_then(|p| p.child("w:trPrChange")) {
                let old = ch.child("w:trPr").map(trpr).unwrap_or_default();
                row.props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
            }
            let mut tcs = Vec::new();
            collect(tr, "w:tc", &mut tcs, 0);
            for tc in tcs.into_iter().take(MAX_COLS) {
                let (mut props, hcont) =
                    tc.child("w:tcPr").map(tcpr).unwrap_or_else(|| (wordcraft_doc::props::CellProps { span: 1, ..Default::default() }, false));
                if let Some(ch) = tc.child("w:tcPr").and_then(|p| p.child("w:tcPrChange")) {
                    let old = ch.child("w:tcPr").map(|o| tcpr(o).0).unwrap_or_default();
                    props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
                }
                if hcont && let Some(prev) = row.cells.last_mut() {
                    prev.props.span = (prev.props.span + props.span.max(1)).min(63);
                    continue;
                }
                let mut blocks = Blocks::new();
                self.read_blocks(sc, tc, rels, &mut blocks, depth + 4);
                self.flush_pending(sc, &mut blocks);
                row.cells.push(Cell { props, blocks });
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

/// Gather `name` children, looking through `w:sdt`/`w:customXml` wrappers.
fn collect<'a>(e: &'a El, name: &str, out: &mut Vec<&'a El>, depth: usize) {
    if depth > 8 {
        return;
    }
    for k in e.els() {
        if k.name == name {
            out.push(k);
        } else if k.name == "w:sdt" {
            if let Some(c) = k.child("w:sdtContent") {
                collect(c, name, out, depth + 1);
            }
        } else if k.name == "w:customXml" {
            collect(k, name, out, depth + 1);
        }
    }
}

fn on_off_attr(e: &El, name: &str) -> bool {
    e.attr(name).is_some_and(|v| !matches!(v, "0" | "false" | "off"))
}

fn anchor_float(c: &El) -> Float {
    let mut f = Float { wrap: Wrap::Square, ..Float::default() };
    let behind = on_off_attr(c, "behindDoc");
    for k in c.els() {
        match k.name.as_str() {
            "wp:wrapSquare" => f.wrap = Wrap::Square,
            "wp:wrapTight" => f.wrap = Wrap::Tight,
            "wp:wrapThrough" => f.wrap = Wrap::Through,
            "wp:wrapTopAndBottom" => f.wrap = Wrap::TopAndBottom,
            "wp:wrapNone" => f.wrap = if behind { Wrap::BehindText } else { Wrap::InFrontOfText },
            "wp:positionH" | "wp:positionV" => {
                let horiz = k.name == "wp:positionH";
                let rel = match (k.attr("relativeFrom").unwrap_or(""), horiz) {
                    ("page", _) => Anchor::Page,
                    ("margin", _) => Anchor::Margin,
                    ("leftMargin", true) => Anchor::LeftMargin,
                    ("rightMargin", true) => Anchor::RightMargin,
                    ("insideMargin", _) => Anchor::InsideMargin,
                    ("outsideMargin", _) => Anchor::OutsideMargin,
                    ("character", true) => Anchor::Character,
                    ("topMargin", false) => Anchor::TopMargin,
                    ("bottomMargin", false) => Anchor::BottomMargin,
                    ("line", false) => Anchor::Line,
                    ("paragraph", false) => Anchor::Paragraph,
                    (_, true) => Anchor::Column,
                    (_, false) => Anchor::Paragraph,
                };
                let align = k.child("wp:align").and_then(|a| FloatAlign::from_ooxml(a.text().trim()));
                let off = k.child("wp:posOffset").and_then(|o| measure(&o.text(), 12_700.0)).unwrap_or(0.0);
                if horiz {
                    f.h_rel = rel;
                    f.h_align = align;
                    f.x = off;
                } else {
                    f.v_rel = rel;
                    f.v_align = align;
                    f.y = off;
                }
            }
            _ => {}
        }
    }
    let dist = |n: &str| c.attr(n).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(0.0, 1584.0);
    f.dist = dist("distL").max(dist("distR"));
    f.dist_top = dist("distT");
    f.dist_bottom = dist("distB");
    f
}

/// The value of property `key` in a VML `style` attribute (CSS declarations; the last one wins).
fn vml_style<'a>(style: &'a str, key: &str) -> Option<&'a str> {
    style.rsplit(';').filter_map(|d| d.split_once(':')).find(|(k, _)| k.trim().eq_ignore_ascii_case(key)).map(|(_, v)| v.trim())
}

/// Placement of a VML shape (ECMA-376 Part 4, VML): `position:absolute` in its style makes it
/// float, offset by `left`/`margin-left` and `top`/`margin-top` from the area named by
/// `mso-position-horizontal-relative` / `mso-position-vertical-relative` (unless aligned with
/// `mso-position-horizontal` / `mso-position-vertical`), wrapped as its `w10:wrap` says. Without
/// wrapping (`none`, or no `w10:wrap`) it's in front of the text, or behind it at a negative
/// `z-index`. Anything else stays inline.
fn vml_float(shape: &El, style: &str) -> Float {
    let get = |k: &str| vml_style(style, k);
    if !get("position").is_some_and(|p| p.eq_ignore_ascii_case("absolute")) {
        // Inline: only the turn (an OLE object turned in WordCraft is saved so, #319).
        let mut f = Float::default();
        f.set_spin(vml_spin(style));
        return f;
    }
    let off = |k: &str| get(k).and_then(|v| measure(v, 1.0)).unwrap_or(0.0);
    let max = crate::units::MAX_LEN_PT;
    let behind = get("z-index").and_then(int).is_some_and(|z| z < 0);
    let wrap = match shape.child("w10:wrap").and_then(|w| w.attr("type")).unwrap_or("none") {
        "square" => Wrap::Square,
        "tight" => Wrap::Tight,
        "through" => Wrap::Through,
        "topAndBottom" => Wrap::TopAndBottom,
        _ if behind => Wrap::BehindText,
        _ => Wrap::InFrontOfText,
    };
    let h_rel = match get("mso-position-horizontal-relative").unwrap_or("text") {
        "page" => Anchor::Page,
        "margin" => Anchor::Margin,
        "char" => Anchor::Character,
        "left-margin-area" => Anchor::LeftMargin,
        "right-margin-area" => Anchor::RightMargin,
        "inner-margin-area" => Anchor::InsideMargin,
        "outer-margin-area" => Anchor::OutsideMargin,
        _ => Anchor::Column,
    };
    let v_rel = match get("mso-position-vertical-relative").unwrap_or("text") {
        "page" => Anchor::Page,
        "margin" => Anchor::Margin,
        "line" => Anchor::Line,
        "top-margin-area" => Anchor::TopMargin,
        "bottom-margin-area" => Anchor::BottomMargin,
        "inner-margin-area" => Anchor::InsideMargin,
        "outer-margin-area" => Anchor::OutsideMargin,
        _ => Anchor::Paragraph,
    };
    let dist = |k: &str| get(k).and_then(|v| measure(v, 1.0)).unwrap_or(0.0).clamp(0.0, 1584.0);
    let mut f = Float {
        wrap,
        h_rel,
        v_rel,
        x: (off("left") + off("margin-left")).clamp(-max, max),
        y: (off("top") + off("margin-top")).clamp(-max, max),
        h_align: get("mso-position-horizontal").and_then(FloatAlign::from_ooxml),
        v_align: get("mso-position-vertical").and_then(FloatAlign::from_ooxml),
        dist: dist("mso-wrap-distance-left").max(dist("mso-wrap-distance-right")),
        dist_top: dist("mso-wrap-distance-top"),
        dist_bottom: dist("mso-wrap-distance-bottom"),
        ..Float::default()
    };
    f.set_spin(vml_spin(style));
    f
}

/// A VML shape's turn: `rotation` in degrees, or 65536ths of one with an `fd` suffix; `flip`: `x`
/// and/or `y`.
fn vml_spin(style: &str) -> wordcraft_geom::Spin {
    let get = |k: &str| vml_style(style, k);
    let rot = get("rotation").and_then(|v| match v.strip_suffix("fd") {
        Some(fd) => fd.trim().parse::<f32>().ok().map(|f| f / 65_536.0),
        None => v.trim().parse::<f32>().ok(),
    });
    let flip = get("flip").unwrap_or("");
    wordcraft_geom::Spin::new(rot.unwrap_or(0.0), flip.contains('x'), flip.contains('y'))
}

/// What a graphic's items cost to keep and draw: one per item, plus one per path segment.
fn graphic_work(items: &[GraphicItem]) -> usize {
    items.iter().map(|it| if let GraphicItem::Path { segs, .. } = it { segs.len() + 1 } else { 1 }).sum()
}

/// A shape's `a:effectLst` (ECMA-376 Part 1 §20.1.8.26): its outer shadow, glow and soft edges.
/// Other effects are dropped.
fn effect_list(l: &El) -> ShapeEffects {
    let pt = |e: &El, n: &str| e.attr(n).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0);
    let shadow = l.child("a:outerShdw").map(|e| {
        let (color, transparency) = effect_color(e);
        // `dir`: 60000ths of a degree, clockwise.
        let angle = e.attr("dir").and_then(int).map(|v| (v.rem_euclid(21_600_000) as f32) / 60_000.0).unwrap_or(0.0);
        // `rotWithShape` is true unless it says otherwise (ECMA-376 §20.1.8.49).
        let rot_with_shape = e.attr("rotWithShape").is_none_or(|v| !matches!(v, "0" | "false" | "off"));
        Shadow { color: color.unwrap_or(Rgb::BLACK), transparency, blur: pt(e, "blurRad"), distance: pt(e, "dist"), angle, rot_with_shape }
    });
    let glow = l.child("a:glow").map(|e| {
        let (color, transparency) = effect_color(e);
        Glow { color: color.unwrap_or(Glow::default().color), size: pt(e, "rad"), transparency }
    });
    let soft_edge = l.child("a:softEdge").map(|e| pt(e, "rad"));
    ShapeEffects { shadow, glow, soft_edge }.sanitized()
}

/// The colour inside an effect (`a:srgbClr`, `a:prstClr`, `a:sysClr`; theme colours aren't
/// resolved here) and its transparency, percent (from `a:alpha`, opaque when absent).
fn effect_color(e: &El) -> (Option<Rgb>, f32) {
    let Some(c) = e.els().find(|c| matches!(c.name.as_str(), "a:srgbClr" | "a:prstClr" | "a:sysClr" | "a:schemeClr" | "a:scrgbClr" | "a:hslClr"))
    else {
        return (None, 0.0);
    };
    let rgb = match c.name.as_str() {
        "a:srgbClr" => c.attr("val").and_then(Rgb::parse),
        "a:sysClr" => c.attr("lastClr").and_then(Rgb::parse),
        "a:prstClr" => match c.attr("val").unwrap_or("") {
            "black" => Some(Rgb::BLACK),
            "white" => Some(Rgb::WHITE),
            "gray" | "grey" => Some(Rgb(0x80, 0x80, 0x80)),
            _ => None,
        },
        _ => None,
    };
    // `a:alpha`: opacity in 1000ths of a percent.
    let opacity = c.child("a:alpha").and_then(|a| a.attr("val")).and_then(int).map(|v| v.clamp(0, 100_000) as f32 / 1000.0).unwrap_or(100.0);
    (rgb, 100.0 - opacity)
}

/// A DrawingML `a:xfrm` (ECMA-376 §20.1.7.5/6) in points: offset, extent and, for a group, its
/// members' offset and extent.
struct Xfrm {
    off: (f32, f32),
    ext: (f32, f32),
    ch_off: Option<(f32, f32)>,
    ch_ext: Option<(f32, f32)>,
}

/// The rotation and flips of the `a:xfrm` in shape properties `sppr` (ECMA-376 §20.1.7.6:
/// `rot` in 60000ths of a degree, clockwise; `flipH`, `flipV`). None when missing or junk.
fn xfrm_spin(sppr: Option<&El>) -> wordcraft_geom::Spin {
    let Some(x) = sppr.and_then(|s| s.child("a:xfrm")) else { return wordcraft_geom::Spin::default() };
    let rot = x.attr("rot").and_then(int).map(|v| v.rem_euclid(21_600_000) as f32 / 60_000.0).unwrap_or(0.0);
    wordcraft_geom::Spin::new(rot, on_off_attr(x, "flipH"), on_off_attr(x, "flipV"))
}

/// The `a:xfrm` in shape properties `sppr` (zeros when missing).
fn group_xfrm(sppr: Option<&El>) -> Xfrm {
    let x = sppr.and_then(|s| s.child("a:xfrm"));
    let pair = |name: &str, a: &str, b: &str, lim: f32| {
        let e = x.and_then(|x| x.child(name))?;
        let v = |n: &str| e.attr(n).and_then(|v| measure(v, 12_700.0)).map(|v| v.clamp(-lim, lim)).unwrap_or(0.0);
        Some((v(a), v(b)))
    };
    let max = crate::units::MAX_LEN_PT;
    let pos = |p: Option<(f32, f32)>| p.map(|(a, b)| (a.max(0.0), b.max(0.0)));
    Xfrm {
        off: pair("a:off", "x", "y", max).unwrap_or((0.0, 0.0)),
        ext: pos(pair("a:ext", "cx", "cy", max)).unwrap_or((0.0, 0.0)),
        ch_off: pair("a:chOff", "x", "y", max),
        ch_ext: pos(pair("a:chExt", "cx", "cy", max)),
    }
}
