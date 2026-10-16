//! Side panes: Navigation (headings, pages, search results), Clipboard, Styles, Style Inspector,
//! Accessibility and Alt Text (see `a11y_pane`), Restrict Editing ([`crate::dialogs_review`]), Comments,
//! and the SmartArt Text Pane.

use egui::{Stroke, Ui, vec2};
use serde_json::{Value, json};
use wordcraft_doc::{Pos, StoryRef};
use wordcraft_engine::StylesPaneDock;

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // SmartArt Design › Text Pane, while a SmartArt graphic WordCraft can edit is selected.
    if app.session.view.smart_art_pane && wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_some() {
        egui::Panel::left("smart_art_pane")
            .default_size(250.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| {
                if header(ui, "Text Pane") {
                    let _ = app.run("smartArt.textPane", json!({"value": false}));
                    return;
                }
                crate::dialogs_smart_art::text_pane(app, ui);
            });
    }
    if app.session.view.nav_pane {
        egui::Panel::left("nav_pane")
            .default_size(260.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| nav(app, ui));
    }
    if app.session.view.clipboard_pane {
        egui::Panel::left("clipboard_pane")
            .default_size(250.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| clipboard(app, ui));
    }
    if app.session.view.styles_pane {
        styles_pane(app, ui, &t);
    }
    if app.session.view.style_inspector {
        egui::Panel::right("style_inspector")
            .default_size(240.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| inspector(app, ui));
    }
    if app.session.view.accessibility_pane {
        egui::Panel::right("accessibility_pane")
            .default_size(280.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| crate::a11y_pane::accessibility(app, ui));
    }
    if app.session.view.alt_text_pane {
        egui::Panel::right("alt_text_pane")
            .default_size(260.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| crate::a11y_pane::alt_text(app, ui));
    }
    if app.restrict.open {
        egui::Panel::right("restrict_pane")
            .default_size(270.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| {
                if header(ui, "Restrict Editing") {
                    app.restrict.open = false;
                    return;
                }
                crate::dialogs_review::restrict_pane(app, ui);
            });
    }
    if app.session.view.comments_pane {
        egui::Panel::right("comments_pane")
            .default_size(290.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| comments(app, ui));
    }
}

/// The smallest floating Styles pane, and the size it first floats at (#1756).
pub(crate) const FLOAT_MIN: egui::Vec2 = vec2(200.0, 240.0);
pub(crate) const FLOAT_DEFAULT: egui::Vec2 = vec2(270.0, 480.0);
/// Dropping the floating pane this close to the window's left or right edge docks it there, and
/// dragging a docked pane's title this far out of the pane floats it.
const DOCK_SNAP: f32 = 32.0;

/// The Styles pane, docked right (the default) or left, or floating as a window of its own (#1756).
fn styles_pane(app: &mut WordApp, ui: &mut Ui, t: &Tokens) {
    let frame = egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border));
    let panel = match app.session.view.styles_pane_dock {
        StylesPaneDock::Right => egui::Panel::right("styles_pane"),
        StylesPaneDock::Left => egui::Panel::left("styles_pane_left"),
        StylesPaneDock::Floating => {
            let screen = ui.ctx().content_rect();
            // The area the panes and the page share (below the ribbon, above the status bar):
            // a pane floated for the first time goes there rather than over the ribbon.
            let home = ui.available_rect_before_wrap();
            let mut win = egui::Window::new(tl!("Styles"))
                .id(egui::Id::new("styles_pane_float"))
                .title_bar(false)
                .collapsible(false)
                .resizable(true)
                .min_size(FLOAT_MIN)
                .frame(frame.corner_radius(6).shadow(ui.style().visuals.window_shadow));
            let rect = float_rect(app.ui.styles_pane_float, screen, home);
            // Placed where it was saved (back on screen) the first time it shows in a run; egui
            // keeps it on screen from there.
            if !std::mem::replace(&mut app.ui.styles_pane_placed, true) {
                win = win.current_pos(rect.min).default_size(rect.size());
            } else {
                win = win.default_rect(rect);
            }
            let Some(shown) = win.show(ui.ctx(), |ui| styles(app, ui)) else { return };
            let r = shown.response.rect;
            if r.is_finite() {
                app.ui.styles_pane_float = Some([r.min.x, r.min.y, r.width(), r.height()]);
            }
            // Dropped against the window's left or right edge: docked there.
            if shown.response.drag_stopped()
                && let Some(p) = ui.ctx().pointer_latest_pos()
                && let Some(side) = edge_side(p, screen)
            {
                let _ = app.run("view.stylesPaneDock", json!({"dock": side}));
            }
            return;
        }
    };
    let screen = ui.ctx().content_rect();
    let shown = panel.default_size(250.0).resizable(true).frame(frame).show(ui, |ui| styles(app, ui));
    // The title dragged out of the docked pane: docked at the window edge it was dropped against
    // (the other side in one drag), or else floating where it was dropped.
    if let Some(p) = app.ui.styles_title_dropped.take()
        && !shown.response.rect.expand(DOCK_SNAP).x_range().contains(p.x)
    {
        if let Some(side) = edge_side(p, screen) {
            let _ = app.run("view.stylesPaneDock", json!({"dock": side}));
            return;
        }
        let size = app.ui.styles_pane_float.map_or(FLOAT_DEFAULT, |r| vec2(r[2], r[3]));
        app.ui.styles_pane_float = Some([p.x - 40.0, p.y - 12.0, size.x, size.y]);
        app.ui.styles_pane_placed = false;
        let _ = app.run("view.stylesPaneDock", json!({"dock": "float"}));
    }
}

