use super::{Counters, Document, LevelSuffix, NumFormat, Path, Pos, StoryRef, Stroke, Tokens, Ui, WordApp, json, regular, semibold, vec2};

/// Draws the Navigation pane: search box and tabs, page list, find results and heading outline.
pub(super) fn nav(app: &mut WordApp, ui: &mut Ui) {
    let qid = app.window_id("nav_query");
    let mut q = ui.data(|d| d.get_temp::<String>(qid)).unwrap_or_default();
    // Ctrl+F with text selected searches for it, as Word does (a selection within one line).
    let focus = std::mem::take(&mut app.nav_search_focus);
    let picked = focus.then(|| app.session.selected_text()).filter(|t| !t.is_empty() && !t.contains(['\n', '\r']) && t.chars().count() <= 255);
    if let Some(t) = &picked {
        q = t.clone();
    }
    let mut out = egui::TextEdit::singleline(&mut q).hint_text(tl!("Search document")).desired_width(f32::INFINITY).show(ui);
    let r = out.response.clone();
    if focus {
        // Focused with its text selected, so typing replaces it.
        r.request_focus();
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(q.chars().count()));
        out.state.cursor.set_char_range(Some(all));
        out.state.store(ui.ctx(), r.id);
    }
    if r.changed() || picked.is_some() {
        app.ui.nav_tab = "results".into();
        ui.data_mut(|d| d.insert_temp(qid, q.clone()));
        if !q.is_empty() {
            let _ = app.session.run("edit.find", &json!({"text": q}));
        } else {
            app.session.find.results.clear();
        }
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (k, l) in [("headings", "Headings"), ("pages", "Pages"), ("results", "Results")] {
            if ui.selectable_label(app.ui.nav_tab == k, tl!(l)).clicked() {
                app.ui.nav_tab = k.into();
            }
        }
    });
    ui.separator();
    // Where a dragged heading may be dropped: the rows, and the empty pane below them.
    let list = ui.available_rect_before_wrap();
    egui::ScrollArea::vertical().show(ui, |ui| match app.ui.nav_tab.as_str() {
        "pages" => {
            let l = app.session.layout();
            for (i, p) in l.pages.iter().enumerate() {
                let label = crate::i18n::fmt(tl!("Page {page}"), &[("page", &p.number.to_string())]);
                if ui.selectable_label(app.session.page_hint == i, label).clicked() {
                    let _ = app.run("edit.goto", json!({"page": i + 1}));
                }
            }
        }
        "results" => {
            let results = app.session.find.results.clone();
            ui.label(egui::RichText::new(format!("{} results", results.len())).small().weak());
            for (a, b) in results.iter().take(500) {
                let Some(p) = app.session.doc.para_at(a) else { continue };
                let from = p.clamp(a.off.saturating_sub(30));
                let to = p.clamp((b.off + 40).min(p.len()));
                let pre = p.text.get(from..a.off).unwrap_or("");
                let hit = p.text.get(a.off..b.off).unwrap_or("");
                let post = p.text.get(b.off..to).unwrap_or("");
                let mut job = egui::text::LayoutJob::default();
                let fmt = |c| egui::TextFormat { font_id: regular(12.0), color: c, ..Default::default() };
                job.append(pre, 0.0, fmt(ui.visuals().text_color()));
                // The match is marked the way Find marks it on the page, per theme (#656).
                let mark = Tokens::get(ui.ctx()).find_highlight;
                job.append(hit, 0.0, egui::TextFormat { font_id: semibold(12.0), background: mark, color: ui.visuals().text_color(), ..Default::default() });
                job.append(post, 0.0, fmt(ui.visuals().text_color()));
                job.wrap.max_width = ui.available_width();
                if ui.add(egui::Button::new(job).frame(false)).clicked() {
                    let _ = app.run("select.range", json!({"anchor": a, "focus": b}));
                }
                ui.separator();
            }
        }
        _ => {
            let caret = app.session.sel.focus.clone();
            let items = nav_headings(&app.session.doc);
            let current = items.iter().rposition(|(path, ..)| *path <= caret.path);
            if items.is_empty() {
                ui.label(egui::RichText::new(tl!("Create an interactive outline of your document.\n\nIt's a great way to keep track of where you are or quickly move your content around.\n\nTo get started, go to the Home tab and apply Heading styles to the headings in your document.")).weak());
            }
            // A heading has children when the next heading is a deeper level (#1997).
            let has_children: Vec<bool> = items.iter().enumerate().map(|(i, (_, lv, _))| items.get(i + 1).is_some_and(|n| n.1 > *lv)).collect();
            let collapsed_flags: Vec<bool> = items.iter().enumerate().map(|(i, (p, _, _))| has_children[i] && app.ui.nav_collapsed.contains(p)).collect();
            let levels: Vec<u8> = items.iter().map(|(_, lv, _)| *lv).collect();
            let visible = visible_headings(&levels, &collapsed_flags);
            // A heading row being dragged (#4288): the row's id and its heading's body index.
            let drag_id = app.window_id("nav_heading_drag");
            let dragging: Option<(egui::Id, usize)> = ui.data(|d| d.get_temp(drag_id));
            let mut rows: Vec<DropRow> = Vec::new();
            for (i, (path, lv, txt)) in items.into_iter().enumerate() {
                if !visible[i] {
                    continue;
                }
                let collapsed = collapsed_flags[i];
                let (toggle, r) = nav_heading(ui, lv, &txt, current == Some(i), has_children[i].then_some(collapsed));
                // Only headings in the body text itself (not in a table) move.
                let block = (path.0.len() == 1).then(|| path.last());
                rows.push(DropRow { rect: r.rect, block, level: lv });
                if r.drag_started()
                    && let Some(b) = block
                {
                    ui.data_mut(|d| d.insert_temp(drag_id, (r.id, b)));
                }
                if toggle {
                    if !app.ui.nav_collapsed.remove(&path) {
                        app.ui.nav_collapsed.insert(path);
                    }
                } else if r.clicked() {
                    let _ = app.run("caret.set", json!({"pos": Pos { story: StoryRef::Body, path, off: 0 }}));
                    // Align the chosen heading's page immediately against this pane, rather
                    // than the indeterminate minimum movement from scroll_to_rect.
                    app.canvas.nav_to_caret = true;
                    app.canvas.scroll_to_caret = false;
                    app.canvas.want_focus = true;
                }
            }
            if let Some((row, from)) = dragging {
                drag_heading(app, ui, (drag_id, row), from, &rows, list);
            }
        }
    });
}

