//! Title bar (Quick Access Toolbar, title, search, account) and status bar.

use egui::{Align2, CursorIcon, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};
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

/// Height of the macOS integrated title bar (#225). AppKit draws the traffic lights itself, centred
/// in the standard 28 pt title-bar band at the top of the window; moving them would take AppKit
/// calls (`unsafe`, which the workspace forbids). So the title bar is that band: the lights sit
/// vertically centred in it, with the controls beside them.
pub const MAC_TITLE_BAR: f32 = 28.0;
/// The traffic lights as AppKit lays them out: button diameter, centre-to-centre pitch and the
/// first button's left edge (8–10 pt depending on the macOS version; the larger value keeps the
/// gap after them from shrinking).
const MAC_LIGHT: f32 = 12.0;
const MAC_LIGHT_PITCH: f32 = 20.0;
const MAC_LIGHTS_LEFT: f32 = 10.0;
/// Where title-bar content starts to the right of the traffic lights (the title bar here, the
/// Backstage back button, #222): the end of the three lights plus the same gap the lights keep
/// from the top of the window.
pub const MAC_CONTENT_LEFT: f32 = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT + (MAC_TITLE_BAR - MAC_LIGHT) / 2.0;

pub fn title_bar(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // macOS: shorter, so the system's traffic lights are centred in it; the tallest controls
    // shrink to keep a margin above and below them.
    let mac = app.integrated_titlebar;
    let left = if mac { MAC_CONTENT_LEFT as i8 } else { 8 };
    let control_h = if mac { 22.0 } else { 26.0 };
    egui::Panel::top("title_bar")
        .exact_size(if mac { MAC_TITLE_BAR } else { 38.0 })
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
                // AutoSave switch: greyed out, with the reason as its tooltip, when AutoSave can't
                // cover the document (#196, #176); clicking it then offers Save As where that helps.
                let block = app.autosave_block();
                let on = app.autosaves();
                let label = if block.is_some() { t.text_dim.gamma_multiply(0.6) } else { t.text_dim };
                ui.label(egui::RichText::new(tl!("AutoSave")).font(regular(11.5)).color(label));
                let (r, resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
                let (fill, stroke, knob_fill) = match (&block, on) {
                    (Some(_), _) => (t.input.gamma_multiply(0.5), t.border.gamma_multiply(0.7), t.text_dim.gamma_multiply(0.45)),
                    (None, true) => (t.accent, t.accent, egui::Color32::WHITE),
                    (None, false) => (t.input, t.border_strong, t.text_dim),
                };
                ui.painter().rect(r, 8.0, fill, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
                let knob = if on { r.max.x - 8.0 } else { r.min.x + 8.0 };
                ui.painter().circle_filled(pos2(knob, r.center().y), 5.0, knob_fill);
                let tip = block
                    .as_ref()
                    .map_or_else(|| tl!("AutoSave saves every change to the file (needs a saved document)").to_string(), |b| b.reason());
                if resp.on_hover_text(tip).clicked() {
                    match block {
                        None => {
                            let _ = app.run("file.autosave", json!({}));
                        }
                        // AutoSave comes on once Save As has saved (maybe on a later frame, #94).
                        Some(b) if b.save_as_helps() => {
                            let _ = app.save_as(crate::file_dialogs::AfterSave::AutoSave);
                        }
                        Some(b) => app.status(b.reason()),
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
                    if app.window_controls {
                        ui.add_space(3.0 * CONTROL_W);
                    }
                    // Account / community.
                    let (r, resp) = ui.allocate_exact_size(vec2(control_h, control_h), Sense::click());
                    ui.painter().circle_filled(r.center(), control_h / 2.0 - 1.0, t.accent);
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
                    let (r, resp) = ui.allocate_exact_size(vec2(260.0, control_h), Sense::click());
                    ui.painter().rect(r, 6.0, if resp.hovered() { t.input } else { t.ribbon }, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(pos2(r.min.x + 16.0, r.center().y), vec2(16.0, 16.0)),
                        "search",
                        t.text_dim,
                        t.accent,
                    );
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
                app.title_stem(),
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

/// Width of one window-control button on the title bar.
const CONTROL_W: f32 = 46.0;
/// Height of the title bar, and of the window-control buttons.
const TITLE_H: f32 = 38.0;
/// How far in from a frameless window's edge the pointer resizes it.
const RESIZE_BAND: f32 = 5.0;
/// How far along an edge from a corner the pointer resizes diagonally.
const RESIZE_CORNER: f32 = 14.0;

/// A frameless window (`WordApp::window_controls`): minimize, maximize/restore and close at the
/// top right, resizing from the edges and corners, and a hairline border. Drawn above everything,
/// so the controls stay reachable from Backstage too.
pub fn window_frame(app: &WordApp, ctx: &egui::Context) {
    if !app.window_controls {
        return;
    }
    let (maximized, fullscreen) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if fullscreen {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let controls = Rect::from_min_size(pos2(screen.max.x - 3.0 * CONTROL_W, screen.min.y), vec2(3.0 * CONTROL_W, TITLE_H));
    egui::Area::new(egui::Id::new("wc_window_controls")).order(egui::Order::Foreground).fixed_pos(controls.min).show(ctx, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.horizontal(|ui| {
            for kind in ["minimize", "maximize", "close"] {
                let (r, resp) = ui.allocate_exact_size(vec2(CONTROL_W, TITLE_H), Sense::click());
                let close = kind == "close";
                let hot = resp.hovered() || resp.is_pointer_button_down_on();
                let c = if close && hot {
                    ui.painter().rect_filled(r, 0.0, egui::Color32::from_rgb(0xC4, 0x2B, 0x1C));
                    egui::Color32::WHITE
                } else {
                    if hot {
                        ui.painter().rect_filled(r, 0.0, if resp.is_pointer_button_down_on() { t.pressed } else { t.hover });
                    }
                    t.icon
                };
                let stroke = Stroke::new(1.0, c);
                let m = r.center();
                let p = ui.painter();
                let tip = match kind {
                    "minimize" => {
                        p.line_segment([pos2(m.x - 5.0, m.y), pos2(m.x + 5.0, m.y)], stroke);
                        tl!("Minimize")
                    }
                    "maximize" if maximized => {
                        let front = Rect::from_min_size(pos2(m.x - 5.0, m.y - 3.0), vec2(8.0, 8.0));
                        p.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Inside);
                        p.line_segment([pos2(m.x - 3.0, m.y - 5.0), pos2(m.x + 5.0, m.y - 5.0)], stroke);
                        p.line_segment([pos2(m.x + 5.0, m.y - 5.0), pos2(m.x + 5.0, m.y + 3.0)], stroke);
                        tl!("Restore Down")
                    }
                    "maximize" => {
                        p.rect_stroke(Rect::from_center_size(m, vec2(10.0, 10.0)), 0.0, stroke, egui::StrokeKind::Inside);
                        tl!("Maximize")
                    }
                    _ => {
                        p.line_segment([pos2(m.x - 5.0, m.y - 5.0), pos2(m.x + 5.0, m.y + 5.0)], stroke);
                        p.line_segment([pos2(m.x - 5.0, m.y + 5.0), pos2(m.x + 5.0, m.y - 5.0)], stroke);
                        tl!("Close")
                    }
                };
                if resp.on_hover_text(tip).clicked() {
                    ctx.send_viewport_cmd(match kind {
                        "minimize" => ViewportCommand::Minimized(true),
                        "maximize" => ViewportCommand::Maximized(!maximized),
                        _ => ViewportCommand::Close,
                    });
                }
            }
        });
    });
    if maximized {
        return;
    }
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("wc_window_border"))).rect_stroke(
        screen,
        0.0,
        Stroke::new(1.0, t.border_strong),
        egui::StrokeKind::Inside,
    );
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    if !screen.contains(pos) {
        return;
    }
    let near = |d: f32, band: f32| d < band;
    let (dl, dr, dt, db) = (pos.x - screen.min.x, screen.max.x - pos.x, pos.y - screen.min.y, screen.max.y - pos.y);
    let (w, e, n, s) = (near(dl, RESIZE_BAND), near(dr, RESIZE_BAND), near(dt, RESIZE_BAND), near(db, RESIZE_BAND));
    let (cw, ce, cn, cs) = (near(dl, RESIZE_CORNER), near(dr, RESIZE_CORNER), near(dt, RESIZE_CORNER), near(db, RESIZE_CORNER));
    let hit = if (n && cw) || (w && cn) {
        Some((ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest))
    } else if (n && ce) || (e && cn) {
        Some((ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast))
    } else if (s && cw) || (w && cs) {
        Some((ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest))
    } else if (s && ce) || (e && cs) {
        Some((ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast))
    } else if n {
        Some((ResizeDirection::North, CursorIcon::ResizeNorth))
    } else if s {
        Some((ResizeDirection::South, CursorIcon::ResizeSouth))
    } else if w {
        Some((ResizeDirection::West, CursorIcon::ResizeWest))
    } else if e {
        Some((ResizeDirection::East, CursorIcon::ResizeEast))
    } else {
        None
    };
    if let Some((dir, icon)) = hit {
        ctx.set_cursor_icon(icon);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
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
                // A selected picture/shape/text box isn't a text selection: show the total.
                let wtxt = if app.session.sel.is_collapsed() || crate::objects::selected(app).is_some() {
                    crate::i18n::fmt(tl!("{words} words"), &[("words", &words.to_string())])
                } else {
                    let sw = wordcraft_doc::count_words(&app.session.selected_text());
                    crate::i18n::fmt(tl!("{selected} of {words} words"), &[("selected", &sw.to_string()), ("words", &words.to_string())])
                };
                if st(ui, &wtxt).clicked() {
                    let _ = app.run("ui.dialog", json!({"name": "wordCount"}));
                }
                if app.session.sel.focus.story != StoryRef::Body && !crate::canvas::in_text_box(app) {
                    st(ui, tl!("Editing header/footer"));
                }
                st(ui, tl!("English (United States)"));
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
    /// Word count, recomputed only when the document changes.
    pub fn cached_word_count(&mut self) -> usize {
        // The count option changes the count without a document change.
        let key = self.session.rev().wrapping_mul(2) | u64::from(self.session.prefs.count_notes);
        if self.word_count.0 != key {
            self.word_count = (key, self.session.word_count());
        }
        self.word_count.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_title_bar_centres_and_clears_the_traffic_lights() {
        // The lights are centred in the bar, and content keeps the same gap after them.
        let margin = (MAC_TITLE_BAR - MAC_LIGHT) / 2.0;
        let lights_end = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT;
        assert_eq!(MAC_CONTENT_LEFT - lights_end, margin);
        // `egui::Margin` is in `i8`.
        assert!(MAC_CONTENT_LEFT <= f32::from(i8::MAX));
    }
}
