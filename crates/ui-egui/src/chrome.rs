//! Title bar (Quick Access Toolbar, title, search, account) and status bar.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::{PartKind, StoryRef};

use crate::theme::{Tokens, TypeRung, fit_text, medium, paint_text, regular, semibold, text_width};
use crate::widgets::{FOCUS_RING_W, focus_ring, focus_ring_with};
use crate::{WordApp, icons};

/// Why the caret's language isn't spell-checked (dictionary unreadable or missing), if it isn't.
pub(crate) fn dictionary_problem(lang: &str, unreadable: &Option<(std::path::PathBuf, String)>, missing: Option<&str>) -> Option<String> {
    match (unreadable, missing) {
        (Some((file, reason)), _) => Some(crate::i18n::fmt(
            tl!("The spelling dictionary for {language} couldn't be read, so it isn't checked. File: {file}. Reason: {reason}"),
            &[("language", lang), ("file", &file.to_string_lossy()), ("reason", reason)],
        )),
        (None, Some(name)) => Some(crate::i18n::fmt(
            tl!(
                "No spelling dictionary for {language} is installed, so it isn't checked. WordCraft looks for the Hunspell dictionary {name} in the WORDCRAFT_DICTIONARIES folder and in the usual system locations."
            ),
            &[("language", lang), ("name", name)],
        )),
        (None, None) => None,
    }
}

/// The Quick Access Toolbar's button for command `id`.
pub(crate) fn qat_button_id(id: &str) -> egui::Id {
    egui::Id::unique(("qat_button", id))
}

fn qat_button(ui: &mut Ui, app: &mut WordApp, icon: &str, tip: &str, id: &str, enabled: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::hover());
    let resp = ui.interact(r, qat_button_id(id), Sense::click());
    if resp.hovered() && enabled {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    focus_ring(ui, r, &resp, &t);
    let c = if enabled { t.icon } else { t.text_disabled };
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), icon, c, if enabled { t.accent } else { c });
    let sc = crate::widgets::shortcut_text(app, id);
    let resp = resp.on_hover_text(if sc.is_empty() { tip.to_string() } else { format!("{tip} ({sc})") });
    if resp.clicked() && enabled {
        let _ = app.run(id, json!({}));
    }
    resp
}

/// Commands the Quick Access Toolbar's menu offers to show or hide with one click (#376).
const QAT_MENU: [&str; 10] =
    ["file.new", "file.open", "file.save", "file.print", "edit.undo", "edit.redo", "edit.repeat", "review.spelling", "view.readMode", "insert.table"];

/// Match the toolbar's enabled state for keyboard activation too.
pub(crate) fn qat_enabled(app: &WordApp, id: &str) -> bool {
    match id {
        "edit.undo" => app.session.can_undo(),
        "edit.redo" => app.session.can_redo(),
        "file.save" => app.has_document(),
        _ => crate::widgets::enabled(app, id),
    }
}

/// The Quick Access Toolbar (#379): the user's commands (Save, Undo, Redo until customized), each
/// with a right-click menu, then the menu button that customizes it (#376).
fn quick_access(ui: &mut Ui, app: &mut WordApp) {
    let ids: Vec<String> = app.session.ribbon.qat().into_iter().map(str::to_string).collect();
    for (index, id) in ids.iter().enumerate() {
        let id = id.as_str();
        let tip = match id {
            "edit.undo" => crate::i18n::prefixed("Undo", app.session.undo_label().unwrap_or("")),
            "edit.redo" => tl!("Redo").to_string(),
            "file.save" => tl!("Save").to_string(),
            _ => app.session.registry.get(id).map(|s| tl!(s.label).to_string()).unwrap_or_default(),
        };
        let enabled = qat_enabled(app, id);
        let resp = qat_button(ui, app, crate::widgets::command_icon(id), &tip, id, enabled);
        if app.ui.keytips == crate::keytips::Phase::Tabs && ui.is_rect_visible(resp.rect) {
            let label = crate::keytips::qat_label(index, ids.len());
            if label.starts_with(&app.ui.keytip_prefix) {
                app.keytip_rects.push((resp.rect, label));
            }
        }
        crate::widgets::command_menu(&resp, app, id);
        if id == "edit.undo" {
            undo_list_button(ui, app, enabled);
        }
    }
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::hover());
    let resp = ui.interact(r, qat_menu_button_id(), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    focus_ring(ui, r, &resp, &t);
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), "more", t.icon, t.accent);
    let resp = resp.on_hover_text(tl!("Customize Quick Access Toolbar"));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(220.0);
        qat_menu(ui, app);
    });
}

