//! Title bar (Quick Access Toolbar, title, search, account) and status bar.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::{PartKind, StoryRef};

use crate::theme::{Tokens, TypeRung, fit_text, medium, paint_text, regular, semibold, text_width};
use crate::{StatusCount, StatusCounts, WordApp, icons};

/// Why the caret's language isn't spell-checked (dictionary unreadable or missing), if it isn't.
pub(crate) fn dictionary_problem(lang: &str, unreadable: &Option<(std::path::PathBuf, String)>, missing: Option<&str>) -> Option<String> {
    match (unreadable, missing) {
        (Some((file, reason)), _) => Some(crate::i18n::fmt(
            tl!("The spelling dictionary for {language} couldn't be read, so it isn't checked. File: {file}. Reason: {reason}"),
            &[("language", lang), ("file", &crate::i18n::isolate_path(&file.to_string_lossy())), ("reason", reason)],
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
        // The Markdown Source pane's arrows undo its typing.
        "edit.undo" | "edit.redo" if app.session.md_source.is_some() => crate::md_source::can_undo(app, id == "edit.redo"),
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
            // The list is the document's steps, not the Markdown Source pane's typing.
            let enabled = enabled && app.session.md_source.is_none();
            undo_list_button(ui, app, enabled);
        }
    }
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::hover());
    let resp = ui.interact(r, qat_menu_button_id(), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), "more", t.icon, t.accent);
    let resp = resp.on_hover_text(tl!("Customize Quick Access Toolbar"));
    crate::rtl::menu(&resp).show(|ui| {
        ui.set_min_width(220.0);
        crate::rtl::root(ui, |ui| qat_menu(ui, app));
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
    let c = if enabled { t.icon } else { t.text_disabled };
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(9.0, 9.0)), "dropdown", c, c);
    let resp = resp.on_hover_text(tl!("Undo History"));
    crate::rtl::menu(&resp).show(|ui| undo_list(ui, app));
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
        let rects: Vec<Rect> = (0..rows.len())
            .map(|i| {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                let resp = ui.interact(r, undo_list_row_id(i), Sense::click());
                if resp.hovered() {
                    hovered = Some(i);
                }
                if resp.clicked() {
                    clicked = Some(i);
                }
                r
            })
            .collect();
        for (i, (r, label)) in rects.iter().zip(&rows).enumerate() {
            if hovered.is_some_and(|h| i <= h) {
                ui.painter().rect_filled(*r, 0.0, t.selection);
            }
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

/// Where the AutoSave switch's knob sits in its track `r`: on at the right and off at the left,
/// mirrored in a right-to-left interface (on at the left).
fn autosave_knob_x(rtl: bool, on: bool, r: Rect) -> f32 {
    crate::rtl::lead_x(rtl != on, r, 8.0)
}

/// The `ctx.data` key for the part of the title bar that drags this document window.
fn title_drag_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::unique(("title_drag_rect", ctx.viewport_id()))
}

/// Notes `rect` as the part of the title bar that drags the window in this frame: behind a modal
/// dialog its backdrop has the pointer there, and moves the window instead ([`title_drag_rect`]).
pub(crate) fn note_title_drag(ctx: &egui::Context, rect: Rect) {
    let (key, pass) = (title_drag_id(ctx), ctx.cumulative_pass_nr());
    ctx.data_mut(|d| d.insert_temp(key, (rect, pass)));
}

