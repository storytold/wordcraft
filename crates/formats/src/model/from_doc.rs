use super::types::{
    BoxLook, Cell, FBlock, FComment, FTable, Flow, FlowOpts, Fmt, FmtChange, Img, Inline, Kind, ListInfo, MAX_DEPTH, MAX_GROUP_DEPTH, Member, Meta,
    Para, RevMark, Spacing,
};
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
        extra: None,
    }
}

/// The flow formatting of a run with properties `props`: with `opts.chars`, its extra formatting,
/// its custom character style by name (only its direct formatting then its own) and a
/// monospaced font by its name.
fn run_fmt(doc: &Document, props: &CharProps, in_code: bool, opts: FlowOpts) -> Fmt {
    let mut f = fmt_of(doc, props, in_code);
    if !opts.chars {
        return f;
    }
    let custom = props.style.as_deref().filter(|id| is_custom_char_style(doc, id));
    let direct;
    let props_now = match custom {
        Some(_) => {
            direct = CharProps { style: None, ..props.clone() };
            f = fmt_of(doc, &direct, in_code);
            &direct
        }
        None => props,
    };
    let e = effective(doc, props_now);
    f.extra = extra_of(&e);
    if let Some(id) = custom {
        f.extra.get_or_insert_with(Default::default).style = Some(id.to_string());
    }
    if f.code {
        f.code = false;
        f.font = e.shown_font().map(Arc::from);
    }
    f
}

/// The formatting in `e` (effective character properties) that only [`Fmt::extra`] holds;
/// `None` when it sets none of it.
fn extra_of(e: &CharProps) -> Option<Box<CharProps>> {
    let set = |v: Option<bool>| v.filter(|b| *b);
    let x = CharProps {
        underline: e.underline.filter(|u| !matches!(u, Underline::None | Underline::Single)),
        underline_color: e.underline_color,
        double_strike: set(e.double_strike),
        caps: set(e.caps),
        small_caps: set(e.small_caps),
        hidden: set(e.hidden),
        spacing: e.spacing.filter(|v| *v != 0.0 && v.is_finite()),
        scale: e.scale.filter(|v| *v != 100.0 && v.is_finite()),
        position: e.position.filter(|v| *v != 0.0 && v.is_finite()),
        outline: set(e.outline),
        shadow: set(e.shadow),
        emboss: set(e.emboss),
        engrave: set(e.engrave),
        lang: e.lang.clone().filter(|l| !l.is_empty()),
        font_cs: e.font_cs.clone().filter(|f| !f.is_empty()),
        font_ea: e.font_ea.clone().filter(|f| !f.is_empty()),
        size_cs: e.size_cs.filter(|s| s.is_finite()),
        bold_cs: e.bold_cs,
        italic_cs: e.italic_cs,
        lang_bidi: e.lang_bidi.clone().filter(|l| !l.is_empty()),
        lang_ea: e.lang_ea.clone().filter(|l| !l.is_empty()),
        ..Default::default()
    };
    (x != CharProps::default()).then(|| Box::new(x))
}

/// Whether character style `id` of `doc` is one of its own rather than one WordCraft gives links,
/// note and comment marks or placeholders.
pub fn is_custom_char_style(doc: &Document, id: &str) -> bool {
    let builtin =
        ["DefaultParagraphFont", "Hyperlink", "FollowedHyperlink", "FootnoteReference", "EndnoteReference", "CommentReference", "PlaceholderText"];
    !builtin.contains(&id) && doc.styles.get(id).is_some_and(|s| s.kind == wordcraft_doc::styles::StyleKind::Character)
}

/// Whether paragraph style `id` of `doc` is one of its own rather than one the flow model's
/// paragraph kinds, lists and notes stand for (Normal, the headings, Title, Quote, Code, List
/// Paragraph, the note texts).
pub fn is_custom_style(doc: &Document, id: &str) -> bool {
    // The styles the kinds stand for, and the names formats write fixed styles under.
    let kinds =
        ["Normal", "Title", "Quote", super::types::CODE_STYLE, "FootnoteText", "EndnoteText", "Standard", "Footnote", "Endnote", "Quotations"];
    let heading = id.strip_prefix("Heading").is_some_and(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_digit()));
    !heading && !kinds.contains(&id) && doc.styles.get(id).is_some_and(|s| s.kind == wordcraft_doc::styles::StyleKind::Paragraph)
}