/// The window edge a pane dropped at `p` docks against, if it is that close to one.
fn edge_side(p: egui::Pos2, screen: egui::Rect) -> Option<&'static str> {
    if p.x <= screen.left() + DOCK_SNAP {
        Some("left")
    } else if p.x >= screen.right() - DOCK_SNAP {
        Some("right")
    } else {
        None
    }
}

/// Where the floating Styles pane goes: `saved` (`[x, y, width, height]`), or the default size
/// right of the middle of `home` (the area below the ribbon), sized to at least [`FLOAT_MIN`]
/// and at most the screen, and moved fully on screen. Junk (NaN, infinite, negative) never
/// escapes: the result is always finite.
pub(crate) fn float_rect(saved: Option<[f32; 4]>, screen: egui::Rect, home: egui::Rect) -> egui::Rect {
    let screen = if screen.is_finite() && screen.is_positive() { screen } else { egui::Rect::from_min_size(egui::Pos2::ZERO, FLOAT_DEFAULT) };
    let home = home.intersect(screen);
    let home = if home.is_finite() && home.is_positive() { home } else { screen };
    let size = saved.map_or(FLOAT_DEFAULT, |r| vec2(r[2], r[3]));
    let size = vec2(if size.x.is_finite() { size.x } else { FLOAT_DEFAULT.x }, if size.y.is_finite() { size.y } else { FLOAT_DEFAULT.y })
        .max(FLOAT_MIN)
        .min(screen.size());
    let free = (home.size() - size).max(egui::Vec2::ZERO);
    // Whole points, so the rectangle's edges don't pick up float noise (198.00002).
    let default = (home.min + vec2(free.x * 0.75, free.y * 0.3)).round();
    let pos = saved.map_or(default, |r| egui::pos2(if r[0].is_finite() { r[0] } else { default.x }, if r[1].is_finite() { r[1] } else { default.y }));
    // Element-wise max/min rather than `clamp`, which panics should rounding put max below min.
    let pos = pos.max(screen.min).min((screen.max - size).max(screen.min));
    egui::Rect::from_min_size(pos, size)
}

pub(crate) fn header(ui: &mut Ui, title: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!(title)).font(semibold(15.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Drawn, not the "✕" character: the interface fonts have no glyph for it.
            if crate::widgets::icon_button(ui, "close", tl!("Close")).clicked() {
                close = true;
            }
        });
    });
    ui.add_space(4.0);
    close
}

/// The Styles pane's header: its title, which can be dragged out to float a docked pane, a
/// menu to dock it left or right or float it (#1756), and Close. `true` when Close was clicked.
fn styles_header(app: &mut WordApp, ui: &mut Ui) -> bool {
    let mut close = false;
    let dock = app.session.view.styles_pane_dock;
    ui.horizontal(|ui| {
        let floating = dock == StylesPaneDock::Floating;
        // A floating pane is moved by egui's window drag, so only a docked title takes the drag.
        let sense = if floating { egui::Sense::hover() } else { egui::Sense::drag() };
        // Not selectable: a selectable label takes the drag for text selection, so a floating
        // pane couldn't be moved by its title.
        let title = ui.add(egui::Label::new(egui::RichText::new(tl!("Styles")).font(semibold(15.0))).selectable(false).sense(sense));
        if !floating {
            let title = title.on_hover_text(tl!("Drag out to float the pane"));
            if title.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if title.drag_stopped() {
                app.ui.styles_title_dropped = ui.ctx().pointer_latest_pos();
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::icon_button(ui, "close", tl!("Close")).clicked() {
                close = true;
            }
            let more = crate::widgets::icon_button(ui, "dropdown", tl!("Pane position"));
            egui::Popup::menu(&more).show(|ui| {
                for (d, label) in [(StylesPaneDock::Left, "Dock Left"), (StylesPaneDock::Right, "Dock Right"), (StylesPaneDock::Floating, "Float")] {
                    if ui.selectable_label(dock == d, tl!(label)).clicked() {
                        if d == StylesPaneDock::Floating {
                            app.ui.styles_pane_placed = false;
                        }
                        let _ = app.run("view.stylesPaneDock", json!({"dock": d.name()}));
                        ui.close();
                    }
                }
            });
        });
    });
    ui.add_space(4.0);
    close
}

