//! Ribbon widgets: large and small buttons, split buttons, groups, combo fields, colour grids.

use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use serde_json::Value;

use crate::theme::{Tokens, regular};
use crate::{WordApp, icons};

mod buttons;
mod combo;
mod groups;
mod icon_buttons;
mod tooltip;

pub use buttons::{
    BIG_ICON_Y, BIG_LABEL_Y, ButtonAction, CONTENT_H, DROPDOWN, LABEL_H, LABEL_LINE_H, LABEL_PITCH, MenuButtonOptions, big, big_toggle, enabled,
    label_line, menu_button, menu_enabled, small, split,
};
pub(crate) use buttons::{bg, run_from_ribbon};
pub(crate) use combo::swatch;
pub use combo::{
    ADD_FONT, COMBO_PREVIEW_ROW_H, DrawnState, FontComboOptions, LOAD_SYSTEM_FONTS, color_grid, combo, combo_hint, combo_list, drawn_state,
    font_combo, font_combo_with_recent, font_combo_with_recent_hint, font_matches,
};
#[cfg(test)]
use combo::{Row, highlight_row, section_row, section_rows};
pub use groups::{command_icon, command_menu, customize_menu_items, group, group_titled, row};
pub use icon_buttons::{
    dialog_tabs, icon_button, icon_menu_item, icon_only_button, icon_text_button, icon_text_button_width, icon_text_menu_button, primary_button,
};
#[cfg(test)]
use tooltip::show_hint;
#[cfg(test)]
pub(crate) use tooltip::tooltip_lines;
pub use tooltip::{key_text, shortcut_text};
// #653: a two-line label ends inside the content, above the group title row.
const _: () = assert!(BIG_LABEL_Y + LABEL_PITCH + LABEL_LINE_H <= CONTENT_H);
const _: () = assert!(BIG_ICON_Y + 16.0 <= BIG_LABEL_Y);

/// Shortcut text for a command (`⌘B` on macOS, `Ctrl+B` elsewhere): its first key in effect,
/// custom keys included (Customize Keyboard).
pub fn shortcut_text(app: &WordApp, id: &str) -> String {
    let keys = app.session.keymap.keys_for(&app.session.registry, id);
    keys.first().map(|k| key_text(k)).unwrap_or_default()
}

/// A key as the platform writes it (`⌘⇧K` on macOS, `Ctrl+Shift+K` elsewhere).
pub fn key_text(sc: &str) -> String {
    if sc.is_empty() {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        sc.replace("Mod+", "⌘").replace("Shift+", "⇧").replace("Alt+", "⌥").replace("Ctrl+", "⌃")
    } else {
        sc.replace("Mod+", "Ctrl+")
    }
}

/// The lines of a control's hover hint: the title (`Name (Ctrl+B)`), then the group it lives in
/// and, while the control is off, why. Empty parts are left out; `reason` falls back to a generic
/// line when the command gave none.
pub(crate) fn tooltip_lines(label: &str, shortcut: &str, desc: &str, disabled: Option<&str>) -> (String, Vec<String>) {
    let title = if shortcut.is_empty() || names_a_shortcut(label) { label.to_string() } else { format!("{label} ({shortcut})") };
    let mut notes = Vec::new();
    if !desc.is_empty() {
        notes.push(desc.to_string());
    }
    if let Some(reason) = disabled {
        notes.push(if reason.is_empty() { tl!("Not available right now").to_string() } else { capitalised(tl!(reason)) });
    }
    (title, notes)
}

/// Whether `label` already ends in a parenthesised key ("Paste (⌘V)"), so none is appended again.
fn names_a_shortcut(label: &str) -> bool {
    let Some(inner) = label.strip_suffix(')').and_then(|l| l.rsplit_once(" (")).map(|(_, k)| k) else {
        return false;
    };
    ["⌘", "⌃", "⌥", "⇧", "Ctrl+", "Alt+", "Shift+", "Mod+"].iter().any(|m| inner.contains(m))
}