/// The paragraph formatting only [`Para::extra`] holds that `p` sets itself; `None` when none.
fn para_extra(doc: &Document, p: &wordcraft_doc::props::ParaProps) -> Option<Box<wordcraft_doc::props::ParaProps>> {
    let tabs = p.tabs.clone().filter(|t| !t.is_empty());
    let style = p.style.clone().filter(|id| is_custom_style(doc, id));
    let x = wordcraft_doc::props::ParaProps {
        tabs,
        shading: p.shading,
        style,
        contextual_spacing: p.contextual_spacing,
        bidi: p.bidi.filter(|b| *b),
        drop_cap: p.drop_cap.filter(|l| *l > 1),
        drop_cap_distance: p.drop_cap_distance.filter(|_| p.drop_cap.is_some_and(|l| l > 1)),
        ..Default::default()
    };
    (x != wordcraft_doc::props::ParaProps::default()).then(|| Box::new(x))
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
        Some(ListInfo {
            ordered: !matches!(lv.format, NumFormat::Bullet | NumFormat::None),
            level: n.level.min(8),
            start: lv.start,
            list: n.num,
            label: Some(super::types::Label::of_level(lv.format, &lv.text).placed(lv.indent, lv.hanging, lv.align)),
        })
    });
    (kind, list)
}

fn media_ext(key: &str, data: &[u8]) -> String {
    match image::guess_format(data) {
        Ok(image::ImageFormat::Png) => "png".into(),
        Ok(image::ImageFormat::Jpeg) => "jpeg".into(),
        Ok(image::ImageFormat::Gif) => "gif".into(),
        Ok(image::ImageFormat::WebP) => "webp".into(),
        Ok(image::ImageFormat::Bmp) => "bmp".into(),
        _ => key.rsplit('.').next().filter(|e| e.len() <= 5 && *e != key).unwrap_or("png").to_ascii_lowercase(),
    }
}

/// The flow inlines for a note reference, comment mark or field `o` of `doc`, when `opts` asks
/// for them; `None` for other objects. `f` is the formatting of the run `o` is in.
fn rich_inlines(doc: &Document, o: &InlineObject, f: &Fmt, opts: FlowOpts) -> Option<Vec<Inline>> {
    Some(match o {
        InlineObject::NoteRef { kind, id, custom } if opts.notes => {
            // The note's content without its own mark (a note holds no notes).
            let story = doc.parts.get(id).map(|part| &part.blocks);
            let mut blocks = story.map(|b| flow_blocks(doc, b, 0, FlowOpts { notes: false, ..opts })).unwrap_or_default();
            if let Some(FBlock::Para(first)) = blocks.first_mut() {
                first.trim_with(&[' ', '\t'], &[]);
            }
            let endnote = *kind == wordcraft_doc::para::NoteKind::Endnote;
            vec![Inline::Note { endnote, custom: custom.clone(), blocks }]
        }
        // The comment, then the replies in its thread (replies to replies too, each answering the
        // comment that starts the thread), which have no place of their own.
        InlineObject::CommentStart { id } if opts.comments => {
            let Some(c) = doc.comments.get(id) else { return Some(Vec::new()) };
            let mut out = vec![Inline::Comment { id: *id, comment: Box::new(flow_comment(doc, c, None, opts)) }];
            let mut thread = std::collections::BTreeSet::from([*id]);
            let mut k = 0;
            while let Some(Inline::Comment { id: at, .. }) = out.get(k) {
                let at = *at;
                k += 1;
                for (rid, r) in doc.comments.iter().filter(|(_, r)| r.parent == Some(at)) {
                    if thread.insert(*rid) {
                        out.push(Inline::Comment { id: *rid, comment: Box::new(flow_comment(doc, r, Some(*id), opts)) });
                    }
                }
            }
            out
        }
        InlineObject::CommentEnd { id } if opts.comments => {
            if doc.comments.contains_key(id) {
                vec![Inline::CommentEnd(*id)]
            } else {
                Vec::new()
            }
        }
        // A text box's content, without notes, comments or boxes of its own.
        // A drawing shape, text box or group (pictures have their own place below).
        InlineObject::Shape { .. } | InlineObject::Group { .. } if opts.boxes => vec![drawing_inline(doc, o, opts, 0, (1.0, 1.0))?],
        InlineObject::FieldStart { instr, .. } if opts.fields => vec![Inline::FieldStart(instr.clone())],
        InlineObject::FieldEnd if opts.fields => vec![Inline::FieldEnd],
        // Where a bookmark ends, so it covers its text (Word's `_GoBack` is the last edit's spot).
        InlineObject::BookmarkEnd { name } if opts.fields && name != "_GoBack" => vec![Inline::AnchorEnd(name.clone())],
        InlineObject::Field { instr, result, .. } if opts.fields => {
            vec![Inline::Field { instr: instr.clone(), result: result.clone(), fmt: f.clone() }]
        }
        _ => return None,
    })
}

