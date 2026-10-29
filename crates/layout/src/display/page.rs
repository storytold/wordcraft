use super::glyphs::{Authors, upright_east_asian};
use super::lines::{LineRender, lines};
use super::objects::{ShapeLook, effects_in, graphic_draws, object_draws, shape_draws, spun};
use super::options::{DisplayOptions, Hidden, MarkupOptions};
use super::paint::Stroke;
use super::{
    BezPath, Border, BorderStyle, ChangeBar, Document, Draw, HashMap, InlineObject, LineEnd, Page, Placed, Rect, Rgb, Spin, StoryRef, TextDirection,
    Underline,
};

/// Colour of moved text (both ends of a tracked move).
pub const MOVE_COLOR: Rgb = Rgb(0x1A, 0x7F, 0x37);

/// Colour for revisions by author index.
pub fn revision_color(i: u32) -> Rgb {
    const C: [Rgb; 6] =
        [Rgb(0xB0, 0x1E, 0x8F), Rgb(0x1E, 0x6E, 0xB0), Rgb(0x2E, 0x8B, 0x3E), Rgb(0xC0, 0x5A, 0x10), Rgb(0x70, 0x3C, 0xB0), Rgb(0x0E, 0x7C, 0x86)];
    C.get(i as usize % C.len()).copied().unwrap_or(C[0])
}

pub(super) fn stroke_of(u: Underline) -> Stroke {
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
    let (ha, ba) = dimming(opts);
    let authors = Authors::new(doc);
    stories_display(doc, &[&page.header[..], &page.footer[..]], None, opts, &authors, ha, &mut out);
    stories_display(doc, &[&page.items[..]], None, opts, &authors, ba, &mut out);
    body_change_bars(doc, page, &page.items, opts, ba, &mut out);
    out
}

/// A page's draw items one placed item at a time, for a caller that draws and tags each item on
/// its own (the PDF export). What every item shares, the revision authors' colours and the cells
/// cut off at their edge, is worked out once for the page, not once for every item as calling
/// [`page_display`] with each would (#3866).
pub struct ItemDisplay<'a> {
    doc: &'a Document,
    page: &'a Page,
    authors: Authors<'a>,
    clips: Clips,
}

impl<'a> ItemDisplay<'a> {
    pub fn new(doc: &'a Document, page: &'a Page) -> Self {
        let cells: Vec<Placed> =
            page.header.iter().chain(&page.footer).chain(&page.items).filter(|c| matches!(c, Placed::Cell { clip: true, .. })).cloned().collect();
        ItemDisplay { doc, page, authors: Authors::new(doc), clips: clipped_cells(&cells) }
    }

    /// The draw items of `it` alone, as [`page_display`] gives them for a page holding just it
    /// (and, for a line in a clipped cell, the cell's clip).
    pub fn draws(&self, it: &Placed, opts: &DisplayOptions) -> Vec<Draw> {
        self.draws_in(&[], it, opts)
    }

    /// [`Self::draws`] for `it` inside the turned text boxes `turns` (their opening
    /// [`Placed::Spun`] markers, outermost first): drawn turned with them.
    pub fn draws_in(&self, turns: &[Placed], it: &Placed, opts: &DisplayOptions) -> Vec<Draw> {
        let mut out = Vec::new();
        let items: Vec<Placed> = turns.iter().cloned().chain(std::iter::once(it.clone())).collect();
        let items = &items[..];
        let (_, ba) = dimming(opts);
        stories_display(self.doc, &[items], Some(&self.clips), opts, &self.authors, ba, &mut out);
        body_change_bars(self.doc, self.page, items, opts, ba, &mut out);
        out
    }
}