/// A heading row as a place to drop a dragged heading.
#[derive(Clone, Copy, Debug)]
pub(super) struct DropRow {
    pub(super) rect: egui::Rect,
    /// The heading's body block index; `None` for a heading in a table, which doesn't move.
    pub(super) block: Option<usize>,
    pub(super) level: u8,
}

/// Where the heading at body block `from` lands when dropped with the pointer at height `y`
/// over the shown rows: before the first body heading row whose middle is below the pointer
/// (`Some(block)`), or at the end (`None`), with the height of the line that shows it. Nothing
/// where it would land in its own content or stay where it is.
pub(super) fn drop_target(rows: &[DropRow], from: usize, y: f32) -> Option<(f32, Option<usize>)> {
    let src = rows.iter().position(|r| r.block == Some(from))?;
    let level = rows.get(src)?.level;
    // Its own content ends at the next row of its level or higher.
    let own_end = rows.iter().enumerate().skip(src + 1).find(|(_, r)| r.level <= level).map_or(rows.len(), |(k, _)| k);
    let (k, at) = match rows.iter().enumerate().find(|(_, r)| r.block.is_some() && r.rect.center().y > y) {
        Some((k, r)) => (k, (r.rect.top(), r.block)),
        None => (rows.len(), (rows.last()?.rect.bottom(), None)),
    };
    (!(src..=own_end).contains(&k)).then_some(at)
}

/// While a heading row is dragged: a line shows where the heading would land, and letting go
/// there moves it with its content (`outline.moveSubtree`). Escape, or letting go away from the
/// list, cancels.
fn drag_heading(app: &mut WordApp, ui: &mut Ui, (drag_id, row): (egui::Id, egui::Id), from: usize, rows: &[DropRow], list: egui::Rect) {
    let (escape, released, down, pointer) =
        ui.input(|i| (i.key_pressed(egui::Key::Escape), i.pointer.primary_released(), i.pointer.primary_down(), i.pointer.latest_pos()));
    // egui drops a drag on Escape itself: a held button with no drag is what is left of it.
    let cancelled = escape || (!released && (!down || ui.ctx().dragged_id() != Some(row)));
    if cancelled || released {
        ui.data_mut(|d| d.remove::<(egui::Id, usize)>(drag_id));
    }
    let target = pointer.filter(|p| list.contains(*p)).and_then(|p| drop_target(rows, from, p.y));
    let Some((y, before)) = target.filter(|_| !cancelled) else { return };
    if !released {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        // Drawn over the whole list, so the line after the last row isn't clipped.
        let painter = egui::Painter::new(ui.ctx().clone(), ui.layer_id(), list);
        painter.hline(list.x_range(), y, Stroke::new(2.0, Tokens::get(ui.ctx()).accent));
        return;
    }
    let tracked = app.session.doc.settings.track_changes;
    let levels = wordcraft_layout::outline::levels(&app.session.doc);
    let n = wordcraft_layout::outline::subtree_end(&levels, from).saturating_sub(from);
    let to = before.unwrap_or(levels.len());
    if app.run("outline.moveSubtree", json!({"block": from, "before": before.map_or(json!("end"), |b| json!(b))})).is_ok() {
        // Folded headings stay folded where they went.
        let folded = std::mem::take(&mut app.ui.nav_collapsed);
        app.ui.nav_collapsed = folded.into_iter().map(|p| moved_path(&p, from, n, to, tracked)).collect();
        app.canvas.scroll_to_caret = true;
    }
}