fn nav(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Navigation") {
        let _ = app.run("view.navigationPane", json!({"value": false}));
        return;
    }
    let qid = egui::Id::new("nav_query");
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
        ui.data_mut(|d| d.insert_temp(qid, q.clone()));
        if !q.is_empty() {
            let _ = app.session.run("edit.find", &json!({"text": q}));
        } else {
            app.session.find.results.clear();
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
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
            let mut current: Option<usize> = None;
            let mut items = Vec::new();
            for path in app.session.doc.para_paths(StoryRef::Body) {
                let Some(p) = app.session.doc.para(StoryRef::Body, &path) else { continue };
                let rp = app.session.doc.styles.resolve_para(&p.props);
                if let Some(lv) = rp.outline_level {
                    let txt = p.plain_text().trim().to_string();
                    if txt.is_empty() || rp.style.starts_with("TOC") {
                        continue;
                    }
                    if path <= caret.path {
                        current = Some(items.len());
                    }
                    items.push((path, lv, txt));
                }
            }
            if items.is_empty() {
                ui.label(egui::RichText::new(tl!("Create an interactive outline of your document.\n\nIt's a great way to keep track of where you are or quickly move your content around.\n\nTo get started, go to the Home tab and apply Heading styles to the headings in your document.")).weak());
            }
            for (i, (path, lv, txt)) in items.into_iter().enumerate() {
                if nav_heading(ui, lv, &txt, current == Some(i)).clicked() {
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

/// Constrain each heading to the available pane width so long document text cannot
/// enlarge the panel. The full heading stays accessible on hover.
fn nav_heading(ui: &mut Ui, level: u8, text: &str, selected: bool) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_space(level as f32 * 14.0);
        let label = egui::RichText::new(text).font(if level == 0 { semibold(12.5) } else { regular(12.5) });
        ui.add_sized(
            vec2(ui.available_width(), ui.spacing().interact_size.y),
            // The trailing grow atom takes the spare width, so the text starts flush left
            // under its parent instead of centred, as in Word (#1199).
            egui::Button::selectable(selected, (label, egui::Atom::grow())).truncate(),
        )
        .on_hover_text(text)
    })
    .inner
}

/// Items collected by Copy and Cut, newest first; clicking one pastes it at the caret.
fn clipboard(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Clipboard") {
        let _ = app.run("edit.clipboardPane", json!({"value": false}));
        return;
    }
    let items: Vec<String> = app.session.clip_history.items().iter().map(|i| i.preview()).collect();
    ui.horizontal(|ui| {
        if ui.add_enabled(!items.is_empty(), egui::Button::new(tl!("Paste All"))).clicked() {
            let _ = app.run("edit.pasteAllClipboard", json!({}));
            app.canvas.want_focus = true;
        }
        if ui.add_enabled(!items.is_empty(), egui::Button::new(tl!("Clear All"))).clicked() {
            let _ = app.run("edit.clearClipboard", json!({}));
        }
    });
    ui.add_space(4.0);
    let hint = if items.is_empty() { "Nothing collected yet. Items you copy or cut appear here." } else { "Click an item to paste it." };
    ui.label(egui::RichText::new(tl!(hint)).small().weak());
    ui.separator();
    let t = Tokens::get(ui.ctx());
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, preview) in items.iter().enumerate() {
            egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let text = if preview.is_empty() { tl!("(picture or object)").to_string() } else { preview.clone() };
                let mut job =
                    egui::text::LayoutJob::single_section(text, egui::TextFormat { font_id: regular(12.5), color: t.text, ..Default::default() });
                job.wrap.max_width = ui.available_width();
                job.wrap.max_rows = 3;
                if ui.add(egui::Button::new(job).frame(false)).on_hover_text(tl!("Paste")).clicked() {
                    let _ = app.run("edit.pasteClipboardItem", json!({"index": i}));
                    app.canvas.want_focus = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.small_button(tl!("Delete")).clicked() {
                        let _ = app.run("edit.deleteClipboardItem", json!({"index": i}));
                    }
                });
            });
            ui.add_space(6.0);
        }
    });
}