/// The Undo button's drop-down arrow (#701), which opens [`undo_list`].
pub(crate) fn undo_list_button_id() -> egui::Id {
    egui::Id::unique("qat_undo_list")
}

/// The status bar's message, the one that says what went wrong (#2402).
pub(crate) fn status_message_id() -> egui::Id {
    egui::Id::unique("status_message")
}

/// Row `i` of the Undo list (0 = the newest step).
fn undo_list_row_id(i: usize) -> egui::Id {
    undo_list_button_id().with(("row", i))
}

/// The footer Cancel row's id (#1903).
pub(crate) fn undo_list_cancel_id() -> egui::Id {
    undo_list_button_id().with("cancel")
}

/// The Undo list's rows, newest first: each step named as the Undo button's tooltip names it.
pub(crate) fn undo_list_rows(app: &WordApp) -> Vec<String> {
    app.session.undo_history().into_iter().map(|l| tl!(l).to_string()).collect()
}

/// The Undo list's footer: how many steps a click undoes, or Cancel while none is highlighted.
pub(crate) fn undo_list_footer(n: usize) -> String {
    match n {
        0 => tl!("Cancel").to_string(),
        1 => tl!("Undo 1 Action").to_string(),
        n => crate::i18n::fmt(tl!("Undo {n} Actions"), &[("n", &n.to_string())]),
    }
}

/// The small arrow beside Undo: opens the list of steps to undo several at once.
fn undo_list_button(ui: &mut Ui, app: &mut WordApp, enabled: bool) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(12.0, 24.0), Sense::hover());
    let resp = ui.interact(r, undo_list_button_id(), if enabled { Sense::click() } else { Sense::hover() });
    if resp.hovered() && enabled {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    focus_ring(ui, r, &resp, &t);
    let c = if enabled { t.icon } else { t.text_disabled };
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(9.0, 9.0)), "dropdown", c, c);
    let resp = resp.on_hover_text(tl!("Undo History"));
    egui::Popup::menu(&resp).show(|ui| undo_list(ui, app));
}

/// The Undo list (#701): recent steps, newest first. Hovering a row highlights it and every newer
/// one; clicking it undoes them all in one go (`edit.undoTo`).
fn undo_list(ui: &mut Ui, app: &mut WordApp) {
    const W: f32 = 210.0;
    const ROW: f32 = 20.0;
    let t = Tokens::get(ui.ctx());
    let rows = undo_list_rows(app);
    ui.set_width(W);
    let (mut hovered, mut clicked) = (None, None);
    egui::ScrollArea::vertical().max_height(ROW * 18.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let rects: Vec<(Rect, egui::Response)> = (0..rows.len())
            .map(|i| {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                let resp = ui.interact(r, undo_list_row_id(i), Sense::click());
                if resp.hovered() {
                    hovered = Some(i);
                }
                if resp.clicked() {
                    clicked = Some(i);
                }
                (r, resp)
            })
            .collect();
        for (i, ((r, resp), label)) in rects.iter().zip(&rows).enumerate() {
            if hovered.is_some_and(|h| i <= h) {
                ui.painter().rect_filled(*r, 0.0, t.selection);
            }
            focus_ring_with(ui, *r, resp, 0, t.focus);
            // Clipped to its row: a long label never runs into the scroll bar.
            ui.painter_at(r.shrink2(vec2(6.0, 0.0))).text(pos2(r.min.x + 6.0, r.center().y), Align2::LEFT_CENTER, label, regular(12.0), t.text);
        }
    });
    ui.separator();
    let n = hovered.map_or(0, |h| h + 1);
    if n == 0 {
        // Cancel is a real row: it closes the popup without undoing anything (#1903).
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
        let resp = ui.interact(r, undo_list_cancel_id(), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.selection);
        }
        focus_ring_with(ui, r, &resp, 0, t.focus);
        ui.painter().text(pos2(r.min.x + 6.0, r.center().y), Align2::LEFT_CENTER, undo_list_footer(0), regular(12.0), t.text);
        if resp.clicked() {
            ui.close();
        }
    } else {
        ui.vertical_centered(|ui| ui.label(egui::RichText::new(undo_list_footer(n)).font(regular(12.0)).color(t.text)));
    }
    if let Some(i) = clicked {
        crate::widgets::run_from_ribbon(app, "edit.undoTo", json!({"count": i + 1}));
        ui.close();
    }
}

