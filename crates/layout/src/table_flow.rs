use super::boxes::{BoxInput, clear_of_float_tables, layout_box};
use super::ctx::{Ctx, with_pct_base};
use super::floats::{WrapArea, align_in, body_wraps};
use super::flow::again;
use super::headers::{MAX_HEADER_ITEMS, header_table_areas};
use super::notes::table_notes;
use super::page::{PageBuilder, PageFrame, page_frame};
use super::placed::Placed;
use super::{Anchor, Arc, HeightRule, Path, Rect, StoryRef, TableFloat, ViewMode, WrapText, table};

/// Footer page tables wrap only the intersecting body lines, which resume below them.
pub(crate) fn footer_table_wraps(ctx: &mut Ctx, pb: &PageBuilder) -> Vec<WrapArea> {
    if pb.web || ctx.opts.view == ViewMode::Read {
        return Vec::new();
    }
    let Some(page) = pb.pages.last() else { return Vec::new() };
    let first = pb.pages.iter().rev().nth(1).is_none_or(|prev| prev.section != page.section);
    let key = (page.section, page.number, first);
    if let Some(areas) = ctx.footer_wraps.get(&key) {
        return areas.clone();
    }
    ctx.footer_wraps.insert(key, Vec::new());
    let doc = ctx.doc;
    let hf = Arc::clone(&ctx.hf);
    let Some((_, sect)) = hf.sections.get(page.section) else { return Vec::new() };
    if sect.text_direction.is_turned() {
        return Vec::new();
    }
    let set = hf.feet.set(page.section);
    let id = if sect.title_page && first {
        set.first
    } else if doc.settings.even_odd_headers && page.number.is_multiple_of(2) {
        set.even.or(set.default)
    } else {
        set.default
    };
    let Some(id) = id else { return Vec::new() };
    let Some(part) = doc.parts.get(&id) else { return Vec::new() };
    let blocks = part.blocks.clone();
    let x = sect.margin_left + sect.gutter;
    let base = hf.defining_width(true, page.section, Some(id));
    let counters = std::mem::take(&mut ctx.counters);
    let boxes = std::mem::take(&mut ctx.boxes);
    let chains = std::mem::take(&mut ctx.chains);
    let fields = ctx.fields.clone();
    let grid = std::mem::take(&mut ctx.grid);
    let track = std::mem::replace(&mut ctx.track_totals, false);
    ctx.fields.page = page.number;
    let areas = again(ctx, 0, |ctx| {
        with_pct_base(ctx, base, |ctx| {
            let story = StoryRef::Part(id);
            let h = layout_box(ctx, BoxInput::other(story, &blocks, sect.text_width(), 0, None)).height;
            let y = sect.page_h - sect.footer - h;
            let frame = PageFrame { sect, origin: (x, y), number: page.number, mirror: ctx.mirror };
            let mut areas = layout_box(ctx, BoxInput::other(story, &blocks, sect.text_width(), 0, Some(frame))).page_table_wraps;
            for (rect, ..) in &mut areas {
                rect.x += x;
                rect.y += y;
            }
            areas
        })
    });
    ctx.counters = counters;
    ctx.boxes = boxes;
    ctx.chains = chains;
    ctx.fields = fields;
    ctx.grid = grid;
    ctx.track_totals = track;
    ctx.footer_wraps.insert(key, areas.clone());
    areas
}

fn row_keeps_next(row: &table::RowLayout) -> bool {
    let Some(items) = row.cells.first().and_then(|cell| row.items.get(cell.items.clone())) else { return false };
    let Some((depth, story)) = items
        .iter()
        .filter_map(|item| match item {
            Placed::Cell { table, story, .. } => Some((table.0.len() + 3, *story)),
            _ => None,
        })
        .min_by_key(|(depth, _)| *depth)
    else {
        return false;
    };
    // Word takes the row's keep flag from the first paragraph of its first cell.
    items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { path, story: own, para, .. } if path.0.len() == depth && path.0.last() == Some(&0) && *own == story => {
                Some(para.rp.keep_next)
            }
            _ => None,
        })
        .unwrap_or(false)
}

fn row_can_start(row: &wordcraft_doc::Row, laid: &table::RowLayout, room: f32, page_height: f32) -> bool {
    if laid.height <= room + 0.01 {
        return true;
    }
    if row.props.header || row.props.cant_split || (row.props.height_rule == HeightRule::Exact && row.props.height.is_some_and(|h| h > 0.0)) {
        return false;
    }
    if row.props.height_rule == HeightRule::AtLeast && row.props.height.is_some_and(|h| h > room + 0.01 && h <= page_height + 0.01) {
        return false;
    }
    table::split_row(laid, room).is_some()
}

