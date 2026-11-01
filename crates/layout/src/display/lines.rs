use super::glyphs::{Authors, check_box, text_color, upright_in_vertical};
use super::objects::{graphic_draws, inline_rect, object_draws, spun};
use super::options::DisplayOptions;
use super::page::{char_box, stroke_of};
use super::paint::Stroke;
use super::{
    Block, Border, ClKind, DeleteMark, Document, Draw, InlineObject, InsertMark, LineEnd, MItem, ParaLayout, Path, Rect, Rgb, SectionStart, Spin,
    StoryRef, TextColor, Underline,
};
use wordcraft_doc::Paragraph;

/// Append display items for the half-open line range `l0..l1` of a laid-out paragraph.
/// `(x, y)` positions the paragraph's left origin and the first selected line's top; later lines
/// retain their layout offsets. Includes glyphs, inline objects, decorations and requested markup.
/// `down` keeps vertical East Asian glyphs upright before the caller turns the resulting draws.
pub(super) struct LineRender<'a, 'authors, 'author_doc> {
    pub doc: &'a Document,
    pub story: StoryRef,
    pub path: &'a Path,
    pub pl: &'a ParaLayout,
    pub range: std::ops::Range<usize>,
    pub origin: (f32, f32),
    pub opts: &'a DisplayOptions,
    pub authors: &'authors Authors<'author_doc>,
    pub alpha: f32,
    pub down: bool,
}

struct LineContext<'a, 'authors, 'author_doc> {
    doc: &'a Document,
    story: StoryRef,
    path: &'a Path,
    pl: &'a ParaLayout,
    para: Option<&'a Paragraph>,
    line: &'a crate::para::Line,
    li: usize,
    x: f32,
    top: f32,
    base: f32,
    bottom: f32,
    opts: &'a DisplayOptions,
    authors: &'authors Authors<'author_doc>,
    alpha: f32,
    down: bool,
    auto: Option<Rgb>,
}

impl LineContext<'_, '_, '_> {
    fn ink(&self, color: &TextColor, background: Option<Rgb>) -> Rgb {
        match (color, self.auto) {
            (TextColor::Auto, Some(rgb)) => rgb,
            _ => text_color(color, background),
        }
    }
}

pub(super) fn lines(input: LineRender<'_, '_, '_>, out: &mut Vec<Draw>) {
    let LineRender { doc, story, path, pl, range, origin: (x, y), opts, authors, alpha, down } = input;
    let (l0, l1) = (range.start, range.end);
    let Some(first) = pl.lines.get(l0) else { return };
    let para = doc.para(story, path);
    // Inside a text box whose shape style gives its text a colour, that is what automatic means.
    let auto = match story {
        StoryRef::Part(id) => doc.parts.get(&id).and_then(|p| p.body.auto_color),
        StoryRef::Body => None,
    };
    for li in l0..l1 {
        let Some(line) = pl.lines.get(li) else { continue };
        let top = y + (line.top - first.top);
        let base = top + (line.baseline - line.top);
        let bottom = top + line.height;
        let ctx = LineContext { doc, story, path, pl, para, line, li, x, top, base, bottom, opts, authors, alpha, down, auto };
        comment_backgrounds(&ctx, out);
        character_backgrounds(&ctx, out);
        draw_label_and_hyphen(&ctx, out);
        draw_glyph_runs(&ctx, out);
        proofing_marks(&ctx, out);
        tab_leaders(&ctx, out);
        inline_objects(&ctx, out);
        enclosures(&ctx, out);
        draw_rubies(&ctx, out);
        formatting_marks(&ctx, out);
    }
}

