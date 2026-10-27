use super::boxes::{BoxContext, BoxLayout, Edges, joined, layout_box, push_para};
use super::cache::has_page_number_fields;
use super::ctx::Ctx;
use super::floats::{ObjFrame, anchor_floats, body_wraps, continued_exclusions, place_objects};
use super::notes::{DECOR_STORY, NoteBox, para_notes};
use super::page::{PageBuilder, PageFrame, item_bottom};
use super::placed::Placed;
use super::*;

/// A closing mark before a continuous section takes no room after content. An otherwise empty
/// section starting a new page retains its paragraph line.
pub(crate) fn place_para(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, body_top: f32) {
    if !pb.web && ctx.quiet_marks.binary_search(&block).is_ok() {
        let follows_section = block.checked_sub(1).and_then(|i| ctx.doc.body.get(i)).and_then(|b| b.as_para()).is_some_and(|p| p.section.is_some());
        let section = pb.sect_idx;
        let own_page = pb.page().is_some_and(|page| page.section == section);
        // Word keeps the line when a new page's section contains only this mark.
        if p.section.is_some()
            && p.objects.is_empty()
            && (ctx.opts.show_hidden || !ctx.doc.styles.resolve_char(p.props.style.as_deref(), &p.mark).hidden)
            && !ctx.has_label(p)
            && (block == 0 || follows_section)
            && pb.at_top()
            && own_page
        {
            place_para_in_flow(ctx, pb, p, block, body_top);
            return;
        }
        // Its (empty) line stays where the text ended, even at the bottom of a full column.
        let (y, prev, bottom, content_bottom, at) = (pb.y, pb.prev.clone(), pb.bottom, pb.content_bottom, (pb.pages.len(), pb.col));
        let drawn = pb.page().map_or(0, |page| page.items.len());
        pb.bottom = f32::MAX / 4.0;
        place_para_in_flow(ctx, pb, p, block, body_top);
        pb.bottom = bottom;
        if (pb.pages.len(), pb.col) == at {
            (pb.y, pb.prev, pb.content_bottom) = (y, prev, content_bottom);
            // Taking no room, it draws no list label either (Word, #3907 review); still counted.
            if let Some(page) = pb.page() {
                strip_mark(page, drawn, true);
            }
        }
        return;
    }
    if !pb.web && ctx.stacked_marks.binary_search(&block).is_ok() {
        // Measured in Word (#3907): each takes its line while it fits; past the bottom it stays
        // there instead of starting a page, so the text after a long run starts the next page.
        let (y, prev, bottom, content_bottom, at) = (pb.y, pb.prev.clone(), pb.bottom, pb.content_bottom, (pb.pages.len(), pb.col));
        let drawn = pb.page().map_or(0, |page| page.items.len());
        pb.bottom = f32::MAX / 4.0;
        place_para_in_flow(ctx, pb, p, block, body_top);
        // A break before it starts a page with its own bottom.
        if pb.pages.len() == at.0 {
            pb.bottom = bottom;
        }
        if (pb.pages.len(), pb.col) == at && pb.y > bottom + 0.01 {
            // At the bottom: nothing more fits on the page, even a line smaller than the mark's.
            (pb.y, pb.prev, pb.content_bottom) = (bottom.max(y), prev, content_bottom);
            // Nothing of it is drawn below the margin, not even its list label or shading (Word):
            // its line stays only for the caret.
            // ponytail: a mark anchoring objects keeps everything; strip those too if one shows up
            if p.objects.is_empty()
                && let Some(page) = pb.page()
            {
                strip_mark(page, drawn, false);
            }
        }
        return;
    }
    place_para_in_flow(ctx, pb, p, block, body_top);
}

/// Take the list label off the lines a mark added to `page` after its first `drawn` items; its
/// other items (shading, borders, its line number) stay only when `keep_others`. Its own lines
/// stay for the caret.
fn strip_mark(page: &mut Page, drawn: usize, keep_others: bool) {
    let tail = page.items.split_off(drawn.min(page.items.len()));
    page.items.extend(tail.into_iter().filter_map(|item| match item {
        Placed::Lines { story, path, para, l0, l1, x, y, turn } if para.label.is_some() => {
            let mut bare = (*para).clone();
            bare.label = None;
            Some(Placed::Lines { story, path, para: Arc::new(bare), l0, l1, x, y, turn })
        }
        Placed::Lines { story, .. } if story == DECOR_STORY && !keep_others => None,
        line @ Placed::Lines { .. } => Some(line),
        other => keep_others.then_some(other),
    }));
}