fn styles(app: &mut WordApp, ui: &mut Ui) {
    if styles_header(app, ui) {
        let _ = app.run("view.stylesPane", json!({"value": false}));
        return;
    }
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    let cur = st.get("style").and_then(Value::as_str).unwrap_or("Normal").to_string();
    let current_name = app
        .session
        .doc
        .styles
        .get(&cur)
        .map(crate::i18n::style_name)
        .unwrap_or_else(|| st.get("styleName").and_then(Value::as_str).unwrap_or("").to_string());
    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Current style: {style}"), &[("style", &current_name)])).small());
    ui.horizontal(|ui| {
        if ui.button(tl!("New Style…")).clicked() {
            app.dialog = crate::dialogs::Dialog::open("newStyle", app);
        }
        if ui.button(tl!("Update to Match")).clicked() {
            let _ = app.run("styles.updateToMatch", json!({}));
        }
        if ui.button(tl!("Clear")).clicked() {
            let _ = app.run("format.clear", json!({}));
        }
    });
    if ui.button(tl!("Manage Styles…")).clicked() {
        let _ = app.run("styles.manage", json!({}));
    }
    if ui.selectable_label(app.session.view.style_inspector, tl!("Style Inspector")).clicked() {
        let _ = app.run("styles.inspector", json!({}));
    }
    ui.separator();
    let mut list: Vec<(String, String, bool)> = app
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| !s.hidden && s.kind != wordcraft_doc::StyleKind::Table)
        .map(|s| (s.id.clone(), crate::i18n::style_name(s), s.kind == wordcraft_doc::StyleKind::Character))
        .collect();
    list.sort_by_key(|s| s.1.to_lowercase());
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, name, is_char) in list {
            ui.horizontal(|ui| {
                let r = ui.selectable_label(id == cur, &name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(if is_char { "a" } else { "¶" }).weak());
                });
                if r.clicked() {
                    let _ = app.run("para.style", json!({"style": id}));
                    app.canvas.want_focus = true;
                }
                r.context_menu(|ui| {
                    if ui.button(tl!("Modify…")).clicked() {
                        app.dialog = crate::dialogs::Dialog::modify_style(app, &id);
                        ui.close();
                    }
                    if ui.button(tl!("Update to Match Selection")).clicked() {
                        let _ = app.run("styles.updateToMatch", json!({"style": id}));
                        ui.close();
                    }
                    if ui.button(tl!("Delete")).clicked() {
                        let _ = app.run("styles.delete", json!({"style": id}));
                        ui.close();
                    }
                });
            });
        }
    });
}

fn comments(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Comments") {
        let _ = app.run("view.commentsPane", json!({"value": false}));
        return;
    }
    if crate::widgets::icon_text_button(ui, "newComment", tl!("New Comment"), regular(12.5)).clicked() {
        let _ = app.run("review.newComment", json!({"text": ""}));
    }
    ui.separator();
    let list = app.session.run("review.comments", &json!({})).unwrap_or_default();
    let t = Tokens::get(ui.ctx());
    egui::ScrollArea::vertical().show(ui, |ui| {
        for c in list.as_array().cloned().unwrap_or_default() {
            let id = c.get("id").and_then(Value::as_u64).unwrap_or(0);
            let reply = c.get("parent").is_some_and(|p| !p.is_null());
            egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if reply {
                    ui.add_space(0.0);
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(c.get("author").and_then(Value::as_str).unwrap_or("")).font(semibold(12.0)));
                    ui.label(egui::RichText::new(comment_stamp(c.get("date").and_then(Value::as_str).unwrap_or(""))).small().weak());
                    if c.get("resolved").and_then(Value::as_bool).unwrap_or(false) {
                        ui.label(egui::RichText::new(tl!("Resolved")).small().color(t.green));
                    }
                });
                // Editable comment text (one undo step per edit, like the balloons).
                comment_editor(app, ui, id as u32, None, false);
                let open_key = egui::Id::new(("comment_reply_open", id));
                let mut replying = ui.data(|d| d.get_temp::<bool>(open_key)).unwrap_or(false);
                ui.horizontal(|ui| {
                    if ui.small_button(tl!("Go to")).clicked()
                        && let Some(a) = c.get("anchor").filter(|a| !a.is_null())
                    {
                        let _ = app.run("caret.set", json!({"pos": a}));
                    }
                    if ui.small_button(tl!("Reply")).clicked() {
                        replying = true;
                        ui.data_mut(|d| d.insert_temp(open_key, true));
                        ui.data_mut(|d| d.insert_temp(egui::Id::new(("comment_reply_focus", id)), true));
                    }
                    let resolved = c.get("resolved").and_then(Value::as_bool).unwrap_or(false);
                    if ui.small_button(if resolved { tl!("Reopen") } else { tl!("Resolve") }).clicked() {
                        let _ = app.run("review.resolveComment", json!({"id": id}));
                    }
                    if ui.small_button(tl!("Delete")).clicked() {
                        let _ = app.run("review.deleteComment", json!({"id": id}));
                    }
                });
                if replying {
                    let focus = ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(("comment_reply_focus", id)))).unwrap_or(false);
                    if reply_editor(app, ui, id as u32, None, focus) {
                        ui.data_mut(|d| d.remove::<bool>(open_key));
                    }
                }
            });
            ui.add_space(6.0);
        }
    });
}

