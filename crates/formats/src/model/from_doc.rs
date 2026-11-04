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
    // Kerning on or off where it differs from the document's default.
    let kerns = |k: Option<f32>| k.is_some_and(|k| k.is_finite() && k > 0.0);
    let (kern, default) = (kerns(e.kern.or(doc.styles.default_chr.kern)), kerns(doc.styles.default_chr.kern));
    if kern != default {
        f.extra.get_or_insert_with(Default::default).kern = Some(if kern { 1.0 } else { 0.0 });
    }
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
        emphasis: e.emphasis.filter(|m| *m != wordcraft_doc::props::EmphasisMark::None),
        position: e.position.filter(|v| *v != 0.0 && v.is_finite()),
        outline: set(e.outline),
        shadow: set(e.shadow),
        emboss: set(e.emboss),
        engrave: set(e.engrave),
        border: e.border.filter(|b| b.is_visible()),
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

/// Whether `doc`'s tab stops are measured from each paragraph's indent (an ODF document whose
/// file says so): [`Para::extra`]'s tab stops then are too.
pub fn tabs_from_indent(doc: &Document) -> bool {
    doc.settings.odf.is_some_and(|o| o.tabs_from_indent)
}

/// The paragraph formatting only [`Para::extra`] holds that `p` sets itself; `None` when none.
fn para_extra(doc: &Document, p: &wordcraft_doc::props::ParaProps) -> Option<Box<wordcraft_doc::props::ParaProps>> {
    let tabs = p.tabs.clone().filter(|t| !t.is_empty()).map(|mut tabs| {
        // Measured from the paragraph's indent where the document measures them so (ODT).
        if tabs_from_indent(doc) {
            let indent = doc.styles.resolve_para(p).indent_left;
            for t in &mut tabs {
                t.pos = (t.pos - indent).clamp(-1584.0, 1584.0);
            }
        }
        tabs
    });
    let style = p.style.clone().filter(|id| is_custom_style(doc, id));
    let x = wordcraft_doc::props::ParaProps {
        tabs,
        shading: p.shading,
        style,
        contextual_spacing: p.contextual_spacing,
        bidi: p.bidi.filter(|b| *b),
        drop_cap: p.drop_cap.filter(|l| *l > 1),
        drop_cap_distance: p.drop_cap_distance.filter(|_| p.drop_cap.is_some_and(|l| l > 1)),
        keep_lines: p.keep_lines.filter(|k| *k),
        outline_level: p.outline_level,
        widow_control: p.widow_control.filter(|w| !*w),
        // Hyphenated (the document hyphenates and the paragraph doesn't opt out): `Some(false)`.
        suppress_hyphens: (doc.settings.auto_hyphenation && p.suppress_hyphens != Some(true)).then_some(false),
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
    // A heading's kind is its style's (Heading 2 looks as Heading 2 whatever its outline level,
    // which the paragraph may set apart); else its outline level's.
    let style_level =
        doc.styles.chain(style).iter().find_map(|s| s.id.strip_prefix("Heading").and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=9).contains(n)));
    let kind = if let Some(l) = style_level.filter(|_| rp.outline_level.is_some()) {
        Kind::Heading(l.min(6))
    } else if let Some(l) = rp.outline_level {
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
            // The space after the mark is the mark's (see `to_doc`'s `note`); a tab or more
            // spaces there are the note's own.
            if let Some(FBlock::Para(first)) = blocks.first_mut() {
                let k = first.inlines.iter().take_while(|i| matches!(i, Inline::Text(t, _) if t.is_empty())).count();
                first.inlines.drain(..k);
                if let Some(Inline::Text(t, _)) = first.inlines.first_mut()
                    && t.starts_with(' ')
                {
                    t.remove(0);
                }
            }
            let endnote = *kind == wordcraft_doc::para::NoteKind::Endnote;
            vec![Inline::Note { endnote, custom: custom.clone(), blocks, fmt: f.clone() }]
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
        InlineObject::Field { instr, result, locked, .. } if opts.fields => {
            vec![Inline::Field { instr: instr.clone(), result: result.clone(), fmt: f.clone(), locked: *locked }]
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
            // A connector stays one in formats that glue them (ODT, with boxes); elsewhere it is a
            // straight line.
            let kind = match kind {
                ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector if !opts.boxes => ShapeKind::Line,
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
                vert: part.map(|p| p.body.vert).unwrap_or_default(),
                // Kept only while it is still the freeform it was read as.
                geometry: extra.odf_geometry.clone().filter(|_| (kind == ShapeKind::Freeform && freeform.is_some()) || kind == ShapeKind::Rectangle),
                conn: (opts.boxes && (extra.id != 0 || extra.start.is_some() || extra.end.is_some()))
                    .then(|| Box::new(crate::model::Conn { id: extra.id, start: extra.start, end: extra.end })),
                // A text box is named after its story; a linked one names the box it continues in.
                chain: story.filter(|_| opts.boxes).map(|id| {
                    let next = part.and_then(|p| p.body.next).map(|n| format!("Frame{n}"));
                    Box::new(crate::model::Chain { name: format!("Frame{id}"), next })
                }),
            };
            let path = freeform.clone().filter(|_| kind == ShapeKind::Freeform);
            Some(Inline::Shape { kind, w: w * sx, h: h * sy, float: kept_float(float), blocks, look, path })
        }
        InlineObject::Image { media, w, h, alt, float, ole, crop, .. } => {
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
                crop: if opts.paras { *crop } else { [0.0; 4] },
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
    Spacing {
        before: p.space_before,
        after: p.space_after,
        line: p.line_spacing,
        left: p.indent_left,
        right: p.indent_right,
        first: p.indent_first,
        ..Default::default()
    }
}

/// A document paragraph as flow paragraphs: a page break character inside the text ends the
/// paragraph, and the text after it starts a new one that carries `page_break`.
/// An object's text in the flow: ASK and SET show nothing, whatever result they hold.
fn shown_text(o: &InlineObject) -> &str {
    match o {
        InlineObject::Field { instr, .. } if super::shows_nothing(instr) => "",
        o => o.plain_text(),
    }
}

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
    let borders = if kind == Kind::Rule { None } else { p.props.borders.filter(|b| b.takes_room()) };
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
        mark: None,
        kept: None,
        wrap: None,
        index: None,
        group: None,
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
        let hidden_obj = (props.hidden.unwrap_or(false) && !opts.chars) || props.del.is_some();
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
                        mark: None,
                        kept: None,
                        wrap: None,
                        index: None,
                        group: None,
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
            // Hidden text a format leaves out, a deleted object, or a hidden drawing: not in the
            // copy (a format keeping hidden text keeps its objects too, as it keeps the text).
            if hidden_obj || obj.and_then(InlineObject::frame).is_some_and(|(_, _, f)| f.hidden) {
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
                Some(o @ InlineObject::Image { media, w, h, alt, float, ole, crop, .. }) => {
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
                            crop: if opts.paras { *crop } else { [0.0; 4] },
                        })),
                        None if !alt.is_empty() => buf.push_str(alt),
                        None => {}
                    }
                }
                Some(InlineObject::Graphic { alt, w, h, float, graphic }) => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    out.inlines.extend(graphic_inline(doc, (alt, *w, *h), float, graphic, opts));
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
                Some(o) => buf.push_str(shown_text(o)),
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
    out.mark = empty_mark(&out, p, opts);
    // Still what the file it came from had (a whole paragraph): its markup, for its format. Not
    // for one starting a page: the break may be its element's or the one before's (a break after
    // it), and only the model says which is written where.
    if opts.paras && done.is_empty() && p.props.page_break_before != Some(true) {
        // With its comments and its text boxes' text, while they're as read.
        out.kept = doc
            .kept
            .of("odt")
            .and_then(|k| {
                k.para(p).filter(|xml| (!xml.contains("<office:annotation") || k.comments_as_read(doc, p, xml)) && k.stories_as_read(doc, p))
            })
            .cloned();
        // With the group it is written as read with, if any (see `model::Group`).
        out.group =
            out.kept.as_ref().and_then(|_| doc.kept.of("odt")?.group(p)).map(|id| Box::new(crate::model::Group { links: Vec::new(), id: Some(id) }));
    }
    // The sections kept as read that it opens or closes.
    if opts.paras {
        out.wrap = doc.kept.of("odt").and_then(|k| k.wrap(p)).map(|w| Box::new(w.clone()));
    }
    done.push(out);
    done
}

