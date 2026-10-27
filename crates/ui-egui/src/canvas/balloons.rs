use super::geometry::{FLOAT_WIDTH, floating_comments, markup_width, shown_comment};
use super::pages::page_screen_scale;
use super::*;

/// Comment balloons in the markup area right of each page, joined to their anchors, with each
/// comment's replies under it. Clicking a balloon selects it, like Word: its text becomes editable
/// in place, with Reply, Resolve and Delete (the same commands as the Comments pane).
pub(super) fn balloons(app: &mut WordApp, ui: &mut Ui, painter: &egui::Painter, rects: &[Rect], layout: &DocLayout, scale: f32, view: Rect) {
    app.canvas.balloon_rects.clear();
    let mw = markup_width(app);
    let float = floating_comments(app);
    if app.canvas.balloon.is_some_and(|id| (mw <= 0.0 && !float) || !app.session.doc.comments.contains_key(&id)) {
        deselect_balloon(app);
    }
    if mw <= 0.0 && !float {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let clip = view;
    let only_shown = app.session.prefs.markup.comment_display == CommentDisplay::Selected;
    let shown = shown_comment(app);
    // Replies hang under the comment they answer rather than getting balloons of their own.
    let is_reply = |c: &wordcraft_doc::Comment| c.parent.is_some_and(|p| app.session.doc.comments.contains_key(&p));
    let mut replies: HashMap<u32, Vec<u32>> = HashMap::new();
    for (id, c) in &app.session.doc.comments {
        if let Some(p) = c.parent.filter(|_| is_reply(c)) {
            replies.entry(p).or_default().push(*id);
        }
    }
    // (page, anchor x, anchor y, item) for each anchored, unresolved-or-not comment, each
    // tracked formatting change (Word's default shows those in balloons too) and, when revisions
    // go in balloons, each tracked deletion.
    let mut by_page: HashMap<usize, Vec<(f32, f32, Balloon, Pos)>> = HashMap::new();
    let m = &app.session.prefs.markup;
    let comments = if m.comments { wordcraft_engine::cmd::review::comment_list(&app.session) } else { Vec::new() };
    for (id, pos) in comments {
        let Some(pos) = pos else { continue };
        if app.session.doc.comments.get(&id).is_some_and(is_reply) || (only_shown && shown != Some(id)) {
            continue;
        }
        let Some(c) = layout.caret_on(&pos, app.session.page_hint) else { continue };
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, Balloon::Comment(id), pos));
    }
    let formats = if m.formatting { wordcraft_engine::cmd::review::format_changes(&app.session) } else { Vec::new() };
    for (pos, author, props) in formats.into_iter().take(500) {
        let Some(c) = layout.caret_on(&pos, app.session.page_hint) else { continue };
        let what = crate::panes::describe_props(&props).join(", ");
        let text = if what.is_empty() { tl!("Formatted").to_string() } else { format!("{}: {what}", tl!("Formatted")) };
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, Balloon::Format(author, text), pos));
    }
    // Each end of a tracked move.
    let moves = if m.insertions_deletions { wordcraft_engine::cmd::review::move_changes(&app.session) } else { Vec::new() };
    for (pos, author, from) in moves.into_iter().take(500) {
        let Some(c) = layout.caret_on(&pos, app.session.page_hint) else { continue };
        let text = if from { tl!("Moved from") } else { tl!("Moved to") }.to_string();
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, Balloon::Format(author, text), pos));
    }
    // Show revisions in balloons: each tracked deletion, taken out of the text, at its place.
    let deletions = if app.session.revisions_in_balloons() { wordcraft_engine::cmd::review::deletion_balloons(&app.session) } else { Vec::new() };
    for d in deletions {
        let Some(c) = layout.caret_on(&d.at, app.session.page_hint) else { continue };
        let text = format!("{}: {}", tl!("Deleted"), d.text);
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, Balloon::Format(d.author, text), d.at));
    }
    let palette = t.author_colors;
    let leaders = t.leader_colors;
    let mut authors: Vec<String> = Vec::new();
    let mut clicked: Option<(u32, Pos)> = None;
    // The markup area extends each page.
    for (pi, pr) in rects.iter().enumerate() {
        let page_scale = layout.pages.get(pi).map_or(scale, |page| page_screen_scale(*pr, page, scale));
        let area = Rect::from_min_max(pos2(pr.max.x, pr.min.y), pos2(pr.max.x + mw * page_scale, pr.max.y));
        if mw > 0.0 && area.intersects(clip) {
            painter.rect_filled(area, 0.0, t.markup_area);
            painter.line_segment([area.left_top(), area.left_bottom()], Stroke::new(1.0, t.markup_edge));
        }
    }
    let mut pages: Vec<usize> = by_page.keys().copied().collect();
    pages.sort_unstable();
    for pi in pages {
        let Some(list) = by_page.get_mut(&pi) else { continue };
        let Some(pr) = rects.get(pi) else { continue };
        if !pr.expand(400.0).intersects(clip) {
            continue;
        }
        list.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        let page_scale = layout.pages.get(pi).map_or(scale, |page| page_screen_scale(*pr, page, scale));
        // Beside the page in the column; floating, beside it when the view has room, else over its edge.
        let (x0, w, over_page) = if mw > 0.0 {
            (pr.max.x + 10.0, (mw * page_scale - 20.0).max(60.0), false)
        } else {
            let w = (FLOAT_WIDTH * page_scale - 20.0).max(60.0);
            let beside = pr.max.x + 10.0;
            if beside + w <= view.max.x - 8.0 { (beside, w, false) } else { ((view.max.x - w - 8.0).max(view.min.x + 8.0), w, true) }
        };
        let mut next_y = pr.min.y;
        for (ax, ay, item, pos) in list.iter() {
            let id = match item {
                Balloon::Comment(id) => id,
                Balloon::Format(author, text) => {
                    // A read-only card: who formatted, and how.
                    let author = author_name(author);
                    let ai = authors.iter().position(|a| *a == author).unwrap_or_else(|| {
                        authors.push(author.clone());
                        authors.len() - 1
                    });
                    let color = palette.get(ai % palette.len()).copied().unwrap_or(t.blue);
                    let fs = (11.0 * page_scale / PX_PER_PT).clamp(8.0, 16.0);
                    let anchor = pos2(pr.min.x + ax * page_scale, pr.min.y + ay * page_scale);
                    let top = (anchor.y - 12.0).max(next_y);
                    let head = painter.layout(author, semibold(fs), t.text, w - 16.0);
                    let body = painter.layout(text.clone(), regular(fs), t.text_dim, w - 16.0);
                    let card = Rect::from_min_size(pos2(x0, top), vec2(w, head.size().y + body.size().y + 16.0));
                    next_y = card.max.y + 6.0;
                    if !card.intersects(clip) {
                        continue;
                    }
                    let lead = Stroke::new(1.0, color.linear_multiply(0.7));
                    let on_page = Stroke::new(1.0, leaders.get(ai % leaders.len()).copied().unwrap_or(color).linear_multiply(0.7));
                    dashed(painter, anchor, pos2(pr.max.x, anchor.y), on_page);
                    painter.line_segment([pos2(pr.max.x, anchor.y), pos2(x0, top + 10.0)], lead);
                    card_shapes(painter, &t, card, color, false, None);
                    let y = card.min.y + 6.0;
                    let hh = head.size().y;
                    painter.galley(pos2(card.min.x + 10.0, y), head, t.text);
                    painter.galley(pos2(card.min.x + 10.0, y + hh + 4.0), body, t.text_dim);
                    continue;
                }
            };
            let Some(c) = app.session.doc.comments.get(id) else { continue };
            let resolved = c.resolved;
            let author = author_name(&c.author);
            let ai = authors.iter().position(|a| *a == author).unwrap_or_else(|| {
                authors.push(author.clone());
                authors.len() - 1
            });
            let color = palette.get(ai % palette.len()).copied().unwrap_or(t.blue);
            let fs = (11.0 * page_scale / PX_PER_PT).clamp(8.0, 16.0);
            let thread: Vec<u32> = replies.get(id).cloned().unwrap_or_default();
            let anchor = pos2(pr.min.x + ax * page_scale, pr.min.y + ay * page_scale);
            // Over the page the card goes under the commented line, not on it.
            let top = (anchor.y + if over_page { 4.0 } else { -12.0 }).max(next_y);
            let active = app.canvas.balloon == Some(*id);
            // Lay out a read-only balloon's text: the comment, then each reply under a rule,
            // each with who wrote it and when. (galley, gap above, a rule above)
            let mut lines: Vec<(craft_ui::text::AlignedGalley, f32, bool)> = Vec::new();
            let card = if active {
                Rect::from_min_size(pos2(x0, top), vec2(w, app.canvas.balloon_h.max(24.0)))
            } else {
                let dim = |r: bool| if r { t.text_dim } else { t.text };
                let small = regular((fs * 0.9).max(8.0));
                for (i, cid) in std::iter::once(*id).chain(thread.iter().copied()).enumerate() {
                    let Some(r) = app.session.doc.comments.get(&cid) else { continue };
                    let gap = if i == 0 { 0.0 } else { 9.0 };
                    lines.push((balloon_text(painter, author_name(&r.author), semibold(fs), t.text, w - 16.0), gap, i > 0));
                    let when = crate::local_time::comment_time(r);
                    if !when.is_empty() {
                        lines.push((balloon_text(painter, when, small.clone(), t.text_dim, w - 16.0), 1.0, false));
                    }
                    let text = app.session.doc.plain_text(StoryRef::Part(r.part));
                    lines.push((balloon_text(painter, text.trim().to_string(), regular(fs), dim(resolved), w - 16.0), 4.0, false));
                }
                let h = lines.iter().map(|(g, gap, _)| g.height() + gap).sum::<f32>() + 12.0;
                Rect::from_min_size(pos2(x0, top), vec2(w, h))
            };
            if !card.intersects(clip) {
                next_y = card.max.y + 6.0;
                continue;
            }
            let selected = active || (app.session.sel.focus.path == pos.path && app.session.sel.focus.story == pos.story);
            // Leader: from the anchor along the text to the page edge, then to the card.
            let lead = Stroke::new(if selected { 1.5 } else { 1.0 }, color.linear_multiply(if selected { 1.0 } else { 0.7 }));
            if !over_page {
                let on_page = Stroke::new(
                    lead.width,
                    leaders.get(ai % leaders.len()).copied().unwrap_or(color).linear_multiply(if selected { 1.0 } else { 0.7 }),
                );
                dashed(painter, anchor, pos2(pr.max.x, anchor.y), on_page);
                painter.line_segment([pos2(pr.max.x, anchor.y), pos2(x0, top + 10.0)], lead);
            }
            let card = if active {
                active_balloon(app, ui, painter, *id, &thread, card, fs, color)
            } else {
                card_shapes(painter, &t, card, color, selected, None);
                let text_painter = painter.with_clip_rect(card.intersect(clip));
                let mut y = card.min.y + 6.0;
                for (g, gap, rule) in lines {
                    y += gap;
                    if rule {
                        // A reply starts: a hairline between it and what it answers.
                        painter.hline(card.min.x + 10.0..=card.max.x - 8.0, y - gap / 2.0, Stroke::new(1.0, t.border));
                    }
                    let h = g.height();
                    g.paint(&text_painter, pos2(card.min.x + 10.0, y), t.text);
                    y += h;
                }
                let r = ui.interact(card, ui.scope_id().with(("balloon", *id)), Sense::click());
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if r.clicked() {
                    clicked = Some((*id, pos.clone()));
                }
                card
            };
            next_y = card.max.y + 6.0;
            app.canvas.balloon_rects.push((*id, card));
        }
    }
    if let Some((id, p)) = clicked {
        select_balloon(app, id);
        app.session.sel = wordcraft_engine::Selection::caret(p);
    }
}

