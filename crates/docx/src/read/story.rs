//! Stories: blocks, paragraphs, runs, fields, drawings, tables.

use std::sync::Arc;

use wordcraft_doc::connector::{ConnEnd, ShapeExtra};
use wordcraft_doc::control::{ContentControl, ControlKind, ControlWrap};
use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
use wordcraft_doc::graphic::{Embedded, Graphic, GraphicItem, GraphicKind};
use wordcraft_doc::para::{AltText, Anchor, Float, FloatAlign, NoteKind, ShapeKind, Wrap, WrapText, cap_alt};
use wordcraft_doc::props::{CharProps, NumChange, PropChange, Rgb, TextColor};
use wordcraft_doc::table::{Cell, MAX_COLS, MAX_ROWS, Row, Table};
use wordcraft_doc::{Block, Blocks, InlineObject, Paragraph, PartKind, RevisionKind, Run, para_block};

use super::Reader;
use super::props::{sectpr, tcpr, trpr};
use super::shape_style::{preset_kind, shape_paint, vml_color, vml_preset_kind};
use crate::package::{ContentTypes, Rels, rt};
use crate::units::{int, measure, u32_of};
use crate::xml::{El, Node};
use wordcraft_doc::wordart::TextWarp;

const INK_URI: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingInk";
const INK_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";
/// The `a:graphicData` URI of an Office 2016 chart (`cx:chart`).
const CHARTEX_URI: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";

/// Deepest table nesting we build (deeper tables become plain paragraphs).
const MAX_TABLE_DEPTH: usize = 24;
/// Deepest field nesting we track.
const MAX_FIELD_DEPTH: usize = 32;
/// Deepest text-box-in-text-box nesting.
const MAX_STORY_DEPTH: usize = 4;
/// Longest field code we keep (Zotero's citation codes embed item data as JSON).
const MAX_INSTR: usize = 2 * 1024 * 1024;
/// Longest legacy form field definition (`w:ffData`) we keep.
const MAX_FORM: usize = 64 * 1024;

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

/// A buffered piece of a field's result. Objects are boxed: an inline object is much larger than
/// a string.
enum Item {
    Text(String),
    Obj(Box<InlineObject>),
}

struct FieldState {
    instr: String,
    phase: Phase,
    spilled: bool,
    buf: Vec<(Item, CharProps)>,
    props: CharProps,
    locked: bool,
    /// `w:dirty` from the begin marker: the result is stale, Word refreshes it on open.
    dirty: bool,
    /// Kept as a range field: its result flows into the document as ordinary content between
    /// `FieldStart` and `FieldEnd` markers instead of being buffered.
    range: bool,
    /// The font size of each piece of the code (where it starts in `instr`): an enclosure's
    /// style is the size of its symbol and text (see [`wordcraft_doc::eq`]).
    sizes: Vec<(usize, Option<f32>)>,
    /// A legacy form field's `w:ffData`, as raw XML.
    form: String,
}

/// Enclosed characters (`eq \o\ac(○,字)`), with their style from the code's font sizes: Word
/// enlarges the symbol (bigger than the text, which keeps the field's size) or shrinks the text
/// (smaller than the symbol).
fn enclosure(f: &FieldState) -> Option<wordcraft_doc::eq::Enclosure> {
    use wordcraft_doc::eq::{EncloseShape, EncloseStyle, Enclosure};
    let mut e = Enclosure::parse(&f.instr)?;
    let size_at = |at: usize| f.sizes.iter().rev().find(|(start, _)| *start <= at).and_then(|(_, s)| *s);
    let sym = f.instr.char_indices().find(|(_, c)| EncloseShape::from_symbol(&c.to_string()) == Some(e.shape)).map(|(i, _)| i)?;
    let txt = e.text.chars().next().and_then(|c| f.instr.get(sym..)?.find(c).map(|i| i + sym)).unwrap_or(sym);
    let base = f.props.size;
    e.style = match (size_at(sym), size_at(txt)) {
        (Some(s), Some(t)) if s > t + 0.01 && base.is_none_or(|b| t >= b - 0.01) => EncloseStyle::Enlarge,
        (Some(s), None) if base.is_none_or(|b| s > b + 0.01) => EncloseStyle::Enlarge,
        _ => EncloseStyle::Shrink,
    };
    Some(e)
}

