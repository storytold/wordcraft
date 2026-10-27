use super::ctx::{Ctx, page_anchored};
use super::floats::{ObjFrame, WrapArea, align_in, float_rect, floating, left_out_objects, place_objects, rel_exclusions, wrap_area};
use super::page::PageFrame;
use super::placed::Placed;
use super::table_flow::below;
use super::*;

/// Cell identity is independent of optional table-style formatting.
pub(crate) enum BoxContext<'a> {
    Other,
    Cell(Option<&'a para::CellText>),
}

/// Lay out a block list into a free-standing box of `width` (no page breaks): table cells,
/// headers, footers, notes, text boxes. Returns items relative to (0, 0) and the height. `frame`
/// says where the box sits on its page, for floating objects positioned relative to the page or
/// its margins; without it (cells, notes, text boxes) they're positioned in the box.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_box(
    ctx: &mut Ctx,
    story: StoryRef,
    blocks: &Blocks,
    prefix: &[u32],
    width: f32,
    context: BoxContext<'_>,
    depth: usize,
    frame: Option<PageFrame>,
) -> BoxLayout {
    // Deletions in balloons leave the body's text only: a header, note or text box keeps its own
    // inline (restored for the story around this box when it's done).
    let outer_hide = std::mem::replace(&mut ctx.hide_deleted, ctx.opts.hides_deleted(story));
    let empty_cell = para::CellText::default();
    let (table, in_cell) = match context {
        BoxContext::Other => (None, false),
        BoxContext::Cell(table) => (Some(table.unwrap_or(&empty_cell)), true),
    };
    // Word 16.113.3: the override applies in modes 11/12/14, but not mode 15.
    let allow_same_style_space =
        in_cell && ctx.doc.settings.allow_space_of_same_style_in_table && matches!(ctx.doc.settings.compat_mode, 11 | 12 | 14);
    let mut items = Vec::new();
    // Pictures behind the text go first; `behind` counts them.
    let mut behind = 0;
    // Wrap areas of the box's floating objects.
    let mut excl: Vec<WrapArea> = Vec::new();
    // Areas of the box's floating tables, which other floating tables may have to avoid.
    let mut float_tables: Vec<Rect> = Vec::new();
    // Deepest a floating picture or shape that takes part in the text flow reaches, so the box
    // (a table cell especially) grows to hold it instead of letting it hang past the box's own
    // bottom edge (#3108). Behind-text and in-front-of-text objects don't: a page-anchored logo or
    // watermark behind a cell's text must not stretch its row (see `align_h` in table.rs).
    let mut float_objects_bottom = 0.0f32;
    let mut table_areas = Vec::new();
    let mut page_table_wraps = Vec::new();
    let mut y = 0.0f32;
    let mut prev_after = 0.0f32;
    let mut prev_after_auto = false;
    let mut prev_listed = false;
    let mut prev_style: Option<(String, bool)> = None;
    let mut last_top = 0.0f32;
    // Word has no frames in a text box: a framed paragraph there (its style's frame, usually)
    // runs with the box's text (#2357).
    let frames = !matches!(story, StoryRef::Part(id) if ctx.doc.parts.get(&id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox));
    // The bottom of the last paragraph's lines.
    let mut prev_end = 0.0f32;
    for (i, b) in blocks.iter().enumerate() {
        last_top = y;
        let mut path = prefix.to_vec();
        path.push(i as u32);
        match &**b {
            Block::Para(p) => {
                let label = ctx.next_label(p);
                let listed = label.is_some();
                let mut pl = ctx.para_labelled(p, width, table, &[], label.clone());
                // A text frame (a footer's page number in a frame, say) is placed apart, like a
                // floating object, and the text after it wraps around it; it takes no room in
                // the flow.
                if let Some(fr) = pl.rp.frame.clone().filter(|_| frames) {
                    let fw = fr.w.unwrap_or_else(|| natural_width(&pl)).clamp(1.0, 1584.0);
                    let pf = if (fw - width).abs() > 0.01 { ctx.para_labelled(p, fw, table, &[], label) } else { pl };
                    let h = match fr.h {
                        Some(h) if fr.h_exact => h,
                        Some(h) => h.max(pf.height),
                        None => pf.height,
                    };
                    let float = frame_float(&fr);
                    let r = float_rect(frame, (0.0, width), y, fw, h, &float);
                    push_para(&mut items, story, &path, &pf, 0, pf.lines.len(), r.x, r.y, fw, Edges::own(&pf, 0, pf.lines.len()));
                    excl.extend(wrap_area(r, &float, None));
                    float_tables.push(r);
                    continue;
                }
                // A legacy table compatibility flag lets same-style paragraphs in a table keep their
                // spacing (`allow_same_style_space`).
                let ctxl = pl.rp.contextual_spacing && !allow_same_style_space;
                let mut before = pl.rp.space_before;
                if let Some((style, previous_contextual)) = &prev_style
                    && *style == pl.rp.style
                    && !allow_same_style_space
                {
                    if *previous_contextual {
                        y -= prev_after;
                        before = (before - prev_after).max(0.0);
                        prev_after = 0.0;
                    }
                    if ctxl {
                        before = 0.0;
                    }
                }
                // Automatic spacing: none between two list items (as in an HTML list).
                if i > 0 && prev_listed && listed {
                    if prev_after_auto {
                        y -= prev_after;
                        prev_after = 0.0;
                    }
                    if pl.rp.before_auto {
                        before = 0.0;
                    }
                }
                // Automatic spacing: none at the top of a cell or box, and it collapses with the
                // space after the paragraph above (the larger one counts).
                if pl.rp.before_auto && i == 0 {
                    before = 0.0;
                } else if i > 0 && before > 0.0 && prev_after > 0.0 && !ctx.doc.settings.sum_paragraph_spacing {
                    before = (before - prev_after).max(0.0);
                }
                // Paragraph-relative floats start above the effective space before, as in the
                // body path. Keep text and wrap exclusions relative to the text's actual top.
                let para_top = y;
                y += if i == 0 { before } else { before.max(0.0) };
                // Floating objects anchored here: place them, then wrap the text around them.
                // One left out of the layout (hidden, or deleted in the final text) takes no room.
                let mut floats = HashMap::new();
                // A header or footer cell's floats placed on the page: no room taken in the cell.
                let mut pinned = HashMap::new();
                let left_out = left_out_objects(ctx.doc, p, table.map(|t| &t.chr), ctx.opts.show_hidden, ctx.hide_deleted);
                for (oi, o) in p.objects.iter().enumerate() {
                    let Some((w, h, float)) = floating(o) else { continue };
                    if left_out(oi) {
                        continue;
                    }
                    if let Some(pin) = ctx.pin.as_ref().filter(|pin| in_cell && pin.story == story && float.outside_cell && page_anchored(float)) {
                        let page = PageFrame { sect: &pin.sect, origin: (0.0, 0.0), number: pin.number, mirror: pin.mirror };
                        pinned.insert(oi, float_rect(Some(page), (0.0, width), para_top, w, h, float));
                        continue;
                    }
                    // Only paragraph-relative placement excludes space-before. Line anchors
                    // and frameless page/margin fallbacks retain the text-top origin.
                    let float_y = if float.v_rel == Anchor::Paragraph { para_top } else { y };
                    let r = float_rect(frame, (0.0, width), float_y, w, h, float);
                    let outline = ctx.wrap_outline(o);
                    excl.extend(wrap_area(r, float, outline.as_deref().map(Vec::as_slice)));
                    floats.insert(oi, r);
                }
                let rel = rel_exclusions(&excl, 0.0, y);
                if !rel.is_empty() {
                    pl = ctx.para_labelled(p, width, table, &rel, label);
                }
                // Bordered paragraphs: room for their borders, those of a group joined in one box.
                let cell = table.map(|t| &t.para);
                let (above, below) = (joined(ctx.doc, blocks, i, i.checked_sub(1), cell), joined(ctx.doc, blocks, i, Some(i + 1), cell));
                let mut edges = Edges::own(&pl, 0, pl.lines.len());
                if above {
                    edges.top = 0.0;
                }
                if below {
                    edges.bottom = 0.0;
                }
                let lead = pl.lines.first().map_or(0.0, |l| l.top.max(0.0)) + edges.top;
                if above {
                    edges.up = (y + lead - prev_end).max(0.0);
                }
                prev_end = push_para(&mut items, story, &path, &pl, 0, pl.lines.len(), 0.0, y + lead, width, edges);
                // Pictures, shapes and text boxes: drawn, their areas and text boxes' text.
                if !p.objects.is_empty() {
                    let at = ObjFrame { page: frame, col: (0.0, width), para_y: para_top, table_chr: table.map(|t| &t.chr), pinned: Some(&pinned) };
                    let (back, front) = place_objects(ctx, story, &path, p, &pl, (0, pl.lines.len()), (0.0, y + lead), &at, &floats, depth);
                    for placed in back.iter().chain(front.iter()) {
                        if let Placed::Object { rect, spin, wrap, .. } = placed
                            && !matches!(wrap, Wrap::Inline | Wrap::BehindText | Wrap::InFrontOfText)
                        {
                            float_objects_bottom = float_objects_bottom.max(spin.bounds(*rect).bottom());
                        }
                    }
                    let at = behind.min(items.len());
                    behind += back.len();
                    items.splice(at..at, back);
                    items.extend(front);
                }
                // Automatic space after: none after the last paragraph of a cell or box.
                let after = if pl.rp.after_auto && i + 1 == blocks.len() { 0.0 } else { pl.rp.space_after };
                y += edges.top + pl.height + edges.bottom + after;
                prev_after = after;
                prev_after_auto = pl.rp.after_auto;
                prev_listed = listed;
                prev_style = Some((pl.rp.style.clone(), ctxl));
            }
            Block::Table(t) => {
                if depth > 8 {
                    continue;
                }
                // Page tables do not contribute to the footer text's intrinsic height.
                if frame.is_none()
                    && !in_cell
                    && matches!(story, StoryRef::Part(id) if ctx.doc.parts.get(&id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::Footer))
                    && t.props.float.is_some_and(|f| f.v_rel == Anchor::Page)
                {
                    continue;
                }
                let tl = table::layout_table(ctx, story, t, &path, width, depth + 1);
                let (tx, ty) = match t.props.float {
                    // A floating table stands at its own position and takes no room; the text
                    // after it wraps around it.
                    Some(f) => {
                        let legacy = if ctx.doc.settings.compat_mode < 15 { table::first_cell_left_margin(ctx, t) } else { 0.0 };
                        let (r, area) = box_float_table_rect(frame, width, y, &tl, &f, legacy, &float_tables);
                        excl.push((area, false, None, WrapText::BothSides, true));
                        float_tables.push(area);
                        table_areas.push(area);
                        if f.v_rel == Anchor::Page {
                            page_table_wraps.push((area, false, None, WrapText::BothSides, true));
                        }
                        (r.x, r.y)
                    }
                    // A table in the text flow starts below the floating objects in its way.
                    None => (tl.x, below(&excl, y, tl.x, tl.x + tl.width)),
                };
                let mut row_y = ty;
                for row in &tl.rows {
                    for it in &row.items {
                        let mut it = it.clone();
                        it.translate(tx, row_y);
                        items.push(it);
                    }
                    row_y += row.height;
                }
                if t.props.float.is_none() {
                    y = row_y;
                    prev_after = 0.0;
                    prev_style = None;
                }
            }
        }
    }
    let floats_bottom = float_tables.iter().map(|r| r.bottom()).fold(float_objects_bottom, f32::max);
    ctx.hide_deleted = outer_hide;
    BoxLayout { items, height: y.max(0.0), floats_bottom, last_top: last_top.max(0.0), table_areas, page_table_wraps }
}