fn collect_glyph_run(
    ctx: &LineContext<'_, '_, '_>,
    start: usize,
    style_index: u32,
    style: &crate::para::StyleRun,
) -> (usize, Vec<(u32, f32, f32)>, String, Vec<std::ops::Range<usize>>) {
    let (pl, para, line, x, base) = (ctx.pl, ctx.para, ctx.line, ctx.x, ctx.base);
    let si = style_index;
    let st = style;
    let mut k = start;
    let run = pl.clusters.get(start..line.c1.min(pl.clusters.len())).unwrap_or_default();
    let (n_glyphs, n_bytes) = run
        .iter()
        .take_while(|cluster| cluster.style == si)
        .filter(|cluster| matches!(cluster.kind, ClKind::Text | ClKind::Space))
        .fold((0usize, 0usize), |(glyphs, bytes), cluster| {
            (glyphs + cluster.g1.saturating_sub(cluster.g0) as usize, bytes + cluster.end.saturating_sub(cluster.start))
        });
    let mut glyphs = Vec::with_capacity(n_glyphs);
    let mut text = String::with_capacity(n_bytes);
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::with_capacity(n_glyphs);
    let symbol_font = wordcraft_doc::symbol_fonts::is_symbol_font(&st.rc.font);
    while k < line.c1 {
        let Some(c) = pl.clusters.get(k) else { break };
        if c.style != si {
            break;
        }
        let cx = x + line.cl_left(k).unwrap_or(0.0);
        if matches!(c.kind, ClKind::Text | ClKind::Space) {
            let a = text.len();
            // An object's cluster (a field, a note number) stands for the text it shows.
            let mut shown = None;
            if let Ok(i) = pl.shown.binary_search_by_key(&k, |(i, _, _)| *i)
                && let Some((_, s, r)) = pl.shown.get(i)
            {
                // All Caps field results read as the capitals drawn (#1473). `r` holds
                // byte ranges into `s`, so a character whose capital has another UTF-8
                // length keeps its own form and every range still lands where it did.
                text.extend(s.chars().map(|ch| {
                    let up = crate::para::caps_char(&st.rc, ch);
                    if up.len_utf8() == ch.len_utf8() { up } else { ch }
                }));
                shown = Some(r);
            } else if let Some(p) = para {
                let t = p.text.get(c.start..c.end).unwrap_or("");
                // Symbol-font characters (Wingdings…) read as the Unicode symbols drawn.
                if symbol_font {
                    text.push_str(&wordcraft_doc::symbol_fonts::map_text(&st.rc.font, t));
                } else if st.rc.caps || st.rc.small_caps {
                    // All Caps and Small Caps read as the capitals drawn (#1473), mapped
                    // as drawn, so a glyph never stands for two cases and ß reads as SS (#1581).
                    for ch in t.chars() {
                        crate::para::push_caps(&st.rc, ch, &mut text);
                    }
                } else {
                    text.push_str(t);
                }
            }
            let b = text.len();
            let cg = pl.glyphs.get(c.g0 as usize..c.g1 as usize).unwrap_or(&[]);
            if let Some(shown) = shown
                && cg.len() > 1
            {
                // A whole field result is one cluster of many glyphs: each glyph gets the
                // text it was shaped from (a ligature its letters), not the whole text as
                // one ligature. Without that, a character each (the last takes any remainder).
                let bounds: Vec<usize> = text.get(a..b).unwrap_or("").char_indices().map(|(i, _)| a + i).collect();
                let known = shown.len() == cg.len();
                for (n, g) in cg.iter().enumerate() {
                    let r = match shown.get(n).filter(|_| known) {
                        Some(r) => (a + r.start).min(b)..(a + r.end).min(b),
                        None => {
                            let from = bounds.get(n).or(bounds.last()).copied().unwrap_or(a);
                            let to = if n + 1 == cg.len() { b } else { bounds.get(n + 1).copied().unwrap_or(b) };
                            from..to
                        }
                    };
                    glyphs.push((g.gid, cx + g.dx, base - st.shift - g.dy));
                    ranges.push(r.start..r.end.max(r.start));
                }
                k += 1;
                continue;
            }
            if cg.is_empty() {
                // No glyph of its own (the second letter of a ligature such as لا): its
                // text belongs to the glyphs before it.
                let from = ranges.last().map(|r| r.start);
                for r in ranges.iter_mut().rev().take_while(|r| Some(r.start) == from) {
                    r.end = b;
                }
            }
            for g in cg {
                glyphs.push((g.gid, cx + g.dx, base - st.shift - g.dy));
                ranges.push(a..b);
            }
        }
        k += 1;
    }
    (k, glyphs, text, ranges)
}

