//! Blocks, paragraphs, runs, objects and tables → WordprocessingML.

use wordcraft_doc::para::{Anchor, Float, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::section::{LineNumberRestart, SectionProps, SectionStart};
use wordcraft_doc::table::Table;
use wordcraft_doc::{Block, Blocks, InlineObject, Paragraph, RevisionKind};

use super::props::{borders, ppr_inner, rpr, rpr_inner, tblpr, tcpr, trpr};
use super::{PartRels, Writer};
use crate::package::rt;
use crate::units::{emu, n, twips};
use crate::xml::W;

const MAX_DEPTH: usize = 24;

enum Piece<'a> {
    Text(String),
    Tab,
    Br(Option<&'static str>),
    NbHyphen,
    SoftHyphen,
    Obj(&'a InlineObject),
}

impl Writer<'_> {
    /// Write a block list. `top` = body level (sections allowed).
    pub fn blocks(&mut self, w: &mut W, bl: &Blocks, rels: &mut PartRels, top: bool, depth: usize) {
        if depth > MAX_DEPTH {
            w.raw("<w:p/>");
            return;
        }
        let toc = if top { toc_span(bl) } else { None };
        // Whether the heading's TOC field was written and still needs its end.
        let mut toc_field_open = false;
        for (i, b) in bl.iter().enumerate() {
            if let Some((start, end)) = toc {
                if i == start {
                    w.raw(TOC_SDT_OPEN);
                    self.toc_hold_end = true;
                }
                self.toc_begin_here = toc_field_open && i == start + 1;
                self.toc_end_here = toc_field_open && i == end;
            }
            match &**b {
                Block::Para(p) => self.para(w, p, rels, top, depth),
                Block::Table(t) => self.table(w, t, rels, depth),
            }
            if let Some((start, end)) = toc {
                if i == start {
                    // `field` holds the heading's TOC field back for the first entry.
                    self.toc_hold_end = false;
                    toc_field_open = self.toc_field.is_some();
                }
                if i == end {
                    w.raw("</w:sdtContent></w:sdt>");
                }
            }
        }
        if bl.last().is_none_or(|b| matches!(**b, Block::Table(_))) {
            w.raw("<w:p/>");
        }
    }

    fn para(&mut self, w: &mut W, p: &Paragraph, rels: &mut PartRels, top: bool, depth: usize) {
        // A drop cap is a separate framed paragraph holding the first character.
        if let Some(lines) = p.props.drop_cap.filter(|l| *l > 0)
            && let Some(first) = p.text.chars().next().filter(|c| !matches!(*c, wordcraft_doc::para::OBJ | '\t' | '\n' | '\u{c}' | '\u{e}'))
            && p.text.len() > first.len_utf8()
        {
            let mut head = p.clone();
            let tail = head.split_off(first.len_utf8()).ok();
            if let Some(mut tail) = tail {
                head.props.drop_cap = Some(lines);
                head.section = None;
                self.para_inner(w, &head, rels, false, depth, true);
                tail.props.drop_cap = None;
                tail.section = p.section.clone();
                self.para_inner(w, &tail, rels, top, depth, false);
                return;
            }
        }
        self.para_inner(w, p, rels, top, depth, false);
    }

    fn para_inner(&mut self, w: &mut W, p: &Paragraph, rels: &mut PartRels, top: bool, depth: usize, framed: bool) {
        let para_id = self.take_para_id(p);
        match &para_id {
            Some(id) => w.open("w:p", &[("w14:paraId", id), ("w14:textId", "77777777")]),
            None => w.open("w:p", &[]),
        }
        let mut pp = p.props.clone();
        if !framed {
            pp.drop_cap = None;
        }
        let section = if top { p.section.as_deref() } else { None };
        let has_mark = super::props::has_rpr(&p.mark);
        if !pp.is_empty() || has_mark || section.is_some() {
            w.open("w:pPr", &[]);
            ppr_inner(w, &pp, framed);
            if has_mark {
                w.open("w:rPr", &[]);
                rpr_inner(w, &p.mark);
                w.close("w:rPr");
            }
            if let Some(s) = section {
                self.sectpr(w, s, rels);
            }
            w.close("w:pPr");
        }
        // A note's first paragraph starts with its reference mark. When the paragraph holds the
        // note's reference to itself (notes read from a file, or made by references.footnote),
        // the mark is written there instead, with that reference's formatting.
        if !self.holds_own_ref(p)
            && let Some(mark) = self.pending_mark.take()
        {
            w.raw("<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr>");
            w.empty(mark, &[]);
            w.raw("</w:r>");
        }
        // Taken before the content so a nested story (a text box in an entry) can't claim them.
        if std::mem::take(&mut self.toc_begin_here)
            && let Some((instr, locked, props)) = self.toc_field.take()
        {
            self.rev_open(w, &props);
            self.field_start(w, &instr, locked, &props);
            self.rev_close(w, &props);
        }
        let toc_end = std::mem::take(&mut self.toc_end_here);
        self.para_content(w, p, rels, depth);
        if toc_end {
            w.raw(r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#);
        }
        w.close("w:p");
    }

    fn para_content(&mut self, w: &mut W, p: &Paragraph, rels: &mut PartRels, depth: usize) {
        // Split into pieces with their props.
        let mut pieces: Vec<(Piece, &CharProps)> = Vec::new();
        let mut k = 0usize;
        for (range, props) in p.run_ranges() {
            let Some(text) = p.text.get(range) else { continue };
            let mut buf = String::new();
            for c in text.chars() {
                let special = match c {
                    '\t' => Some(Piece::Tab),
                    '\n' => Some(Piece::Br(None)),
                    '\u{000C}' => Some(Piece::Br(Some("page"))),
                    '\u{000E}' => Some(Piece::Br(Some("column"))),
                    '\u{2011}' => Some(Piece::NbHyphen),
                    '\u{00AD}' => Some(Piece::SoftHyphen),
                    wordcraft_doc::para::OBJ => {
                        let o = p.objects.get(k);
                        k += 1;
                        Some(match o {
                            Some(o) => Piece::Obj(o),
                            None => Piece::Text(String::new()),
                        })
                    }
                    _ => None,
                };
                match special {
                    Some(sp) => {
                        if !buf.is_empty() {
                            pieces.push((Piece::Text(std::mem::take(&mut buf)), props));
                        }
                        pieces.push((sp, props));
                    }
                    None => buf.push(c),
                }
            }
            if !buf.is_empty() {
                pieces.push((Piece::Text(buf), props));
            }
        }

        let mut open_link: Option<String> = None;
        let mut i = 0;
        while i < pieces.len() {
            let Some((piece, props)) = pieces.get(i) else { break };
            let props: &CharProps = props;
            // Hyperlink grouping.
            if props.link != open_link {
                if open_link.is_some() {
                    w.close("w:hyperlink");
                }
                open_link = None;
                if let Some(l) = props.link.as_deref().filter(|l| !l.is_empty()) {
                    if let Some(anchor) = l.strip_prefix('#') {
                        w.open("w:hyperlink", &[("w:anchor", anchor), ("w:history", "1")]);
                    } else {
                        let id = rels.add(rt::HYPERLINK, l, true);
                        w.open("w:hyperlink", &[("r:id", &id), ("w:history", "1")]);
                    }
                    open_link = Some(l.to_string());
                }
            }
            match piece {
                Piece::Obj(o) => {
                    self.object(w, o, props, rels, depth);
                    i += 1;
                }
                _ => {
                    // Gather simple pieces sharing these props into one run.
                    let mut j = i;
                    while let Some((pc, pr)) = pieces.get(j) {
                        if matches!(pc, Piece::Obj(_)) || !(std::ptr::eq(*pr, props) || *pr == props) {
                            break;
                        }
                        j += 1;
                    }
                    let del = self.is_del(props);
                    self.rev_open(w, props);
                    w.open("w:r", &[]);
                    rpr(w, props);
                    for (pc, _) in pieces.get(i..j).unwrap_or(&[]) {
                        match pc {
                            Piece::Text(t) if !t.is_empty() => w.leaf(if del { "w:delText" } else { "w:t" }, &[("xml:space", "preserve")], t),
                            Piece::Tab => w.empty("w:tab", &[]),
                            Piece::Br(None) => w.empty("w:br", &[]),
                            Piece::Br(Some(t)) => w.empty("w:br", &[("w:type", t)]),
                            Piece::NbHyphen => w.empty("w:noBreakHyphen", &[]),
                            Piece::SoftHyphen => w.empty("w:softHyphen", &[]),
                            _ => {}
                        }
                    }
                    w.close("w:r");
                    self.rev_close(w, props);
                    i = j.max(i + 1);
                }
            }
        }
        if open_link.is_some() {
            w.close("w:hyperlink");
        }
    }

    fn rev_kind(&self, idx: Option<u32>) -> Option<RevisionKind> {
        idx.and_then(|i| self.doc.revisions.get(i as usize)).map(|r| r.kind)
    }

    fn is_del(&self, props: &CharProps) -> bool {
        props.del.is_some() && self.rev_kind(props.del) != Some(RevisionKind::Format)
    }

    fn rev_attrs(&mut self, idx: u32) -> (String, String, String) {
        self.next_rev_id += 1;
        let r = self.doc.revisions.get(idx as usize);
        let author = r.map(|r| r.author.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| "Unknown".into());
        let date = r.map(|r| r.date.clone()).unwrap_or_default();
        (self.next_rev_id.to_string(), author, date)
    }

    fn rev_open(&mut self, w: &mut W, props: &CharProps) {
        for (idx, tag) in [(props.ins, "w:ins"), (props.del, "w:del")] {
            let Some(idx) = idx else { continue };
            if self.rev_kind(Some(idx)) == Some(RevisionKind::Format) {
                continue;
            }
            let (id, author, date) = self.rev_attrs(idx);
            if date.is_empty() {
                w.open(tag, &[("w:id", &id), ("w:author", &author)]);
            } else {
                w.open(tag, &[("w:id", &id), ("w:author", &author), ("w:date", &date)]);
            }
        }
    }

    fn rev_close(&mut self, w: &mut W, props: &CharProps) {
        for (idx, tag) in [(props.del, "w:del"), (props.ins, "w:ins")] {
            if idx.is_some() && self.rev_kind(idx) != Some(RevisionKind::Format) {
                w.close(tag);
            }
        }
    }

    fn object(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, depth: usize) {
        match o {
            InlineObject::BookmarkStart { name } => {
                let id = self.bookmark_id(name);
                w.empty("w:bookmarkStart", &[("w:id", &id), ("w:name", name)]);
            }
            InlineObject::BookmarkEnd { name } => {
                let id = self.bookmark_id(name);
                w.empty("w:bookmarkEnd", &[("w:id", &id)]);
            }
            InlineObject::CommentStart { id } => {
                if self.doc.comments.contains_key(id) {
                    w.empty("w:commentRangeStart", &[("w:id", &id.to_string())]);
                }
            }
            InlineObject::CommentEnd { id } => {
                if self.doc.comments.contains_key(id) {
                    let s = id.to_string();
                    w.empty("w:commentRangeEnd", &[("w:id", &s)]);
                    w.open("w:r", &[]);
                    w.open("w:rPr", &[]);
                    w.val("w:rStyle", "CommentReference");
                    w.close("w:rPr");
                    w.empty("w:commentReference", &[("w:id", &s)]);
                    w.close("w:r");
                }
            }
            InlineObject::Equation { linear, display } => {
                if *display {
                    w.open("m:oMathPara", &[]);
                }
                w.open("m:oMath", &[]);
                w.open("m:r", &[]);
                w.leaf("m:t", &[("xml:space", "preserve")], linear);
                w.close("m:r");
                w.close("m:oMath");
                if *display {
                    w.close("m:oMathPara");
                }
            }
            InlineObject::Field { instr, result, locked } => self.field(w, instr, result, *locked, props),
            InlineObject::NoteRef { kind, id, custom } => {
                let foot = *kind == NoteKind::Footnote;
                if self.current_note == Some((foot, *id)) {
                    // The note's own number is the w:footnoteRef / w:endnoteRef mark, written once
                    // per note: here for its first reference to itself, never as a reference.
                    if let Some(mark) = self.pending_mark.take() {
                        let mut p = props.clone();
                        if p.style.is_none() && p.vert_align.is_none() {
                            p.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript);
                        }
                        self.rev_open(w, props);
                        w.open("w:r", &[]);
                        rpr(w, &p);
                        w.empty(mark, &[]);
                        w.close("w:r");
                        self.rev_close(w, props);
                    }
                    return;
                }
                let nid = self.note_id(*id, foot);
                let mut p = props.clone();
                if p.style.is_none() && p.vert_align.is_none() {
                    p.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript);
                }
                self.rev_open(w, props);
                w.open("w:r", &[]);
                rpr(w, &p);
                let tag = if foot { "w:footnoteReference" } else { "w:endnoteReference" };
                if custom.is_empty() {
                    w.empty(tag, &[("w:id", &nid)]);
                } else {
                    w.empty(tag, &[("w:customMarkFollows", "1"), ("w:id", &nid)]);
                    w.leaf("w:t", &[("xml:space", "preserve")], custom);
                }
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Image { media, w: iw, h: ih, alt, float, crop } => {
                let Some(file) = self.media_files.get(media).cloned() else { return };
                let rid = rels.add(rt::IMAGE, &format!("media/{file}"), false);
                self.used_media.insert(media.clone());
                self.rev_open(w, props);
                w.open("w:r", &[]);
                rpr(w, props);
                w.open("w:drawing", &[]);
                let docpr = self.next_docpr();
                let name = format!("Picture {docpr}");
                self.drawing_open(w, float, *iw, *ih, &docpr, &name, alt);
                w.empty("wp:cNvGraphicFramePr", &[]);
                w.open("a:graphic", &[]);
                w.open("a:graphicData", &[("uri", "http://schemas.openxmlformats.org/drawingml/2006/picture")]);
                w.open("pic:pic", &[]);
                w.open("pic:nvPicPr", &[]);
                w.empty("pic:cNvPr", &[("id", "0"), ("name", &file), ("descr", alt)]);
                w.open("pic:cNvPicPr", &[]);
                w.empty("a:picLocks", &[("noChangeAspect", "1"), ("noChangeArrowheads", "1")]);
                w.close("pic:cNvPicPr");
                w.close("pic:nvPicPr");
                w.open("pic:blipFill", &[]);
                w.empty("a:blip", &[("r:embed", &rid)]);
                if crop.iter().any(|c| *c > 0.0) {
                    let c = |v: f32| n((wordcraft_geom::finite(v).clamp(0.0, 1.0) * 100_000.0).round() as i64);
                    w.empty("a:srcRect", &[("l", &c(crop[0])), ("t", &c(crop[1])), ("r", &c(crop[2])), ("b", &c(crop[3]))]);
                }
                w.open("a:stretch", &[]);
                w.empty("a:fillRect", &[]);
                w.close("a:stretch");
                w.close("pic:blipFill");
                w.open("pic:spPr", &[("bwMode", "auto")]);
                xfrm(w, *iw, *ih);
                w.open("a:prstGeom", &[("prst", "rect")]);
                w.empty("a:avLst", &[]);
                w.close("a:prstGeom");
                w.close("pic:spPr");
                w.close("pic:pic");
                w.close("a:graphicData");
                w.close("a:graphic");
                w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
                w.close("w:drawing");
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Shape { kind, w: sw, h: sh, fill, stroke, stroke_width, float, story } => {
                self.rev_open(w, props);
                w.open("w:r", &[]);
                rpr(w, props);
                w.open("w:drawing", &[]);
                let docpr = self.next_docpr();
                let name = format!("Shape {docpr}");
                self.drawing_open(w, float, *sw, *sh, &docpr, &name, "");
                w.empty("wp:cNvGraphicFramePr", &[]);
                w.open("a:graphic", &[]);
                w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingShape")]);
                w.open("wps:wsp", &[]);
                if *kind == ShapeKind::TextBox {
                    w.empty("wps:cNvSpPr", &[("txBox", "1")]);
                } else {
                    w.empty("wps:cNvSpPr", &[]);
                }
                w.open("wps:spPr", &[]);
                xfrm(w, *sw, *sh);
                let prst = match kind {
                    ShapeKind::Rectangle | ShapeKind::TextBox => "rect",
                    ShapeKind::RoundedRectangle => "roundRect",
                    ShapeKind::Ellipse => "ellipse",
                    ShapeKind::Triangle => "triangle",
                    ShapeKind::Diamond => "diamond",
                    ShapeKind::Line => "line",
                    ShapeKind::Arrow => "rightArrow",
                    ShapeKind::Star => "star5",
                    ShapeKind::Heart => "heart",
                };
                w.open("a:prstGeom", &[("prst", prst)]);
                w.empty("a:avLst", &[]);
                w.close("a:prstGeom");
                match fill {
                    Some(c) => {
                        w.open("a:solidFill", &[]);
                        w.empty("a:srgbClr", &[("val", &c.hex())]);
                        w.close("a:solidFill");
                    }
                    None => w.empty("a:noFill", &[]),
                }
                match stroke {
                    Some(c) => {
                        w.open("a:ln", &[("w", &emu(stroke_width.clamp(0.0, 100.0)))]);
                        w.open("a:solidFill", &[]);
                        w.empty("a:srgbClr", &[("val", &c.hex())]);
                        w.close("a:solidFill");
                        w.close("a:ln");
                    }
                    None => {
                        w.open("a:ln", &[]);
                        w.empty("a:noFill", &[]);
                        w.close("a:ln");
                    }
                }
                w.close("wps:spPr");
                if let Some(part) = story.and_then(|s| self.doc.parts.get(&s))
                    && depth < 4
                {
                    w.open("wps:txbx", &[]);
                    w.open("w:txbxContent", &[]);
                    let blocks = part.blocks.clone();
                    self.blocks(w, &blocks, rels, false, depth + 1);
                    w.close("w:txbxContent");
                    w.close("wps:txbx");
                }
                w.empty("wps:bodyPr", &[]);
                w.close("wps:wsp");
                w.close("a:graphicData");
                w.close("a:graphic");
                w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
                w.close("w:drawing");
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Opaque { format, xml, text } => {
                if format == "docx" && crate::xml::parse(xml.as_bytes()).is_ok() {
                    w.raw(xml);
                } else if !text.is_empty() {
                    self.rev_open(w, props);
                    w.open("w:r", &[]);
                    rpr(w, props);
                    w.leaf("w:t", &[("xml:space", "preserve")], text);
                    w.close("w:r");
                    self.rev_close(w, props);
                }
            }
        }
    }

    fn field(&mut self, w: &mut W, instr: &str, result: &str, locked: bool, props: &CharProps) {
        if self.toc_hold_end && is_toc(instr) {
            // A TOC heading's field opens in the first entry and closes after the last (see `toc_span`).
            self.toc_hold_end = false;
            self.toc_field = Some((instr.to_string(), locked, props.clone()));
            return;
        }
        let del = self.is_del(props);
        let run = |w: &mut W, f: &dyn Fn(&mut W)| {
            w.open("w:r", &[]);
            rpr(w, props);
            f(w);
            w.close("w:r");
        };
        self.rev_open(w, props);
        self.field_start(w, instr, locked, props);
        if !result.is_empty() {
            run(w, &|w| {
                let mut buf = String::new();
                let flush = |w: &mut W, buf: &mut String| {
                    if !buf.is_empty() {
                        w.leaf(if del { "w:delText" } else { "w:t" }, &[("xml:space", "preserve")], buf);
                        buf.clear();
                    }
                };
                for c in result.chars() {
                    match c {
                        '\t' => {
                            flush(w, &mut buf);
                            w.empty("w:tab", &[]);
                        }
                        '\n' | '\r' => {
                            flush(w, &mut buf);
                            w.empty("w:br", &[]);
                        }
                        c if (c as u32) < 0x20 || c == wordcraft_doc::para::OBJ => {}
                        c => buf.push(c),
                    }
                }
                flush(w, &mut buf);
            });
        }
        run(w, &|w| w.empty("w:fldChar", &[("w:fldCharType", "end")]));
        self.rev_close(w, props);
    }

    /// A field's begin, instruction and separate runs.
    fn field_start(&self, w: &mut W, instr: &str, locked: bool, props: &CharProps) {
        let del = self.is_del(props);
        let run = |w: &mut W, f: &dyn Fn(&mut W)| {
            w.open("w:r", &[]);
            rpr(w, props);
            f(w);
            w.close("w:r");
        };
        run(w, &|w| {
            if locked {
                w.empty("w:fldChar", &[("w:fldCharType", "begin"), ("w:fldLock", "1")])
            } else {
                w.empty("w:fldChar", &[("w:fldCharType", "begin")])
            }
        });
        let instr_text = format!(" {} ", instr.trim());
        run(w, &|w| w.leaf(if del { "w:delInstrText" } else { "w:instrText" }, &[("xml:space", "preserve")], &instr_text));
        run(w, &|w| w.empty("w:fldChar", &[("w:fldCharType", "separate")]));
    }

    #[allow(clippy::too_many_arguments)]
    fn drawing_open(&mut self, w: &mut W, float: &Float, iw: f32, ih: f32, docpr: &str, name: &str, alt: &str) {
        let (cx, cy) = (emu(iw.max(0.0)), emu(ih.max(0.0)));
        if float.wrap == Wrap::Inline {
            w.open("wp:inline", &[("distT", "0"), ("distB", "0"), ("distL", "0"), ("distR", "0")]);
        } else {
            let d = emu(float.dist.clamp(0.0, 1584.0));
            self.z += 1;
            let behind = if float.wrap == Wrap::BehindText { "1" } else { "0" };
            w.open(
                "wp:anchor",
                &[
                    ("distT", &d),
                    ("distB", &d),
                    ("distL", &d),
                    ("distR", &d),
                    ("simplePos", "0"),
                    ("relativeHeight", &self.z.to_string()),
                    ("behindDoc", behind),
                    ("locked", "0"),
                    ("layoutInCell", "1"),
                    ("allowOverlap", "1"),
                ],
            );
            w.empty("wp:simplePos", &[("x", "0"), ("y", "0")]);
            let rel = |a: Anchor, horiz: bool| match a {
                Anchor::Column => {
                    if horiz {
                        "column"
                    } else {
                        "paragraph"
                    }
                }
                Anchor::Margin => "margin",
                Anchor::Page => "page",
                Anchor::Paragraph => {
                    if horiz {
                        "column"
                    } else {
                        "paragraph"
                    }
                }
            };
            w.open("wp:positionH", &[("relativeFrom", rel(float.h_rel, true))]);
            w.leaf("wp:posOffset", &[], &emu(float.x));
            w.close("wp:positionH");
            w.open("wp:positionV", &[("relativeFrom", rel(float.v_rel, false))]);
            w.leaf("wp:posOffset", &[], &emu(float.y));
            w.close("wp:positionV");
        }
        w.empty("wp:extent", &[("cx", &cx), ("cy", &cy)]);
        w.empty("wp:effectExtent", &[("l", "0"), ("t", "0"), ("r", "0"), ("b", "0")]);
        match float.wrap {
            Wrap::Inline => {}
            Wrap::Square => w.empty("wp:wrapSquare", &[("wrapText", "bothSides")]),
            Wrap::Tight | Wrap::Through => {
                let tag = if float.wrap == Wrap::Tight { "wp:wrapTight" } else { "wp:wrapThrough" };
                w.open(tag, &[("wrapText", "bothSides")]);
                w.open("wp:wrapPolygon", &[("edited", "0")]);
                w.empty("wp:start", &[("x", "0"), ("y", "0")]);
                for (x, y) in [("0", "21600"), ("21600", "21600"), ("21600", "0"), ("0", "0")] {
                    w.empty("wp:lineTo", &[("x", x), ("y", y)]);
                }
                w.close("wp:wrapPolygon");
                w.close(tag);
            }
            Wrap::TopAndBottom => w.empty("wp:wrapTopAndBottom", &[]),
            Wrap::BehindText | Wrap::InFrontOfText => w.empty("wp:wrapNone", &[]),
        }
        w.empty("wp:docPr", &[("id", docpr), ("name", name), ("descr", alt)]);
    }

    fn table(&mut self, w: &mut W, t: &Table, rels: &mut PartRels, depth: usize) {
        let rows: Vec<_> = t.rows.iter().filter(|r| !r.cells.is_empty()).collect();
        if rows.is_empty() {
            return;
        }
        w.open("w:tbl", &[]);
        tblpr(w, &t.props);
        w.open("w:tblGrid", &[]);
        let cols = t.cols().clamp(1, 63);
        for g in 0..cols {
            let gw = t.grid.get(g).copied().unwrap_or(72.0);
            w.empty("w:gridCol", &[("w:w", &twips(gw.max(0.0)))]);
        }
        w.close("w:tblGrid");
        for r in rows {
            w.open("w:tr", &[]);
            trpr(w, &r.props);
            for c in &r.cells {
                w.open("w:tc", &[]);
                tcpr(w, &c.props);
                self.blocks(w, &c.blocks, rels, false, depth + 1);
                w.close("w:tc");
            }
            w.close("w:tr");
        }
        w.close("w:tbl");
    }

    pub fn sectpr(&mut self, w: &mut W, s: &SectionProps, rels: &mut PartRels) {
        w.open("w:sectPr", &[]);
        for (set, footer) in [(&s.headers, false), (&s.footers, true)] {
            for (kind, id) in [("default", set.default), ("first", set.first), ("even", set.even)] {
                let Some(id) = id else { continue };
                if let Some(rid) = self.hf_rel(id, footer, rels) {
                    w.empty(if footer { "w:footerReference" } else { "w:headerReference" }, &[("w:type", kind), ("r:id", &rid)]);
                }
            }
        }
        let start = match s.start {
            SectionStart::NextPage => "nextPage",
            SectionStart::Continuous => "continuous",
            SectionStart::EvenPage => "evenPage",
            SectionStart::OddPage => "oddPage",
            SectionStart::NextColumn => "nextColumn",
        };
        w.val("w:type", start);
        let (pw, ph) = (s.page_w.clamp(36.0, crate::units::MAX_LEN_PT), s.page_h.clamp(36.0, crate::units::MAX_LEN_PT));
        if s.landscape {
            w.empty("w:pgSz", &[("w:w", &twips(pw)), ("w:h", &twips(ph)), ("w:orient", "landscape")]);
        } else {
            w.empty("w:pgSz", &[("w:w", &twips(pw)), ("w:h", &twips(ph))]);
        }
        w.empty(
            "w:pgMar",
            &[
                ("w:top", &twips(s.margin_top)),
                ("w:right", &twips(s.margin_right.max(0.0))),
                ("w:bottom", &twips(s.margin_bottom)),
                ("w:left", &twips(s.margin_left.max(0.0))),
                ("w:header", &twips(s.header.max(0.0))),
                ("w:footer", &twips(s.footer.max(0.0))),
                ("w:gutter", &twips(s.gutter.max(0.0))),
            ],
        );
        if let Some(b) = &s.page_borders {
            borders(w, "w:pgBorders", b, None, &[("w:offsetFrom", "text")]);
        }
        if let Some(l) = &s.line_numbers {
            let restart = match l.restart {
                LineNumberRestart::Page => "newPage",
                LineNumberRestart::Section => "newSection",
                LineNumberRestart::Continuous => "continuous",
            };
            let mut a = vec![
                ("w:countBy", n(l.count_by.clamp(1, 100) as i64)),
                ("w:start", n(l.start.saturating_sub(1).min(32_767) as i64)),
                ("w:restart", restart.to_string()),
            ];
            if l.distance > 0.0 {
                a.push(("w:distance", twips(l.distance)));
            }
            let refs: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.empty("w:lnNumType", &refs);
        }
        if s.page_num_start.is_some() || s.page_num_format != wordcraft_doc::section::NumFormat::Decimal {
            let mut a = vec![("w:fmt", s.page_num_format.ooxml().to_string())];
            if let Some(st) = s.page_num_start {
                a.push(("w:start", st.to_string()));
            }
            let refs: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.empty("w:pgNumType", &refs);
        }
        let c = &s.columns;
        let count = c.count.clamp(1, 45);
        let mut ca = vec![("w:space", twips(c.space.max(0.0))), ("w:num", n(count as i64))];
        if c.separator {
            ca.push(("w:sep", "1".into()));
        }
        let unequal = !c.widths.is_empty() && c.widths.len() == count as usize;
        if unequal {
            ca.push(("w:equalWidth", "0".into()));
            let refs: Vec<(&str, &str)> = ca.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.open("w:cols", &refs);
            for (cw, sp) in &c.widths {
                w.empty("w:col", &[("w:w", &twips(cw.max(0.0))), ("w:space", &twips(sp.max(0.0)))]);
            }
            w.close("w:cols");
        } else {
            let refs: Vec<(&str, &str)> = ca.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.empty("w:cols", &refs);
        }
        match s.valign {
            wordcraft_doc::props::VAlign::Top => {}
            wordcraft_doc::props::VAlign::Center => w.val("w:vAlign", "center"),
            wordcraft_doc::props::VAlign::Bottom => w.val("w:vAlign", "bottom"),
        }
        if s.title_page {
            w.empty("w:titlePg", &[]);
        }
        if s.rtl {
            w.empty("w:bidi", &[]);
        }
        w.close("w:sectPr");
    }
}

fn xfrm(w: &mut W, cw: f32, ch: f32) {
    w.open("a:xfrm", &[]);
    w.empty("a:off", &[("x", "0"), ("y", "0")]);
    w.empty("a:ext", &[("cx", &emu(cw.max(0.0))), ("cy", &emu(ch.max(0.0)))]);
    w.close("a:xfrm");
}

/// Word's Table of Contents content control, which gives the TOC its frame and Update Table.
const TOC_SDT_OPEN: &str =
    r#"<w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/><w:docPartUnique/></w:docPartObj></w:sdtPr><w:sdtContent>"#;

fn is_toc(instr: &str) -> bool {
    instr.trim_start().get(..3).is_some_and(|k| k.eq_ignore_ascii_case("TOC"))
}

/// The engine's TOC: a paragraph holding an empty `TOC` field, then its TOC-styled entries.
/// Returns (heading, last entry) so the field can be written spanning the entries, as Word does.
fn toc_span(bl: &Blocks) -> Option<(usize, usize)> {
    let start = bl.iter().position(|b| {
        b.as_para().is_some_and(|p| {
            p.section.is_none()
                && p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, result, .. } if is_toc(instr) && result.is_empty()))
        })
    })?;
    let entries = bl[start + 1..]
        .iter()
        .take_while(|b| {
            b.as_para().is_some_and(|p| p.section.is_none() && p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
        })
        .count();
    (entries > 0).then_some((start, start + entries))
}