/// How many times one body paragraph is laid out again for the page numbers of the later pages
/// it runs onto (#1352).
const MAX_PAGE_RELAYS: usize = 64;

/// Run `f`, a layout of a paragraph again, with the automatic equation numbers it counted from
/// (`eq_start`) the first time, and leave the count where it was: laying it out again doesn't
/// count its numbered equations twice.
pub(crate) fn again<T>(ctx: &mut Ctx, eq_start: u32, f: impl FnOnce(&mut Ctx) -> T) -> T {
    let now = std::mem::replace(&mut ctx.eq_count, eq_start);
    let out = f(ctx);
    ctx.eq_count = now;
    out
}

/// How many times one body paragraph's lines are broken again for the wrap areas of the later
/// columns and pages it runs onto (#1374).
const MAX_WRAP_RELAYS: usize = 64;

/// Run `f` with `map` as the field context's page map (`FieldCtx::page_from`), then take it back.
fn with_page_map<T>(ctx: &mut Ctx, map: &mut Vec<(usize, u32)>, f: impl FnOnce(&mut Ctx) -> T) -> T {
    ctx.fields.page_from = std::mem::take(map);
    let out = f(ctx);
    *map = std::mem::take(&mut ctx.fields.page_from);
    out
}

fn place_para_in_flow(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, body_top: f32) {
    let width = pb.col_w();
    // Floating objects anchored here join the page's wrap areas before the text is laid out; if
    // the paragraph then starts somewhere else (another column or page, or contextual spacing
    // drops its space before), they are placed again there.
    let space_before = ctx.doc.styles.resolve_para(&p.props).space_before;
    let excl_mark = pb.excl.len();
    let guess = pb.y + space_before;
    let label = ctx.next_label(p);
    // The pages this paragraph's fields are on after its first (`FieldCtx::page_from`), kept here
    // and lent to the field context only while this paragraph itself is laid out, so footnotes,
    // text boxes and headers laid out in between never see it (#1352).
    let mut page_map: Vec<(usize, u32)> = Vec::new();
    // Where its automatic equation numbers start: each layout of it again counts from here.
    let eq_start = ctx.eq_count;
    let (mut float_rects, mut pl, mut rel) = anchor_floats(ctx, pb, p, block, guess, space_before, excl_mark, &label, true);
    // The page its fields were laid out for (and the width), and how many times it was laid out again for a later
    // page (bounded: a paragraph over hundreds of pages keeps the last number it got).
    let (mut laid_page, mut laid_width) = (pb.number, pb.col_w());
    let mut page_relays = 0usize;
    // How many times its lines were broken again for the wrap areas of a later column (#1374).
    let mut wrap_relays = 0usize;
    // The anchor paragraph's top (above its space before) the floats were placed from.
    let mut anchor_y = pb.y;
    let spot = |pb: &PageBuilder| (pb.pages.len(), pb.col);
    let placed_at = spot(pb);
    let broke_before = pl.rp.page_break_before && !pb.at_top() && !pb.web;
    if broke_before {
        pb.new_page(block, body_top);
    }
    // Same-style contextual spacing removes this paragraph's before-space, or subtracts the
    // previous paragraph's after-space from the merged gap (ISO/IEC 29500-1 §17.3.1.9).
    let mut before = pl.rp.space_before;
    let prev_para = pb.prev.clone();
    // At the top of a page, measured in Word for Windows: no space before after a paragraph that
    // ended with a page break, nor (from Word 2013's layout, compatibility mode 15) for a page
    // break before; a section break or, in older modes, a page break before keeps it.
    let after_break_mark = std::mem::take(&mut pb.after_break_mark);
    if pb.at_top() && (after_break_mark || (broke_before && ctx.doc.settings.compat_mode >= 15)) {
        before = 0.0;
    } else if pb.at_top()
        && !ctx.doc.settings.sum_paragraph_spacing
        && let Some((_, _, after, _, _)) = &prev_para
    {
        // After a page break before or a section break the space between the paragraphs is
        // still counted as one: the previous paragraph's space after (left on its page) comes
        // off the space before here (Word: 24 pt before after 10 pt after leaves 14 pt).
        before = (before - after.max(0.0)).max(0.0);
    }
    if let Some((style, ctxl, mut after, after_auto, listed)) = pb.prev.take()
        && !pb.at_top()
    {
        // Contextual spacing suppresses each paragraph's own side of the gap.
        if style == pl.rp.style {
            if ctxl {
                pb.y -= after;
                before = (before - after).max(0.0);
                after = 0.0;
            }
            if pl.rp.contextual_spacing {
                before = 0.0;
            }
        }
        // Automatic spacing: none between two list items (as in an HTML list; measured in Word:
        // any two items, of one list or two, at any level).
        if listed && label.is_some() {
            if after_auto {
                pb.y -= after;
                after = 0.0;
            }
            if pl.rp.before_auto {
                before = 0.0;
            }
        }
        // The space between two paragraphs is the larger of the first's space after and the
        // second's space before, not their sum.
        if before > 0.0 && after > 0.0 && !ctx.doc.settings.sum_paragraph_spacing {
            before = (before - after).max(0.0);
        }
    }
    // Automatic spacing: none at the top of the document.
    if pl.rp.before_auto && block == 0 {
        before = 0.0;
    }
    pb.y += before;
    let mut applied_before = before;
    // Bordered paragraphs: room for their top and bottom borders, those of a group joined in one box.
    let above = joined(ctx.doc, &ctx.doc.body, block, block.checked_sub(1), None);
    let below = joined(ctx.doc, &ctx.doc.body, block, Some(block + 1), None);
    let own = Edges::own(&pl, 0, pl.lines.len());
    let (top_room, bottom_room) = (if above { 0.0 } else { own.top }, if below { 0.0 } else { own.bottom });
    // Keep lines together: if it doesn't fit but would on an empty column, move it. Keep with
    // next alone moves a paragraph only when it would fit here by itself (Word for Windows): one
    // that doesn't breaks as usual, its last lines going over with the next paragraph.
    let height = pl.height + top_room + bottom_room;
    if pl.rp.keep_lines || (pl.rp.keep_next && pb.y + height <= pb.bottom + 0.01) {
        let need = height + if pl.rp.keep_next { next_first_line(ctx, block, width, 0, &mut ctx.counters.clone()) } else { 0.0 };
        // (Against the whole column: balanced columns grow to hold it.)
        // Room in the next column, or on the next page when this is the last.
        // With the 0.01 pt of slack the other fit checks take.
        if pb.y + need > pb.bottom + 0.01 && !pb.fresh() && need <= pb.next_room(body_top) {
            pb.advance(block, body_top);
            pb.y += pl.rp.space_before.min(0.0);
            applied_before = pl.rp.space_before.min(0.0);
        }
    }
    // A different top only matters when there is something to wrap around.
    if spot(pb) != placed_at || ((pb.y - guess).abs() > 0.01 && !pb.excl.is_empty()) {
        let y0 = pb.y;
        (float_rects, pl, rel) = again(ctx, eq_start, |ctx| anchor_floats(ctx, pb, p, block, y0, applied_before, excl_mark, &label, true));
        (laid_page, laid_width) = (pb.number, pb.col_w());
        anchor_y = y0 - applied_before;
    }
    let mut n = pl.lines.len();
    // Worked out only for a last line on a page with notes.
    let mut after_on_page: Option<f32> = None;
    let mut l0 = 0;
    while l0 < n {
        // The lines going into a new column (or page) were broken around the wrap areas where the
        // paragraph started: break them again around this column's (the lines before stay as they
        // were, and so do the paragraph's own floating objects), when they differ (#1374).
        if l0 > 0
            && !pb.web
            && wrap_relays < MAX_WRAP_RELAYS
            && let Some((resume, t)) = pl.lines.get(l0).map(|l| (l.start, l.top))
        {
            let here = continued_exclusions(&rel, &body_wraps(ctx, pb), pb.col_x(), pb.y, t, laid_width);
            if here != rel {
                wrap_relays += 1;
                ctx.fields.page = laid_page;
                #[cfg(test)]
                WRAP_RELAYS.with(|c| c.set(c.get() + 1));
                let relaid =
                    with_page_map(ctx, &mut page_map, |ctx| again(ctx, eq_start, |ctx| ctx.para_labelled(p, laid_width, None, &here, label.clone())));
                // Only if the rest still starts at the same line, at the same height (an outline
                // reaching above it could move the lines before down, and the column's areas with them).
                if let Some(k) = relaid.lines.iter().position(|l| l.start == resume)
                    && relaid.lines.get(k).is_some_and(|l| (l.top - t).abs() < 0.01)
                {
                    (pl, rel) = (relaid, here);
                    n = pl.lines.len();
                    l0 = k;
                }
            }
        }
        // Page fields in the lines going onto a new page show that page's number: lay the
        // paragraph out again with the fields from there on on this page (the lines before stay as
        // they were, so the line the rest starts with is still there), breaking the rest again for
        // the new results' widths (#1352). Only a paragraph with page fields pays for this.
        if l0 > 0
            && pl.has_page_fields
            && !pb.web
            && page_relays < MAX_PAGE_RELAYS
            && let Some(resume) = pl.lines.get(l0).map(|l| l.start)
            && page_map.last().map_or(laid_page, |e| e.1) != pb.number
            && has_page_number_fields(p)
        {
            page_relays += 1;
            page_map.push((resume, pb.number));
            ctx.fields.page = laid_page;
            let relaid =
                with_page_map(ctx, &mut page_map, |ctx| again(ctx, eq_start, |ctx| ctx.para_labelled(p, laid_width, None, &rel, label.clone())));
            match relaid.lines.iter().position(|l| l.start == resume) {
                Some(k) => {
                    pl = relaid;
                    n = pl.lines.len();
                    l0 = k;
                }
                None => {
                    page_map.pop();
                }
            }
        }
        // What is laid out for these lines from here (their footnotes) is on this page.
        ctx.fields.page = pb.number;
        // How many lines fit?
        let Some(first) = pl.lines.get(l0) else { break };
        // A paragraph's first line may start below its top (pushed down by a floating object).
        let lead = if l0 == 0 { first.top.max(0.0) + top_room } else { 0.0 };
        let mut l1 = l0;
        let mut forced_break = None;
        // A page break followed only by the paragraph mark: the mark's (empty) line stays on this
        // page with the break, so nothing of the paragraph goes to the next page (Word).
        let mut mark_with_break = false;
        let mut new_notes: Vec<NoteBox> = Vec::new();
        let notes = para_notes(ctx.doc, p, &pl);
        while l1 < n {
            let Some(l) = pl.lines.get(l1) else { break };
            // Footnotes referenced on this line go to the bottom of this page.
            let mut line_notes = Vec::new();
            if !pb.web {
                for (ci, id) in &notes {
                    if *ci >= l.c0
                        && *ci < l.c1
                        && !pb.notes.iter().chain(new_notes.iter()).any(|x| x.id == *id)
                        && let Some(part) = ctx.doc.parts.get(id).filter(|p| p.kind == wordcraft_doc::PartKind::Footnote)
                    {
                        let blocks = part.blocks.clone();
                        let BoxLayout { items, height: h, .. } =
                            layout_box(ctx, StoryRef::Part(*id), &blocks, &[], pb.sect.text_width(), BoxContext::Other, 0, None);
                        line_notes.push(NoteBox { id: *id, items, h, cont: false });
                    }
                }
            }
            let extra: f32 = line_notes.iter().map(|n| n.h).sum::<f32>()
                + if pb.notes.is_empty() && new_notes.is_empty() && !line_notes.is_empty() { pb.seps.sep_h(false) } else { 0.0 };
            let limit = pb.bottom
                - new_notes.iter().map(|n| n.h).sum::<f32>()
                - if pb.notes.is_empty() && !new_notes.is_empty() { pb.seps.sep_h(false) } else { 0.0 }
                - extra;
            // Multiple spacing above single puts its extra leading below the text (see
            // `baseline_offset`), and Word lets that leading of a page's last line run into the
            // bottom margin: only the text has to fit (observed: a double-spaced 12pt line stays on
            // the page while its text ends above the margin, about half a line lower than when the
            // whole line must fit). Only the bottom margin takes that leading: a footer taller than
            // the margin, or footnotes, keep the whole line clear of them.
            let at_margin = pb.bottom >= pb.sect.page_h - pb.sect.margin_bottom - 0.01;
            let fit_h = match pl.rp.line_spacing {
                wordcraft_doc::props::LineSpacing::Multiple(m)
                    if m > 1.0 && at_margin && pb.notes.is_empty() && new_notes.is_empty() && line_notes.is_empty() =>
                {
                    // `baseline` is from the paragraph's top; the text ends a descent below it.
                    (l.baseline - l.top + l.text_desc).clamp(0.0, l.height)
                }
                _ => l.height,
            };
            let bottom = pb.y + lead + (l.top + fit_h - first.top);
            // On a page with footnotes the paragraph's last line keeps its space after clear of
            // the separator (Word): if the space doesn't fit, the line goes to the next page (and,
            // with widow control, one more with it). Without notes the space after may run past
            // the bottom margin as usual.
            let has_notes = !pb.notes.is_empty() || !new_notes.is_empty() || !line_notes.is_empty();
            let tail = if has_notes && l1 + 1 == n {
                *after_on_page.get_or_insert_with(|| space_after_kept(ctx, block, &pl.rp, label.is_some()))
            } else {
                0.0
            };
            let over = bottom + tail > limit + 0.01;
            // Notes too long for the rest of the page continue on the next one: the line still
            // fits if the notes before its last one, and that one's first line, fit below it.
            // The notes then take the rest of the page.
            let split = bottom > limit + 0.01
                && line_notes.split_last().is_some_and(|(last, others)| {
                    let before: f32 = pb.notes.iter().chain(&new_notes).chain(others).map(|n| n.h).sum();
                    let sep = pb.seps.sep_h(pb.notes.first().is_some_and(|n| n.cont));
                    pb.body_floor().max(bottom) + sep + before + last.first_piece() + pb.seps.notice_h() <= pb.orig_bottom + 0.01
                });
            if over && l1 > l0 && !split {
                break;
            }
            // A line holding nothing but a page or column break stays at the bottom of the page
            // even when it doesn't fit: it only ends the page, and moving it would leave a blank
            // page behind it (Word does the same).
            let only_break = matches!(l.end, LineEnd::PageBreak | LineEnd::ColumnBreak)
                && p.text
                    .get(l.start..l.stop)
                    .is_some_and(|t| t.chars().all(|c| matches!(c, wordcraft_doc::para::PAGE_BREAK | wordcraft_doc::para::COLUMN_BREAK)));
            // An empty paragraph closing a section before a new page: measured in Word, it stays
            // on a full page instead of starting one of its own.
            let closing_mark = pb.next_starts_page && p.section.is_some() && p.text.is_empty();
            if over && l1 == l0 && !pb.fresh() && !split && !only_break && !closing_mark {
                break;
            }
            new_notes.extend(line_notes);
            l1 += 1;
            if matches!(l.end, LineEnd::PageBreak | LineEnd::ColumnBreak) && !pb.web {
                forced_break = Some(l.end);
                if l.end == LineEnd::PageBreak && l1 + 1 == n && pl.lines.get(l1).is_some_and(|m| only_markers(p, m.start..m.stop)) {
                    l1 += 1;
                    mark_with_break = true;
                }
                break;
            }
            if split {
                break;
            }
        }
        // Widow/orphan control (2-line minimum at either end).
        if forced_break.is_none() && l1 < n && pl.rp.widow_control && n >= 2 {
            if l1 - l0 == 1 && l0 == 0 && !pb.fresh() {
                l1 = l0; // orphan: move the first line to the next page
            } else if n - l1 == 1 && (l1 - l0 >= 3 || (l0 > 0 && l1 - l0 >= 2)) {
                l1 -= 1; // widow: take one more line along
            } else if n - l1 == 1 && l0 == 0 && l1 - l0 == 2 && !pb.fresh() {
                // Neither split of a three-line paragraph is allowed (two and a widow, or an
                // orphan and two): it moves whole, as in Word.
                l1 = l0;
            }
        }
        // A row (text both sides of a floating object) stays together.
        while l1 > l0 && l1 < n && pl.lines.get(l1).is_some_and(|l| l.beside) {
            l1 -= 1;
        }
        if l1 == l0 {
            if pb.fresh() {
                l1 = l0 + 1; // can't fit even one line on an empty page: overflow
            } else {
                pb.advance(block, body_top);
                if l0 == 0 {
                    let y0 = pb.y;
                    (float_rects, pl, rel) = again(ctx, eq_start, |ctx| anchor_floats(ctx, pb, p, block, y0, 0.0, excl_mark, &label, true));
                    (laid_page, laid_width) = (pb.number, pb.col_w());
                    anchor_y = y0;
                    n = pl.lines.len();
                }
                continue;
            }
        }
        // Keep only the notes of the lines that are placed.
        let placed_end = pl.lines.get(l1.saturating_sub(1)).map(|l| l.c1).unwrap_or(0);
        new_notes.retain(|n| notes.iter().any(|(ci, nid)| *nid == n.id && *ci < placed_end));
        let before = pb.notes_h();
        pb.notes.extend(new_notes);
        if pb.balance.is_some() {
            // Balanced columns end where the balance puts them unless the notes reach higher.
            pb.cap_columns();
        } else {
            // Relative, so a bottom moved down for notes continuing on the next page stays put.
            pb.bottom -= pb.notes_h() - before;
        }
        // Mirror indents: on an even (left-hand) page the inside indent is on the right, so the
        // paragraph's lines (broken for the same width) move over by the indents' difference.
        let x = pb.col_x() + mirror_shift(&pl.rp, pb.number, pb.web);
        let y = pb.y + lead;
        // Notes that no longer fit below the text (a long one, or the rest of one continued from
        // the page before) get what's left under it and continue on the next page.
        if !pb.notes.is_empty()
            && !pb.web
            && let Some(placed_bottom) = item_bottom(y, &pl, l0, l1)
            && placed_bottom > pb.bottom + 0.01
        {
            pb.bottom = pb.body_floor().max(placed_bottom).min(pb.orig_bottom);
        }
        let mut items = Vec::new();
        let mut edges = Edges { top: if l0 == 0 { top_room } else { 0.0 }, bottom: if l1 >= n { bottom_room } else { 0.0 }, up: 0.0 };
        if l0 == 0
            && above
            && let Some((page, col, end)) = pb.para_end
            && page == pb.pages.len()
            && col == pb.col
        {
            edges.up = (y - end).max(0.0);
        }
        let end = push_para(&mut items, StoryRef::Body, &[block as u32], &pl, l0, l1, x, y, width, edges);
        pb.para_end = Some((pb.pages.len(), pb.col, end));
        // Pictures, shapes and text boxes in these lines (web view: floats placed by this piece).
        let at = ObjFrame {
            page: Some(PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror }),
            col: (x, width),
            para_y: if pb.web { y } else { anchor_y },
            table_chr: None,
            pinned: None,
        };
        let (behind, front) = place_objects(ctx, StoryRef::Body, &[block as u32], p, &pl, (l0, l1), (x, y), &at, &float_rects, 0);
        items.extend(front);
        // Line numbers in the left margin.
        if let Some(ln) = pb.sect.line_numbers.clone().filter(|_| !pb.web && pl.rp.style != "Header" && !pl.rp.suppress_line_numbers)
            && let Some(first) = pl.lines.get(l0)
        {
            for li in l0..l1 {
                let Some(line) = pl.lines.get(li).filter(|l| !l.beside) else { continue };
                pb.line_no = pb.line_no.saturating_add(1);
                let n = pb.line_no.saturating_add(ln.start.saturating_sub(1));
                if ln.count_by > 1 && !n.is_multiple_of(ln.count_by) {
                    continue;
                }
                let num = ctx.number_para(n);
                let w = num.lines.first().and_then(|l| l.xs.last().copied()).unwrap_or(10.0);
                let dist = if ln.distance > 0.0 { ln.distance } else { 18.0 };
                let ly = y + (line.top - first.top) + (line.baseline - line.top) - num.lines.first().map(|l| l.baseline).unwrap_or(10.0);
                items.push(Placed::Lines {
                    story: DECOR_STORY,
                    path: Path(vec![]),
                    para: num.clone(),
                    l0: 0,
                    l1: 1,
                    x: pb.sect.margin_left + pb.sect.gutter - dist - w,
                    y: ly,
                    turn: TextDirection::Horizontal,
                });
            }
        }
        let n_behind = behind.len();
        if let Some(pg) = pb.pages.last_mut() {
            for (i, it) in behind.into_iter().enumerate() {
                pg.items.insert(i.min(pg.items.len()), it);
            }
            pg.items.extend(items);
        }
        if let Some(b) = pb.behind.last_mut() {
            *b += n_behind;
        }
        if let (Some(f), Some(l)) = (pl.lines.get(l0), pl.lines.get(l1 - 1)) {
            pb.y = y + (l.top + l.height - f.top);
            pb.content_bottom = pb.content_bottom.max(pb.y);
        }
        l0 = l1;
        // A paragraph that starts with a page break starts on the new page, its space before
        // there (Word).
        let starts_with_break = l0 == 1
            && pl.lines.first().is_some_and(|l| {
                p.text.get(l.start..l.stop).is_some_and(|t| !t.is_empty() && t.chars().all(|c| c == wordcraft_doc::para::PAGE_BREAK))
            });
        match forced_break {
            Some(LineEnd::PageBreak) if mark_with_break => {
                pb.new_page(block, body_top);
                pb.after_break_mark = true;
            }
            Some(LineEnd::PageBreak) if starts_with_break && !pb.web => {
                pb.new_page(block, body_top);
                pb.y += pl.rp.space_before.max(0.0);
            }
            Some(LineEnd::PageBreak) => pb.new_page(block, body_top),
            Some(LineEnd::ColumnBreak) => {
                pb.col_break_page = Some(pb.pages.len().saturating_sub(1));
                pb.advance(block, body_top);
                // Unequal columns: the text after the break is broken for its own column's width
                // (Word), not the width of the column it started in.
                if (pb.col_w() - width).abs() > 0.5
                    && let Some(resume) = pl.lines.get(l0).map(|l| l.start)
                {
                    let y0 = pb.y;
                    // The floats were placed and drawn with the first piece: they stay there.
                    let (rects, relaid, relaid_rel) = with_page_map(ctx, &mut page_map, |ctx| {
                        again(ctx, eq_start, |ctx| anchor_floats(ctx, pb, p, block, y0, 0.0, excl_mark, &label, false))
                    });
                    if let Some(k) = relaid.lines.iter().position(|l| l.start == resume) {
                        (float_rects, pl, rel) = (rects, relaid, relaid_rel);
                        (laid_page, laid_width) = (pb.number, pb.col_w());
                        anchor_y = y0;
                        n = pl.lines.len();
                        l0 = k;
                    }
                }
            }
            _ if l0 < n => pb.advance(block, body_top),
            _ => {}
        }
    }
    // A mark that stayed with its page break keeps its space after on that page too, and its
    // bottom border, drawn there: the next page starts with the next paragraph (Word,
    // compatibility modes 12 to 15).
    let (bottom_room, space_after) = if pb.after_break_mark { (0.0, 0.0) } else { (bottom_room, pl.rp.space_after) };
    pb.y += bottom_room + space_after;
    pb.prev = Some((pl.rp.style.clone(), pl.rp.contextual_spacing, space_after, pl.rp.after_auto, label.is_some()));
}