fn draw_glyph_runs(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    let (doc, pl, line, x, base, opts, authors, alpha, down) =
        (ctx.doc, ctx.pl, ctx.line, ctx.x, ctx.base, ctx.opts, ctx.authors, ctx.alpha, ctx.down);
    let mut k = line.c0;
    while k < line.c1 {
        let Some(c) = pl.clusters.get(k) else { break };
        let si = c.style;
        let Some(st) = pl.styles.get(si as usize) else {
            k += 1;
            continue;
        };
        let run_start = k;
        let (run_end, mut glyphs, text, ranges) = collect_glyph_run(ctx, run_start, si, st);
        k = run_end;
        let rc = &st.rc;
        let m = &opts.revisions;
        // Without markup a deletion is never drawn as ordinary text, even in a layout that
        // kept it; nor when its mark is Hidden.
        if rc.del.is_some() && (!opts.markup || m.hides_deletion(doc.move_name(rc.del).is_some())) {
            continue;
        }
        let shown = opts.markup && m.insertions_deletions;
        let del = rc.del.filter(|_| shown);
        let ins = rc.ins.filter(|_| shown && del.is_none());
        let author = |r: u32| authors.color(r);
        // Moved text: the Moves marks and colours (by default green, double strikethrough
        // where it was, double underline where it went).
        let moved_to = doc.move_name(ins).is_some();
        let moved_from = del.is_some() && doc.move_name(del).is_some();
        // Moved text marked (none) is plain text, its colour setting aside (Word for Mac).
        let plain_move = (moved_from && m.move_from_mark == DeleteMark::None) || (moved_to && m.move_to_mark == InsertMark::None);
        let color = match (ins, del) {
            _ if plain_move => text_color(&rc.color, rc.shading.or(rc.highlight)),
            (_, Some(r)) if moved_from => m.move_from_color.unwrap_or_else(|| author(r)),
            (Some(r), _) if moved_to => m.move_to_color.unwrap_or_else(|| author(r)),
            (Some(r), _) => m.insert_color.unwrap_or_else(|| author(r)),
            (None, Some(r)) => m.delete_color.unwrap_or_else(|| author(r)),
            _ => ctx.ink(&rc.color, rc.shading.or(rc.highlight)),
        };
        // The highlight or shading drawn under this run (as filled in the backgrounds pass).
        let text_bg = rc.highlight.or(rc.shading);
        let ins_mark = ins.map(|_| if moved_to { m.move_to_mark } else { m.insert_mark });
        let del_mark = del.map(|_| if moved_from { m.move_from_mark } else { m.delete_mark });
        if del_mark == Some(DeleteMark::Caret) {
            // A caret where the text was, instead of the text.
            if let Some(cx) = line.cl_left(run_start).map(|c| x + c) {
                let h = st.size * 0.3;
                let w = (st.size / 18.0).max(0.6);
                out.push(Draw::Line {
                    x0: cx - h * 0.6,
                    y0: base + 1.0,
                    x1: cx,
                    y1: base + 1.0 - h,
                    width: w,
                    color,
                    stroke: Stroke::Solid,
                    alpha,
                    text_bg,
                });
                out.push(Draw::Line {
                    x0: cx,
                    y0: base + 1.0 - h,
                    x1: cx + h * 0.6,
                    y1: base + 1.0,
                    width: w,
                    color,
                    stroke: Stroke::Solid,
                    alpha,
                    text_bg,
                });
            }
            continue;
        }
        if del_mark == Some(DeleteMark::Hash) {
            // Each deleted character shows as #.
            let gid = st.face.glyph_for('#');
            for g in &mut glyphs {
                g.0 = gid;
            }
        }
        let synth_bold = st.synth_bold || ins_mark == Some(InsertMark::Bold) || del_mark == Some(DeleteMark::Bold);
        let synth_italic = st.synth_italic || ins_mark == Some(InsertMark::Italic) || del_mark == Some(DeleteMark::Italic);
        // Text effects draw the letters from their outlines; the glyphs stay, invisible, for
        // the PDF's text. Not for colour glyphs (no outlines), top-to-bottom text with upright
        // characters (their outlines would lie on their side), `#` deletion marks, or WordArt
        // in revision colours.
        let upright = down && text.chars().any(upright_in_vertical);
        let art =
            (!upright && !glyphs.is_empty() && del_mark != Some(DeleteMark::Hash) && (rc.text_effects.is_none() || ins.is_none() && del.is_none()))
                .then(|| crate::wordart::run_effects(rc, color))
                .flatten()
                .filter(|_| !st.face.has_color_glyphs());
        let has_art = art.is_some();
        if let Some(mut fx) = art {
            if synth_bold {
                crate::wordart::embolden(&mut fx, color, st.size);
            }
            let path = crate::wordart::glyph_path(&st.face, st.size, &glyphs, synth_italic);
            out.push(Draw::Art { path, fx: Box::new(fx), color, alpha, text_bg });
        }
        if !glyphs.is_empty() {
            out.push(Draw::Glyphs {
                face: st.face,
                size: st.size,
                glyphs,
                color,
                alpha: if has_art { 0.0 } else { alpha },
                synth_bold,
                synth_italic,
                text,
                link: rc.link.clone(),
                ranges,
                text_bg,
                x_scale: rc.scale / 100.0,
            });
        }
        // Decorations across the run (underline skips trailing spaces of the line). Right-to-left
        // text inside a run can split it into several visual spans.
        let mut end_k = run_end;
        while end_k > run_start
            && pl.clusters.get(end_k - 1).is_some_and(|c| matches!(c.kind, ClKind::Space | ClKind::Marker | ClKind::LineBreak))
            && end_k == line.c1
        {
            end_k -= 1;
        }
        let thick = (st.size / 18.0).max(0.5);
        let underline = match (ins_mark, del_mark) {
            (Some(InsertMark::Underline), _) | (_, Some(DeleteMark::Underline)) => Underline::Single,
            (Some(InsertMark::DoubleUnderline), _) | (_, Some(DeleteMark::DoubleUnderline)) => Underline::Double,
            _ => rc.underline,
        };
        let double = rc.double_strike || del_mark == Some(DeleteMark::DoubleStrikethrough);
        let struck = rc.strike || double || ins_mark == Some(InsertMark::Strikethrough) || del_mark == Some(DeleteMark::Strikethrough);
        let spans = if underline != Underline::None || struck { line.spans(run_start, end_k) } else { Vec::new() };
        for (x0, x1) in spans.into_iter().map(|(a, b)| (x + a, x + b)).filter(|(a, b)| b > a) {
            if underline != Underline::None {
                let uy = base - st.shift + st.size * 0.12;
                let ucolor = rc.underline_color.unwrap_or(color);
                if underline == Underline::Words {
                    for kk in run_start..end_k {
                        let Some(c) = pl.clusters.get(kk) else { continue };
                        if c.kind != ClKind::Text {
                            continue;
                        }
                        let a = x + line.cl_left(kk).unwrap_or(0.0);
                        let b = x + line.cl_right(kk).unwrap_or(a - x);
                        if a < x0 - 0.01 || b > x1 + 0.01 {
                            continue; // drawn with its own span
                        }
                        out.push(Draw::Line { x0: a, y0: uy, x1: b, y1: uy, width: thick, color: ucolor, stroke: Stroke::Solid, alpha, text_bg });
                    }
                } else {
                    let w = if underline == Underline::Thick { thick * 2.0 } else { thick };
                    out.push(Draw::Line { x0, y0: uy, x1, y1: uy, width: w, color: ucolor, stroke: stroke_of(underline), alpha, text_bg });
                }
            }
            if struck {
                let sy = base - st.shift - st.size * 0.28;
                let stroke = if double { Stroke::Double } else { Stroke::Solid };
                out.push(Draw::Line { x0, y0: sy, x1, y1: sy, width: thick, color, stroke, alpha, text_bg });
            }
        }
    }
}