/// Comment `c` (a reply to `parent`) of `doc` as a flow comment: its text holds no comments or
/// notes of its own.
/// The flow inline for picture, shape or group `o` of `doc` (a member of groups `depth` deep),
/// its size scaled by `scale` (a group's members are in its own coordinates). A shape or text
/// box keeps its text, but not notes, comments or boxes of its own. `None` for what a flow has
/// no drawing for: a freeform's paths, a picture whose data is missing.
fn drawing_inline(doc: &Document, o: &InlineObject, opts: FlowOpts, depth: usize, scale: (f32, f32)) -> Option<Inline> {
    use wordcraft_doc::para::ShapeKind;
    let (sx, sy) = scale;
    match o {
        // A freeform needs its paths.
        InlineObject::Shape { kind, w, h, float, story, fill, stroke, stroke_width, extra, freeform, .. }
            if *kind != ShapeKind::Freeform || freeform.is_some() =>
        {
            // A connector is a straight line.
            let kind = match kind {
                ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => ShapeKind::Line,
                k => *k,
            };
            let inner = FlowOpts { notes: false, comments: false, boxes: false, ..opts };
            let part = story.and_then(|id| doc.parts.get(&id));
            let blocks = part.map(|p| flow_blocks(doc, &p.blocks, 0, inner)).unwrap_or_default();
            let look = BoxLook {
                fill: fill.clone().filter(|_| kind != ShapeKind::Line),
                line: stroke.filter(|_| stroke_width.is_finite() && *stroke_width > 0.0).map(|c| (c, stroke_width.min(12.0))),
                dash: extra.dash.clone(),
                arrows: (extra.arrow_start, extra.arrow_end),
                inset: part.and_then(|p| p.body.inset.is_some().then(|| p.body.insets_pt())),
                auto_fit: part.is_some_and(|p| p.body.auto_fit),
            };
            let path = freeform.clone().filter(|_| kind == ShapeKind::Freeform);
            Some(Inline::Shape { kind, w: w * sx, h: h * sy, float: kept_float(float), blocks, look, path })
        }
        InlineObject::Image { media, w, h, alt, float, ole, .. } => {
            let data = doc.media.get(media)?;
            let alt = if o.is_decorative() { "" } else { alt.as_str() };
            Some(Inline::Image(Img {
                data: data.clone(),
                ext: media_ext(media, data),
                w: w * sx,
                h: h * sy,
                alt: alt.to_string(),
                float: kept_float(float),
                object: ole.as_deref().filter(|_| opts.boxes).and_then(|e| super::odf_object::load(doc, e)),
            }))
        }
        InlineObject::Group { w, h, float, ch_w, ch_h, children, .. } if depth < MAX_GROUP_DEPTH => {
            let ratio = |outer: f32, inner: f32| {
                let r = outer / inner;
                if r.is_finite() && r > 0.0 { r } else { 1.0 }
            };
            let inner = (sx * ratio(*w, *ch_w), sy * ratio(*h, *ch_h));
            let members: Vec<Member> = children
                .iter()
                .filter_map(|c| {
                    drawing_inline(doc, &c.obj, opts, depth + 1, inner).map(|inline| Member { x: c.x * inner.0, y: c.y * inner.1, inline })
                })
                .collect();
            (!members.is_empty()).then(|| Inline::Group { w: w * sx, h: h * sy, float: kept_float(float), members })
        }
        _ => None,
    }
}