/// A chart or diagram in a flow: an editable chart for formats that keep charts (ODT), a
/// diagram's drawing as a group for formats that keep shapes, else its alt text.
fn graphic_inline(
    doc: &Document,
    (alt, w, h): (&str, f32, f32),
    float: &wordcraft_doc::para::Float,
    graphic: &wordcraft_doc::graphic::Graphic,
    opts: FlowOpts,
) -> Option<Inline> {
    if let Some(spec) = graphic.chart.as_ref().filter(|_| opts.boxes) {
        return Some(Inline::Chart(crate::model::Chart {
            spec: spec.clone(),
            w,
            h,
            alt: alt.to_string(),
            float: kept_float(float).filter(|_| opts.paras),
            source: graphic.source.as_deref().and_then(|e| super::odf_object::load(doc, e)),
        }));
    }
    // A diagram (SmartArt) for formats without one: its drawing as a group of shapes, text boxes
    // and pictures every reader draws.
    if opts.boxes
        && let Some(group) = graphic_group(doc, (w, h), float, graphic)
    {
        return Some(group);
    }
    (!alt.trim().is_empty()).then(|| Inline::Figure(alt.to_string()))
}

/// A graphic's drawing items ([`wordcraft_doc::graphic::GraphicItem`]) as a group `w` × `h`
/// points: its shapes as shapes, its text as borderless text boxes centred on their places, its
/// pictures as pictures. `None` when nothing in it can be drawn so (paths are left out).
fn graphic_group(
    doc: &Document,
    (w, h): (f32, f32),
    float: &wordcraft_doc::para::Float,
    graphic: &wordcraft_doc::graphic::Graphic,
) -> Option<Inline> {
    use wordcraft_doc::graphic::{GraphicItem, TextAlign};
    use wordcraft_doc::para::ShapeKind;
    let ratio = |outer: f32, inner: f32| {
        let r = outer / inner;
        if r.is_finite() && r > 0.0 { r } else { 1.0 }
    };
    let (sx, sy) = (ratio(w, graphic.w), ratio(h, graphic.h));
    let finite = |r: &[f32; 4]| r.iter().all(|v| v.is_finite()) && r[2] >= 0.0 && r[3] >= 0.0;
    let plain = BoxLook { fill: None, line: None, inset: Some([0.0; 4]), ..BoxLook::default() };
    let members: Vec<Member> = graphic
        .items
        .iter()
        .take(10_000)
        .filter_map(|item| match item {
            GraphicItem::Shape { rect, kind, fill, stroke, stroke_width } if finite(rect) => {
                let look = BoxLook {
                    fill: fill.map(wordcraft_doc::fill::Fill::solid).filter(|_| *kind != ShapeKind::Line),
                    line: stroke.filter(|_| stroke_width.is_finite() && *stroke_width > 0.0).map(|c| (c, stroke_width.min(12.0))),
                    ..plain.clone()
                };
                let inline = Inline::Shape { kind: *kind, w: rect[2] * sx, h: rect[3] * sy, float: None, blocks: Vec::new(), look, path: None };
                Some(Member { x: rect[0] * sx, y: rect[1] * sy, inline })
            }
            GraphicItem::Text { rect, text, size, color, bold, align, font } if finite(rect) && !text.trim().is_empty() => {
                let size = if size.is_finite() { size.clamp(1.0, 400.0) } else { 11.0 };
                // One line, centred vertically in its place as the graphic draws it.
                let line = size * 1.2;
                let (bw, bh) = (rect[2] * sx, line.max(1.0));
                let y = rect[1] * sy + (rect[3] * sy - bh) / 2.0;
                let fmt = Fmt { bold: *bold, color: Some(*color), size: Some(size), font: font.as_deref().map(Arc::from), ..Fmt::default() };
                let para = Para {
                    align: Some(match align {
                        TextAlign::Left => wordcraft_doc::Align::Left,
                        TextAlign::Right => wordcraft_doc::Align::Right,
                        _ => wordcraft_doc::Align::Center,
                    }),
                    inlines: vec![Inline::Text(text.clone(), fmt)],
                    spacing: Spacing { before: Some(0.0), after: Some(0.0), ..Spacing::default() },
                    ..Para::default()
                };
                let inline = Inline::Shape {
                    kind: ShapeKind::Rectangle,
                    w: bw,
                    h: bh,
                    float: None,
                    blocks: vec![FBlock::Para(para)],
                    look: plain.clone(),
                    path: None,
                };
                Some(Member { x: rect[0] * sx, y, inline })
            }
            GraphicItem::Image { rect, media } if finite(rect) => {
                let data = doc.media.get(media)?;
                let img = Img {
                    data: data.clone(),
                    ext: media_ext(media, data),
                    w: rect[2] * sx,
                    h: rect[3] * sy,
                    alt: String::new(),
                    float: None,
                    object: None,
                    crop: [0.0; 4],
                };
                Some(Member { x: rect[0] * sx, y: rect[1] * sy, inline: Inline::Image(img) })
            }
            _ => None,
        })
        .collect();
    if members.is_empty() {
        return None;
    }
    // A group is as big as its members together (ODF 1.3 §10.3.15): an invisible rectangle the
    // size of the graphic keeps the room around its drawing.
    let frame = Inline::Shape { kind: ShapeKind::Rectangle, w, h, float: None, blocks: Vec::new(), look: plain, path: None };
    let all = std::iter::once(Member { x: 0.0, y: 0.0, inline: frame }).chain(members).collect();
    Some(Inline::Group { w, h, float: kept_float(float), members: all })
}

