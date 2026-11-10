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
    /// The page's text boxes (see [`text_frames`]): a turned one's lines are drawn turned.
    turned: Vec<(u32, Rect, Spin)>,
}

impl<'a> ItemDisplay<'a> {
    pub fn new(doc: &'a Document, page: &'a Page) -> Self {
        let cells: Vec<Placed> =
            page.header.iter().chain(&page.footer).chain(&page.items).filter(|c| matches!(c, Placed::Cell { clip: true, .. })).cloned().collect();
        let turned = [&page.items, &page.header, &page.footer].into_iter().flat_map(|items| text_frames(items)).collect();
        ItemDisplay { doc, page, authors: Authors::new(doc), clips: clipped_cells(&cells), turned }
    }

    /// The draw items of `it` alone, as [`page_display`] gives them for a page holding just it
    /// (and, for a line in a clipped cell, the cell's clip).
    pub fn draws(&self, it: &Placed, opts: &DisplayOptions) -> Vec<Draw> {
        let mut out = Vec::new();
        let items = std::slice::from_ref(it);
        let (_, ba) = dimming(opts);
        stories_display(self.doc, &[items], Some(&self.clips), opts, &self.authors, ba, &mut out);
        // A line of a turned text box turns with it (as `stories_display` turns them on a page).
        if let Some((_, rect, spin)) = turned_frame(&self.turned, it).and_then(|k| self.turned.get(k)) {
            let mut turned = Vec::new();
            spun(*spin, *rect, std::mem::take(&mut out), &mut turned);
            out = turned;
        }
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
        let frames = text_frames(items);
        let mut spun_frames: Vec<usize> = Vec::new();
        let own;
        let clips = match shared {
            Some(c) => c,
            None => {
                own = clipped_cells(items);
                &own
            }
        };
        let mut done: Vec<u32> = Vec::new();
        for it in items.iter() {
            // The object being dragged is drawn by its preview instead: leave it out here.
            if hidden.as_ref().is_some_and(|h| h.hides(it)) {
                continue;
            }
            // A line in a clipped cell is drawn in its clip; in a turned text box, with the box's
            // other lines (clipped there), so it is drawn once.
            if let Placed::Lines { story, path, .. } = it
                && let Some(rect) = cell_clip(clips, *story, &path.0)
                && turned_frame(&frames, it).is_none()
            {
                let mut inner = Vec::new();
                item(doc, it, opts, authors, a, &mut inner);
                out.push(Draw::Clip { rect, items: inner });
                continue;
            }
            match it {
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
                _ => match turned_frame(&frames, it) {
                    // A turned text box's lines are drawn together, turned with it about its
                    // centre, where its first one is. Linked boxes show one story: each line goes
                    // with the box it was laid out in.
                    Some(k) => {
                        if spun_frames.contains(&k) {
                            continue;
                        }
                        spun_frames.push(k);
                        if let Some((_, rect, spin)) = frames.get(k) {
                            let mut inner = Vec::new();
                            for line in items.iter().filter(|i| turned_frame(&frames, i) == Some(k)) {
                                if let Placed::Lines { story, path, .. } = line
                                    && let Some(rect) = cell_clip(clips, *story, &path.0)
                                {
                                    let mut clipped = Vec::new();
                                    item(doc, line, opts, authors, a, &mut clipped);
                                    inner.push(Draw::Clip { rect, items: clipped });
                                } else {
                                    item(doc, line, opts, authors, a, &mut inner);
                                }
                            }
                            super::objects::spun(*spin, *rect, inner, out);
                        }
                    }
                    None => item(doc, it, opts, authors, a, out),
                },
            }
        }
    }
}

/// The text boxes among `items`: the story each shows, its frame (unturned) and how its text
/// turns with it (not at all for most). As in Word, the text turns with the box but isn't
/// mirrored: a box flipped across turns its text upside down, one flipped side to side leaves it
/// readable. Linked boxes all show the first box's story, so a story can have several frames.
fn text_frames(items: &[Placed]) -> Vec<(u32, Rect, Spin)> {
    items
        .iter()
        .filter_map(|it| {
            let Placed::Object { rect, spin, text_box: Some(id), .. } = it else { return None };
            let text = Spin::new(spin.deg + if spin.flip_v { 180.0 } else { 0.0 }, false, false);
            Some((*id, *rect, text))
        })
        .collect()
}