/// The part of the title bar that drags the window, if it was drawn in this frame.
pub(crate) fn title_drag_rect(ctx: &egui::Context) -> Option<Rect> {
    let (key, pass) = (title_drag_id(ctx), ctx.cumulative_pass_nr());
    ctx.data(|d| d.get_temp::<(Rect, u64)>(key)).filter(|(_, p)| *p == pass).map(|(r, _)| r)
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
            // Elsewhere the system's own title bar above this one moves the window.
            if mac {
                note_title_drag(ui.ctx(), full);
            }
            if drag.double_clicked() {
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            // The free stretch between the two clusters, for the centred title. Right to left the
            // clusters swap sides: AutoSave and the Quick Access Toolbar start at the right.
            let rtl = crate::rtl::on();
            let mut qat_end = if rtl { full.max.x } else { full.min.x };
            let mut right_start = if rtl { full.min.x } else { full.max.x };
            crate::rtl::root(ui, |ui| {
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
                        egui::Label::new(egui::RichText::new(tl!("AutoSave")).font(regular(11.5)).color(color))
                            .selectable(false)
                            .sense(Sense::click()),
                    );
                    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
                    let (fill, stroke, knob_fill) = match (&block, on) {
                        (Some(_), _) => (t.input.gamma_multiply(0.5), t.border.gamma_multiply(0.7), t.text_dim.gamma_multiply(0.45)),
                        (None, true) => (t.accent, t.accent, t.on_accent),
                        (None, false) => (t.input, t.border_strong, t.text_dim),
                    };
                    ui.painter().rect(r, 8.0, fill, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
                    let knob = autosave_knob_x(rtl, on, r);
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
                    qat_end = if rtl { ui.min_rect().min.x } else { ui.min_rect().max.x };
                    ui.with_layout(crate::rtl::far_side(ui, egui::Align::Center), |ui| {
                        // Account / community.
                        let (r, resp) = ui.allocate_exact_size(vec2(control_h, control_h), Sense::click());
                        ui.painter().circle_filled(r.center(), control_h / 2.0 - 1.0, t.accent);
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
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(pos2(crate::rtl::lead_x(rtl, r, 13.0), r.center().y), vec2(14.0, 14.0)),
                                "discord",
                                t.on_accent,
                                t.on_accent,
                            );
                            ui.painter().text(
                                pos2(crate::rtl::lead_x(rtl, r, 24.0), r.center().y),
                                crate::rtl::lead_center(rtl),
                                tl!("Discord"),
                                medium(11.5),
                                t.on_accent,
                            );
                            if resp.on_hover_text(tl!("Join the ArtCraft community on Discord")).clicked() {
                                let _ = app.run("ui.discord", json!({}));
                            }
                            ui.add_space(8.0);
                        }
                        search_box(app, ui, control_h);
                        right_start = if rtl { ui.min_rect().max.x } else { ui.min_rect().min.x };
                    });
                })
            });
            let (qat_end, right_start) = if rtl { (right_start, qat_end) } else { (qat_end, right_start) };
            // Centred title.
            let title = if !app.has_document() {
                // No document open (#1129): the app's name alone.
                "WordCraft".to_string()
            } else {
                format!(
                    "{} — {}",
                    crate::i18n::isolate(&app.title_stem()),
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

/// The title bar's Search box while it is typed in (#5398): the query, whether the text of the
/// shortcut that opened it (Alt+Q's `q` on some platforms) is still to be dropped, and where the
/// results showed last frame.
#[derive(Clone, Default)]
struct TitleSearch {
    query: String,
    swallow: bool,
    results: Option<Rect>,
}

/// Width of the Search box, and of its results under it.
const SEARCH_BOX_W: f32 = 260.0;
const SEARCH_RESULTS_W: f32 = 380.0;

/// Search in the title bar's box, as Word's: when this window drew the box in its last frame
/// (the title bar, or Full Screen's Commands bar), Search opens there, typed into in place with
/// its results under it, instead of in the Search Commands dialog. `true` when it did.
pub(crate) fn open_title_search(app: &mut WordApp) -> bool {
    let Some(ctx) = app.ctx.clone() else { return false };
    // Focus hides the box even when the last frame, before Focus came on, drew it.
    if Screen::of(&app.session.view) == Screen::Focus || !search_box_drawn(app, &ctx) {
        return false;
    }
    let id = app.window_id("title_search");
    // Only Alt+Q may send its `q` as text after it; a click sends none. Alt is read from the
    // shortcut's key event too: the box takes the keys from its first frame, that same text
    // included.
    let swallow =
        ctx.input(|i| i.modifiers.alt || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, modifiers, .. } if modifiers.alt)));
    ctx.data_mut(|d| d.insert_temp(id, TitleSearch { swallow, ..Default::default() }));
    ctx.request_repaint();
    true
}

/// What is typed in the title bar's Search, while it is open.
pub(crate) fn title_search_query(app: &WordApp, ctx: &egui::Context) -> Option<String> {
    ctx.data(|d| d.get_temp::<TitleSearch>(app.window_id("title_search"))).map(|s| s.query)
}