pub(crate) fn place_table(ctx: &mut Ctx, pb: &mut PageBuilder, t: &wordcraft_doc::Table, block: usize, body_top: f32) {
    pb.prev = None;
    let width = pb.col_w();
    let mut tl = table::layout_table(ctx, StoryRef::Body, t, &[block as u32], width, 1);
    if let Some(f) = t.props.float.filter(|_| !pb.web) {
        // Before Word 2013 layout (compatibility mode 15) an offset places the first cell's text,
        // so the edge sits a cell margin further out.
        let legacy = if ctx.doc.settings.compat_mode < 15 { table::first_cell_left_margin(ctx, t) } else { 0.0 };
        let h: f32 = tl.rows.iter().map(|r| r.height).sum();
        if h <= pb.uncapped_bottom() - pb.top + 0.01 {
            place_floating_table(ctx, pb, &tl, &f, legacy, block, body_top);
            return;
        }
        // Too tall for a page: Word lets it run across pages like a table in the text, from its
        // own left edge.
        tl.x = float_table_x(pb, tl.width, &f, legacy, ctx.mirror) - pb.col_x();
    }
    // A table in the text flow starts below the floating objects in its way.
    below_floats(ctx, pb, pb.col_x() + tl.x, pb.col_x() + tl.x + tl.width);
    let header_rows: Vec<usize> = (0..t.rows.len()).take_while(|r| t.rows.get(*r).is_some_and(|row| row.props.header)).collect();
    // Items repeated so far, and what one more repeat of the header rows costs (see
    // `MAX_HEADER_ITEMS`).
    let mut header_repeats = 0usize;
    let header_cost = header_rows.iter().filter_map(|i| tl.rows.get(*i)).map(|r| r.items.len().max(1)).fold(0, usize::saturating_add);
    // Resolve kept chains once; nested paragraphs cannot keep their enclosing row.
    let mut kept: Vec<(usize, f32)> = (0..tl.rows.len()).map(|i| (i, 0.0)).collect();
    for i in (0..tl.rows.len().saturating_sub(1)).rev() {
        let Some(row) = tl.rows.get(i) else { continue };
        if i < header_rows.len() || !row_keeps_next(row) {
            continue;
        }
        let Some((end, height)) = kept.get(i + 1).copied() else { continue };
        if let Some(group) = kept.get_mut(i) {
            *group = (end, row.height + height);
        }
    }
    let repeated_height: f32 = header_rows.iter().filter_map(|i| tl.rows.get(*i)).map(|r| r.height).sum();
    let place = |pb: &mut PageBuilder, row: &table::RowLayout| {
        let (x, y) = (pb.col_x() + tl.x, pb.y);
        if let Some(pg) = pb.page() {
            for it in &row.items {
                let mut it = it.clone();
                it.translate(x, y);
                pg.items.push(it);
            }
        }
        pb.y += row.height;
        pb.content_bottom = pb.content_bottom.max(pb.y);
    };
    // Measured in Word: an at-least row's minimum height is reserved, not split. Where less room
    // than that is left on a page that already holds something, the row starts the next page
    // whole; a row taller than a page then splits from the top of that page.
    let short_of_min = |ri: usize, room: f32| {
        t.rows.get(ri).is_some_and(|r| r.props.height_rule == HeightRule::AtLeast && r.props.height.is_some_and(|h| h > room + 0.01))
    };
    // Like Word, repeated header rows don't stay alone at the bottom of a page: when the first row
    // after them can't start under them there, the table starts on the next page.
    let first = header_rows.len();
    if !header_rows.is_empty()
        && !pb.at_top()
        && let Some(row) = tl.rows.get(first)
    {
        let room = pb.bottom - pb.y - tl.rows.iter().take(first).map(|r| r.height).sum::<f32>();
        let splittable = !t.rows.get(first).is_some_and(|r| r.props.cant_split);
        if row.height + row.edge > room + 0.01 && (short_of_min(first, room) || !(splittable && table::split_row(row, room).is_some())) {
            pb.advance(block, body_top);
        }
    }
    // For the move-on rule, the table's own header rows at the top of a page leave it empty.
    let table_at_top = pb.at_top();
    for (ri, row) in tl.rows.iter().enumerate() {
        // Word never splits a row of an exact height: it moves on whole.
        let splittable = !header_rows.contains(&ri)
            && !t
                .rows
                .get(ri)
                .is_some_and(|r| r.props.cant_split || (r.props.height_rule == HeightRule::Exact && r.props.height.is_some_and(|h| h > 0.0)));
        let mut rest: Option<table::RowLayout> = None;
        // On a new page holding only the repeated header rows.
        let mut fresh = false;
        // Nothing but this table's header rows above it on a page it started at the top of.
        let under_top_headers = ri == first && table_at_top;
        // Each pass places the part of the row that fits, then breaks the page.
        for _ in 0..1000 {
            let cur = rest.as_ref().unwrap_or(row);
            // A fitting kept row follows the next row when that row cannot start below it.
            // Move only the prefix's start; an oversized follower cannot separate its kept rows.
            let defer_kept = pb.y + cur.height <= pb.bottom + 0.01
                && !pb.web
                && !pb.at_top()
                && !fresh
                && rest.is_none()
                && ri.checked_sub(1).and_then(|prev| kept.get(prev)).is_none_or(|&(end, _)| end < ri)
                && kept.get(ri).is_some_and(|&(end, height)| {
                    if end == ri {
                        return false;
                    }
                    let (Some(next), Some(laid)) = (t.rows.get(end), tl.rows.get(end)) else { return false };
                    let page_height = pb.bottom - body_top - repeated_height;
                    height <= page_height + 0.01 && !row_can_start(next, laid, pb.bottom - pb.y - height, page_height)
                });
            // A row another row follows needs room for the border below it too (measured in Word).
            if pb.y + cur.height + cur.edge <= pb.bottom + 0.01 && !defer_kept {
                break;
            }
            // An at-least row with less room than its minimum moves on whole, unless nothing
            // is above it on this page (moving on would gain nothing).
            let room = pb.bottom - pb.y;
            let moves_on = rest.is_none() && !pb.at_top() && !fresh && !under_top_headers && short_of_min(ri, room);
            let split = if splittable && !moves_on && !defer_kept { table::split_row(cur, room) } else { None };
            match split {
                Some((a, b)) => {
                    place(pb, &a);
                    table_notes(ctx, pb, &a.items);
                    rest = Some(b);
                }
                // Like Word, a row that does not fit on an empty page, or under the header rows
                // repeated there, is placed anyway and runs into the bottom margin.
                None if pb.at_top() || fresh => break,
                None => {}
            }
            pb.advance(block, body_top);
            // Repeat header rows, up to this table's cap.
            if !header_rows.contains(&ri) && header_repeats.saturating_add(header_cost) <= MAX_HEADER_ITEMS {
                header_repeats += header_cost;
                for h in header_rows.iter().filter_map(|hr| tl.rows.get(*hr)) {
                    place(pb, h);
                }
            }
            fresh = true;
        }
        let last = rest.as_ref().unwrap_or(row);
        place(pb, last);
        table_notes(ctx, pb, &last.items);
    }
}

