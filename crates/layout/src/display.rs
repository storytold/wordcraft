//! Page → draw items, shared by the raster renderer, the PDF exporter and thumbnails.

use wordcraft_doc::para::{InlineObject, ShapeKind};
use wordcraft_doc::props::{Border, BorderStyle, Rgb, TextColor, TextDirection, Underline};
use wordcraft_doc::{Document, Path, StoryRef};
use wordcraft_fonts::FaceRef;
use wordcraft_geom::Rect;

use crate::math::MItem;
use crate::para::{ClKind, LineEnd, ParaLayout};
use crate::{Page, Placed};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stroke {
    Solid,
    Dotted,
    Dashed,
    Double,
    Wave,
}

#[derive(Clone, Debug)]
pub enum Draw {
    Glyphs {
        face: FaceRef,
        size: f32,
        /// Glyph id, x, baseline y (page coordinates).
        glyphs: Vec<(u32, f32, f32)>,
        color: Rgb,
        alpha: f32,
        synth_bold: bool,
        synth_italic: bool,
        /// The text these glyphs show (for PDF text extraction), and the hyperlink.
        text: String,
        link: Option<String>,
        /// Byte range of `text` each glyph shows (one per glyph; a ligature's range covers all
        /// its characters). Empty when unknown: the PDF writer then guesses from the font.
        ranges: Vec<std::ops::Range<usize>>,
    },
    Fill {
        rect: Rect,
        color: Rgb,
        alpha: f32,
    },
    Line {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: Rgb,
        stroke: Stroke,
        alpha: f32,
    },
    Image {
        rect: Rect,
        media: String,
        crop: [f32; 4],
        alpha: f32,
    },
    Shape {
        rect: Rect,
        kind: ShapeKind,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
        stroke_width: f32,
        /// Shadow, glow and soft edges (drawn with [`wordcraft_doc::effects::bands`]).
        effects: wordcraft_doc::effects::ShapeEffects,
    },
    /// A formatting mark (¶ · → ↵) in the UI's mark colour, or in `color` (a tracked
    /// paragraph mark in its reviser's colour).
    Mark {
        x: f32,
        baseline: f32,
        size: f32,
        ch: char,
        color: Option<Rgb>,
    },
    /// Turned text (a table cell's text direction): `items` are drawn in a frame turned `turn`
    /// whose origin is page point (`x`, `y`) (see [`crate::turn_point`]).
    Turned {
        x: f32,
        y: f32,
        turn: TextDirection,
        items: Vec<Draw>,
    },
}

impl Draw {
    /// The affine map (a, b, c, d, e, f: x' = a·x + c·y + e, y' = b·x + d·y + f) from a frame
    /// turned `turn` with its origin at page point (`x`, `y`) to the page.
    pub fn turn_matrix(turn: TextDirection, x: f32, y: f32) -> [f32; 6] {
        match turn {
            TextDirection::Horizontal => [1.0, 0.0, 0.0, 1.0, x, y],
            TextDirection::Down => [0.0, 1.0, -1.0, 0.0, x, y],
            TextDirection::Up => [0.0, -1.0, 1.0, 0.0, x, y],
        }
    }
}

#[derive(Clone, Debug)]
pub struct DisplayOptions {
    /// Show formatting marks (¶).
    pub marks: bool,
    /// Dim headers/footers (editing the body) or the body (editing a header/footer).
    pub dim_header: bool,
    pub dim_body: bool,
    /// Show tracked changes as markup (coloured, underlined/struck).
    pub markup: bool,
    /// Show on-screen-only marks: equation placeholders and prompts (never in print or PDF).
    pub placeholders: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        DisplayOptions { marks: false, dim_header: true, dim_body: false, markup: true, placeholders: false }
    }
}

/// Colour for revisions by author index.
pub fn revision_color(i: u32) -> Rgb {
    const C: [Rgb; 6] =
        [Rgb(0xB0, 0x1E, 0x8F), Rgb(0x1E, 0x6E, 0xB0), Rgb(0x2E, 0x8B, 0x3E), Rgb(0xC0, 0x5A, 0x10), Rgb(0x70, 0x3C, 0xB0), Rgb(0x0E, 0x7C, 0x86)];
    C.get(i as usize % C.len()).copied().unwrap_or(C[0])
}

fn stroke_of(u: Underline) -> Stroke {
    match u {
        Underline::Dotted => Stroke::Dotted,
        Underline::Dash | Underline::DotDash | Underline::DotDotDash => Stroke::Dashed,
        Underline::Double => Stroke::Double,
        Underline::Wave | Underline::DoubleWave => Stroke::Wave,
        _ => Stroke::Solid,
    }
}

