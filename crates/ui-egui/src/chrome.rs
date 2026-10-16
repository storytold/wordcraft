//! Title bar (Quick Access Toolbar, title, search, account) and status bar.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::StoryRef;

use crate::theme::{Tokens, TypeRung, medium, paint_text, regular, semibold, text_width};
use crate::{WordApp, icons};

fn qat_button(ui: &mut Ui, app: &mut WordApp, icon: &str, tip: &str, id: &str, enabled: bool) -> egui::Response {
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
        #[cfg(test)]
        HEADER_RECTS.with(|h| h.borrow_mut().push(("qat_button", resp.rect)));
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
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), "more", t.icon, t.accent);
    #[cfg(test)]
    HEADER_RECTS.with(|h| h.borrow_mut().push(("qat_more", r)));
    let resp = resp.on_hover_text(tl!("Customize Quick Access Toolbar"));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(220.0);
        qat_menu(ui, app);
    });
}

/// The Undo button's drop-down arrow (#701), which opens [`undo_list`].
pub(crate) fn undo_list_button_id() -> egui::Id {
    egui::Id::new("qat_undo_list")
}

/// Row `i` of the Undo list (0 = the newest step).
fn undo_list_row_id(i: usize) -> egui::Id {
    undo_list_button_id().with(("row", i))
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
    #[cfg(test)]
    HEADER_RECTS.with(|h| h.borrow_mut().push(("qat_undo_list", r)));
    let resp = ui.interact(r, undo_list_button_id(), if enabled { Sense::click() } else { Sense::hover() });
    if resp.hovered() && enabled {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
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
    let footer = undo_list_footer(hovered.map_or(0, |h| h + 1));
    ui.vertical_centered(|ui| ui.label(egui::RichText::new(footer).font(regular(12.0)).color(t.text)));
    if let Some(i) = clicked {
        let _ = app.run("edit.undoTo", json!({"count": i + 1}));
        ui.close();
    }
}

/// The Quick Access Toolbar's menu button (…), which opens [`qat_menu`].
pub(crate) fn qat_menu_button_id() -> egui::Id {
    egui::Id::new("qat_menu_button")
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
/// from the top of the window. Also the floor our own measurement can never go below.
pub const MAC_CONTENT_LEFT: f32 = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT + (MAC_TITLE_BAR - MAC_LIGHT) / 2.0;
/// The horizontal space the native traffic lights take, in egui points, for the current interface
/// zoom: the window's own measurement when it can report one (in native-scale points, so it does not
/// change with our zoom), otherwise the conservative fallback below, plus our own gap. Dividing by
/// the zoom is what makes the *physical* reservation constant at 80 % and at 200 % — never
/// `pixels_per_point`, which describes the display, not the interface size.
/// The macOS title bar geometry the UI draws with, sanitised: a non-finite or out-of-range height
/// keeps the standard 28-point bar, and a nonsense `content_left` keeps the standard 70 points.
pub fn sanitised_mac_title_bar(app: &WordApp) -> MacTitleBar {
    let raw = app.mac_titlebar;
    let height = if raw.height.is_finite() && (MAC_TITLE_BAR..=400.0).contains(&raw.height) { raw.height } else { MAC_TITLE_BAR };
    let content_left =
        if raw.content_left.is_finite() && raw.content_left > 0.0 { raw.content_left.clamp(MAC_CONTENT_LEFT, 400.0) } else { MAC_CONTENT_LEFT };
    MacTitleBar { height, content_left }
}

/// The native strip the macOS header and Backstage must leave free, in egui points.
///
/// Two independent, safe observations are combined conservatively: our own per-frame measurement of
/// the lights' width (`native_lights`, through eframe's `WindowChromeMetrics`) plus the gap we keep
/// after them, and upstream's measured `MacTitleBar::content_left` — where AppKit itself leaves room
/// for content. The larger wins, so neither observation can shrink the strip below what the other
/// saw, and the result is divided by the interface zoom the way the caller has always done.
///
/// The zoom gate below is deliberately left as it is: it accepts 0.8…2.0 and otherwise falls back to
/// 1.0, which is wrong for a fractional zoom such as the 0.5 a 100 % interface has on a 2× display.
/// That known defect is reported, not silently repaired here.
pub fn native_reservation(app: &WordApp, ctx: &egui::Context) -> f32 {
    native_to_ui(native_exclusion(app), valid_zoom(ctx))
}

/// The interface zoom, or 1 when the context reports something unusable.
fn valid_zoom(ctx: &egui::Context) -> f32 {
    let zoom = ctx.zoom_factor();
    if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 }
}

/// A native distance in egui points at `zoom`. Division by a hostile zoom is guarded: the unscaled
/// native distance is returned rather than a non-finite or non-positive one.
fn native_to_ui(native: f32, zoom: f32) -> f32 {
    let ui = native / zoom;
    if ui.is_finite() && ui > 0.0 { ui } else { native }
}

/// The native strip the macOS header and Backstage must leave free, in **native** points: the
/// window's own measurement of the traffic lights plus the gap we keep after them, or the measured
/// title-bar content start, whichever is larger. Kept in native points so the same physical distance
/// stays free at every interface size.
pub fn native_exclusion(app: &WordApp) -> f32 {
    let measured = app.native_lights.filter(|s| s.x.is_finite() && s.x > 0.0).map_or(NATIVE_LIGHTS_W, |s| s.x.clamp(1.0, 400.0)) + NATIVE_GAP;
    measured.max(sanitised_mac_title_bar(app).content_left)
}

/// The vertical room the native buttons need, in **native** points: the measured bar height, or the
/// lights' own height when that is larger. Never below the standard 28-point bar.
pub fn native_band_min(app: &WordApp) -> f32 {
    let bar = sanitised_mac_title_bar(app).height;
    let lights = app.native_lights.filter(|s| s.y.is_finite() && s.y > 0.0).map_or(0.0, |s| s.y.clamp(MAC_TITLE_BAR, 400.0));
    bar.max(lights)
}

/// The macOS title band's height in egui points: the native minimum converted at the current zoom,
/// and never less than the 28 points the controls need — so a reduced interface size enlarges the
/// band instead of letting the tabs slide under the native buttons.
pub fn mac_band_height(app: &WordApp, ctx: &egui::Context) -> f32 {
    MAC_TITLE_BAR.max(native_to_ui(native_band_min(app), valid_zoom(ctx)))
}

#[cfg(test)]
thread_local! {
    /// The real rectangles the macOS title bar laid out: the drag region and the actionable controls.
    /// Tests check these instead of re-deriving the guessed constants.
    pub(crate) static HEADER_RECTS: std::cell::RefCell<Vec<(&'static str, Rect)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The native macOS traffic lights, as the plan models them: the three buttons plus the symmetric
/// margins AppKit keeps around them. Used as the fallback when the window cannot report its own
/// metrics, and as the envelope tests require the header to clear.
pub const NATIVE_LIGHTS_W: f32 = 80.0;
/// Breathing room between the native controls and our own content.
pub const NATIVE_GAP: f32 = 8.0;

/// The title bar's right-hand controls (account, community, search), laid out in the `budget` strip
/// from the end of the Quick Access Toolbar to the window's right edge, right to left.
///
/// Every width below is decided from the strip's real width, so a narrow window (a small window at a
/// large interface size) hides the optional controls instead of drawing them over AutoSave, the
/// toolbar or the macOS traffic lights. Nothing is ever written to preferences: hiding is transient
/// layout. Returns the strip's left edge for the centred title's collision check.
fn header_right_group(ui: &mut Ui, app: &mut WordApp, t: &Tokens, budget: Rect, control_h: f32) -> f32 {
    const GAP: f32 = 12.0;
    const DISCORD_W: f32 = 84.0;
    const SEARCH_W: f32 = 260.0;
    const SEARCH_COMPACT_W: f32 = 32.0;
    let width = budget.width().max(0.0);
    // The account circle needs its own width, a gap and at least the compact search beside it.
    let show_account = width >= control_h + GAP + SEARCH_COMPACT_W;
    let after_account = width - if show_account { control_h + GAP } else { 0.0 };
    // Discord needs a gap and the *full* search beside it.
    let show_discord = app.ui.show_discord && after_account >= DISCORD_W + GAP + SEARCH_W;
    let after_discord = after_account - if show_discord { DISCORD_W + GAP } else { 0.0 };
    let search_w = if after_discord >= SEARCH_W {
        SEARCH_W
    } else if after_discord >= 160.0 {
        after_discord
    } else if after_discord >= SEARCH_COMPACT_W {
        SEARCH_COMPACT_W
    } else {
        0.0
    };
    let mut left = budget.max.x;
    ui.scope_builder(egui::UiBuilder::new().max_rect(budget).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        // Nothing may paint or take input outside the strip.
        ui.shrink_clip_rect(budget);
        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
        if show_account {
            let (r, resp) = ui.allocate_exact_size(vec2(control_h, control_h), Sense::click());
            #[cfg(test)]
            HEADER_RECTS.with(|h| h.borrow_mut().push(("account", r)));
            ui.painter().circle_filled(r.center(), control_h / 2.0 - 1.0, t.accent);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Account")));
            let initials: String = app.session.user_initials().chars().take(2).collect();
            ui.painter().text(r.center(), Align2::CENTER_CENTER, initials, semibold(10.5), t.on_accent);
            if resp.on_hover_text(format!("{} — set your name in File › Options", app.session.author)).clicked() {
                let _ = app.run("ui.backstage", json!({"value": true, "page": "options", "pane": "general"}));
            }
            ui.add_space(8.0);
            left = r.min.x;
        }
        if show_discord {
            let (r, resp) = ui.allocate_exact_size(vec2(DISCORD_W, 22.0), Sense::click());
            #[cfg(test)]
            HEADER_RECTS.with(|h| h.borrow_mut().push(("discord", r)));
            ui.painter().rect_filled(r, 11.0, if resp.hovered() { t.accent_text } else { t.accent });
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
            left = r.min.x;
        }
        if search_w > 0.0 {
            let (r, resp) = ui.allocate_exact_size(vec2(search_w, control_h), Sense::click());
            #[cfg(test)]
            HEADER_RECTS.with(|h| h.borrow_mut().push(("search", r)));
            ui.painter().rect(r, 6.0, if resp.hovered() { t.input } else { t.ribbon }, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let glyph = Rect::from_center_size(pos2(r.min.x + 16.0, r.center().y), vec2(16.0, 16.0));
            icons::paint(ui.painter(), glyph, "search", t.text_dim, t.accent);
            // Only a field wide enough for the words shows them, and they are clipped to the field.
            if search_w >= 160.0 {
                let ph = tl!("Search commands and help");
                let pfont = TypeRung::Control.regular();
                let ptrack = TypeRung::Control.tracking();
                paint_text(
                    &ui.painter_at(r),
                    pos2(r.min.x + 30.0, r.center().y - TypeRung::Control.line_height() / 2.0),
                    Align2::LEFT_TOP,
                    ph,
                    &pfont,
                    ptrack,
                    t.text_dim,
                );
            }
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
            left = r.min.x;
        }
    });
    left
}

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

pub fn title_bar(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // macOS: shorter, so the system's traffic lights are centred in it; the tallest controls
    // shrink to keep a margin above and below them.
    let mac = app.integrated_titlebar;
    // macOS: no frame margin at all — the content is a floating sub-Ui that starts after the native
    // reservation, so nothing is truncated through an `i8` margin and no control or drag region
    // reaches into the traffic lights' strip. The bar's height still follows the measured traffic
    // lights (`MacTitleBar`), sanitised so a hostile measurement cannot collapse or explode it.
    let left = if mac { 0 } else { 8 };
    let reserve = if mac { native_reservation(app, ui.ctx()) } else { 0.0 };
    let control_h = if mac { 22.0 } else { 26.0 };
    egui::Panel::top("title_bar")
        .exact_size(if mac { mac_band_height(app, ui.ctx()) } else { 38.0 })
        .frame(egui::Frame::NONE.fill(t.title_bar).inner_margin(egui::Margin { left, right: 10, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let panel = ui.max_rect();
            #[cfg(test)]
            HEADER_RECTS.with(|h| h.borrow_mut().push(("panel", panel)));
            let content = egui::Rect::from_min_max(egui::pos2((panel.min.x + reserve).min(panel.max.x), panel.min.y), panel.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                // A child's `max_rect` is only a hint: on macOS the clip is what keeps painting and
                // effective hit areas out of the native traffic lights' strip.
                if mac {
                    ui.shrink_clip_rect(content);
                }
                let full = ui.max_rect();
                // Drag the window by the title bar (integrated title bar on macOS).
                let drag = ui.interact(full, ui.id().with("title_drag"), Sense::click_and_drag());
                #[cfg(test)]
                HEADER_RECTS.with(|h| h.borrow_mut().push(("drag", full)));
                if drag.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                let mut qat_end = full.min.x;
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                    // AutoSave switch: greyed out, with the reason as its tooltip, when AutoSave can't
                    // cover the document (#196, #176); clicking it then offers Save As where that helps.
                    let block = app.autosave_block();
                    let on = app.autosaves();
                    let label = if block.is_some() { t.text_dim.gamma_multiply(0.6) } else { t.text_dim };
                    let autosave = ui.label(egui::RichText::new(tl!("AutoSave")).font(regular(11.5)).color(label));
                    #[cfg(test)]
                    HEADER_RECTS.with(|h| h.borrow_mut().push(("autosave", autosave.rect)));
                    #[cfg(not(test))]
                    let _ = &autosave;
                    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
                    #[cfg(test)]
                    HEADER_RECTS.with(|h| h.borrow_mut().push(("autosave_toggle", r)));
                    let (fill, stroke, knob_fill) = match (&block, on) {
                        (Some(_), _) => (t.input.gamma_multiply(0.5), t.border.gamma_multiply(0.7), t.text_dim.gamma_multiply(0.45)),
                        (None, true) => (t.accent, t.accent, t.on_accent),
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
                    quick_access(ui, app);
                    qat_end = ui.min_rect().max.x;
                });
                // The right-hand controls get their own bounded, clipped strip between the end of the
                // Quick Access Toolbar and the window's right edge, so a narrow window cannot make them
                // overflow leftwards over AutoSave, the toolbar or the native buttons.
                let budget = egui::Rect::from_min_max(egui::pos2((qat_end + 12.0).min(full.max.x), full.min.y), full.max);
                let right_start = header_right_group(ui, app, &t, budget, control_h);
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
                    let response = ui.interact(rect, ui.id().with("document_name"), Sense::click()).on_hover_text(tl!("Rename Document"));
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
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
            // No document open (#1129): an empty status bar.
            if !app.has_document() {
                return;
            }
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = vec2(14.0, 0.0);
                let l = app.session.layout();
                let page = if app.session.selects_whole_story() {
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
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s));
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
                // The language at the caret; a click sets the proofing language (Review › Language).
                let lang = crate::ribbon::caret_language(app);
                let missing = crate::ribbon::caret_missing_dictionary(app);
                let shown = if missing.is_some() { crate::i18n::fmt(tl!("{language} (no dictionary)"), &[("language", &lang)]) } else { lang.clone() };
                let tip = match missing {
                    Some(name) => crate::i18n::fmt(
                        tl!("No spelling dictionary for {language} is installed, so it isn't checked. WordCraft looks for the Hunspell dictionary {name} in the WORDCRAFT_DICTIONARIES folder and in the usual system locations."),
                        &[("language", &lang), ("name", name)],
                    ),
                    None => tl!("Language").to_string(),
                };
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
                    let reading = app.session.view.read_mode;
                    let z = (if reading { app.session.view.read_zoom } else { app.canvas.scale / crate::canvas::PX_PER_PT } * 100.0).round();
                    let ztxt = format!("{z}%");
                    let zfont = TypeRung::Caption.regular();
                    let ztrack = TypeRung::Caption.tracking();
                    let zw = text_width(ui.painter(), &ztxt, &zfont, ztrack, t.text_dim);
                    let (zr, zresp) = ui.allocate_exact_size(vec2(zw + 1.0, TypeRung::Caption.line_height()), Sense::click());
                    paint_text(ui.painter(), pos2(zr.min.x, zr.min.y), Align2::LEFT_TOP, &ztxt, &zfont, ztrack, t.text_dim);
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
                    if ui.add(slider).changed() {
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
                        icons::paint(ui.painter(), r.shrink(3.0), icon, t.icon, t.accent);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, name));
                        if resp.on_hover_text(name).clicked() {
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
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, focus));
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

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx
    }

    fn frame(ctx: &egui::Context, n: &mut u32, events: Vec<egui::Event>, a: &mut WordApp) {
        *n += 1;
        let input = egui::RawInput { time: Some(f64::from(*n) / 60.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
        ctx.run_ui(input, |ui| title_bar(a, ui)).drop_without_applying_deltas();
    }

    /// Runs one macOS title-bar frame at the given interface zoom and returns the rectangles it
    /// actually laid out (drag region, AutoSave label and switch, and the other actionable controls).
    fn header_rects_at(zoom: f32) -> Vec<(&'static str, Rect)> {
        let ctx = ctx();
        ctx.set_zoom_factor(zoom);
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.integrated_titlebar = true;
        HEADER_RECTS.with(|h| h.borrow_mut().clear());
        frame(&ctx, &mut 0, vec![], &mut a);
        HEADER_RECTS.with(|h| h.borrow().clone())
    }

    fn header_rects() -> Vec<(&'static str, Rect)> {
        header_rects_at(1.0)
    }

    fn click(ctx: &egui::Context, n: &mut u32, pos: egui::Pos2, a: &mut WordApp) {
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, n, vec![egui::Event::PointerMoved(pos), button(true)], a);
        frame(ctx, n, vec![button(false)], a);
    }

    /// #701: the arrow beside Undo lists the session's undo steps, newest first; clicking the third
    /// row undoes three steps at once.
    #[test]
    fn undo_list_matches_the_session_and_undoes_up_to_the_clicked_row() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        a.run("select.all", json!({})).unwrap();
        a.run("format.bold", json!({})).unwrap();
        a.run("format.italic", json!({})).unwrap();
        a.run("caret.docEnd", json!({})).unwrap();
        a.run("text.insert", json!({"text": " world"})).unwrap();
        let session: Vec<String> = a.session.undo_history().into_iter().map(str::to_string).collect();
        assert_eq!(session, ["Typing", "Italic", "Bold", "Typing"]);
        assert_eq!(undo_list_rows(&a), session);
        assert_eq!(undo_list_footer(0), "Cancel");
        assert_eq!(undo_list_footer(1), "Undo 1 Action");
        assert_eq!(undo_list_footer(3), "Undo 3 Actions");

        let (ctx, mut n) = (ctx(), 0);
        frame(&ctx, &mut n, vec![], &mut a);
        let arrow = ctx.read_response(undo_list_button_id()).unwrap();
        let popup = egui::Popup::default_response_id(&arrow);
        click(&ctx, &mut n, arrow.rect.center(), &mut a);
        assert!(egui::Popup::is_id_open(&ctx, popup), "the arrow opens the Undo list");
        frame(&ctx, &mut n, vec![], &mut a);
        for i in 0..4 {
            assert!(ctx.read_response(undo_list_row_id(i)).is_some(), "row {i} is drawn");
        }
        assert!(ctx.read_response(undo_list_row_id(4)).is_none(), "one row per step");
        let third = ctx.read_response(undo_list_row_id(2)).unwrap();
        click(&ctx, &mut n, third.rect.center(), &mut a);
        assert_eq!(a.session.undo_history(), ["Typing"], "three steps undone in one go");
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello");
        frame(&ctx, &mut n, vec![], &mut a);
        assert!(!egui::Popup::is_id_open(&ctx, popup), "the list closes");
    }

    /// Runs the macOS title bar in a window of `width` points at `percent` interface size with the
    /// given native traffic-light metrics, and returns the zoom plus every rectangle the bar laid out.
    fn header_case(width: f32, percent: u32, lights: Option<egui::Vec2>) -> (f32, Vec<(&'static str, Rect)>) {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.integrated_titlebar = true;
        a.native_lights = lights;
        a.run("ui.interfaceSize", json!({"percent": percent})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(3);
        // One settled frame, recorded on its own: the recorders accumulate per frame.
        HEADER_RECTS.with(|r| r.borrow_mut().clear());
        h.run_steps(1);
        (h.ctx.zoom_factor(), HEADER_RECTS.with(|r| r.borrow().clone()))
    }

    fn overlaps(a: Rect, b: Rect) -> bool {
        let x = a.max.x.min(b.max.x) - a.min.x.max(b.min.x);
        let y = a.max.y.min(b.max.y) - a.min.y.max(b.min.y);
        x > 0.5 && y > 0.5
    }

    /// #1130: at 760 points with the interface at 200 % the window is only ~380 egui points wide, and
    /// the right-hand controls used to overflow leftwards over AutoSave, the Quick Access Toolbar and
    /// the native traffic-light strip. Every visible control's real rectangle must stay inside the
    /// safe content, clear of the toolbar, and the right-hand group must never overlap itself.
    #[test]
    fn mac_header_controls_never_overflow_at_narrow_high_zoom() {
        for width in [760.0f32, 900.0, 1440.0] {
            for percent in [80u32, 90, 100, 110, 125, 150, 175, 200] {
                let (zoom, rects) = header_case(width, percent, Some(egui::vec2(NATIVE_LIGHTS_W, 28.0)));
                let edge = NATIVE_LIGHTS_W + NATIVE_GAP;
                let case = format!("{width:.0}pt/{percent}%");
                for (name, r) in rects.iter().filter(|(n, _)| *n != "panel") {
                    assert!(
                        r.min.x * zoom >= edge - 0.5,
                        "{case}: the {name} region starts at {:.1} egui points = {:.1} physical, inside the native strip",
                        r.min.x,
                        r.min.x * zoom
                    );
                    assert!(r.max.x * zoom <= width + 0.5, "{case}: the {name} region runs past the window: {r:?}");
                }
                // The right-hand controls never overlap AutoSave or any Quick Access Toolbar button.
                let left: Vec<(&str, Rect)> = rects
                    .iter()
                    .filter(|(n, _)| matches!(*n, "autosave" | "autosave_toggle" | "qat_button" | "qat_more" | "qat_undo_list"))
                    .copied()
                    .collect();
                let right: Vec<(&str, Rect)> = rects.iter().filter(|(n, _)| matches!(*n, "account" | "discord" | "search")).copied().collect();
                assert!(!left.is_empty(), "{case}: the toolbar recorded nothing");
                for (ln, lr) in &left {
                    for (rn, rr) in &right {
                        assert!(!overlaps(*lr, *rr), "{case}: the {rn} control overlaps the {ln} region: {rr:?} vs {lr:?}");
                    }
                }
                for (i, (an, ar)) in right.iter().enumerate() {
                    for (bn, br) in right.iter().skip(i + 1) {
                        assert!(!overlaps(*ar, *br), "{case}: the {an} and {bn} controls overlap: {ar:?} vs {br:?}");
                    }
                }
                // The search field is always present (full, shortened or compact) while any room is left.
                let search = right.iter().find(|(n, _)| *n == "search");
                let toolbar_end = left.iter().map(|(_, r)| r.max.x).fold(0.0f32, f32::max);
                if let Some((_, r)) = search {
                    assert!(r.min.x >= toolbar_end - 0.5, "{case}: the search field starts before the toolbar ends: {r:?} vs {toolbar_end:.1}");
                }
            }
        }
    }

    /// The right-hand group adapts to the room it has, in steps: the community button only ever
    /// appears beside a *full* field, a narrower window shortens the field, and the narrowest case
    /// falls back to a 32-point icon-only field — never a stretched or inverted rectangle, and never
    /// a control pushed into the native strip.
    #[test]
    fn mac_header_right_group_shrinks_in_steps() {
        for width in [760.0f32, 900.0, 1440.0] {
            for percent in [80u32, 100, 125, 150, 200] {
                let (zoom, rects) = header_case(width, percent, Some(egui::vec2(NATIVE_LIGHTS_W, 28.0)));
                let has = |n: &str| rects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
                let case = format!("{width:.0}pt/{percent}%");
                if let Some(r) = has("search") {
                    assert!(r.width() >= 32.0 - 0.5 && r.width() <= 260.0 + 0.5, "{case}: the search field is {:.1} wide", r.width());
                    if r.width() > 32.0 + 0.5 && r.width() < 260.0 - 0.5 {
                        assert!(has("discord").is_none(), "{case}: the community button survived beside a shortened field");
                    }
                    if (r.width() - 260.0).abs() < 0.5 {
                        assert!(has("discord").is_some(), "{case}: the full field lost the community button");
                    }
                    if (r.width() - 32.0).abs() < 0.5 {
                        assert!(has("discord").is_none(), "{case}: the icon-only field kept the community button");
                    }
                    assert!(r.min.x * zoom >= NATIVE_LIGHTS_W + NATIVE_GAP - 0.5, "{case}: the search field is inside the native strip");
                }
                for (name, r) in rects.iter().filter(|(n, _)| *n != "panel") {
                    assert!(r.width() >= 0.0 && r.height() >= 0.0, "{case}: the {name} rectangle is inverted: {r:?}");
                    assert!(r.min.x * zoom >= NATIVE_LIGHTS_W + NATIVE_GAP - 0.5, "{case}: the {name} region is inside the native strip");
                }
            }
        }
        // The case the review caught: 760 points at 200 % has room only for the icon-only field.
        let (_, rects) = header_case(760.0, 200, Some(egui::vec2(NATIVE_LIGHTS_W, 28.0)));
        let search = rects.iter().find(|(n, _)| *n == "search").expect("the compact search field").1;
        assert!((search.width() - 32.0).abs() < 0.5, "760pt/200%: the search field is {:.1} wide, not 32", search.width());
        assert!(rects.iter().all(|(n, _)| *n != "discord"), "760pt/200%: the community button is still drawn");
        // And the wide default arrangement is intact.
        let (_, rects) = header_case(1440.0, 100, Some(egui::vec2(NATIVE_LIGHTS_W, 28.0)));
        let search = rects.iter().find(|(n, _)| *n == "search").expect("the full search field").1;
        assert!((search.width() - 260.0).abs() < 0.5, "1440pt/100%: the search field is {:.1} wide, not 260", search.width());
        assert!(rects.iter().any(|(n, _)| *n == "account") && rects.iter().any(|(n, _)| *n == "discord"), "the wide arrangement changed");
    }

    /// Wider, narrower, missing and nonsense native metrics all keep the controls out of the strip,
    /// and a hostile metric never produces a non-finite or inverted layout.
    #[test]
    fn mac_header_survives_any_native_metrics() {
        for lights in [
            None,
            Some(egui::vec2(60.0, 28.0)),
            Some(egui::vec2(NATIVE_LIGHTS_W, 28.0)),
            Some(egui::vec2(120.0, 28.0)),
            Some(egui::vec2(f32::NAN, 28.0)),
            Some(egui::vec2(-5.0, 0.0)),
        ] {
            for (width, percent) in [(760.0f32, 200u32), (900.0, 125), (1440.0, 100)] {
                let (zoom, rects) = header_case(width, percent, lights);
                let measured = lights.filter(|s| s.x.is_finite() && s.x > 0.0).map_or(NATIVE_LIGHTS_W, |s| s.x.clamp(1.0, 400.0));
                let edge = measured + NATIVE_GAP;
                for (name, r) in rects.iter().filter(|(n, _)| *n != "panel") {
                    assert!(r.min.x.is_finite() && r.max.x.is_finite(), "{name} is not finite with {lights:?}");
                    assert!(r.min.x * zoom >= edge - 0.5, "{name} starts inside the {edge:.0}-point strip with {lights:?}");
                }
            }
        }
    }

    /// Real pointer input on the narrow macOS header: the icon-only search opens the command search,
    /// the Quick Access Toolbar's menu and the Undo list still open, and a click inside the native
    /// strip dispatches nothing at all.
    #[test]
    fn mac_header_controls_still_work_when_narrow() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
        a.integrated_titlebar = true;
        a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
        a.run("ui.interfaceSize", json!({"percent": 200})).unwrap();
        // An edit, so the Quick Access Toolbar's Undo button (and its list) are live.
        a.run("caret.docEnd", json!({})).unwrap();
        a.run("text.insert", json!({"text": " world"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(760.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(3);
        HEADER_RECTS.with(|r| r.borrow_mut().clear());
        h.run_steps(1);
        let rects = HEADER_RECTS.with(|r| r.borrow().clone());
        let rect = |n: &str| rects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
        let click = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: egui::Pos2| {
            h.input_mut().events.push(egui::Event::PointerMoved(pos));
            h.input_mut().events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            });
            h.run_steps(1);
            h.input_mut().events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            });
            h.run_steps(2);
        };
        // The compact search field opens the command search.
        let search = rect("search").expect("the compact search field");
        assert!((search.width() - 32.0).abs() < 0.5, "the narrow header is not compact: {search:?}");
        click(&mut h, search.center());
        assert_eq!(
            h.state().dialog.as_ref().map(crate::dialogs::Dialog::name),
            Some("commands"),
            "the compact search did not open the command search"
        );
        h.state_mut().dialog = None;
        h.run_steps(2);
        // The Quick Access Toolbar's menu still opens.
        let more = rect("qat_more").expect("the toolbar's menu button");
        let qat_popup = qat_menu_button_id().with("popup");
        click(&mut h, more.center());
        assert!(egui::Popup::is_id_open(&h.ctx, qat_popup), "the toolbar menu did not open");
        egui::Popup::close_id(&h.ctx, qat_popup);
        h.run_steps(2);
        // The Undo list still opens.
        let undo = rect("qat_undo_list").expect("the Undo list button");
        let undo_popup = undo_list_button_id().with("popup");
        click(&mut h, undo.center());
        assert!(egui::Popup::is_id_open(&h.ctx, undo_popup), "the Undo list did not open");
        egui::Popup::close_id(&h.ctx, undo_popup);
        egui::Popup::close_id(&h.ctx, qat_menu_button_id());
        egui::Popup::close_id(&h.ctx, undo_list_button_id());
        h.run_steps(2);
        // A click inside the native strip reaches nothing of ours.
        let before = (h.state().dialog.is_some(), h.state().session.undo_depth(), h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body));
        let strip = egui::pos2(10.0, more.center().y);
        click(&mut h, strip);
        assert_eq!(
            (h.state().dialog.is_some(), h.state().session.undo_depth(), h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body)),
            before,
            "a click in the native strip changed something"
        );
    }

    /// Every actionable piece of the macOS title bar — the drag region and the AutoSave label and
    /// switch, the first content in the bar — starts to the right of the native traffic lights plus
    /// the gap. The rects are the ones the bar actually laid out. Expected to fail while the header
    /// reserves the old guessed 70 points.
    #[test]
    fn mac_header_controls_clear_the_native_traffic_lights() {
        let rects = header_rects();
        assert!(!rects.is_empty(), "the title bar recorded no rectangles");
        let edge = NATIVE_LIGHTS_W + NATIVE_GAP;
        for (name, r) in rects.iter().filter(|(n, _)| *n != "panel") {
            assert!(r.min.x >= edge - 0.01, "macOS: the {name} region starts at {:.1}, inside the native {edge:.0}-point strip", r.min.x);
        }
    }

    /// The same measurement for every supported interface size: the native reservation is a physical
    /// width, so it is the same at 80 % and at 200 %, while the egui-space reservation shrinks as the
    /// zoom grows.
    #[test]
    fn mac_header_clears_the_lights_at_every_interface_size() {
        for zoom in [0.8f32, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0] {
            let rects = header_rects_at(zoom);
            let edge = (NATIVE_LIGHTS_W + NATIVE_GAP) / zoom;
            for (name, r) in rects.iter().filter(|(n, _)| *n != "panel") {
                // Compared in physical points, with half a point of slack for the pixel grid the
                // content rect is rounded onto.
                assert!(
                    r.min.x * zoom >= NATIVE_LIGHTS_W + NATIVE_GAP - 0.5,
                    "at {zoom:.2}x zoom the {name} region starts at {:.1} egui points = {:.1} physical points, inside the {:.0}-point reservation ({edge:.1} egui points)",
                    r.min.x,
                    r.min.x * zoom,
                    NATIVE_LIGHTS_W + NATIVE_GAP
                );
            }
        }
    }

    /// The reservation is a physical width: the same native 80 points plus the 8-point gap at every
    /// supported interface size, whatever the display's device pixels are. It also filters hostile
    /// values from the window (not a number, negative, zero, absurd) to the safe fallback.
    /// The two native observations are combined: our own per-frame measurement of the lights' width
    /// and upstream's measured `MacTitleBar::content_left`. The larger wins, so neither can shrink the
    /// strip below what the other saw, and nonsense keeps the standard layout.
    /// The native clearance is a *native* distance: at any legitimate interface zoom the same
    /// physical strip must stay free. A 100 % interface on a 2x display has zoom 0.5, so the strip
    /// must be twice as wide in egui points, not the same.
    #[test]
    fn native_clearance_is_normalized_at_every_valid_zoom() {
        let ctx = ctx();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.integrated_titlebar = true;
        a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
        a.mac_titlebar = MacTitleBar::default();
        for zoom in [0.25f32, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0] {
            ctx.set_zoom_factor(zoom);
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            let ui_points = native_reservation(&a, &ctx);
            let native = ui_points * zoom;
            assert!(
                (native - (NATIVE_LIGHTS_W + NATIVE_GAP)).abs() < 0.5,
                "at zoom {zoom} the clearance is {native:.1} native points, not {}",
                NATIVE_LIGHTS_W + NATIVE_GAP
            );
        }
        ctx.set_zoom_factor(1.0);
    }

    /// Pointer contracts at the two scales that matter: a 760-point window on a 1x display at 200 %
    /// really draws at zoom 2, and the same width on a 2x display at 100 % draws at zoom 0.5. In both,
    /// the header's controls must answer a real pointer, the native strip must stay free, and the
    /// Backstage's back button and trailing drag must sit where the geometry says.
    ///
    /// The command-search part runs in its own harness: closing a dialog leaves the accessibility tree
    /// one frame behind, and a fresh harness keeps that out of the rest of the checks.
    #[test]
    fn pointer_contracts_hold_at_true_zoom_two_and_at_retina_one_hundred() {
        for (native, percent, want_zoom) in [(1.0f32, 200u32, 2.0f32), (2.0, 100, 0.5)] {
            let case = format!("{native}x/{percent}%");
            let app = || {
                let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
                a.integrated_titlebar = true;
                a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
                a.mac_titlebar = MacTitleBar::around_lights(78.0, 18.0, 14.0);
                a.run("ui.interfaceSize", json!({"percent": percent})).unwrap();
                a.run("caret.docEnd", json!({})).unwrap();
                a.run("text.insert", json!({"text": " world"})).unwrap();
                a
            };
            let harness = |a: WordApp| {
                let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(760.0, 900.0)).with_pixels_per_point(native).build_ui_state(
                    |ui, app: &mut WordApp| {
                        let ctx = ui.ctx().clone();
                        app.logic(&ctx);
                        app.ui(ui);
                    },
                    a,
                );
                h.run_steps(3);
                h
            };
            let click = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: egui::Pos2| {
                h.input_mut().events.push(egui::Event::PointerMoved(pos));
                h.input_mut().events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                });
                h.run_steps(1);
                h.input_mut().events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                });
                h.run_steps(2);
            };
            // Part 1: the search field opens the command search.
            let mut h = harness(app());
            let zoom = h.ctx.zoom_factor();
            assert!((zoom - want_zoom).abs() < 0.01, "{case}: the app draws at zoom {zoom}, not {want_zoom}");
            HEADER_RECTS.with(|r| r.borrow_mut().clear());
            h.run_steps(1);
            let rects = HEADER_RECTS.with(|r| r.borrow().clone());
            let get = |n: &str| rects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
            let content = get("drag").expect("the header content");
            assert!(content.min.x * zoom >= native_exclusion(h.state()) - 0.5, "{case}: the header starts inside the native strip");
            let search = get("search").expect("a search field");
            assert!(search.min.x >= content.min.x - 0.5 && search.max.x <= content.max.x + 0.5, "{case}: the search left the clipped header");
            click(&mut h, search.center());
            assert_eq!(
                h.state().dialog.as_ref().map(crate::dialogs::Dialog::name),
                Some("commands"),
                "{case}: the search did not open the command search"
            );
            drop(h);
            // Part 2: the toolbar's menu, the Undo list, the native strip and the Backstage.
            let mut h = harness(app());
            HEADER_RECTS.with(|r| r.borrow_mut().clear());
            h.run_steps(1);
            let rects = HEADER_RECTS.with(|r| r.borrow().clone());
            let get = |n: &str| rects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
            let more = get("qat_more").expect("the toolbar menu button");
            let qat_popup = qat_menu_button_id().with("popup");
            click(&mut h, more.center());
            assert!(egui::Popup::is_id_open(&h.ctx, qat_popup), "{case}: the toolbar menu did not open");
            egui::Popup::close_id(&h.ctx, qat_popup);
            h.run_steps(2);
            let undo = get("qat_undo_list").expect("the Undo list button");
            let undo_popup = undo_list_button_id().with("popup");
            click(&mut h, undo.center());
            assert!(egui::Popup::is_id_open(&h.ctx, undo_popup), "{case}: the Undo list did not open");
            egui::Popup::close_id(&h.ctx, undo_popup);
            h.run_steps(2);
            // A click inside the native strip changes nothing of ours.
            let before = (h.state().dialog.is_some(), h.state().session.undo_depth());
            click(&mut h, egui::pos2(4.0, more.center().y));
            assert_eq!((h.state().dialog.is_some(), h.state().session.undo_depth()), before, "{case}: a click in the native strip changed something");
            drop(h);
            // Part 3: the Backstage's geometry and its back button, in a harness of its own (the click
            // closes the Backstage and takes the focused widget with it).
            let mut a = app();
            a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
            let mut h = harness(a);
            crate::backstage::BACKSTAGE_RECTS.with(|b| b.borrow_mut().clear());
            h.run_steps(1);
            let brects = crate::backstage::BACKSTAGE_RECTS.with(|b| b.borrow().clone());
            let bget = |n: &str| brects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
            let band = bget("band").expect("the Backstage band");
            let back = bget("back").expect("the back hitbox");
            let drag = bget("drag").expect("the trailing drag region");
            let reserve = native_reservation(h.state(), &h.ctx);
            assert!(back.min.x * zoom >= native_exclusion(h.state()) - 0.5, "{case}: the back hitbox is inside the native strip");
            assert!(back.max.x <= band.max.x + 0.5 && drag.max.x <= band.max.x + 0.5, "{case}: the band's hit areas left the column");
            assert!(drag.width() >= 22.0 - 0.5, "{case}: too little room to drag");
            assert!(band.width() >= reserve + 44.0 - 0.5, "{case}: the column is narrower than the strip plus the button and drag room");
            click(&mut h, back.center());
            assert!(!h.state().ui.backstage, "{case}: the back button did not leave the Backstage");
        }
    }

    /// The whole supported interface range at every display scale, in one durable suite: AUTO and every
    /// integer size from 50 % to 300 %, on 1x, 1.5x and 2x displays, at three window widths and with
    /// both the standard and the measured macOS 26 bar. 252 settings × 3 densities × 3 widths × 2 bars
    /// = 4536 configurations, driven through the app's own `ui.interfaceSize` command with settled
    /// frames and per-frame recording buffers.
    ///
    /// What is asserted is the geometry that must hold everywhere: a real, positive zoom; the native
    /// horizontal strip free (in native points, so the same physical distance at every size); the band
    /// at least as tall as the native buttons need; every right-hand control inside the header's own
    /// clip; and a search field that is either the full, a shortened or the 32-point icon-only one.
    /// Oversized interfaces are not required to fit every raw toolbar rectangle — that is what the
    /// bounded, clipped strip is for.
    #[test]
    fn scale_matrix_geometry_is_safe_at_every_supported_setting() {
        let settings: Vec<Option<u16>> = std::iter::once(None).chain((50u16..=300).map(Some)).collect();
        assert_eq!(settings.len(), 252, "AUTO plus every integer 50..300");
        let mut checked = 0usize;
        let mut backstage_checked = 0usize;
        let mut compact = 0usize;
        let mut hidden_community = 0usize;
        for native in [1.0f32, 1.5, 2.0] {
            for bar in ["default", "mac26"] {
                for width in [760.0f32, 900.0, 1440.0] {
                    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
                    a.integrated_titlebar = true;
                    a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
                    a.mac_titlebar = if bar == "mac26" { MacTitleBar::around_lights(78.0, 18.0, 14.0) } else { MacTitleBar::default() };
                    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 900.0)).with_pixels_per_point(native).build_ui_state(
                        |ui, app: &mut WordApp| {
                            let ctx = ui.ctx().clone();
                            app.logic(&ctx);
                            app.ui(ui);
                        },
                        a,
                    );
                    h.run_steps(3);
                    // Pass A: the editor's header.
                    for setting in &settings {
                        let case = format!("{native}x/{width:.0}pt/{bar}/{setting:?}");
                        match setting {
                            Some(pct) => h.state_mut().run("ui.interfaceSize", json!({"percent": pct})).unwrap(),
                            None => h.state_mut().run("ui.interfaceSize", json!({"value": "auto"})).unwrap(),
                        };
                        h.run_steps(2);
                        HEADER_RECTS.with(|r| r.borrow_mut().clear());
                        h.run_steps(1);
                        let rects = HEADER_RECTS.with(|r| r.borrow().clone());
                        let get = |n: &str| rects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
                        let zoom = h.ctx.zoom_factor();
                        assert!(zoom.is_finite() && zoom > 0.0, "{case}: zoom {zoom} is not usable");
                        let content = get("drag").unwrap_or_else(|| panic!("{case}: no header content rect"));
                        let exclusion = native_exclusion(h.state());
                        let native_clear = content.min.x * zoom;
                        assert!(
                            native_clear >= exclusion - 0.5,
                            "{case}: the header starts {native_clear:.1} native points in, but the strip needs {exclusion:.1}"
                        );
                        // The rects inside a panel are one point smaller than the panel's exact size
                        // (its own separator stroke), so that point is accounted for in native terms.
                        let panel = get("panel").unwrap_or_else(|| panic!("{case}: no header panel rect"));
                        let band_native = panel.height() * zoom;
                        let band_min = native_band_min(h.state());
                        assert!(
                            band_native >= band_min - zoom - 0.5,
                            "{case}: the band is {band_native:.1} native points tall, but the buttons need {band_min:.1}"
                        );
                        assert!(content.min.x.is_finite() && content.max.x.is_finite(), "{case}: the header content is not finite");
                        // Every right-hand control stays inside the header's own clipped content.
                        for name in ["search", "account", "discord"] {
                            if let Some(r) = get(name) {
                                assert!(
                                    r.min.x >= content.min.x - 0.5 && r.max.x <= content.max.x + 0.5,
                                    "{case}: the {name} control left the clipped header: {r:?} vs {content:?}"
                                );
                                assert!(r.width() >= 0.0 && r.height() >= 0.0, "{case}: the {name} rectangle is inverted");
                            }
                        }
                        if let Some(search) = get("search") {
                            let w = search.width();
                            assert!(
                                (w - 260.0).abs() < 0.5 || (w - 32.0).abs() < 0.5 || (160.0..260.0).contains(&w),
                                "{case}: the search field is {w:.1} wide, which is neither the full, a shortened nor the compact one"
                            );
                            if (w - 32.0).abs() < 0.5 {
                                compact += 1;
                            }
                        }
                        if get("discord").is_none() {
                            hidden_community += 1;
                        }
                        checked += 1;
                    }
                    // Pass B: the Backstage at every setting, in the same settled harness.
                    h.state_mut().run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
                    h.run_steps(3);
                    for setting in &settings {
                        let case = format!("{native}x/{width:.0}pt/{bar}/backstage/{setting:?}");
                        match setting {
                            Some(pct) => h.state_mut().run("ui.interfaceSize", json!({"percent": pct})).unwrap(),
                            None => h.state_mut().run("ui.interfaceSize", json!({"value": "auto"})).unwrap(),
                        };
                        h.run_steps(2);
                        crate::backstage::BACKSTAGE_RECTS.with(|b| b.borrow_mut().clear());
                        h.run_steps(1);
                        let rects = crate::backstage::BACKSTAGE_RECTS.with(|b| b.borrow().clone());
                        let get = |n: &str| rects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
                        let zoom = h.ctx.zoom_factor();
                        assert!(zoom.is_finite() && zoom > 0.0, "{case}: zoom {zoom} is not usable");
                        let band = get("band").unwrap_or_else(|| panic!("{case}: no Backstage band"));
                        let back = get("back").unwrap_or_else(|| panic!("{case}: no back hitbox"));
                        let drag = get("drag").unwrap_or_else(|| panic!("{case}: no trailing drag region"));
                        let reserve = native_reservation(h.state(), &h.ctx);
                        assert!(
                            band.width() >= reserve + 44.0 - 0.5,
                            "{case}: the column is {:.1} points, narrower than the strip plus the button and drag room ({reserve:.1} + 44)",
                            band.width()
                        );
                        assert!(
                            band.width() * zoom >= native_exclusion(h.state()) + 22.0 * zoom - 0.5,
                            "{case}: the column does not clear the native strip in native points"
                        );
                        let back_native = back.min.x * zoom;
                        assert!(
                            back_native >= native_exclusion(h.state()) - 0.5,
                            "{case}: the back hitbox starts {back_native:.1} native points in, inside the strip"
                        );
                        assert!(back.max.x <= band.max.x + 0.5, "{case}: the back hitbox left the column: {back:?} vs {band:?}");
                        assert!(drag.min.x >= back.max.x - 0.5, "{case}: the drag region overlaps the back button");
                        assert!(drag.max.x <= band.max.x + 0.5, "{case}: the drag region left the column");
                        assert!(drag.width() >= 22.0 - 0.5, "{case}: only {:.1} points are left to drag", drag.width());
                        assert!(band.min.x.is_finite() && band.max.x.is_finite(), "{case}: the band is not finite");
                        backstage_checked += 1;
                    }
                    h.state_mut().run("ui.backstage", json!({"value": false})).unwrap();
                    h.run_steps(2);
                }
            }
        }
        assert_eq!(checked, 4536, "the whole editor matrix must run");
        assert_eq!(backstage_checked, 4536, "the whole Backstage matrix must run");
        // Sanity: the suite really visits the narrow and the roomy ends of the range.
        assert!(compact > 0, "no configuration reached the icon-only search");
        assert!(hidden_community > 0, "no configuration hid the community button");
    }

    /// Native density is not interface zoom: on a 1x display AUTO draws at zoom 1, while on a 2x
    /// display the app divides a chosen 100 % by the display's scale, giving zoom 0.5 — and the strip
    /// is then twice as wide in egui points so the *native* distance stays the same.
    #[test]
    fn display_density_and_interface_zoom_are_distinct() {
        let strip = |native: f32, setting: &str| {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            a.integrated_titlebar = true;
            a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
            a.run("ui.interfaceSize", json!({"value": setting})).unwrap();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 900.0)).with_pixels_per_point(native).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                a,
            );
            h.run_steps(3);
            HEADER_RECTS.with(|r| r.borrow_mut().clear());
            h.run_steps(1);
            let content = HEADER_RECTS.with(|r| r.borrow().iter().find(|(n, _)| *n == "drag").map(|(_, r)| *r)).expect("the header content");
            (h.ctx.zoom_factor(), h.ctx.pixels_per_point(), content.min.x)
        };
        let (zoom1, ppp1, points1) = strip(1.0, "auto");
        let (zoom2, ppp2, points2) = strip(2.0, "auto");
        assert!((zoom1 - 1.0).abs() < 0.01 && (zoom2 - 1.0).abs() < 0.01, "AUTO follows the display, it does not scale the UI: {zoom1} / {zoom2}");
        assert!((ppp1 - 1.0).abs() < 0.01 && (ppp2 - 2.0).abs() < 0.01, "the display scale must reach the context: {ppp1} / {ppp2}");
        assert!((points1 - points2).abs() < 0.5, "AUTO on a 2x display lays out the same egui points: {points1} vs {points2}");
        let (zoom100_at_1x, _, points100_at_1x) = strip(1.0, "100");
        let (zoom100_at_2x, _, points100_at_2x) = strip(2.0, "100");
        assert!((zoom100_at_1x - 1.0).abs() < 0.01, "100 % on a 1x display is zoom 1, not {zoom100_at_1x}");
        assert!((zoom100_at_2x - 0.5).abs() < 0.01, "100 % on a 2x display is zoom 0.5, not {zoom100_at_2x}");
        // The strip itself doubles in egui points; the window's own 8-point inset does not, so the
        // measured content start grows by slightly less than exactly twice.
        assert!(
            points100_at_2x >= 1.85 * points100_at_1x,
            "the native strip must be about twice as wide in egui points at zoom 0.5: {points100_at_1x} vs {points100_at_2x}"
        );
        assert!(
            points100_at_1x * zoom100_at_1x >= NATIVE_LIGHTS_W + NATIVE_GAP - 0.5
                && points100_at_2x * zoom100_at_2x >= NATIVE_LIGHTS_W + NATIVE_GAP - 0.5,
            "the native clearance must hold at both densities: {} vs {}",
            points100_at_1x * zoom100_at_1x,
            points100_at_2x * zoom100_at_2x
        );
    }

    /// Hostile zoom values never produce a nonsense reservation. egui itself cannot lay out a
    /// non-finite zoom (its own layout panics), so the guard is exercised at the conversion level,
    /// which is where our code applies it; valid fractional and large zooms are checked through the
    /// real context.
    #[test]
    fn native_conversion_survives_hostile_zoom_and_metrics() {
        for zoom in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
            assert_eq!(native_to_ui(88.0, zoom), 88.0, "zoom {zoom} must fall back to the unscaled native distance");
        }
        let ctx = ctx();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.integrated_titlebar = true;
        a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
        // A tiny but valid zoom scales up instead of clamping, and a large one scales down.
        for (zoom, factor) in [(0.25f32, 4.0f32), (0.5, 2.0), (2.0, 0.5), (4.0, 0.25)] {
            ctx.set_zoom_factor(zoom);
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            let want = (NATIVE_LIGHTS_W + NATIVE_GAP) * factor;
            assert!(
                (native_reservation(&a, &ctx) - want).abs() < 0.5,
                "at zoom {zoom} the strip is {:.1}, not {want:.1}",
                native_reservation(&a, &ctx)
            );
            assert!(mac_band_height(&a, &ctx) >= MAC_TITLE_BAR, "at zoom {zoom} the band fell below its minimum");
        }
        // Hostile native metrics keep the standard layout on both axes.
        a.native_lights = Some(egui::vec2(f32::NAN, f32::NAN));
        a.mac_titlebar = MacTitleBar { height: f32::NAN, content_left: -1.0 };
        assert_eq!(native_exclusion(&a), NATIVE_LIGHTS_W + NATIVE_GAP);
        assert_eq!(native_band_min(&a), MAC_TITLE_BAR);
        a.native_lights = Some(egui::vec2(10_000.0, 10_000.0));
        a.mac_titlebar = MacTitleBar { height: 10_000.0, content_left: 10_000.0 };
        assert_eq!(native_exclusion(&a), 408.0, "absurd measurements are bounded, not honoured");
        assert_eq!(native_band_min(&a), 400.0);
        ctx.set_zoom_factor(1.0);
    }

    /// The Backstage column widens with the native strip so the back button stays clear of the
    /// native buttons and the page content stays clear of the strip, at every interface size.
    #[test]
    fn backstage_nav_keeps_the_back_button_and_content_clear() {
        for (percent, want_nav_at_least) in [(50u32, 200.0f32), (100, 200.0), (200, 200.0)] {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            a.integrated_titlebar = true;
            a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
            a.run("ui.interfaceSize", json!({"percent": percent})).unwrap();
            a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                a,
            );
            h.run_steps(3);
            crate::backstage::BACKSTAGE_RECTS.with(|b| b.borrow_mut().clear());
            h.run_steps(1);
            let rects = crate::backstage::BACKSTAGE_RECTS.with(|b| b.borrow().clone());
            let get = |n: &str| rects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
            let band = get("band").expect("the Backstage band");
            let back = get("back").expect("the back hitbox");
            let drag = get("drag").expect("the trailing drag region");
            let zoom = h.ctx.zoom_factor();
            let reserve = native_reservation(h.state(), &h.ctx);
            let case = format!("{percent}% (zoom {zoom})");
            assert!(band.width() >= want_nav_at_least - 0.5, "{case}: the column is {:.1} wide", band.width());
            assert!(
                band.width() >= reserve + 44.0 - 0.5,
                "{case}: the column {:.1} is narrower than the strip plus the button and drag room",
                band.width()
            );
            assert!(back.min.x >= band.min.x + reserve - 0.5, "{case}: the back hitbox starts inside the native strip: {back:?}");
            assert!(back.max.x <= band.max.x + 0.5, "{case}: the back hitbox left the column");
            assert!(drag.width() >= 22.0 - 0.5, "{case}: only {:.1} points are left to drag", drag.width());
            assert!(drag.min.x >= back.max.x - 0.5, "{case}: the drag region overlaps the back button");
            assert!(drag.max.x <= band.max.x + 0.5, "{case}: the drag region left the column");
        }
    }

    /// The macOS band keeps room for the native buttons vertically too: the band's egui height is the
    /// measured native height converted at the current zoom, never smaller than the 28-point minimum
    /// the controls need.
    #[test]
    fn mac_band_height_keeps_native_room_at_small_zoom() {
        let ctx = ctx();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.integrated_titlebar = true;
        // The macOS 26 measurement: a 36-point native bar.
        a.mac_titlebar = MacTitleBar::around_lights(78.0, 18.0, 14.0);
        a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 36.0));
        for (zoom, want) in [(0.5f32, 72.0f32), (1.0, 36.0), (2.0, 28.0)] {
            ctx.set_zoom_factor(zoom);
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            let h = mac_band_height(&a, &ctx);
            assert!((h - want).abs() < 0.5, "at zoom {zoom} the band is {h:.1} points, not {want:.1}");
        }
        ctx.set_zoom_factor(1.0);
    }

    #[test]
    fn native_reservation_combines_both_native_observations() {
        let ctx = ctx();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.integrated_titlebar = true;
        ctx.set_zoom_factor(1.0);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        // Standard layout on both sides: the 80-point fallback plus our gap.
        a.native_lights = None;
        a.mac_titlebar = MacTitleBar::default();
        assert!((native_reservation(&a, &ctx) - (NATIVE_LIGHTS_W + NATIVE_GAP)).abs() < 0.01, "the default reservation changed");
        // A macOS 26 measurement (content_left 89) wins over the 88-point fallback.
        a.mac_titlebar = MacTitleBar::around_lights(78.0, 18.0, 14.0);
        assert_eq!(a.mac_titlebar.height, 36.0);
        assert!((native_reservation(&a, &ctx) - 89.0).abs() < 0.01, "the measured content_left was not honoured: {}", native_reservation(&a, &ctx));
        // A wider lights measurement wins over the standard 70-point content start.
        a.mac_titlebar = MacTitleBar::default();
        a.native_lights = Some(egui::vec2(120.0, 28.0));
        assert!((native_reservation(&a, &ctx) - 128.0).abs() < 0.01, "our own measurement was not honoured");
        // Nonsense keeps the standard layout on both sides.
        a.native_lights = Some(egui::vec2(f32::NAN, 28.0));
        a.mac_titlebar = MacTitleBar { height: f32::NAN, content_left: -5.0 };
        let bar = sanitised_mac_title_bar(&a);
        assert_eq!(bar, MacTitleBar::default(), "a hostile measurement did not fall back to the standard layout");
        assert!((native_reservation(&a, &ctx) - (NATIVE_LIGHTS_W + NATIVE_GAP)).abs() < 0.01);
        // Absurd values are bounded, not honoured.
        a.mac_titlebar = MacTitleBar { height: 10_000.0, content_left: 10_000.0 };
        assert_eq!(sanitised_mac_title_bar(&a), MacTitleBar { height: MAC_TITLE_BAR, content_left: 400.0 });
    }

    /// The measured macOS 26 bar (36 points) is used by both the editor title bar and the Backstage
    /// band, while the standard 28-point layout stays the default and the non-macOS bar stays 38.
    #[test]
    fn measured_mac_title_bar_height_reaches_both_bars() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.integrated_titlebar = true;
        a.mac_titlebar = MacTitleBar::around_lights(78.0, 18.0, 14.0);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        HEADER_RECTS.with(|r| r.borrow_mut().clear());
        h.run_steps(3);
        h.run_steps(1);
        let drag = HEADER_RECTS.with(|r| r.borrow().iter().find(|(n, _)| *n == "drag").map(|(_, r)| *r)).expect("the title bar's drag region");
        assert!((drag.height() - 36.0).abs() < 1.5, "the measured bar's content is {:.1} points tall, not the measured 36", drag.height());
        // The panel's own rect starts 8 points in (the app's root inset, as upstream's bar also does), so
        // the contract here is that the measured content start is respected as a lower bound.
        assert!(drag.min.x >= 89.0 - 0.5, "the measured content start was not honoured: {drag:?}");
        // The same bar with the standard layout is a shorter one, so the measurement really reaches it.
        let mut standard = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        standard.integrated_titlebar = true;
        let mut hs = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            standard,
        );
        hs.run_steps(3);
        HEADER_RECTS.with(|r| r.borrow_mut().clear());
        hs.run_steps(1);
        let standard_drag =
            HEADER_RECTS.with(|r| r.borrow().iter().find(|(n, _)| *n == "drag").map(|(_, r)| *r)).expect("the standard bar's drag region");
        assert!(
            drag.height() > standard_drag.height() + 5.0,
            "the measured 36-point bar ({:.1}) is not taller than the standard one ({:.1})",
            drag.height(),
            standard_drag.height()
        );
        // The Backstage band follows the same measured height.
        h.state_mut().run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        crate::backstage::BACKSTAGE_RECTS.with(|b| b.borrow_mut().clear());
        h.run_steps(3);
        h.run_steps(1);
        let band =
            crate::backstage::BACKSTAGE_RECTS.with(|b| b.borrow().iter().find(|(n, _)| *n == "band").map(|(_, r)| *r)).expect("the Backstage band");
        assert!((band.height() - 36.0).abs() < 0.5, "the Backstage band is {:.1} points tall, not the measured 36", band.height());
    }

    #[test]
    fn native_reservation_is_zoom_correct_and_filters_hostile_values() {
        let ctx = ctx();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.integrated_titlebar = true;
        for zoom in [0.8f32, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0] {
            ctx.set_zoom_factor(zoom);
            // The zoom is reported from the input state, so it applies on the next frame.
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
            let want = (NATIVE_LIGHTS_W + NATIVE_GAP) / zoom;
            assert!(
                (native_reservation(&a, &ctx) - want).abs() < 0.01,
                "at {zoom:.2}x the reservation is {:.1}, not {want:.1}",
                native_reservation(&a, &ctx)
            );
            // The native 88 points are 88 physical points at every zoom.
            assert!((native_reservation(&a, &ctx) * zoom - 88.0).abs() < 0.01, "the physical reservation changed at {zoom:.2}x");
        }
        ctx.set_zoom_factor(1.0);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        for (case, lights) in [
            ("measured 80", Some(egui::vec2(80.0, 28.0))),
            ("wider 120", Some(egui::vec2(120.0, 28.0))),
            ("narrower 60", Some(egui::vec2(60.0, 28.0))),
            ("missing", None),
            ("not a number", Some(egui::vec2(f32::NAN, 28.0))),
            ("negative", Some(egui::vec2(-10.0, 28.0))),
            ("zero", Some(egui::vec2(0.0, 28.0))),
            ("absurd", Some(egui::vec2(10_000.0, 28.0))),
        ] {
            a.native_lights = lights;
            let r = native_reservation(&a, &ctx);
            assert!(r.is_finite() && r > 0.0 && r < 1000.0, "{case}: reservation {r} is not sane");
            let expected = lights.filter(|s| s.x.is_finite() && s.x > 0.0).map_or(NATIVE_LIGHTS_W, |s| s.x.clamp(1.0, 400.0));
            // The standard macOS content start (70 points) is a floor: a narrower lights measurement
            // can never pull the strip below it, since upstream's `MacTitleBar` keeps its own floor.
            let want = (expected + NATIVE_GAP).max(MAC_CONTENT_LEFT);
            assert!((r - want).abs() < 0.01, "{case}: reservation {r} does not follow {want}");
        }
        // An invalid zoom falls back to 1 rather than dividing by nonsense.
        a.native_lights = None;
        ctx.set_zoom_factor(f32::NAN);
        assert!((native_reservation(&a, &ctx) - (NATIVE_LIGHTS_W + NATIVE_GAP)).abs() < 0.01, "an invalid zoom must fall back to 1");
        ctx.set_zoom_factor(1.0);
    }

    /// The macOS bar keeps its 28-point height and 22-point controls, and the non-macOS bar keeps its
    /// 8-point inset and 38-point height: this change is horizontal spacing only.
    #[test]
    fn title_bar_heights_are_unchanged() {
        let mut heights = Vec::new();
        for mac in [true, false] {
            let ctx = ctx();
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            a.integrated_titlebar = mac;
            a.native_lights = Some(egui::vec2(NATIVE_LIGHTS_W, 28.0));
            HEADER_RECTS.with(|h| h.borrow_mut().clear());
            frame(&ctx, &mut 0, vec![], &mut a);
            let rects = HEADER_RECTS.with(|h| h.borrow().clone());
            let drag = rects.iter().find(|(n, _)| *n == "drag").expect("the drag region").1;
            let want = if mac { MAC_TITLE_BAR } else { 38.0 };
            assert!((drag.height() - want).abs() < 1.5, "mac={mac}: the bar content is {:.1} points tall, not about {want}", drag.height());
            heights.push(drag.height());
            let search = rects.iter().find(|(n, _)| *n == "search").expect("the search box").1;
            assert!((search.height() - if mac { 22.0 } else { 26.0 }).abs() < 0.5, "mac={mac}: the controls are {:.1} points tall", search.height());
            let toggle = rects.iter().find(|(n, _)| *n == "autosave_toggle").expect("the AutoSave switch").1;
            assert!((toggle.height() - 16.0).abs() < 0.5, "mac={mac}: the AutoSave switch changed height: {toggle:?}");
            if !mac {
                assert!((drag.min.x - 8.0).abs() < 0.5, "non-macOS: the bar no longer keeps its 8-point inset: {drag:?}");
            }
        }
        assert!(
            (heights[1] - heights[0] - 10.0).abs() < 1.5,
            "the macOS bar is {:.1} and the other {:.1}: the ten-point difference changed",
            heights[0],
            heights[1]
        );
    }

    #[test]
    fn mac_title_bar_follows_the_measured_lights() {
        // The standard lights give the standard bar.
        let lights_end = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT;
        assert_eq!(MacTitleBar::around_lights(lights_end, MAC_TITLE_BAR / 2.0, MAC_LIGHT), MacTitleBar::default());
        // Larger, lower lights (macOS 26): a taller bar, and content clear of the green button.
        let bar = MacTitleBar::around_lights(78.0, 18.0, 14.0);
        assert_eq!(bar.height, 36.0);
        assert_eq!(bar.content_left, 89.0);
        // Never tighter than the standard layout.
        assert_eq!(MacTitleBar::around_lights(40.0, 10.0, 12.0), MacTitleBar::default());
    }
}
