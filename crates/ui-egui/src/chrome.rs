//! Title bar (Quick Access Toolbar, title, search, account) and status bar.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::StoryRef;

use crate::theme::{Tokens, TypeRung, medium, paint_text, regular, semibold, text_width};
use crate::{WordApp, icons};

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
                // AutoSave toggle (on once the document has been saved in this session).
                let saved_here = app.saved_here();
                ui.label(egui::RichText::new(tl!("AutoSave")).font(regular(11.5)).color(t.text_dim));
                let (r, resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
                let on = app.autosaves();
                ui.painter().rect(
                    r,
                    8.0,
                    if on { t.accent } else { t.input },
                    Stroke::new(1.0, if on { t.accent } else { t.border_strong }),
                    egui::StrokeKind::Inside,
                );
                let knob = if on { r.max.x - 8.0 } else { r.min.x + 8.0 };
                ui.painter().circle_filled(pos2(knob, r.center().y), 5.0, if on { egui::Color32::WHITE } else { t.text_dim });
                if resp.on_hover_text(tl!("AutoSave saves every change to the file (needs a saved document)")).clicked() {
                    if saved_here {
                        app.toggle_autosave();
                    } else if app.save_as_dialog() {
                        app.autosave = true;
                    }
                }
                ui.add_space(8.0);
                qat_button(ui, app, "save", tl!("Save"), "file.save", true);
                let can_undo = app.session.can_undo();
                let can_redo = app.session.can_redo();
                qat_button(ui, app, "undo", &crate::i18n::prefixed("Undo", app.session.undo_label().unwrap_or("")), "edit.undo", can_undo);
                qat_button(ui, app, "redo", tl!("Redo"), "edit.redo", can_redo);
                qat_button(ui, app, "more", tl!("Customize Quick Access Toolbar"), "ui.dialog", true);
                qat_end = ui.min_rect().max.x;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Account / community.
                    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 12.0, t.accent);
                    let initials: String = app.session.author.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, initials, semibold(10.5), egui::Color32::WHITE);
                    resp.on_hover_text(crate::i18n::fmt(tl!("{name} — set your name in File › Options"), &[("name", &app.session.author)]));
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
                        ui.painter().text(
                            pos2(r.min.x + 24.0, r.center().y),
                            Align2::LEFT_CENTER,
                            tl!("Discord"),
                            medium(11.5),
                            egui::Color32::WHITE,
                        );
                        if resp.on_hover_text(tl!("Join the ArtCraft community on Discord")).clicked() {
                            let _ = app.run("ui.discord", json!({}));
                        }
                        ui.add_space(8.0);
                    }
                    // Search ("Tell me").
                    let (r, resp) = ui.allocate_exact_size(vec2(260.0, 26.0), Sense::click());
                    ui.painter().rect(r, 6.0, if resp.hovered() { t.input } else { t.ribbon }, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    icons::paint(ui.painter(), Rect::from_min_size(r.min + vec2(8.0, 5.0), vec2(16.0, 16.0)), "search", t.text_dim, t.accent);
                    let ph = tl!("Search commands and help");
                    let pfont = TypeRung::Control.regular();
                    let ptrack = TypeRung::Control.tracking();
                    paint_text(
                        ui.painter(),
                        pos2(r.min.x + 30.0, r.center().y - TypeRung::Control.line_height() / 2.0),
                        Align2::LEFT_TOP,
                        ph,
                        &pfont,
                        ptrack,
                        t.text_dim,
                    );
                    if resp.clicked() {
                        let _ = app.run("ui.dialog", json!({"name": "commands"}));
                    }
                    right_start = ui.min_rect().min.x;
                });
            });
            // Centred title.
            let title = format!(
                "{} — {}",
                app.display_title(),
                if app.session.dirty {
                    tl!("Edited")
                } else if app.session.path.is_some() {
                    tl!("Saved")
                } else {
                    tl!("Not saved")
                }
            );
            let font = TypeRung::Chrome.semibold();
            let track = TypeRung::Chrome.tracking();
            let w = text_width(ui.painter(), &title, &font, track, t.text);
            let cx = full.center().x;
            if cx - w / 2.0 > qat_end + 10.0 && cx + w / 2.0 < right_start - 10.0 {
                paint_text(
                    ui.painter(),
                    pos2(cx - w / 2.0, full.center().y - TypeRung::Chrome.line_height() / 2.0),
                    Align2::LEFT_TOP,
                    &title,
                    &font,
                    track,
                    t.text,
                );
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
                    let font = TypeRung::Caption.regular();
                    let track = TypeRung::Caption.tracking();
                    let w = text_width(ui.painter(), s, &font, track, t.text_dim);
                    let (r, resp) = ui.allocate_exact_size(vec2(w + 1.0, TypeRung::Caption.line_height()), Sense::click());
                    paint_text(ui.painter(), pos2(r.min.x, r.min.y), Align2::LEFT_TOP, s, &font, track, t.text_dim);
                    resp
                };
                if st(ui, &crate::i18n::fmt(tl!("Page {page} of {pages}"), &[("page", &page.to_string()), ("pages", &l.pages.len().to_string())]))
                    .on_hover_text(tl!("Go To (⌘⌥G)"))
                    .clicked()
                {
                    let _ = app.run("ui.dialog", json!({"name": "goto"}));
                }
                let words = app.cached_word_count();
                let wtxt = if app.session.sel.is_collapsed() {
                    crate::i18n::fmt(tl!("{words} words"), &[("words", &words.to_string())])
                } else {
                    let sw = wordcraft_doc::count_words(&app.session.selected_text());
                    crate::i18n::fmt(tl!("{selected} of {words} words"), &[("selected", &sw.to_string()), ("words", &words.to_string())])
                };
                if st(ui, &wtxt).clicked() {
                    let _ = app.run("ui.dialog", json!({"name": "wordCount"}));
                }
                if app.session.sel.focus.story != StoryRef::Body {
                    st(ui, tl!("Editing header/footer"));
                }
                st(ui, &language_name(app));
                if app.session.doc.settings.track_changes {
                    st(ui, tl!("Track Changes: On"));
                }
                if let Some((msg, at)) = &app.status_msg
                    && crate::now_ms() - at < 6000.0
                {
                    let font = TypeRung::Caption.regular();
                    let track = TypeRung::Caption.tracking();
                    paint_text(ui.painter(), pos2(ui.cursor().min.x, ui.cursor().min.y), Align2::LEFT_TOP, msg, &font, track, t.accent_text);
                    ui.allocate_space(vec2(text_width(ui.painter(), msg, &font, track, t.accent_text) + 1.0, TypeRung::Caption.line_height()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
                    let z = (app.canvas.scale / crate::canvas::PX_PER_PT * 100.0).round();
                    let ztxt = format!("{z}%");
                    let zfont = TypeRung::Caption.regular();
                    let ztrack = TypeRung::Caption.tracking();
                    let zw = text_width(ui.painter(), &ztxt, &zfont, ztrack, t.text_dim);
                    let (zr, zresp) = ui.allocate_exact_size(vec2(zw + 1.0, TypeRung::Caption.line_height()), Sense::click());
                    paint_text(ui.painter(), pos2(zr.min.x, zr.min.y), Align2::LEFT_TOP, &ztxt, &zfont, ztrack, t.text_dim);
                    if zresp.clicked() {
                        let _ = app.run("ui.dialog", json!({ "name": "zoom" }));
                    }
                    if small_icon(ui, "plus", tl!("Zoom In")) {
                        let _ = app.run("view.zoomIn", json!({}));
                    }
                    // Zoom slider (10%–500%, 100% in the middle).
                    let mut v = app.canvas.scale / crate::canvas::PX_PER_PT;
                    let slider = egui::Slider::new(&mut v, 0.1..=5.0).logarithmic(true).show_value(false);
                    ui.spacing_mut().slider_width = 110.0;
                    if ui.add(slider).changed() {
                        let _ = app.run("view.zoom", json!({"value": (v * 100.0).round()}));
                    }
                    if small_icon(ui, "minus", tl!("Zoom Out")) {
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
                    let focus = tl!("Focus");
                    let focus_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(focus.to_string(), regular(11.5), t.text_dim).size().x);
                    let (r, resp) = ui.allocate_exact_size(vec2((focus_w + 26.0).max(56.0), 20.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 3.0, t.hover);
                    }
                    icons::paint(ui.painter(), Rect::from_min_size(r.min + vec2(2.0, 2.0), vec2(16.0, 16.0)), "focus", t.icon, t.accent);
                    let ffont = TypeRung::Caption.regular();
                    let ftrack = TypeRung::Caption.tracking();
                    paint_text(
                        ui.painter(),
                        pos2(r.min.x + 22.0, r.center().y - TypeRung::Caption.line_height() / 2.0),
                        Align2::LEFT_TOP,
                        focus,
                        &ffont,
                        ftrack,
                        t.text_dim,
                    );
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

/// The proofing language at the caret for the status bar (`English (United States)`); the tag
/// itself for languages without a name here.
fn language_name(app: &WordApp) -> String {
    let s = &app.session;
    let lang = s.doc.para_at(&s.sel.focus).and_then(|p| s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(s.sel.focus.off)).lang);
    let name = match lang.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("en-us") => "English (United States)",
        Some("en-gb") => "English (United Kingdom)",
        Some("de-de") => "German (Germany)",
        Some("de-at") => "German (Austria)",
        Some("de-ch") => "German (Switzerland)",
        Some(_) => return lang.unwrap_or_default(),
    };
    tl!(name).to_string()
}