/// The mark formatting of `out`, the last piece of `p`, when it holds no text (its height, its
/// drawings' formatting), for
/// formats that keep it (flows asked for paragraph formatting).
fn empty_mark(out: &Para, p: &Paragraph, opts: FlowOpts) -> Option<Box<CharProps>> {
    let m = CharProps { ins: None, del: None, fmt_change: None, ..p.mark.clone() };
    (opts.paras && out.textless() && !m.is_empty()).then(|| Box::new(m))
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

/// The index kept as read starting at block `i` of `bl`, while the paragraphs it takes are all
/// as read: how many blocks, and its markup.
fn kept_index(doc: &Document, bl: &Blocks, i: usize) -> Option<(usize, std::sync::Arc<str>)> {
    let k = doc.kept.of("odt")?;
    let p = bl.get(i)?.as_para()?;
    let (n, run, xml) = k.index(p)?;
    let paras: Vec<&Paragraph> = bl.get(i..i + n)?.iter().filter_map(|b| b.as_para()).collect();
    (paras.len() == *n && wordcraft_doc::kept::run_fingerprint(paras.into_iter()) == *run).then(|| (*n, xml.clone()))
}

fn flow_blocks(doc: &Document, bl: &Blocks, depth: usize, tracked: FlowOpts) -> Vec<FBlock> {
    let mut out = Vec::new();
    let mut skip = 0usize;
    for (i, b) in bl.iter().enumerate() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        // An index kept as read, while every paragraph it takes still is what was read: its
        // flow paragraphs, the first carrying its markup and how many there are.
        if tracked.paras
            && depth == 0
            && let Some(ix) = kept_index(doc, bl, i)
        {
            let from = out.len();
            for b in bl.get(i..i + ix.0).unwrap_or(&[]) {
                if let Block::Para(p) = &**b {
                    out.extend(flow_paras_in(doc, p, tracked).into_iter().map(FBlock::Para));
                }
            }
            let n = out.len() - from;
            if let Some(FBlock::Para(first)) = out.get_mut(from) {
                first.index = Some(Box::new((ix.1, n)));
            }
            skip = ix.0.saturating_sub(1);
            continue;
        }
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

/// What a table's style and borders give one cell, worked out as the layout does, for formats
/// that keep cell formatting (ODT) but have no table styles of Word's kind: its shading, its four
/// edges (a side without a line is a "none" border), its vertical alignment, and its text's
/// character formatting and alignment.
struct StyledCell {
    fill: Option<wordcraft_doc::props::Rgb>,
    borders: wordcraft_doc::props::Borders,
    v_align: Option<wordcraft_doc::props::VAlign>,
    chr: CharProps,
    align: Option<wordcraft_doc::Align>,
    /// The style's paragraph formatting (spacing, line spacing) under each paragraph's own.
    para: ParaProps,
}

fn styled_cell(t: &Table, style: Option<&wordcraft_doc::styles::TableStyleProps>, (ri, ci): (usize, usize), ncols: usize) -> Option<StyledCell> {
    use wordcraft_doc::props::{Border, BorderStyle, Borders};
    use wordcraft_doc::styles::{PartSet, TablePart};
    let row = t.rows.get(ri)?;
    let cell = row.cells.get(ci)?;
    let nrows = t.rows.len();
    let g = t.grid_col(ri, ci);
    let span = cell.span();
    let mut out = StyledCell {
        fill: None,
        borders: Borders::default(),
        v_align: None,
        chr: CharProps::default(),
        align: None,
        para: style.map(|s| s.para.clone()).unwrap_or_default(),
    };
    let mut part_borders = Vec::new();
    if let Some(st) = style {
        let sizes = (st.parts.band_size.unwrap_or(0), st.parts.col_band_size.unwrap_or(0));
        for part in PartSet::of_cell(&t.props.look, sizes, (ri, nrows), (g, span, ncols)).iter() {
            let f = st.part(part);
            out.fill = f.fill.or(out.fill);
            out.v_align = f.v_align.or(out.v_align);
            out.chr.overlay(&f.chr);
            out.align = f.align.or(out.align);
            if part != TablePart::WholeTable
                && let Some(b) = f.borders
            {
                part_borders.push((part, b));
            }
        }
    }
    let mut tb = style.and_then(|s| s.parts.borders).unwrap_or_default();
    if let Some(own) = t.props.borders {
        tb.overlay(&own);
    }
    let cb = cell.props.borders.unwrap_or_default();
    let rowb = row.props.exceptions.as_ref().and_then(|e| e.borders).unwrap_or_default();
    let (first_g, last_g) = (g == row.first_grid(), g + span >= ncols.saturating_sub(row.grid_after()));
    let rb = wordcraft_doc::styles::part_borders(&part_borders, (ri, nrows), (first_g, last_g));
    let edge = |own: Option<Border>, outer: bool, outer_b: Option<Border>, inner_b: Option<Border>| own.or(if outer { outer_b } else { inner_b });
    let none = Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 };
    let top = edge(cb.top.or(if ri == 0 { rowb.top } else { rowb.between }).or(rb.top), ri == 0, tb.top, tb.between);
    let bottom = edge(cb.bottom.or(if ri + 1 == nrows { rowb.bottom } else { rowb.between }).or(rb.bottom), ri + 1 == nrows, tb.bottom, tb.between);
    let left = edge(
        cb.left.or(if first_g { rowb.left } else { rowb.inside_v }).or(if first_g { rb.left } else { rb.inside_v }),
        first_g,
        tb.left,
        tb.inside_v,
    );
    let right = edge(
        cb.right.or(if last_g { rowb.right } else { rowb.inside_v }).or(if last_g { rb.right } else { rb.inside_v }),
        last_g,
        tb.right,
        tb.inside_v,
    );
    out.borders = Borders {
        top: top.or(Some(none)),
        bottom: bottom.or(Some(none)),
        left: left.or(Some(none)),
        right: right.or(Some(none)),
        between: None,
        inside_v: None,
    };
    Some(out)
}

/// `blocks` with what a table style's character and paragraph formatting changes (see
/// [`wordcraft_doc::styles::StyleSheet::table_para_patch`]) stated on each paragraph and run.
fn with_cell_text(doc: &Document, blocks: &Blocks, chr: &CharProps, align: Option<wordcraft_doc::Align>, para: &ParaProps) -> Blocks {
    if chr.is_empty() && align.is_none() && para.is_empty() {
        return blocks.clone();
    }
    blocks
        .iter()
        .map(|b| match &**b {
            Block::Para(p) => {
                let mut p = p.clone();
                let pstyle = p.props.style.clone();
                if !chr.is_empty() {
                    for r in &mut p.runs {
                        let mut props = doc.styles.table_char_patch(pstyle.as_deref(), r.props.style.as_deref(), chr);
                        props.overlay(&r.props);
                        r.props = props;
                    }
                    let mut mark = doc.styles.table_char_patch(pstyle.as_deref(), p.mark.style.as_deref(), chr);
                    mark.overlay(&p.mark);
                    p.mark = mark;
                }
                // A part's alignment is table style paragraph formatting too, under the paragraph's styles.
                let table = ParaProps { align: align.or(para.align), ..para.clone() };
                if !table.is_empty() {
                    let mut props = doc.styles.table_para_patch(&p.props, &table);
                    props.overlay(&p.props);
                    p.props = props;
                }
                Arc::new(Block::Para(p))
            }
            _ => b.clone(),
        })
        .collect()
}

fn flow_table(doc: &Document, t: &Table, depth: usize, tracked: FlowOpts) -> FTable {
    let mut rows = Vec::new();
    // Formats that keep cell formatting get the table style's and the table's borders cell by cell.
    let style = t.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
    let bake = tracked.paras;
    let ncols =
        t.rows.iter().map(|r| r.first_grid() + r.cells.iter().map(|c| c.span()).sum::<usize>() + r.grid_after()).max().unwrap_or(0).max(t.grid.len());
    for (ri, r) in t.rows.iter().enumerate() {
        let header = r.props.header || (ri == 0 && t.props.look.header_row && t.rows.len() > 1);
        let mut cells = Vec::new();
        for (ci, c) in r.cells.iter().enumerate() {
            let styled = if bake { styled_cell(t, style.as_ref(), (ri, ci), ncols) } else { None };
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
            let blocks = match &styled {
                Some(st) => flow_blocks(doc, &with_cell_text(doc, &c.blocks, &st.chr, st.align, &st.para), depth + 1, tracked),
                None => flow_blocks(doc, &c.blocks, depth + 1, tracked),
            };
            cells.push(Cell {
                blocks,
                colspan: c.span() as u32,
                rowspan,
                covered: c.props.vmerge == VMerge::Continue,
                header,
                shading: c.props.shading.or(styled.as_ref().and_then(|s| s.fill)),
                borders: styled.as_ref().map(|s| s.borders).or(c.props.borders),
                valign: c.props.valign.or(styled.as_ref().and_then(|s| s.v_align)),
                // Formats that keep cell padding get each cell's, its table's or Word's default.
                margins: tracked.paras.then(|| c.props.resolved_margins(t.props.cell_margins.unwrap_or(WORD_CELL_MARGINS))),
                direction: c.props.text_direction,
                // Its background picture, for formats that keep cell formatting (ODT).
                picture: c.props.picture.as_deref().filter(|_| tracked.paras).and_then(|p| {
                    let data = doc.media.get(&p.media)?;
                    let img = Img {
                        data: data.clone(),
                        ext: media_ext(&p.media, data),
                        w: p.size[0],
                        h: p.size[1],
                        alt: String::new(),
                        float: None,
                        object: None,
                        crop: [0.0; 4],
                    };
                    Some(Box::new(crate::model::CellPic { img, repeat: p.repeat, align: p.align }))
                }),
                width: None,
            });
        }
        rows.push(cells);
    }
    // Without a table style and without visible table or cell borders the table shows no grid.
    let visible = |b: &Option<wordcraft_doc::props::Borders>| b.as_ref().is_some_and(|b| b.any_visible());
    let borderless =
        t.props.style.is_none() && !visible(&t.props.borders) && t.rows.iter().flat_map(|r| &r.cells).all(|c| !visible(&c.props.borders));
    // Formats that keep them get the table's alignment, width and row heights.
    let extra = tracked.paras.then(|| {
        use wordcraft_doc::props::HeightRule;
        let heights = t
            .rows
            .iter()
            .map(|r| match (r.props.height, r.props.height_rule) {
                (Some(h), HeightRule::Exact) => Some((h, true)),
                (Some(h), HeightRule::AtLeast) => Some((h, false)),
                _ => None,
            })
            .collect::<Vec<_>>();
        super::types::TableExtra {
            align: t.props.align,
            width: t.props.width,
            width_pct: t.props.width_pct,
            heights,
            fixed: t.props.fixed,
            space: [t.props.space_before, t.props.space_after],
            header_rows: t.rows.iter().take_while(|r| r.props.header).count(),
        }
    });
    // Kept whole for formats that ask for it: its header rows count even without the rest.
    let extra = extra.map(Box::new);
    let kept = (tracked.paras && depth == 0).then(|| doc.kept.of("odt").and_then(|k| k.table(t)).cloned()).flatten();
    let wrap = (tracked.paras && depth == 0).then(|| doc.kept.of("odt").and_then(|k| k.table_wrap(t)).cloned().map(Box::new)).flatten();
    FTable { rows, widths: t.grid.clone(), borderless, extra, kept, wrap }
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
    // Paragraphs kept as read are checked against the document's comments and page setup:
    // worked out once for the whole document, not once per paragraph.
    wordcraft_doc::kept::while_writing(doc, || flow_of(doc, opts))
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
