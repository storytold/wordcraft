use super::types::{Cell, FBlock, FTable, Flow, Fmt, FmtChange, Img, Inline, Kind, ListInfo, MAX_DEPTH, Meta, Para, RevMark, Spacing};
use super::{
    Arc, Block, Blocks, CharProps, Document, InlineObject, NumFormat, OBJ, PAGE_BREAK, ParaProps, Paragraph, StyleKind, Table, TextColor, Underline,
    VMerge, VertAlign,
};

pub(super) fn uses_kind(blocks: &[FBlock], k: Kind, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && blocks.iter().any(|b| match b {
            FBlock::Para(p) => p.kind == k,
            FBlock::Table(t) => t.rows.iter().flatten().any(|c| uses_kind(&c.blocks, k, depth + 1)),
        })
}

// ---------------------------------------------------------------------------------------------
// Document → Flow

/// Is this a monospace family name?
pub fn is_mono(font: &str) -> bool {
    let f = font.to_ascii_lowercase();
    f.contains("courier") || f.contains("mono") || f.contains("consolas") || f.contains("menlo") || f.contains("code") || f == "monaco"
}

fn is_code_style(doc: &Document, id: &str) -> bool {
    let names = |s: &str| {
        let l = s.to_ascii_lowercase().replace([' ', '-', '_'], "");
        l == "code" || l.contains("preformatted") || l == "plaintext" || l == "codeblock" || l == "sourcecode"
    };
    doc.styles.chain(id).iter().any(|s| names(&s.id) || names(&s.name)) || names(id)
}

/// Character formatting from a run's character style chain plus direct formatting (the
/// paragraph style is left out: it is expressed by the paragraph kind).
fn effective(doc: &Document, cp: &CharProps) -> CharProps {
    let mut out = CharProps::default();
    if let Some(cs) = cp.style.as_deref() {
        for s in doc.styles.chain(cs) {
            if s.kind == StyleKind::Character {
                out.overlay(&s.chr);
            }
        }
    }
    let mut direct = cp.clone();
    direct.style = None;
    out.overlay(&direct);
    out
}

fn fmt_of(doc: &Document, cp: &CharProps, in_code: bool) -> Fmt {
    let e = effective(doc, cp);
    let code = !in_code && e.font.as_deref().is_some_and(is_mono);
    Fmt {
        bold: e.bold.unwrap_or(false),
        italic: e.italic.unwrap_or(false),
        underline: e.underline.is_some_and(|u| u != Underline::None) && e.link.is_none(),
        strike: e.strike.unwrap_or(false) || e.double_strike.unwrap_or(false),
        code,
        sup: e.vert_align == Some(VertAlign::Superscript),
        sub: e.vert_align == Some(VertAlign::Subscript),
        link: e.link.as_deref().map(Arc::from),
        color: match e.color {
            Some(TextColor::Rgb(c)) => Some(c),
            _ => None,
        },
        size: e.size,
        font: if code || in_code { None } else { e.shown_font().map(Arc::from) },
        background: e.highlight.and_then(|h| h.rgb()).or(e.shading),
        ins: None,
        del: None,
        fmt_change: None,
    }
}

/// Revision `i` as a flow mark (unknown author and date when there is none).
fn rev_mark(doc: &Document, i: u32) -> RevMark {
    doc.revisions.get(i as usize).map(|r| RevMark { author: r.author.clone(), date: r.date.clone(), id: Some(i) }).unwrap_or_default()
}

/// The insertion and deletion `cp` tracks, as flow marks.
fn rev_marks(doc: &Document, cp: &CharProps) -> (Option<RevMark>, Option<RevMark>) {
    (cp.ins.map(|i| rev_mark(doc, i)), cp.del.map(|i| rev_mark(doc, i)))
}

/// Paragraph kind and list membership of a document paragraph.
pub fn para_kind(doc: &Document, p: &Paragraph) -> (Kind, Option<ListInfo>) {
    let blank = p.plain_text().trim().is_empty() && p.objects.iter().all(|o| o.is_marker());
    props_kind(doc, &p.props, blank)
}