/// Which of `frames` the text box line `it` was laid out in, if that frame is turned: of the
/// frames showing its story, the one it is in (or nearest, for a line on an edge).
fn turned_frame(frames: &[(u32, Rect, Spin)], it: &Placed) -> Option<usize> {
    let Placed::Lines { story: StoryRef::Part(id), x, y, .. } = it else { return None };
    let away = |r: &Rect| {
        let dx = (r.x - x).max(x - r.right()).max(0.0);
        let dy = (r.y - y).max(y - r.bottom()).max(0.0);
        dx * dx + dy * dy
    };
    let (k, _) = frames.iter().enumerate().filter(|(_, (s, ..))| s == id).min_by(|(_, (_, a, _)), (_, (_, b, _))| away(a).total_cmp(&away(b)))?;
    frames.get(k).is_some_and(|(.., spin)| !spin.is_identity()).then_some(k)
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
    // Keep only requested offsets. A page may show a few boxes from a much longer paragraph,
    // and several placements may share an offset or a text-box story.
    let mut needed: HashMap<_, HashMap<_, Option<&wordcraft_doc::wordart::TextWarp>>> = HashMap::new();
    for it in items {
        if let Placed::Object { story, path, off, text_box: Some(_), .. } = it {
            needed.entry((*story, path)).or_default().insert(*off, None);
        }
    }
    for ((story, path), offsets) in &mut needed {
        let Some(p) = doc.para(*story, path) else { continue };
        offsets.retain(|off, _| p.text.get(*off..).is_some_and(|tail| tail.starts_with(wordcraft_doc::para::OBJ)));
        let Some(last) = offsets.keys().max().copied() else { continue };
        // Stop at the last requested byte offset, including when it precedes a long run of
        // ordinary text. Pair markers and objects exactly as Paragraph::objects_with_offsets.
        let markers = p.text.char_indices().take_while(|(off, _)| *off <= last).filter(|(_, c)| *c == wordcraft_doc::para::OBJ);
        for ((off, _), object) in markers.zip(&p.objects) {
            if let Some(warp) = offsets.get_mut(&off)
                && let InlineObject::Shape { extra, .. } = object
            {
                *warp = extra.warp.as_ref().filter(|w| !w.is_plain());
            }
        }
    }
    items
        .iter()
        .filter_map(|it| {
            let Placed::Object { rect, story, path, off, text_box: Some(id), .. } = it else { return None };
            let warp = needed.get(&(*story, path))?.get(off)?.as_ref()?;
            Some((*id, *rect, (*warp).clone()))
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

/// Keep Word's line width for Double/Triple; expand only the thin/thick styles.
fn rule(from: (f32, f32), to: (f32, f32), side: crate::BorderSide, b: &Border, alpha: f32, out: &mut Vec<Draw>) {
    let (x0, y0) = from;
    let (x1, y1) = to;
    let width = if b.width.is_finite() { b.width.clamp(0.0, 12.0) } else { 0.0 };
    let mut line = |offset: f32, width: f32, stroke| {
        let (nx, ny) = side.inward();
        out.push(Draw::Line {
            x0: x0 + nx * offset,
            y0: y0 + ny * offset,
            x1: x1 + nx * offset,
            y1: y1 + ny * offset,
            width,
            color: b.color.unwrap_or(Rgb::BLACK),
            stroke,
            alpha,
            text_bg: None,
        });
    };
    let mut compound = |pattern: &[f32], unit: f32| {
        let mut edge = -unit * pattern.iter().sum::<f32>() / 2.0;
        for (index, part) in pattern.iter().enumerate() {
            let width = part * unit;
            if index % 2 == 0 && width > 0.0 {
                line(edge + width / 2.0, width, Stroke::Solid);
            }
            edge += width;
        }
    };
    if let Some([inner, gap, outer]) = b.style.stroke_widths() {
        compound(&[outer, gap, inner], (12.0 / (outer + gap + inner)).min(1.0));
    } else if let Some(pattern) = b.style.compound_pattern().filter(|_| !matches!(b.style, BorderStyle::Double | BorderStyle::Triple)) {
        compound(pattern, (width * 0.6).max(0.25));
    } else {
        line(0.0, if b.style == BorderStyle::Thick { width.max(1.5) } else { width.max(0.25) }, border_stroke(b.style));
    }
}

/// Closed box around one line segment of a character-border group, widened by `space`.
// Note: border drawn outside text advance, reserve width in line breaking if overlap shows.
pub(super) fn char_box(b: &Border, x0: f32, x1: f32, top: f32, bottom: f32, alpha: f32, out: &mut Vec<Draw>) {
    let (l, r) = (x0 - b.space, x1 + b.space);
    rule((l, top), (r, top), crate::BorderSide::Top, b, alpha, out);
    rule((l, bottom), (r, bottom), crate::BorderSide::Bottom, b, alpha, out);
    rule((l, top), (l, bottom), crate::BorderSide::Left, b, alpha, out);
    rule((r, top), (r, bottom), crate::BorderSide::Right, b, alpha, out);
}

fn item(doc: &Document, it: &Placed, opts: &DisplayOptions, authors: &Authors, alpha: f32, out: &mut Vec<Draw>) {
    item_in(doc, it, opts, authors, alpha, Spin::default(), out)
}

/// [`item`] drawn inside the turn `outer` (a turned Drawing Canvas's members): a shape's
/// effects follow all the turns it is drawn inside.
fn item_in(doc: &Document, it: &Placed, opts: &DisplayOptions, authors: &Authors, alpha: f32, outer: Spin, out: &mut Vec<Draw>) {
    match it {
        Placed::Fill { rect, color } => out.push(Draw::Fill { rect: *rect, color: *color, alpha, text_bg: false }),
        Placed::Rule { side, x0, y0, x1, y1, border } => rule((*x0, *y0), (*x1, *y1), *side, border, alpha, out),
        Placed::Image { rect, media, crop, spin, fx, .. } => {
            let alpha = alpha * fx.opacity();
            spun(
                *spin,
                *rect,
                vec![Draw::Image { rect: *rect, media: media.clone(), crop: *crop, alpha, alt: String::new(), fx: *fx, decorative: false }],
                out,
            )
        }
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
        Placed::Cell { .. } | Placed::Object { .. } => {}
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

#[cfg(test)]
mod tests_warp_lookups {
    use super::*;
    use wordcraft_doc::para::{OBJ, ShapeKind};
    use wordcraft_doc::wordart::TextWarp;
    use wordcraft_doc::{Block, CharProps, Paragraph, Path};

    fn shape(warp: Option<TextWarp>) -> InlineObject {
        InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 100.0,
            h: 30.0,
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            float: Default::default(),
            story: Some(7),
            freeform: None,
            effects: Default::default(),
            extra: wordcraft_doc::connector::ShapeExtra { warp, ..Default::default() },
        }
    }

    fn placed(story: StoryRef, path: Path, off: usize, id: Option<u32>) -> Placed {
        Placed::Object {
            rect: Rect::new(off as f32, 2.0, 100.0, 30.0),
            spin: Default::default(),
            story,
            path,
            off,
            text_box: id,
            wrap: Default::default(),
            origin: Default::default(),
        }
    }

    #[test]
    fn text_box_warp_lookups_preserve_item_order_and_missing_objects() {
        let arch = TextWarp::new("textArchUp").unwrap();
        let wave = TextWarp::new("textWave1").unwrap();
        let objects = vec![
            InlineObject::BookmarkStart { name: "original".into() },
            shape(None),
            shape(TextWarp::new("textPlain")),
            shape(Some(arch.clone())),
            InlineObject::FieldEnd,
            shape(Some(wave.clone())),
        ];
        let mut p = Paragraph::new();
        p.text = format!("é{}λ", OBJ.to_string().repeat(objects.len() + 1));
        p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: CharProps::default() }];
        p.objects = objects;
        let mut d = Document::from_text("");
        d.body = vec![Block::Para(p).into()];
        let item = |index, id| placed(StoryRef::Body, Path::top(0), 2 + index * OBJ.len_utf8(), id);
        let items = vec![
            item(5, Some(11)),
            item(0, Some(8)),
            item(1, Some(9)),
            item(2, Some(10)),
            item(3, Some(11)),
            item(3, Some(12)),
            item(5, None),
            item(6, Some(13)),
            placed(StoryRef::Body, Path::top(0), 3, Some(14)),
            placed(StoryRef::Body, Path::top(0), 0, Some(14)),
            placed(StoryRef::Body, Path::top(1), 2, Some(14)),
            placed(StoryRef::Part(7), Path::top(0), 2, Some(14)),
        ];
        let got = warped_boxes(&d, &items);
        assert_eq!(
            got,
            vec![
                (11, Rect::new(17.0, 2.0, 100.0, 30.0), wave),
                (11, Rect::new(11.0, 2.0, 100.0, 30.0), arch.clone()),
                (12, Rect::new(11.0, 2.0, 100.0, 30.0), arch)
            ]
        );
        // Extra stored objects without a matching marker are not addressable.
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.text.truncate(2 + 4 * OBJ.len_utf8());
        assert_eq!(warped_boxes(&d, &items), got[1..]);
        assert!(warped_boxes(&d, &[]).is_empty());
    }

    #[test]
    fn text_box_warp_lookups_keep_paragraph_and_story_keys_separate() {
        let presets = ["textArchUp", "textWave1", "textArchDown", "textSlantUp"];
        let para = |preset| {
            let mut p = Paragraph::new();
            p.text = OBJ.to_string();
            p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: CharProps::default() }];
            p.objects = vec![shape(TextWarp::new(preset))];
            p
        };
        let mut d = Document::from_text("");
        d.body = vec![Block::Para(para(presets[0])).into(), Block::Para(para(presets[1])).into()];
        let mut table = wordcraft_doc::Table::new(1, 1, 100.0);
        table.rows[0].cells[0].blocks = vec![Block::Para(para(presets[2])).into()];
        d.body.push(Block::Table(table).into());
        let id = d.add_part(wordcraft_doc::PartKind::Header, vec![Block::Para(para(presets[3])).into()]);
        let items = vec![
            placed(StoryRef::Body, Path::top(0), 0, Some(1)),
            placed(StoryRef::Body, Path::top(1), 0, Some(1)),
            placed(StoryRef::Body, Path(vec![2, 0, 0, 0]), 0, Some(1)),
            placed(StoryRef::Part(id), Path::top(0), 0, Some(1)),
        ];
        assert_eq!(warped_boxes(&d, &items), presets.map(|p| (1, Rect::new(0.0, 2.0, 100.0, 30.0), TextWarp::new(p).unwrap())));
    }

    /// #3614: deciding whether text boxes have a warp is part of every page repaint,
    /// including pages whose boxes are all plain. Measure discovery without shaping text.
    #[test]
    #[ignore = "performance regression; run explicitly"]
    fn text_box_warp_lookups_do_not_rescan_the_paragraph_prefix() {
        let make = |n: usize| {
            let mut objects = Vec::new();
            let mut items = Vec::new();
            for i in 0..n {
                objects.extend((0..16).map(|_| InlineObject::BookmarkEnd { name: "padding".into() }));
                items.push(placed(StoryRef::Body, Path::top(0), objects.len() * OBJ.len_utf8(), Some(i as u32)));
                objects.push(shape(None));
            }
            let mut p = Paragraph::new();
            p.text = OBJ.to_string().repeat(objects.len());
            p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: CharProps::default() }];
            p.objects = objects;
            let mut d = Document::from_text("");
            d.body = vec![Block::Para(p).into()];
            (d, items)
        };
        let (small, big) = (make(500), make(4000));
        let scan = |(doc, items): &(Document, Vec<Placed>)| assert!(std::hint::black_box(warped_boxes(doc, items)).is_empty());
        let (a, b, ratio) = crate::tests::cost_ratio(8, || scan(&small), || scan(&big));
        eprintln!("text-box warp discovery: {a:.6}s -> {b:.6}s, {ratio:.2}x for 8x boxes");
        assert!(ratio < 24.0, "text-box warp discovery rescanned paragraph prefixes: {ratio:.2}x");
    }

    /// #3614: an invalid request must not extend the prefix walk into unrelated trailing text.
    #[test]
    #[ignore = "performance regression; run explicitly"]
    fn text_box_warp_lookups_stop_at_the_last_valid_requested_offset() {
        let make = |suffix: usize| {
            let mut p = Paragraph::new();
            p.text = format!("{OBJ}{}", "x".repeat(suffix));
            p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: CharProps::default() }];
            p.objects = vec![shape(TextWarp::new("textArchUp"))];
            let mut d = Document::from_text("");
            d.body = vec![Block::Para(p).into()];
            d
        };
        let (short, long) = (make(0), make(1_000_000));
        let items = vec![
            placed(StoryRef::Body, Path::top(0), 0, Some(7)),
            placed(StoryRef::Body, Path::top(0), usize::MAX, Some(8)),
            placed(StoryRef::Body, Path::top(0), 1, Some(9)),
            placed(StoryRef::Body, Path::top(0), OBJ.len_utf8(), Some(10)),
        ];
        let expected = vec![(7, Rect::new(0.0, 2.0, 100.0, 30.0), TextWarp::new("textArchUp").unwrap())];
        assert_eq!(warped_boxes(&short, &items), expected);
        assert_eq!(warped_boxes(&long, &items), expected);
        let scan = |doc: &Document| {
            for _ in 0..1000 {
                std::hint::black_box(warped_boxes(doc, &items));
            }
        };
        let (a, b, _) = crate::tests::cost_ratio(1, || scan(&short), || scan(&long));
        eprintln!("text-box warp suffix: {a:.6}s -> {b:.6}s for 1000 calls with one million trailing bytes");
        assert!(b <= a * 8.0 + 0.005, "text-box warp lookup scanned the suffix: {a:.6}s -> {b:.6}s");
    }
}