/// The width a text frame without one takes: its widest line, with the paragraph's indents.
fn natural_width(pl: &ParaLayout) -> f32 {
    let widest = (0..pl.lines.len())
        .filter_map(|li| {
            let l = pl.lines.get(li)?;
            Some((pl.x_of(li, l.stop)? - pl.x_of(li, l.start)?).abs())
        })
        .fold(0.0f32, f32::max);
    widest + pl.rp.indent_left.max(0.0) + pl.rp.indent_right.max(0.0)
}

/// A text frame's placement as a floating object's (`w:framePr` → anchors, alignment, wrap).
fn frame_float(fr: &wordcraft_doc::props::TextFrame) -> Float {
    let anchor = |a: &str, text: Anchor| match a {
        "page" => Anchor::Page,
        "margin" => Anchor::Margin,
        _ => text,
    };
    let align = |a: Option<&str>| match a {
        Some("left" | "top") => Some(FloatAlign::Start),
        Some("center") => Some(FloatAlign::Center),
        Some("right" | "bottom") => Some(FloatAlign::End),
        Some("inside") => Some(FloatAlign::Inside),
        Some("outside") => Some(FloatAlign::Outside),
        _ => None,
    };
    Float {
        wrap: match fr.wrap.as_str() {
            "notBeside" => Wrap::TopAndBottom,
            "none" => Wrap::InFrontOfText,
            "tight" => Wrap::Tight,
            "through" => Wrap::Through,
            _ => Wrap::Square,
        },
        h_rel: anchor(&fr.h_anchor, Anchor::Column),
        v_rel: anchor(&fr.v_anchor, Anchor::Paragraph),
        x: fr.x,
        y: fr.y,
        h_align: align(fr.x_align.as_deref()),
        v_align: align(fr.y_align.as_deref()),
        dist: fr.h_space,
        dist_top: fr.v_space,
        dist_bottom: fr.v_space,
        ..Default::default()
    }
}