/// Where the heading at `path` is once the `n` body blocks at `from` moved before block `to`:
/// they move, or with Track Changes on a copy of them goes there.
pub(super) fn moved_path(path: &Path, from: usize, n: usize, to: usize, tracked: bool) -> Path {
    let Some(i) = path.0.first().map(|i| *i as usize) else { return path.clone() };
    let end = from.saturating_add(n);
    let j = if tracked {
        if i >= to { i.saturating_add(n) } else { i }
    } else if to < from {
        if (to..from).contains(&i) {
            i.saturating_add(n)
        } else if (from..end).contains(&i) {
            i - from + to
        } else {
            i
        }
    } else if (from..end).contains(&i) {
        (i - from + to).saturating_sub(n)
    } else if (end..to).contains(&i) {
        i - n
    } else {
        i
    };
    let mut p = path.clone();
    if let Some(first) = p.0.first_mut() {
        *first = u32::try_from(j).unwrap_or(u32::MAX);
    }
    p
}

/// Which headings the pane shows: those under a collapsed heading (every following heading
/// of a deeper level) are hidden (#1997).
pub(super) fn visible_headings(levels: &[u8], collapsed: &[bool]) -> Vec<bool> {
    let mut hide_below: Option<u8> = None;
    levels
        .iter()
        .zip(collapsed)
        .map(|(&lv, &c)| {
            if hide_below.is_some_and(|h| lv > h) {
                return false;
            }
            hide_below = c.then_some(lv);
            true
        })
        .collect()
}

/// The body's headings (paragraphs with an outline level) in document order, as (path, level,
/// text). A numbered heading starts with the number it shows (`2.1 Method`), as in Word.
pub(super) fn nav_headings(doc: &Document) -> Vec<(Path, u8, String)> {
    // Every numbered paragraph advances the list counters, headings or not.
    let mut counters = Counters::default();
    let mut items = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        let rp = doc.styles.resolve_para(&p.props);
        let number = p
            .props
            .numbering
            .or(rp.numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| counters.next_label(&doc.numbering, n.num, n.level))
            .filter(|(label, def)| !label.is_empty() && !matches!(def.format, NumFormat::Bullet | NumFormat::None));
        if let Some(lv) = rp.outline_level {
            let txt = p.plain_text().trim().to_string();
            if txt.is_empty() || rp.style.starts_with("TOC") {
                continue;
            }
            let txt = match number {
                Some((label, def)) if def.suffix == LevelSuffix::Nothing => format!("{label}{txt}"),
                Some((label, _)) => format!("{label} {txt}"),
                None => txt,
            };
            items.push((path, lv, txt));
        }
    }
    items
}

/// Constrain each heading to the available pane width so long document text cannot
/// enlarge the panel. A heading too long for the width wraps onto further lines — a word
/// wider than the pane is broken too — and the row grows with it, so the label reads whole
/// instead of ending in an ellipsis (#3015).
pub(super) fn nav_heading(ui: &mut Ui, level: u8, text: &str, selected: bool, collapsed: Option<bool>) -> (bool, egui::Response) {
    ui.horizontal(|ui| {
        ui.add_space(level as f32 * 14.0);
        // The triangle is drawn in code; headings without children keep an empty slot so text aligns.
        let mut toggled = false;
        let (rect, tri) = ui.allocate_exact_size(vec2(14.0, ui.spacing().interact_size.y), egui::Sense::click());
        if let Some(c) = collapsed {
            let ctr = rect.center();
            let col = ui.visuals().text_color();
            let pts = if c {
                vec![ctr + vec2(-2.5, -4.0), ctr + vec2(-2.5, 4.0), ctr + vec2(3.0, 0.0)]
            } else {
                vec![ctr + vec2(-4.0, -2.5), ctr + vec2(4.0, -2.5), ctr + vec2(0.0, 3.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
            toggled = tri.clicked();
        }
        let label = egui::RichText::new(text).font(if level == 0 { semibold(12.5) } else { regular(12.5) });
        let width = ui.available_width();
        let r = ui.add(
            // The trailing grow atom takes the spare width, so the text starts flush left
            // under its parent instead of centred, as in Word (#1199). A minimum height
            // keeps short headings on one row; a wrapped one becomes as tall as it needs.
            // Dragging a row moves its heading with its content (#4288).
            egui::Button::selectable(selected, (label, egui::Atom::grow()))
                .wrap()
                .min_size(vec2(width, ui.spacing().interact_size.y))
                .sense(egui::Sense::click_and_drag()),
        );
        (toggled, r)
    })
    .inner
}