/// Whether `range` of `p`'s text holds nothing but zero-width markers (bookmarks, field and
/// control boundaries…), or nothing at all: a line of them takes no room after a page break.
fn only_markers(p: &Paragraph, range: std::ops::Range<usize>) -> bool {
    let Some(text) = p.text.get(range.clone()) else { return false };
    text.char_indices().all(|(i, c)| c == wordcraft_doc::para::OBJ && p.object_at(range.start + i).is_some_and(InlineObject::is_marker))
}

/// The outline of the body for an Outline view pass.
pub(crate) struct OutlinePass {
    view: outline::OutlineView,
    levels: Vec<u8>,
    pub(crate) hidden: Vec<bool>,
}

impl OutlinePass {
    pub(crate) fn new(doc: &Document, view: outline::OutlineView) -> Self {
        let levels = outline::levels(doc);
        let hidden = outline::hidden(&levels, &view);
        OutlinePass { view, levels, hidden }
    }
}

/// A body paragraph in Outline view (one long sheet: no page or column breaks, no floats
/// wrapping): indented by its level behind its symbol; hidden when collapsed or deeper than Show
/// Level; body text cut to its first line with Show First Line Only.
pub(crate) fn place_outline_para(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, o: &OutlinePass) {
    // List numbers count the hidden paragraphs too.
    let label = ctx.next_label(p);
    if o.hidden.get(block).copied().unwrap_or(false) {
        return;
    }
    let level = o.levels.get(block).copied().unwrap_or(outline::BODY);
    let indent = outline::indent(&o.levels, block);
    let x = pb.col_x() + indent + outline::SYMBOL_W;
    let width = (pb.col_w() - indent - outline::SYMBOL_W).max(72.0);
    let q = outline::outline_para(ctx.doc, p, o.view.plain);
    ctx.fields.page = pb.number;
    let pl = ctx.para_labelled(&q, width, None, &[], label);
    pb.y += pl.rp.space_before.max(0.0);
    let n = pl.lines.len();
    let l1 = if o.view.first_line_only && level >= outline::BODY { n.min(1) } else { n };
    // Its borders take their room above and below its lines.
    let edges = Edges::own(&pl, 0, l1);
    let y = pb.y + pl.lines.first().map(|f| f.top.max(0.0)).unwrap_or(0.0) + edges.top;
    let bottom_room = edges.bottom;
    let mut items = Vec::new();
    push_para(&mut items, StoryRef::Body, &[block as u32], &pl, 0, l1, x, y, width, edges);
    let at = ObjFrame {
        page: Some(PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror }),
        col: (x, width),
        para_y: y,
        table_chr: None,
        pinned: None,
    };
    let (behind, front) = place_objects(ctx, StoryRef::Body, &[block as u32], &q, &pl, (0, l1), (x, y), &at, &HashMap::new(), 0);
    items.extend(front);
    let children = level < outline::BODY && outline::has_children(&o.levels, block);
    // Collapsed, or with something under it hidden by Show Level: the wavy line says so.
    let collapsed = children && o.hidden.get(block + 1).copied().unwrap_or(false);
    items.extend(outline::symbol_items(&pl, x, y, level, children, collapsed));
    if let Some(pg) = pb.page() {
        for (i, it) in behind.into_iter().enumerate() {
            pg.items.insert(i.min(pg.items.len()), it);
        }
        pg.items.extend(items);
    }
    if let (Some(f), Some(l)) = (pl.lines.first(), l1.checked_sub(1).and_then(|k| pl.lines.get(k))) {
        pb.y = y + (l.top + l.height - f.top);
    }
    pb.y += bottom_room + pl.rp.space_after.max(0.0);
}

