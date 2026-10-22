//! Stories: blocks, paragraphs (`text:p`, or `text:h` with the outline level of headings), runs
//! as spans with their automatic or character styles, links, bookmarks, tabs, line breaks,
//! spaces (`text:s`) and ruby. Page and column breaks inside a paragraph split it, the part
//! after the break starting on a new page (column). Lists go to [`lists`], tables to [`tables`].
//!
//! The other inline objects are written as the exchange-flow writer wrote them: pictures in
//! line, equations as their linear text, fields as their result (`PAGE` and `NUMPAGES` live);
//! notes, comments and drawings without a picture are left out.

use std::sync::Arc;

use wordcraft_doc::para::{COLUMN_BREAK, InlineObject, LINE_BREAK, OBJ, PAGE_BREAK};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Blocks, Paragraph};

use super::pages::Plan;
use super::{Ctx, Props, lists, pt, styles, tables};
use crate::odt::x;

/// The body, section by section: each starts its master page or is a `text:section` with its
/// columns. A list cut by a section boundary goes on numbering after it.
pub(super) fn body(ctx: &mut Ctx, plan: &Plan, out: &mut String) {
    let body = &ctx.doc.body;
    let is_list = |i: Option<usize>| i.and_then(|i| body.get(i)).is_some_and(|b| matches!(&**b, Block::Para(p) if lists::info(ctx.doc, p).is_some()));
    for seg in &plan.segments {
        ctx.page_start.clone_from(&seg.start);
        ctx.lists.continue_next = is_list(Some(seg.blocks.start)) && is_list(seg.blocks.start.checked_sub(1));
        if let Some(c) = &seg.columns {
            let style =
                ctx.auto_style("section", "", &format!("<style:section-properties>{}</style:section-properties>", crate::odt::columns_xml(c)));
            let name = ctx.name("Section");
            out.push_str(&format!("<text:section text:style-name=\"{style}\" text:name=\"{name}\">"));
        }
        run(ctx, body.get(seg.blocks.clone()).unwrap_or(&[]), out);
        if seg.columns.is_some() {
            out.push_str("</text:section>");
        }
    }
}

/// A nested story (header, footer, cell): no page starts.
pub(super) fn blocks(ctx: &mut Ctx, bl: &Blocks, out: &mut String) {
    let saved = (ctx.page_start.take(), ctx.break_next.take(), std::mem::take(&mut ctx.lists.continue_next));
    ctx.nested(out, |ctx, out| {
        run(ctx, bl, out);
        // A story ends with a paragraph, as Word writes it (and draws it): one after a table.
        if bl.last().is_some_and(|b| matches!(**b, Block::Table(_))) {
            para(ctx, &Paragraph::new(), out);
        }
    });
    (ctx.page_start, ctx.break_next, ctx.lists.continue_next) = saved;
}

/// Blocks one after another: list paragraphs grouped into lists, other paragraphs, tables.
fn run(ctx: &mut Ctx, bl: &[Arc<Block>], out: &mut String) {
    let mut i = 0;
    while let Some(b) = bl.get(i) {
        match &**b {
            Block::Para(p) if lists::info(ctx.doc, p).is_some() => i += lists::list(ctx, bl.get(i..).unwrap_or(&[]), out),
            Block::Para(p) => {
                para(ctx, p, out);
                i += 1;
            }
            Block::Table(t) => {
                tables::table(ctx, t, out);
                i += 1;
            }
        }
    }
}

/// A piece of paragraph content.
enum Piece<'a> {
    Text(String),
    Tab,
    Line,
    /// A page (`true`) or column break.
    Break(bool),
    Obj(&'a InlineObject),
}

/// The paragraph's content split into pieces, each with its run's formatting.
fn pieces(p: &Paragraph) -> Vec<(Piece<'_>, &CharProps)> {
    let mut out = Vec::new();
    let mut objects = p.objects.iter();
    for (range, props) in p.run_ranges() {
        let Some(text) = p.text.get(range) else { continue };
        let mut buf = String::new();
        for c in text.chars() {
            let special = match c {
                '\t' => Some(Piece::Tab),
                LINE_BREAK => Some(Piece::Line),
                PAGE_BREAK => Some(Piece::Break(true)),
                COLUMN_BREAK => Some(Piece::Break(false)),
                OBJ => Some(objects.next().map(Piece::Obj).unwrap_or(Piece::Text(String::new()))),
                c if c.is_control() => Some(Piece::Text(String::new())),
                _ => None,
            };
            match special {
                Some(sp) => {
                    if !buf.is_empty() {
                        out.push((Piece::Text(std::mem::take(&mut buf)), props));
                    }
                    out.push((sp, props));
                }
                None => buf.push(c),
            }
        }
        if !buf.is_empty() {
            out.push((Piece::Text(buf), props));
        }
    }
    out
}

