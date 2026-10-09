//! Title bar (Quick Access Toolbar, title, search, account) and status bar.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::StoryRef;

use crate::theme::{Tokens, medium, regular, semibold};
use crate::{WordApp, icons};
use crate::i18n::{Language, tr};

fn tl(app: &WordApp, s: &str) -> String { tr(app.ui_language(), s) }

fn qat_button(ui: &mut Ui, app: &mut WordApp, icon: &str, tip: &str, id: &str, enabled: bool) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
    if resp.hovered() && enabled {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let c = if enabled { t.icon } else { t.text_disabled };
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), icon, c, if enabled { t.accent } else { c });
    let sc = crate::widgets::shortcut_text(app, id);
    let resp = resp.on_hover_text(if sc.is_empty() { tip.to_string() } else { format!("{tip} ({sc})") });
    if resp.clicked() && enabled {
        let _ = app.run(id, json!({}));
    }
}

pub fn title_bar(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let left = if app.integrated_titlebar { 78 } else { 8 };
    egui::Panel::top("title_bar")
        .exact_size(38.0)
        .frame(egui::Frame::NONE.fill(t.title_bar).inner_margin(egui::Margin { left, right: 10, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let full = ui.max_rect();
            // Drag the window by the title bar (integrated title bar on macOS).
            let drag = ui.interact(full, ui.id().with("title_drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            let mut qat_end = full.min.x;
            let mut right_start = full.max.x;
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                // AutoSave toggle (on when the document has a path).
                let has_path = app.session.path.is_some();
                ui.label(egui::RichText::new("AutoSave").font(regular(11.5)).color(t.text_dim));
                let (r, resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
                let on = has_path && app.canvas_autosave();
                ui.painter().rect(
                    r,
                    8.0,
                    if on { t.accent } else { t.input },
                    Stroke::new(1.0, if on { t.accent } else { t.border_strong }),
                    egui::StrokeKind::Inside,
                );
                let knob = if on { r.max.x - 8.0 } else { r.min.x + 8.0 };
                ui.painter().circle_filled(pos2(knob, r.center().y), 5.0, if on { egui::Color32::WHITE } else { t.text_dim });
                if resp.on_hover_text("AutoSave saves every change to the file (needs a saved document)").clicked() {
                    if has_path {
                        app.toggle_autosave();
                    } else {
                        app.save_as_dialog();
                    }
                }
                ui.add_space(8.0);
                let lang_label = if app.is_rtl() { "EN" } else { "עב" };
                if ui.button(egui::RichText::new(lang_label).font(semibold(11.0))).on_hover_text(tl(app, "Language")).clicked() {
                    let next = if app.is_rtl() { "en" } else { "he" };
                    let _ = app.run("ui.language", json!({"language": next}));
                    ui.ctx().request_repaint();
                }
                qat_button(ui, app, "save", "Save", "file.save", true);
                let can_undo = app.session.can_undo();
                let can_redo = app.session.can_redo();
                qat_button(ui, app, "undo", &format!("Undo {}", app.session.undo_label().unwrap_or("")), "edit.undo", can_undo);
                qat_button(ui, app, "redo", "Redo", "edit.redo", can_redo);
                qat_button(ui, app, "more", "Customize Quick Access Toolbar", "ui.dialog", true);
                qat_end = ui.min_rect().max.x;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Account / community.
                    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 12.0, t.accent);
                    let initials: String = app.session.author.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, initials, semibold(10.5), egui::Color32::WHITE);
                    resp.on_hover_text(format!("{} — set your name in File › Options", app.session.author));
                    ui.add_space(8.0);
                    if app.ui.show_discord {
                        let (r, resp) = ui.allocate_exact_size(vec2(84.0, 22.0), Sense::click());
                        ui.painter().rect_filled(
                            r,
                            11.0,
                            if resp.hovered() { egui::Color32::from_rgb(0x47, 0x52, 0xC4) } else { egui::Color32::from_rgb(0x58, 0x65, 0xF2) },
                        );
                        icons::paint(
                            ui.painter(),
                            Rect::from_center_size(pos2(r.min.x + 13.0, r.center().y), vec2(14.0, 14.0)),
                            "discord",
                            egui::Color32::WHITE,
                            egui::Color32::WHITE,
                        );
                        ui.painter().text(pos2(r.min.x + 24.0, r.center().y), Align2::LEFT_CENTER, "Discord", medium(11.5), egui::Color32::WHITE);
                        if resp.on_hover_text("Join the ArtCraft community on Discord").clicked() {
                            let _ = app.run("ui.discord", json!({}));
                        }
                        ui.add_space(8.0);
                    }
                    // Search ("אמור לי").
                    let (r, resp) = ui.allocate_exact_size(vec2(260.0, 26.0), Sense::click());
                    ui.painter().rect(r, 6.0, if resp.hovered() { t.input } else { t.ribbon }, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    icons::paint(ui.painter(), Rect::from_min_size(r.min + vec2(8.0, 5.0), vec2(16.0, 16.0)), "search", t.text_dim, t.accent);
                    ui.painter().text(pos2(r.min.x + 30.0, r.center().y), Align2::LEFT_CENTER, "Search commands and help", regular(12.0), t.text_dim);
                    if resp.clicked() {
                        let _ = app.run("ui.dialog", json!({"name": "commands"}));
                    }
                    right_start = ui.min_rect().min.x;
                });
            });
            // Centred title.
            let title = format!(
                "{} — {}",
                app.title_stem(),
                if app.session.dirty {
                    "Edited"
                } else if app.session.path.is_some() {
                    "Saved"
                } else {
                    "לא נשמר"
                }
            );
            let g = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.clone(), semibold(12.5), t.text));
            let w = g.size().x;
            let cx = full.center().x;
            if cx - w / 2.0 > qat_end + 10.0 && cx + w / 2.0 < right_start - 10.0 {
                ui.painter().galley(pos2(cx - w / 2.0, full.center().y - g.size().y / 2.0), g, t.text);
            }
        });
}