/// The Quick Access Toolbar's menu button (…), which opens [`qat_menu`].
pub(crate) fn qat_menu_button_id() -> egui::Id {
    egui::Id::unique("qat_menu_button")
}

/// The Quick Access Toolbar's menu: common commands to show or hide, and More Commands…
pub(crate) fn qat_menu(ui: &mut Ui, app: &mut WordApp) {
    ui.label(egui::RichText::new(tl!("Customize Quick Access Toolbar")).font(semibold(12.0)));
    ui.separator();
    for id in QAT_MENU {
        let Some(label) = app.session.registry.get(id).map(|s| tl!(s.label).to_string()) else { continue };
        let on = app.session.ribbon.qat().contains(&id);
        let mark = egui::RichText::new("✓");
        let mark = if on { mark } else { mark.color(egui::Color32::TRANSPARENT) };
        if ui.add(egui::Button::new((mark, label)).selected(on)).clicked() {
            let key = if on { "qatRemove" } else { "qatAdd" };
            let _ = app.run("tools.customizeRibbon", json!({ key: id }));
        }
    }
    ui.separator();
    if ui.button(tl!("More Commands…")).clicked() {
        crate::dialogs_ribbon::open(app, crate::dialogs_ribbon::Page::QuickAccess);
        ui.close();
    }
    ui.separator();
    // Last, as in Word (#1945); the ribbon's right-click menu brings the toolbar back.
    if ui.button(tl!("Hide Quick Access Toolbar")).clicked() {
        let _ = app.run("ui.quickAccessToolbar", json!({"value": false}));
        ui.close();
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

/// The macOS title bar as laid out around the traffic lights: its height and where content starts
/// after the lights. The app measures the real buttons (they grew in macOS 26); until then, or if it
/// can't, the standard layout above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacTitleBar {
    pub height: f32,
    pub content_left: f32,
}

impl Default for MacTitleBar {
    fn default() -> Self {
        Self { height: MAC_TITLE_BAR, content_left: MAC_CONTENT_LEFT }
    }
}

impl MacTitleBar {
    /// The bar for traffic lights whose right edge is `lights_right` and whose centre is
    /// `lights_centre_y` below the top of the window: the lights centred in it, and content the
    /// same gap after them as they keep from the top. Never smaller than the standard layout.
    pub fn around_lights(lights_right: f32, lights_centre_y: f32, light: f32) -> Self {
        let height = (2.0 * lights_centre_y).max(MAC_TITLE_BAR);
        let gap = (height - light) / 2.0;
        Self { height, content_left: (lights_right + gap).clamp(MAC_CONTENT_LEFT, f32::from(i8::MAX)) }
    }
}

/// Draws the title bar with window dragging, AutoSave, quick access, account, community and search
/// controls, and the centred clickable document title.
pub fn title_bar(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // macOS: shorter, so the system's traffic lights are centred in it; the tallest controls
    // shrink to keep a margin above and below them.
    let mac = app.integrated_titlebar;
    let bar = app.mac_titlebar;
    let left = if mac { bar.content_left as i8 } else { 8 };
    let control_h = if mac { 22.0 } else { 26.0 };
    egui::Panel::top("title_bar")
        .exact_size(if mac { bar.height } else { 38.0 })
        .frame(egui::Frame::NONE.fill(t.title_bar).inner_margin(egui::Margin { left, right: 10, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let full = ui.max_rect();
            // Drag the window by the title bar (integrated title bar on macOS).
            let drag = ui.interact(full, ui.scope_id().with("title_drag"), Sense::click_and_drag());
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
                let color = if block.is_some() { t.text_dim.gamma_multiply(0.6) } else { t.text_dim };
                // The text is part of the switch (#3370): clicking it toggles AutoSave, and chrome
                // text is not document text to select (double-clicking it copied "AutoSave").
                let text = ui.add(
                    egui::Label::new(egui::RichText::new(tl!("AutoSave")).font(regular(11.5)).color(color)).selectable(false).sense(Sense::click()),
                );
                let (r, resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
                let (fill, stroke, knob_fill) = match (&block, on) {
                    (Some(_), _) => (t.input.gamma_multiply(0.5), t.border.gamma_multiply(0.7), t.text_dim.gamma_multiply(0.45)),
                    (None, true) => (t.accent, t.accent, t.on_accent),
                    (None, false) => (t.input, t.border_strong, t.text_dim),
                };
                ui.painter().rect(r, 8.0, fill, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
                // Outside the switch: on, it is filled with the accent.
                focus_ring_with(ui, r.expand(3.0), &resp, 11, t.focus);
                let knob = if on { r.max.x - 8.0 } else { r.min.x + 8.0 };
                ui.painter().circle_filled(pos2(knob, r.center().y), 5.0, knob_fill);
                let tip = block
                    .as_ref()
                    .map_or_else(|| tl!("AutoSave saves every change to the file (needs a saved document)").to_string(), |b| b.reason());
                let text = text.on_hover_text(tip.as_str()).on_hover_cursor(egui::CursorIcon::PointingHand);
                let resp = resp.on_hover_text(tip.as_str()).on_hover_cursor(egui::CursorIcon::PointingHand);
                if text.clicked() || resp.clicked() {
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
                // Hidden (#1945): neither the buttons nor the … menu; the title centres as before.
                if app.ui.quick_access_toolbar {
                    quick_access(ui, app);
                }
                qat_end = ui.min_rect().max.x;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Account / community.
                    let (r, resp) = ui.allocate_exact_size(vec2(control_h, control_h), Sense::click());
                    ui.painter().circle_filled(r.center(), control_h / 2.0 - 1.0, t.accent);
                    if resp.has_focus() {
                        // Around the accent disc, a point clear of it.
                        ui.painter().circle_stroke(r.center(), control_h / 2.0 + FOCUS_RING_W / 2.0, Stroke::new(FOCUS_RING_W, t.focus));
                    }
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Account")));
                    let initials: String = app.session.user_initials().chars().take(2).collect();
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, initials, semibold(10.5), t.on_accent);
                    if resp.on_hover_text(format!("{} — set your name in File › Options", app.session.author)).clicked() {
                        let _ = app.run("ui.backstage", json!({"value": true, "page": "options", "pane": "general"}));
                    }
                    ui.add_space(8.0);
                    if app.ui.show_discord {
                        let (r, resp) = ui.allocate_exact_size(vec2(84.0, 22.0), Sense::click());
                        ui.painter().rect_filled(r, 11.0, if resp.hovered() { t.accent_text } else { t.accent });
                        focus_ring_with(ui, r.expand(3.0), &resp, 14, t.focus);
                        icons::paint(
                            ui.painter(),
                            Rect::from_center_size(pos2(r.min.x + 13.0, r.center().y), vec2(14.0, 14.0)),
                            "discord",
                            t.on_accent,
                            t.on_accent,
                        );
                        ui.painter().text(pos2(r.min.x + 24.0, r.center().y), Align2::LEFT_CENTER, tl!("Discord"), medium(11.5), t.on_accent);
                        if resp.on_hover_text(tl!("Join the ArtCraft community on Discord")).clicked() {
                            let _ = app.run("ui.discord", json!({}));
                        }
                        ui.add_space(8.0);
                    }
                    search_box(app, ui, control_h);
                    right_start = ui.min_rect().min.x;
                });
            });
            // Centred title.
            let title = if !app.has_document() {
                // No document open (#1129): the app's name alone.
                "WordCraft".to_string()
            } else {
                format!(
                    "{} — {}",
                    app.title_stem(),
                    if app.session.dirty {
                        tl!("Edited")
                    } else if app.session.path.is_some() {
                        tl!("Saved")
                    } else {
                        tl!("Not saved")
                    }
                )
            };
            let font = TypeRung::Chrome.semibold();
            let track = TypeRung::Chrome.tracking();
            let cx = full.center().x;
            let available = 2.0 * (cx - qat_end - 10.0).min(right_start - 10.0 - cx);
            let mut shown: String = title.chars().take(240).collect();
            if text_width(ui.painter(), &shown, &font, track, t.text) > available {
                while !shown.is_empty() && text_width(ui.painter(), &format!("{shown}…"), &font, track, t.text) > available {
                    shown.pop();
                }
                shown.push('…');
            }
            let w = text_width(ui.painter(), &shown, &font, track, t.text);
            if available > 24.0 {
                let rect = Rect::from_center_size(pos2(cx, full.center().y), vec2(w + 8.0, control_h));
                let response = ui.interact(rect, ui.scope_id().with("document_name"), Sense::click()).on_hover_text(tl!("Rename Document"));
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &title));
                focus_ring(ui, rect, &response, &t);
                if response.clicked() {
                    let _ = app.run("file.rename", json!({}));
                }

                paint_text(
                    ui.painter(),
                    pos2(cx - w / 2.0, full.center().y - TypeRung::Chrome.line_height() / 2.0),
                    Align2::LEFT_TOP,
                    &shown,
                    &font,
                    track,
                    t.text,
                );
            }
        });
}