/// A page's draw items as two layers, its headers and footers then its body, each at full
/// strength with the opacity to draw it at. A dimmed layer is faded as one picture, as Word dims
/// it: where its lines or fills overlap (a table's borders meeting) they don't darken each other.
pub fn page_layers(doc: &Document, page: &Page, opts: &DisplayOptions) -> [(f32, Vec<Draw>); 2] {
    let (ha, ba) = dimming(opts);
    let (mut hf, mut body) = (Vec::new(), Vec::new());
    let authors = Authors::new(doc);
    stories_display(doc, &[&page.header[..], &page.footer[..]], None, opts, &authors, 1.0, &mut hf);
    stories_display(doc, &[&page.items[..]], None, opts, &authors, 1.0, &mut body);
    body_change_bars(doc, page, &page.items, opts, 1.0, &mut body);
    [(ha, hf), (ba, body)]
}

/// The opacity of the headers/footers and of the body.
fn dimming(opts: &DisplayOptions) -> (f32, f32) {
    let a = |dim: bool| if dim { 0.45 } else { 1.0 };
    (a(opts.dim_header), a(opts.dim_body))
}

fn body_change_bars(doc: &Document, page: &Page, items: &[Placed], opts: &DisplayOptions, alpha: f32, out: &mut Vec<Draw>) {
    if opts.markup && (opts.revisions.insertions_deletions || opts.revisions.formatting) && opts.revisions.changed_lines != ChangeBar::None {
        change_bars(doc, page, items, &opts.revisions, alpha, out);
    }
}

/// Draw items for the placed items of `stories`, faded by `a`.
/// `shared` gives the page's clipped cells when the caller worked them out already.
fn stories_display(
    doc: &Document,
    stories: &[&[Placed]],
    shared: Option<&Clips>,
    opts: &DisplayOptions,
    authors: &Authors,
    a: f32,
    out: &mut Vec<Draw>,
) {
    let hidden = opts.hide_object.as_ref().map(|p| Hidden::new(doc, p));
    for items in stories {
        let warps = warped_boxes(doc, items);
        let own;
        let clips = match shared {
            Some(c) => c,
            None => {
                own = clipped_cells(items);
                &own
            }
        };
        let mut done: Vec<u32> = Vec::new();
        // Turned text boxes being drawn: what was drawn before each, and its turn (`None` when
        // shown straight, as while its text is edited).
        let mut turns: Vec<(Vec<Draw>, Option<(Rect, Spin)>)> = Vec::new();
        for it in items.iter() {
            // The turn the item is drawn inside (a shape's effects follow it).
            let outer = turns.iter().filter_map(|(_, t)| t.map(|(_, s)| s)).fold(Spin::default(), |o, s| s.within(o));
            // The object being dragged is drawn by its preview instead: leave it out here.
            if hidden.as_ref().is_some_and(|h| h.hides(it)) {
                continue;
            }
            if let Placed::Lines { story, path, .. } = it
                && let Some(rect) = cell_clip(clips, *story, &path.0)
            {
                let mut inner = Vec::new();
                item(doc, it, opts, authors, a, &mut inner);
                out.push(Draw::Clip { rect, items: inner });
                continue;
            }
            match it {
                Placed::Spun { rect, spin, part, open: true } => {
                    let shown = (opts.straight != Some(*part)).then_some((*rect, *spin));
                    turns.push((std::mem::take(out), shown));
                }
                Placed::Spun { open: false, .. } => {
                    if let Some((before, t)) = turns.pop() {
                        unturn(before, t, out);
                    }
                }
                Placed::Lines { story: StoryRef::Part(id), .. } if warps.iter().any(|(w, ..)| w == id) => {
                    // A warped text box's lines are drawn together, bent, where its first one is.
                    if done.contains(id) {
                        continue;
                    }
                    done.push(*id);
                    if let Some((_, rect, warp)) = warps.iter().find(|(w, ..)| w == id) {
                        let lines: Vec<&Placed> =
                            items.iter().filter(|i| matches!(i, Placed::Lines { story: StoryRef::Part(s), .. } if s == id)).collect();
                        warped_text(WarpedText { doc, lines: &lines, rect: *rect, warp, opts, authors, alpha: a }, out);
                    }
                }
                _ => item_in(doc, it, opts, authors, a, outer, out),
            }
        }
        while let Some((before, t)) = turns.pop() {
            unturn(before, t, out);
        }
    }
}