/// What a balloon in the markup area shows.
enum Balloon {
    Comment(u32),
    /// A tracked formatting change, move or deletion: its author and "Formatted: …" /
    /// "Moved from" / "Deleted: …".
    Format(String, String),
}

fn author_name(a: &str) -> String {
    if a.is_empty() { tl!("Author").to_string() } else { a.to_string() }
}

/// A balloon card's background, border and author-colour stripe; `slot` paints them under
/// widgets added before the card's size was known.
fn card_shapes(painter: &egui::Painter, t: &Tokens, card: Rect, color: Color32, selected: bool, slot: Option<egui::layers::ShapeIdx>) {
    let shapes = vec![
        egui::Shape::rect_filled(card.translate(vec2(0.0, 1.0)), 4.0, t.page_shadow),
        egui::Shape::rect_filled(card, 4.0, if selected { t.checked } else { t.balloon }),
        egui::Shape::rect_stroke(card, 4.0, Stroke::new(if selected { 1.5 } else { 1.0 }, color), egui::StrokeKind::Inside),
        egui::Shape::rect_filled(Rect::from_min_size(card.min, vec2(4.0, card.height())), egui::CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, color),
    ];
    match slot {
        Some(i) => painter.set(i, egui::Shape::Vec(shapes)),
        None => {
            painter.extend(shapes);
        }
    }
}