/// Search ("Tell me"): a box that opens command search. In the title bar, and in the Commands bar
/// of Full Screen (#1678).
fn search_box(app: &mut WordApp, ui: &mut Ui, control_h: f32) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(260.0, control_h), Sense::click());
    let ph = tl!("Search commands and help");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, ph));
    ui.painter().rect(r, 6.0, if resp.hovered() { t.input } else { t.ribbon }, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    focus_ring_with(ui, r, &resp, 6, t.focus);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 16.0, r.center().y), vec2(16.0, 16.0)), "search", t.text_dim, t.accent);
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
    if app.ui.keytips == crate::keytips::Phase::Tabs {
        app.keytip_rects.push((r, crate::keytips::SEARCH_LETTER.to_string()));
    }
    let tip = match crate::widgets::shortcut_text(app, "tools.search") {
        k if k.is_empty() => tl!("Search").to_string(),
        k => format!("{} ({k})", tl!("Search")),
    };
    if resp.on_hover_text(tip).clicked() {
        let _ = app.run("tools.search", json!({}));
    }
}

/// What fills the screen (#1678): the usual window, Focus (the document on black, a slim bar to
/// drop the ribbon down) or Full Screen (the pages alone, a slim bar with Search). Both of the
/// latter take the whole screen and hide the title bar, ribbon, status bar, panes and rulers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Normal,
    Focus,
    FullScreen,
}

