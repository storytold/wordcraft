//! Page → draw items, shared by the raster renderer, the PDF exporter and thumbnails.

use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg, TextAlign};
use wordcraft_doc::para::{InlineObject, ShapeKind};
use wordcraft_doc::props::{Border, BorderStyle, Rgb, TextColor, TextDirection, Underline};
use wordcraft_doc::section::SectionStart;
use wordcraft_doc::{Block, Document, Path, StoryRef};
use wordcraft_fonts::{BezPath, FaceRef};
use wordcraft_geom::{Rect, Spin};

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
    /// An ink stroke: a line through `pts` (page points) with round ends and joins, `width`
    /// wide, in `color` at opacity `alpha`.
    Ink {
        pts: Vec<(f32, f32)>,
        color: Rgb,
        width: f32,
        alpha: f32,
    },
    /// A vector path (a chart's or diagram's polygons, slices, lines): filled, then stroked.
    Path {
        segs: Vec<PathSeg>,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
        stroke_width: f32,
    },
    /// An inline chart or diagram's draws, as one figure with its alt text (for tagged PDF).
    Figure {
        alt: String,
        kind: GraphicKind,
        draws: Vec<Draw>,
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
    /// A rotated or flipped picture, shape or chart: `items` drawn turned by `spin` about page
    /// point (`cx`, `cy`) (see [`Spin::matrix`]).
    Rotated {
        cx: f32,
        cy: f32,
        spin: Spin,
        items: Vec<Draw>,
    },
    /// A formatting-mark label (a section break's name) in the UI's mark colour, left edge at
    /// `x`, drawn with the same face as [`Draw::Mark`]. Screen only, like every mark.
    MarkText {
        x: f32,
        baseline: f32,
        size: f32,
        text: String,
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
    /// Review › Hide Ink: leave ink strokes out (they stay in the document).
    pub hide_ink: bool,
    /// How tracked changes and comments are marked when `markup` is on (Track Changes Options).
    pub revisions: MarkupOptions,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        DisplayOptions {
            marks: false,
            dim_header: true,
            dim_body: false,
            markup: true,
            placeholders: false,
            hide_ink: false,
            revisions: MarkupOptions::default(),
        }
    }
}

/// An option list with stable names for commands and saved preferences.
macro_rules! named {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum $name { $($(#[$vm])* $v),+ }
        impl $name {
            /// Every choice with its name, in menu order.
            pub const ALL: &'static [($name, &'static str)] = &[$(($name::$v, $s)),+];
            pub fn name(self) -> &'static str {
                Self::ALL.iter().find(|(v, _)| *v == self).map(|(_, s)| *s).unwrap_or("")
            }
            pub fn parse(s: &str) -> Option<Self> {
                Self::ALL.iter().find(|(_, n)| *n == s).map(|(v, _)| *v)
            }
        }
    };
}

named! {
    /// How inserted text is marked.
    InsertMark {
        #[default]
        Underline = "underline",
        DoubleUnderline = "doubleUnderline",
        Bold = "bold",
        Italic = "italic",
        Strikethrough = "strikethrough",
        ColorOnly = "colorOnly",
        None = "none",
    }
}

named! {
    /// How deleted text is marked.
    DeleteMark {
        #[default]
        Strikethrough = "strikethrough",
        DoubleStrikethrough = "doubleStrikethrough",
        Hidden = "hidden",
        Caret = "caret",
        Hash = "hash",
        Underline = "underline",
        ColorOnly = "colorOnly",
    }
}

named! {
    /// Where the bar beside changed lines goes.
    ChangeBar {
        #[default]
        Outside = "outside",
        Left = "left",
        Right = "right",
        None = "none",
    }
}

named! {
    /// What the markup area beside the page shows.
    BalloonMode {
        /// Deletions, formatting and comments in balloons (drawn like `CommentsAndFormatting`
        /// for now: revisions stay inline).
        Revisions = "revisions",
        /// Everything inline, no markup area.
        Inline = "inline",
        #[default]
        CommentsAndFormatting = "commentsAndFormatting",
    }
}