/// `s` as ODF text: runs of spaces (and a leading one) as `text:s`, tabs and line breaks as
/// elements. `prev_space` says whether the text before ended with a space (or the paragraph
/// starts here), and is updated. `last`: nothing follows in the paragraph, so trailing spaces
/// are all `text:s` (the reader drops a literal space at a paragraph's end).
fn chars(s: &str, prev_space: &mut bool, last: bool) -> String {
    let mut out = String::new();
    let mut spaces = 0usize;
    let flush = |out: &mut String, spaces: &mut usize, prev: bool| {
        if *spaces == 0 {
            return;
        }
        let extra = if prev { *spaces } else { *spaces - 1 };
        if !prev {
            out.push(' ');
        }
        if extra > 0 {
            out.push_str(&format!("<text:s text:c=\"{extra}\"/>"));
        }
        *spaces = 0;
    };
    for c in s.chars() {
        if c == ' ' {
            spaces += 1;
            continue;
        }
        flush(&mut out, &mut spaces, *prev_space);
        *prev_space = false;
        match c {
            '\t' => {
                // The reader collapses a literal space after a tab, so spaces
                // that follow are written as `<text:s/>`.
                out.push_str("<text:tab/>");
                *prev_space = true;
            }
            '\n' => {
                out.push_str("<text:line-break/>");
                *prev_space = true;
            }
            c if c.is_control() => {}
            c => out.push_str(&x(c.encode_utf8(&mut [0; 4]))),
        }
    }
    if spaces > 0 {
        flush(&mut out, &mut spaces, *prev_space || last);
        *prev_space = true;
    }
    out
}

/// `inner` in a span of the run's style, if it has one.
fn span(ctx: &mut Ctx, props: &CharProps, inner: &str, out: &mut String) {
    match styles::text(ctx, props) {
        Some(n) => out.push_str(&format!("<text:span text:style-name=\"{n}\">{inner}</text:span>")),
        None => out.push_str(inner),
    }
}

/// The link target of a run, when it is one ODF may point at: a web address, mail, or a
/// bookmark in the document (`#name`). Script, `file:` and UNC targets are written as plain
/// text, as the other exporters do.
fn link_of(props: &CharProps) -> Option<&str> {
    props.link.as_deref().filter(|l| !l.is_empty() && (l.starts_with('#') || wordcraft_doc::link::is_safe_href(l)))
}

/// How the paragraph being written opens and closes, to split it at a break.
struct Tag<'a> {
    p: &'a Paragraph,
    /// The opening tag with `{}` for the style name.
    open: String,
    close: &'static str,
}

/// Paragraph `p`. A page start waiting for the body (a new section) goes on its style.
pub(super) fn para(ctx: &mut Ctx, p: &Paragraph, out: &mut String) {
    let start = ctx.page_start.take();
    let mut extra = Props::new();
    if let Some(n) = start.as_ref().and_then(|s| s.number) {
        extra.insert("style:page-number", n.min(32_767).to_string());
    }
    if let Some(b) = ctx.break_next.take() {
        extra.insert("fo:break-before", b.into());
    }
    let master = start.as_ref().map(|s| s.master.as_str());
    let style = styles::paragraph(ctx, p, &extra, master, p.text.is_empty());
    let level = ctx.doc.styles.resolve_para(&p.props).outline_level.filter(|l| *l < 9);
    let (open, close) = match level {
        Some(l) => (format!("<text:h text:style-name=\"{{}}\" text:outline-level=\"{}\">", l + 1), "</text:h>"),
        None => ("<text:p text:style-name=\"{}\">".to_string(), "</text:p>"),
    };
    let tag = Tag { p, open, close };
    out.push_str(&tag.open.replace("{}", &style));
    let items = pieces(p);
    let mut prev_space = true;
    runs(ctx, &items, &tag, &mut prev_space, out);
    out.push_str(close);
}