impl Screen {
    pub fn of(v: &wordcraft_engine::ViewState) -> Self {
        if v.focus_mode {
            Screen::Focus
        } else if v.full_screen_view {
            Screen::FullScreen
        } else {
            Screen::Normal
        }
    }

    /// The command that leaves this screen, if it isn't the usual window.
    pub fn exit_command(self) -> Option<&'static str> {
        match self {
            Screen::Normal => None,
            Screen::Focus => Some("view.focus"),
            Screen::FullScreen => Some("view.fullScreenView"),
        }
    }
}

/// Focus's background around the page, in both themes (our own value, #1678).
pub const FOCUS_CANVAS: egui::Color32 = egui::Color32::BLACK;

/// Height of the slim bar Focus and Full Screen View show in place of the chrome (#1678).
pub const SCREEN_BAR_HEIGHT: f32 = 30.0;

/// The slim bar across the top in Focus ("⋯" drops the ribbon down over the page) and Full
/// Screen View (Search, which finds and runs any command).
pub fn screen_bar(app: &mut WordApp, ui: &mut Ui, screen: Screen) {
    let t = Tokens::get(ui.ctx());
    let fill = if screen == Screen::Focus { FOCUS_CANVAS } else { t.title_bar };
    egui::Panel::top("screen_bar").exact_size(SCREEN_BAR_HEIGHT).show_separator_line(false).frame(egui::Frame::NONE.fill(fill)).show(ui, |ui| {
        let full = ui.max_rect();
        if screen == Screen::Focus {
            let r = Rect::from_center_size(full.center(), vec2(40.0, 20.0));
            let resp = ui.allocate_rect(r, Sense::click());
            let shade = if resp.hovered() || app.focus_ribbon { egui::Color32::from_gray(70) } else { egui::Color32::from_gray(40) };
            ui.painter().rect_filled(r, 4.0, shade);
            focus_ring_with(ui, r, &resp, 4, egui::Color32::from_gray(220));
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), "more", egui::Color32::from_gray(220), t.accent);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Show the ribbon")));
            if resp.on_hover_text(tl!("Show the ribbon")).clicked() {
                let _ = app.run("ui.focusRibbon", json!({}));
            }
        } else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_center_size(full.center(), vec2(260.0, 22.0))), |ui| {
                search_box(app, ui, 22.0);
            });
        }
    });
}

