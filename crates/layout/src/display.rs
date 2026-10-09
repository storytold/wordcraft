//! Page → draw items, shared by the raster renderer, the PDF exporter and thumbnails.

use wordcraft_doc::para::{InlineObject, ShapeKind};
use wordcraft_doc::props::{Border, BorderStyle, Rgb, TextColor, Underline};
use wordcraft_doc::{Document, Path, StoryRef};
use wordcraft_fonts::FaceRef;
use wordcraft_geom::Rect;

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
    },
    /// A formatting mark (¶ · → ↵) in the UI's mark colour.
    Mark {
        x: f32,
        baseline: f32,
        size: f32,
        ch: char,
    },
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
}

impl Default for DisplayOptions {
    fn default() -> Self {
        DisplayOptions { marks: false, dim_header: true, dim_body: false, markup: true }
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
        Placed::Shape { rect, kind, fill, stroke, stroke_width } => {
            out.push(Draw::Shape { rect: *rect, kind: *kind, fill: *fill, stroke: *stroke, stroke_width: *stroke_width })
        }
        Placed::Cell { .. } => {}
        Placed::Lines { story, path, para, l0, l1, x, y } => lines(doc, *story, path, para, *l0, *l1, *x, *y, opts, alpha, out),
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
                if let (Some(x0), Some(x1)) = (pl.x_of(li, a.max(line.start)), pl.x_of(li, b.min(line.stop)))
                    && x1 > x0
                {
                    out.push(Draw::Fill { rect: Rect::new(x + x0, top, x1 - x0, line.height), color: Rgb(0xEF, 0xE3, 0xF7), alpha });
                }
            }
        }
        // Backgrounds first: highlight and character shading; character borders group equal adjacent borders.
        let mut open: Option<(Border, f32, f32)> = None;
        for k in line.c0..line.c1 {
            let (Some(c), Some(cx), Some(nx)) = (pl.clusters.get(k), line.xs.get(k - line.c0), line.xs.get(k + 1 - line.c0)) else { continue };
            let Some(st) = pl.styles.get(c.style as usize) else { continue };
            if c.kind == ClKind::Marker {
                continue;
            }
            if let Some(h) = st.rc.highlight.or(st.rc.shading) {
                out.push(Draw::Fill { rect: Rect::new(x + cx, top, (nx - cx).max(0.0), line.height), color: h, alpha });
            }
            let b = st.rc.border;
            if let Some((ob, _, x1)) = open.as_mut()
                && Some(*ob) == b
            {
                *x1 = x + nx;
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
            });
        }
        // Hyphen at a hyphenated line end.
        if let Some((si, gid, adv)) = line.hyphen
            && let Some(st) = pl.styles.get(si as usize)
        {
            let hx = x + line.xs.last().copied().unwrap_or(0.0) - adv;
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
            let run_start = k;
            while k < line.c1 {
                let Some(c) = pl.clusters.get(k) else { break };
                if c.style != si {
                    break;
                }
                let cx = x + line.xs.get(k - line.c0).copied().unwrap_or(0.0);
                if matches!(c.kind, ClKind::Text | ClKind::Space) {
                    for g in pl.glyphs.get(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                        glyphs.push((g.gid, cx + g.dx, base - st.shift - g.dy));
                    }
                    if let Some(p) = para {
                        text.push_str(p.text.get(c.start..c.end).unwrap_or(""));
                    }
                }
                k += 1;
            }
            let run_end = k;
            let rc = &st.rc;
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
                });
            }
            // Decorations across the run (underline skips trailing spaces of the line).
            let x0 = x + line.xs.get(run_start - line.c0).copied().unwrap_or(0.0);
            let mut end_k = run_end;
            while end_k > run_start
                && pl.clusters.get(end_k - 1).is_some_and(|c| matches!(c.kind, ClKind::Space | ClKind::Marker | ClKind::LineBreak))
                && end_k == line.c1
            {
                end_k -= 1;
            }
            let x1 = x + line.xs.get(end_k - line.c0).copied().unwrap_or(0.0);
            let thick = (st.size / 18.0).max(0.5);
            let underline = if rc.ins.is_some() && opts.markup { Underline::Single } else { rc.underline };
            if underline != Underline::None && x1 > x0 {
                let uy = base - st.shift + st.size * 0.12;
                let ucolor = rc.underline_color.unwrap_or(color);
                if underline == Underline::Words {
                    for kk in run_start..end_k {
                        let Some(c) = pl.clusters.get(kk) else { continue };
                        if c.kind != ClKind::Text {
                            continue;
                        }
                        let a = x + line.xs.get(kk - line.c0).copied().unwrap_or(0.0);
                        let b = x + line.xs.get(kk + 1 - line.c0).copied().unwrap_or(a);
                        out.push(Draw::Line { x0: a, y0: uy, x1: b, y1: uy, width: thick, color: ucolor, stroke: Stroke::Solid, alpha });
                    }
                } else {
                    let w = if underline == Underline::Thick { thick * 2.0 } else { thick };
                    out.push(Draw::Line { x0, y0: uy, x1, y1: uy, width: w, color: ucolor, stroke: stroke_of(underline), alpha });
                }
            }
            if (rc.strike || rc.double_strike || (rc.del.is_some() && opts.markup)) && x1 > x0 {
                let sy = base - st.shift - st.size * 0.28;
                let stroke = if rc.double_strike { Stroke::Double } else { Stroke::Solid };
                out.push(Draw::Line { x0, y0: sy, x1, y1: sy, width: thick, color, stroke, alpha });
            }
        }
        // Proofing squiggles.
        for (a, b, grammar) in &pl.issues {
            if *b <= line.start || *a >= line.stop {
                continue;
            }
            let (Some(x0), Some(x1)) = (pl.x_of(li, (*a).max(line.start)), pl.x_of(li, (*b).min(line.stop))) else { continue };
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
        // Tab leaders.
        for (k, leader) in &line.leaders {
            let (Some(c), Some(a), Some(b)) = (pl.clusters.get(*k), line.xs.get(k - line.c0), line.xs.get(k + 1 - line.c0)) else { continue };
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
            });
        }
        // Inline objects.
        for k in line.c0..line.c1 {
            let Some(c) = pl.clusters.get(k) else { continue };
            let ClKind::Object(oi) = c.kind else { continue };
            if c.obj_h <= 0.0 {
                continue;
            }
            let cx = x + line.xs.get(k - line.c0).copied().unwrap_or(0.0);
            let rect = Rect::new(cx, base - c.obj_h, c.adv, c.obj_h);
            match para.and_then(|p| p.objects.get(oi)) {
                Some(InlineObject::Image { media, crop, .. }) => out.push(Draw::Image { rect, media: media.clone(), crop: *crop, alpha }),
                Some(InlineObject::Shape { kind, fill, stroke, stroke_width, .. }) => {
                    out.push(Draw::Shape { rect, kind: *kind, fill: *fill, stroke: *stroke, stroke_width: *stroke_width })
                }
                _ => {}
            }
        }
        // Formatting marks.
        if opts.marks {
            let msize = pl.styles.first().map(|s| s.size).unwrap_or(11.0);
            for k in line.c0..line.c1 {
                let Some(c) = pl.clusters.get(k) else { continue };
                let cx = x + line.xs.get(k - line.c0).copied().unwrap_or(0.0);
                let nx = x + line.xs.get(k + 1 - line.c0).copied().unwrap_or(cx);
                let size = pl.styles.get(c.style as usize).map(|s| s.size).unwrap_or(msize);
                match c.kind {
                    ClKind::Space => out.push(Draw::Mark { x: (cx + nx) / 2.0 - size * 0.12, baseline: base - size * 0.08, size, ch: '·' }),
                    ClKind::Tab => out.push(Draw::Mark { x: cx + ((nx - cx) / 2.0 - size * 0.3).max(0.0), baseline: base, size, ch: '→' }),
                    ClKind::LineBreak => out.push(Draw::Mark { x: cx + 1.0, baseline: base, size, ch: '↵' }),
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
                        out.push(Draw::Mark { x: cx + 1.0, baseline: base, size, ch: label });
                    }
                    _ => {}
                }
            }
            if line.end == LineEnd::Para {
                let ex = x + line.xs.last().copied().unwrap_or(line.left);
                let size = pl.lines.first().map(|_| msize).unwrap_or(msize);
                out.push(Draw::Mark { x: ex + 1.0, baseline: base, size, ch: '¶' });
            }
        }
        let _ = bottom;
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