/// How far a paragraph with mirrored indents (`w:mirrorIndents`) moves right on page `number`:
/// its left and right indents are the inside and outside ones, which swap sides on even pages.
fn mirror_shift(rp: &wordcraft_doc::resolve::ResolvedPara, number: u32, web: bool) -> f32 {
    if !rp.mirror_indents || web || !number.is_multiple_of(2) {
        return 0.0;
    }
    let (left, right) = rp.visual_indents();
    right - left
}

/// The space after body paragraph `block` (resolved as `rp`) that is really left below it: none
/// when contextual spacing or automatic spacing between list items drops it before the next
/// paragraph. Same tests as the spacing between paragraphs in `place_para_in_flow`: contextual
/// spacing removes this paragraph's space after only when this one asks for it, and the next
/// counts as a list item when it gets a label.
fn space_after_kept(ctx: &Ctx, block: usize, rp: &wordcraft_doc::resolve::ResolvedPara, listed: bool) -> f32 {
    let after = rp.space_after.max(0.0);
    if after <= 0.0 {
        return 0.0;
    }
    if let Some(Block::Para(next)) = ctx.doc.body.get(block + 1).map(|b| &**b) {
        let nrp = ctx.doc.styles.resolve_para(&next.props);
        if nrp.style == rp.style && rp.contextual_spacing {
            return 0.0;
        }
        if listed && rp.after_auto && ctx.has_label(next) {
            return 0.0;
        }
    }
    after
}