/// A read-only balloon's text, wrapped to `width`. The fork's layout orders right-to-left and
/// mixed-direction paragraphs itself; `TextDirection::Auto` takes each paragraph's first strong
/// character. Place each already-shaped row against the card's start edge, including neutral
/// paragraphs inheriting their predecessor's direction. Logical text and glyph order stay intact.
pub(super) fn balloon_text(painter: &egui::Painter, text: String, font: egui::FontId, color: Color32, width: f32) -> craft_ui::text::AlignedGalley {
    let mut job = egui::text::LayoutJob::simple(text, font, color, width);
    job.direction = egui::epaint::text::TextDirection::Auto;
    job.halign = egui::Align::Min;
    // Row placement below supplies the reading edge, including empty RTL rows.
    job.halign_min_is_start = false;
    craft_ui::text::align_galley_rows(painter.layout_job(job), width, |row| if row.rtl_paragraph { egui::Align::Max } else { egui::Align::Min })
}

/// The selected balloon: the comment and its replies, each with who wrote it and when, as
/// editable fields (each edit is one undo step via `review.editComment`), an optional reply
/// field, and Reply / Resolve / Delete.
/// `card` carries last frame's size; returns this frame's.
#[allow(clippy::too_many_arguments)]
fn active_balloon(app: &mut WordApp, ui: &mut Ui, painter: &egui::Painter, id: u32, thread: &[u32], card: Rect, fs: f32, color: Color32) -> Rect {
    let t = Tokens::get(ui.ctx());
    let slot = painter.add(egui::Shape::Noop);
    // The card's own click area sits under its fields, so a click on its padding stays here.
    let _ = ui.interact(card, ui.scope_id().with(("balloon", id)), Sense::click());
    let inner = Rect::from_min_max(pos2(card.min.x + 10.0, card.min.y + 6.0), pos2(card.max.x - 6.0, card.min.y + 20_000.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    child.spacing_mut().item_spacing.y = 3.0;
    let (font, bold, small) = (regular(fs), semibold(fs), regular((fs * 0.9).max(8.0)));
    let focus = std::mem::take(&mut app.canvas.balloon_focus);
    let name = |app: &WordApp, cid: u32| app.session.doc.comments.get(&cid).map(|c| author_name(&c.author)).unwrap_or_default();
    // When it was written, in local time, under the author.
    let when = |app: &WordApp, ui: &mut Ui, cid: u32| {
        let when = app.session.doc.comments.get(&cid).map(crate::local_time::comment_time).unwrap_or_default();
        if !when.is_empty() {
            ui.label(egui::RichText::new(when).font(small.clone()).color(t.text_dim));
        }
    };
    let resolved = app.session.doc.comments.get(&id).is_some_and(|c| c.resolved);
    child.horizontal(|ui| {
        ui.label(egui::RichText::new(name(app, id)).font(bold.clone()).color(t.text));
        if resolved {
            ui.label(egui::RichText::new(tl!("Resolved")).font(small.clone()).color(t.resolved_text));
        }
    });
    when(app, &mut child, id);
    crate::panes::comment_editor(app, &mut child, id, Some(font.clone()), focus && !app.canvas.balloon_reply);
    for rid in thread {
        child.add_space(4.0);
        child.label(egui::RichText::new(name(app, *rid)).font(bold.clone()).color(t.text));
        when(app, &mut child, *rid);
        crate::panes::comment_editor(app, &mut child, *rid, Some(font.clone()), false);
    }
    if app.canvas.balloon_reply && crate::panes::reply_editor(app, &mut child, id, Some(font.clone()), focus) {
        app.canvas.balloon_reply = false;
    }
    let (mut reply, mut resolve, mut delete) = (false, false, false);
    child.add_space(2.0);
    child.horizontal(|ui| {
        let button = |ui: &mut Ui, s: &str| ui.add(egui::Button::new(egui::RichText::new(s).font(small.clone())).small()).clicked();
        reply = button(ui, tl!("Reply"));
        resolve = button(ui, if resolved { tl!("Reopen") } else { tl!("Resolve") });
        delete = button(ui, tl!("Delete"));
    });
    let card = Rect::from_min_max(card.min, pos2(card.max.x, child.min_rect().max.y + 6.0));
    app.canvas.balloon_h = card.height();
    card_shapes(painter, &t, card, color, true, Some(slot));
    if reply {
        app.canvas.balloon_reply = true;
        app.canvas.balloon_focus = true;
    }
    if resolve {
        let _ = app.run("review.resolveComment", json!({"id": id}));
    }
    if delete {
        deselect_balloon(app);
        let _ = app.run("review.deleteComment", json!({"id": id}));
    }
    card
}

/// Select a comment's balloon (its text gets the keyboard), writing back any edit in the one
/// selected before.
pub fn select_balloon(app: &mut WordApp, id: u32) {
    if app.canvas.balloon == Some(id) {
        return;
    }
    deselect_balloon(app);
    app.canvas.balloon = Some(id);
    app.canvas.balloon_focus = true;
    app.canvas.balloon_h = 0.0;
}

/// A press over a balloon card belongs to the card, though the canvas under it also contains the pointer (#1013).
pub(super) fn over_balloon(app: &WordApp, p: Pos2) -> bool {
    app.canvas.balloon_rects.iter().any(|(_, r)| r.contains(p))
}

/// Leave the selected balloon, writing back what was typed in it (its fields may not be drawn
/// again to notice they lost focus).
pub fn deselect_balloon(app: &mut WordApp) {
    let Some(id) = app.canvas.balloon.take() else { return };
    app.canvas.balloon_reply = false;
    app.canvas.balloon_focus = false;
    let thread: Vec<u32> =
        std::iter::once(id).chain(app.session.doc.comments.iter().filter(|(_, c)| c.parent == Some(id)).map(|(k, _)| *k)).collect();
    for c in thread {
        // A refusal is on the status line; the draft stays for when the balloon is selected again.
        let _ = crate::panes::commit_comment(app, c);
    }
    let _ = crate::panes::commit_reply(app, id);
}

pub(super) fn dashed(p: &egui::Painter, a: Pos2, b: Pos2, s: Stroke) {
    p.extend(egui::Shape::dashed_line(&[a, b], s, 4.0, 3.0));
}