/// `text` with its first letter in capitals (the engine's reasons are lower case).
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Show the hover hint on `resp`. A control reached with the keyboard shows it too, after the
/// usual delay (egui itself only hints for the pointer), unless the pointer is busy elsewhere or a
/// menu is open, whose hint would otherwise be drawn over the menu.
fn show_hint(resp: &Response, title: &str, notes: &[String]) {
    let add = |ui: &mut Ui| {
        ui.set_max_width(260.0);
        ui.label(egui::RichText::new(title).font(crate::theme::semibold(12.0)));
        for n in notes {
            ui.label(egui::RichText::new(n).small().weak());
        }
    };
    let ctx = &resp.ctx;
    let since_key = id_since_focus(resp);
    // The pass on which the pointer was last over any hinted control (see the focus hint below).
    let hover_key = egui::Id::new("hint_pointer_over_a_control");
    let pass = ctx.cumulative_pass_nr();
    // A disabled response never reports hover, so test the pointer against its rectangle, as
    // `Tooltip::for_disabled` does.
    let over = resp.hovered() || (!resp.enabled() && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| resp.rect.contains(p)));
    if over {
        ctx.data_mut(|d| d.insert_temp(hover_key, pass));
    }
    if resp.hovered() || !resp.has_focus() {
        // A response from a disabled `Ui` only shows hints through `for_disabled`.
        if resp.enabled() {
            egui::Tooltip::for_enabled(resp).show(add);
        } else {
            egui::Tooltip::for_disabled(resp).show(add);
        }
    }
    if let Some(waited) = since_key {
        let delay = ctx.global_style().interaction.tooltip_delay;
        let pointer_idle = !ctx.input(|i| i.pointer.has_pointer()) || ctx.input(|i| i.pointer.time_since_last_movement()) > 2.0;
        // Another control under the pointer (seen on this or the previous pass) owns the hint.
        let pointer_over_other = ctx.data(|d| d.get_temp::<u64>(hover_key)).is_some_and(|p| p + 1 >= pass);
        if waited < delay {
            ctx.request_repaint_after_secs(delay - waited);
        } else if !resp.hovered() && pointer_idle && !pointer_over_other && !egui::Popup::is_any_open(ctx) {
            egui::Tooltip::for_widget(resp).show(add);
        }
    }
}

/// Seconds `resp` has had the keyboard focus (None when it hasn't), tracked in egui memory.
fn id_since_focus(resp: &Response) -> Option<f32> {
    let key = resp.id.with("hint_focus_since");
    let now = resp.ctx.input(|i| i.time);
    if !resp.has_focus() {
        resp.ctx.data_mut(|d| d.remove::<f64>(key));
        return None;
    }
    let since = resp.ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
    Some((now - since) as f32)
}

/// `label` is already in the interface language.
fn tooltip(app: &WordApp, resp: Response, label: &str, id: &str) -> Response {
    let spec = app.session.registry.get(id);
    let desc = spec.map(|s| crate::i18n::location(s.location)).unwrap_or_default();
    // `enabled` also covers what the registry doesn't know (no document open, `ui.*` commands).
    let reason = if enabled(app, id) {
        None
    } else if !app.has_document() && !crate::works_without_document(id) {
        Some(tl!("Open a document first"))
    } else {
        Some(app.session.why_disabled(id).unwrap_or(""))
    };
    let (title, notes) = tooltip_lines(label, &shortcut_text(app, id), &desc, reason);
    show_hint(&resp, &title, &notes);
    resp
}

/// Record a keytip badge over a control's rect when keytips are showing commands. `ui` reaches
/// the phase and badge list through egui memory, so widgets without `&mut WordApp` can record too.
fn keytip_badge(ui: &Ui, rect: Rect, id: &str, name: &str) {
    if ui.data(|d| d.get_temp::<crate::keytips::Phase>(crate::keytips::phase_id())) != Some(crate::keytips::Phase::Commands) {
        return;
    }
    let tab = ui.data(|d| d.get_temp::<String>(crate::keytips::tab_id())).unwrap_or_default();
    let controls = crate::keytips::controls_public(&tab);
    // Match by command id first; fall back to tooltip prefix, since widget tooltips carry
    // extra text ("Paste (⌘V)") the canonical control name does not.
    let Some(i) = controls
        .iter()
        .position(|c| c.id == id && id != "ui.dialog")
        .or_else(|| controls.iter().position(|c| c.tip == name))
        .or_else(|| controls.iter().position(|c| name.starts_with(c.tip)))
    else {
        return;
    };
    let letter = ui.data(|d| d.get_temp::<Vec<String>>(crate::keytips::letters_id().with(&tab))).and_then(|l| l.get(i).cloned());
    if let Some(l) = letter {
        ui.data_mut(|d| {
            let mut v: Vec<(Rect, String)> = d.get_temp::<Vec<(Rect, String)>>(crate::keytips::rects_id()).unwrap_or_default();
            v.push((rect, l));
            d.insert_temp(crate::keytips::rects_id(), v);
        });
    }
}