/// Draw items for a page.
pub fn page_display(doc: &Document, page: &Page, opts: &DisplayOptions) -> Vec<Draw> {
    let mut out = Vec::new();
    let ha = if opts.dim_header { 0.45 } else { 1.0 };
    let ba = if opts.dim_body { 0.45 } else { 1.0 };
    for it in &page.header {
        item(doc, it, opts, ha, &mut out);
    }
    for it in &page.footer {
        item(doc, it, opts, ha, &mut out);
    }
    for it in &page.items {
        item(doc, it, opts, ba, &mut out);
    }
    out
}

fn border_stroke(s: BorderStyle) -> Stroke {
    match s {
        BorderStyle::Dotted => Stroke::Dotted,
        BorderStyle::Dashed | BorderStyle::DotDash => Stroke::Dashed,
        BorderStyle::Double | BorderStyle::Triple => Stroke::Double,
        BorderStyle::Wave => Stroke::Wave,
        _ => Stroke::Solid,
    }
}

/// A border line from `(x0, y0)` to `(x1, y1)` (paragraph rules and character borders).
fn rule(x0: f32, y0: f32, x1: f32, y1: f32, b: &Border, alpha: f32) -> Draw {
    Draw::Line {
        x0,
        y0,
        x1,
        y1,
        width: if b.style == BorderStyle::Thick { b.width.max(1.5) } else { b.width.max(0.25) },
        color: b.color.unwrap_or(Rgb::BLACK),
        stroke: border_stroke(b.style),
        alpha,
    }
}

/// Closed box around one line segment of a character-border group, widened by `space`.
// Note: border drawn outside text advance, reserve width in line breaking if overlap shows.
fn char_box(b: &Border, x0: f32, x1: f32, top: f32, bottom: f32, alpha: f32, out: &mut Vec<Draw>) {
    let (l, r) = (x0 - b.space, x1 + b.space);
    out.push(rule(l, top, r, top, b, alpha));
    out.push(rule(l, bottom, r, bottom, b, alpha));
    out.push(rule(l, top, l, bottom, b, alpha));
    out.push(rule(r, top, r, bottom, b, alpha));
}

fn item(doc: &Document, it: &Placed, opts: &DisplayOptions, alpha: f32, out: &mut Vec<Draw>) {
    match it {
        Placed::Fill { rect, color } => out.push(Draw::Fill { rect: *rect, color: *color, alpha }),
        Placed::Rule { x0, y0, x1, y1, border } => out.push(rule(*x0, *y0, *x1, *y1, border, alpha)),
        Placed::Image { rect, media, crop, .. } => out.push(Draw::Image { rect: *rect, media: media.clone(), crop: *crop, alpha }),
        Placed::Shape { rect, kind, fill, stroke, stroke_width, effects } => {
            out.push(Draw::Shape { rect: *rect, kind: *kind, fill: *fill, stroke: *stroke, stroke_width: *stroke_width, effects: *effects })
        }
        Placed::Cell { .. } | Placed::Object { .. } => {}
        Placed::Lines { story, path, para, l0, l1, x, y, turn } if turn.is_turned() => {
            let mut items = Vec::new();
            lines(doc, *story, path, para, *l0, *l1, 0.0, 0.0, opts, alpha, &mut items);
            // Link areas are axis-aligned page rectangles: turned text gets none.
            for d in &mut items {
                if let Draw::Glyphs { link, .. } = d {
                    *link = None;
                }
            }
            out.push(Draw::Turned { x: *x, y: *y, turn: *turn, items });
        }
        Placed::Lines { story, path, para, l0, l1, x, y, .. } => lines(doc, *story, path, para, *l0, *l1, *x, *y, opts, alpha, out),
    }
}