/// The room the paragraphs after `block` need on the page for keep with next: enough of the
/// next one to start there — its first line, two of its lines with widow/orphan control (it
/// doesn't leave a single line at the bottom of a page; it moves over whole instead), or all
/// of it under keep lines together — or all of it and what it keeps with in turn when it is
/// kept with the next too (a chain moves together in Word). Word carries a kept heading over
/// when the paragraph after it cannot start on the page (#966).
fn next_first_line(ctx: &mut Ctx, block: usize, width: f32, depth: usize, counters: &mut Counters) -> f32 {
    match ctx.doc.body.get(block + 1).map(|b| &**b) {
        Some(Block::Para(p)) => {
            // Keep list indents and label metrics without consuming the document counters.
            let rp = ctx.doc.styles.resolve_para(&p.props);
            let label =
                p.props.numbering.or(rp.numbering).filter(|n| n.num != 0).and_then(|n| counters.next_label(&ctx.doc.numbering, n.num, n.level));
            let pl = match ctx.para_ahead(p, width, label.clone()) {
                Some(pl) => pl,
                None => {
                    let env = para::ParaEnv {
                        doc: ctx.doc,
                        width,
                        label,
                        fields: &ctx.fields,
                        show_hidden: ctx.opts.show_hidden,
                        field_codes: ctx.opts.field_codes,
                        hide_deleted: ctx.opts.hides_deleted(StoryRef::Body),
                        table: None,
                        proofing: None,
                        typing_word: None,
                        numeral: ctx.opts.numeral,
                        exclusions: &[],
                        eq_number: ctx.eq_count,
                        read_line_spacing: if ctx.opts.view == ViewMode::Read { ctx.opts.read_line_spacing } else { None },
                        grid: ctx.grid,
                        fit_objects: false,
                        fitted: &[],
                    };
                    Arc::new(para::layout_para(p, &env))
                }
            };
            // Its border room, unless it joins the paragraph before (or after) in one box.
            let own = Edges::own(&pl, 0, pl.lines.len());
            let top = if joined(ctx.doc, &ctx.doc.body, block + 1, Some(block), None) { 0.0 } else { own.top };
            if pl.rp.keep_next && depth < 8 {
                let bottom = if joined(ctx.doc, &ctx.doc.body, block + 1, Some(block + 2), None) { 0.0 } else { own.bottom };
                pl.rp.space_before + top + pl.height + bottom + pl.rp.space_after + next_first_line(ctx, block + 1, width, depth + 1, counters)
            } else {
                // The room this one needs to start on the page with the kept paragraph before
                // it (#966).
                let to = |k: usize| match (pl.lines.first(), pl.lines.get(k)) {
                    (Some(first), Some(last)) => last.top + last.height - first.top,
                    _ => 0.0,
                };
                pl.rp.space_before
                    + top
                    + if pl.rp.keep_lines {
                        pl.height
                    } else if pl.rp.widow_control && pl.lines.len() >= 2 {
                        to(1)
                    } else {
                        to(0)
                    }
            }
        }
        Some(Block::Table(_)) => 24.0,
        None => 0.0,
    }
}