/// Whether this window drew the Search box in its last frame.
fn search_box_drawn(app: &WordApp, ctx: &egui::Context) -> bool {
    let shown = ctx.data(|d| d.get_temp::<u64>(app.window_id("title_search_shown")));
    shown.is_some_and(|f| f.saturating_add(1) >= ctx.cumulative_frame_nr())
}

/// Whether the title bar's Search is being typed in (Escape closes it, not Full Screen). Search
/// whose box is no longer drawn (Focus came on) is dropped, so it can't keep Escape.
pub(crate) fn title_search_open(app: &WordApp, ctx: &egui::Context) -> bool {
    if title_search_query(app, ctx).is_none() {
        return false;
    }
    if search_box_drawn(app, ctx) {
        return true;
    }
    ctx.data_mut(|d| d.remove::<TitleSearch>(app.window_id("title_search")));
    false
}

/// The Search box being typed in: a text box in its place and the results under it. `false`
/// once it closes: a command or the find ran, Escape, or a click outside both.
fn typing_search(app: &mut WordApp, ui: &mut Ui, control_h: f32, s: &mut TitleSearch) -> bool {
    if s.swallow {
        ui.ctx().input_mut(|i| i.events.retain(|e| !crate::dialogs::swallow_shortcut_text(&mut s.swallow, e)));
    }
    let ph = tl!("Search commands and help");
    // The box takes the keys from its first frame: focused before it reads this frame's input,
    // so what is typed straight after Alt+Q goes into it. It takes the focus back whenever it lost
    // it, except to one of its results (Tab moves there, so a keyboard user can pick one), and
    // not while a press on a result is under way (#2986).
    let id = app.window_id("title_search_edit");
    let on_result = ui
        .memory(|m| m.focused())
        .filter(|f| *f != id)
        .and_then(|f| ui.ctx().read_response(f))
        .is_some_and(|r| s.results.is_some_and(|x| x.contains_rect(r.rect)));
    if !on_result && !ui.memory(|m| m.has_focus(id)) && !ui.input(|i| i.pointer.any_down()) {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let typing = ui.memory(|m| m.has_focus(id));
    let edit = ui.add_sized(
        vec2(SEARCH_BOX_W, control_h),
        egui::TextEdit::singleline(&mut s.query).id(id).hint_text(ph).align(if crate::rtl::on() {
            Align2::RIGHT_CENTER
        } else {
            Align2::LEFT_CENTER
        }),
    );
    let r = edit.rect;
    // Enter in the box runs the first result, and is used up: a dialog that result opens (Word
    // Count) mustn't take the same Enter as its OK. A result Tab moved to takes Enter or Space
    // itself.
    let (enter, escape, pressed_at) = ui.input_mut(|i| {
        let pressed = i.pointer.any_pressed().then(|| i.pointer.press_origin()).flatten();
        (typing && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter), i.key_pressed(egui::Key::Escape), pressed)
    });
    let inside = |p: egui::Pos2| r.contains(p) || s.results.is_some_and(|x| x.contains(p));
    if escape || pressed_at.is_some_and(|p| !inside(p)) {
        return false;
    }
    let rtl = crate::rtl::on();
    let (pivot, at) = if rtl { (Align2::RIGHT_TOP, r.right_bottom()) } else { (Align2::LEFT_TOP, r.left_bottom()) };
    let mut done = false;
    let shown = egui::Area::new(app.window_id("title_search_results"))
        .order(egui::Order::Foreground)
        .pivot(pivot)
        .fixed_pos(at + vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(SEARCH_RESULTS_W);
                // Rows of a list, as a menu's, not a stack of framed buttons.
                ui.style_mut().visuals.button_frame = false;
                done = crate::dialogs::search_results(app, ui, &s.query, enter, SEARCH_RESULTS_W);
            });
        });
    s.results = Some(shown.response.rect);
    if done {
        // A result run with Enter from the keyboard: the dialog it opens mustn't take that Enter.
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    }
    !done
}