/// Close a turned text box: what was drawn since it opened (`out`) goes inside its turn `t`
/// (straight when `None`), after what was drawn before it.
fn unturn(before: Vec<Draw>, t: Option<(Rect, Spin)>, out: &mut Vec<Draw>) {
    let inner = std::mem::replace(out, before);
    match t {
        Some((rect, spin)) => {
            // Link areas are axis-aligned page rectangles: turned text gets none.
            let mut inner = inner;
            clear_links(&mut inner);
            spun(spin, rect, inner, out)
        }
        None => out.extend(inner),
    }
}

/// Take the hyperlink off every glyph run in `draws`, however deeply nested.
fn clear_links(draws: &mut [Draw]) {
    for d in draws {
        match d {
            Draw::Glyphs { link, .. } => *link = None,
            Draw::Turned { items, .. } | Draw::Rotated { items, .. } | Draw::Clip { items, .. } => clear_links(items),
            Draw::Figure { draws, .. } => clear_links(draws),
            _ => {}
        }
    }
}

/// One object drawn alone, as its page draws it: the picture, shape or group `o` in `rect` (its
/// unturned frame) and the text of the text box story `text_box` on `page`. A dragged object's
/// preview, which mustn't show what was behind it (#1010).
pub fn object_display(doc: &Document, page: &Page, o: &InlineObject, rect: Rect, text_box: Option<u32>, opts: &DisplayOptions) -> Vec<Draw> {
    let mut out = Vec::new();
    object_draws(o, rect, Spin::default(), 1.0, opts, &mut out);
    let Some(id) = text_box else { return out };
    let authors = Authors::new(doc);
    for items in [&page.items, &page.header, &page.footer] {
        let lines: Vec<&Placed> = items.iter().filter(|i| matches!(i, Placed::Lines { story: StoryRef::Part(s), .. } if *s == id)).collect();
        match warped_boxes(doc, items).into_iter().find(|(w, ..)| *w == id) {
            Some((_, rect, warp)) if !lines.is_empty() => {
                warped_text(WarpedText { doc, lines: &lines, rect, warp: &warp, opts, authors: &authors, alpha: 1.0 }, &mut out)
            }
            _ => lines.into_iter().for_each(|it| item(doc, it, opts, &authors, 1.0, &mut out)),
        }
    }
    out
}

/// The cells among `items` whose content is cut off at their edge (see [`Placed::Cell`]'s
/// `clip`), by story and the path of their content (table, row, cell): their area, and the
/// order they come in (the first of two that hold a line clips it).
type Clips = HashMap<StoryRef, HashMap<Vec<u32>, (usize, Rect)>>;

fn clipped_cells(items: &[Placed]) -> Clips {
    let mut clips = Clips::new();
    let cells = items.iter().filter_map(|it| match it {
        Placed::Cell { rect, table, row, cell, story, clip: true, .. } => {
            let mut prefix = table.0.clone();
            prefix.extend([u32::try_from(*row).ok()?, u32::try_from(*cell).ok()?]);
            Some((*story, prefix, *rect))
        }
        _ => None,
    });
    for (i, (story, prefix, rect)) in cells.enumerate() {
        crate::work::steps(1);
        clips.entry(story).or_default().entry(prefix).or_insert((i, rect));
    }
    clips
}

/// The area to clip a line at `path` of `story` to: its cell's, when that cell is clipped. Only
/// the cells holding the line are looked up, not every clipped cell on the page.
fn cell_clip(clips: &Clips, story: StoryRef, path: &[u32]) -> Option<Rect> {
    let cells = clips.get(&story)?;
    (0..=path.len())
        .filter_map(|n| {
            crate::work::steps(1);
            cells.get(path.get(..n)?)
        })
        .min_by_key(|(i, _)| *i)
        .map(|(_, r)| *r)
}