#[cfg(test)]
mod compound_border_tests {
    use super::*;
    use crate::BorderSide;

    fn geometry(draws: &[Draw]) -> Vec<(f32, f32, f32, f32, f32)> {
        draws
            .iter()
            .map(|d| {
                let Draw::Line { x0, y0, x1, y1, width, stroke, color, alpha, .. } = d else { panic!("line") };
                assert_eq!(*stroke, Stroke::Solid);
                assert_eq!(*color, Rgb(12, 34, 56));
                assert_eq!(*alpha, 0.5);
                (*x0, *y0, *x1, *y1, *width)
            })
            .collect()
    }

    #[test]
    fn exact_double_strokes_keep_their_widths() {
        for side in [BorderSide::Top, BorderSide::Bottom] {
            let border = Border { color: Some(Rgb(12, 34, 56)), ..Default::default() }.double_widths([0.7, 0.2, 0.05]).unwrap();
            let mut out = Vec::new();
            rule((0.0, 10.0), (100.0, 10.0), side, &border, 0.5, &mut out);
            let lines = geometry(&out);
            assert_eq!(lines.len(), 2);
            for (line, offset, width) in [(lines[0], -0.45, 0.05), (lines[1], 0.125, 0.7)] {
                assert_eq!(line.4, width);
                assert!((line.1 - (10.0 + side.inward().1 * offset)).abs() < 1e-6);
                assert_eq!(line.1, line.3);
            }
            assert!(((lines[0].1 - lines[1].1).abs() - 0.575).abs() < 1e-6);
        }
        for (style, expected) in [
            (BorderStyle::DoubleWidths { inner: 255, gap: 255, outer: 255 }, vec![4.0, 4.0]),
            (BorderStyle::DoubleWidths { inner: 14, gap: 4, outer: 0 }, vec![0.7]),
            (BorderStyle::DoubleWidths { inner: 0, gap: 0, outer: 0 }, vec![]),
        ] {
            let mut out = Vec::new();
            rule((0.0, 0.0), (100.0, 0.0), BorderSide::Top, &Border { style, ..Default::default() }, 1.0, &mut out);
            let widths: Vec<_> = out
                .iter()
                .map(|draw| match draw {
                    Draw::Line { width, .. } => *width,
                    _ => panic!("line"),
                })
                .collect();
            assert_eq!(widths, expected);
        }
    }