/// Search ("Tell me"): a box that opens command search. In the title bar, and in the Commands bar
/// of Full Screen (#1678). Clicked (or Alt+Q), it takes the typing in place (#5398).
fn search_box(app: &mut WordApp, ui: &mut Ui, control_h: f32) {
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|d| d.insert_temp(app.window_id("title_search_shown"), frame));
    let id = app.window_id("title_search");
    if let Some(mut s) = ui.ctx().data(|d| d.get_temp::<TitleSearch>(id)) {
        let open = typing_search(app, ui, control_h, &mut s);
        ui.ctx().data_mut(|d| {
            if open {
                d.insert_temp(id, s);
            } else {
                d.remove::<TitleSearch>(id);
            }
        });
        return;
    }
    let t = Tokens::get(ui.ctx());
    let rtl = crate::rtl::on();
    let (r, resp) = ui.allocate_exact_size(vec2(SEARCH_BOX_W, control_h), Sense::click());
    let ph = tl!("Search commands and help");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, ph));
    ui.painter().rect(r, 6.0, if resp.hovered() { t.input } else { t.ribbon }, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(crate::rtl::lead_x(rtl, r, 16.0), r.center().y), vec2(16.0, 16.0)),
        "search",
        t.text_dim,
        t.accent,
    );
    let pfont = TypeRung::Control.regular();
    let ptrack = TypeRung::Control.tracking();
    paint_text(ui.painter(), pos2(crate::rtl::lead_x(rtl, r, 30.0), r.center().y), crate::rtl::lead_center(rtl), ph, &pfont, ptrack, t.text_dim);
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
            let rtl = crate::rtl::on();
            crate::rtl::root(ui, |ui| {
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
                    // A status item: its text, after an icon when it has one (one click target).
                    let st_icon = |ui: &mut Ui, icon: Option<&str>, s: &str| {
                        let font = TypeRung::Caption.regular();
                        let track = TypeRung::Caption.tracking();
                        let w = text_width(ui.painter(), s, &font, track, t.text_dim);
                        let lead = if icon.is_some() { 17.0 } else { 0.0 };
                        let (r, resp) = ui.allocate_exact_size(vec2(lead + w + 1.0, TypeRung::Caption.line_height()), Sense::click());
                        if let Some(icon) = icon {
                            let ir = Rect::from_min_size(pos2(crate::rtl::lead_left(rtl, r, 0.0, 13.0), r.center().y - 6.5), vec2(13.0, 13.0));
                            icons::paint(ui.painter(), ir, icon, t.icon, t.accent);
                            paint_text(
                                ui.painter(),
                                pos2(crate::rtl::lead_x(rtl, r, lead), r.min.y),
                                crate::rtl::lead_top(rtl),
                                s,
                                &font,
                                track,
                                t.text_dim,
                            );
                        } else {
                            paint_text(ui.painter(), pos2(r.min.x, r.min.y), Align2::LEFT_TOP, s, &font, track, t.text_dim);
                        }
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, s));
                        resp
                    };
                    let st = |ui: &mut Ui, s: &str| st_icon(ui, None, s);
                    if st(ui, &crate::i18n::fmt(tl!("Page {page} of {pages}"), &[("page", &page.to_string()), ("pages", &l.pages.len().to_string())]))
                        .on_hover_text(tl!("Go To (⌘⌥G)"))
                        .clicked()
                    {
                        let _ = app.run("ui.dialog", json!({"name": "goto"}));
                    }
                    // The live count: words (the default, as Word's status bar counts), characters
                    // with spaces, or characters without spaces — whichever the person chose (#2797).
                    let unit = app.ui.status_count;
                    let counts = app.cached_status_counts();
                    // A selected picture/shape/text box isn't a text selection: show the total.
                    let wtxt = if app.session.sel.is_collapsed() || crate::objects::selected(app).is_some() {
                        status_count_text(unit, counts.in_unit(unit), None)
                    } else {
                        let selected = crate::count_in_unit(&app.session.selected_text(), unit);
                        status_count_text(unit, counts.in_unit(unit), Some(selected))
                    };
                    let counted = st(ui, &wtxt);
                    // Right-click, as Word's status bar offers: choose what the count shows (#2797).
                    counted.context_menu(|ui| {
                        ui.set_min_width(180.0);
                        for c in crate::StatusCount::ALL {
                            if ui.selectable_label(app.ui.status_count == c, tl!(c.label())).clicked() {
                                let _ = app.run("ui.statusCount", json!({"value": c.code()}));
                                ui.close();
                            }
                        }
                    });
                    if counted.clicked() {
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
                    crate::rtl::menu(&resp).show(|ui| {
                        ui.set_min_width(200.0);
                        crate::rtl::root(ui, |ui| crate::ribbon::language_menu(ui, app));
                    });
                    if app.session.doc.settings.track_changes {
                        st(ui, tl!("Track Changes: On"));
                    }
                    // Fonts the document uses that aren't installed: they're drawn in other fonts, so
                    // the text can wrap and paginate unlike where the document was made.
                    let missing = app.cached_missing_fonts(&l).0;
                    let count = missing.iter().filter(|f| !f.metric_compatible).count();
                    if count > 0 {
                        let label = crate::i18n::tn(count as u64, "{n} font not installed", "{n} fonts not installed");
                        let resp = st_icon(ui, Some("fontMissing"), &label).on_hover_text(tl!("Shown in substitute fonts. Click for details."));
                        crate::rtl::menu(&resp).show(|ui| {
                            ui.set_min_width(260.0);
                            ui.label(egui::RichText::new(tl!("Not installed — shown as")).strong());
                            // A hostile file can name thousands of fonts: list the first few.
                            const SHOWN: usize = 20;
                            for f in missing.iter().filter(|f| !f.metric_compatible).take(SHOWN) {
                                ui.label(format!("{} → {}", f.font, f.shown_as.join(", ")));
                            }
                            if count > SHOWN {
                                ui.label(crate::i18n::tn((count - SHOWN) as u64, "…and {n} more", "…and {n} more"));
                            }
                            ui.separator();
                            if ui.button(tl!("Add Font…")).clicked() {
                                let _ = app.run("font.add", json!({}));
                                ui.close();
                            }
                        });
                    }
                    if app.session.prefs.overtype && st(ui, tl!("Overtype")).on_hover_text(tl!("Click to switch back to inserting")).clicked() {
                        let _ = app.run("text.overtype", json!({"value": false}));
                    }
                    // The message is drawn where the row is now; the controls on the right are laid
                    // out first, so the message can be shortened to the space they leave (#2402): a
                    // long one (the path of a file that isn't there) must not be drawn over the
                    // Focus and view buttons.
                    // Right to left the row runs the other way: the message starts at the right edge of
                    // what is left, and the controls (a cluster at the left) end at their right edge.
                    let msg_at = ui.cursor().min;
                    let msg_edge = ui.available_rect_before_wrap().max.x;
                    let mut controls_left = if rtl { f32::NEG_INFINITY } else { f32::INFINITY };
                    ui.with_layout(crate::rtl::far_side(ui, egui::Align::Center), |ui| {
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
                        if zresp.clicked() && !reading {
                            let _ = app.run("ui.dialog", json!({ "name": "zoom" }));
                        }
                        // The slider grows to the right in every language, so the buttons stay at its
                        // ends: plus on the right. This cluster fills from the far end inwards.
                        let zoom_step = |app: &mut WordApp, plus: bool| {
                            if reading {
                                let value = if plus { (z + 10.0).min(150.0) } else { (z - 10.0).max(75.0) };
                                let _ = app.run("view.readTextSize", json!({"value": value}));
                            } else {
                                let _ = app.run(if plus { "view.zoomIn" } else { "view.zoomOut" }, json!({}));
                            }
                        };
                        let (inner, outer) = (("minus", "Zoom Out", false), ("plus", "Zoom In", true));
                        let (near, far) = if rtl { (inner, outer) } else { (outer, inner) };
                        if small_icon(ui, near.0, tl!(near.1)) {
                            zoom_step(app, near.2);
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
                        if small_icon(ui, far.0, tl!(far.1)) {
                            zoom_step(app, far.2);
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
                        controls_left = if rtl { r.max.x } else { r.min.x };
                        if resp.hovered() {
                            ui.painter().rect_filled(r, 3.0, t.hover);
                        }
                        let icon_at = pos2(crate::rtl::lead_left(rtl, r, 2.0, 16.0), r.min.y + 2.0);
                        icons::paint(ui.painter(), Rect::from_min_size(icon_at, vec2(16.0, 16.0)), "focus", t.icon, t.accent);
                        let ffont = TypeRung::Caption.regular();
                        let ftrack = TypeRung::Caption.tracking();
                        paint_text(
                            ui.painter(),
                            pos2(crate::rtl::lead_x(rtl, r, 22.0), r.center().y - TypeRung::Caption.line_height() / 2.0),
                            crate::rtl::lead_top(rtl),
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
                        && crate::now_ms() - at < crate::app::STATUS_MS
                    {
                        let font = TypeRung::Caption.regular();
                        let track = TypeRung::Caption.tracking();
                        // The room the message has: what is left between here and the controls.
                        let room = if rtl {
                            msg_edge - controls_left - ui.spacing().item_spacing.x
                        } else {
                            controls_left - ui.spacing().item_spacing.x - msg_at.x
                        };
                        let msg_shown = fit_text(ui.painter(), msg, &font, track, t.accent_text, room);
                        if !msg_shown.is_empty() {
                            let w = text_width(ui.painter(), &msg_shown, &font, track, t.accent_text);
                            let at = if rtl { pos2(msg_edge - w - 1.0, msg_at.y) } else { msg_at };
                            let rect = Rect::from_min_size(at, vec2(w + 1.0, TypeRung::Caption.line_height()));
                            // The whole message is in the tooltip; the bar only has room for the
                            // beginning of it (#2402).
                            let _ = ui.interact(rect, status_message_id(), Sense::hover()).on_hover_text(msg.clone());
                            paint_text(ui.painter(), rect.min, Align2::LEFT_TOP, &msg_shown, &font, track, t.accent_text);
                            ui.allocate_space(vec2(w + 1.0, TypeRung::Caption.line_height()));
                        }
                    }
                })
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

/// [`WordApp::cached_missing_fonts`]: the list, and the warnings by font key.
pub type MissingFontsCache =
    (std::sync::Arc<Vec<wordcraft_engine::cmd::font::MissingFont>>, std::sync::Arc<std::collections::HashMap<String, String>>);

/// [`WordApp::missing_fonts`]: the layout the list was read from, and the list.
pub type MissingFontsMemo = (std::sync::Weak<wordcraft_layout::DocLayout>, MissingFontsCache);

impl WordApp {
    /// [`wordcraft_engine::cmd::font::missing_fonts`] for `layout`, and the font box's warnings
    /// by [`font_key`](wordcraft_engine::cmd::font::font_key) (the layout-changing ones, with the
    /// faces drawn instead), read again only when the document is laid out again (adding or
    /// removing a font lays it out again).
    pub fn cached_missing_fonts(&mut self, layout: &std::sync::Arc<wordcraft_layout::DocLayout>) -> MissingFontsCache {
        if !self.missing_fonts.0.upgrade().is_some_and(|seen| std::sync::Arc::ptr_eq(&seen, layout)) {
            let found = wordcraft_engine::cmd::font::missing_fonts(&self.session.doc, layout);
            let warnings = found
                .iter()
                .filter(|f| !f.metric_compatible)
                .map(|f| (wordcraft_engine::cmd::font::font_key(&f.font), f.shown_as.join(", ")))
                .collect();
            self.missing_fonts = (std::sync::Arc::downgrade(layout), (std::sync::Arc::new(found), std::sync::Arc::new(warnings)));
        }
        self.missing_fonts.1.clone()
    }

    /// The font box's warning for `font` (a stored name): the faces its text is drawn in
    /// instead, when the status bar lists it as missing and changing the layout. Read from the
    /// same list as the status bar, so the two never disagree. At a collapsed caret, where the
    /// box shows the font typing will use (an empty paragraph's, or one just picked at the end
    /// of a line), the substitute layout will draw it in is named instead.
    pub fn font_box_warning(&mut self, font: &str) -> Option<String> {
        let ahead = self.session.sel.is_collapsed();
        self.font_box_warning_or(font, ahead, |doc| wordcraft_engine::cmd::font::shown_as(doc, font))
    }

    /// The Font dialog's font box's warning for `font`, the font OK applies: from the status
    /// bar's list, else the substitute layout will draw it in, whatever is selected, as no text
    /// may show the font yet.
    pub fn font_dialog_warning(&mut self, font: &str) -> Option<String> {
        self.font_box_warning_or(font, true, |doc| wordcraft_engine::cmd::font::shown_as(doc, font))
    }

    /// The Font dialog's Asian text font box's warning for `font_ea` (the Latin text font being
    /// `font`): the face layout draws Chinese, Japanese and Korean text in when `font_ea` is
    /// missing (the run's font or a fallback face). Not from the status bar's list, which is by
    /// font name: a font also used for Latin text is listed with the faces that text is drawn
    /// in. Whatever is selected: selected Latin text shows no East Asian font, but text typed or
    /// pasted into it would be in it.
    pub fn east_asian_font_box_warning(&mut self, font_ea: &str, font: &str) -> Option<String> {
        let shown = wordcraft_engine::cmd::font::east_asian_shown_as(&self.session.doc, font_ea, font)?;
        wordcraft_engine::cmd::font::warns(font_ea, &shown).then_some(shown)
    }

    /// The font box's warning for `font`: from the status bar's list, else, when the box shows
    /// a font to come (`ahead`), the family `unshown` says its text will be drawn in.
    fn font_box_warning_or(&mut self, font: &str, ahead: bool, unshown: impl FnOnce(&wordcraft_doc::Document) -> Option<String>) -> Option<String> {
        let layout = self.session.layout();
        let key = wordcraft_engine::cmd::font::font_key(font);
        if let Some(w) = self.cached_missing_fonts(&layout).1.get(&key) {
            return Some(w.clone());
        }
        // A font to come (typing at a collapsed caret, or what the Font dialog applies) may be in
        // no shown text yet, so the list can't have it: name the family it will be drawn in. A
        // ribbon box over a selection shows the selected text's font, which the list covers.
        let shown = ahead.then(|| unshown(&self.session.doc)).flatten()?;
        wordcraft_engine::cmd::font::warns(font, &shown).then_some(shown)
    }

    /// The document's counts, recomputed only when the document (or the count option) changes:
    /// words as the status bar and the Word Count dialog count them, and characters with and
    /// without spaces as the dialog does (#2797).
    pub fn cached_status_counts(&mut self) -> StatusCounts {
        // The count option changes the counts without a document change.
        let key = self.session.rev().wrapping_mul(2) | u64::from(self.session.prefs.count_notes);
        if self.status_counts.0 != key {
            let text = self.session.counted_text();
            self.status_counts = (
                key,
                StatusCounts {
                    words: self.session.word_count(),
                    chars: wordcraft_doc::count_chars(&text),
                    chars_no_spaces: wordcraft_doc::count_chars_no_spaces(&text),
                },
            );
        }
        self.status_counts.1
    }
}

/// The status bar's count line (#2797): the whole document's `total`, or "selected of total" when
/// part of it is selected, worded as the Word Count dialog words the same counts.
pub(crate) fn status_count_text(unit: StatusCount, total: usize, selected: Option<usize>) -> String {
    match (unit, selected) {
        (StatusCount::Words, None) => crate::i18n::fmt(tl!("{words} words"), &[("words", &total.to_string())]),
        (StatusCount::Words, Some(n)) => {
            crate::i18n::fmt(tl!("{selected} of {words} words"), &[("selected", &n.to_string()), ("words", &total.to_string())])
        }
        (StatusCount::Characters, None) => crate::i18n::fmt(tl!("{chars} characters"), &[("chars", &total.to_string())]),
        (StatusCount::Characters, Some(n)) => {
            crate::i18n::fmt(tl!("{selected} of {chars} characters"), &[("selected", &n.to_string()), ("chars", &total.to_string())])
        }
        (StatusCount::CharactersNoSpaces, None) => crate::i18n::fmt(tl!("{chars} characters (no spaces)"), &[("chars", &total.to_string())]),
        (StatusCount::CharactersNoSpaces, Some(n)) => {
            crate::i18n::fmt(tl!("{selected} of {chars} characters (no spaces)"), &[("selected", &n.to_string()), ("chars", &total.to_string())])
        }
    }
}

#[cfg(test)]
mod tests;