/// Focus: the ribbon dropped down from the bar, drawn over the page so the page doesn't move. A
/// click on the page puts it away; Focus stays on.
pub fn focus_ribbon(app: &mut WordApp, ctx: &egui::Context) {
    let width = ctx.content_rect().width();
    egui::Area::new(egui::Id::unique("focus_ribbon")).fixed_pos(pos2(0.0, SCREEN_BAR_HEIGHT)).order(egui::Order::Middle).show(ctx, |ui| {
        ui.set_width(width);
        ui.set_max_width(width);
        crate::ribbon::show(app, ui);
    });
    let click = ctx.input(|i| if i.pointer.any_click() { i.pointer.interact_pos() } else { None });
    if let Some(pos) = click
        && app.canvas.viewport.is_some_and(|v| v.contains(pos))
        && ctx.layer_id_at(pos).is_none_or(|l| l.order == egui::Order::Background)
    {
        let _ = app.run("ui.focusRibbon", json!({"value": false}));
    }
}

/// Draws the status bar's page, word count, zoom and status controls, fitting a status message
/// with a tooltip; without a document the bar is empty.
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
            // No document open (#1129): an empty status bar.
            if !app.has_document() {
                return;
            }
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = vec2(14.0, 0.0);
                let l = app.session.layout();
                let page = if let Some(p) = app.canvas.viewed_page {
                    // The page in view (#3880), as the scrollbar's tip names it.
                    p.min(l.pages.len().saturating_sub(1)) + 1
                } else if app.session.selects_whole_story() {
                    app.session.page_hint.min(l.pages.len().saturating_sub(1)) + 1
                } else {
                    l.caret_on(&app.session.sel.focus, app.session.page_hint).map(|c| c.page + 1).unwrap_or(1)
                };
                let st = |ui: &mut Ui, s: &str| {
                    let font = TypeRung::Caption.regular();
                    let track = TypeRung::Caption.tracking();
                    let w = text_width(ui.painter(), s, &font, track, t.text_dim);
                    let (r, resp) = ui.allocate_exact_size(vec2(w + 1.0, TypeRung::Caption.line_height()), Sense::click());
                    paint_text(ui.painter(), pos2(r.min.x, r.min.y), Align2::LEFT_TOP, s, &font, track, t.text_dim);
                    // Around the text, which fills the control.
                    focus_ring_with(ui, r.expand(2.0), &resp, 3, t.focus);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, s));
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
                    let label = match crate::canvas::part_kind(app, app.session.sel.focus.story) {
                        Some(PartKind::Footnote) => tl!("Editing footnote"),
                        Some(PartKind::Endnote) => tl!("Editing endnote"),
                        _ => tl!("Editing header/footer"),
                    };
                    st(ui, label);
                }
                // The language at the caret; a click sets the proofing language (Review › Language).
                let lang = crate::ribbon::caret_language(app);
                let missing = crate::ribbon::caret_missing_dictionary(app);
                let unreadable = crate::ribbon::caret_unreadable_dictionary(app);
                let shown = if unreadable.is_some() {
                    crate::i18n::fmt(tl!("{language} (dictionary unreadable)"), &[("language", &lang)])
                } else if missing.is_some() {
                    crate::i18n::fmt(tl!("{language} (no dictionary)"), &[("language", &lang)])
                } else {
                    lang.clone()
                };
                let tip = dictionary_problem(&lang, &unreadable, missing).unwrap_or_else(|| tl!("Language").to_string());
                let resp = st(ui, &shown).on_hover_text(tip);
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(200.0);
                    crate::ribbon::language_menu(ui, app);
                });
                if app.session.doc.settings.track_changes {
                    st(ui, tl!("Track Changes: On"));
                }
                if app.session.prefs.overtype && st(ui, tl!("Overtype")).on_hover_text(tl!("Click to switch back to inserting")).clicked() {
                    let _ = app.run("text.overtype", json!({"value": false}));
                }
                // The message is drawn where the row is now; the controls on the right are laid
                // out first, so the message can be shortened to the space they leave (#2402): a
                // long one (the path of a file that isn't there) must not be drawn over the
                // Focus and view buttons.
                let msg_at = ui.cursor().min;
                let mut controls_left = f32::INFINITY;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
                    let reading = app.session.view.read_mode;
                    let z = (if reading { app.session.view.read_zoom } else { app.canvas.scale / crate::canvas::PX_PER_PT } * 100.0).round();
                    let ztxt = format!("{z}%");
                    let zfont = TypeRung::Caption.regular();
                    let ztrack = TypeRung::Caption.tracking();
                    let zw = text_width(ui.painter(), &ztxt, &zfont, ztrack, t.text_dim);
                    // A fixed width for three digits plus a small reserve, so the controls to the
                    // left don't shift as the percentage gains or loses digits (#1975).
                    let zslot = text_width(ui.painter(), "000%", &zfont, ztrack, t.text_dim) + 4.0;
                    let (zr, zresp) = ui.allocate_exact_size(vec2(zw.max(zslot) + 1.0, TypeRung::Caption.line_height()), Sense::click());
                    paint_text(ui.painter(), pos2(zr.max.x - 1.0, zr.min.y), Align2::RIGHT_TOP, &ztxt, &zfont, ztrack, t.text_dim);
                    focus_ring_with(ui, zr.expand(2.0), &zresp, 3, t.focus);
                    if zresp.clicked() && !reading {
                        let _ = app.run("ui.dialog", json!({ "name": "zoom" }));
                    }
                    if small_icon(ui, "plus", tl!("Zoom In")) {
                        if reading {
                            let _ = app.run("view.readTextSize", json!({"value": (z + 10.0).min(150.0)}));
                        } else {
                            let _ = app.run("view.zoomIn", json!({}));
                        }
                    }
                    // Reading zoom changes only reading presentation, never the normal view zoom.
                    let mut v = if reading { app.session.view.read_zoom } else { app.canvas.scale / crate::canvas::PX_PER_PT };
                    let slider = egui::Slider::new(&mut v, if reading { 0.75..=1.5 } else { 0.1..=5.0 }).logarithmic(!reading).show_value(false);
                    ui.spacing_mut().slider_width = 110.0;
                    let response = ui.add(slider);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Slider, ui.is_enabled(), if reading { tl!("Text Size") } else { tl!("Zoom") })
                    });
                    if response.changed() {
                        let command = if reading { "view.readTextSize" } else { "view.zoom" };
                        let _ = app.run(command, json!({"value": (v * 100.0).round()}));
                    }
                    if small_icon(ui, "minus", tl!("Zoom Out")) {
                        if reading {
                            let _ = app.run("view.readTextSize", json!({"value": (z - 10.0).max(75.0)}));
                        } else {
                            let _ = app.run("view.zoomOut", json!({}));
                        }
                    }
                    ui.add_space(10.0);
                    let mode = app.session.view.mode;
                    let read = app.session.view.read_mode;
                    for (icon, id, name, on) in [
                        ("webLayout", "view.webLayout", tl!("Web Layout"), mode == wordcraft_layout::ViewMode::Web),
                        ("printLayout", "view.printLayout", tl!("Print Layout"), mode == wordcraft_layout::ViewMode::Print && !read),
                        ("readMode", "view.readMode", tl!("Read Mode"), read),
                    ] {
                        let (r, resp) = ui.allocate_exact_size(vec2(22.0, 20.0), Sense::click());
                        if on {
                            ui.painter().rect_filled(r, 3.0, t.checked);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, 3.0, t.hover);
                        }
                        focus_ring_with(ui, r, &resp, 3, t.focus);
                        icons::paint(ui.painter(), r.shrink(3.0), icon, t.icon, t.accent);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
                        if resp.on_hover_text(name).clicked() {
                            let _ = app.run(id, json!({}));
                        }
                    }
                    ui.add_space(8.0);
                    let focus = tl!("Focus");
                    let focus_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(focus.to_string(), regular(11.5), t.text_dim).size().x);
                    let (r, resp) = ui.allocate_exact_size(vec2((focus_w + 26.0).max(56.0), 20.0), Sense::click());
                    // The Focus button is the last of the controls, so it is the leftmost one: the
                    // message has to stop before it (#2402).
                    controls_left = r.min.x;
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 3.0, t.hover);
                    }
                    focus_ring_with(ui, r, &resp, 3, t.focus);
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
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, focus));
                    if resp.clicked() {
                        let _ = app.run("view.focus", json!({}));
                    }
                });
                if let Some((msg, at)) = &app.status_msg
                    && crate::now_ms() - at < 6000.0
                {
                    let font = TypeRung::Caption.regular();
                    let track = TypeRung::Caption.tracking();
                    // The room the message has: what is left between here and the controls.
                    let room = controls_left - ui.spacing().item_spacing.x - msg_at.x;
                    let msg_shown = fit_text(ui.painter(), msg, &font, track, t.accent_text, room);
                    if !msg_shown.is_empty() {
                        let w = text_width(ui.painter(), &msg_shown, &font, track, t.accent_text);
                        let rect = Rect::from_min_size(msg_at, vec2(w + 1.0, TypeRung::Caption.line_height()));
                        // The whole message is in the tooltip; the bar only has room for the
                        // beginning of it (#2402).
                        let _ = ui.interact(rect, status_message_id(), Sense::hover()).on_hover_text(msg.clone());
                        paint_text(ui.painter(), rect.min, Align2::LEFT_TOP, &msg_shown, &font, track, t.accent_text);
                        ui.allocate_space(vec2(w + 1.0, TypeRung::Caption.line_height()));
                    }
                }
            });
        });
}

fn small_icon(ui: &mut Ui, icon: &str, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    focus_ring_with(ui, r, &resp, 3, t.focus);
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
mod tests;