/// The placement a flow keeps for a drawing: when it floats, or turns or mirrors it in line.
fn kept_float(f: &wordcraft_doc::para::Float) -> Option<Box<wordcraft_doc::para::Float>> {
    let turned = f.rot.is_finite() && f.rot.rem_euclid(360.0) > 0.001 && f.rot.rem_euclid(360.0) < 359.999;
    (f.wrap != wordcraft_doc::para::Wrap::Inline || turned || f.flip_h || f.flip_v).then(|| Box::new(f.clone()))
}

fn flow_comment(doc: &Document, c: &wordcraft_doc::Comment, parent: Option<u32>, opts: FlowOpts) -> FComment {
    let story = doc.parts.get(&c.part).map(|p| &p.blocks);
    let blocks = story.map(|b| flow_blocks(doc, b, 0, FlowOpts { notes: false, comments: false, ..opts })).unwrap_or_default();
    FComment { author: c.author.clone(), initials: c.initials.clone(), date: c.date.clone(), resolved: c.resolved, parent, blocks }
}

/// A paragraph's list membership as the flow keeps it: without the list's identity and label
/// unless `opts` asks for them, and none for a heading numbered only through its style, which a
/// format with heading numbering of its own (ODT's outline style) numbers instead.
fn flow_list(kind: Kind, list: Option<ListInfo>, p: &Paragraph, opts: FlowOpts) -> Option<ListInfo> {
    let mut li = list?;
    if !opts.lists {
        (li.list, li.label) = (0, None);
    } else if matches!(kind, Kind::Heading(_)) && p.props.numbering.is_none() {
        return None;
    }
    Some(li)
}

/// The spacing and indents a paragraph sets itself (its style's are not included).
fn direct_spacing(p: &wordcraft_doc::props::ParaProps) -> Spacing {
    Spacing { before: p.space_before, after: p.space_after, line: p.line_spacing, left: p.indent_left, right: p.indent_right, first: p.indent_first }
}

/// A document paragraph as flow paragraphs: a page break character inside the text ends the
/// paragraph, and the text after it starts a new one that carries `page_break`.
pub fn flow_paras(doc: &Document, p: &Paragraph) -> Vec<Para> {
    flow_paras_in(doc, p, FlowOpts::default())
}