fn edit_key(id: u32) -> egui::Id {
    egui::Id::new(("comment_edit", id))
}

fn reply_key(id: u32) -> egui::Id {
    egui::Id::new(("comment_reply", id))
}

/// A comment's text, editable in place (the Comments pane and the balloons). Typing stays in a
/// draft until the field loses focus; then [`commit_comment`] writes it back with one
/// `review.editComment`, so each editing session is one undo step.
pub(crate) fn comment_editor(app: &mut WordApp, ui: &mut Ui, id: u32, font: Option<egui::FontId>, focus: bool) {
    let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part) else { return };
    let key = edit_key(id);
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| app.session.doc.plain_text(StoryRef::Part(part)));
    let mut edit = egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(f32::INFINITY).hint_text(tl!("Add a comment"));
    if let Some(f) = font {
        edit = edit.font(f);
    }
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
    if r.lost_focus() {
        commit_comment(app, ui.ctx(), id);
    }
}

/// Write a comment's draft text back, when it has one that differs from the comment.
pub(crate) fn commit_comment(app: &mut WordApp, ctx: &egui::Context, id: u32) {
    let Some(text) = ctx.data_mut(|d| d.remove_temp::<String>(edit_key(id))) else { return };
    let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part) else { return };
    if text != app.session.doc.plain_text(StoryRef::Part(part)) {
        let _ = app.run("review.editComment", json!({"id": id, "text": text}));
    }
}

/// The comment a reply to `id` answers: replies to a reply join its thread.
fn thread_root(app: &WordApp, id: u32) -> u32 {
    match app.session.doc.comments.get(&id).and_then(|c| c.parent) {
        Some(p) if app.session.doc.comments.contains_key(&p) => p,
        _ => id,
    }
}

/// A field for replying to a comment. When it loses focus a non-empty reply is posted with
/// `review.reply`; returns true when the field is done (posted or left empty).
pub(crate) fn reply_editor(app: &mut WordApp, ui: &mut Ui, id: u32, font: Option<egui::FontId>, focus: bool) -> bool {
    let key = reply_key(id);
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_default();
    let mut edit = egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(f32::INFINITY).hint_text(tl!("Write a reply"));
    if let Some(f) = font {
        edit = edit.font(f);
    }
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
    if r.lost_focus() {
        commit_reply(app, ui.ctx(), id);
        return true;
    }
    false
}

/// Post a pending reply draft to comment `id` (nothing when there is none or it is blank).
pub(crate) fn commit_reply(app: &mut WordApp, ctx: &egui::Context, id: u32) {
    let Some(text) = ctx.data_mut(|d| d.remove_temp::<String>(reply_key(id))) else { return };
    if !text.trim().is_empty() {
        let root = thread_root(app, id);
        let _ = app.run("review.reply", json!({"id": root, "text": text.trim_end()}));
    }
}

/// Style Inspector: the paragraph and character levels at the caret, each with its style and the
/// direct formatting on top, and a button to clear each level.
fn inspector(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Style Inspector") {
        let _ = app.run("styles.inspector", json!({"value": false}));
        return;
    }
    // Read-only, so call it directly: running the command every frame would pay for the command
    // machinery and consume per-command state (a pending `join_next_undo`) just to draw the pane.
    let r = wordcraft_engine::cmd::inspector::inspect(&app.session);
    let t = Tokens::get(ui.ctx());
    let none = tl!("None").to_string();
    let para = &r["paragraph"];
    let chr = &r["character"];
    let pstyle = para["styleName"].as_str().unwrap_or("Normal").to_string();
    let cstyle = chr["styleName"].as_str().map(str::to_string);
    // Built-in style names show in the interface language; the document keeps the English ones.
    let shown = |id: &Value, name: &str| {
        id.as_str().and_then(|id| app.session.doc.styles.get(id)).map(crate::i18n::style_name).unwrap_or_else(|| name.to_string())
    };
    let pstyle_shown = shown(&para["style"], &pstyle);
    let cstyle_shown = cstyle.as_deref().map(|n| shown(&chr["style"], n));
    let pdirect = direct_list(&para["direct"]);
    let cdirect = direct_list(&chr["direct"]);
    let mut clear: Option<&str> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        section(ui, &t, "Paragraph", |ui| {
            if level_row(ui, &t, "Style", &pstyle_shown, false, "Reset to Normal", pstyle == "Normal" && para["style"] == "Normal") {
                clear = Some("paragraphStyle");
            }
            let (txt, empty) = if pdirect.is_empty() { (none.clone(), true) } else { (pdirect.join(", "), false) };
            if level_row(ui, &t, "Direct formatting", &txt, empty, "Clear Paragraph Formatting", empty) {
                clear = Some("paragraphFormatting");
            }
        });
        ui.add_space(8.0);
        section(ui, &t, "Characters", |ui| {
            let (txt, empty) = match &cstyle_shown {
                Some(n) => (n.clone(), false),
                None => (none.clone(), true),
            };
            if level_row(ui, &t, "Style", &txt, empty, "Clear Character Style", empty) {
                clear = Some("characterStyle");
            }
            let (txt, empty) = if cdirect.is_empty() { (none.clone(), true) } else { (cdirect.join(", "), false) };
            if level_row(ui, &t, "Direct formatting", &txt, empty, "Clear Character Formatting", empty) {
                clear = Some("characterFormatting");
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(tl!("Clear All")).on_hover_text(tl!("Clear All Formatting")).clicked() {
                let _ = app.run("format.clear", json!({}));
                app.canvas.want_focus = true;
            }
            if ui.button(tl!("Styles Pane")).clicked() {
                let _ = app.run("view.stylesPane", json!({"value": true}));
            }
        });
    });
    if let Some(level) = clear {
        let _ = app.run("styles.inspectorClear", json!({"level": level}));
        app.canvas.want_focus = true;
    }
}