/// Whether a command exists and is enabled.
pub fn enabled(app: &WordApp, id: &str) -> bool {
    // No document open (#1129): only what opens one, Help and the settings.
    if !app.has_document() && !crate::works_without_document(id) {
        return false;
    }
    match app.session.registry.get(id) {
        Some(_) => app.session.why_disabled(id).is_none(),
        // Zotero: one command at a time, and only in the desktop app.
        None if id.starts_with("ui.zotero.") => app.zotero.busy.is_none() && !cfg!(target_arch = "wasm32"),
        None if id == "ui.linkTextBox" => enabled(app, "shape.link"),
        None if id == "ui.crop" => crate::crop::can_toggle(app),
        None => id.starts_with("ui."),
    }
}

pub(crate) fn bg(ui: &Ui, r: Rect, resp: &Response, checked: bool, t: &Tokens) {
    let fill = if resp.is_pointer_button_down_on() {
        t.pressed
    } else if checked {
        t.checked
    } else if resp.hovered() {
        t.hover
    } else {
        return;
    };
    ui.painter().rect_filled(r, CornerRadius::same(4), fill);
    if checked {
        ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.accent.linear_multiply(0.5)), egui::StrokeKind::Inside);
    }
}

/// Runs a ribbon button's command, then hands keyboard focus back to the document so typing
/// continues without another click, as in Word (#686). Interface commands (`ui.*`) open dialogs,
/// panes and tabs that take focus themselves, so they keep it.
pub(crate) fn run_from_ribbon(app: &mut WordApp, id: &str, params: Value) {
    let _ = app.run(id, params);
    if !id.starts_with("ui.") {
        app.canvas.want_focus = true;
    }
}

/// A large ribbon button: 32 px icon over a (possibly two-line) label.
pub fn big(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, params: Value, menu: bool) -> Response {
    big_button(ui, app, icon, label, id, params, menu, false)
}

/// A large ribbon button for a mode that stays on (`checked` draws it pressed), run without params.
pub fn big_toggle(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, checked: bool) -> Response {
    big_button(ui, app, icon, label, id, Value::Object(Default::default()), false, checked)
}

/// Size of the drop-down marker drawn after a label or beside a split button (#659: one marker,
/// the `dropdown` icon, everywhere; no text glyph).
pub const DROPDOWN: f32 = 10.0;

/// One centred label line (top at `y`); `marker` draws the drop-down icon after the text, in the
/// text's colour, and the pair is centred together.
pub fn label_line(ui: &Ui, cx: f32, y: f32, text: &str, font: egui::FontId, color: Color32, marker: bool) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let extra = if marker { DROPDOWN + 3.0 } else { 0.0 };
    let size = galley.size();
    let x = cx - (size.x + extra) / 2.0;
    ui.painter().galley(pos2(x, y), galley, color);
    if marker {
        let c = pos2(x + size.x + 3.0 + DROPDOWN / 2.0, y + size.y / 2.0);
        icons::paint(ui.painter(), Rect::from_center_size(c, vec2(DROPDOWN, DROPDOWN)), "dropdown", color, color);
    }
}