fn comment_backgrounds(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    let Some(para) = ctx.para.filter(|_| ctx.opts.markup && ctx.opts.revisions.comments) else { return };
    let mut open: Option<usize> = None;
    let mut ranges = Vec::new();
    for off in para.object_offsets() {
        match para.object_at(off) {
            Some(InlineObject::CommentStart { .. }) => open = open.or(Some(off)),
            Some(InlineObject::CommentEnd { .. }) => ranges.push((open.take().unwrap_or(0), off)),
            _ => {}
        }
    }
    if let Some(start) = open {
        ranges.push((start, para.len()));
    }
    for (start, end) in ranges {
        if end <= ctx.line.start || start >= ctx.line.stop {
            continue;
        }
        for (x0, x1) in ctx.pl.x_spans(ctx.li, start.max(ctx.line.start), end.min(ctx.line.stop)) {
            if x1 > x0 {
                out.push(Draw::Fill {
                    rect: Rect::new(ctx.x + x0, ctx.top, x1 - x0, ctx.line.height),
                    color: Rgb(0xEF, 0xE3, 0xF7),
                    alpha: ctx.alpha,
                    text_bg: false,
                });
            }
        }
    }
}

/// Paint highlights and shading before glyphs, grouping adjacent equal character borders.
fn character_backgrounds(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    let mut open: Option<(Border, f32, f32)> = None;
    for k in ctx.line.c0..ctx.line.c1 {
        let (Some(cluster), Some(left), Some(right)) = (ctx.pl.clusters.get(k), ctx.line.cl_left(k), ctx.line.cl_right(k)) else { continue };
        let Some(style) = ctx.pl.styles.get(cluster.style as usize) else { continue };
        if cluster.kind == ClKind::Marker {
            continue;
        }
        if let Some(color) = style.rc.highlight.or(style.rc.shading) {
            out.push(Draw::Fill {
                rect: Rect::new(ctx.x + left, ctx.top, (right - left).max(0.0), ctx.line.height),
                color,
                alpha: ctx.alpha,
                text_bg: true,
            });
        }
        let border = style.rc.border;
        if let Some((open_border, x0, x1)) = open.as_mut()
            && Some(*open_border) == border
        {
            *x0 = x0.min(ctx.x + left);
            *x1 = x1.max(ctx.x + right);
            continue;
        }
        if let Some((previous, x0, x1)) = open.take() {
            char_box(&previous, x0, x1, ctx.top, ctx.bottom, ctx.alpha, out);
        }
        open = border.map(|border| (border, ctx.x + left, ctx.x + right));
    }
    if let Some((border, x0, x1)) = open {
        char_box(&border, x0, x1, ctx.top, ctx.bottom, ctx.alpha, out);
    }
}