/// Track Changes Options: what markup shows and how revisions are drawn. A per-user preference,
/// as in Word (saved with the interface settings, not the document).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarkupOptions {
    pub comments: bool,
    pub ink: bool,
    pub insertions_deletions: bool,
    pub formatting: bool,
    pub balloons: BalloonMode,
    pub insert_mark: InsertMark,
    /// `None` = by author.
    pub insert_color: Option<Rgb>,
    pub delete_mark: DeleteMark,
    /// `None` = by author.
    pub delete_color: Option<Rgb>,
    pub changed_lines: ChangeBar,
    /// `None` = automatic (dark grey).
    pub changed_lines_color: Option<Rgb>,
    /// Record formatting changes while tracking.
    pub track_formatting: bool,
}

impl Default for MarkupOptions {
    fn default() -> Self {
        MarkupOptions {
            comments: true,
            ink: true,
            insertions_deletions: true,
            formatting: true,
            balloons: BalloonMode::default(),
            insert_mark: InsertMark::default(),
            insert_color: None,
            delete_mark: DeleteMark::default(),
            delete_color: None,
            changed_lines: ChangeBar::default(),
            changed_lines_color: None,
            track_formatting: true,
        }
    }
}

impl MarkupOptions {
    /// Tracked deletions take no room: they are hidden, or insertions and deletions aren't shown.
    pub fn hides_deletions(&self) -> bool {
        !self.insertions_deletions || self.delete_mark == DeleteMark::Hidden
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
    if opts.markup && (opts.revisions.insertions_deletions || opts.revisions.formatting) && opts.revisions.changed_lines != ChangeBar::None {
        change_bars(doc, page, &opts.revisions, ba, &mut out);
    }
    out
}

/// The automatic colour of the bars beside changed lines.
pub const CHANGE_BAR: Rgb = Rgb(0x50, 0x50, 0x50);

/// Bars in the margin beside body lines with tracked insertions or deletions, or (when shown)
/// tracked formatting changes of their text, paragraph or paragraph mark.
fn change_bars(doc: &Document, page: &Page, m: &MarkupOptions, alpha: f32, out: &mut Vec<Draw>) {
    // Outside: the left margin, or the outer one of a right-hand page with mirrored margins.
    let right = match m.changed_lines {
        ChangeBar::Right => true,
        ChangeBar::Outside => doc.settings.mirror_margins && page.number % 2 == 1,
        _ => false,
    };
    let bx = if right { page.body.right() + 9.0 } else { page.body.x - 9.0 };
    let color = m.changed_lines_color.unwrap_or(CHANGE_BAR);
    for it in &page.items {
        let Placed::Lines { story, path, para: pl, l0, l1, y, turn, .. } = it else { continue };
        if turn.is_turned() {
            continue;
        }
        let Some(first) = pl.lines.get(*l0) else { continue };
        let (text, fmt) = (m.insertions_deletions, m.formatting);
        let para = doc.para(*story, path);
        let mark_rev = para.is_some_and(|p| (text && (p.mark.ins.is_some() || p.mark.del.is_some())) || (fmt && p.mark.fmt_change.is_some()));
        // A paragraph whose own formatting changed is marked on every line.
        let para_fmt = fmt && para.is_some_and(|p| p.props.fmt_change.is_some());
        for li in *l0..*l1 {
            let Some(line) = pl.lines.get(li) else { continue };
            let changed = para_fmt
                || (line.c0..line.c1)
                    .filter_map(|k| pl.clusters.get(k).and_then(|c| pl.styles.get(c.style as usize)))
                    .any(|st| (text && (st.rc.ins.is_some() || st.rc.del.is_some())) || (fmt && st.rc.fmt.is_some()))
                || (mark_rev && line.end == LineEnd::Para);
            if changed {
                let top = y + (line.top - first.top);
                out.push(Draw::Line { x0: bx, y0: top, x1: bx, y1: top + line.height, width: 0.75, color, stroke: Stroke::Solid, alpha });
            }
        }
    }
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
        Placed::Image { rect, media, crop, spin, .. } => {
            spun(*spin, *rect, vec![Draw::Image { rect: *rect, media: media.clone(), crop: *crop, alpha }], out)
        }
        Placed::Shape { rect, kind, fill, stroke, stroke_width, effects, freeform, spin } => {
            let mut items = Vec::new();
            shape_draws(*rect, *kind, *fill, *stroke, *stroke_width, effects_in(*effects, *spin), freeform.as_deref(), opts, &mut items);
            spun(*spin, *rect, items, out)
        }
        Placed::Graphic { rect, graphic, spin, .. } => spun(*spin, *rect, graphic_draws(doc, graphic, *rect, alpha), out),
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
            if *turn == TextDirection::Down {
                items = upright_east_asian(items);
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
                    let mut shown = None;
                    if let Ok(i) = pl.shown.binary_search_by_key(&k, |(i, _, _)| *i)
                        && let Some((_, s, r)) = pl.shown.get(i)
                    {
                        text.push_str(s);
                        shown = Some(r);
                    } else if let Some(p) = para {
                        text.push_str(p.text.get(c.start..c.end).unwrap_or(""));
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
            // kept it; nor when deletions are hidden.
            if rc.del.is_some() && (!opts.markup || m.hides_deletions()) {
                continue;
            }
            let shown = opts.markup && m.insertions_deletions;
            let del = rc.del.filter(|_| shown);
            let ins = rc.ins.filter(|_| shown && del.is_none());
            let author = |r: u32| revision_color(doc.revisions.get(r as usize).map(|v| author_index(doc, &v.author)).unwrap_or(0));
            let color = match (ins, del) {
                (Some(r), _) => m.insert_color.unwrap_or_else(|| author(r)),
                (None, Some(r)) => m.delete_color.unwrap_or_else(|| author(r)),
                _ => text_color(&rc.color, rc.shading.or(rc.highlight)),
            };
            let ins_mark = ins.map(|_| m.insert_mark);
            let del_mark = del.map(|_| m.delete_mark);
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
            if !glyphs.is_empty() {
                out.push(Draw::Glyphs {
                    face: st.face,
                    size: st.size,
                    glyphs,
                    color,
                    alpha,
                    synth_bold: st.synth_bold || ins_mark == Some(InsertMark::Bold),
                    synth_italic: st.synth_italic || ins_mark == Some(InsertMark::Italic),
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
            let underline = match (ins_mark, del_mark) {
                (Some(InsertMark::Underline), _) | (_, Some(DeleteMark::Underline)) => Underline::Single,
                (Some(InsertMark::DoubleUnderline), _) => Underline::Double,
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
                            out.push(Draw::Line { x0: a, y0: uy, x1: b, y1: uy, width: thick, color: ucolor, stroke: Stroke::Solid, alpha });
                        }
                    } else {
                        let w = if underline == Underline::Thick { thick * 2.0 } else { thick };
                        out.push(Draw::Line { x0, y0: uy, x1, y1: uy, width: w, color: ucolor, stroke: stroke_of(underline), alpha });
                    }
                }
                if struck {
                    let sy = base - st.shift - st.size * 0.28;
                    let stroke = if double { Stroke::Double } else { Stroke::Solid };
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
            let spin = obj.and_then(InlineObject::frame).map(|(_, _, f)| f.spin()).unwrap_or_default();
            match obj {
                Some(o @ (InlineObject::Image { .. } | InlineObject::Shape { .. } | InlineObject::Group { .. })) => {
                    object_draws(o, rect, Spin::default(), alpha, opts, out)
                }
                Some(InlineObject::Graphic { graphic, alt, .. }) => {
                    let mut draws = Vec::new();
                    spun(spin, rect, graphic_draws(doc, graphic, rect, alpha), &mut draws);
                    out.push(Draw::Figure { alt: alt.clone(), kind: graphic.kind, draws })
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
                // The mark follows the line's visual end, past every glyph, even where the
                // logically last text runs against the paragraph's direction.
                let ex = x + line.visual_end_x();
                let size = pl.lines.first().map(|_| msize).unwrap_or(msize);
                // A right-to-left paragraph's mark sits at its end, on the left.
                let mx = if line.rtl { ex - 1.0 - size * 0.6 } else { ex + 1.0 };
                // A tracked (inserted or deleted) paragraph mark is drawn in its author's colour.
                let m = &opts.revisions;
                let color = para.filter(|_| opts.markup && m.insertions_deletions).and_then(|p| match (p.mark.ins, p.mark.del) {
                    (_, Some(r)) => Some(
                        m.delete_color
                            .unwrap_or_else(|| revision_color(doc.revisions.get(r as usize).map(|v| author_index(doc, &v.author)).unwrap_or(0))),
                    ),
                    (Some(r), None) => Some(
                        m.insert_color
                            .unwrap_or_else(|| revision_color(doc.revisions.get(r as usize).map(|v| author_index(doc, &v.author)).unwrap_or(0))),
                    ),
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
            out.push(Draw::Line { x0: a, y0: y, x1: b, y1: y, width: 0.5, color, stroke: Stroke::Dotted, alpha });
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

/// The draws of a chart or diagram's items inside `rect` (page coordinates). The items were built
/// for the size `g.w` × `g.h` (0: `rect`'s own size), so they are scaled to `rect`; strokes and text
/// by the mean of the two scales.
fn graphic_draws(doc: &Document, g: &Graphic, rect: Rect, alpha: f32) -> Vec<Draw> {
    let (sx, sy) = (scale_of(rect.w, g.w), scale_of(rect.h, g.h));
    let k = (sx + sy) / 2.0;
    let inside = |r: &[f32; 4]| Rect::new(rect.x + r[0] * sx, rect.y + r[1] * sy, r[2] * sx, r[3] * sy);
    let mut out = Vec::with_capacity(g.items.len());
    for it in &g.items {
        match it {
            GraphicItem::Shape { rect: r, kind, fill, stroke, stroke_width } => out.push(Draw::Shape {
                rect: inside(r),
                kind: *kind,
                fill: *fill,
                stroke: *stroke,
                stroke_width: stroke_width * k,
                effects: Default::default(),
            }),
            GraphicItem::Path { segs, fill, stroke, stroke_width } => {
                let segs = page_segs(segs, |x, y| (rect.x + x * sx, rect.y + y * sy));
                out.push(Draw::Path { segs, fill: *fill, stroke: *stroke, stroke_width: stroke_width * k })
            }
            GraphicItem::Text { rect: r, .. } => out.extend(text_glyphs(doc, inside(r), it, alpha, k)),
            GraphicItem::Image { rect: r, media } => out.push(Draw::Image { rect: inside(r), media: media.clone(), crop: [0.0; 4], alpha }),
        }
    }
    out
}

/// Scale from the size `built` points a graphic's items were made for to `len` points; 1 when
/// the built size is unknown (0).
fn scale_of(len: f32, built: f32) -> f32 {
    if built.is_finite() && built > 0.0 && len.is_finite() { (len / built).max(0.0) } else { 1.0 }
}

/// Path segments mapped to page coordinates by `map`, cleaned for the rasterizer and PDF: a segment
/// with a non-finite number is dropped, and so is anything before a move (or after a broken one).
fn page_segs(segs: &[PathSeg], map: impl Fn(f32, f32) -> (f32, f32)) -> Vec<PathSeg> {
    let fin = |p: (f32, f32)| p.0.is_finite() && p.1.is_finite();
    let mut out = Vec::with_capacity(segs.len());
    let mut open = false;
    for s in segs {
        match *s {
            PathSeg::Move(x, y) => {
                let p = map(x, y);
                open = fin(p);
                if open {
                    out.push(PathSeg::Move(p.0, p.1));
                }
            }
            PathSeg::Line(x, y) => {
                let p = map(x, y);
                if open && fin(p) {
                    out.push(PathSeg::Line(p.0, p.1));
                }
            }
            PathSeg::Cubic(a, b, c, d, e, f) => {
                let (p1, p2, p3) = (map(a, b), map(c, d), map(e, f));
                if open && fin(p1) && fin(p2) && fin(p3) {
                    out.push(PathSeg::Cubic(p1.0, p1.1, p2.0, p2.1, p3.0, p3.1));
                }
            }
            PathSeg::Close => {
                if open {
                    out.push(PathSeg::Close);
                }
                open = false;
            }
        }
    }
    out
}

/// A path of clean segments (see [`page_segs`]) as a kurbo path, for the rasteriser and PDF.
/// The draws of a shape at `rect`: a preset outline, or a freeform's paths (an ink stroke left out
/// under Review › Hide Ink).
#[allow(clippy::too_many_arguments)]
fn shape_draws(
    rect: Rect,
    kind: ShapeKind,
    fill: Option<Rgb>,
    stroke: Option<Rgb>,
    stroke_width: f32,
    effects: wordcraft_doc::effects::ShapeEffects,
    freeform: Option<&wordcraft_doc::freeform::Freeform>,
    opts: &DisplayOptions,
    out: &mut Vec<Draw>,
) {
    let Some(f) = freeform.filter(|_| kind == ShapeKind::Freeform) else {
        out.push(Draw::Shape { rect, kind, fill, stroke, stroke_width, effects });
        return;
    };
    if f.is_ink() && opts.hide_ink {
        return;
    }
    let width = if stroke_width.is_finite() { stroke_width.clamp(0.0, 200.0) } else { 0.75 };
    for (pts, closed) in f.placed(rect.x, rect.y, rect.w, rect.h) {
        if f.is_ink() || (!closed && fill.is_none()) {
            if let Some(color) = stroke {
                let mut pts: Vec<(f32, f32)> = pts.iter().map(|[x, y]| (*x, *y)).collect();
                if closed && let Some(first) = pts.first().copied() {
                    pts.push(first);
                }
                out.push(Draw::Ink { pts, color, width: width.max(0.25), alpha: f.alpha });
            }
            continue;
        }
        let mut segs: Vec<PathSeg> = Vec::with_capacity(pts.len() + 1);
        for (i, [x, y]) in pts.iter().enumerate() {
            segs.push(if i == 0 { PathSeg::Move(*x, *y) } else { PathSeg::Line(*x, *y) });
        }
        if closed {
            segs.push(PathSeg::Close);
        }
        out.push(Draw::Path { segs, fill: if closed { fill } else { None }, stroke, stroke_width: width });
    }
}

pub fn seg_path(segs: &[PathSeg]) -> BezPath {
    let mut p = BezPath::new();
    for s in segs {
        match *s {
            PathSeg::Move(x, y) => p.move_to((x as f64, y as f64)),
            PathSeg::Line(x, y) => p.line_to((x as f64, y as f64)),
            PathSeg::Cubic(a, b, c, d, e, f) => p.curve_to((a as f64, b as f64), (c as f64, d as f64), (e as f64, f as f64)),
            PathSeg::Close => p.close_path(),
        }
    }
    p
}

/// One line of a `GraphicItem::Text` in `r`, shaped in its font (the document's body font when
/// unset) and aligned horizontally, centred vertically, at its size times `k`. `None` when there is
/// nothing to draw, or the item's rectangle or size isn't a finite number (a degenerate scale:
/// `r` itself is always finite, as [`Rect::new`] makes it).
fn text_glyphs(doc: &Document, r: Rect, it: &GraphicItem, alpha: f32, k: f32) -> Option<Draw> {
    let GraphicItem::Text { rect, text, size, color, bold, align, font } = it else { return None };
    let size = *size * k;
    if !rect.iter().chain([&size]).all(|v| v.is_finite()) {
        return None;
    }
    let family = font.as_deref().filter(|f| !f.is_empty()).unwrap_or(&doc.settings.minor_font);
    let size = size.clamp(1.0, 400.0);
    let resolved = wordcraft_fonts::word::resolve(family, *bold, false);
    let face = resolved.face.get();
    let shaped = wordcraft_fonts::shape(face, text, &[], |c| c);
    if shaped.is_empty() {
        return None;
    }
    let k = size / face.upem.max(1.0) as f32;
    let width: f32 = shaped.iter().map(|g| g.x_advance as f32 * k).sum();
    let (ascent, descent) = wordcraft_fonts::word::line_metrics(face);
    let baseline = r.y + r.h / 2.0 + (ascent - descent) as f32 * k / 2.0;
    let mut pen = match align {
        TextAlign::Left => r.x,
        TextAlign::Center => r.x + (r.w - width) / 2.0,
        TextAlign::Right => r.x + r.w - width,
    };
    let mut glyphs = Vec::with_capacity(shaped.len());
    for g in &shaped {
        glyphs.push((g.gid, pen + g.x_offset as f32 * k, baseline - g.y_offset as f32 * k));
        pen += g.x_advance as f32 * k;
    }
    Some(Draw::Glyphs {
        face: resolved.face,
        size,
        glyphs,
        color: *color,
        alpha,
        synth_bold: resolved.synth_bold,
        synth_italic: false,
        text: text.clone(),
        link: None,
        ranges: Vec::new(),
    })
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

/// `items` turned by `spin` about the centre of `rect` (as they are, unturned).
fn spun(spin: Spin, rect: Rect, items: Vec<Draw>, out: &mut Vec<Draw>) {
    if spin.is_identity() {
        out.extend(items);
    } else {
        out.push(Draw::Rotated { cx: rect.x + rect.w / 2.0, cy: rect.y + rect.h / 2.0, spin, items });
    }
}

/// A picture, shape or group drawn in `rect` (its unturned frame), turned by its own spin about
/// the rect's centre; a group's members are turned inside it. `outer` is the spin it is already
/// drawn inside (its group's). Nothing for anything else.
fn object_draws(o: &InlineObject, rect: Rect, outer: Spin, alpha: f32, opts: &DisplayOptions, out: &mut Vec<Draw>) {
    let own = o.frame().map(|(_, _, f)| f.spin()).unwrap_or_default();
    match o {
        InlineObject::Image { media, crop, .. } => spun(own, rect, vec![Draw::Image { rect, media: media.clone(), crop: *crop, alpha }], out),
        InlineObject::Shape { kind, fill, stroke, stroke_width, effects, freeform, .. } => {
            let effects = effects_in(*effects, own.within(outer));
            let mut items = Vec::new();
            shape_draws(rect, *kind, *fill, *stroke, *stroke_width, effects, freeform.as_deref(), opts, &mut items);
            spun(own, rect, items, out)
        }
        InlineObject::Group { .. } => {
            let mut members = Vec::new();
            for ([x, y, w, h], c) in o.group_rects(rect.x, rect.y, rect.w, rect.h) {
                object_draws(c, Rect::new(x, y, w, h), own.within(outer), alpha, opts, &mut members);
            }
            spun(own, rect, members, out)
        }
        _ => {}
    }
}

/// A shape's `effects` as drawn turned by `spin` (all the turns it is drawn inside): glows and
/// soft edges turn with the shape, and so does a shadow's offset when it rotates with the shape
/// (`rotWithShape`); one that doesn't keeps its direction on the page, so its angle is turned back.
pub(crate) fn effects_in(effects: wordcraft_doc::effects::ShapeEffects, spin: Spin) -> wordcraft_doc::effects::ShapeEffects {
    let mut e = effects;
    if let Some(s) = e.shadow.as_mut().filter(|s| !s.rot_with_shape && !spin.is_identity()) {
        let a = s.angle.to_radians();
        let (dx, dy) = spin.unapply(0.0, 0.0, a.cos(), a.sin());
        s.angle = wordcraft_geom::normalize_degrees(dy.atan2(dx).to_degrees());
    }
    e
}

/// Where inline object `obj` is drawn, given its cluster's box (`adv` × `obj_h` standing on the
/// baseline at `cx`): inside the room kept for its effects.
/// A rotated one's unrotated frame, centred in that room.
pub(crate) fn inline_rect(obj: Option<&InlineObject>, cx: f32, base: f32, adv: f32, obj_h: f32) -> Rect {
    let frame = obj.and_then(InlineObject::frame);
    let [l, t, r, b] = frame.map_or([0.0; 4], |(_, _, float)| float.effect_extent());
    let room = Rect::new(cx + l, base - obj_h + t, (adv - l - r).max(0.0), (obj_h - t - b).max(0.0));
    let Some((w, h, float)) = frame.filter(|(_, _, f)| f.rot != 0.0) else { return room };
    // The room is the rotated bounds of the (possibly scaled-down) frame: undo the rotation.
    let (w, h) = (wordcraft_geom::finite(w).clamp(1.0, 4000.0), wordcraft_geom::finite(h).clamp(1.0, 4000.0));
    let (bw, _) = float.spin().extent(w, h);
    let k = if bw > 0.0 { room.w / bw } else { 1.0 };
    let (fw, fh) = (w * k, h * k);
    Rect::new(room.x + (room.w - fw) / 2.0, room.y + (room.h - fh) / 2.0, fw, fh)
}

/// Whether `c` stands upright in top-to-bottom East Asian text (`tbRl`) rather than turning with
/// the line: ideographs, kana, Hangul, full-width letters and digits, and the ideographic comma
/// and full stop (an approximation of Unicode's vertical orientation, UAX #50). Brackets, dashes
/// and the prolonged sound mark turn with the line, as their vertical forms do.
pub fn upright_in_vertical(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{11FF}'
        | '\u{2E80}'..='\u{2FFF}'
        | '\u{3001}'..='\u{3007}'
        | '\u{3012}'..='\u{3013}'
        | '\u{3020}'..='\u{302F}'
        | '\u{3031}'..='\u{303F}'
        | '\u{3040}'..='\u{30FB}'
        | '\u{30FD}'..='\u{30FF}'
        | '\u{3100}'..='\u{31EF}'
        | '\u{31F0}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{A960}'..='\u{A97F}'
        | '\u{AC00}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF01}'..='\u{FF07}'
        | '\u{FF0A}'..='\u{FF0C}'
        | '\u{FF0E}'..='\u{FF3A}'
        | '\u{FF3C}'
        | '\u{FF3E}'..='\u{FF5A}'
        | '\u{FFE0}'..='\u{FFE6}'
        | '\u{20000}'..='\u{3FFFF}'
    )
}

/// Top-to-bottom text drawn turned 90° clockwise, with its East Asian characters turned back
/// upright, each about the centre of its em square (`tbRl`: ideographs stand, Latin lies).
/// Glyph runs are split where uprightness changes, so the text keeps its order.
fn upright_east_asian(items: Vec<Draw>) -> Vec<Draw> {
    let mut out = Vec::with_capacity(items.len());
    for d in items {
        let Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges } = d else {
            out.push(d);
            continue;
        };
        let up = |k: usize| ranges.get(k).and_then(|r| text.get(r.clone())).and_then(|t| t.chars().next()).is_some_and(upright_in_vertical);
        if ranges.len() != glyphs.len() || !(0..glyphs.len()).any(up) {
            out.push(Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges });
            continue;
        }
        let piece = |k0: usize, k1: usize| -> Draw {
            let gs = glyphs.get(k0..k1).unwrap_or(&[]).to_vec();
            let rs = ranges.get(k0..k1).unwrap_or(&[]);
            let (b0, b1) = (rs.iter().map(|r| r.start).min().unwrap_or(0), rs.iter().map(|r| r.end).max().unwrap_or(0));
            let t = text.get(b0..b1).unwrap_or("").to_string();
            let rs = rs.iter().map(|r| r.start.saturating_sub(b0)..r.end.saturating_sub(b0)).collect();
            Draw::Glyphs { face, size, glyphs: gs, color, alpha, synth_bold, synth_italic, text: t, link: link.clone(), ranges: rs }
        };
        let mut k0 = 0;
        while k0 < glyphs.len() {
            let u = up(k0);
            let k1 = (k0 + 1..glyphs.len()).find(|&k| up(k) != u).unwrap_or(glyphs.len());
            if !u {
                out.push(piece(k0, k1));
            } else {
                for k in k0..k1 {
                    let Some(&(_, gx, base)) = glyphs.get(k) else { continue };
                    // The em square: one em along the line (the advance to the next glyph when
                    // known), from about 0.88 em above the baseline to 0.12 em below it.
                    let adv = glyphs.get(k + 1).map(|g| g.1 - gx).filter(|a| *a > 0.0 && *a < size * 2.0).unwrap_or(size);
                    let (cx, cy) = (gx + adv / 2.0, base - size * 0.38);
                    let mut g = piece(k, k + 1);
                    // The ideographic comma and full stop sit in the upper right of their square in
                    // vertical text (lower left in horizontal): moved there before standing up.
                    if let Draw::Glyphs { text, glyphs, .. } = &mut g
                        && matches!(text.chars().next(), Some('\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}'))
                    {
                        for (_, x, y) in glyphs.iter_mut() {
                            *x += size * 0.55;
                            *y -= size * 0.55;
                        }
                    }
                    out.push(Draw::Rotated { cx, cy, spin: Spin::new(-90.0, false, false), items: vec![g] });
                }
            }
            k0 = k1;
        }
    }
    out
}