#[allow(clippy::too_many_arguments)]
fn big_button(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, params: Value, menu: bool, checked: bool) -> Response {
    let label = tl!(label);
    let t = Tokens::get(ui.ctx());
    let galley_w =
        label.split('\n').map(|l| ui.ctx().fonts_mut(|f| f.layout_no_wrap(l.to_string(), regular(11.5), t.text).size().x)).fold(0.0, f32::max);
    let w = (galley_w + 12.0).max(44.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, CONTENT_H), Sense::click());
    keytip_badge(ui, r, id, &label.replace('\n', " "));
    let on = enabled(app, id);
    bg(ui, r, &resp, checked, &t);
    let ic = Rect::from_center_size(pos2(r.center().x, r.min.y + BIG_ICON_Y), vec2(32.0, 32.0));
    let (c, a) = if on { (t.icon, t.accent) } else { (t.text_disabled, t.text_disabled) };
    icons::paint(ui.painter(), ic, icon, c, a);
    let mut y = r.min.y + BIG_LABEL_Y;
    let lines: Vec<&str> = label.split('\n').collect();
    for (i, l) in lines.iter().enumerate() {
        label_line(ui, r.center().x, y, l, regular(11.5), if on { t.text } else { t.text_disabled }, menu && i + 1 == lines.len());
        y += LABEL_PITCH;
    }
    let resp = tooltip(app, resp, &label.replace('\n', " "), id);
    command_menu(&resp, app, id);
    if resp.clicked() && on && !menu {
        run_from_ribbon(app, id, params);
    }
    resp
}

/// A small button: 16 px icon with an optional label; `checked` draws the toggled state.
pub fn small(ui: &mut Ui, app: &mut WordApp, icon: &str, label: Option<&str>, tip: &str, id: &str, params: Value, checked: bool) -> Response {
    let (label, tip) = (label.map(|l| tl!(l)), tl!(tip));
    // A compact group (#662) shows only the icon; the tooltip keeps the name.
    let label = label.filter(|_| !crate::ribbon_fit::compact(ui.ctx()));
    let t = Tokens::get(ui.ctx());
    let text_w = label.map(|l| ui.ctx().fonts_mut(|f| f.layout_no_wrap(l.to_string(), regular(11.5), t.text).size().x) + 6.0).unwrap_or(0.0);
    let (r, resp) = ui.allocate_exact_size(vec2(24.0 + text_w, 22.0), Sense::click());
    keytip_badge(ui, r, id, tip);
    let on = enabled(app, id);
    bg(ui, r, &resp, checked, &t);
    let ic = Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(17.0, 17.0));
    let (c, a) = if on { (t.icon, t.accent) } else { (t.text_disabled, t.text_disabled) };
    icons::paint(ui.painter(), ic, icon, c, a);
    if let Some(l) = label {
        ui.painter().text(pos2(r.min.x + 24.0, r.center().y), Align2::LEFT_CENTER, l, regular(11.5), if on { t.text } else { t.text_disabled });
    }
    let resp = tooltip(app, resp, tip, id);
    command_menu(&resp, app, id);
    if resp.clicked() && on {
        run_from_ribbon(app, id, params);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, tip));
    resp
}

/// A split button: the left part runs `id`, the arrow opens a menu filled by `menu`.
pub fn split(
    ui: &mut Ui,
    app: &mut WordApp,
    icon: &str,
    tip: &str,
    id: &str,
    params: Value,
    checked: bool,
    swatch: Option<Color32>,
    menu: impl FnOnce(&mut Ui, &mut WordApp),
) {
    let tip = tl!(tip);
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    keytip_badge(ui, r, id, tip);
    let (ar, aresp) = ui.allocate_exact_size(vec2(11.0, 22.0), Sense::click());
    let on = enabled(app, id);
    bg(ui, r, &resp, checked, &t);
    bg(ui, ar, &aresp, false, &t);
    let ic = Rect::from_center_size(r.center(), vec2(17.0, 17.0));
    let (c, a) = if on { (t.icon, t.accent) } else { (t.text_disabled, t.text_disabled) };
    icons::paint(ui.painter(), ic, icon, c, a);
    if let Some(sw) = swatch {
        let sw = if on { sw } else { t.text_disabled };
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.min.x + 4.0, r.max.y - 5.0), pos2(r.max.x - 4.0, r.max.y - 2.0)), 0.0, sw);
    }
    icons::paint(ui.painter(), Rect::from_center_size(ar.center(), vec2(DROPDOWN, DROPDOWN)), "dropdown", c, a);
    let resp = tooltip(app, resp, tip, id);
    command_menu(&resp, app, id);
    if resp.clicked() && on {
        run_from_ribbon(app, id, params);
    }
    let (arrow_title, _) = tooltip_lines(&crate::i18n::fmt(tl!("{name} options"), &[("name", tip)]), "", "", None);
    show_hint(&aresp, &arrow_title, &[]);
    // The split button reads as one control named after its tip, so a screen reader (and the
    // tests) can find it; the arrow is its menu.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, tip));
    // The arrow opens the choices beside the button: it says whose, so it is not just "button"
    // (#3172).
    aresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, &arrow_title));
    if !on {
        return;
    }
    egui::Popup::menu(&aresp).show(|ui| {
        ui.set_min_width(180.0);
        menu(ui, app);
    });
}