/// The text boxes among `items` whose text is warped (Text Effects › Transform, drawn bent): their
/// story, frame and warp. Empty, without a walk of the document, when none is.
fn warped_boxes(doc: &Document, items: &[Placed]) -> Vec<(u32, Rect, wordcraft_doc::wordart::TextWarp)> {
    items
        .iter()
        .filter_map(|it| {
            let Placed::Object { rect, story, path, off, text_box: Some(id), .. } = it else { return None };
            match doc.para(*story, path)?.object_at(*off)? {
                InlineObject::Shape { extra, .. } => extra.warp.clone().filter(|w| !w.is_plain()).map(|w| (*id, *rect, w)),
                _ => None,
            }
        })
        .collect()
}

/// The text of a warped text box at `rect`: its lines' letters as outlines bent by `warp`, the
/// box's text stretched over the frame inside its margins (as WordArt is). Underlines, marks and
/// other decorations are left out; a warp that is drawn flat leaves the text as it is.
struct WarpedText<'doc, 'opts, 'lines, 'placed, 'authors, 'author_doc> {
    doc: &'doc Document,
    lines: &'lines [&'placed Placed],
    rect: Rect,
    warp: &'lines wordcraft_doc::wordart::TextWarp,
    opts: &'opts DisplayOptions,
    authors: &'authors Authors<'author_doc>,
    alpha: f32,
}

fn warped_text(input: WarpedText<'_, '_, '_, '_, '_, '_>, out: &mut Vec<Draw>) {
    let WarpedText { doc, lines, rect, warp, opts, authors, alpha } = input;
    let mut raw = Vec::new();
    for it in lines {
        item(doc, it, opts, authors, alpha, &mut raw);
    }
    // The letters: runs with effects as they are, plain runs from their glyphs.
    let mut arts: Vec<(BezPath, Box<wordcraft_doc::wordart::TextEffects>, Rgb, f32)> = Vec::new();
    let mut hidden = Vec::new();
    for d in &raw {
        match d {
            Draw::Art { path, fx, color, alpha, .. } => arts.push((path.clone(), fx.clone(), *color, *alpha)),
            Draw::Glyphs { face, size, glyphs, color, alpha, synth_italic, .. } => {
                if *alpha > 0.0 {
                    arts.push((crate::wordart::glyph_path(face, *size, glyphs, *synth_italic), Box::default(), *color, *alpha));
                }
                let mut g = d.clone();
                if let Draw::Glyphs { alpha, .. } = &mut g {
                    *alpha = 0.0;
                }
                hidden.push(g);
            }
            _ => {}
        }
    }
    let src = arts.iter().filter_map(|(p, ..)| crate::wordart::path_box(p)).reduce(|a, b| a.union(b));
    let inset = Rect::new(rect.x + 7.2, rect.y + 3.6, (rect.w - 14.4).max(1.0), (rect.h - 7.2).max(1.0));
    let fitted = src.and_then(|b| crate::wordart::Warp::new(warp, inset, (b.height() / b.width().max(1e-3)) as f32).map(|w| (b, w)));
    let Some((src, w)) = fitted else {
        out.extend(raw);
        return;
    };
    for (path, fx, color, alpha) in arts {
        // No text background: the bent letters leave their highlight behind.
        out.push(Draw::Art { path: crate::wordart::warp_path(&path, src, &w), fx, color, alpha, text_bg: None });
    }
    out.extend(hidden);
}

/// The automatic colour of the bars beside changed lines.
pub const CHANGE_BAR: Rgb = Rgb(0x50, 0x50, 0x50);