fn lines(
    doc: &Document,
    story: StoryRef,
    path: &Path,
    pl: &ParaLayout,
    l0: usize,
    l1: usize,
    x: f32,
    y: f32,
    opts: &DisplayOptions,
    alpha: f32,
    out: &mut Vec<Draw>,
) {
    let Some(first) = pl.lines.get(l0) else { return };
    let para = doc.para(story, path);
    for li in l0..l1 {
        let Some(line) = pl.lines.get(li) else { continue };
        let top = y + (line.top - first.top);
        let base = top + (line.baseline - line.top);
        let bottom = top + line.height;
        // Commented text gets a soft shade (comment anchors are object markers).
        if opts.markup
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
                        out.push(Draw::Fill { rect: Rect::new(x + x0, top, x1 - x0, line.height), color: Rgb(0xEF, 0xE3, 0xF7), alpha });
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
                out.push(Draw::Fill { rect: Rect::new(x + cx, top, (nx - cx).max(0.0), line.height), color: h, alpha });
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
        if li == 0
            && let Some(lab) = &pl.label
            && let Some(st) = pl.styles.get(lab.style as usize)
        {
            let gx = x + lab.x;
            out.push(Draw::Glyphs {
                face: st.face,
                size: st.size,
                glyphs: lab.glyphs.iter().map(|g| (g.gid, gx + g.dx, base - st.shift - g.dy)).collect(),
                color: text_color(&st.rc.color, None),
                alpha,
                synth_bold: st.synth_bold,
                synth_italic: st.synth_italic,
                text: lab.text.clone(),
                link: None,
                ranges: Vec::new(),
            });
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
                color: text_color(&st.rc.color, None),
                alpha,
                synth_bold: st.synth_bold,
                synth_italic: st.synth_italic,
                text: "-".into(),
                link: None,
                ranges: Vec::new(),
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
            let mut glyphs = Vec::new();
            let mut text = String::new();
            let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
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
                    let mut shown = false;
                    if let Ok(i) = pl.shown.binary_search_by_key(&k, |(i, _)| *i)
                        && let Some((_, s)) = pl.shown.get(i)
                    {
                        text.push_str(s);
                        shown = true;
                    } else if let Some(p) = para {
                        text.push_str(p.text.get(c.start..c.end).unwrap_or(""));
                    }
                    let b = text.len();
                    let cg = pl.glyphs.get(c.g0 as usize..c.g1 as usize).unwrap_or(&[]);
                    if shown && cg.len() > 1 {
                        // A whole field result is one cluster of many glyphs: give them a character
                        // each (the last takes any remainder), not the whole text as one ligature.
                        let bounds: Vec<usize> = text.get(a..b).unwrap_or("").char_indices().map(|(i, _)| a + i).collect();
                        for (n, g) in cg.iter().enumerate() {
                            let from = bounds.get(n).or(bounds.last()).copied().unwrap_or(a);
                            let to = if n + 1 == cg.len() { b } else { bounds.get(n + 1).copied().unwrap_or(b) };
                            glyphs.push((g.gid, cx + g.dx, base - st.shift - g.dy));
                            ranges.push(from..to.max(from));
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
            // Without markup a deletion is never drawn as ordinary text, even in a layout that kept it.
            if rc.del.is_some() && !opts.markup {
                continue;
            }
            let rev = rc.ins.or(rc.del).filter(|_| opts.markup);
            let color = match rev {
                Some(r) => revision_color(doc.revisions.get(r as usize).map(|v| author_index(doc, &v.author)).unwrap_or(0)),
                None => text_color(&rc.color, rc.shading.or(rc.highlight)),
            };
            if !glyphs.is_empty() {
                out.push(Draw::Glyphs {
                    face: st.face,
                    size: st.size,
                    glyphs,
                    color,
                    alpha,
                    synth_bold: st.synth_bold,
                    synth_italic: st.synth_italic,
                    text,
                    link: rc.link.clone(),
                    ranges,
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
            let underline = if rc.ins.is_some() && opts.markup { Underline::Single } else { rc.underline };
            let struck = rc.strike || rc.double_strike || (rc.del.is_some() && opts.markup);
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
                            out.push(Draw::Line { x0: a, y0: uy, x1: b, y1: uy, width: thick, color: ucolor, stroke: Stroke::Solid, alpha });
                        }
                    } else {
                        let w = if underline == Underline::Thick { thick * 2.0 } else { thick };
                        out.push(Draw::Line { x0, y0: uy, x1, y1: uy, width: w, color: ucolor, stroke: stroke_of(underline), alpha });
                    }
                }
                if struck {
                    let sy = base - st.shift - st.size * 0.28;
                    let stroke = if rc.double_strike { Stroke::Double } else { Stroke::Solid };
                    out.push(Draw::Line { x0, y0: sy, x1, y1: sy, width: thick, color, stroke, alpha });
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
                color: text_color(&st.rc.color, None),
                alpha,
                synth_bold: false,
                synth_italic: false,
                text: String::new(),
                link: None,
                ranges: Vec::new(),
            });
        }
        // Inline objects.
        for k in line.c0..line.c1 {
            let Some(c) = pl.clusters.get(k) else { continue };
            let ClKind::Object(oi) = c.kind else { continue };
            if c.obj_h <= 0.0 {
                continue;
            }
            let cx = x + line.cl_left(k).unwrap_or(0.0);
            let obj = para.and_then(|p| p.objects.get(oi));
            let rect = inline_rect(obj, cx, base, c.adv, c.obj_h);
            match obj {
                Some(InlineObject::Image { media, crop, .. }) => out.push(Draw::Image { rect, media: media.clone(), crop: *crop, alpha }),
                Some(InlineObject::Shape { kind, fill, stroke, stroke_width, effects, .. }) => {
                    out.push(Draw::Shape { rect, kind: *kind, fill: *fill, stroke: *stroke, stroke_width: *stroke_width, effects: *effects })
                }
                Some(InlineObject::Equation { .. }) => {
                    if let Some((_, ml)) = pl.maths.iter().find(|(k, _)| *k == oi) {
                        equation(&ml.items, cx, base, alpha, opts.placeholders, out);
                    }
                }
                _ => {}
            }
        }
        // Formatting marks.
        if opts.marks {
            let msize = pl.styles.first().map(|s| s.size).unwrap_or(11.0);
            for k in line.c0..line.c1 {
                let Some(c) = pl.clusters.get(k) else { continue };
                let cx = x + line.cl_left(k).unwrap_or(0.0);
                let nx = x + line.cl_right(k).unwrap_or(cx - x);
                let size = pl.styles.get(c.style as usize).map(|s| s.size).unwrap_or(msize);
                let arrow = if line.rtl { '←' } else { '→' };
                match c.kind {
                    ClKind::Space => {
                        out.push(Draw::Mark { x: (cx + nx) / 2.0 - size * 0.12, baseline: base - size * 0.08, size, ch: '·', color: None })
                    }
                    ClKind::Tab => {
                        out.push(Draw::Mark { x: cx + ((nx - cx) / 2.0 - size * 0.3).max(0.0), baseline: base, size, ch: arrow, color: None })
                    }
                    ClKind::LineBreak => out.push(Draw::Mark { x: cx + 1.0, baseline: base, size, ch: '↵', color: None }),
                    ClKind::PageBreak | ClKind::ColumnBreak => {
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
                        });
                        out.push(Draw::Mark { x: cx + 1.0, baseline: base, size, ch: label, color: None });
                    }
                    _ => {}
                }
            }
            if line.end == LineEnd::Para {
                let ex = x + line.end_x();
                let size = pl.lines.first().map(|_| msize).unwrap_or(msize);
                // A right-to-left paragraph's mark sits at its end, on the left.
                let mx = if line.rtl { ex - 1.0 - size * 0.6 } else { ex + 1.0 };
                // A tracked (inserted or deleted) paragraph mark is drawn in its author's colour.
                let color = para
                    .and_then(|p| p.mark.ins.or(p.mark.del))
                    .filter(|_| opts.markup)
                    .map(|r| revision_color(doc.revisions.get(r as usize).map(|v| author_index(doc, &v.author)).unwrap_or(0)));
                out.push(Draw::Mark { x: mx, baseline: base, size, ch: '¶', color });
            }
        }
        let _ = bottom;
    }
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
            }),
            MItem::Rect { x: rx, y, w, h, color } => out.push(Draw::Fill { rect: Rect::new(x + rx, base - y - h, *w, *h), color: *color, alpha }),
            MItem::Line { x0, y0, x1, y1, width, color, dotted } => out.push(Draw::Line {
                x0: x + x0,
                y0: base - y0,
                x1: x + x1,
                y1: base - y1,
                width: *width,
                color: *color,
                stroke: if *dotted { Stroke::Dotted } else { Stroke::Solid },
                alpha,
            }),
            MItem::ScreenOnly(inner) => {
                if screen {
                    equation(std::slice::from_ref(inner), x, base, alpha, screen, out);
                }
            }
        }
    }
}