    #[test]
    fn double_and_triple_keep_word_width_and_stroke_kind() {
        for (style, stroke) in [(BorderStyle::Double, Stroke::Double), (BorderStyle::Triple, Stroke::Triple)] {
            for width in [0.0_f32, 0.1, 0.5, 0.75, 2.0] {
                let mut out = Vec::new();
                rule((0., 10.), (100., 10.), BorderSide::Top, &Border { style, width, color: None, space: 0.0 }, 1.0, &mut out);
                assert!(
                    matches!(out.as_slice(), [Draw::Line { width: w, stroke: s, y0: 10.0, y1: 10.0, .. }] if *w == width.max(0.25) && *s == stroke)
                );
            }
        }
    }

    #[test]
    fn thin_thick_strokes_keep_a_minimum_width() {
        for width in [0.0, 0.01, f32::NAN, f32::INFINITY] {
            let mut out = Vec::new();
            rule(
                (0., 10.),
                (100., 10.),
                BorderSide::Top,
                &Border { style: BorderStyle::ThinThickSmallGap, width, color: None, space: 0.0 },
                1.0,
                &mut out,
            );
            assert_eq!(out.len(), 2);
            for draw in out {
                assert!(matches!(draw, Draw::Line { width, y0, y1, .. } if width >= 0.25 && width.is_finite() && y0.is_finite() && y1.is_finite()));
            }
        }
    }