/// Paragraph kind and list membership of paragraph properties; `blank` = the paragraph shows
/// nothing (a bottom border then makes it a rule).
fn props_kind(doc: &Document, props: &ParaProps, blank: bool) -> (Kind, Option<ListInfo>) {
    let rp = doc.styles.resolve_para(props);
    let style = props.style.as_deref().unwrap_or("Normal");
    let kind = if let Some(l) = rp.outline_level {
        Kind::Heading((l + 1).min(6))
    } else if doc.styles.chain(style).iter().any(|s| s.id == "Title") {
        Kind::Title
    } else if doc.styles.chain(style).iter().any(|s| s.id == "Quote" || s.id == "IntenseQuote") {
        Kind::Quote
    } else if is_code_style(doc, style) {
        Kind::Code
    } else if blank && rp.borders.and_then(|b| b.bottom).is_some_and(|b| b.is_visible()) {
        Kind::Rule
    } else {
        Kind::Normal
    };
    let list = rp.numbering.and_then(|n| {
        let lv = doc.numbering.level(n.num, n.level)?;
        Some(ListInfo { ordered: !matches!(lv.format, NumFormat::Bullet | NumFormat::None), level: n.level.min(8), start: lv.start })
    });
    (kind, list)
}

pub(super) fn media_ext(key: &str, data: &[u8]) -> String {
    if wordcraft_svg::is_svg(data) {
        return "svg".into();
    }
    match image::guess_format(data) {
        Ok(image::ImageFormat::Png) => "png".into(),
        Ok(image::ImageFormat::Jpeg) => "jpeg".into(),
        Ok(image::ImageFormat::Gif) => "gif".into(),
        Ok(image::ImageFormat::WebP) => "webp".into(),
        Ok(image::ImageFormat::Bmp) => "bmp".into(),
        _ => key.rsplit('.').next().filter(|e| e.len() <= 5 && *e != key).unwrap_or("png").to_ascii_lowercase(),
    }
}

/// A document paragraph as flow paragraphs: a page break character inside the text ends the
/// paragraph, and the text after it starts a new one that carries `page_break`.
pub fn flow_paras(doc: &Document, p: &Paragraph) -> Vec<Para> {
    flow_paras_in(doc, p, false)
}