/// A laid-out box: items relative to its top-left, the height of its text flow, where its
/// floating tables end (a table cell grows to hold them) and where its last block starts (the
/// height without it).
pub(crate) struct BoxLayout {
    pub(crate) items: Vec<Placed>,
    pub(crate) table_areas: Vec<Rect>,
    pub(crate) page_table_wraps: Vec<WrapArea>,
    pub(crate) height: f32,
    pub(crate) floats_bottom: f32,
    pub(crate) last_top: f32,
}

/// Where floating table `tl` goes in a box `width` wide whose next paragraph starts at `y`, and
/// the area text keeps clear of. Positions relative to the page or its margins need the box's
/// `frame`; without one (a table cell) they're relative to the box.
fn box_float_table_rect(
    frame: Option<PageFrame>,
    width: f32,
    y: f32,
    tl: &table::TableLayout,
    f: &TableFloat,
    legacy: f32,
    float_tables: &[Rect],
) -> (Rect, Rect) {
    let h = tl.rows.iter().map(|r| r.height).sum::<f32>().clamp(0.0, 100_000.0);
    let w = tl.width.clamp(1.0, 100_000.0);
    // Across, as laid out before an even page is mirrored (see `float_rect`).
    let page_x = |fr: &PageFrame| {
        let (left, _, shift) = fr.sides();
        match f.h_rel {
            Anchor::Page => Some((-shift, fr.sect.page_w)),
            Anchor::Margin => Some((left - shift, fr.sect.text_width())),
            _ => None,
        }
    };
    let page_y = |s: &SectionProps| match f.v_rel {
        Anchor::Page => Some((0.0, s.page_h)),
        Anchor::Margin => Some((s.margin_top, s.text_height())),
        _ => None,
    };
    let h_area = frame.and_then(|fr| page_x(&fr).map(|(a, e)| (a - fr.origin.0, e))).unwrap_or((0.0, width));
    // In a cell an offset from the page or margin has nothing to measure from: the table stands
    // where the text is.
    let (v_area, y_off) = match frame.and_then(|fr| page_y(fr.sect).map(|(a, e)| (a - fr.origin.1, e))) {
        Some(area) => (area, f.y),
        None if matches!(f.v_rel, Anchor::Page | Anchor::Margin) => ((y, 0.0), 0.0),
        None => ((y, 0.0), f.y),
    };
    // Nor is there an area to align in: the table starts where the text is.
    let v_align = if frame.is_none() && matches!(f.v_rel, Anchor::Page | Anchor::Margin) { None } else { f.v_align };
    let x = align_in(h_area, f.x, w, PageFrame::h_align(frame.as_ref(), f.h_align)) - if f.h_align.is_none() { legacy } else { 0.0 };
    let r = clear_of_float_tables(Rect::new(x, align_in(v_area, y_off, h, v_align), w, h), f, legacy, h_area.0, float_tables);
    let [dl, dt, dr, db] = f.dist_from_text();
    (r, r.inset(-dl, -dt, -dr, -db))
}