/// Whether a menu button whose menu is about command `id` is on ("" = always).
pub fn menu_enabled(app: &WordApp, id: &str) -> bool {
    id.is_empty() || enabled(app, id)
}

/// A button that only opens a menu (icon + label + drop-down marker).
pub fn menu_button(
    ui: &mut Ui,
    app: &mut WordApp,
    icon: &str,
    label: Option<&str>,
    tip: &str,
    big_btn: bool,
    id: &str,
    menu: impl FnOnce(&mut Ui, &mut WordApp),
) {
    let (label, tip) = (label.map(|l| tl!(l)), tl!(tip));
    let label = label.filter(|_| big_btn || !crate::ribbon_fit::compact(ui.ctx()));
    let t = Tokens::get(ui.ctx());
    // `id` names the command the menu is about (Header for the Header menu…); while it is
    // disabled the button greys out and doesn't open, like the other ribbon buttons. "" = always on.
    let on = menu_enabled(app, id);
    let (c, a, text) = if on { (t.icon, t.accent, t.text) } else { (t.text_disabled, t.text_disabled, t.text_disabled) };
    let resp = if big_btn {
        let galley_w = label
            .unwrap_or("")
            .split('\n')
            .map(|l| ui.ctx().fonts_mut(|f| f.layout_no_wrap(l.to_string(), regular(11.5), t.text).size().x))
            .fold(0.0, f32::max);
        let w = (galley_w + 22.0).max(44.0);
        let (r, resp) = ui.allocate_exact_size(vec2(w, CONTENT_H), Sense::click());
        keytip_badge(ui, r, "menu", tip);
        bg(ui, r, &resp, false, &t);
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.min.y + BIG_ICON_Y), vec2(32.0, 32.0)), icon, c, a);
        let lines: Vec<&str> = label.unwrap_or("").split('\n').collect();
        let mut y = r.min.y + BIG_LABEL_Y;
        for (i, l) in lines.iter().enumerate() {
            label_line(ui, r.center().x, y, l, regular(11.5), text, i + 1 == lines.len());
            y += LABEL_PITCH;
        }
        resp
    } else {
        let text_w = label.map(|l| ui.ctx().fonts_mut(|f| f.layout_no_wrap(l.to_string(), regular(11.5), t.text).size().x) + 6.0).unwrap_or(0.0);
        let (r, resp) = ui.allocate_exact_size(vec2(24.0 + text_w + 10.0, 22.0), Sense::click());
        keytip_badge(ui, r, "menu", tip);
        bg(ui, r, &resp, false, &t);
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(17.0, 17.0)), icon, c, a);
        if let Some(l) = label {
            ui.painter().text(pos2(r.min.x + 24.0, r.center().y), Align2::LEFT_CENTER, l, regular(11.5), text);
        }
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 6.0, r.center().y), vec2(DROPDOWN, DROPDOWN)), "dropdown", c, c);
        resp
    };
    let reason = if on { None } else { Some(app.session.why_disabled(id).unwrap_or("")) };
    let (title, notes) = tooltip_lines(tip, &shortcut_text(app, id), "", reason);
    show_hint(&resp, &title, &notes);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, tip));
    if !on {
        return;
    }
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(200.0);
        menu(ui, app);
    });
}


#[cfg(test)]
thread_local! {
    /// Groups drawn taller than [`CONTENT_H`] (title and height): their content runs into the
    /// group label below, as View › Show did with four check boxes in one column.
    pub(crate) static TALL_GROUPS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
mod tests;