/// A titled, bordered group in the inspector.
fn section(ui: &mut Ui, t: &Tokens, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.label(egui::RichText::new(tl!(title)).font(semibold(12.5)).color(t.text));
    ui.add_space(2.0);
    egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(4).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
}

/// One level: a caption, its value, and a clear button (disabled when there's nothing to clear).
/// Returns true when the button was clicked.
fn level_row(ui: &mut Ui, t: &Tokens, caption: &str, value: &str, dim: bool, tip: &str, disabled: bool) -> bool {
    ui.label(egui::RichText::new(tl!(caption)).font(regular(11.0)).color(t.text_dim));
    let mut clicked = false;
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 26.0).max(40.0);
        ui.allocate_ui(vec2(w, 18.0), |ui| {
            ui.set_width(w);
            let text = egui::RichText::new(value).font(regular(12.5)).color(if dim { t.text_disabled } else { t.text });
            ui.add(egui::Label::new(text).wrap());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 20.0), egui::Sense::click());
            let enabled = !disabled;
            if enabled && resp.hovered() {
                ui.painter().rect_filled(r, 3.0, t.hover);
            }
            let c = if enabled { t.icon } else { t.text_disabled };
            crate::icons::paint(
                ui.painter(),
                egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
                "clear",
                c,
                if enabled { t.accent } else { c },
            );
            let resp = resp.on_hover_text(tl!(tip));
            clicked = enabled && resp.clicked();
        });
    });
    ui.add_space(4.0);
    clicked
}

/// Readable descriptions of the inspector's `direct` entries (`[{"prop", "value"}]`).
fn direct_list(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().take(64).map(|d| describe(d["prop"].as_str().unwrap_or(""), &d["value"])).collect()).unwrap_or_default()
}

/// Each `{"prop", "value"}` of a list (the Style Inspector's, or a formatting change's) as text.
pub(crate) fn describe_props(list: &[Value]) -> Vec<String> {
    list.iter().take(64).map(|d| describe(d["prop"].as_str().unwrap_or(""), &d["value"])).collect()
}