    #[test]
    fn compound_borders_scale_strokes_and_gaps_by_model_width() {
        use BorderStyle::*;
        for (style, parts) in [
            (ThinThickSmallGap, vec![1., 1., 2.]),
            (ThickThinSmallGap, vec![2., 1., 1.]),
            (ThinThickMediumGap, vec![1., 2., 2.]),
            (ThickThinMediumGap, vec![2., 2., 1.]),
            (ThinThickLargeGap, vec![1., 4., 2.]),
            (ThickThinLargeGap, vec![2., 4., 1.]),
            (ThinThickThinSmallGap, vec![1., 1., 2., 1., 1.]),
            (ThinThickThinMediumGap, vec![1., 2., 2., 2., 1.]),
            (ThinThickThinLargeGap, vec![1., 4., 2., 4., 1.]),
        ] {
            let total: f32 = parts.iter().sum();
            let b = Border { style, width: 1.0 / 0.6, color: Some(Rgb(12, 34, 56)), space: 0.0 };
            let mut out = Vec::new();
            rule((0., 10.), (100., 10.), BorderSide::Top, &b, 0.5, &mut out);
            let mut edge = 10. - total / 2.;
            let mut expected = Vec::new();
            for (i, width) in parts.into_iter().enumerate() {
                if i % 2 == 0 {
                    expected.push((0., edge + width / 2., 100., edge + width / 2., width));
                }
                edge += width;
            }
            assert_eq!(geometry(&out), expected, "{style:?}");
        }
    }

