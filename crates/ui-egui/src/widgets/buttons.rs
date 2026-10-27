use super::groups::command_menu;
use super::tooltip::{keytip_badge, shortcut_text, show_hint, tooltip, tooltip_lines};
use super::*;

/// Height of the ribbon content area (without group labels).
pub const CONTENT_H: f32 = 66.0;

/// Height of the group title row under the content.
pub const LABEL_H: f32 = 18.0;

/// Large buttons: centre of the 32 px icon, top of the first label line, and the pitch and
/// height of a label line (`regular(11.5)`; a test checks the font's real line height).
pub const BIG_ICON_Y: f32 = 18.0;

pub const BIG_LABEL_Y: f32 = 36.0;

pub const LABEL_PITCH: f32 = 13.0;

pub const LABEL_LINE_H: f32 = 14.0;

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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, tip));
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
        let bar = Rect::from_min_max(pos2(r.min.x + 4.0, r.max.y - 5.0), pos2(r.max.x - 4.0, r.max.y - 2.0));
        ui.painter().rect_filled(bar, 0.0, sw);
        ui.painter().rect_stroke(bar, 0.0, Stroke::new(0.5, t.border), egui::StrokeKind::Inside);
    }
    icons::paint(ui.painter(), Rect::from_center_size(ar.center(), vec2(DROPDOWN, DROPDOWN)), "dropdown", c, a);
    let resp = tooltip(app, resp, tip, id);
    command_menu(&resp, app, id);
    if resp.clicked() && on {
        run_from_ribbon(app, id, params);
    }
    let (arrow_title, _) = tooltip_lines(&crate::i18n::fmt(tl!("{name} options"), &[("name", tip)]), "", "", None);
    show_hint(&aresp, &arrow_title, &[]);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, tip));
    // The arrow opens the choices beside the button: it says whose, so it is not just "button"
    // (#3172).
    aresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, &arrow_title));
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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, tip));
    if !on {
        return;
    }
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(200.0);
        menu(ui, app);
    });
}