/// "Bold", "Not Italic", "Font: Arial", "Size: 14 pt"…
fn describe(prop: &str, v: &Value) -> String {
    let label = match prop {
        "font" => "Font",
        "size" => "Size",
        "bold" => "Bold",
        "italic" => "Italic",
        "underline" => "Underline",
        "underlineColor" => "Underline color",
        "strike" => "Strikethrough",
        "doubleStrike" => "Double strikethrough",
        "color" => "Font color",
        "highlight" => "Highlight",
        "shading" => "Shading",
        "vertAlign" => "Position",
        "caps" => "All caps",
        "smallCaps" => "Small caps",
        "hidden" => "Hidden",
        "spacing" => "Character spacing",
        "scale" => "Scale",
        "position" => "Raised/lowered",
        "outline" => "Outline",
        "shadow" => "Shadow",
        "emboss" => "Emboss",
        "engrave" => "Engrave",
        "align" => "Alignment",
        "indentLeft" => "Left indent",
        "indentRight" => "Right indent",
        "indentFirst" => "First line",
        "spaceBefore" => "Space before",
        "spaceAfter" => "Space after",
        "lineSpacing" => "Line spacing",
        "contextualSpacing" => "Don't add space between paragraphs of the same style",
        "mirrorIndents" => "Mirror indents",
        "keepNext" => "Keep with next",
        "keepLines" => "Keep lines together",
        "pageBreakBefore" => "Page break before",
        "widowControl" => "Widow/Orphan control",
        "outlineLevel" => "Outline level",
        "tabs" => "Tabs",
        "borders" => "Borders",
        "suppressHyphens" => "Don't hyphenate",
        "suppressLineNumbers" => "Suppress line numbers",
        "bidi" => "Right-to-left",
        "dropCap" => "Drop cap",
        "style" => "Style",
        "numbering" => "Numbering",
        "lang" => "Language",
        other => other,
    };
    let label = tl!(label);
    let pt = |x: f64| crate::i18n::fmt(tl!("{n} pt"), &[("n", &trim_num(x))]);
    let hex = |c: &Value| -> Option<String> {
        let a = c.as_array()?;
        let ch = |i: usize| a.get(i).and_then(Value::as_u64).unwrap_or(0).min(255);
        Some(format!("#{:02X}{:02X}{:02X}", ch(0), ch(1), ch(2)))
    };
    let value = match (prop, v) {
        (_, Value::Bool(true)) => return label.to_string(),
        (_, Value::Bool(false)) => return crate::i18n::fmt(tl!("Not {name}"), &[("name", label)]),
        ("scale", Value::Number(n)) => format!("{}%", trim_num(n.as_f64().unwrap_or(100.0))),
        ("outlineLevel", Value::Number(n)) => match n.as_u64().unwrap_or(9) {
            l @ 0..=8 => crate::i18n::fmt(tl!("Level {n}"), &[("n", &(l + 1).to_string())]),
            _ => tl!("Body Text").to_string(),
        },
        ("dropCap", Value::Number(n)) => crate::i18n::fmt(tl!("{n} lines"), &[("n", &n.to_string())]),
        (_, Value::Number(n)) => pt(n.as_f64().unwrap_or(0.0)),
        ("align", Value::String(s)) => tl!(match s.as_str() {
            "center" => "Centered",
            "right" => "Right",
            "justify" => "Justified",
            "distribute" => "Distributed",
            _ => "Left",
        })
        .to_string(),
        ("color", Value::String(_)) => tl!("Automatic").to_string(),
        ("color", Value::Object(o)) => o.get("Rgb").and_then(hex).unwrap_or_default(),
        (_, Value::Array(_)) if prop != "tabs" => hex(v).unwrap_or_default(),
        ("lineSpacing", Value::Object(o)) => {
            let x = o.get("value").and_then(Value::as_f64).unwrap_or(1.0);
            match o.get("rule").and_then(Value::as_str) {
                Some("atLeast") => crate::i18n::fmt(tl!("At least {n}"), &[("n", &pt(x))]),
                Some("exactly") => crate::i18n::fmt(tl!("Exactly {n}"), &[("n", &pt(x))]),
                _ => crate::i18n::fmt(tl!("{n} lines"), &[("n", &trim_num(x))]),
            }
        }
        (_, Value::String(s)) => tl!(&camel_words(s)).to_string(),
        _ => return label.to_string(),
    };
    format!("{label}: {value}")
}

/// `1.50` → `1.5`, `12.0` → `12`.
fn trim_num(x: f64) -> String {
    let s = format!("{:.2}", if x.is_finite() { x } else { 0.0 });
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `2026-10-12T04:44:00Z` → `2026-10-12 04:44` (date and time, no seconds) for comment cards.
fn comment_stamp(iso: &str) -> String {
    iso.chars().take(16).map(|c| if c == 'T' { ' ' } else { c }).collect()
}

/// `dotDash` → `Dot dash` (enum values from the document model).
fn camel_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().take(64).enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::comment_stamp;

    #[test]
    fn comment_stamp_keeps_date_and_time() {
        assert_eq!(comment_stamp("2026-10-12T04:44:31Z"), "2026-10-12 04:44");
        assert_eq!(comment_stamp("2026-10-12"), "2026-10-12");
        assert_eq!(comment_stamp(""), "");
    }
}

#[cfg(test)]
mod styles_float_tests {
    use super::*;

