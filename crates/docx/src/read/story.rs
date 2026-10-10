//! Stories: blocks, paragraphs, runs, fields, drawings, tables.

use std::sync::Arc;

use wordcraft_doc::para::{Anchor, Float, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{CharProps, Rgb};
use wordcraft_doc::table::{Cell, MAX_COLS, MAX_ROWS, Row, Table};
use wordcraft_doc::{Block, Blocks, InlineObject, Paragraph, PartKind, RevisionKind, Run, para_block};

use super::Reader;
use super::props::{sectpr, tcpr, trpr};
use crate::package::Rels;
use crate::units::{int, measure};
use crate::xml::El;

/// Deepest table nesting we build (deeper tables become plain paragraphs).
const MAX_TABLE_DEPTH: usize = 24;
/// Deepest field nesting we track.
const MAX_FIELD_DEPTH: usize = 32;
/// Deepest text-box-in-text-box nesting.
const MAX_STORY_DEPTH: usize = 4;

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
    const KNOWN: &[&str] = &["wps", "w14", "w15", "wp14", "a14", "w16se", "w16cid", "w16", "w16cex", "w16sdtdh", "v"];
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

    /// Markers left over at the end of a story go to its last paragraph.
    pub fn flush_pending(&mut self, sc: &mut StoryCtx, out: &mut Blocks) {
        if sc.pending.is_empty() {
            return;
        }
        if let Some(Block::Para(p)) = out.last_mut().map(Arc::make_mut) {
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

    pub fn read_section(&mut self, s: &El, rels: &Rels) -> wordcraft_doc::SectionProps {
        let (mut sp, refs) = sectpr(s);
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
                if let Some(f) = sc.fields.last_mut() {
                    f.phase = Phase::Result;
                }
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
                self.emit_obj(sc, pb, InlineObject::Equation { linear: k.deep_text(), display: false }, &props);
            }
            "m:oMathPara" => {
                let props = self.run_props_none(ctx);
                for m in k.children("m:oMath") {
                    self.emit_obj(sc, pb, InlineObject::Equation { linear: m.deep_text(), display: true }, &props);
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

    fn run_props(&self, rpr: &El, ctx: &RunCtx) -> CharProps {
        let mut p = self.pc.rpr(rpr);
        p.link = ctx.link.clone();
        p.ins = ctx.ins;
        p.del = ctx.del;
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
                    && f.instr.len() < 64 * 1024
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
                Some("separate") => {
                    if sc.ignored_begins == 0
                        && let Some(f) = sc.fields.last_mut()
                    {
                        f.phase = Phase::Result;
                    }
                }
                Some("end") => self.end_field(sc, pb),
                _ => {}
            },
            "w:drawing" => {
                if let Some(o) = self.read_drawing(sc, k, rels) {
                    self.emit_obj(sc, pb, o, props);
                }
            }
            "w:pict" | "w:object" => {
                if let Some(o) = self.read_vml(sc, k, rels) {
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
        sc.fields.push(FieldState { instr, phase: Phase::Instr, spilled: false, buf: Vec::new(), props, locked });
    }

    fn end_field(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        if sc.ignored_begins > 0 {
            sc.ignored_begins -= 1;
            return;
        }
        let Some(f) = sc.fields.pop() else { return };
        if f.spilled {
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
            if f.spilled {
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
        match sc.fields.last_mut() {
            Some(f) if !f.spilled => {
                if f.phase == Phase::Result && f.buf.len() < 100_000 {
                    f.buf.push((Item::Text(s.to_string()), props.clone()));
                }
            }
            _ => pb.push_text(s, props),
        }
    }

    fn emit_obj(&mut self, sc: &mut StoryCtx, pb: &mut PB, o: InlineObject, props: &CharProps) {
        match sc.fields.last_mut() {
            Some(f) if !f.spilled => {
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
        let float = if anchored { anchor_float(c) } else { Float::default() };
        let gd = c.child("a:graphic").and_then(|g| g.child("a:graphicData"))?;
        if let Some(blip) = gd.find("a:blip") {
            let media = blip.attr("r:embed").and_then(|id| self.media_for(rels, id))?;
            let mut crop = [0.0f32; 4];
            if let Some(sr) = gd.find("a:srcRect") {
                for (i, n) in ["l", "t", "r", "b"].iter().enumerate() {
                    if let Some(v) = sr.attr(n).and_then(int)
                        && let Some(slot) = crop.get_mut(i)
                    {
                        *slot = (v.clamp(0, 100_000) as f32 / 100_000.0).clamp(0.0, 1.0);
                    }
                }
            }
            return Some(InlineObject::Image { media, w, h, alt, float, crop });
        }
        if let Some(wsp) = gd.find("wps:wsp") {
            return Some(self.read_wsp(sc, wsp, rels, w, h, float));
        }
        None
    }

    fn read_wsp(&mut self, sc: &mut StoryCtx, wsp: &El, rels: &Rels, w: f32, h: f32, float: Float) -> InlineObject {
        let sppr = wsp.child("wps:spPr");
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
        let story = match txbx {
            Some(t) if sc.story_depth < MAX_STORY_DEPTH => Some(self.read_textbox(sc, t, rels)),
            _ => None,
        };
        InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, story }
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
        let mut w = 0.0;
        let mut h = 0.0;
        for decl in style.split(';') {
            let mut kv = decl.splitn(2, ':');
            let (Some(k), Some(v)) = (kv.next(), kv.next()) else { continue };
            let v = measure(v.trim(), 1.0).unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT);
            match k.trim() {
                "width" => w = v,
                "height" => h = v,
                _ => {}
            }
        }
        if let Some(img) = shape.find("v:imagedata") {
            let id = img.attr("r:id").or_else(|| img.attr("r:pict"))?;
            let media = self.media_for(rels, id)?;
            let alt = shape.attr("alt").or_else(|| img.attr("o:title")).unwrap_or("").to_string();
            return Some(InlineObject::Image { media, w, h, alt, float: Float::default(), crop: [0.0; 4] });
        }
        if let Some(t) = shape.find("w:txbxContent")
            && sc.story_depth < MAX_STORY_DEPTH
        {
            let story = Some(self.read_textbox(sc, t, rels));
            let fill = shape.attr("fillcolor").and_then(Rgb::parse);
            let stroke = shape.attr("strokecolor").and_then(Rgb::parse);
            return Some(InlineObject::Shape { kind: ShapeKind::TextBox, w, h, fill, stroke, stroke_width: 0.75, float: Float::default(), story });
        }
        None
    }

    // ---- tables ----

    fn read_table(&mut self, sc: &mut StoryCtx, t: &El, rels: &Rels, depth: usize) -> Option<Table> {
        let mut table = Table::default();
        if let Some(p) = t.child("w:tblPr") {
            table.props = self.pc.tblpr(p);
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
            let mut tcs = Vec::new();
            collect(tr, "w:tc", &mut tcs, 0);
            for tc in tcs.into_iter().take(MAX_COLS) {
                let (props, hcont) =
                    tc.child("w:tcPr").map(tcpr).unwrap_or_else(|| (wordcraft_doc::props::CellProps { span: 1, ..Default::default() }, false));
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
                let rel = match k.attr("relativeFrom").unwrap_or("") {
                    "page" => Anchor::Page,
                    "margin" | "leftMargin" | "rightMargin" | "insideMargin" | "outsideMargin" | "topMargin" | "bottomMargin" => Anchor::Margin,
                    "paragraph" | "line" => Anchor::Paragraph,
                    _ => {
                        if horiz {
                            Anchor::Column
                        } else {
                            Anchor::Paragraph
                        }
                    }
                };
                let off = k.child("wp:posOffset").and_then(|o| measure(&o.text(), 12_700.0)).unwrap_or(0.0);
                if horiz {
                    f.h_rel = rel;
                    f.x = off;
                } else {
                    f.v_rel = rel;
                    f.y = off;
                }
            }
            _ => {}
        }
    }
    f.dist = c.attr("distL").and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(0.0, 1584.0);
    f
}