/// Place floating table `tl` (`w:tblpPr`): at its own position, which takes no room in the text
/// flow; the text after it wraps around it.
fn place_floating_table(ctx: &mut Ctx, pb: &mut PageBuilder, tl: &table::TableLayout, f: &TableFloat, legacy: f32, block: usize, body_top: f32) {
    let h: f32 = tl.rows.iter().map(|r| r.height).sum::<f32>().clamp(0.0, 100_000.0);
    let w = tl.width.clamp(1.0, 100_000.0);
    let [dl, dt, dr, db] = f.dist_from_text();
    for attempt in 0..2 {
        let s = pb.sect;
        let h_area = float_table_h_area(pb, f, ctx.mirror);
        // The text a table anchored to it stands in: where the next paragraph starts.
        let v_area = match f.v_rel {
            Anchor::Page => (0.0, s.page_h),
            Anchor::Margin => (s.margin_top, s.text_height()),
            _ => (pb.y, 0.0),
        };
        let x = float_table_x(pb, w, f, legacy, ctx.mirror);
        let mut obstacles = if f.overlap { Vec::new() } else { header_table_areas(ctx, pb) };
        obstacles.extend_from_slice(&pb.float_tables);
        let r = clear_of_float_tables(Rect::new(x, align_in(v_area, f.y, h, f.v_align), w, h), f, legacy, h_area.0, &obstacles);
        // Like Word, a table that doesn't fit below the text before it moves to the next page with
        // its anchor: one placed relative to the paragraph when it would run off the page, one
        // placed on the page or margin when it would also cover the text already on the page.
        let moves = if f.v_rel == Anchor::Paragraph {
            r.bottom() > pb.bottom + 0.01
        } else {
            let covers_text = pb.page().is_some_and(|page| page.items.iter().any(|item| placed_intersects(item, &r)));
            covers_text && pb.y + h > pb.bottom + 0.01
        };
        if attempt == 0 && moves && !pb.at_top() {
            pb.advance(block, body_top);
            continue;
        }
        if let Some(pg) = pb.page() {
            let mut y = r.y;
            for row in &tl.rows {
                for it in &row.items {
                    let mut it = it.clone();
                    it.translate(r.x, y);
                    pg.items.push(it);
                }
                y += row.height;
            }
            pg.float_tables.push(Path::top(block));
        }
        let area = r.inset(-dl, -dt, -dr, -db);
        pb.excl.push((area, false, None, WrapText::BothSides, true));
        pb.float_tables.push(area);
        return;
    }
}

