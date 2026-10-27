use super::glyphs::{Authors, check_box, text_color, upright_in_vertical};
use super::objects::{graphic_draws, inline_rect, object_draws, spun};
use super::options::DisplayOptions;
use super::page::{char_box, stroke_of};
use super::paint::Stroke;
use super::*;

/// Append display items for the half-open line range `l0..l1` of a laid-out paragraph.
/// `(x, y)` positions the paragraph's left origin and the first selected line's top; later lines
/// retain their layout offsets. Includes glyphs, inline objects, decorations and requested markup.
/// `down` keeps vertical East Asian glyphs upright before the caller turns the resulting draws.
pub(super) fn lines(
    doc: &Document,
    story: StoryRef,
    path: &Path,
    pl: &ParaLayout,
    l0: usize,
    l1: usize,
    x: f32,
    y: f32,
    opts: &DisplayOptions,
    authors: &Authors,
    alpha: f32,
    down: bool,
    out: &mut Vec<Draw>,
) {
    let Some(first) = pl.lines.get(l0) else { return };
    let para = doc.para(story, path);
    // Inside a text box whose shape style gives its text a colour, that is what automatic means.
    let auto = match story {
        StoryRef::Part(id) => doc.parts.get(&id).and_then(|p| p.body.auto_color),
        StoryRef::Body => None,
    };
    let ink = |c: &TextColor, background: Option<Rgb>| match (c, auto) {
        (TextColor::Auto, Some(rgb)) => rgb,
        _ => text_color(c, background),
    };
    for li in l0..l1 {
        let Some(line) = pl.lines.get(li) else { continue };
        let top = y + (line.top - first.top);
        let base = top + (line.baseline - line.top);
        let bottom = top + line.height;
        // Commented text gets a soft shade (comment anchors are object markers).
        if opts.markup
            && opts.revisions.comments
            && let Some(p) = para
        {
            let mut open: Option<usize> = None;
            let mut ranges = Vec::new();
            for off in p.object_offsets() {
                match p.object_at(off) {
                    Some(InlineObject::CommentStart { .. }) => open = open.or(Some(off)),
                    Some(InlineObject::CommentEnd { .. }) => {
                        ranges.push((open.take().unwrap_or(0), off));
                    }
                    _ => {}
                }
            }
            if let Some(o) = open {
                ranges.push((o, p.len()));
            }
            for (a, b) in ranges {
                if b <= line.start || a >= line.stop {
                    continue;
                }
                for (x0, x1) in pl.x_spans(li, a.max(line.start), b.min(line.stop)) {
                    if x1 > x0 {
                        out.push(Draw::Fill {
                            rect: Rect::new(x + x0, top, x1 - x0, line.height),
                            color: Rgb(0xEF, 0xE3, 0xF7),
                            alpha,
                            text_bg: false,
                        });
                    }
                }
            }
        }
        // Backgrounds first: highlight and character shading; character borders group equal adjacent borders.
        let mut open: Option<(Border, f32, f32)> = None;
        for k in line.c0..line.c1 {
            let (Some(c), Some(cx), Some(nx)) = (pl.clusters.get(k), line.cl_left(k), line.cl_right(k)) else { continue };
            let Some(st) = pl.styles.get(c.style as usize) else { continue };
            if c.kind == ClKind::Marker {
                continue;
            }
            if let Some(h) = st.rc.highlight.or(st.rc.shading) {
                out.push(Draw::Fill { rect: Rect::new(x + cx, top, (nx - cx).max(0.0), line.height), color: h, alpha, text_bg: true });
            }
            let b = st.rc.border;
            if let Some((ob, x0, x1)) = open.as_mut()
                && Some(*ob) == b
            {
                // Right-to-left text grows leftwards.
                *x0 = x0.min(x + cx);
                *x1 = x1.max(x + nx);
                continue;
            }
            if let Some((ob, x0, x1)) = open.take() {
                char_box(&ob, x0, x1, top, bottom, alpha, out);
            }
            open = b.map(|b| (b, x + cx, x + nx));
        }
        if let Some((ob, x0, x1)) = open {
            char_box(&ob, x0, x1, top, bottom, alpha, out);
        }
        // Label.
        if li == pl.label_line()
            && let Some(lab) = &pl.label
        {
            let gx = x + lab.x;
            // One draw per font run: a glyph id only means something in its own face.
            for run in &lab.runs {
                let Some(st) = pl.styles.get(run.style as usize) else { continue };
                out.push(Draw::Glyphs {
                    face: st.face,
                    size: st.size,
                    glyphs: lab.glyphs.get(run.glyphs.clone()).into_iter().flatten().map(|g| (g.gid, gx + g.dx, base - st.shift - g.dy)).collect(),
                    color: ink(&st.rc.color, None),
                    alpha,
                    synth_bold: st.synth_bold,
                    synth_italic: st.synth_italic,
                    text: run.text.clone(),
                    link: None,
                    ranges: Vec::new(),
                    text_bg: None,
                    x_scale: 1.0,
                });
            }
        }
        // Hyphen at a hyphenated line end.
        if let Some((si, gid, adv)) = line.hyphen
            && let Some(st) = pl.styles.get(si as usize)
        {
            let hx = x + line.hyphen_x(adv);
            out.push(Draw::Glyphs {
                face: st.face,
                size: st.size,
                glyphs: vec![(gid, hx, base - st.shift)],
                color: ink(&st.rc.color, None),
                alpha,
                synth_bold: st.synth_bold,
                synth_italic: st.synth_italic,
                text: "-".into(),
                link: None,
                ranges: Vec::new(),
                text_bg: None,
                x_scale: st.rc.scale / 100.0,
            });
        }
        // Glyph runs grouped by style.
        let mut k = line.c0;
        while k < line.c1 {
            let Some(c) = pl.clusters.get(k) else { break };
            let si = c.style;
            let Some(st) = pl.styles.get(si as usize) else {
                k += 1;
                continue;
            };
            // Sized for the run's text clusters up front: no growing per glyph.
            let run = pl.clusters.get(k..line.c1.min(pl.clusters.len())).unwrap_or_default();
            let (n_glyphs, n_bytes) = run
                .iter()
                .take_while(|c| c.style == si)
                .filter(|c| matches!(c.kind, ClKind::Text | ClKind::Space))
                .fold((0usize, 0usize), |(g, b), c| (g + c.g1.saturating_sub(c.g0) as usize, b + c.end.saturating_sub(c.start)));
            let mut glyphs = Vec::with_capacity(n_glyphs);
            let mut text = String::with_capacity(n_bytes);
            let mut ranges: Vec<std::ops::Range<usize>> = Vec::with_capacity(n_glyphs);
            let symbol_font = wordcraft_doc::symbol_fonts::is_symbol_font(&st.rc.font);
            let run_start = k;
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
            let run_end = k;
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
                _ => ink(&rc.color, rc.shading.or(rc.highlight)),
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
            let art = (!upright
                && !glyphs.is_empty()
                && del_mark != Some(DeleteMark::Hash)
                && (rc.text_effects.is_none() || ins.is_none() && del.is_none()))
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
        // Proofing squiggles.
        for (a, b, grammar) in &pl.issues {
            if *b <= line.start || *a >= line.stop {
                continue;
            }
            for (x0, x1) in pl.x_spans(li, (*a).max(line.start), (*b).min(line.stop)) {
                if x1 - x0 < 1.0 {
                    continue;
                }
                let y = base + 2.5;
                let color = if *grammar { Rgb(0x2B, 0x57, 0xC0) } else { Rgb(0xE0, 0x24, 0x24) };
                out.push(Draw::Line {
                    x0: x + x0,
                    y0: y,
                    x1: x + x1,
                    y1: y,
                    width: 0.8,
                    color,
                    stroke: if *grammar { Stroke::Double } else { Stroke::Wave },
                    alpha,
                    text_bg: None,
                });
            }
        }
        // Tab leaders.
        for (k, leader) in &line.leaders {
            let (Some(c), Some(a), Some(b)) = (pl.clusters.get(*k), line.cl_left(*k), line.cl_right(*k)) else { continue };
            let Some(st) = pl.styles.get(c.style as usize) else { continue };
            let Some(ch) = leader.char() else { continue };
            let gid = st.face.glyph_for(ch);
            let adv = (st.face.advance(gid) as f32 * st.size / st.face.upem.max(1.0) as f32).max(1.0);
            let (mut gx, end) = (x + a + adv * 0.5, x + b - adv * 1.2);
            let mut glyphs = Vec::new();
            // Align leader dots to a grid so consecutive lines line up.
            gx = (gx / adv).ceil() * adv;
            while gx < end && glyphs.len() < 2000 {
                glyphs.push((gid, gx, base));
                gx += adv;
            }
            out.push(Draw::Glyphs {
                face: st.face,
                size: st.size,
                glyphs,
                color: ink(&st.rc.color, None),
                alpha,
                synth_bold: false,
                synth_italic: false,
                text: String::new(),
                link: None,
                ranges: Vec::new(),
                text_bg: None,
                x_scale: 1.0,
            });
        }
        // Inline objects.
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
        // Enclosed characters' shapes, in the text's colour.
        for (k, e) in &pl.enclosures {
            if *k < line.c0 || *k >= line.c1 {
                continue;
            }
            let (Some(c), Some(left)) = (pl.clusters.get(*k), line.cl_left(*k)) else { continue };
            let Some(st) = pl.styles.get(c.style as usize) else { continue };
            let rect = Rect::new(x + left + e.x, base - st.shift - e.top, e.w, e.h);
            out.push(Draw::Shape {
                rect,
                kind: e.kind,
                fill: None,
                stroke: Some(ink(&st.rc.color, None)),
                stroke_width: e.stroke,
                effects: Default::default(),
                corner: None,
                dash: Vec::new(),
                cap: Default::default(),
            });
        }
        // Ruby (Phonetic Guide) text over its base text, in runs of one style.
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
                    color: ink(&st.rc.color, st.rc.shading.or(st.rc.highlight)),
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
        // Formatting marks.
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
        let _ = bottom;
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