fn draw_label_and_hyphen(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    if ctx.li == ctx.pl.label_line()
        && let Some(label) = &ctx.pl.label
    {
        let gx = ctx.x + label.x;
        for run in &label.runs {
            let Some(style) = ctx.pl.styles.get(run.style as usize) else { continue };
            out.push(Draw::Glyphs {
                face: style.face,
                size: style.size,
                glyphs: label
                    .glyphs
                    .get(run.glyphs.clone())
                    .into_iter()
                    .flatten()
                    .map(|g| (g.gid, gx + g.dx, ctx.base - style.shift - g.dy))
                    .collect(),
                color: ctx.ink(&style.rc.color, None),
                alpha: ctx.alpha,
                synth_bold: style.synth_bold,
                synth_italic: style.synth_italic,
                text: run.text.clone(),
                link: None,
                ranges: Vec::new(),
                text_bg: None,
                x_scale: 1.0,
            });
        }
    }
    if let Some((style_index, glyph, advance)) = ctx.line.hyphen
        && let Some(style) = ctx.pl.styles.get(style_index as usize)
    {
        let hx = ctx.x + ctx.line.hyphen_x(advance);
        out.push(Draw::Glyphs {
            face: style.face,
            size: style.size,
            glyphs: vec![(glyph, hx, ctx.base - style.shift)],
            color: ctx.ink(&style.rc.color, None),
            alpha: ctx.alpha,
            synth_bold: style.synth_bold,
            synth_italic: style.synth_italic,
            text: "-".into(),
            link: None,
            ranges: Vec::new(),
            text_bg: None,
            x_scale: style.rc.scale / 100.0,
        });
    }
}