fn placed_intersects(item: &Placed, rect: &Rect) -> bool {
    match item {
        Placed::Lines { para, l0, l1, x, y, .. } => para.lines.get(*l0..*l1).is_some_and(|lines| {
            lines.iter().enumerate().any(|(offset, line)| {
                if para.label.is_some() && *l0 + offset == 0 {
                    return Rect::new(x + line.left, y + line.top, line.right - line.left, line.height).intersects(rect);
                }
                para.clusters.get(line.c0..line.c1).is_some_and(|clusters| {
                    clusters.iter().enumerate().any(|(index, cluster)| {
                        if !matches!(
                            cluster.kind,
                            crate::para::ClKind::Text | crate::para::ClKind::Space | crate::para::ClKind::Tab | crate::para::ClKind::Object(_)
                        ) {
                            return false;
                        }
                        let ci = line.c0 + index;
                        let Some(left) = line.cl_left(ci) else { return false };
                        let Some(right) = line.cl_right(ci) else { return false };
                        Rect::new(x + left, y + line.top, right - left, line.height).intersects(rect)
                    })
                })
            })
        }),
        Placed::Fill { rect: bounds, .. }
        | Placed::Image { rect: bounds, .. }
        | Placed::Shape { rect: bounds, .. }
        | Placed::Graphic { rect: bounds, .. }
        | Placed::Cell { rect: bounds, .. }
        | Placed::Object { rect: bounds, .. }
        | Placed::Clip { rect: bounds, .. } => bounds.intersects(rect),
        Placed::Rule { x0, y0, x1, y1, .. } => Rect::new(x0.min(*x1), y0.min(*y1), (x1 - x0).abs(), (y1 - y0).abs()).intersects(rect),
    }
}

/// The area a floating table's `x` is measured in: its start and width on the page, as laid out
/// before an even page is mirrored (see [`float_rect`]).
fn float_table_h_area(pb: &PageBuilder, f: &TableFloat, mirror: bool) -> (f32, f32) {
    let s = pb.sect;
    let (left, _, shift) = page_frame(pb, mirror).sides();
    match f.h_rel {
        Anchor::Page => (-shift, s.page_w),
        Anchor::Margin => (left - shift, s.text_width()),
        _ => (pb.col_x(), pb.col_w()),
    }
}

/// A floating table's left edge on the page (`legacy`: the first cell's left margin before Word
/// 2013 layout, when an offset places the cell's text).
fn float_table_x(pb: &PageBuilder, w: f32, f: &TableFloat, legacy: f32, mirror: bool) -> f32 {
    let align = PageFrame::h_align(Some(&page_frame(pb, mirror)), f.h_align);
    let x = align_in(float_table_h_area(pb, f, mirror), f.x, w, align);
    if f.h_align.is_none() { x - legacy } else { x }
}

/// Move `pb.y` below the floating objects in the way of something spanning `x0..x1` that
/// starts there (a table in the text flow doesn't wrap around them).
fn below_floats(ctx: &mut Ctx, pb: &mut PageBuilder, x0: f32, x1: f32) {
    pb.y = below(&body_wraps(ctx, pb), pb.y, x0, x1);
}

/// `y`, moved below the wrap areas in `excl` in the way of something spanning `x0..x1` that
/// starts there.
pub(crate) fn below(excl: &[WrapArea], mut y: f32, x0: f32, x1: f32) -> f32 {
    for _ in 0..64 {
        let in_the_way = excl.iter().filter(|(r, ..)| r.y <= y + 0.01 && r.bottom() > y && r.x < x1 && r.right() > x0);
        let Some(bottom) = in_the_way.map(|(r, ..)| r.bottom()).reduce(f32::max) else { break };
        y = bottom;
    }
    y
}