    /// #1756: a saved floating Styles pane comes back on screen, whatever the file says.
    #[test]
    fn floating_styles_pane_stays_on_screen() {
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1440.0, 900.0));
        let on = |r: egui::Rect| screen.contains_rect(r) && r.is_finite() && r.width() >= FLOAT_MIN.x && r.height() >= FLOAT_MIN.y;
        // A sane rectangle is kept as it is.
        assert_eq!(
            float_rect(Some([300.0, 120.0, 280.0, 500.0]), screen, screen),
            egui::Rect::from_min_size(egui::pos2(300.0, 120.0), vec2(280.0, 500.0))
        );
        // None: the default size, right of the middle.
        let d = float_rect(None, screen, screen);
        assert!(on(d), "{d:?}");
        assert_eq!(d.size(), FLOAT_DEFAULT);
        // Saved on a larger monitor: moved back on screen, size kept.
        let r = float_rect(Some([3000.0, 2000.0, 280.0, 500.0]), screen, screen);
        assert!(on(r), "{r:?}");
        assert_eq!(r.size(), vec2(280.0, 500.0));
        // Negative positions, tiny, huge, NaN and infinite values.
        for junk in [
            [-5000.0, -20.0, 280.0, 500.0],
            [10.0, 10.0, 1.0, -3.0],
            [10.0, 10.0, 1.0e9, 1.0e9],
            [f32::NAN, 10.0, 280.0, 500.0],
            [10.0, 10.0, f32::NAN, f32::INFINITY],
            [f32::NEG_INFINITY, f32::INFINITY, 0.0, 0.0],
        ] {
            let r = float_rect(Some(junk), screen, screen);
            assert!(on(r), "{junk:?} → {r:?}");
        }
        // A screen smaller than the minimum pane: the pane fills it from its top-left corner.
        let small = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(100.0, 80.0));
        let r = float_rect(Some([50.0, 50.0, 280.0, 500.0]), small, small);
        assert!(r.is_finite() && r.min == egui::Pos2::ZERO, "{r:?}");
        // A screen that isn't finite: still a finite rectangle.
        let bad = egui::Rect::from_min_max(egui::pos2(f32::NAN, 0.0), egui::pos2(f32::INFINITY, 900.0));
        assert!(float_rect(Some([1.0, 1.0, 280.0, 500.0]), bad, bad).is_finite());

        // #1756 review: first floated, the pane goes below the ribbon (inside `home`), not over it;
        // a `home` that is junk, off screen or too small for the pane still gives a rect on screen.
        let home = egui::Rect::from_min_max(egui::pos2(0.0, 160.0), egui::pos2(1440.0, 870.0));
        let d = float_rect(None, screen, home);
        assert!(home.contains_rect(d), "{d:?}");
        for home in [
            egui::Rect::NOTHING,
            egui::Rect::from_min_max(egui::pos2(f32::NAN, 0.0), egui::pos2(10.0, 10.0)),
            egui::Rect::from_min_size(egui::pos2(5000.0, 5000.0), vec2(100.0, 100.0)),
            egui::Rect::from_min_size(egui::pos2(0.0, 800.0), vec2(1440.0, 50.0)),
        ] {
            let r = float_rect(None, screen, home);
            assert!(on(r), "{home:?} → {r:?}");
        }
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    #[test]
    fn long_headings_truncate_without_stretching_navigation() {
        let text = "An extremely long heading with words that must stay inside the navigation pane ".repeat(20);
        for (width, level) in [(260.0, 0), (180.0, 5)] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0));
            let mut panel_rect = egui::Rect::NOTHING;
            let mut heading_rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                let result = egui::Panel::left("navigation_test")
                    .default_size(width)
                    .resizable(true)
                    .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| nav_heading(ui, level, &text, true)).inner);
                panel_rect = result.response.rect;
                heading_rect = result.inner.rect;
            });

            assert!(panel_rect.width() < width + 20.0, "long heading expanded the {width}px pane to {panel_rect:?}");
            assert!(heading_rect.max.x <= panel_rect.max.x, "heading escaped pane: {heading_rect:?} / {panel_rect:?}");
            assert!(
                output.shapes.iter().any(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        t.galley.elided && t.galley.rows.last().is_some_and(|r| r.row.glyphs.last().is_some_and(|g| g.chr == '…'))
                    }
                    _ => false,
                }),
                "long heading must render a trailing ellipsis at {width}px"
            );
            output.drop_without_applying_deltas();
        }
    }

    /// Word lists headings flush left, each level indented under its parent (#1199).
    #[test]
    fn headings_align_left_and_indent_by_level() {
        let text_left = |level: u8| {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0));
            let mut panel_left = 0.0;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                let result = egui::Panel::left("navigation_test").default_size(260.0).show(ui, |ui| nav_heading(ui, level, "Kilns", false));
                panel_left = result.response.rect.min.x;
            });
            let x = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "Kilns" => Some(t.pos.x + t.galley.rect.min.x),
                    _ => None,
                })
                .unwrap();
            output.drop_without_applying_deltas();
            x - panel_left
        };
        let (top, second) = (text_left(0), text_left(1));
        assert!(top < 30.0, "a top-level heading starts at the pane's left edge, not centred: {top}px in");
        assert!((second - top - 14.0).abs() < 0.5, "a level-2 heading is indented 14px under its parent: {top} / {second}");
    }
}