/// Floating table `f` at `r`, moved below the floating tables in its way when it may not
/// overlap them. In Word 2013+ layout one placed by an offset keeps its distance from the text's
/// left edge (`text_left`).
pub(crate) fn clear_of_float_tables(mut r: Rect, f: &TableFloat, legacy: f32, text_left: f32, others: &[Rect]) -> Rect {
    let [dl, dt, dr, db] = f.dist_from_text();
    let keeps_text_distance = legacy == 0.0 && f.h_align.is_none() && f.x >= 0.0 && matches!(f.h_rel, Anchor::Margin | Anchor::Column);
    for _ in 0..64 {
        let area = r.inset(-dl, -dt, -dr, -db);
        let Some(below) = others.iter().filter(|o| !f.overlap && o.intersects(&area)).map(|o| o.bottom()).reduce(f32::max) else { break };
        r.y = below + dt;
        if keeps_text_distance {
            r.x = r.x.max(text_left + dl);
        }
    }
    r
}

/// A text box's default internal margins (Word's: 0.1" left/right, 0.05" top/bottom); a box's
/// own come from its [`wordcraft_doc::TextBody`].
#[cfg(test)]
pub(crate) const BOX_INSET_X: f32 = 7.2;

#[cfg(test)]
pub(crate) const BOX_INSET_Y: f32 = 3.6;