fn formatting_marks(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    let (doc, story, path, para, pl, line, x, base, opts, authors, alpha) =
        (ctx.doc, ctx.story, ctx.path, ctx.para, ctx.pl, ctx.line, ctx.x, ctx.base, ctx.opts, ctx.authors, ctx.alpha);
    let marks = opts.always.shown(opts.marks);
    if marks.any() {
        let msize = pl.styles.first().map(|s| s.size).unwrap_or(11.0);
        for k in line.c0..line.c1 {
            let Some(c) = pl.clusters.get(k) else { continue };
            let cx = x + line.cl_left(k).unwrap_or(0.0);
            let nx = x + line.cl_right(k).unwrap_or(cx - x);
            let size = pl.styles.get(c.style as usize).map(|s| s.size).unwrap_or(msize);
            let arrow = if line.rtl { '←' } else { '→' };
            match c.kind {
                ClKind::Space if marks.spaces => {
                    out.push(Draw::Mark { x: (cx + nx) / 2.0 - size * 0.12, baseline: base - size * 0.08, size, ch: '·', color: None })
                }
                ClKind::Tab if marks.tabs => {
                    out.push(Draw::Mark { x: cx + ((nx - cx) / 2.0 - size * 0.3).max(0.0), baseline: base, size, ch: arrow, color: None })
                }
                ClKind::LineBreak if marks.paragraphs => out.push(Draw::Mark { x: cx + 1.0, baseline: base, size, ch: '↵', color: None }),
                ClKind::PageBreak | ClKind::ColumnBreak if marks.paragraphs => {
                    let label = if c.kind == ClKind::PageBreak { '⤓' } else { '⇥' };
                    out.push(Draw::Line {
                        x0: cx + 2.0,
                        y0: base - size * 0.3,
                        x1: x + line.right,
                        y1: base - size * 0.3,
                        width: 0.5,
                        color: Rgb(0x60, 0x60, 0x60),
                        stroke: Stroke::Dotted,
                        alpha,
                        text_bg: None,
                    });
                    out.push(Draw::Mark { x: cx + 1.0, baseline: base, size, ch: label, color: None });
                }
                _ => {}
            }
        }
        // A deleted paragraph mark that isn't shown (no markup, or its mark is Hidden) draws no ¶.
        let mark_gone =
            para.is_some_and(|p| p.mark.del.is_some() && (!opts.markup || opts.revisions.hides_deletion(doc.move_name(p.mark.del).is_some())));
        if line.end == LineEnd::Para && marks.paragraphs && !mark_gone {
            // The mark follows the line's visual end, past every glyph, even where the
            // logically last text runs against the paragraph's direction.
            let ex = x + line.visual_end_x();
            let size = pl.lines.first().map(|_| msize).unwrap_or(msize);
            // A right-to-left paragraph's mark sits at its end, on the left.
            let mx = if line.rtl { ex - 1.0 - size * 0.6 } else { ex + 1.0 };
            // A tracked (inserted or deleted) paragraph mark is drawn in its author's colour.
            let m = &opts.revisions;
            let by_author = |r: u32| authors.color(r);
            let color = para.filter(|_| opts.markup && m.insertions_deletions).and_then(|p| match (p.mark.ins, p.mark.del) {
                (_, Some(r)) if doc.move_name(Some(r)).is_some() => Some(m.move_from_color.unwrap_or_else(|| by_author(r))),
                (Some(r), None) if doc.move_name(Some(r)).is_some() => Some(m.move_to_color.unwrap_or_else(|| by_author(r))),
                (_, Some(r)) => Some(m.delete_color.unwrap_or_else(|| by_author(r))),
                (Some(r), None) => Some(m.insert_color.unwrap_or_else(|| by_author(r))),
                _ => None,
            });
            // The last paragraph of a section ends in a section break instead of a plain ¶.
            if let Some(start) = section_break_after(doc, story, path) {
                let (x0, x1) = if line.rtl { (x + line.left, ex - 2.0) } else { (ex + 2.0, x + line.right) };
                section_break_mark(start, x0, x1, base, size, alpha, out);
            } else {
                out.push(Draw::Mark { x: mx, baseline: base, size, ch: '¶', color });
            }
        }
    }
}

fn draw_rubies(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    let (pl, line, opts, x, base, alpha) = (ctx.pl, ctx.line, ctx.opts, ctx.x, ctx.base, ctx.alpha);
    for r in pl.rubies.iter().filter(|r| (line.c0..line.c1).contains(&r.cluster)) {
        let Some(bst) = pl.clusters.get(r.cluster).and_then(|c| pl.styles.get(c.style as usize)) else { continue };
        if bst.rc.del.is_some() && !opts.markup {
            continue;
        }
        let cx = x + line.cl_left(r.cluster).unwrap_or(0.0);
        let y = base - bst.shift - r.raise;
        let mut i = 0;
        while let Some(&(style, _)) = r.glyphs.get(i) {
            let n = r.glyphs.get(i..).map_or(0, |g| g.iter().take_while(|(s, _)| *s == style).count()).max(1);
            let run = r.glyphs.get(i..i + n).unwrap_or(&[]);
            let ranges = r.ranges.get(i..i + n).map(<[_]>::to_vec).unwrap_or_default();
            i += n;
            let Some(st) = pl.styles.get(style as usize) else { continue };
            out.push(Draw::Glyphs {
                face: st.face,
                size: st.size,
                glyphs: run.iter().map(|(_, g)| (g.gid, cx + g.dx, y - g.dy)).collect(),
                color: ctx.ink(&st.rc.color, st.rc.shading.or(st.rc.highlight)),
                alpha,
                synth_bold: st.synth_bold,
                synth_italic: st.synth_italic,
                text: r.text.clone(),
                link: None,
                ranges,
                text_bg: st.rc.highlight.or(st.rc.shading),
                x_scale: 1.0,
            });
        }
    }
}