/// Fields kept as ranges: those whose result another program writes, with formatting and often
/// several paragraphs (TOCs and citation managers' `ADDIN` fields).
fn is_range_instr(instr: &str) -> bool {
    super::is_toc(instr) || instr.trim_start().get(..5).is_some_and(|k| k.eq_ignore_ascii_case("ADDIN"))
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
    /// Reading a header or footer (not a text box inside one): where Word keeps its watermark.
    pub in_hf: bool,
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
    const KNOWN: &[&str] = &["wps", "wpg", "wpc", "wpi", "w14", "w15", "wp14", "a14", "w16se", "w16cid", "w16", "w16cex", "w16sdtdh", "v"];
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
                "w:sdt" => self.read_block_sdt(sc, e, rels, out, depth),
                "w:customXml" | "w:smartTag" | "w:ins" | "w:moveTo" => self.read_blocks(sc, e, rels, out, depth + 1),
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
        let mut ctl = super::sdt::sdt_pr(e.child("w:sdtPr"), true);
        ctl.end_pr_xml = super::sdt::sdt_end_pr(e.child("w:sdtEndPr"));
        let mut inner = Blocks::new();
        self.read_blocks(sc, content, rels, &mut inner, depth + 1);
        // A table of contents' control belongs to the TOC: the writer puts it back around the
        // TOC field and its entries, which References › Update Table rebuilds.
        let toc = matches!(&ctl.kind, ControlKind::Gallery { list: false, gallery, .. } if gallery == "Table of Contents");
        if inner.is_empty() || toc {
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

    fn comment_id(&self, file_id: &str) -> Option<u32> {
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
            super::props::resolve_level_only(&self.doc.styles, &mut props, "Normal");
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
                        super::props::resolve_level_only(&self.doc.styles, &mut old, "Normal");
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
                let url = k.attr("r:id").and_then(|id| rels.by_id(id)).map(|r| r.target.trim().to_string()).filter(|u| !u.is_empty());
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
                c.ins = Some(if k.name == "w:moveTo" { self.move_revision(true, k, false) } else { self.revision(RevisionKind::Insert, k) });
                self.read_inline_children(sc, pb, k, rels, &c, depth + 1);
            }
            "w:del" | "w:moveFrom" => {
                let mut c = ctx.clone();
                c.del = Some(if k.name == "w:moveFrom" { self.move_revision(false, k, false) } else { self.revision(RevisionKind::Delete, k) });
                self.read_inline_children(sc, pb, k, rels, &c, depth + 1);
            }
            "w:moveFromRangeStart" | "w:moveFromRangeEnd" | "w:moveToRangeStart" | "w:moveToRangeEnd" => self.move_range(k),
            "w:fldSimple" => {
                let instr = k.attr("w:instr").unwrap_or("").to_string();
                let props = k.child("w:r").and_then(|r| r.child("w:rPr")).map(|p| self.run_props(p, ctx)).unwrap_or_else(|| self.run_props_none(ctx));
                self.begin_field(sc, props, instr, on_off_attr(k, "w:fldLock"), on_off_attr(k, "w:dirty"), String::new());
                self.separate_field(sc, pb);
                self.read_inline_children(sc, pb, k, rels, ctx, depth + 1);
                self.end_field(sc, pb);
            }
            "w:bookmarkStart" | "w:bookmarkEnd" | "w:commentRangeStart" | "w:commentRangeEnd" | "w:permStart" | "w:permEnd" => {
                if let Some(o) = self.marker(k) {
                    let props = self.run_props_none(ctx);
                    // Markers go straight into the paragraph (never into a field's cached result).
                    pb.push_obj(o, &props);
                }
            }
            "w:sdt" => {
                let Some(c) = k.child("w:sdtContent") else { return };
                let mut ctl = super::sdt::sdt_pr(k.child("w:sdtPr"), false);
                ctl.end_pr_xml = super::sdt::sdt_end_pr(k.child("w:sdtEndPr"));
                // The markers take the formatting of the content's first run (typing at the
                // start of the content continues it).
                let props = c.find("w:rPr").map(|p| self.run_props(p, ctx)).unwrap_or_else(|| self.run_props_none(ctx));
                let (at, k0) = (pb.text.len(), pb.objects.len());
                pb.push_obj(InlineObject::ControlStart { control: Box::new(ctl) }, &props);
                self.read_inline_children(sc, pb, c, rels, ctx, depth + 1);
                if let Some(InlineObject::ControlStart { control }) = pb.objects.get_mut(k0)
                    && control.showing_placeholder
                {
                    let shown: String = pb.text.get(at..).unwrap_or("").chars().filter(|c| *c != wordcraft_doc::para::OBJ).collect();
                    control.placeholder_text = placeholder_of(&shown);
                }
                let end_props = pb.runs.last().map(|r| r.props.clone()).unwrap_or(props);
                pb.push_obj(InlineObject::ControlEnd, &end_props);
            }
            "w:smartTag" | "w:customXml" | "w:dir" | "w:bdo" => self.read_inline_children(sc, pb, k, rels, ctx, depth + 1),
            "m:oMath" => {
                let mut props = self.run_props_none(ctx);
                let mut math = super::math::read_omath(k, Default::default());
                if let Some(c) = super::math::lift_color(&mut math) {
                    props.color = Some(TextColor::Rgb(c));
                }
                let linear = wordcraft_doc::math::to_linear(&math.nodes);
                self.emit_obj(sc, pb, InlineObject::Equation { linear, display: false, math }, &props);
            }
            "m:oMathPara" => {
                let props = self.run_props_none(ctx);
                let jc = super::math::para_jc(k);
                for m in k.children("m:oMath") {
                    let mut math = super::math::read_omath(m, jc);
                    let mut props = props.clone();
                    if let Some(c) = super::math::lift_color(&mut math) {
                        props.color = Some(TextColor::Rgb(c));
                    }
                    let linear = wordcraft_doc::math::to_linear(&math.nodes);
                    self.emit_obj(sc, pb, InlineObject::Equation { linear, display: true, math }, &props);
                }
            }
            "mc:AlternateContent" => {
                if let Some(o) = self.read_ink_alternate(sc, k, rels).or_else(|| self.read_chartex_alternate(sc, k, rels)) {
                    let props = self.run_props_none(ctx);
                    self.emit_obj(sc, pb, o, &props);
                } else if let Some(c) = children_of_choice(k) {
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
                    if f.sizes.len() < 64 {
                        f.sizes.push((f.instr.len(), props.size));
                    }
                    f.instr.push_str(&k.text());
                }
            }
            "w:tab" | "w:ptab" => self.emit_text(sc, pb, "\t", props),
            "w:br" => {
                if sc.fields.iter().any(|f| f.phase == Phase::Instr) {
                    return;
                }
                let c = match k.attr("w:type") {
                    Some("page") => "\u{000C}",
                    Some("column") => "\u{000E}",
                    _ => "\n",
                };
                self.emit_text(sc, pb, c, props);
            }
            "w:cr" => {
                if !sc.fields.iter().any(|f| f.phase == Phase::Instr) {
                    self.emit_text(sc, pb, "\n", props);
                }
            }
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
                Some("begin") => {
                    let form = k.child("w:ffData").map(|f| f.to_xml()).filter(|x| x.len() <= MAX_FORM).unwrap_or_default();
                    self.begin_field(sc, props.clone(), String::new(), on_off_attr(k, "w:fldLock"), on_off_attr(k, "w:dirty"), form)
                }
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
            "w:contentPart" => {
                if let Some(o) = self.read_ink_content_part(k, rels) {
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
            // The lines of a note separator (only there).
            "w:separator" | "w:continuationSeparator" if self.in_note_sep => {
                self.emit_obj(sc, pb, InlineObject::note_separator(k.name == "w:continuationSeparator"), props);
            }
            "w:commentReference" => {
                if let Some(cid) = k.attr("w:id").and_then(|i| self.comment_id(i))
                    && !self.comments_ended.contains(&cid)
                {
                    self.comments_ended.insert(cid);
                    pb.push_obj(InlineObject::CommentEnd { id: cid }, props);
                }
            }
            "w:pgNum" => self.emit_obj(
                sc,
                pb,
                InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
                props,
            ),
            "w:ruby" => self.read_ruby(sc, pb, k, props),
            "mc:AlternateContent" => {
                if let Some(o) = self.read_ink_alternate(sc, k, rels).or_else(|| self.read_chartex_alternate(sc, k, rels)) {
                    self.emit_obj(sc, pb, o, props);
                } else if let Some(c) = children_of_choice(k) {
                    for x in c.els() {
                        self.read_run_child(sc, pb, x, props, rels, note, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    /// `w:ruby` (ECMA-376 §17.3.3.25): ruby text (`w:rt`) over base text (`w:rubyBase`), kept as
    /// one [`InlineObject::Ruby`] in the base text's formatting (the first base run's).
    fn read_ruby(&mut self, sc: &mut StoryCtx, pb: &mut PB, k: &El, props: &CharProps) {
        const MAX: usize = 4096;
        // The text of a part's runs and the formatting of its first run.
        let part = |name: &str| -> (String, Option<CharProps>) {
            let mut text = String::new();
            let mut first = None;
            for r in k.child(name).map(|e| e.children("w:r").collect::<Vec<_>>()).unwrap_or_default() {
                if first.is_none() {
                    first = Some(r.child("w:rPr").map(|p| self.pc.rpr(p)).unwrap_or_default());
                }
                for t in r.els().filter(|t| matches!(t.name.as_str(), "w:t" | "w:delText")) {
                    if text.len() < MAX {
                        text.push_str(&clean_text(&t.text()).replace('\t', " "));
                    }
                }
            }
            (text.chars().take(MAX).collect(), first)
        };
        let (base, base_props) = part("w:rubyBase");
        let (ruby, ruby_props) = part("w:rt");
        let mut bp = base_props.unwrap_or_default();
        (bp.link, bp.ins, bp.del) = (props.link.clone(), props.ins, props.del);
        if base.is_empty() {
            return;
        }
        if ruby.is_empty() {
            self.emit_text(sc, pb, &base, &bp);
            return;
        }
        let pr = k.child("w:rubyPr");
        let val = |n: &str| pr.and_then(|p| p.child(n)).and_then(|e| e.attr("w:val"));
        let pt = |n: &str| val(n).and_then(|v| measure(v, 2.0)).filter(|v| v.is_finite() && *v >= 0.0);
        let mut rp = ruby_props.unwrap_or_default();
        let base_size = pt("w:hpsBaseText").filter(|v| *v > 0.0).or(bp.size).unwrap_or(0.0).min(1584.0);
        let size = pt("w:hps").filter(|v| *v > 0.0).or(rp.size).unwrap_or(if base_size > 0.0 { base_size / 2.0 } else { 5.0 }).clamp(1.0, 1584.0);
        let raise = pt("w:hpsRaise").unwrap_or_else(|| wordcraft_doc::para::ruby_raise(0.0, base_size.max(size), size)).min(1584.0);
        // The ruby's size is `size`; its run keeps what it sets beyond the base text's formatting
        // (writers repeat the base's).
        (rp.size, rp.size_cs) = (None, None);
        let rp = rp.minus(&bp);
        let obj = InlineObject::Ruby {
            base,
            ruby,
            align: val("w:rubyAlign").and_then(wordcraft_doc::para::RubyAlign::from_ooxml).unwrap_or_default(),
            size,
            raise,
            base_size,
            lang: val("w:lid").map(|l| l.chars().take(32).collect()).unwrap_or_default(),
            props: Box::new(rp),
        };
        self.emit_obj(sc, pb, obj, &bp);
    }

    // ---- fields ----

    fn begin_field(&mut self, sc: &mut StoryCtx, props: CharProps, instr: String, locked: bool, dirty: bool, form: String) {
        if sc.fields.len() >= MAX_FIELD_DEPTH || sc.ignored_begins > 0 {
            sc.ignored_begins += 1;
            return;
        }
        // Fields nested deeper than the cap are ignored (their content flows to the enclosing field).
        sc.fields.push(FieldState {
            instr,
            phase: Phase::Instr,
            spilled: false,
            buf: Vec::new(),
            props,
            locked,
            dirty,
            range: false,
            sizes: Vec::new(),
            form,
        });
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
            let (obj, props) = (InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, f.props.clone());
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
            self.emit_obj(sc, pb, InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, &f.props);
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        // A cached result holding a picture, chart or shape (`INCLUDEPICTURE`'s embedded
        // picture) can't be a text result: keep the field as a range around it so the picture
        // stays in the document and is saved again. The linked source is never read.
        // A tracked insertion or deletion inside the result (one the field itself isn't wrapped
        // in) keeps the field a range too: a cached text result has one revision at most, so
        // each piece stays an ordinary run with its own `w:ins` / `w:del` (#1037).
        let tracked_inside = f.buf.iter().any(|(_, p)| (p.ins, p.del) != (f.props.ins, f.props.del));
        if !nested_in_instr && (tracked_inside || f.buf.iter().any(|(it, _)| matches!(it, Item::Obj(o) if o.frame().is_some()))) {
            self.emit_obj(sc, pb, InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, &f.props);
            for (it, p) in f.buf {
                match it {
                    Item::Text(t) => self.emit_text(sc, pb, &t, &p),
                    Item::Obj(o) => self.emit_obj(sc, pb, *o, &p),
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
        let (instr, result) = match enclosure(&f) {
            // Word writes no result for enclosed characters: the text is their plain text.
            Some(e) => (e.instr(), if result.is_empty() { e.text } else { result }),
            None => (f.instr.trim().to_string(), result),
        };
        let obj = InlineObject::Field { instr, result, locked: f.locked, dirty: f.dirty, code: false, form: f.form };
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
            let form = std::mem::take(&mut f.form);
            pb.push_obj(
                InlineObject::Field { instr: f.instr.trim().to_string(), result: String::new(), locked: f.locked, dirty: f.dirty, code: false, form },
                &f.props,
            );
            for (it, p) in f.buf.drain(..) {
                match it {
                    Item::Text(t) => pb.push_text(&t, &p),
                    Item::Obj(o) => pb.push_obj(*o, &p),
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
                    f.buf.push((Item::Obj(Box::new(o)), props.clone()));
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
        let (alt, meta) = nv_alt(c.child("wp:docPr"));
        let mut float = if anchored { anchor_float(c) } else { Float::default() };
        float.alt = meta;
        float.hidden = c.child("wp:docPr").is_some_and(|p| on_off_attr(p, "hidden"));
        if let Some(e) = c.child("wp:effectExtent") {
            for (slot, n) in float.effect.iter_mut().zip(["l", "t", "r", "b"]) {
                // Negative for a rotated object narrower than its frame; settled below.
                *slot = e.attr(n).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(-1584.0, 1584.0);
            }
        }
        let gd = c.child("a:graphic").and_then(|g| g.child("a:graphicData"))?;
        // Checked by URI before the blip search: a diagram's data can hold a blip further down.
        let uri = gd.attr("uri").unwrap_or("");
        if uri == INK_URI {
            let content = gd.child("w14:contentPart")?;
            let part = super::part_of(rels, content.attr("r:id")?, INK_REL)?;
            let pkg = self.pkg;
            if self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(&part) != Some("application/inkml+xml") {
                return None;
            }
            let source = self.keep_embedded(c.child("a:graphic")?, rels, &[])?;
            float.set_spin(xfrm_spin(content.child("w14:xfrm")));
            let (px, py) = float.spin_pad(w, h);
            for (e, p) in float.effect.iter_mut().zip([px, py, px, py]) {
                *e = (*e - p).clamp(0.0, 1584.0);
            }
            // The renderer's existing missing-picture box marks ink without a usable preview.
            // InkML stays opaque: no approximation of its strokes or conversion to freeforms.
            let items = vec![GraphicItem::Image { rect: [0.0, 0.0, w, h], media: String::new() }];
            let graphic = Arc::new(Graphic { kind: GraphicKind::Ink, items, w, h, source: Some(source), ..Default::default() });
            return Some(InlineObject::Graphic { w, h, alt, float, graphic });
        }
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
                GraphicKind::Chart | GraphicKind::Ink => Vec::new(),
                GraphicKind::Diagram => self.diagram_drawing_rel(gd, rels).into_iter().collect(),
            };
            let source = c.child("a:graphic").and_then(|g| self.keep_embedded(g, rels, &extra));
            let graphic = self.graphic(kind, gd, rels, w, h, source);
            float.effect = float.effect.map(|e| e.max(0.0));
            return Some(InlineObject::Graphic { w, h, alt, float, graphic });
        }
        let mut obj = if let Some(c) = gd.child("wpc:wpc") {
            self.read_canvas(sc, c, rels, w, h, float)
        } else if let Some(g) = gd.child("wpg:wgp") {
            self.read_group(sc, g, rels, w, h, float)?
        } else if gd.find("a:blip").is_some() {
            self.read_pic(gd, rels, w, h, alt.clone(), float)?
        } else {
            let wsp = gd.find("wps:wsp")?;
            let mut obj = self.read_wsp(sc, wsp, rels, w, h, float);
            // A freeform: whether it's ink WordCraft wrote.
            if let InlineObject::Shape { freeform: Some(f), .. } = &mut obj
                && let Some(name) = c.child("wp:docPr").and_then(|p| p.attr("name"))
                && let Some(tool) = super::freeform::ink_tool(name)
            {
                let f = Arc::make_mut(f);
                f.ink = Some(tool);
                f.order = super::freeform::ink_order(name);
            }
            obj
        };
        if !alt.is_empty() {
            obj.set_alt_text(&alt);
        }
        self.set_shape_id(&mut obj, c.child("wp:docPr").and_then(|p| p.attr("id")));
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

    /// Keep the selected native-ink branch and its picture fallback as one object. Reading only
    /// the fallback flattens the ink; reading only the choice loses its preview on the next save.
    fn read_ink_alternate(&mut self, sc: &mut StoryCtx, ac: &El, rels: &Rels) -> Option<InlineObject> {
        let choice = children_of_choice(ac).filter(|e| e.name == "mc:Choice")?;
        let drawing = choice.child("w:drawing")?;
        if drawing.find("a:graphicData")?.attr("uri") != Some(INK_URI) {
            return None;
        }
        let fallback = ac.child("mc:Fallback");
        let preview = fallback.and_then(|f| f.child("w:pict")).and_then(|p| self.read_vml(sc, p, rels));
        let Some(mut object) = self.read_drawing(sc, drawing, rels) else { return preview };
        if let InlineObject::Graphic { w, h, graphic, .. } = &mut object {
            let graphic = Arc::make_mut(graphic);
            if let Some(InlineObject::Image { media, .. }) = preview {
                graphic.items = vec![GraphicItem::Image { rect: [0.0, 0.0, *w, *h], media }];
            }
            if let Some(fallback) = fallback {
                let kept =
                    El { name: "mc:AlternateContent".into(), attrs: Vec::new(), kids: vec![Node::El(choice.clone()), Node::El(fallback.clone())] };
                if let Some(source) = self.keep_embedded(&kept, rels, &[]) {
                    graphic.source = Some(source);
                }
                // A malformed or over-budget fallback must not discard a valid native part.
            }
        }
        Some(object)
    }

    /// An Office 2016 chart (`cx:chart`: waterfall, sunburst, box & whisker…) drawn from its
    /// fallback picture, keeping both branches so saving writes the chart back rather than only
    /// the picture. `None` (read the fallback alone) when it can't be kept whole.
    fn read_chartex_alternate(&mut self, sc: &mut StoryCtx, ac: &El, rels: &Rels) -> Option<InlineObject> {
        let choice = ac
            .children("mc:Choice")
            .find(|c| c.child("w:drawing").and_then(|d| d.find("a:graphicData")).and_then(|g| g.attr("uri")) == Some(CHARTEX_URI))?;
        // A requirement in a namespace we can't name couldn't be written back.
        let requires = choice.attr("Requires").unwrap_or("");
        if requires.split_whitespace().next().is_none() || requires.split_whitespace().any(|p| p == "?") {
            return None;
        }
        let fallback = ac.child("mc:Fallback")?;
        let kept = El { name: "mc:AlternateContent".into(), attrs: Vec::new(), kids: vec![Node::El(choice.clone()), Node::El(fallback.clone())] };
        let source = self.keep_embedded(&kept, rels, &[])?;
        let Some(InlineObject::Image { media, w, h, alt, float, .. }) = self.read_drawing(sc, fallback.child("w:drawing")?, rels) else {
            return None;
        };
        let items = vec![GraphicItem::Image { rect: [0.0, 0.0, w, h], media }];
        let graphic = Arc::new(Graphic { kind: GraphicKind::Chart, items, w, h, source: Some(source), ..Default::default() });
        Some(InlineObject::Graphic { w, h, alt, float, graphic })
    }

    fn read_ink_content_part(&mut self, content: &El, rels: &Rels) -> Option<InlineObject> {
        let part = super::part_of(rels, content.attr("r:id")?, INK_REL)?;
        let pkg = self.pkg;
        if self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(&part) != Some("application/inkml+xml") {
            return None;
        }
        let source = self.keep_embedded(content, rels, &[])?;
        let graphic = Arc::new(Graphic { kind: GraphicKind::Ink, source: Some(source), ..Default::default() });
        Some(InlineObject::Graphic { w: 0.0, h: 0.0, alt: String::new(), float: Float::default(), graphic })
    }

    /// The chart or SmartArt diagram in an `a:graphicData`, as items in points from its top-left
    /// corner. Charts are drawn from their cached data, SmartArt diagrams from the drawing Word
    /// stores beside them. Built once per part and size; once the file's graphic budget is spent,
    /// later ones are empty.
    fn graphic(&mut self, kind: GraphicKind, gd: &El, rels: &Rels, w: f32, h: f32, source: Option<Arc<Embedded>>) -> Arc<Graphic> {
        let part = match kind {
            GraphicKind::Chart => gd.child("c:chart").and_then(|c| c.attr("r:id")).and_then(|id| super::part_of(rels, id, rt::CHART)),
            GraphicKind::Diagram => self.diagram_part(gd, rels),
            GraphicKind::Ink => None,
        };
        let Some(path) = part else { return Arc::new(Graphic { kind, items: Vec::new(), w, h, source, chart: None, smart_art: None }) };
        let key = (kind, path, w.to_bits(), h.to_bits());
        if let Some(g) = self.graphics.get(&key) {
            return g.clone();
        }
        let items = if self.graphic_budget == 0 { Vec::new() } else { self.graphic_items(kind, &key.1, w, h) };
        self.graphic_budget = self.graphic_budget.saturating_sub(graphic_work(&items));
        // A chart or SmartArt that is exactly what WordCraft writes for its model can be edited
        // (and is written from the model); any other stays as it was read.
        let chart = match kind {
            GraphicKind::Chart if !items.is_empty() => self.graphic_part(&key.1).and_then(|space| crate::chart_spec::spec_of(&space)).map(Arc::new),
            _ => None,
        };
        let smart_art = match kind {
            GraphicKind::Diagram if !items.is_empty() => self.smart_art_spec(gd, rels).map(Arc::new),
            _ => None,
        };
        let source = if chart.is_some() || smart_art.is_some() { None } else { source };
        let g = Arc::new(Graphic { kind, items, w, h, source, chart, smart_art });
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
            GraphicKind::Ink => Vec::new(),
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
        Some(InlineObject::Group { w, h, float, ch_w, ch_h, children, canvas: None })
    }

    /// A `wpc:wpc` Drawing Canvas (`w` × `h`, ECMA-376 Part 1 §20.4 and the Word 2010 canvas
    /// schema): its `wpc:bg` fill, `wpc:whole` outline and members, which sit in the canvas's
    /// own space (offsets from its top-left corner, unscaled). An empty canvas is kept.
    /// The canvas schema has no `a:xfrm` of its own (nor does the anchor), so a turned or
    /// flipped canvas keeps its turn in our `wpc:extLst` extension ([`CANVAS_SPIN_EXT`]).
    fn read_canvas(&mut self, sc: &mut StoryCtx, c: &El, rels: &Rels, w: f32, h: f32, mut float: Float) -> InlineObject {
        let spin_ext = c.child("wpc:extLst").and_then(|l| l.els().find(|e| e.name == "a:ext" && e.attr("uri") == Some(crate::CANVAS_SPIN_EXT)));
        float.set_spin(xfrm_spin(spin_ext));
        let solid = |e: Option<&El>| {
            e.and_then(|f| f.child("a:solidFill")).and_then(|f| f.child("a:srgbClr")).and_then(|c| c.attr("val")).and_then(Rgb::parse)
        };
        let ln = c.child("wpc:whole").and_then(|e| e.child("a:ln"));
        let style = wordcraft_doc::para::CanvasStyle {
            fill: solid(c.child("wpc:bg")),
            stroke: solid(ln),
            stroke_width: ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(0.0, 100.0),
        };
        let mut children = Vec::new();
        self.group_members(sc, c, rels, (0.0, 0.0, 1.0, 1.0), 0, &mut children);
        InlineObject::Group { w, h, float, ch_w: w.max(1.0), ch_h: h.max(1.0), children, canvas: Some(style) }
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
                // A group inside a canvas is a whole `wpg:wgp`.
                "wpg:grpSp" | "wpg:wgp" => e.child("wpg:grpSpPr"),
                _ => continue,
            };
            let x = group_xfrm(sppr);
            let ((ox, oy), (ew, eh)) = (x.off, x.ext);
            if e.name == "wpg:grpSp" || e.name == "wpg:wgp" {
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
            let nv = if e.name == "pic:pic" { e.find("pic:cNvPr") } else { e.child("wps:cNvPr") };
            let (alt, meta) = nv_alt(nv);
            let float = Float { alt: meta, hidden: nv.is_some_and(|p| on_off_attr(p, "hidden")), ..Float::default() };
            let obj = if e.name == "pic:pic" {
                self.read_pic(e, rels, w, h, alt.clone(), float)
            } else {
                let mut o = self.read_wsp(sc, e, rels, w, h, float);
                self.set_shape_id(&mut o, e.child("wps:cNvPr").and_then(|p| p.attr("id")));
                Some(o)
            };
            let obj = obj.map(|mut o| {
                o.set_alt_text(&alt);
                o
            });
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
        let mut kind = preset_kind(prst);
        let is_tb = wsp.child("wps:cNvSpPr").and_then(|c| c.attr("txBox")).is_some_and(|v| v == "1" || v == "true");
        if is_tb || (txbx.is_some() && kind == ShapeKind::Rectangle) {
            kind = ShapeKind::TextBox;
        }
        // Explicit paint, else the theme's through the style references (#611).
        let paint = shape_paint(sppr, wsp.child("wps:style"), &self.doc.settings.theme_colors);
        let (stroke, stroke_width) = (paint.stroke, paint.stroke_width);
        let fill = if kind.is_open() { None } else { paint.fill };
        let ln = sppr.and_then(|s| s.child("a:ln"));
        let effects = sppr.and_then(|s| s.child("a:effectLst")).map(effect_list).unwrap_or_default();
        let story = match txbx {
            Some(t) if sc.story_depth < MAX_STORY_DEPTH => Some(self.read_textbox(sc, t, rels)),
            _ => None,
        };
        // A box continuing another's text (Create Link): its own, empty story, linked up once the
        // whole document is read.
        let linked = wsp.child("wps:linkedTxbx").filter(|_| txbx.is_none());
        let story = match linked {
            Some(l) => {
                kind = ShapeKind::TextBox;
                let id = l.attr("id").and_then(|v| v.trim().parse::<u32>().ok());
                let seq = l.attr("seq").and_then(|v| v.trim().parse::<u32>().ok());
                match (id, seq) {
                    (Some(id), Some(seq)) if seq > 0 => {
                        let part = self.doc.add_part(PartKind::TextBox, Blocks::new());
                        self.txbx_links.push((id, seq, part));
                        Some(part)
                    }
                    _ => story,
                }
            }
            None => {
                if let (Some(part), Some(id)) = (story, wsp.child("wps:txbx").and_then(|t| t.attr("id")).and_then(|v| v.trim().parse::<u32>().ok())) {
                    self.txbx_links.push((id, 0, part));
                }
                story
            }
        };
        // Text direction and alignment (`wps:bodyPr`).
        if let (Some(id), Some(bp)) = (story, wsp.child("wps:bodyPr"))
            && let Some(part) = self.doc.parts.get_mut(&id)
        {
            if let Some(v) = bp.attr("vert") {
                part.body.vert = wordcraft_doc::props::TextVert::from_ooxml(v);
            }
            if let Some(a) = bp.attr("anchor") {
                part.body.anchor = wordcraft_doc::props::valign_from_anchor(a);
            }
            // Internal margins, each defaulting to Word's (#743).
            let mut inset = wordcraft_doc::TEXT_BOX_INSET;
            for (slot, n) in inset.iter_mut().zip(["lIns", "tIns", "rIns", "bIns"]) {
                if let Some(v) = bp.attr(n).and_then(int) {
                    *slot = u32::try_from(v.clamp(0, i64::from(wordcraft_doc::MAX_TEXT_BOX_INSET))).unwrap_or(0);
                }
            }
            part.body.inset = (inset != wordcraft_doc::TEXT_BOX_INSET).then_some(inset);
            part.body.auto_fit = bp.child("a:spAutoFit").is_some();
        }
        let mut extra = ShapeExtra::default();
        // A rounded rectangle's own corner (`adj`, ECMA-376 §20.1.10.56); other presets drawn as
        // rounded rectangles keep the default.
        if prst == "roundRect" {
            extra.corner = sppr
                .and_then(|s| s.child("a:prstGeom"))
                .and_then(|g| preset_adj(g, "adj"))
                .and_then(|v| u32::try_from(v.clamp(0, i64::from(wordcraft_doc::connector::MAX_CORNER))).ok());
        }
        // Arrowheads: any end type but `none` (ECMA-376 §20.1.10.33) is drawn as a triangle.
        let arrow = |tag: &str| ln.and_then(|l| l.child(tag)).and_then(|e| e.attr("type")).is_some_and(|t| t != "none");
        extra.arrow_start = arrow("a:headEnd");
        extra.arrow_end = arrow("a:tailEnd");
        // A connector's glued ends (ECMA-376 §20.1.2.2.36, §20.1.2.2.13).
        if let Some(cn) = wsp.child("wps:cNvCnPr") {
            let end = |tag: &str| {
                let e = cn.child(tag)?;
                let id = e.attr("id").and_then(u32_of).filter(|v| *v != 0)?;
                let site = e.attr("idx").and_then(u32_of).unwrap_or(0).min(wordcraft_doc::connector::MAX_SITE);
                Some(ConnEnd { id, site })
            };
            extra.start = end("a:stCxn");
            extra.end = end("a:endCxn");
            self.glued.extend(extra.start.iter().chain(extra.end.iter()).map(|e| e.id));
        }
        extra.warp = wsp.child("wps:bodyPr").and_then(|b| b.child("a:prstTxWarp")).and_then(text_warp);
        // A freeform: its custom geometry (#743: in a group too).
        let freeform = sppr.and_then(|s| super::freeform::cust_geom(s, w, h)).map(|mut f| {
            f.alpha = super::freeform::line_alpha(ln);
            kind = ShapeKind::Freeform;
            Arc::new(f)
        });
        InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, story, freeform, effects, extra }
    }

    /// Keep drawing id `id` on a shape read from a file (see [`Reader::glued`]).
    fn set_shape_id(&mut self, obj: &mut InlineObject, id: Option<&str>) {
        if let InlineObject::Shape { extra, .. } = obj
            && let Some(id) = id.and_then(u32_of).filter(|v| *v != 0)
        {
            extra.id = id;
            self.shape_ids = true;
        }
    }

    fn read_textbox(&mut self, sc: &StoryCtx, content: &El, rels: &Rels) -> u32 {
        let mut inner = StoryCtx { story_depth: sc.story_depth + 1, ..Default::default() };
        let mut blocks = Blocks::new();
        self.read_blocks(&mut inner, content, rels, &mut blocks, 0);
        self.flush_pending(&mut inner, &mut blocks);
        self.doc.add_part(PartKind::TextBox, blocks)
    }

    /// Legacy VML pictures, text boxes, lines, rectangles, ovals and preset shapes (best effort).
    fn read_vml(&mut self, sc: &mut StoryCtx, pict: &El, rels: &Rels) -> Option<InlineObject> {
        let shape = pict.els().find(|e| e.name.starts_with("v:") && e.local() != "shapetype")?;
        if sc.in_hf
            && let Some(wm) = vml_watermark(shape)
        {
            // The document's watermark (the first one found); drawn on every page, not as a shape.
            self.doc.settings.watermark.get_or_insert(wm);
            return None;
        }
        let style = shape.attr("style").unwrap_or("");
        let len = |k: &str| vml_style(style, k).and_then(|v| measure(v, 1.0)).unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT);
        let (mut w, mut h) = (len("width"), len("height"));
        let mut float = vml_float(shape, style);
        if let Some(img) = shape.find("v:imagedata") {
            let id = img.attr("r:id").or_else(|| img.attr("r:pict"))?;
            let media = self.media_for(rels, id)?;
            let alt = shape.attr("alt").or_else(|| img.attr("o:title")).unwrap_or("").to_string();
            return Some(InlineObject::Image { media, w, h, alt, float, crop: [0.0; 4], ole: None });
        }
        // The shapetype the shape is drawn from (its `type`, ECMA-376 Part 4 VML), when written in
        // the same picture: what the shape leaves out, it takes from there.
        let type_id = shape.attr("type").map(|t| t.trim().trim_start_matches('#'));
        let shapetype = type_id.and_then(|id| pict.els().find(|e| e.name == "v:shapetype" && e.attr("id") == Some(id)));
        let attr = |name: &str| shape.attr(name).or_else(|| shapetype.and_then(|t| t.attr(name)));
        let on = |name: &str| attr(name).is_none_or(|v| !matches!(v, "f" | "false" | "0"));
        let stroke = on("stroked").then(|| attr("strokecolor").map_or(Some(Rgb::BLACK), vml_color)).flatten();
        let stroke_width = attr("strokeweight").and_then(|v| measure(v, 1.0)).unwrap_or(0.75).clamp(0.0, 100.0);
        let fill = on("filled").then(|| attr("fillcolor").map_or(Some(Rgb::WHITE), vml_color)).flatten();
        let kind = match shape.local() {
            "line" => Some(ShapeKind::Line),
            "rect" => Some(ShapeKind::Rectangle),
            "roundrect" => Some(ShapeKind::RoundedRectangle),
            "oval" => Some(ShapeKind::Ellipse),
            // A preset, unless the shape's own path replaces its shapetype's geometry.
            "shape" if shape.attr("path").is_none() && shape.child("v:path").and_then(|p| p.attr("v")).is_none() => {
                vml_spt(shape, shapetype, type_id).and_then(vml_preset_kind)
            }
            _ => None,
        };
        if let Some((tb, t)) = vml_textbox(shape, 0)
            && sc.story_depth < MAX_STORY_DEPTH
        {
            let id = self.read_textbox(sc, t, rels);
            if let Some(part) = self.doc.parts.get_mut(&id) {
                // Internal margins `inset="l,t,r,b"` (ECMA-376 Part 4 §19.1.2.22), each one left
                // out keeping Word's default; a bare number is in EMUs, VML's base unit.
                // `o:insetmode="auto"` leaves them to the application.
                let mut inset = wordcraft_doc::TEXT_BOX_INSET;
                let custom = tb.attr("o:insetmode").is_none_or(|m| !m.eq_ignore_ascii_case("auto"));
                for (slot, v) in inset.iter_mut().zip(tb.attr("inset").filter(|_| custom).unwrap_or("").split(',')) {
                    if let Some(pt) = measure(v, 12_700.0) {
                        *slot = (pt.max(0.0) * 12_700.0).round().min(wordcraft_doc::MAX_TEXT_BOX_INSET as f32) as u32;
                    }
                }
                part.body.inset = (inset != wordcraft_doc::TEXT_BOX_INSET).then_some(inset);
                // Resize shape to fit text (DrawingML's `a:spAutoFit`).
                let fit = vml_style(tb.attr("style").unwrap_or(""), "mso-fit-shape-to-text");
                part.body.auto_fit = fit.is_some_and(|v| v.eq_ignore_ascii_case("t") || v.eq_ignore_ascii_case("true"));
                // Where the text sits between the box's top and bottom (DrawingML's `anchor`).
                if let Some(a) = vml_style(style, "v-text-anchor") {
                    part.body.anchor = match a {
                        a if a.starts_with("middle") => wordcraft_doc::props::VAlign::Center,
                        a if a.starts_with("bottom") => wordcraft_doc::props::VAlign::Bottom,
                        _ => wordcraft_doc::props::VAlign::Top,
                    };
                }
            }
            let story = Some(id);
            // Text in a preset shape keeps the shape around it; otherwise it is a text box.
            let kind = kind.filter(|k| *k != ShapeKind::Rectangle && !k.is_open()).unwrap_or(ShapeKind::TextBox);
            return Some(InlineObject::Shape {
                kind,
                w,
                h,
                fill,
                stroke,
                stroke_width,
                float,
                story,
                freeform: None,
                effects: Default::default(),
                extra: Default::default(),
            });
        }
        // A freeform: its `path`, in its `coordsize` space; a `v:fill` with no opacity
        // or `nf` in the path leaves it unfilled. Not WordArt (`v:textpath`).
        if shape.local() == "shape"
            && shape.child("v:textpath").is_none()
            && let Some(path) = shape.attr("path")
            && let Some(vp) = super::freeform::vml_path(path, shape.attr("coordsize"), shape.attr("coordorigin"), w, h)
        {
            let opacity = |el: &str| shape.child(el).and_then(|e| e.attr("opacity")).and_then(vml_fraction).unwrap_or(1.0);
            let fill = fill.filter(|_| !vp.no_fill && opacity("v:fill") > 0.0);
            let stroke = stroke.filter(|_| !vp.no_stroke);
            if stroke.is_none() && fill.is_none() {
                return None;
            }
            let mut f = vp.freeform;
            f.alpha = opacity("v:stroke");
            return Some(InlineObject::Shape {
                kind: ShapeKind::Freeform,
                w,
                h,
                fill,
                stroke,
                stroke_width,
                float,
                story: None,
                freeform: Some(Arc::new(f)),
                effects: Default::default(),
                extra: Default::default(),
            });
        }
        let kind = kind?;
        let mut extra = ShapeExtra::default();
        if kind == ShapeKind::Line {
            // Arrowheads (`v:stroke` startarrow / endarrow): any but `none` is drawn.
            let arrow = |name: &str| shape.child("v:stroke").and_then(|s| s.attr(name)).is_some_and(|v| v != "none");
            extra.arrow_start = arrow("startarrow");
            extra.arrow_end = arrow("endarrow");
        }
        if shape.local() == "line" {
            // A line runs `from` → `to` (points, offsets from where its style places it). A preset
            // line (`v:shape`) runs corner to corner of the box its style gives it.
            let pt = |name: &str| -> (f32, f32) {
                let v = shape.attr(name).unwrap_or("0,0");
                let (x, y) = v.split_once(',').unwrap_or((v, "0"));
                let m = |s: &str| measure(s, 1.0).map_or(0.0, wordcraft_geom::finite).clamp(-crate::units::MAX_LEN_PT, crate::units::MAX_LEN_PT);
                (m(x), m(y))
            };
            let ((x0, y0), (x1, y1)) = (pt("from"), pt("to"));
            let max = crate::units::MAX_LEN_PT;
            (w, h) = ((x1 - x0).abs().min(max), (y1 - y0).abs().min(max));
            float.x = (float.x + x0.min(x1)).clamp(-max, max);
            float.y = (float.y + y0.min(y1)).clamp(-max, max);
            // Drawn top-left to bottom-right; flipped for the other directions.
            float.flip_h ^= x1 < x0;
            float.flip_v ^= y1 < y0;
        }
        if stroke.is_none() && (fill.is_none() || kind == ShapeKind::Line) {
            return None;
        }
        Some(InlineObject::Shape {
            kind,
            w,
            h,
            fill: fill.filter(|_| kind != ShapeKind::Line),
            stroke,
            stroke_width,
            float,
            story: None,
            freeform: None,
            effects: Default::default(),
            extra,
        })
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

/// Gather `name` children (rows, cells), looking through `w:sdt`/`w:customXml` wrappers; each
/// with the content controls that open before it and how many close after it. `pending` holds
/// controls opened but not yet given to an element.
fn collect<'a>(e: &'a El, name: &str, out: &mut Vec<(&'a El, ControlWrap)>, pending: &mut Vec<ContentControl>, depth: usize) {
    if depth > 8 {
        return;
    }
    for k in e.els() {
        if k.name == name {
            out.push((k, ControlWrap { open: std::mem::take(pending), close: 0 }));
        } else if k.name == "w:sdt" {
            if let Some(c) = k.child("w:sdtContent") {
                let mut ctl = super::sdt::sdt_pr(k.child("w:sdtPr"), true);
                ctl.end_pr_xml = super::sdt::sdt_end_pr(k.child("w:sdtEndPr"));
                let before = out.len();
                pending.push(ctl);
                collect(c, name, out, pending, depth + 1);
                let grew = out.len() > before;
                match out.last_mut() {
                    Some((_, w)) if grew => w.close = w.close.saturating_add(1),
                    // Nothing inside: the control goes (as before).
                    _ => {
                        pending.pop();
                    }
                }
            }
        } else if k.name == "w:customXml" {
            collect(k, name, out, pending, depth + 1);
        }
    }
}

/// Placeholder text as read from a control showing it: one line, bounded.
fn placeholder_of(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(wordcraft_doc::control::MAX_CONTROL_TEXT).collect::<String>().trim().to_string()
}

fn on_off_attr(e: &El, name: &str) -> bool {
    e.attr(name).is_some_and(|v| !matches!(v, "0" | "false" | "off"))
}

/// The alternative text of a `wp:docPr`, `pic:cNvPr` or `wps:cNvPr` (ECMA-376
/// `CT_NonVisualDrawingProps`): the description (the title when there is none, as before), and
/// the title and decorative mark (the `adec:decorative` extension of [MS-ODRAWXML]).
fn nv_alt(p: Option<&El>) -> (String, AltText) {
    let Some(p) = p else { return Default::default() };
    let title = cap_alt(p.attr("title").unwrap_or(""));
    let descr = p.attr("descr").filter(|s| !s.is_empty()).map(cap_alt).unwrap_or_else(|| title.clone());
    let decorative = p
        .child("a:extLst")
        .into_iter()
        .flat_map(|l| l.children("a:ext"))
        .filter(|x| x.attr("uri").is_some_and(|u| u.eq_ignore_ascii_case(crate::xml::DECORATIVE_EXT)))
        .flat_map(|x| x.els())
        .any(|d| d.local() == "decorative" && on_off_attr(d, "val"));
    (descr, AltText { text: String::new(), title, decorative })
}

fn anchor_float(c: &El) -> Float {
    let mut f = Float { wrap: Wrap::Square, ..Float::default() };
    let behind = on_off_attr(c, "behindDoc");
    // Word wraps a position relative to the page's size (`wp14:pctPosVOffset`) in
    // `mc:AlternateContent`; its Fallback gives the same position as an offset (#743).
    let kids = c.els().flat_map(|k| match k.name.as_str() {
        "mc:AlternateContent" => k.child("mc:Fallback").map(|f| f.els().collect()).unwrap_or_default(),
        _ => vec![k],
    });
    for k in kids {
        match k.name.as_str() {
            "wp:wrapSquare" => {
                f.wrap = Wrap::Square;
                f.wrap_text = k.attr("wrapText").and_then(WrapText::from_ooxml).unwrap_or_default();
            }
            "wp:wrapTight" | "wp:wrapThrough" => {
                f.wrap = if k.name == "wp:wrapTight" { Wrap::Tight } else { Wrap::Through };
                f.wrap_text = k.attr("wrapText").and_then(WrapText::from_ooxml).unwrap_or_default();
                f.wrap_polygon = k.child("wp:wrapPolygon").and_then(wrap_polygon).map(Arc::new);
            }
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
    // Required by the schema; a file leaving it out lays out in the cell, as before.
    f.outside_cell = !on_off_attr(c, "layoutInCell") && c.attr("layoutInCell").is_some();
    f
}

/// A `wp:wrapPolygon`: `wp:start`, then the `wp:lineTo` points (ECMA-376 Part 1, §20.4.2.16).
fn wrap_polygon(e: &El) -> Option<wordcraft_doc::wrap::WrapPolygon> {
    let pts = e
        .els()
        .filter(|k| k.name == "wp:start" || k.name == "wp:lineTo")
        .take(wordcraft_doc::wrap::MAX_WRAP_POINTS + 1)
        .filter_map(|k| Some((int(k.attr("x")?)?, int(k.attr("y")?)?)));
    let mut pts: Vec<(i64, i64)> = pts.collect();
    // The last point usually repeats the start to close the polygon.
    if pts.len() > 3 && pts.first() == pts.last() {
        pts.pop();
    }
    let edited = on_off_attr(e, "edited");
    // An unedited polygon round the whole frame says nothing the object doesn't (it's what we
    // write when there's no outline): keep deriving the outline.
    let s = i64::from(wordcraft_doc::wrap::WRAP_SPACE);
    let mut corners = pts.clone();
    corners.sort_unstable();
    if !edited && corners == [(0, 0), (0, s), (s, 0), (s, s)] {
        return None;
    }
    wordcraft_doc::wrap::WrapPolygon::new(edited, pts)
}

/// A VML shape's preset type (`o:spt`): its own, its shapetype's, or the number in Word's
/// shapetype id `_x0000_tNN` (`type_id`), Word writing each shapetype only at its first use.
fn vml_spt(shape: &El, shapetype: Option<&El>, type_id: Option<&str>) -> Option<u32> {
    let num = |v: &str| v.trim().parse::<u32>().ok();
    shape
        .attr("o:spt")
        .and_then(num)
        .or_else(|| shapetype.and_then(|t| t.attr("o:spt")).and_then(num))
        .or_else(|| type_id?.strip_prefix("_x0000_t").and_then(num))
}

/// The value of property `key` in a VML `style` attribute (CSS declarations; the last one wins).
/// The first `v:textbox` under `e` holding text, with its `w:txbxContent` (depth-first, bounded
/// like `El::find`): in a group, a member's margins go with that member's text.
fn vml_textbox(e: &El, depth: usize) -> Option<(&El, &El)> {
    if depth > crate::xml::MAX_DEPTH {
        return None;
    }
    e.els().find_map(|c| match c.child("w:txbxContent") {
        Some(t) if c.name == "v:textbox" => Some((c, t)),
        _ => vml_textbox(c, depth + 1),
    })
}

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
    let hidden = get("visibility").is_some_and(|v| v.eq_ignore_ascii_case("hidden"));
    if !get("position").is_some_and(|p| p.eq_ignore_ascii_case("absolute")) {
        // Inline: only the turn (an OLE object turned in WordCraft is saved so, #319).
        let mut f = Float { hidden, ..Float::default() };
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
        hidden,
        // Word's picture and text watermarks: placed on the page even from a table cell.
        outside_cell: shape.attr("o:allowincell").is_some_and(|v| matches!(v, "f" | "false" | "0")),
        ..Float::default()
    };
    f.set_spin(vml_spin(style));
    f
}

/// Word's text watermark (Design › Watermark): a VML shape in a header whose `v:textpath` draws
/// its `string` as text (ECMA-376 Part 4, VML `textpath`), in the textpath's `font-family`, filled
/// with the shape's `fillcolor`, semitransparent when its `v:fill` has an `opacity` below 1, and
/// diagonal when the shape is turned. Only Word's watermark shape counts (its `id`), or text
/// placed absolutely and centred both ways; other header WordArt (a logo or a title) is None.
fn vml_watermark(shape: &El) -> Option<wordcraft_doc::Watermark> {
    let tp = shape.child("v:textpath")?;
    if tp.attr("on").is_some_and(|v| matches!(v, "f" | "false" | "0")) {
        return None;
    }
    let style = shape.attr("style").unwrap_or("");
    let centred = |k: &str| vml_style(style, k).is_some_and(|v| v.eq_ignore_ascii_case("center"));
    let word = shape.attr("id").is_some_and(|id| id.starts_with("PowerPlusWaterMarkObject"));
    let placed = vml_style(style, "position").is_some_and(|p| p.eq_ignore_ascii_case("absolute"))
        && centred("mso-position-horizontal")
        && centred("mso-position-vertical");
    if !word && !placed {
        return None;
    }
    let text: String = tp.attr("string")?.chars().take(200).collect();
    if text.trim().is_empty() {
        return None;
    }
    let default = wordcraft_doc::Watermark::default();
    let font = tp
        .attr("style")
        .and_then(|s| vml_style(s, "font-family"))
        .map(|f| f.trim_matches(|c: char| c == '"' || c == '\'' || c.is_whitespace()).chars().take(100).collect::<String>())
        .filter(|f| !f.is_empty())
        .unwrap_or(default.font);
    let color = shape.attr("fillcolor").and_then(vml_color).unwrap_or(default.color);
    let opacity = shape.child("v:fill").and_then(|f| f.attr("opacity")).and_then(vml_fraction).unwrap_or(1.0);
    Some(wordcraft_doc::Watermark { text, font, color, diagonal: vml_spin(style).deg != 0.0, semitransparent: opacity < 1.0 })
}

/// A VML fraction (`opacity`): a number, a percentage, or 65536ths with an `f` suffix; 0..=1.
fn vml_fraction(v: &str) -> Option<f32> {
    let v = v.trim();
    let f = if let Some(n) = v.strip_suffix('f') {
        n.trim().parse::<f32>().ok()? / 65_536.0
    } else if let Some(n) = v.strip_suffix('%') {
        n.trim().parse::<f32>().ok()? / 100.0
    } else {
        v.parse::<f32>().ok()?
    };
    f.is_finite().then(|| f.clamp(0.0, 1.0))
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
/// A preset text warp (`a:prstTxWarp`, ECMA-376 §20.1.9.19) and its adjust values
/// (`a:avLst/a:gd` with `fmla="val N"`). Unknown presets are dropped.
fn text_warp(e: &El) -> Option<TextWarp> {
    let mut w = TextWarp::new(e.attr("prst")?)?;
    for gd in e.child("a:avLst").into_iter().flat_map(|l| l.children("a:gd")) {
        let (Some(name), Some(f)) = (gd.attr("name"), gd.attr("fmla")) else { continue };
        if let Some(v) = f.trim().strip_prefix("val").and_then(|v| v.trim().parse::<i64>().ok()) {
            w.set_adj(name, v);
        }
    }
    Some(w)
}

/// A preset geometry's adjust value `name` (`a:avLst/a:gd` with `fmla="val N"`).
fn preset_adj(geom: &El, name: &str) -> Option<i64> {
    let gd = geom.child("a:avLst")?.children("a:gd").find(|g| g.attr("name") == Some(name))?;
    gd.attr("fmla")?.trim().strip_prefix("val")?.trim().parse::<i64>().ok()
}

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
    let Some(x) = sppr.and_then(|s| if matches!(s.name.as_str(), "a:xfrm" | "w14:xfrm") { Some(s) } else { s.child("a:xfrm") }) else {
        return wordcraft_geom::Spin::default();
    };
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