pub fn status_bar(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("status")
        .exact_size(26.0)
        .frame(
            egui::Frame::NONE
                .fill(t.status_bar)
                .inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 0 })
                .stroke(Stroke::new(1.0, t.border)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = vec2(14.0, 0.0);
                let l = app.session.layout();
                let page = l.caret_on(&app.session.sel.focus, app.session.page_hint).map(|c| c.page + 1).unwrap_or(1);
                let st = |ui: &mut Ui, s: &str| {
                    ui.add(egui::Label::new(egui::RichText::new(s).font(regular(11.5)).color(t.text_dim)).sense(Sense::click()))
                };
                if st(ui, &format!("Page {page} of {}", l.pages.len())).on_hover_text("Go To (⌘⌥G)").clicked() {
                    let _ = app.run("ui.dialog", json!({"name": "goto"}));
                }
                let words = app.cached_word_count();
                let wtxt = if app.session.sel.is_collapsed() {
                    format!("{words} words")
                } else {
                    let sw = wordcraft_doc::count_words(&app.session.selected_text());
                    format!("{sw} of {words} words")
                };
                if st(ui, &wtxt).clicked() {
                    let _ = app.run("ui.dialog", json!({"name": "wordCount"}));
                }
                if app.session.sel.focus.story != StoryRef::Body {
                    st(ui, "Editing header/footer");
                }
                st(ui, if app.is_rtl() { "עברית (ישראל)" } else { "English (United States)" });
                if app.session.doc.settings.track_changes {
                    st(ui, "Track Changes: On");
                }
                if let Some((msg, at)) = &app.status_msg
                    && crate::now_ms() - at < 6000.0
                {
                    ui.label(egui::RichText::new(msg).font(regular(11.5)).color(t.accent_text));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
                    let z = (app.canvas.scale / crate::canvas::PX_PER_PT * 100.0).round();
                    if ui
                        .add(egui::Label::new(egui::RichText::new(format!("{z}%")).font(regular(11.5)).color(t.text_dim)).sense(Sense::click()))
                        .clicked()
                    {
                        let _ = app.run("ui.dialog", json!({"name": "zoom"}));
                    }
                    if small_icon(ui, "plus", "הגדל תצוגה") {
                        let _ = app.run("view.zoomIn", json!({}));
                    }
                    // Zoom slider (10%–500%, 100% in the middle).
                    let mut v = app.canvas.scale / crate::canvas::PX_PER_PT;
                    let slider = egui::Slider::new(&mut v, 0.1..=5.0).logarithmic(true).show_value(false);
                    ui.spacing_mut().slider_width = 110.0;
                    if ui.add(slider).changed() {
                        let _ = app.run("view.zoom", json!({"value": (v * 100.0).round()}));
                    }
                    if small_icon(ui, "minus", "הקטן תצוגה") {
                        let _ = app.run("view.zoomOut", json!({}));
                    }
                    ui.add_space(10.0);
                    let mode = app.session.view.mode;
                    let read = app.session.view.read_mode;
                    for (icon, id, on) in [
                        ("webLayout", "view.webLayout", mode == wordcraft_layout::ViewMode::Web),
                        ("printLayout", "view.printLayout", mode == wordcraft_layout::ViewMode::Print && !read),
                        ("readMode", "view.readMode", read),
                    ] {
                        let (r, resp) = ui.allocate_exact_size(vec2(22.0, 20.0), Sense::click());
                        if on {
                            ui.painter().rect_filled(r, 3.0, t.checked);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, 3.0, t.hover);
                        }
                        icons::paint(ui.painter(), r.shrink(3.0), icon, t.icon, t.accent);
                        if resp.on_hover_text(id.trim_start_matches("view.")).clicked() {
                            let _ = app.run(id, json!({}));
                        }
                    }
                    ui.add_space(8.0);
                    let (r, resp) = ui.allocate_exact_size(vec2(56.0, 20.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 3.0, t.hover);
                    }
                    icons::paint(ui.painter(), Rect::from_min_size(r.min + vec2(2.0, 2.0), vec2(16.0, 16.0)), "focus", t.icon, t.accent);
                    ui.painter().text(pos2(r.min.x + 22.0, r.center().y), Align2::LEFT_CENTER, tl(app, "Focus"), regular(11.5), t.text_dim);
                    if resp.clicked() {
                        let _ = app.run("view.focus", json!({}));
                    }
                });
            });
        });
}

fn small_icon(ui: &mut Ui, icon: &str, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    icons::paint(ui.painter(), r.shrink(3.0), icon, t.icon, t.accent);
    resp.on_hover_text(tip).clicked()
}

impl WordApp {
    pub fn canvas_autosave(&self) -> bool {
        self.autosave
    }
    pub fn toggle_autosave(&mut self) {
        self.autosave = !self.autosave;
    }
    /// Word count, recomputed only when the document changes.
    pub fn cached_word_count(&mut self) -> usize {
        let rev = self.session.rev();
        if self.word_count.0 != rev {
            self.word_count = (rev, self.session.doc.word_count());
        }
        self.word_count.1
    }
}