fn author_index(doc: &Document, author: &str) -> u32 {
    let mut seen: Vec<&str> = Vec::new();
    for r in &doc.revisions {
        if !seen.contains(&r.author.as_str()) {
            seen.push(&r.author);
        }
    }
    seen.iter().position(|a| *a == author).unwrap_or(0) as u32
}

/// Automatic colour is black, or white on a dark background.
pub fn text_color(c: &TextColor, background: Option<Rgb>) -> Rgb {
    match c {
        TextColor::Rgb(r) => *r,
        TextColor::Auto => {
            if background.is_some_and(|b| b.luma() < 100.0) {
                Rgb::WHITE
            } else {
                Rgb::BLACK
            }
        }
    }
}

/// Where inline object `obj` is drawn, given its cluster's box (`adv` × `obj_h` standing on the
/// baseline at `cx`): inside the room kept for its effects.
pub(crate) fn inline_rect(obj: Option<&InlineObject>, cx: f32, base: f32, adv: f32, obj_h: f32) -> Rect {
    let [l, t, r, b] = match obj {
        Some(InlineObject::Image { float, .. } | InlineObject::Shape { float, .. }) => float.effect_extent(),
        _ => [0.0; 4],
    };
    Rect::new(cx + l, base - obj_h + t, (adv - l - r).max(0.0), (obj_h - t - b).max(0.0))
}