/// Bars in the margin beside body lines with tracked insertions or deletions, or (when shown)
/// tracked formatting changes of their text, paragraph or paragraph mark.
fn change_bars(doc: &Document, page: &Page, items: &[Placed], m: &MarkupOptions, alpha: f32, out: &mut Vec<Draw>) {
    // Outside: the left margin, or the outer one of a right-hand page with mirrored margins.
    let right = match m.changed_lines {
        ChangeBar::Right => true,
        ChangeBar::Outside => doc.settings.mirror_margins && page.number % 2 == 1,
        _ => false,
    };
    let bx = if right { page.body.right() + 9.0 } else { page.body.x - 9.0 };
    let color = m.changed_lines_color.unwrap_or(CHANGE_BAR);
    for it in items {
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
                out.push(Draw::Line {
                    x0: bx,
                    y0: top,
                    x1: bx,
                    y1: top + line.height,
                    width: 0.75,
                    color,
                    stroke: Stroke::Solid,
                    alpha,
                    text_bg: None,
                });
            }
        }
    }
}

fn border_stroke(s: BorderStyle) -> Stroke {
    match s {
        BorderStyle::Dotted => Stroke::BorderDotted,
        BorderStyle::Dashed => Stroke::BorderDashed,
        BorderStyle::DotDash => Stroke::BorderDotDash,
        BorderStyle::Double => Stroke::Double,
        BorderStyle::Triple => Stroke::Triple,
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
        text_bg: None,
    }
}

/// Closed box around one line segment of a character-border group, widened by `space`.
// Note: border drawn outside text advance, reserve width in line breaking if overlap shows.
pub(super) fn char_box(b: &Border, x0: f32, x1: f32, top: f32, bottom: f32, alpha: f32, out: &mut Vec<Draw>) {
    let (l, r) = (x0 - b.space, x1 + b.space);
    out.push(rule(l, top, r, top, b, alpha));
    out.push(rule(l, bottom, r, bottom, b, alpha));
    out.push(rule(l, top, l, bottom, b, alpha));
    out.push(rule(r, top, r, bottom, b, alpha));
}

fn item(doc: &Document, it: &Placed, opts: &DisplayOptions, authors: &Authors, alpha: f32, out: &mut Vec<Draw>) {
    item_in(doc, it, opts, authors, alpha, Spin::default(), out)
}

