use super::{Counters, Document, LevelSuffix, NumFormat, Path, Pos, StoryRef, Stroke, Tokens, Ui, WordApp, json, regular, semibold, vec2};

type HeadingList = std::sync::Arc<Vec<(Path, u8, String)>>;
#[derive(Default)]
pub(crate) struct HeadingCache {
    stamp: Option<(u64, u64)>,
    items: HeadingList,
}

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
            let items = cached_headings(app);
            let current = items.iter().rposition(|(path, ..)| *path <= caret.path);
            if items.is_empty() {
                ui.label(egui::RichText::new(tl!("Create an interactive outline of your document.\n\nIt's a great way to keep track of where you are or quickly move your content around.\n\nTo get started, go to the Home tab and apply Heading styles to the headings in your document.")).weak());
            }
            // A heading has children when the next heading is a deeper level (#1997).
            let has_children: Vec<bool> = items.iter().enumerate().map(|(i, (_, lv, _))| items.get(i + 1).is_some_and(|n| n.1 > *lv)).collect();
            let collapsed_flags: Vec<bool> = items.iter().enumerate().map(|(i, (p, _, _))| has_children[i] && app.ui.nav_collapsed.contains(p)).collect();
            let levels: Vec<u8> = items.iter().map(|(_, lv, _)| *lv).collect();
            let visible = visible_headings(&levels, &collapsed_flags);
            for (i, (path, lv, txt)) in items.iter().enumerate() {
                if !visible[i] {
                    continue;
                }
                let collapsed = collapsed_flags[i];
                let (toggle, r) = nav_heading(ui, *lv, txt, current == Some(i), has_children[i].then_some(collapsed));
                if toggle {
                    if !app.ui.nav_collapsed.remove(path) {
                        app.ui.nav_collapsed.insert(path.clone());
                    }
                } else if r.clicked() {
                    let _ = app.run("caret.set", json!({"pos": Pos { story: StoryRef::Body, path: path.clone(), off: 0 }}));
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

/// The window owns one heading list, so other viewports cannot evict it and closing the
/// window drops it. Document identity/revision changes replace its contents.
fn cached_headings(app: &mut WordApp) -> HeadingList {
    let stamp = (app.session.document_id(), app.session.rev());
    let cache = &mut app.heading_cache;
    if cache.stamp != Some(stamp) {
        cache.items = std::sync::Arc::new(nav_headings(&app.session.doc));
        cache.stamp = Some(stamp);
    }
    cache.items.clone()
}

/// The body's headings (paragraphs with an outline level) in document order, as (path, level,
/// text). A numbered heading starts with the number it shows (`2.1 Method`), as in Word.
pub(super) fn nav_headings(doc: &Document) -> Vec<(Path, u8, String)> {
    #[cfg(test)]
    HEADING_BUILDS.with(|n| n.set(n.get() + 1));
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

#[cfg(test)]
thread_local! {
    static HEADING_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use wordcraft_doc::{CharProps, Paragraph, para_block};
    use wordcraft_engine::Session;

    fn app() -> WordApp {
        let mut doc = Document::new();
        doc.body = vec![para_block(Paragraph::with_text("First heading", CharProps::default()).styled("Heading1"))];
        WordApp::new(Session::new(doc), crate::Services::default())
    }

    fn paint(ctx: &egui::Context, app: &mut WordApp) -> Vec<String> {
        paint_viewport(ctx, app, egui::ViewportId::ROOT)
    }

    fn paint_viewport(ctx: &egui::Context, app: &mut WordApp, viewport_id: egui::ViewportId) -> Vec<String> {
        let mut input =
            egui::RawInput { viewport_id, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 600.0))), ..Default::default() };
        input.viewports.entry(viewport_id).or_default();
        let out = ctx.run_ui(input, |ui| nav(app, ui));
        let text = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .collect();
        out.drop_without_applying_deltas();
        text
    }

    #[test]
    fn navigation_repaints_do_not_rebuild_headings() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut app = app();
        HEADING_BUILDS.with(|n| n.set(0));
        for _ in 0..4 {
            assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading"));
        }
        assert_eq!(HEADING_BUILDS.with(|n| n.get()), 1, "unchanged repaints reuse the heading list");
        app.session.run("caret.docEnd", &json!({})).unwrap();
        paint(&ctx, &mut app);
        assert_eq!(HEADING_BUILDS.with(|n| n.get()), 1, "moving the caret does not rebuild document headings");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn navigation_alternating_windows_reuse_their_own_heading_lists() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut app = app();
        let first = app.windows.shown();
        app.new_window();
        let second = app.windows.shown();
        assert_ne!(first, second);
        HEADING_BUILDS.with(|n| n.set(0));
        for _ in 0..4 {
            for key in [first, second] {
                assert!(app.enter_window(key));
                assert!(paint_viewport(&ctx, &mut app, egui::ViewportId::from_hash_of(key)).iter().any(|t| t == "First heading"));
            }
        }
        assert_eq!(HEADING_BUILDS.with(|n| n.get()), 2, "each window builds once across alternating viewport passes");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn navigation_heading_cache_is_dropped_with_its_window() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut app = app();
        paint(&ctx, &mut app);
        let first = app.windows.shown();
        let weak = std::sync::Arc::downgrade(&app.heading_cache.items);
        app.new_window();
        assert_eq!(app.window_count(), 2);
        assert!(weak.upgrade().is_some(), "the parked window owns its list");
        app.close_window(first);
        assert_eq!(app.window_count(), 1);
        assert!(weak.upgrade().is_none(), "the closed window releases its list while egui stays alive");
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading"));
    }

    #[test]
    fn navigation_headings_refresh_after_edits_styles_numbering_and_history() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut app = app();
        paint(&ctx, &mut app);
        app.session.run("caret.docEnd", &json!({})).unwrap();
        app.session.run("text.insert", &json!({"text": " edited", "raw": true})).unwrap();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading edited"));
        app.session.run("edit.undo", &json!({})).unwrap();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading"));
        app.session.run("edit.redo", &json!({})).unwrap();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading edited"));
        app.session.run("para.numbering", &json!({"value": true})).unwrap();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "1. First heading edited"));
        app.session.run("edit.undo", &json!({})).unwrap();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading edited"));
        app.session.run("styles.modify", &json!({"style": "Heading1", "para": {"outlineLevel": 9}})).unwrap();
        assert!(!paint(&ctx, &mut app).iter().any(|t| t == "First heading edited"), "a style edit can remove a heading");
        app.session.run("edit.undo", &json!({})).unwrap();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading edited"));
    }

    #[test]
    fn navigation_headings_follow_document_replacement_and_switches() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut app = app();
        let first = app.session.document_id();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading"));
        let mut second = Document::new();
        second.body = vec![para_block(Paragraph::with_text("Other heading", CharProps::default()).styled("Heading1"))];
        let other = app.session.add_document(second.clone());
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "Other heading"));
        app.session.switch_document(first).unwrap();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "First heading"));
        app.session.switch_document(other).unwrap();
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "Other heading"));
        app.session.set_document(Document::new());
        assert!(!paint(&ctx, &mut app).iter().any(|t| t == "Other heading"));
        app.session.set_document(second);
        assert!(paint(&ctx, &mut app).iter().any(|t| t == "Other heading"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn navigation_cached_headings_keep_collapse_and_click_targets_live() {
        use egui_kittest::kittest::Queryable;
        let mut app = app();
        app.session.doc.body.push(para_block(Paragraph::with_text("Child heading", CharProps::default()).styled("Heading2")));
        app.session.doc.body.push(para_block(Paragraph::with_text("Last heading", CharProps::default()).styled("Heading1")));
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(400.0, 600.0))
            .with_context_setup(crate::theme::install_fonts)
            .build_ui_state(|ui, app: &mut WordApp| nav(app, ui), app);
        h.run_steps(4);
        h.get_by_label("Child heading").click();
        h.run_steps(2);
        assert_eq!(h.state().session.sel.focus.path, Path(vec![1]));
        h.state_mut().ui.nav_collapsed.insert(Path(vec![0]));
        h.run_steps(2);
        assert!(h.query_by_label("Child heading").is_none());
        h.get_by_label("Last heading").click();
        h.run_steps(2);
        assert_eq!(h.state().session.sel.focus.path, Path(vec![2]));
        h.state_mut().ui.nav_collapsed.clear();
        h.run_steps(2);
        h.get_by_label("Child heading").click();
        h.run_steps(2);
        assert_eq!(h.state().session.sel.focus.path, Path(vec![1]));
    }
}