/// Keep what fits in a box `height` tall (items relative to its top): lines that don't fit are
/// dropped (whole lines; a line that only partly fits too) and so is anything starting below.
/// The first line always stays, so even a tiny box shows the start of its text.
pub(crate) fn fit_box(items: Vec<Placed>, height: f32) -> Vec<Placed> {
    let limit = height + 0.5;
    let mut out = Vec::with_capacity(items.len());
    let mut kept_line = false;
    for it in items {
        if let Some(r) = it.turned_bounds() {
            // Turned text (in a table cell) stays whole.
            if r.y < limit {
                out.push(it);
            }
            continue;
        }
        match it {
            Placed::Lines { story, path, para, l0, l1, x, y, turn } => {
                let Some(first) = para.lines.get(l0) else { continue };
                let mut end = l0;
                for k in l0..l1 {
                    let Some(l) = para.lines.get(k) else { break };
                    if y + (l.top - first.top) + l.height > limit && kept_line {
                        break;
                    }
                    end = k + 1;
                    kept_line = true;
                }
                if end > l0 {
                    out.push(Placed::Lines { story, path, para, l0, l1: end, x, y, turn });
                }
            }
            Placed::Fill { rect, color } if rect.y < limit => {
                out.push(Placed::Fill { rect: Rect::new(rect.x, rect.y, rect.w, rect.h.min(limit - rect.y)), color });
            }
            Placed::Rule { x0, y0, x1, y1, border } if y0.min(y1) < limit => {
                out.push(Placed::Rule { x0, y0: y0.min(limit), x1, y1: y1.min(limit), border });
            }
            Placed::Image { rect, .. }
            | Placed::Shape { rect, .. }
            | Placed::Graphic { rect, .. }
            | Placed::Clip { rect, .. }
            | Placed::Cell { rect, .. }
            | Placed::Object { rect, .. }
                if rect.y < limit =>
            {
                out.push(it);
            }
            _ => {}
        }
    }
    out
}

/// Word joins neighbouring paragraphs with the same borders and indents into one box: the top
/// border goes above the first of them only, the bottom one below the last, and the shading and
/// side borders run on through the space between them (`table`: the cell's paragraph properties).
fn border_group(doc: &Document, a: &Paragraph, b: &Paragraph, table: Option<&wordcraft_doc::props::ParaProps>) -> bool {
    let (a, b) = (doc.styles.resolve_para_in(&a.props, table), doc.styles.resolve_para_in(&b.props, table));
    a.borders.is_some() && a.borders == b.borders && a.indent_left == b.indent_left && a.indent_right == b.indent_right
}