fn inline_objects(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    let (doc, story, path, pl, para, line, x, base, opts, alpha) =
        (ctx.doc, ctx.story, ctx.path, ctx.pl, ctx.para, ctx.line, ctx.x, ctx.base, ctx.opts, ctx.alpha);
    for k in line.c0..line.c1 {
        let Some(c) = pl.clusters.get(k) else { continue };
        let ClKind::Object(oi) = c.kind else { continue };
        if c.obj_h <= 0.0 {
            continue;
        }
        // A dragged inline object is left out too: its preview stands in for it.
        if opts.hide_object.as_ref().is_some_and(|p| p.story == story && p.path == *path && p.off == c.start) {
            continue;
        }
        let cx = x + line.cl_left(k).unwrap_or(0.0);
        let obj = para.and_then(|p| p.objects.get(oi));
        let rect = inline_rect(obj, cx, base, c.adv, c.obj_h);
        let spin = obj.and_then(InlineObject::frame).map(|(_, _, f)| f.spin()).unwrap_or_default();
        match obj {
            Some(o @ (InlineObject::Image { .. } | InlineObject::Shape { .. } | InlineObject::Group { .. })) => {
                object_draws(o, rect, Spin::default(), alpha, opts, out)
            }
            Some(InlineObject::Graphic { graphic, alt, .. }) => {
                let mut draws = Vec::new();
                spun(spin, rect, graphic_draws(doc, graphic, rect, alpha), &mut draws);
                out.push(Draw::Figure { alt: alt.clone(), kind: Some(graphic.kind), draws })
            }
            Some(InlineObject::Equation { .. }) => {
                if let Some((_, ml)) = pl.maths.iter().find(|(k, _)| *k == oi) {
                    equation(&ml.items, cx, base, alpha, opts.placeholders, out);
                }
            }
            Some(o @ InlineObject::Field { .. }) => {
                if let Some(wordcraft_doc::legacy_form::LegacyForm::CheckBox { checked, .. }) = o.legacy_form() {
                    let color = pl.styles.get(c.style as usize).map_or(Rgb::BLACK, |st| text_color(&st.rc.color, None));
                    let frame = Rect::new(cx, base - c.obj_h, c.adv, c.obj_h + c.obj_d);
                    check_box(frame, checked, color, alpha, out);
                }
            }
            _ => {}
        }
    }
}

fn proofing_marks(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    for (start, end, grammar) in &ctx.pl.issues {
        if *end <= ctx.line.start || *start >= ctx.line.stop {
            continue;
        }
        for (x0, x1) in ctx.pl.x_spans(ctx.li, (*start).max(ctx.line.start), (*end).min(ctx.line.stop)) {
            if x1 - x0 < 1.0 {
                continue;
            }
            let y = ctx.base + 2.5;
            let color = if *grammar { Rgb(0x2B, 0x57, 0xC0) } else { Rgb(0xE0, 0x24, 0x24) };
            out.push(Draw::Line {
                x0: ctx.x + x0,
                y0: y,
                x1: ctx.x + x1,
                y1: y,
                width: 0.8,
                color,
                stroke: if *grammar { Stroke::Double } else { Stroke::Wave },
                alpha: ctx.alpha,
                text_bg: None,
            });
        }
    }
}

fn tab_leaders(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    for (k, leader) in &ctx.line.leaders {
        let (Some(cluster), Some(left), Some(right)) = (ctx.pl.clusters.get(*k), ctx.line.cl_left(*k), ctx.line.cl_right(*k)) else { continue };
        let Some(style) = ctx.pl.styles.get(cluster.style as usize) else { continue };
        let Some(ch) = leader.char() else { continue };
        let gid = style.face.glyph_for(ch);
        let advance = (style.face.advance(gid) as f32 * style.size / style.face.upem.max(1.0) as f32).max(1.0);
        let (mut gx, end) = (ctx.x + left + advance * 0.5, ctx.x + right - advance * 1.2);
        let mut glyphs = Vec::new();
        gx = (gx / advance).ceil() * advance;
        while gx < end && glyphs.len() < 2000 {
            glyphs.push((gid, gx, ctx.base));
            gx += advance;
        }
        out.push(Draw::Glyphs {
            face: style.face,
            size: style.size,
            glyphs,
            color: ctx.ink(&style.rc.color, None),
            alpha: ctx.alpha,
            synth_bold: false,
            synth_italic: false,
            text: String::new(),
            link: None,
            ranges: Vec::new(),
            text_bg: None,
            x_scale: 1.0,
        });
    }
}

fn enclosures(ctx: &LineContext<'_, '_, '_>, out: &mut Vec<Draw>) {
    for (k, enclosure) in &ctx.pl.enclosures {
        if *k < ctx.line.c0 || *k >= ctx.line.c1 {
            continue;
        }
        let (Some(cluster), Some(left)) = (ctx.pl.clusters.get(*k), ctx.line.cl_left(*k)) else { continue };
        let Some(style) = ctx.pl.styles.get(cluster.style as usize) else { continue };
        let rect = Rect::new(ctx.x + left + enclosure.x, ctx.base - style.shift - enclosure.top, enclosure.w, enclosure.h);
        out.push(Draw::Shape {
            rect,
            kind: enclosure.kind,
            fill: None,
            stroke: Some(ctx.ink(&style.rc.color, None)),
            stroke_width: enclosure.stroke,
            effects: Default::default(),
            corner: None,
            dash: Vec::new(),
            cap: Default::default(),
        });
    }
}