/// [`flow_paras`]; `tracked` keeps tracked changes: deleted text stays, and text carries its
/// insertion and deletion marks.
fn flow_paras_in(doc: &Document, p: &Paragraph, tracked: bool) -> Vec<Para> {
    let (kind, list) = para_kind(doc, p);
    let mut done: Vec<Para> = Vec::new();
    let borders = if kind == Kind::Rule { None } else { p.props.borders.filter(|b| b.any_visible()) };
    let mut out = Para {
        kind,
        list,
        align: p.props.align,
        page_break: p.props.page_break_before.unwrap_or(false),
        keep_next: p.props.keep_next.unwrap_or(false),
        inlines: Vec::new(),
        spacing: Spacing::default(),
        borders,
        section: None,
        mark_ins: None,
        mark_del: None,
        fmt_change: None,
    };
    let in_code = kind == Kind::Code;
    let mut k = 0usize;
    for (range, props) in p.run_ranges() {
        let Some(text) = p.text.get(range.clone()) else { continue };
        // Deleted objects and page breaks stay out even when tracking: the flow has no marks for them.
        let hidden = props.hidden.unwrap_or(false) || props.del.is_some();
        let hidden_text = props.hidden.unwrap_or(false) || (props.del.is_some() && !tracked);
        let mut f = fmt_of(doc, props, in_code);
        if tracked {
            (f.ins, f.del) = rev_marks(doc, props);
            f.fmt_change = props.fmt_change.as_deref().map(|c| Box::new(FmtChange { mark: rev_mark(doc, c.rev), old: fmt_of(doc, &c.old, in_code) }));
        }
        let mut buf = String::new();
        for c in text.chars() {
            if c == PAGE_BREAK {
                if !hidden {
                    out.push_text(&std::mem::take(&mut buf), &f);
                    let next = Para {
                        kind,
                        list: None,
                        align: p.props.align,
                        page_break: true,
                        keep_next: p.props.keep_next.unwrap_or(false),
                        inlines: Vec::new(),
                        spacing: Spacing::default(),
                        borders,
                        section: None,
                        mark_ins: None,
                        mark_del: None,
                        fmt_change: None,
                    };
                    let prev = std::mem::replace(&mut out, next);
                    if prev.inlines.is_empty() {
                        out.page_break |= prev.page_break;
                        out.list = prev.list;
                    } else {
                        done.push(prev);
                    }
                }
                continue;
            }
            if c != OBJ {
                buf.push(c);
                continue;
            }
            let obj = p.objects.get(k);
            k += 1;
            // Hidden text, or a hidden drawing: not in the copy.
            if hidden || obj.and_then(InlineObject::frame).is_some_and(|(_, _, f)| f.hidden) {
                continue;
            }
            match obj {
                Some(o @ InlineObject::Image { media, w, h, alt, .. }) => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    // A picture marked decorative keeps its description in the document, but
                    // exports with none (`alt=""`), so screen readers skip it.
                    let alt = if o.is_decorative() { "" } else { alt.as_str() };
                    match doc.media.get(media) {
                        Some(data) => out.inlines.push(Inline::Image(Img {
                            data: data.clone(),
                            ext: media_ext(media, data),
                            w: *w,
                            h: *h,
                            alt: alt.to_string(),
                        })),
                        None if !alt.is_empty() => buf.push_str(alt),
                        None => {}
                    }
                }
                Some(InlineObject::Graphic { alt, .. }) => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    if !alt.trim().is_empty() {
                        out.inlines.push(Inline::Figure(alt.clone()));
                    }
                }
                Some(InlineObject::Equation { linear, display, math }) => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    // The linear format is the shared form; an equation known only by its
                    // structure gets it from there.
                    let linear = if linear.is_empty() { wordcraft_doc::math::to_linear(&math.nodes) } else { linear.clone() };
                    out.inlines.push(Inline::Equation { linear, display: *display });
                }
                Some(InlineObject::Ruby { base, ruby, .. }) => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    // The base text and reading, in the run's formatting; the guide's own
                    // attributes (alignment, size, raise) stay in the document model.
                    out.inlines.push(Inline::Ruby { base: base.clone(), ruby: ruby.clone(), fmt: f.clone() });
                }
                Some(InlineObject::BookmarkStart { name }) if name != "_GoBack" => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    out.inlines.push(Inline::Anchor(name.clone()));
                }
                Some(o) => buf.push_str(o.plain_text()),
                None => {}
            }
        }
        if !hidden_text {
            out.push_text(&buf, &f);
        }
    }
    if tracked {
        let (ins, del) = rev_marks(doc, &p.mark);
        (out.mark_ins, out.mark_del) = (ins.map(Box::new), del.map(Box::new));
        // The formatting change belongs to the paragraph the mark ends.
        out.fmt_change = p.props.fmt_change.as_deref().map(|c| {
            let blank = out.inlines.is_empty();
            let (kind, list) = props_kind(doc, &c.old, blank);
            let old = Para { kind, list, align: c.old.align, page_break: c.old.page_break_before.unwrap_or(false), ..Default::default() };
            Box::new(FmtChange { mark: rev_mark(doc, c.rev), old })
        });
    }
    // The paragraph that ends a section carries its properties (the last piece of it does).
    out.section.clone_from(&p.section);
    done.push(out);
    done
}

fn flow_blocks(doc: &Document, bl: &Blocks, depth: usize, tracked: bool) -> Vec<FBlock> {
    let mut out = Vec::new();
    for b in bl {
        match &**b {
            Block::Para(p) => out.extend(flow_paras_in(doc, p, tracked).into_iter().map(FBlock::Para)),
            Block::Table(t) if depth < MAX_DEPTH => out.push(FBlock::Table(flow_table(doc, t, depth, tracked))),
            Block::Table(t) => {
                for r in &t.rows {
                    for c in &r.cells {
                        out.extend(flow_blocks(doc, &c.blocks, depth, tracked));
                    }
                }
            }
        }
    }
    out
}