/// Whether paragraph `i` of `blocks` and the one at `j` are of one border group.
pub(crate) fn joined(doc: &Document, blocks: &Blocks, i: usize, j: Option<usize>, table: Option<&wordcraft_doc::props::ParaProps>) -> bool {
    match (blocks.get(i).map(|b| &**b), j.and_then(|j| blocks.get(j)).map(|b| &**b)) {
        (Some(Block::Para(a)), Some(Block::Para(b))) => border_group(doc, a, b, table),
        _ => false,
    }
}

/// The room a paragraph's top or bottom border takes in the text flow: its distance from the
/// text plus its width (Word).
fn border_room(b: Option<Border>) -> f32 {
    b.filter(Border::is_visible).map_or(0.0, |b| b.space + b.width)
}

/// How a piece of a paragraph meets the paragraphs around it: the room taken by its top and
/// bottom borders (0: none drawn there — the paragraph goes on from or into one of its border
/// group, or it is split there) and how far its shading and side borders reach up to join the
/// paragraph above in its group.
#[derive(Clone, Copy, Default)]
pub(crate) struct Edges {
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    pub(crate) up: f32,
}

impl Edges {
    /// A paragraph on its own: its top and bottom borders at its ends.
    pub(crate) fn own(pl: &ParaLayout, l0: usize, l1: usize) -> Edges {
        let b = pl.rp.borders.unwrap_or_default();
        Edges { top: if l0 == 0 { border_room(b.top) } else { 0.0 }, bottom: if l1 >= pl.lines.len() { border_room(b.bottom) } else { 0.0 }, up: 0.0 }
    }
}

/// Paragraph lines plus their shading and borders; returns the bottom of the lines. The
/// shading fills the box out to the borders, the room they take included (Word).
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_para(
    items: &mut Vec<Placed>,
    story: StoryRef,
    path: &[u32],
    pl: &Arc<ParaLayout>,
    l0: usize,
    l1: usize,
    x: f32,
    y: f32,
    width: f32,
    e: Edges,
) -> f32 {
    let (Some(first), Some(last)) = (pl.lines.get(l0), l1.checked_sub(1).and_then(|k| pl.lines.get(k))) else {
        return y;
    };
    let h = last.top + last.height - first.top;
    // Indents are logical: a right-to-left paragraph's start indent (and hanging indent) is on the right.
    let start = pl.rp.indent_left.min(pl.rp.indent_left + pl.rp.indent_first);
    let (li, ri) = if pl.rp.bidi { (pl.rp.indent_right, start) } else { (start, pl.rp.indent_right) };
    let left = x + li;
    let right = x + width - ri;
    let (top, bottom) = (y - e.top - e.up, y + h + e.bottom);
    if let Some(c) = pl.rp.shading {
        let pad = pl.rp.borders.as_ref().map(|b| b.left.map(|l| l.space).unwrap_or(4.0)).unwrap_or(0.0);
        items.push(Placed::Fill { rect: Rect::new(left - pad, top, right - left + pad * 2.0, bottom - top), color: c });
    }
    items.push(Placed::Lines { story, path: Path(path.to_vec()), para: pl.clone(), l0, l1, x, y, turn: TextDirection::Horizontal });
    if let Some(b) = &pl.rp.borders {
        let sp = |o: &Option<Border>| o.map(|b| b.space).unwrap_or(0.0);
        let (lx, rx) = (left - sp(&b.left), right + sp(&b.right));
        // The top and bottom lines lie beyond their distance from the text.
        if let Some(t) = b.top.filter(|t| e.top > 0.0 && t.is_visible()) {
            let ty = y - t.space - t.width / 2.0;
            items.push(Placed::Rule { x0: lx, y0: ty, x1: rx, y1: ty, border: t });
        }
        if let Some(t) = b.bottom.filter(|t| e.bottom > 0.0 && t.is_visible()) {
            let by = y + h + t.space + t.width / 2.0;
            items.push(Placed::Rule { x0: lx, y0: by, x1: rx, y1: by, border: t });
        }
        if let Some(t) = b.left.filter(Border::is_visible) {
            items.push(Placed::Rule { x0: lx, y0: top, x1: lx, y1: bottom, border: t });
        }
        if let Some(t) = b.right.filter(Border::is_visible) {
            items.push(Placed::Rule { x0: rx, y0: top, x1: rx, y1: bottom, border: t });
        }
    }
    y + h
}