/// Paragraph content `items`: spans, links and objects. A page or column break closes the
/// paragraph and opens its continuation on a new page or column.
fn runs(ctx: &mut Ctx, items: &[(Piece<'_>, &CharProps)], tag: &Tag<'_>, prev_space: &mut bool, out: &mut String) {
    let mut link: Option<&str> = None;
    let close_link = |link: &mut Option<&str>, out: &mut String| {
        if link.take().is_some() {
            out.push_str("</text:a>");
        }
    };
    let mut i = 0;
    while let Some((piece, props)) = items.get(i) {
        let props: &CharProps = props;
        // Breaks split the paragraph: the rest starts on a new page or column.
        if let Piece::Break(page) = piece {
            let kind = if *page { "page" } else { "column" };
            // A break ending the paragraph moves the next block, as Word draws it (the
            // paragraph mark stays beside the break), rather than leaving an empty
            // paragraph at the top of the new page.
            if items.get(i + 1..).unwrap_or(&[]).iter().all(|(pc, _)| ends(pc)) {
                ctx.break_next = Some(kind);
            } else {
                close_link(&mut link, out);
                out.push_str(tag.close);
                let mut brk = Props::new();
                brk.insert("fo:break-before", kind.into());
                let style = styles::paragraph(ctx, tag.p, &brk, None, false);
                out.push_str(&tag.open.replace("{}", &style));
                *prev_space = true;
            }
            i += 1;
            continue;
        }
        let want = link_of(props);
        if want != link {
            close_link(&mut link, out);
            if let Some(l) = want {
                out.push_str(&format!("<text:a xlink:type=\"simple\" xlink:href=\"{}\">", x(l)));
                link = Some(l);
            }
        }
        if let Piece::Obj(o) = piece {
            object(ctx, o, props, prev_space, out);
            i += 1;
            continue;
        }
        // Text, tabs and line breaks sharing these props make one span.
        let mut inner = String::new();
        while let Some((pc, pr)) = items.get(i) {
            if matches!(pc, Piece::Obj(_) | Piece::Break(_)) || !(std::ptr::eq(*pr, props) || *pr == props) {
                break;
            }
            match pc {
                Piece::Text(t) => inner.push_str(&chars(t, prev_space, at_end(items, i))),
                Piece::Tab => {
                    inner.push_str("<text:tab/>");
                    // Spaces after a tab are written as `<text:s/>` (the reader collapses them).
                    *prev_space = true;
                }
                Piece::Line => {
                    inner.push_str("<text:line-break/>");
                    *prev_space = true;
                }
                Piece::Obj(_) | Piece::Break(_) => {}
            }
            i += 1;
        }
        span(ctx, props, &inner, out);
    }
    close_link(&mut link, out);
}

/// Whether nothing but empty pieces follows item `i` before the paragraph ends or a break
/// splits it.
fn at_end(items: &[(Piece<'_>, &CharProps)], i: usize) -> bool {
    items.get(i + 1..).unwrap_or(&[]).iter().find(|(pc, _)| !ends(pc)).is_none_or(|(pc, _)| matches!(pc, Piece::Break(_)))
}

/// Whether `pc` adds nothing to the line it is on: an empty text, a marker.
fn ends(pc: &Piece<'_>) -> bool {
    match pc {
        Piece::Text(t) => t.is_empty(),
        Piece::Obj(o) => o.is_marker(),
        Piece::Tab | Piece::Line | Piece::Break(_) => false,
    }
}

/// An inline object in a run with formatting `props`.
fn object(ctx: &mut Ctx, o: &InlineObject, props: &CharProps, prev_space: &mut bool, out: &mut String) {
    match o {
        InlineObject::Image { media, w, h, alt, .. } => {
            if o.frame().is_some_and(|(_, _, f)| f.hidden) {
                return;
            }
            let Some(href) = ctx.picture(media) else {
                // A picture whose bytes are missing leaves its description.
                let t = chars(alt, prev_space, false);
                span(ctx, props, &t, out);
                return;
            };
            let name = ctx.name("Image");
            out.push_str(&format!(
                "<draw:frame draw:name=\"{name}\" text:anchor-type=\"as-char\" svg:width=\"{}\" svg:height=\"{}\" draw:z-index=\"0\"><draw:image xlink:href=\"{href}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>",
                pt(w.max(0.0)),
                pt(h.max(0.0))
            ));
            if !alt.is_empty() {
                out.push_str(&format!("<svg:desc>{}</svg:desc>", x(alt)));
            }
            out.push_str("</draw:frame>");
            *prev_space = false;
        }
        InlineObject::Field { instr, result, .. } => {
            let shown = chars(result, prev_space, false);
            let f = match instr.split_whitespace().next().map(str::to_ascii_uppercase).as_deref() {
                Some("PAGE") => format!("<text:page-number text:select-page=\"current\">{shown}</text:page-number>"),
                Some("NUMPAGES") => format!("<text:page-count>{shown}</text:page-count>"),
                _ => shown,
            };
            span(ctx, props, &f, out);
        }
        InlineObject::Equation { linear, math, .. } => {
            let linear = if linear.is_empty() { wordcraft_doc::math::to_linear(&math.nodes) } else { linear.clone() };
            out.push_str(&chars(&linear, prev_space, false));
        }
        InlineObject::BookmarkStart { name } => out.push_str(&format!("<text:bookmark-start text:name=\"{}\"/>", x(name))),
        InlineObject::BookmarkEnd { name } => out.push_str(&format!("<text:bookmark-end text:name=\"{}\"/>", x(name))),
        InlineObject::Ruby { base, ruby, .. } => {
            let inner = format!("<text:ruby><text:ruby-base>{}</text:ruby-base><text:ruby-text>{}</text:ruby-text></text:ruby>", x(base), x(ruby));
            span(ctx, props, &inner, out);
            *prev_space = false;
        }
        // ODF has no positional tab: a plain tab stands in, and its alignment and leader are lost.
        InlineObject::PositionalTab { .. } => {
            span(ctx, props, "<text:tab/>", out);
            *prev_space = true;
        }
        InlineObject::Opaque { text, .. } => {
            let t = chars(text, prev_space, false);
            span(ctx, props, &t, out);
        }
        // todo: notes, comments and drawings without a picture. The marks of complex fields and
        // content controls write nothing: their content is in the text.
        _ => {}
    }
}