/// [`flow_paras`]; `tracked` keeps tracked changes: deleted text stays, and text carries its
/// insertion and deletion marks.
fn flow_paras_in(doc: &Document, p: &Paragraph, opts: FlowOpts) -> Vec<Para> {
    let tracked = opts.tracked;
    let (kind, list) = para_kind(doc, p);
    let list = flow_list(kind, list, p, opts);
    let mut done: Vec<Para> = Vec::new();
    let borders = if kind == Kind::Rule { None } else { p.props.borders.filter(|b| b.any_visible()) };
    let mut out = Para {
        kind,
        list,
        align: p.props.align,
        page_break: p.props.page_break_before.unwrap_or(false),
        keep_next: p.props.keep_next.unwrap_or(false),
        inlines: Vec::new(),
        spacing: direct_spacing(&p.props),
        borders,
        section: None,
        mark_ins: None,
        mark_del: None,
        fmt_change: None,
        extra: None,
    };
    if opts.paras {
        out.extra = para_extra(doc, &p.props);
    }
    let in_code = kind == Kind::Code;
    let mut k = 0usize;
    let mut control = OpenControl::default();
    for (range, props) in p.run_ranges() {
        let Some(text) = p.text.get(range.clone()) else { continue };
        // Deleted objects and page breaks stay out even when tracking: the flow has no marks for them.
        let hidden = props.hidden.unwrap_or(false) || props.del.is_some();
        // Formats that keep hidden text get it, marked hidden.
        let hidden_text = (props.hidden.unwrap_or(false) && !opts.chars) || (props.del.is_some() && !tracked);
        let mut f = run_fmt(doc, props, in_code, opts);
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
                        spacing: direct_spacing(&p.props),
                        borders,
                        section: None,
                        mark_ins: None,
                        mark_del: None,
                        fmt_change: None,
                        extra: None,
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
            control.interrupt(obj, &mut out, &f);
            // Hidden text, or a hidden drawing: not in the copy.
            if hidden || obj.and_then(InlineObject::frame).is_some_and(|(_, _, f)| f.hidden) {
                continue;
            }
            let from = out.inlines.len();
            if let Some(items) = obj.and_then(|o| rich_inlines(doc, o, &f, opts)) {
                if !buf.is_empty() {
                    out.push_text(&std::mem::take(&mut buf), &f);
                }
                out.inlines.extend(items);
                raise_drawings(&mut out.inlines, from, props.position.filter(|_| opts.paras));
                continue;
            }
            match obj {
                Some(o @ InlineObject::Image { media, w, h, alt, float, ole, .. }) => {
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
                            float: kept_float(float).filter(|_| opts.paras),
                            object: ole.as_deref().filter(|_| opts.boxes).and_then(|e| super::odf_object::load(doc, e)),
                        })),
                        None if !alt.is_empty() => buf.push_str(alt),
                        None => {}
                    }
                }
                Some(InlineObject::Graphic { alt, w, h, float, graphic }) => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    // An editable chart, for formats that keep charts (ODT).
                    if let Some(spec) = graphic.chart.as_ref().filter(|_| opts.boxes) {
                        out.inlines.push(Inline::Chart(crate::model::Chart {
                            spec: spec.clone(),
                            w: *w,
                            h: *h,
                            alt: alt.clone(),
                            float: kept_float(float).filter(|_| opts.paras),
                        }));
                    } else if !alt.trim().is_empty() {
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
                Some(o @ (InlineObject::ControlStart { .. } | InlineObject::ControlEnd)) if control.object(o, opts, &mut buf, &mut out, &f) => {}
                Some(o) => buf.push_str(o.plain_text()),
                None => {}
            }
            raise_drawings(&mut out.inlines, from, props.position.filter(|_| opts.paras));
        }
        if !hidden_text {
            control.run_end(&buf, &mut out, &f);
        }
    }
    control.finish(&mut out);
    if tracked {
        let (ins, del) = rev_marks(doc, &p.mark);
        (out.mark_ins, out.mark_del) = (ins.map(Box::new), del.map(Box::new));
        // The formatting change belongs to the paragraph the mark ends.
        out.fmt_change = p.props.fmt_change.as_deref().map(|c| {
            let blank = out.inlines.is_empty();
            let (kind, mut list) = props_kind(doc, &c.old, blank);
            if !opts.lists
                && let Some(li) = &mut list
            {
                (li.list, li.label) = (0, None);
            }
            let old = Para { kind, list, align: c.old.align, page_break: c.old.page_break_before.unwrap_or(false), ..Default::default() };
            Box::new(FmtChange { mark: rev_mark(doc, c.rev), old })
        });
    }
    // The paragraph that ends a section carries its properties (the last piece of it does).
    out.section.clone_from(&p.section);
    done.push(out);
    done
}

/// The drawings from `from` on (just added) raised by their run's position `raise` (points): a
/// drawing in line keeps it as its float's `y` (see [`crate::model::Img::float`]).
fn raise_drawings(inlines: &mut [Inline], from: usize, raise: Option<f32>) {
    let Some(r) = raise.filter(|r| r.is_finite() && *r != 0.0) else { return };
    for i in inlines.iter_mut().skip(from) {
        let float = match i {
            Inline::Image(img) => &mut img.float,
            Inline::Chart(c) => &mut c.float,
            Inline::Shape { float, .. } | Inline::Group { float, .. } => float,
            _ => continue,
        };
        let f = float.get_or_insert_with(Default::default);
        if f.wrap == wordcraft_doc::para::Wrap::Inline {
            f.y = r;
        }
    }
}

/// A content control of plain text being read (formats with form controls): it, its text so
/// far and that text's formatting.
#[derive(Default)]
struct OpenControl(Option<(Box<wordcraft_doc::control::ContentControl>, String, Option<Fmt>)>);