/// The kind of section break that ends at body paragraph `path`: the start type of the section
/// that follows it. `None` for any other paragraph, and for the document's last section.
fn section_break_after(doc: &Document, story: StoryRef, path: &Path) -> Option<SectionStart> {
    if story != StoryRef::Body || path.0.len() != 1 {
        return None;
    }
    let i = *path.0.first()? as usize;
    match doc.body.get(i).map(|b| &**b) {
        Some(Block::Para(p)) if p.section.is_some() => {}
        _ => return None,
    }
    let secs = doc.sections();
    let k = secs.iter().position(|(end, _)| *end == i)?;
    secs.get(k + 1).map(|(_, s)| s.start)
}

/// On-screen name of a section break.
pub fn section_break_label(start: SectionStart) -> &'static str {
    match start {
        SectionStart::NextPage => "Section Break (Next Page)",
        SectionStart::Continuous => "Section Break (Continuous)",
        SectionStart::EvenPage => "Section Break (Even Page)",
        SectionStart::OddPage => "Section Break (Odd Page)",
        SectionStart::NextColumn => "Section Break (Next Column)",
    }
}

/// Width of a [`Draw::MarkText`] label.
pub fn mark_text_width(text: &str, size: f32) -> f32 {
    let face = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    let k = size / face.upem.max(1.0) as f32;
    text.chars().map(|c| face.advance(face.glyph_for(c)) as f32 * k).sum()
}

/// A section break mark between `x0` and `x1`: a dotted double rule with the break's name centred
/// in it (just the name when there is no room for the rule).
fn section_break_mark(start: SectionStart, x0: f32, x1: f32, base: f32, size: f32, alpha: f32, out: &mut Vec<Draw>) {
    let text = section_break_label(start);
    let lsize = (size * 0.8).clamp(6.0, 11.0);
    let w = mark_text_width(text, lsize);
    let gap = lsize * 0.4;
    let mid = base - size * 0.3;
    let color = Rgb(0x60, 0x60, 0x60);
    if x1 - x0 <= w + 4.0 * gap {
        out.push(Draw::MarkText { x: x0, baseline: base, size: lsize, text: text.into() });
        return;
    }
    let tx = (x0 + x1 - w) / 2.0;
    for (a, b) in [(x0, tx - gap), (tx + w + gap, x1)] {
        for y in [mid - 1.2, mid + 1.2] {
            out.push(Draw::Line { x0: a, y0: y, x1: b, y1: y, width: 0.5, color, stroke: Stroke::Dotted, alpha, text_bg: None });
        }
    }
    out.push(Draw::MarkText { x: tx, baseline: mid + lsize * 0.33, size: lsize, text: text.into() });
}

/// An equation's glyphs and rules with its origin at (`x`, `base`).
fn equation(items: &[MItem], x: f32, base: f32, alpha: f32, screen: bool, out: &mut Vec<Draw>) {
    for it in items {
        match it {
            MItem::Glyphs { face, size, color, synth_bold, synth_italic, glyphs, text } => out.push(Draw::Glyphs {
                face: *face,
                size: *size,
                glyphs: glyphs.iter().map(|(g, gx, gy)| (*g, x + gx, base - gy)).collect(),
                color: *color,
                alpha,
                synth_bold: *synth_bold,
                synth_italic: *synth_italic,
                text: text.clone(),
                link: None,
                // Equation text maps back through the font (no per-glyph ranges from the math layout).
                ranges: Vec::new(),
                text_bg: None,
                x_scale: 1.0,
            }),
            MItem::Rect { x: rx, y, w, h, color } => {
                out.push(Draw::Fill { rect: Rect::new(x + rx, base - y - h, *w, *h), color: *color, alpha, text_bg: false })
            }
            MItem::Line { x0, y0, x1, y1, width, color, dotted } => out.push(Draw::Line {
                x0: x + x0,
                y0: base - y0,
                x1: x + x1,
                y1: base - y1,
                width: *width,
                color: *color,
                stroke: if *dotted { Stroke::Dotted } else { Stroke::Solid },
                alpha,
                text_bg: None,
            }),
            MItem::ScreenOnly(inner) => {
                if screen {
                    equation(std::slice::from_ref(inner), x, base, alpha, screen, out);
                }
            }
        }
    }
}