/// [`item`] drawn inside the turn `outer` (a turned Drawing Canvas's members): a shape's
/// effects follow all the turns it is drawn inside.
fn item_in(doc: &Document, it: &Placed, opts: &DisplayOptions, authors: &Authors, alpha: f32, outer: Spin, out: &mut Vec<Draw>) {
    match it {
        Placed::Fill { rect, color } => out.push(Draw::Fill { rect: *rect, color: *color, alpha, text_bg: false }),
        Placed::Rule { x0, y0, x1, y1, border } => out.push(rule(*x0, *y0, *x1, *y1, border, alpha)),
        Placed::Image { rect, media, crop, spin, .. } => spun(
            *spin,
            *rect,
            vec![Draw::Image { rect: *rect, media: media.clone(), crop: *crop, alpha, alt: String::new(), decorative: false }],
            out,
        ),
        Placed::Shape { rect, kind, fill, stroke, stroke_width, effects, freeform, spin, extra, .. } => {
            let mut items = Vec::new();
            let fx = effects_in(*effects, spin.within(outer));
            shape_draws(
                *rect,
                &ShapeLook {
                    kind: *kind,
                    fill: fill.as_ref(),
                    stroke: *stroke,
                    stroke_width: *stroke_width,
                    effects: fx,
                    freeform: freeform.as_deref(),
                    extra,
                },
                opts,
                &mut items,
            );
            spun(*spin, *rect, items, out)
        }
        Placed::Graphic { rect, graphic, spin, .. } => spun(*spin, *rect, graphic_draws(doc, graphic, *rect, alpha), out),
        Placed::Clip { rect, spin, items } => {
            let mut inner = Vec::new();
            for it in items {
                item_in(doc, it, opts, authors, alpha, spin.within(outer), &mut inner);
            }
            spun(*spin, *rect, vec![Draw::Clip { rect: *rect, items: inner }], out)
        }
        // A turned text box's ends: the page's walk turns what is between (see `stories_display`).
        Placed::Cell { .. } | Placed::Object { .. } | Placed::Spun { .. } => {}
        Placed::Lines { story, path, para, l0, l1, x, y, turn } if turn.is_turned() => {
            let mut items = Vec::new();
            lines(
                LineRender {
                    doc,
                    story: *story,
                    path,
                    pl: para,
                    range: *l0..*l1,
                    origin: (0.0, 0.0),
                    opts,
                    authors,
                    alpha,
                    down: *turn == TextDirection::Down,
                },
                &mut items,
            );
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
        Placed::Lines { story, path, para, l0, l1, x, y, .. } => {
            lines(LineRender { doc, story: *story, path, pl: para, range: *l0..*l1, origin: (*x, *y), opts, authors, alpha, down: false }, out)
        }
    }
}

#[cfg(test)]
mod tests_clips {
    use super::*;
    use wordcraft_doc::Path as DocPath;

    /// A cell whose area is told apart by `x`.
    fn cell(story: StoryRef, table: Vec<u32>, row: usize, cell: usize, clip: bool, x: f32) -> Placed {
        Placed::Cell { rect: Rect::new(x, 0.0, 1.0, 1.0), covered: false, table: DocPath(table), row, cell, story, clip }
    }

    /// Which cell (by its `x`) clips a line at `path` of `story`.
    fn found(clips: &Clips, story: StoryRef, path: &[u32]) -> Option<f32> {
        cell_clip(clips, story, path).map(|r| r.x)
    }

    /// A line in a cell cut off at its edge is drawn with that cell's clip, found by path in an
    /// index built once per page: every line used to look through every clipped cell on the page,
    /// so a page of exact-height rows cost lines x cells.
    #[test]
    fn a_line_finds_its_clipped_cell_by_path() {
        let mut page = Page {
            items: vec![cell(StoryRef::Body, vec![0], 0, 1, false, 1.0), cell(StoryRef::Body, vec![2], 1, 0, true, 2.0)],
            ..Default::default()
        };
        // A nested clipped cell inside the second one; the outer one comes first in the page.
        page.items.push(cell(StoryRef::Body, vec![2, 1, 0, 3], 0, 0, true, 3.0));
        page.footer.push(cell(StoryRef::Part(7), vec![0], 0, 0, true, 4.0));
        // The page's cells in header, footer, items order, as the PDF export and layout see them.
        let all: Vec<Placed> = page.header.iter().chain(&page.footer).chain(&page.items).cloned().collect();
        let clips = clipped_cells(&all);
        assert_eq!(found(&clips, StoryRef::Body, &[2, 1, 0, 0]), Some(2.0));
        assert_eq!(found(&clips, StoryRef::Body, &[2, 1, 0, 3, 0, 0, 1]), Some(2.0), "the first cell in page order wins, as in layout");
        assert_eq!(found(&clips, StoryRef::Body, &[0, 0, 1, 0]), None, "that cell isn't clipped");
        assert_eq!(found(&clips, StoryRef::Body, &[2, 0, 0, 0]), None, "another row");
        assert_eq!(found(&clips, StoryRef::Part(7), &[0, 0, 0, 2]), Some(4.0));
        assert_eq!(found(&clips, StoryRef::Body, &[0, 0, 0, 2]), None, "another story");
    }

    /// Thousands of clipped cells and as many lines: a lookup each, not a pass over the cells each.
    #[test]
    fn looking_up_many_clipped_cells_takes_linear_time() {
        let make = |n: usize| (0..n).map(|r| cell(StoryRef::Body, vec![0], r, 0, true, r as f32)).collect::<Vec<_>>();
        crate::tests::linear::assert_linear_steps("clip lookups", 1000, make, |cells| {
            let clips = clipped_cells(cells);
            let hits = (0..cells.len() as u32).filter(|r| cell_clip(&clips, StoryRef::Body, &[0, *r, 0, 0]).is_some()).count();
            assert_eq!(hits, cells.len());
        });
    }
}