impl OpenControl {
    /// Object `obj` comes: anything but the control's end makes the control its text after all.
    fn interrupt(&mut self, obj: Option<&InlineObject>, out: &mut Para, f: &Fmt) {
        if !matches!(obj, Some(InlineObject::ControlEnd))
            && let Some((_, text, fmt)) = self.0.take()
        {
            out.push_text(&text, &fmt.unwrap_or_else(|| f.clone()));
        }
    }

    /// A content control starts or ends (`o`): whether it is one read here (a plain-text
    /// control, in a flow that keeps them).
    fn object(&mut self, o: &InlineObject, opts: FlowOpts, buf: &mut String, out: &mut Para, f: &Fmt) -> bool {
        match o {
            InlineObject::ControlStart { control: c } if opts.boxes && !c.block && plain_control(&c.kind) => {
                if !buf.is_empty() {
                    out.push_text(&std::mem::take(buf), f);
                }
                self.0 = Some((c.clone(), String::new(), None));
                true
            }
            InlineObject::ControlEnd if self.0.is_some() => {
                self.close(buf, out, f);
                true
            }
            _ => false,
        }
    }

    /// The control ends: it, with its text (the rest in `buf`), goes into `out`.
    fn close(&mut self, buf: &mut String, out: &mut Para, f: &Fmt) {
        if let Some((c, mut text, fmt)) = self.0.take() {
            text.push_str(&std::mem::take(buf));
            out.inlines.push(Inline::Control { control: c, text, fmt: fmt.unwrap_or_else(|| f.clone()) });
        }
    }

    /// A run ends with `buf`: the control's text when one is open, else the paragraph's.
    fn run_end(&mut self, buf: &str, out: &mut Para, f: &Fmt) {
        match &mut self.0 {
            Some((_, text, fmt)) => {
                text.push_str(buf);
                fmt.get_or_insert_with(|| f.clone());
            }
            None => out.push_text(buf, f),
        }
    }

    /// The paragraph ends: a control it doesn't close is its text.
    fn finish(&mut self, out: &mut Para) {
        if let Some((_, text, fmt)) = self.0.take() {
            out.push_text(&text, &fmt.unwrap_or_default());
        }
    }
}

/// A content control that holds plain text, as a form control does.
fn plain_control(kind: &wordcraft_doc::control::ControlKind) -> bool {
    use wordcraft_doc::control::ControlKind as K;
    matches!(kind, K::Text { .. } | K::CheckBox { .. } | K::DropDown { .. } | K::ComboBox { .. } | K::Date { .. })
}

fn flow_blocks(doc: &Document, bl: &Blocks, depth: usize, tracked: FlowOpts) -> Vec<FBlock> {
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

/// Word's default cell margins (top, left, bottom, right), points.
const WORD_CELL_MARGINS: [f32; 4] = [0.0, 5.4, 0.0, 5.4];

fn flow_table(doc: &Document, t: &Table, depth: usize, tracked: FlowOpts) -> FTable {
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
                // Formats that keep cell padding get each cell's, its table's or Word's default.
                margins: tracked.paras.then(|| c.props.resolved_margins(t.props.cell_margins.unwrap_or(WORD_CELL_MARGINS))),
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
    flow_of(doc, FlowOpts::default())
}

/// [`from_doc`] keeping tracked changes: deleted text stays, and text carries its insertion and
/// deletion marks ([`Fmt::ins`], [`Fmt::del`]), for formats that can write revisions (RTF).
pub fn from_doc_tracked(doc: &Document) -> Flow {
    flow_of(doc, FlowOpts { tracked: true, ..Default::default() })
}

/// [`from_doc`] keeping what `opts` asks for.
pub fn from_doc_with(doc: &Document, opts: FlowOpts) -> Flow {
    flow_of(doc, opts)
}

/// `blocks` of `doc` (a header, footer or note) as flow blocks, keeping what `opts` asks for.
pub fn part_flow(doc: &Document, blocks: &Blocks, opts: FlowOpts) -> Vec<FBlock> {
    flow_blocks(doc, blocks, 0, opts)
}

fn flow_of(doc: &Document, tracked: FlowOpts) -> Flow {
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