    #[test]
    fn compound_stroke_order_comes_from_the_side_not_endpoint_direction() {
        let b = Border { style: BorderStyle::ThinThickSmallGap, width: 1.0 / 0.6, color: Some(Rgb(12, 34, 56)), space: 0.0 };
        for (side, from, to, expected) in [
            (BorderSide::Top, (0., 10.), (100., 10.), vec![(0., 8.5, 100., 8.5, 1.), (0., 11., 100., 11., 2.)]),
            (BorderSide::Bottom, (0., 10.), (100., 10.), vec![(0., 11.5, 100., 11.5, 1.), (0., 9., 100., 9., 2.)]),
            (BorderSide::Left, (10., 0.), (10., 100.), vec![(8.5, 0., 8.5, 100., 1.), (11., 0., 11., 100., 2.)]),
            (BorderSide::Right, (10., 0.), (10., 100.), vec![(11.5, 0., 11.5, 100., 1.), (9., 0., 9., 100., 2.)]),
        ] {
            let mut out = Vec::new();
            rule(from, to, side, &b, 0.5, &mut out);
            assert_eq!(geometry(&out), expected);
            out.clear();
            rule(to, from, side, &b, 0.5, &mut out);
            let reversed: Vec<_> = geometry(&out).into_iter().map(|(a, b, c, d, w)| (c, d, a, b, w)).collect();
            assert_eq!(reversed, expected);
        }
    }

    #[test]
    fn turned_compound_rules_keep_their_inward_side() {
        let doc = Document::new();
        let authors = Authors::new(&doc);
        let border = Border { style: BorderStyle::ThinThickSmallGap, width: 1.0 / 0.6, color: Some(Rgb(12, 34, 56)), space: 0.0 };
        for side in [BorderSide::Top, BorderSide::Bottom, BorderSide::Left, BorderSide::Right] {
            let original = Placed::Rule { side, x0: 0., y0: 0., x1: 100., y1: 0., border };
            let mut before = Vec::new();
            item_in(&doc, &original, &DisplayOptions::default(), &authors, 0.5, Spin::default(), &mut before);
            for turn in [TextDirection::Up, TextDirection::Down] {
                let mut placed = original.clone();
                placed.turn(turn, 200., 300.);
                let mut after = Vec::new();
                item_in(&doc, &placed, &DisplayOptions::default(), &authors, 0.5, Spin::default(), &mut after);
                let expected: Vec<_> = geometry(&before)
                    .into_iter()
                    .map(|(a, b, c, d, w)| {
                        let (a, b) = crate::turn_point(turn, 200., 300., a, b);
                        let (c, d) = crate::turn_point(turn, 200., 300., c, d);
                        (a, b, c, d, w)
                    })
                    .collect();
                assert_eq!(geometry(&after), expected);
            }
        }
    }
}
