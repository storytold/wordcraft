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
    // Enter in the box goes to the next result and Shift+Enter to the previous one (#5019); the
    // box keeps the keyboard, so Enter can go on through them.
    let enter = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } => Some(modifiers.shift),
            _ => None,
        })
    });
    if !q.is_empty()
        && r.lost_focus()
        && let Some(back) = enter
    {
        step_result(app, ui, back);
        r.request_focus();
    }
    if !q.is_empty() {
        result_bar(app, ui);
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
            let current = current_result(app);
            // When the arrows (or a search) move to another result, the list scrolls to it.
            let seen = app.window_id("nav_result_seen");
            let moved = ui.data_mut(|d| {
                let last = d.get_temp::<Option<usize>>(seen);
                d.insert_temp(seen, current);
                last != Some(current)
            });
            for (i, (a, b)) in results.iter().enumerate().skip(results_window(results.len(), current).start).take(MAX_RESULT_ROWS) {
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
                // The result the arrows are on is marked in the list too.
                let resp = ui.add(egui::Button::selectable(current == Some(i), job).wrap());
                if moved && current == Some(i) {
                    resp.scroll_to_me(Some(egui::Align::Center));
                }
                if resp.clicked() {
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
            for (i, (path, lv, txt)) in items.into_iter().enumerate() {
                if !visible[i] {
                    continue;
                }
                let collapsed = collapsed_flags[i];
                let (toggle, r) = nav_heading(ui, lv, &txt, current == Some(i), has_children[i].then_some(collapsed));
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
        }
    });
}

/// The line under the search box: which result is selected, of how many, and buttons to the
/// previous and next one, which go round from the last result to the first (#5019).
fn result_bar(app: &mut WordApp, ui: &mut Ui) {
    let n = app.session.find.results.len();
    let text = match current_result(app) {
        Some(i) => crate::i18n::fmt(tl!("Result {current} of {count}"), &[("current", &(i + 1).to_string()), ("count", &n.to_string())]),
        None if n == 0 => tl!("No results").to_string(),
        None => crate::i18n::fmt(tl!("{count} results"), &[("count", &n.to_string())]),
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(text).small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(n > 0, |ui| {
                // Right to left: Next goes first, so Previous sits on its left.
                if result_button(ui, "chevronDown", tl!("Next result")).clicked() {
                    step_result(app, ui, false);
                }
                if result_button(ui, "chevronUp", tl!("Previous result")).clicked() {
                    step_result(app, ui, true);
                }
            });
        });
    });
}

/// A 22 px arrow button of the result bar, named for screen readers, greyed when there are no
/// results.
fn result_button(ui: &mut Ui, icon: &str, name: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let enabled = ui.is_enabled();
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let color = if enabled { t.icon } else { t.text_disabled };
    crate::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), icon, color, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
    resp.on_hover_text(name)
}

/// The search result the selection is on, if it is one.
/// Most search results the Results list shows at once.
const MAX_RESULT_ROWS: usize = 500;

/// The results the Results list shows, of `len`: the first [`MAX_RESULT_ROWS`], or as many around
/// `current` when it is past them, so the result the arrows are on is always listed and marked.
pub(super) fn results_window(len: usize, current: Option<usize>) -> std::ops::Range<usize> {
    let start = match current {
        Some(c) if c >= MAX_RESULT_ROWS => c.saturating_sub(MAX_RESULT_ROWS / 2).min(len.saturating_sub(MAX_RESULT_ROWS)),
        _ => 0,
    };
    start..len.min(start.saturating_add(MAX_RESULT_ROWS))
}

fn current_result(app: &WordApp) -> Option<usize> {
    let sel = app.session.sel.ordered();
    app.session.find.results.iter().position(|r| *r == sel)
}

/// Go to the next search result, or the previous one, going round at either end. At most once a
/// frame: when egui runs a frame's pass again (`request_discard`, which the ribbon's font list
/// uses), the second pass gets the same input, and a step is not idempotent as selecting a
/// result is.
fn step_result(app: &mut WordApp, ui: &Ui, back: bool) {
    let once = app.window_id("nav_result_step");
    let frame = ui.ctx().cumulative_frame_nr();
    let last = ui.data_mut(|d| {
        let last = d.get_temp::<u64>(once);
        d.insert_temp(once, frame);
        last
    });
    if last == Some(frame) {
        return;
    }
    let _ = app.run(if back { "edit.findPrevious" } else { "edit.findNext" }, json!({}));
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
            egui::Button::selectable(selected, (label, egui::Atom::grow())).wrap().min_size(vec2(width, ui.spacing().interact_size.y)),
        );
        (toggled, r)
    })
    .inner
}