fn flow_table(doc: &Document, t: &Table, depth: usize, tracked: bool) -> FTable {
    let mut rows = Vec::new();
    for (ri, r) in t.rows.iter().enumerate() {
        let header = r.props.header || (ri == 0 && t.props.look.header_row && t.rows.len() > 1);
        let mut cells = Vec::new();
        for (ci, c) in r.cells.iter().enumerate() {
            let mut rowspan = 1;
            if c.props.vmerge == VMerge::Restart {
                let g = t.grid_col(ri, ci);
                for (k, below) in t.rows.iter().enumerate().skip(ri + 1) {
                    let cont = t.cell_at_grid(k, g).and_then(|i| below.cells.get(i)).is_some_and(|bc| bc.props.vmerge == VMerge::Continue);
                    if !cont {
                        break;
                    }
                    rowspan += 1;
                }
            }
            cells.push(Cell {
                blocks: flow_blocks(doc, &c.blocks, depth + 1, tracked),
                colspan: c.span() as u32,
                rowspan,
                covered: c.props.vmerge == VMerge::Continue,
                header,
                shading: c.props.shading,
                borders: c.props.borders,
                valign: c.props.valign,
            });
        }
        rows.push(cells);
    }
    // Without a table style and without visible table or cell borders the table shows no grid.
    let visible = |b: &Option<wordcraft_doc::props::Borders>| b.as_ref().is_some_and(|b| b.any_visible());
    let borderless =
        t.props.style.is_none() && !visible(&t.props.borders) && t.rows.iter().flat_map(|r| &r.cells).all(|c| !visible(&c.props.borders));
    FTable { rows, widths: t.grid.clone(), borderless }
}

/// The document body as a flow.
pub fn from_doc(doc: &Document) -> Flow {
    flow_of(doc, false)
}

/// [`from_doc`] keeping tracked changes: deleted text stays, and text carries its insertion and
/// deletion marks ([`Fmt::ins`], [`Fmt::del`]), for formats that can write revisions (RTF).
pub fn from_doc_tracked(doc: &Document) -> Flow {
    flow_of(doc, true)
}

fn flow_of(doc: &Document, tracked: bool) -> Flow {
    let c = &doc.core;
    Flow {
        blocks: flow_blocks(doc, &doc.body, 0, tracked),
        meta: Meta {
            title: c.title.clone(),
            author: c.creator.clone(),
            subject: c.subject.clone(),
            keywords: c.keywords.clone(),
            description: c.description.clone(),
            created: c.created.clone(),
        },
    }
}

/// List labels in document order for list paragraphs of a flow ("1.", "a.", "•"), computed by a
/// simple per-level counter (what text exports show).
#[derive(Default)]
pub struct ListCounter {
    counts: [u32; 9],
    ordered: [bool; 9],
}

impl ListCounter {
    pub fn reset(&mut self) {
        *self = ListCounter::default();
    }
    /// Advance for `li` and return its number (1-based; bullets also count).
    pub fn next(&mut self, li: ListInfo) -> u32 {
        let lv = li.level.min(8) as usize;
        if self.ordered.get(lv).copied() != Some(li.ordered)
            && let (Some(o), Some(c)) = (self.ordered.get_mut(lv), self.counts.get_mut(lv))
        {
            *o = li.ordered;
            *c = 0;
        }
        let n = match self.counts.get_mut(lv) {
            Some(c) => {
                if *c == 0 {
                    *c = li.start.saturating_sub(1);
                }
                *c = c.saturating_add(1);
                *c
            }
            None => 1,
        };
        for d in self.counts.iter_mut().skip(lv + 1) {
            *d = 0;
        }
        n
    }
    pub fn label(&mut self, li: ListInfo) -> String {
        let n = self.next(li);
        if !li.ordered {
            return ["•", "◦", "▪"].get(li.level as usize % 3).copied().unwrap_or("•").to_string();
        }
        match li.level % 3 {
            0 => format!("{n}."),
            1 => format!("{}.", wordcraft_doc::section::letters(n)),
            _ => format!("{}.", wordcraft_doc::section::roman(n)),
        }
    }
}
