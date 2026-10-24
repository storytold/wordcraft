//! Ribbon widgets: large and small buttons, split buttons, groups, combo fields, colour grids.

use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use serde_json::Value;

use crate::theme::{Tokens, regular};
use crate::{WordApp, icons};

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

/// A ribbon group: content, a centred label below and a divider on the right.
pub fn group(ui: &mut Ui, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    // The English title keys the launcher's id; the drawn title is translated.
    group_titled(ui, title, tl!(title), launcher, app, add);
}

/// A ribbon group whose title is shown as is (a custom group's name, Customize Ribbon).
pub fn group_titled(ui: &mut Ui, id_title: &str, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    let t = Tokens::get(ui.ctx());
    let start = ui.cursor().min;
    // #662: the window may be too narrow for the tab; the plan says how this group fits.
    let position = crate::ribbon_fit::position(ui.ctx());
    let level = crate::ribbon_fit::level(ui.ctx(), id_title);
    if level == crate::ribbon_fit::COLLAPSED {
        collapsed_group(ui, id_title, position, title, launcher, app, add);
    } else {
        // Not folded: a drop-down left open while it was folded doesn't come back by itself.
        ui.data_mut(|d| d.remove::<bool>(collapsed_open_id(ui, id_title, position)));
        let label_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.0), t.group_label).size().x);
        let inner = ui.vertical(|ui| {
            ui.set_min_width(label_w + if launcher.is_some() { 22.0 } else { 10.0 });
            crate::ribbon_fit::set_compact(ui.ctx(), level == crate::ribbon_fit::COMPACT);
            let content = ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTENT_H), egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
                add(ui, app);
            });
            crate::ribbon_fit::end_compact(ui.ctx());
            #[cfg(test)]
            if content.response.rect.height() > CONTENT_H + 0.5 {
                TALL_GROUPS.with(|g| g.borrow_mut().push(format!("{id_title}: {:.1}", content.response.rect.height())));
            }
            #[cfg(not(test))]
            let _ = content;
        });
        let r = inner.response.rect;
        title_row(
            ui,
            Rect::from_min_max(pos2(start.x, start.y + CONTENT_H), pos2(r.max.x, start.y + CONTENT_H + LABEL_H)),
            id_title,
            title,
            launcher,
            app,
        );
    }
    ui.add_space(5.0);
    let x = ui.cursor().min.x;
    ui.painter().line_segment([pos2(x, start.y + 4.0), pos2(x, start.y + CONTENT_H + LABEL_H - 4.0)], Stroke::new(1.0, t.border));
    ui.add_space(6.0);
    crate::ribbon_fit::record(ui.ctx(), id_title, level, ui.cursor().min.x - start.x);
}

/// A group's title row (`r`): the centred title and, with a `launcher`, its dialog launcher.
fn title_row(ui: &mut Ui, r: Rect, id_title: &str, title: &str, launcher: Option<&str>, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    let title_y = r.center().y;
    ui.painter().text(pos2(r.center().x, title_y), Align2::CENTER_CENTER, title, regular(11.0), t.group_label);
    if let Some(cmd) = launcher {
        let lr = Rect::from_center_size(pos2(r.max.x - 6.0, title_y), vec2(11.0, 11.0));
        let resp = ui.interact(lr, ui.id().with(("launch", id_title)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(lr.expand(2.0), 2.0, t.hover);
        }
        icons::paint(ui.painter(), lr, "launcher", t.text_dim, t.accent);
        if resp.on_hover_text(crate::i18n::fmt(tl!("{group} settings"), &[("group", title)])).clicked() {
            let _ = app.run(cmd, serde_json::json!({}));
        }
    }
}

/// Where a collapsed group remembers whether its drop-down is open.
fn collapsed_open_id(ui: &Ui, id_title: &str, position: usize) -> egui::Id {
    ui.id().with(("collapsed_group", id_title, position))
}

/// A group folded into one drop-down button (the window is too narrow, #662): the button, as tall
/// as the group with its title, shows the group's name, and its drop-down holds the whole group as
/// designed with its title and launcher. The drop-down stays open while it is used (#1285): only a
/// click outside it, or on the button, closes it; clicks in the menus and lists its controls open
/// (font and size lists, Change Case, colours) count as inside.
fn collapsed_group(
    ui: &mut Ui,
    id_title: &str,
    position: usize,
    title: &str,
    launcher: Option<&str>,
    app: &mut WordApp,
    add: impl FnOnce(&mut Ui, &mut WordApp),
) {
    let t = Tokens::get(ui.ctx());
    let text_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.5), t.text).size().x);
    let (r, resp) = ui.allocate_exact_size(vec2((text_w + DROPDOWN + 14.0).max(48.0), CONTENT_H + LABEL_H), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    let open_id = collapsed_open_id(ui, id_title, position);
    let was_open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    bg(ui, r, &resp, was_open, &t);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(r.center().x, r.min.y + BIG_ICON_Y), vec2(32.0, 32.0)),
        group_icon(id_title),
        t.icon,
        t.accent,
    );
    label_line(ui, r.center().x, r.min.y + BIG_LABEL_Y, title, regular(11.5), t.text, true);
    let resp = resp.on_hover_text(title);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    let mut open = was_open;
    if resp.clicked() {
        open = !open;
    } else if open && ui.input(|i| i.pointer.any_click()) {
        // Popups (the drop-down, its controls' menus and lists) sit above the ribbon; a click on
        // the window below them is outside.
        let pos = ui.input(|i| i.pointer.interact_pos());
        let layer = pos.and_then(|p| ui.ctx().layer_id_at(p));
        if !layer.is_some_and(|l| matches!(l.order, egui::Order::Foreground | egui::Order::Tooltip)) {
            open = false;
        }
    }
    // Esc closes an open list inside first, then the drop-down.
    let child_open = egui::Popup::is_any_open(ui.ctx());
    let mut shown = open;
    if open {
        // This popup isn't counted by `Popup::is_any_open`, so keep the same Esc from also leaving
        // full screen.
        app.escape_free = false;
        egui::Popup::menu(&resp).open_bool(&mut shown).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(|ui| {
            ui.vertical(|ui| {
                ui.horizontal_top(|ui| {
                    ui.set_min_height(CONTENT_H);
                    ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
                    add(ui, app);
                });
                let (row, _) = ui.allocate_exact_size(vec2(ui.min_rect().width(), LABEL_H), Sense::hover());
                title_row(ui, row, id_title, title, launcher, app);
            });
        });
        if !shown && child_open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            shown = true;
        }
    }
    ui.data_mut(|d| d.insert_temp(open_id, shown));
}

/// The icon a collapsed group's button shows.
fn group_icon(title: &str) -> &'static str {
    match title {
        "Clipboard" => "paste",
        "Font" => "bold",
        "Paragraph" => "alignLeft",
        "Styles" => "styles",
        "Editing" => "find",
        "Voice" => "dictate",
        "Tables" => "table",
        "Links" => "link",
        _ => "more",
    }
}

/// The icon a command's button shows outside its own ribbon group (custom groups, the Quick
/// Access Toolbar): the one its ribbon button uses, which is usually named after the id's last
/// part. Names the icon set doesn't have draw a lettered tile ([`icons::paint`]).
pub fn command_icon(id: &str) -> &str {
    const ICONS: &[(&str, &str)] = &[
        ("arrange.wrap", "wrapText"),
        ("draw.highlighter", "highlight"),
        ("format.border", "charborder"),
        ("format.color", "fontcolor"),
        ("format.growFont", "grow"),
        ("format.shrinkFont", "shrink"),
        ("format.strikethrough", "strike"),
        ("format.highlight", "highlight"),
        ("insert.signatureLine", "signature"),
        ("mailings.envelopes", "envelope"),
        ("mailings.insertField", "mergeField"),
        ("para.ltr", "textLtr"),
        ("para.rtl", "textRtl"),
        ("para.bullets", "bullets"),
        ("para.numbering", "numbering"),
        ("table.insertRowAbove", "insertAbove"),
        ("view.commentsPane", "showComments"),
        ("view.marks", "pilcrow"),
        ("edit.repeat", "redo"),
        ("file.new", "blankPage"),
        ("tools.customizeRibbon", "more"),
    ];
    ICONS.iter().find(|(c, _)| *c == id).map_or_else(|| id.rsplit('.').next().unwrap_or(id), |(_, icon)| *icon)
}

/// Right-click on a command's button: put it on (or take it off) the Quick Access Toolbar, or
/// open Customize Ribbon (#379).
pub fn command_menu(resp: &Response, app: &mut WordApp, id: &str) {
    resp.context_menu(|ui| {
        let known = app.session.registry.get(id).is_some();
        let on_qat = app.session.ribbon.qat().contains(&id);
        if known && !on_qat && ui.button(tl!("Add to Quick Access Toolbar")).clicked() {
            let _ = app.run("tools.customizeRibbon", serde_json::json!({"qatAdd": id}));
            ui.close();
        }
        if on_qat && ui.button(tl!("Remove from Quick Access Toolbar")).clicked() {
            let _ = app.run("tools.customizeRibbon", serde_json::json!({"qatRemove": id}));
            ui.close();
        }
        if known {
            ui.separator();
        }
        customize_menu_items(ui, app);
    });
}

/// "Customize the Ribbon…" and "Customize Quick Access Toolbar…" (context menus), and "Show
/// Quick Access Toolbar" while the toolbar is hidden (#1945).
pub fn customize_menu_items(ui: &mut Ui, app: &mut WordApp) {
    if !app.ui.quick_access_toolbar && ui.button(tl!("Show Quick Access Toolbar")).clicked() {
        let _ = app.run("ui.quickAccessToolbar", serde_json::json!({"value": true}));
        ui.close();
    }
    if ui.button(tl!("Customize the Ribbon…")).clicked() {
        crate::dialogs_ribbon::open(app, crate::dialogs_ribbon::Page::Ribbon);
        ui.close();
    }
    if ui.button(tl!("Customize Quick Access Toolbar…")).clicked() {
        crate::dialogs_ribbon::open(app, crate::dialogs_ribbon::Page::QuickAccess);
        ui.close();
    }
}

/// A row of small buttons (used inside a vertical stack).
pub fn row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        add(ui);
    });
}

/// Word's standard colour grid (theme colours + tints, standard colours). Returns a picked hex.
pub fn color_grid(ui: &mut Ui, theme: &[wordcraft_doc::Rgb]) -> Option<String> {
    let mut picked = None;
    ui.label(egui::RichText::new(tl!("Theme Colors")).small().weak());
    let base: Vec<wordcraft_doc::Rgb> = theme.iter().take(10).copied().collect();
    let tint = |c: wordcraft_doc::Rgb, k: f32| {
        let f = |v: u8| if k >= 0.0 { (v as f32 + (255.0 - v as f32) * k) as u8 } else { (v as f32 * (1.0 + k)) as u8 };
        wordcraft_doc::Rgb(f(c.0), f(c.1), f(c.2))
    };
    let order = [1usize, 0, 3, 2, 4, 5, 6, 7, 8, 9];
    for k in [0.0f32, 0.8, 0.6, 0.4, -0.25, -0.5] {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, if k == 0.0 { 6.0 } else { 0.0 });
            for i in order {
                let Some(c) = base.get(i).copied() else { continue };
                let c = tint(c, k);
                if swatch(ui, c) {
                    picked = Some(c.hex());
                }
            }
        });
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Standard Colors")).small().weak());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
        for hex in ["C00000", "FF0000", "FFC000", "FFFF00", "92D050", "00B050", "00B0F0", "0070C0", "002060", "7030A0"] {
            if let Some(c) = wordcraft_doc::Rgb::parse(hex)
                && swatch(ui, c)
            {
                picked = Some(hex.to_string());
            }
        }
    });
    picked
}

pub(crate) fn swatch(ui: &mut Ui, c: wordcraft_doc::Rgb) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::click());
    ui.painter().rect_filled(r, 0.0, Color32::from_rgb(c.0, c.1, c.2));
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(
        r,
        0.0,
        Stroke::new(if resp.hovered() { 1.5 } else { 0.5 }, if resp.hovered() { t.accent } else { t.border_strong }),
        egui::StrokeKind::Inside,
    );
    resp.on_hover_text(format!("#{}", c.hex())).clicked()
}

/// A ribbon-style text field with a dropdown list. Returns a chosen/typed value; Enter commits
/// the typed text as is (the font size box relies on that: "1" must not become "10").
pub fn combo(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> Option<String> {
    combo_impl(ui, id, width, current, items, &[], preview, false, "")
}

/// [`combo`] whose field and arrow show the hover hint `tip` (already translated).
pub fn combo_hint(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
    tip: &str,
) -> Option<String> {
    combo_impl(ui, id, width, current, items, &[], preview, false, tip)
}

/// What [`font_combo`] returns when its list's last entry, Add Font…, is chosen (#582): the
/// caller runs `font.add`, which shows a file picker. Not a family name anyone can type.
pub const ADD_FONT: &str = "\u{1}font.add";

/// What [`font_combo`] returns when "Load System Installed Fonts…" is chosen on web: the
/// caller runs `font.loadSystem`.
pub const LOAD_SYSTEM_FONTS: &str = "\u{1}font.loadSystem";

/// The font box: a [`combo`] where typing lists the matching families under the field (#359).
/// Its list ends with Add Font… ([`ADD_FONT`]).
/// Up/Down move the highlight, Enter or a click applies the highlighted family ("arial" applies
/// "Arial"), Escape restores the current font. Text matching no family is applied as typed, as
/// Word allows fonts that are not installed.
pub fn font_combo(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> Option<String> {
    combo_impl(ui, id, width, current, items, &[], preview, true, "")
}

/// [`font_combo`] with the `recent` families (most recent first) in a Recently Used Fonts
/// section above the list, then All Fonts, as in Word (#2068). The sections show when nothing is
/// typed; a query lists its matches only. Up/Down walk the recent entries, then the list; the
/// headings are skipped. A recent font that is no longer installed stays listed and is applied as
/// typed.
pub fn font_combo_with_recent(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    recent: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> Option<String> {
    combo_impl(ui, id, width, current, items, recent, preview, true, "")
}

/// [`font_combo_with_recent`] with a hover hint `tip` (already translated).
pub fn font_combo_with_recent_hint(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    recent: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
    tip: &str,
) -> Option<String> {
    combo_impl(ui, id, width, current, items, recent, preview, true, tip)
}

/// Indices of the `items` matching `query`, ignoring case: exact matches, then the names starting
/// with it, then those containing it, each group in list order. A blank query matches everything.
pub fn font_matches(query: &str, items: &[String]) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    let (mut exact, mut prefix, mut inner) = (Vec::new(), Vec::new(), Vec::new());
    for (i, it) in items.iter().enumerate() {
        let name = it.to_lowercase();
        if name == q {
            exact.push(i);
        } else if name.starts_with(&q) {
            prefix.push(i);
        } else if name.contains(&q) {
            inner.push(i);
        }
    }
    exact.extend(prefix);
    exact.extend(inner);
    exact
}

/// Width of a combo arrow's border, the field's own (the theme's input border is 1 pt).
const ARROW_BORDER: f32 = 1.0;

/// A searching combo's state between frames.
#[derive(Clone, Default)]
struct ComboSearch {
    /// The match list is showing (the user typed since focusing the field).
    open: bool,
    /// The query `matches` were computed for, and the list length then.
    query: String,
    items_len: usize,
    matches: std::sync::Arc<Vec<usize>>,
    /// Index among the entries: the recent families shown, then `matches`.
    highlight: usize,
    /// Up or Down moved the highlight since the query last changed.
    moved: bool,
    /// Rows the list drew last frame; 0 while it is hidden.
    rows: usize,
    /// Where the list was drawn last frame, its scroll offset and visible height.
    popup: Option<Rect>,
    offset: f32,
    view_h: f32,
}

fn combo_impl(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    recent: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
    search: bool,
    tip: &str,
) -> Option<String> {
    let mut out = None;
    let edit_id = ui.id().with(("combo_text", id));
    let te_id = ui.id().with(("combo_edit", id));
    let search_id = ui.id().with(("combo_search", id));
    let mut text = ui.data_mut(|d| d.get_temp::<String>(edit_id)).unwrap_or_else(|| current.to_string());
    let mut s: ComboSearch = if search { ui.data_mut(|d| d.get_temp(search_id)).unwrap_or_default() } else { ComboSearch::default() };
    let mut focused = false;
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        // The field and its arrow share one outline: rounded on the outside, square where they
        // meet. The field's frame takes the widget visuals' radius in every state.
        let r = ui.visuals().widgets.inactive.corner_radius;
        let w = &mut ui.visuals_mut().widgets;
        for v in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            v.corner_radius.ne = 0;
            v.corner_radius.se = 0;
        }
        // Up/Down move the list's highlight: take them before the text field moves its cursor.
        let mut step = 0i32;
        if s.open && ui.memory(|m| m.has_focus(te_id)) {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    step += 1;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    step -= 1;
                }
            });
        }
        let te = egui::TextEdit::singleline(&mut text).id(te_id).desired_width(width - 16.0).font(regular(12.0)).margin(vec2(4.0, 3.0));
        // The arrow's box is painted here, under the field, once its size is known.
        let arrow_bg = ui.painter().add(egui::Shape::Noop);
        let resp = ui.add(te);
        // The field says what it is: without a name a screen reader has only "edit" to say
        // (#3172). The arrow says whose options it opens, as the split buttons' arrow does.
        if !tip.is_empty() {
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, ui.is_enabled(), tip));
        }
        keytip_badge(ui, resp.rect, "combo", id);
        focused = resp.has_focus();
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut reveal = step != 0;
        if search && resp.changed() {
            s.open = true;
            s.highlight = 0;
            s.moved = false;
            reveal = true;
        }
        // The recent section shows while nothing is typed; a query lists its matches only.
        let recent_shown: &[String] = if text.trim().is_empty() { recent } else { &[] };
        let mut regain_focus = false;
        if s.open {
            if s.query != text || s.items_len != items.len() {
                s.matches = std::sync::Arc::new(font_matches(&text, items));
                s.query = text.clone();
                s.items_len = items.len();
            }
            let last = recent_shown.len().saturating_add(s.matches.len()).saturating_sub(1);
            s.moved |= step != 0;
            s.highlight = match step {
                1.. => s.highlight.saturating_add(1),
                0 => s.highlight,
                _ => s.highlight.saturating_sub(1),
            }
            .min(last);
            if enter {
                let picked = recent_shown
                    .get(s.highlight)
                    .or_else(|| s.highlight.checked_sub(recent_shown.len()).and_then(|k| s.matches.get(k)).and_then(|&i| items.get(i)));
                // A cleared box commits as typed (nothing) unless Up or Down picked an entry.
                out = Some(match picked {
                    Some(name) if !text.trim().is_empty() || s.moved => name.clone(),
                    _ => text.clone(),
                });
                s.open = false;
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Tab)) {
                s.open = false;
            } else if !focused {
                // A click in the list takes focus from the field: keep the list, give focus back.
                let over = ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| s.popup.is_some_and(|r| r.contains(p)));
                if over {
                    regain_focus = true;
                } else {
                    s.open = false;
                }
            }
        } else if enter {
            out = Some(text.clone());
        }
        let (ar, aresp) = ui.allocate_exact_size(vec2(16.0, resp.rect.height()), Sense::click());
        if !tip.is_empty() {
            let arrow_name = crate::i18n::fmt(tl!("{name} options"), &[("name", tip)]);
            aresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &arrow_name));
        }
        // It reaches one border width under the field, so the field's right border is the only line
        // between them: two borders side by side read as a double-width divider.
        let ar_radius = CornerRadius { nw: 0, sw: 0, ne: r.ne, se: r.se };
        let under = Rect::from_min_max(pos2(ar.min.x - ARROW_BORDER, ar.min.y), ar.max);
        let fill = if aresp.hovered() { t.hover } else { t.input };
        let arrow_box = egui::epaint::RectShape::new(under, ar_radius, fill, Stroke::new(ARROW_BORDER, t.input_border), egui::StrokeKind::Inside);
        ui.painter().set(arrow_bg, arrow_box);
        // Not while typing: the family list drops from the field's bottom edge and the hint would cover it.
        // A field only focused by Tab, with nothing typed, still gets its (delayed) keyboard hint.
        let typing = focused && text != current;
        if !tip.is_empty() && !typing && !s.open {
            show_hint(&resp, tip, &[]);
            show_hint(&aresp, tip, &[]);
        }
        icons::paint(ui.painter(), Rect::from_center_size(ar.center(), vec2(DROPDOWN, DROPDOWN)), "dropdown", t.icon, t.accent);
        // The list drops from the box's left edge, as in Word, not from the arrow (#535).
        // The font list closes on a chosen entry or a click outside it, not on its headings.
        let close = if search { egui::PopupCloseBehavior::CloseOnClickOutside } else { egui::PopupCloseBehavior::CloseOnClick };
        egui::Popup::menu(&aresp).anchor(resp.rect.union(ar)).close_behavior(close).show(|ui| {
            let item = |k: usize| items.get(k);
            let row = |k: usize| section_row(k, recent, &item);
            if let Some(v) = list_rows(ui, width, current, section_rows(recent.len(), items.len()), &row, None, None, preview).0 {
                out = Some(v);
                ui.close();
            }
            if search {
                ui.separator();
                if ui.button(tl!("Add Font…")).clicked() {
                    out = Some(ADD_FONT.to_string());
                    ui.close();
                }
                #[cfg(target_arch = "wasm32")]
                if ui.button(tl!("Load System Installed Fonts…")).clicked() {
                    out = Some(LOAD_SYSTEM_FONTS.to_string());
                    ui.close();
                }
            }
        });
        let n = section_rows(recent_shown.len(), s.matches.len());
        if s.open && n > 0 {
            let matches = s.matches.clone();
            let scroll = reveal.then_some((s.offset, s.view_h));
            let lit = highlight_row(s.highlight, recent_shown.len());
            // A list of a new length is sized again (an area keeps its last size, so a list that
            // once held one match stayed one row tall). That pass draws nothing, so it is
            // discarded and the frame drawn again, as egui does for its own sizing passes.
            let list_id = search_id.with("list");
            let resize = n != s.rows;
            if resize || egui::AreaState::load(ui.ctx(), list_id).is_none() {
                ui.ctx().request_discard("font list resized");
            }
            s.rows = n;
            let area = egui::Area::new(list_id).order(egui::Order::Tooltip).fixed_pos(resp.rect.left_bottom()).sizing_pass(resize);
            let shown = area.show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        let item = |k: usize| matches.get(k).and_then(|&i| items.get(i));
                        let row = |k: usize| section_row(k, recent_shown, &item);
                        list_rows(ui, width, current, n, &row, Some(lit), scroll, preview)
                    })
                    .inner
            });
            s.popup = Some(shown.response.rect);
            let (picked, offset, view_h) = shown.inner;
            (s.offset, s.view_h) = (offset, view_h);
            if let Some(v) = picked {
                out = Some(v);
                s.open = false;
                regain_focus = false;
                ui.memory_mut(|m| m.surrender_focus(te_id));
            }
        } else {
            s.popup = None;
            s.rows = 0;
        }
        if regain_focus && s.open {
            ui.memory_mut(|m| m.request_focus(te_id));
        }
    });
    let keep = (focused || s.open) && out.is_none();
    ui.data_mut(|d| {
        if keep {
            d.insert_temp(edit_id, text);
        } else {
            d.remove::<String>(edit_id);
        }
        if search {
            d.insert_temp(search_id, s);
        }
    });
    out
}

/// Height of a row drawn by a combo's preview closure (`Previews::font_preview_fn`'s entries).
pub const COMBO_PREVIEW_ROW_H: f32 = 24.0;

/// A combo's scrolling list. Only the rows in view are built: the font menu lists every installed
/// family and each preview loads that font, so building all rows loaded every font on the system
/// (gigabytes on large font collections) and froze the app (#121).
pub fn combo_list(ui: &mut Ui, width: f32, current: &str, items: &[String], preview: Option<&dyn Fn(&mut Ui, &str) -> Response>) -> Option<String> {
    list_rows(ui, width, current, items.len(), &|k| items.get(k).map(Row::Item), None, None, preview).0
}

/// A row of a combo's list: a dim section heading nothing picks, or an entry.
enum Row<'a> {
    Heading(&'a str),
    Item(&'a String),
}

/// Rows in a list of `n` entries headed by `recent` recent ones: the two headings and the recent
/// entries come on top when there are any.
fn section_rows(recent: usize, n: usize) -> usize {
    if recent == 0 { n } else { n.saturating_add(recent).saturating_add(2) }
}

/// Row `k` of a list headed by the `recent` families: Recently Used Fonts, those families, All
/// Fonts, then `item(0..)`. Without recent families the rows are the items.
fn section_row<'a>(k: usize, recent: &'a [String], item: &dyn Fn(usize) -> Option<&'a String>) -> Option<Row<'a>> {
    if recent.is_empty() {
        return item(k).map(Row::Item);
    }
    if k == 0 {
        return Some(Row::Heading(tl!("Recently Used Fonts")));
    }
    if let Some(r) = recent.get(k.saturating_sub(1)) {
        return Some(Row::Item(r));
    }
    if k == recent.len().saturating_add(1) {
        return Some(Row::Heading(tl!("All Fonts")));
    }
    item(k.saturating_sub(recent.len()).saturating_sub(2)).map(Row::Item)
}

/// The row of entry `entry` in a list headed by `recent` recent families ([`section_row`]).
fn highlight_row(entry: usize, recent: usize) -> usize {
    if recent == 0 {
        entry
    } else if entry < recent {
        entry.saturating_add(1)
    } else {
        entry.saturating_add(2)
    }
}

/// A section heading's row of `size`: dim text, a rule across the list above it after the
/// first row, nothing to click. The row asks for no more than `size`: the list's width is its
/// entries' (a sizing pass offers an unbounded width).
fn heading_row(ui: &mut Ui, text: &str, size: egui::Vec2, rule: bool) {
    let weak = ui.visuals().weak_text_color();
    let line = ui.visuals().widgets.noninteractive.bg_stroke;
    if rule {
        ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), line);
    }
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(size);
        ui.add_space(6.0);
        ui.add(egui::Label::new(egui::RichText::new(text).small().color(weak)).selectable(false));
    });
}

/// The rows of [`combo_list`], virtualised, for `n` rows fetched by index. With a `highlight`
/// that row is marked instead of `current`; `scroll` (last frame's offset and visible height)
/// scrolls the highlight into view. Returns the clicked item, the scroll offset and the height.
fn list_rows<'a>(
    ui: &mut Ui,
    width: f32,
    current: &str,
    n: usize,
    row: &dyn Fn(usize) -> Option<Row<'a>>,
    highlight: Option<usize>,
    scroll: Option<(f32, f32)>,
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> (Option<String>, f32, f32) {
    let mut out = None;
    let row_h = if preview.is_some() { COMBO_PREVIEW_ROW_H } else { ui.spacing().interact_size.y };
    let desired_h = (n as f32 * (row_h + ui.spacing().item_spacing.y)).min(420.0);
    ui.set_min_height(desired_h);
    let mut area = egui::ScrollArea::vertical().max_height(desired_h);
    if let (Some(h), Some((offset, view_h))) = (highlight, scroll) {
        let top = h as f32 * (row_h + ui.spacing().item_spacing.y);
        let bottom = top + row_h;
        let offset = if view_h <= 0.0 {
            // No list was visible for the previous query. Start at the first row, including
            // its heading, rather than scrolling to the first selectable entry below it.
            0.0
        } else if top < offset {
            top
        } else if bottom > offset + view_h {
            bottom - view_h
        } else {
            offset
        };
        area = area.vertical_scroll_offset(offset.max(0.0));
    }
    let fill = Tokens::get(ui.ctx()).checked;
    let shown = area.show_rows(ui, row_h, n, |ui, range| {
        ui.set_min_width(width + 60.0);
        for k in range {
            let it = match row(k) {
                Some(Row::Item(it)) => it,
                Some(Row::Heading(h)) => {
                    heading_row(ui, h, vec2(width + 60.0, row_h), k > 0);
                    continue;
                }
                None => continue,
            };
            let lit = highlight == Some(k);
            let clicked = match preview {
                Some(p) => {
                    if lit {
                        let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), row_h));
                        ui.painter().rect_filled(r, 3.0, fill);
                    }
                    p(ui, it).clicked()
                }
                None => ui.selectable_label(if highlight.is_some() { lit } else { it == current }, it.as_str()).clicked(),
            };
            if clicked {
                out = Some(it.clone());
            }
        }
    });
    (out, shown.state.offset.y, shown.inner_rect.height())
}

/// How a drawn button looks this frame. Hover fades in over egui's animation time and the
/// pressed state blends in while the pointer is down, as on egui's own buttons. Framed buttons
/// take egui's widget visuals (fill, stroke, text colour, expansion) so they match `ui.button`;
/// frameless ones take the ribbon's hover/pressed tokens, like [`small`] and [`big`]. With
/// animation effects off (egui's animation time is zero) both states show at once.
pub struct DrawnState {
    pub fill: Color32,
    pub stroke: Stroke,
    pub text: Color32,
    pub expansion: f32,
}

pub fn drawn_state(ui: &Ui, resp: &Response, framed: bool) -> DrawnState {
    let pressed = resp.is_pointer_button_down_on();
    let h = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered() || pressed);
    let p = ui.ctx().animate_bool_with_time(resp.id.with("press"), pressed, ui.style().animation_time.min(0.06));
    if framed {
        let w = &ui.visuals().widgets;
        let mix = |a: Color32, b: Color32, c: Color32| a.lerp_to_gamma(b, h).lerp_to_gamma(c, p);
        let num = |a: f32, b: f32, c: f32| egui::lerp(egui::lerp(a..=b, h)..=c, p);
        DrawnState {
            fill: mix(w.inactive.weak_bg_fill, w.hovered.weak_bg_fill, w.active.weak_bg_fill),
            stroke: Stroke::new(
                num(w.inactive.bg_stroke.width, w.hovered.bg_stroke.width, w.active.bg_stroke.width),
                mix(w.inactive.bg_stroke.color, w.hovered.bg_stroke.color, w.active.bg_stroke.color),
            ),
            text: mix(w.inactive.fg_stroke.color, w.hovered.fg_stroke.color, w.active.fg_stroke.color),
            expansion: num(w.inactive.expansion, w.hovered.expansion, w.active.expansion),
        }
    } else {
        let t = Tokens::get(ui.ctx());
        DrawnState {
            fill: Color32::TRANSPARENT.lerp_to_gamma(t.hover, h).lerp_to_gamma(t.pressed, p),
            stroke: Stroke::NONE,
            text: t.text,
            expansion: 0.0,
        }
    }
}

/// A dialog's primary button (OK, Save, Close): the accent fill with `on_accent` text, per theme.
pub fn primary_button(ui: &Ui, text: impl Into<String>) -> egui::Button<'static> {
    let t = Tokens::get(ui.ctx());
    egui::Button::new(egui::RichText::new(text.into()).color(t.on_accent)).fill(t.accent)
}

/// A frameless 22 px icon button (pane close buttons): ribbon-style hover and press feedback.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let st = drawn_state(ui, &resp, false);
    ui.painter().rect_filled(r, CornerRadius::same(4), st.fill);
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), icon, t.icon, t.accent);
    resp.on_hover_text(tip)
}

/// A text button with a drawn icon in front. Symbols such as ✎ or 💬 aren't in the interface
/// fonts and would draw as empty boxes, so icons are painted (`icons.rs`) rather than typed.
/// It looks and reacts like `ui.button` (see [`drawn_state`]).
pub fn icon_text_button(ui: &mut Ui, icon: &str, text: &str, font: egui::FontId) -> Response {
    icon_text_button_with(ui, icon, text, font, false, true)
}

/// [`icon_text_button`] that opens a menu: the drop-down marker follows the text.
pub fn icon_text_menu_button(ui: &mut Ui, icon: &str, text: &str, font: egui::FontId) -> Response {
    icon_text_button_with(ui, icon, text, font, true, true)
}

/// The icon-only form of [`icon_text_button`] / [`icon_text_menu_button`] for a crowded strip
/// (#2026): `label` is the accessible name and the tooltip, not drawn.
pub fn icon_only_button(ui: &mut Ui, icon: &str, label: &str, font: egui::FontId, marker: bool) -> Response {
    icon_text_button_with(ui, icon, label, font, marker, false).on_hover_text(label)
}

/// Width [`icon_text_button`] (or its menu / icon-only forms) takes.
pub fn icon_text_button_width(ui: &Ui, text: &str, font: egui::FontId, marker: bool, show_text: bool) -> f32 {
    let text_w = if show_text { ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER).size().x) } else { 0.0 };
    icon_text_button_size(ui, text_w, marker, show_text)
}

fn icon_text_button_size(ui: &Ui, text_w: f32, marker: bool, show_text: bool) -> f32 {
    let pad = ui.spacing().button_padding;
    let gap = if show_text { 4.0 } else { 0.0 };
    let marker_w = if marker { DROPDOWN + 4.0 } else { 0.0 };
    pad.x + 16.0 + gap + text_w + marker_w + pad.x
}

fn icon_text_button_with(ui: &mut Ui, icon: &str, text: &str, font: egui::FontId, marker: bool, show_text: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let pad = ui.spacing().button_padding;
    let icon_w = 16.0;
    let text_w = if show_text { galley.size().x } else { 0.0 };
    let size = vec2(icon_text_button_size(ui, text_w, marker, show_text), galley.size().y.max(icon_w) + 2.0 * pad.y);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    let st = drawn_state(ui, &resp, true);
    let r = r.expand(st.expansion);
    ui.painter().rect(r, ui.visuals().widgets.inactive.corner_radius, st.fill, st.stroke, egui::StrokeKind::Inside);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(r.min.x + pad.x + icon_w / 2.0, r.center().y), vec2(icon_w, icon_w)),
        icon,
        t.icon,
        t.accent,
    );
    let text_x = r.min.x + pad.x + icon_w + if show_text { 4.0 } else { 0.0 };
    if show_text {
        ui.painter().galley(pos2(text_x, r.center().y - galley.size().y / 2.0), galley, st.text);
    }
    if marker {
        let c = pos2(text_x + text_w + 4.0 + DROPDOWN / 2.0, r.center().y);
        icons::paint(ui.painter(), Rect::from_center_size(c, vec2(DROPDOWN, DROPDOWN)), "dropdown", st.text, st.text);
    }
    resp
}

/// A drop-down menu item with `icon` before `text`, at least `min_w` wide.
pub fn icon_menu_item(ui: &mut Ui, icon: &str, text: &str, shortcut: &str, min_w: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::PLACEHOLDER);
    let sc = (!shortcut.is_empty()).then(|| ui.painter().layout_no_wrap(shortcut.to_string(), font, Color32::PLACEHOLDER));
    let pad = ui.spacing().button_padding;
    let icon_w = 16.0;
    let sc_w = sc.as_ref().map_or(0.0, |g| g.size().x + 3.0 * pad.x);
    let w = (pad.x + icon_w + 6.0 + galley.size().x + sc_w + pad.x).max(min_w).max(ui.available_width());
    let (r, resp) = ui.allocate_exact_size(vec2(w, galley.size().y.max(icon_w) + 2.0 * pad.y), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    let st = drawn_state(ui, &resp, false);
    ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, st.fill);
    let (line, accent) = if ui.is_enabled() { (t.icon, t.accent) } else { (t.text_disabled, t.text_disabled) };
    let text_c = if ui.is_enabled() { st.text } else { t.text_disabled };
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + pad.x + icon_w / 2.0, r.center().y), vec2(icon_w, icon_w)), icon, line, accent);
    ui.painter().galley(pos2(r.min.x + pad.x + icon_w + 6.0, r.center().y - galley.size().y / 2.0), galley, text_c);
    if let Some(g) = sc {
        ui.painter().galley(pos2(r.max.x - pad.x - g.size().x, r.center().y - g.size().y / 2.0), g, t.text_disabled);
    }
    resp
}

/// A strip of dialog tabs. The active tab joins the page below it.
/// Returns the index of the tab the user clicked.
pub fn dialog_tabs(ui: &mut Ui, labels: &[String], selected: usize) -> Option<usize> {
    const H: f32 = 26.0;
    let t = Tokens::get(ui.ctx());
    let page = ui.visuals().window_fill;
    let need: f32 = labels.iter().map(|l| ui.painter().layout_no_wrap(l.clone(), regular(12.5), t.text).size().x + 27.0).sum::<f32>() + 1.0;
    ui.set_min_width(need);
    let origin = ui.cursor().min;
    let width = ui.available_width();
    let base_y = origin.y + H;
    let base = ui.painter().add(egui::Shape::Noop);
    let top = CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 };
    let mut x = origin.x;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let on = i == selected;
        let galley = ui.painter().layout_no_wrap(label.clone(), regular(12.5), t.text);
        let w = galley.size().x + 28.0;
        let r = Rect::from_min_max(pos2(x, origin.y + 2.0), pos2(x + w, base_y));
        let resp = ui.interact(r, ui.id().with(("dialog_tab", i)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, label));
        let hover = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered());
        let fill = if on { page } else { page.lerp_to_gamma(t.border, 0.14 + 0.1 * hover) };
        let p = ui.painter();
        let shown = if on { r } else { Rect::from_min_max(pos2(r.min.x, r.min.y + 2.0), r.max) };
        p.rect(shown, top, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        if on {
            p.rect_filled(Rect::from_min_max(pos2(r.min.x + 1.0, base_y - 1.0), pos2(r.max.x - 1.0, base_y + 1.0)), 0.0, page);
        }
        let color = if on { t.text } else { t.text.gamma_multiply(0.8) };
        p.galley(pos2(r.center().x - galley.size().x / 2.0, r.center().y - galley.size().y / 2.0), galley, color);
        if resp.clicked() && !on {
            clicked = Some(i);
        }
        x += w - 1.0;
    }
    ui.allocate_space(vec2(width, H + 1.0));
    ui.painter().set(base, egui::Shape::line_segment([pos2(origin.x, base_y), pos2(origin.x + width, base_y)], Stroke::new(1.0, t.border)));
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #653: a large button's label line is no taller than `LABEL_LINE_H` in the interface font, so
    /// two lines (checked at compile time) end above the group title row.
    #[test]
    fn big_button_label_lines_fit_their_slot() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(false, false));
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let h = ctx.fonts_mut(|f| f.layout_no_wrap("Page Number Mail Merge gjpqy".into(), crate::theme::regular(11.5), Color32::WHITE).size().y);
        assert!(h <= LABEL_LINE_H, "label line is {h} px tall");
    }

    /// #1945: a ribbon button's right-click menu offers Show Quick Access Toolbar only while the
    /// toolbar is hidden, and it brings the toolbar back.
    #[test]
    fn ribbon_menu_offers_show_quick_access_toolbar_only_while_hidden() {
        use egui_kittest::kittest::Queryable;
        let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        // The ribbon's Bold is the topmost one (the mini toolbar's, when up, sits lower).
        let ribbon_bold = |h: &egui_kittest::Harness<'static, WordApp>| {
            h.get_all_by_label("Bold").map(|n| n.rect().center()).min_by(|a, b| a.y.total_cmp(&b.y)).expect("the ribbon has Bold")
        };
        let right_click = |h: &mut egui_kittest::Harness<'static, WordApp>, at: egui::Pos2| {
            h.event(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            }
            h.run_steps(2);
        };
        let at = ribbon_bold(&h);
        right_click(&mut h, at);
        assert!(h.query_by_label("Add to Quick Access Toolbar").is_some(), "the button's menu opened");
        assert!(h.query_by_label("Show Quick Access Toolbar").is_none(), "nothing extra while the toolbar shows");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        h.state_mut().run("ui.quickAccessToolbar", serde_json::json!({"value": false})).unwrap();
        h.run_steps(2);
        let at = ribbon_bold(&h);
        right_click(&mut h, at);
        h.get_by_label("Show Quick Access Toolbar").click();
        h.run_steps(2);
        assert!(h.state().ui.quick_access_toolbar, "Show Quick Access Toolbar brings it back");
        assert!(h.query_by_label("Show Quick Access Toolbar").is_none(), "and closes the menu");
    }

    fn hint_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(false, false));
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx
    }

    /// Runs `frames` frames of `build` from time `t0` on 10 fps, with `events` in the first; returns
    /// whether a tooltip layer is showing after the last.
    fn hint_showing(ctx: &egui::Context, t0: f64, frames: u32, events: Vec<egui::Event>, build: &mut dyn FnMut(&mut Ui)) -> bool {
        let mut shown = false;
        for k in 0..frames {
            let input = egui::RawInput {
                time: Some(t0 + f64::from(k) * 0.1),
                predicted_dt: 0.1,
                events: if k == 0 { events.clone() } else { vec![] },
                ..Default::default()
            };
            ctx.run_ui(input, |ui| build(ui)).drop_without_applying_deltas();
            shown = ctx.memory(|m| m.areas().visible_layer_ids().iter().any(|l| l.order == egui::Order::Tooltip));
        }
        shown
    }

    #[test]
    fn a_combo_shows_its_hint_on_hover_and_not_before() {
        let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
        let ctx = hint_ctx();
        let field = std::cell::Cell::new(Rect::NOTHING);
        let mut build = |ui: &mut Ui| {
            combo_hint(ui, "f", 150.0, "12", &sizes, None, "Font Size");
            field.set(ui.min_rect());
        };
        assert!(!hint_showing(&ctx, 1.0, 3, vec![], &mut build));
        let over = field.get().left_center() + vec2(20.0, 0.0);
        assert!(hint_showing(&ctx, 2.0, 12, vec![egui::Event::PointerMoved(over)], &mut build), "no hint over the field");
        // Over the arrow too.
        let ctx = hint_ctx();
        let mut build2 = |ui: &mut Ui| {
            combo_hint(ui, "f", 150.0, "12", &sizes, None, "Font Size");
        };
        hint_showing(&ctx, 1.0, 3, vec![], &mut build2);
        let arrow = field.get().right_center() - vec2(6.0, 0.0);
        assert!(hint_showing(&ctx, 2.0, 12, vec![egui::Event::PointerMoved(arrow)], &mut build2), "no hint over the arrow");
    }

    #[test]
    fn a_disabled_control_still_shows_its_hint_on_hover() {
        let ctx = hint_ctx();
        let rect = std::cell::Cell::new(Rect::NOTHING);
        let mut build = |ui: &mut Ui| {
            ui.add_enabled_ui(false, |ui| {
                let resp = ui.allocate_response(vec2(60.0, 22.0), Sense::click());
                show_hint(&resp, "Bold", &["Select text first".to_string()]);
                rect.set(resp.rect);
            });
        };
        assert!(!hint_showing(&ctx, 1.0, 3, vec![], &mut build));
        let over = rect.get().center();
        assert!(hint_showing(&ctx, 2.0, 12, vec![egui::Event::PointerMoved(over)], &mut build), "no hint over a disabled control");
    }

    #[test]
    fn a_focus_hint_gives_way_to_a_disabled_control_under_the_pointer() {
        let ctx = hint_ctx();
        let off = std::cell::Cell::new(Rect::NOTHING);
        let mut build = |ui: &mut Ui| {
            ui.horizontal(|ui| {
                let (_, a) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
                show_hint(&a, "Bold", &[]);
                ui.add_enabled_ui(false, |ui| {
                    let b = ui.allocate_response(vec2(40.0, 20.0), Sense::click());
                    show_hint(&b, "Italic", &["Select text first".to_string()]);
                    off.set(b.rect);
                });
            });
        };
        let tips = |ctx: &egui::Context| ctx.memory(|m| m.areas().visible_layer_ids().iter().filter(|l| l.order == egui::Order::Tooltip).count());
        hint_showing(&ctx, 1.0, 2, vec![], &mut build);
        let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let over = egui::Event::PointerMoved(off.get().center());
        // Focus the first control, rest the pointer on the disabled one: past the idle time only its reason shows.
        hint_showing(&ctx, 2.0, 2, vec![tab, over], &mut build);
        assert!(hint_showing(&ctx, 2.2, 40, vec![], &mut build), "the disabled control shows no hint");
        assert_eq!(tips(&ctx), 1, "the focused control's hint is drawn beside the disabled control's");
    }

    #[test]
    fn a_combo_field_reached_with_tab_shows_its_hint_after_a_delay() {
        let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
        let ctx = hint_ctx();
        let focused = std::cell::Cell::new(false);
        let mut build = |ui: &mut Ui| {
            combo_hint(ui, "f", 150.0, "12", &sizes, None, "Font Size");
            focused.set(ui.memory(|m| m.focused()).is_some());
        };
        hint_showing(&ctx, 1.0, 2, vec![], &mut build);
        let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        assert!(!hint_showing(&ctx, 2.0, 2, vec![tab], &mut build));
        assert!(focused.get(), "Tab did not focus the field");
        assert!(hint_showing(&ctx, 2.2, 10, vec![], &mut build), "focused field shows no hint");
    }

    #[test]
    fn keyboard_focus_shows_the_hint_after_a_delay_but_not_over_a_menu() {
        let ctx = hint_ctx();
        let btn_id = std::cell::Cell::new(None);
        let menu = std::cell::Cell::new(false);
        let mut build = |ui: &mut Ui| {
            let (_, resp) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
            btn_id.set(Some(resp.id));
            if menu.get() {
                egui::Popup::menu(&resp).open_memory(egui::SetOpenCommand::Bool(true)).show(|ui| ui.label("menu"));
            }
            show_hint(&resp, "Bold", &[]);
        };
        hint_showing(&ctx, 1.0, 2, vec![], &mut build);
        let Some(id) = btn_id.get() else { return };
        // Tab moves the keyboard focus onto the button, as it does for a user.
        let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        // Just focused: no hint yet; after the tooltip delay: shown.
        assert!(!hint_showing(&ctx, 2.0, 2, vec![tab], &mut build));
        assert!(ctx.memory(|m| m.has_focus(id)), "Tab did not focus the button");
        assert!(hint_showing(&ctx, 2.2, 10, vec![], &mut build), "focused control shows no hint");
        // A menu open anywhere hides it.
        menu.set(true);
        assert!(!hint_showing(&ctx, 4.0, 3, vec![], &mut build), "hint drawn over an open menu");
    }

    #[test]
    fn tooltip_lines_do_not_repeat_a_shortcut_already_in_the_label() {
        assert_eq!(tooltip_lines("Paste (⌘V)", "⌘V", "", None).0, "Paste (⌘V)");
        assert_eq!(tooltip_lines("Paste (Ctrl+V)", "Ctrl+V", "", None).0, "Paste (Ctrl+V)");
        assert_eq!(tooltip_lines("Font (dialog)", "Ctrl+D", "", None).0, "Font (dialog) (Ctrl+D)");
    }

    #[test]
    fn tooltip_lines_name_shortcut_group_and_reason() {
        let (t, n) = tooltip_lines("Bold", "Ctrl+B", "Home > Font", None);
        assert_eq!(t, "Bold (Ctrl+B)");
        assert_eq!(n, vec!["Home > Font".to_string()]);
        let (t, n) = tooltip_lines("Header", "", "", Some("Open a document first"));
        assert_eq!(t, "Header");
        assert_eq!(n, vec!["Open a document first".to_string()]);
        let (_, n) = tooltip_lines("Header", "", "", Some("nothing is selected"));
        assert_eq!(n, vec!["Nothing is selected".to_string()]);
        // No reason given: the generic line.
        let (_, n) = tooltip_lines("Header", "", "", Some(""));
        assert_eq!(n, vec!["Not available right now".to_string()]);
    }

    #[test]
    fn drawn_buttons_fade_in_on_hover_and_darken_while_pressed() {
        // #305's drawn replacements for ✕, 💬, ✎ and ➕ must react like egui's buttons.
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        let clock = std::cell::Cell::new(0u32);
        let seen = std::cell::Cell::new((Rect::NOTHING, Rect::NOTHING, Color32::PLACEHOLDER, Color32::PLACEHOLDER));
        // One 60 fps frame; returns both buttons' rects and fills (framed, frameless).
        let frame = |events: Vec<egui::Event>| {
            clock.set(clock.get() + 1);
            let input = egui::RawInput { time: Some(f64::from(clock.get()) / 60.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
            ctx.run_ui(input, |ui| {
                let framed = icon_text_button(ui, "comment", "Comments", regular(12.0));
                let frameless = icon_button(ui, "close", "Close");
                let (a, b) = (drawn_state(ui, &framed, true).fill, drawn_state(ui, &frameless, false).fill);
                seen.set((framed.rect, frameless.rect, a, b));
            })
            .drop_without_applying_deltas();
            seen.get()
        };
        // Half a second of frames, so animations finish; returns the last one.
        let settle = || {
            for _ in 0..29 {
                frame(vec![]);
            }
            frame(vec![])
        };
        let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let w = ctx.global_style().visuals.widgets.clone();
        let t = Tokens::light();

        let (framed, frameless, idle, idle_bare) = settle();
        assert_eq!(idle, w.inactive.weak_bg_fill);
        assert_eq!(idle_bare, Color32::TRANSPARENT);

        // Hover fades in over a few frames rather than jumping, then settles on egui's hovered fill.
        let first = frame(vec![egui::Event::PointerMoved(framed.center())]).2;
        let between = |c: Color32, a: Color32, b: Color32| a.r().min(b.r()) < c.r() && c.r() < a.r().max(b.r());
        assert!(between(first, idle, w.hovered.weak_bg_fill), "hover should animate: {idle:?} → {first:?} → {:?}", w.hovered.weak_bg_fill);
        assert_eq!(settle().2, w.hovered.weak_bg_fill);
        // Held down: blends to the pressed (active) fill.
        frame(vec![press(framed.center(), true)]);
        assert_eq!(settle().2, w.active.weak_bg_fill);
        frame(vec![press(framed.center(), false)]);

        // The frameless close button uses the ribbon's hover and pressed tokens; the other fades out.
        frame(vec![egui::Event::PointerMoved(frameless.center())]);
        let after = settle();
        assert_eq!(after.3, t.hover);
        assert_eq!(after.2, w.inactive.weak_bg_fill);
        frame(vec![press(frameless.center(), true)]);
        assert_eq!(settle().3, t.pressed);
    }

    #[test]
    fn font_matches_puts_exact_and_prefix_before_substring() {
        let items: Vec<String> = ["Arial Black", "Arial", "Courier New", "Times New Roman", "Roman Serif"].map(String::from).to_vec();
        // Case-insensitive; the exact name first, then prefixes, then other substrings.
        assert_eq!(font_matches("ARIAL", &items), vec![1, 0]);
        assert_eq!(font_matches("roman", &items), vec![4, 3]);
        assert_eq!(font_matches(" new ", &items), vec![2, 3]);
        assert!(font_matches("zapfino", &items).is_empty());
        assert_eq!(font_matches("", &items), vec![0, 1, 2, 3, 4]);
    }

    /// Focuses a fresh combo, types `typed`, presses `keys` (one per frame) and returns its value.
    fn type_into_combo(search: bool, items: &[String], typed: &str, keys: &[egui::Key]) -> Option<String> {
        type_into(search, items, &[], typed, keys)
    }

    /// [`type_into_combo`] for a font box headed by the `recent` families.
    fn type_into(search: bool, items: &[String], recent: &[String], typed: &str, keys: &[egui::Key]) -> Option<String> {
        let ctx = egui::Context::default();
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let mut frames = vec![vec![], vec![egui::Event::Text(typed.into())]];
        frames.extend(keys.iter().map(|&k| vec![key(k)]));
        let mut out = None;
        for (n, events) in frames.into_iter().enumerate() {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                if n == 0 {
                    ui.memory_mut(|m| m.request_focus(ui.id().with(("combo_edit", "f"))));
                }
                let v = if search { font_combo_with_recent(ui, "f", 150.0, "", items, recent, None) } else { combo(ui, "f", 150.0, "", items, None) };
                out = out.take().or(v);
            })
            .drop_without_applying_deltas();
        }
        out
    }

    #[test]
    fn typing_in_the_font_box_finds_the_family() {
        // #359: typing "arial" or "roman" picks the installed family; the size box stays literal.
        use egui::Key::{ArrowDown, Enter, Escape};
        let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
        assert_eq!(type_into_combo(true, &fonts, "arial", &[Enter]).as_deref(), Some("Arial"));
        assert_eq!(type_into_combo(true, &fonts, "roman", &[Enter]).as_deref(), Some("Times New Roman"));
        assert_eq!(type_into_combo(true, &fonts, "arial", &[ArrowDown, Enter]).as_deref(), Some("Arial Black"));
        // Not installed: applied as typed, as Word allows.
        assert_eq!(type_into_combo(true, &fonts, "Zapfino", &[Enter]).as_deref(), Some("Zapfino"));
        assert_eq!(type_into_combo(true, &fonts, "arial", &[Escape]), None);
        let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
        assert_eq!(type_into_combo(false, &sizes, "1", &[Enter]).as_deref(), Some("1"));
    }

    /// #2068: with nothing typed, the recent families head the list and Enter applies the one
    /// highlighted; Down walks past them into All Fonts; a query lists its matches only.
    #[test]
    fn recent_fonts_head_the_typed_list() {
        use egui::Key::{ArrowDown, ArrowUp, Backspace, Enter};
        let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
        let recent: Vec<String> = ["Georgia", "Courier New"].map(String::from).to_vec();
        // Typing, then clearing the box, leaves the list open on the recent fonts. Enter alone
        // applies nothing the user didn't pick; Down picks the second recent font.
        assert_eq!(type_into(true, &fonts, &recent, "x", &[Backspace, Enter]).as_deref(), Some(""));
        assert_eq!(type_into(true, &fonts, &recent, "x", &[Backspace, ArrowDown, ArrowUp, Enter]).as_deref(), Some("Georgia"));
        assert_eq!(type_into(true, &fonts, &recent, "x", &[Backspace, ArrowDown, Enter]).as_deref(), Some("Courier New"));
        // Past the recent entries comes All Fonts; the headings are skipped.
        assert_eq!(type_into(true, &fonts, &recent, "x", &[Backspace, ArrowDown, ArrowDown, Enter]).as_deref(), Some("Arial"));
        // A query lists the matching installed families only.
        assert_eq!(type_into(true, &fonts, &recent, "cour", &[Enter]).as_deref(), Some("Courier New"));
        assert_eq!(type_into(true, &fonts, &recent, "arial", &[ArrowDown, Enter]).as_deref(), Some("Arial Black"));
        // Nothing recent, nothing typed and no key moved the highlight: the text is committed as before.
        assert_eq!(type_into(true, &fonts, &[], "x", &[Backspace, Enter]).as_deref(), Some(""));
    }

    /// The typed list once held one match and stayed one row tall when the query then matched
    /// many: an area keeps its last size unless a changed query sizes it again.
    #[test]
    fn the_typed_list_grows_when_the_query_matches_more() {
        let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
        let recent: Vec<String> = ["Georgia", "Courier New"].map(String::from).to_vec();
        let ctx = egui::Context::default();
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let erase = || vec![key(egui::Key::Backspace)];
        let frames = vec![vec![], vec![egui::Event::Text("cour".into())], vec![], erase(), erase(), erase(), erase(), vec![]];
        let mut area = None;
        let mut heights = Vec::new();
        for (n, events) in frames.into_iter().enumerate() {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                if n == 0 {
                    ui.memory_mut(|m| m.request_focus(ui.id().with(("combo_edit", "f"))));
                }
                area = Some(ui.id().with(("combo_search", "f")).with("list"));
                font_combo_with_recent(ui, "f", 150.0, "", &fonts, &recent, None);
            })
            .drop_without_applying_deltas();
            heights.push(area.and_then(|id| egui::AreaState::load(&ctx, id)).and_then(|s| s.size).map(|s| s.y));
        }
        let one = heights[2].expect("one match listed");
        let all = heights[7].expect("every font listed");
        assert!(all > 4.0 * one, "one match: {one} tall; two headings, two recent and four fonts: {all}");
    }

    /// The typed list was built invisibly (a sizing pass) on each keystroke that changed its
    /// length, so it blinked out for a frame. It is drawn on the keystroke's own frame.
    #[test]
    fn the_typed_list_shows_on_the_frame_of_the_keystroke() {
        fn has_text(shape: &egui::Shape, s: &str) -> bool {
            match shape {
                egui::Shape::Text(t) => t.galley.text() == s,
                egui::Shape::Vec(v) => v.iter().any(|x| has_text(x, s)),
                _ => false,
            }
        }
        let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
        let recent: Vec<String> = ["Georgia", "Courier New"].map(String::from).to_vec();
        let ctx = egui::Context::default();
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let erase = || vec![key(egui::Key::Backspace)];
        // Each keystroke and a row the list must draw on that frame: one match, the four
        // queries on the way back to a cleared box (whose list shows the recent fonts), then two
        // matches. The field itself shows only the typed text.
        let frames = [
            (vec![egui::Event::Text("cour".into())], "Courier New"),
            (erase(), "Courier New"),
            (erase(), "Courier New"),
            (erase(), "Arial Black"),
            (erase(), "Recently Used Fonts"),
            (vec![egui::Event::Text("ar".into())], "Arial Black"),
        ];
        ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.memory_mut(|m| m.request_focus(ui.id().with(("combo_edit", "f"))));
            font_combo_with_recent(ui, "f", 150.0, "", &fonts, &recent, None);
        })
        .drop_without_applying_deltas();
        for (n, (events, row)) in frames.into_iter().enumerate() {
            let out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                font_combo_with_recent(ui, "f", 150.0, "", &fonts, &recent, None);
            });
            let drawn = out.shapes.iter().any(|c| has_text(&c.shape, row));
            out.drop_without_applying_deltas();
            assert!(drawn, "keystroke {n}: {row} is drawn");
        }
    }

    /// Clearing a query with no matches must reveal the recent section, even before the list
    /// has ever had a visible viewport. A matching query is the control for a measured viewport.
    #[test]
    fn clearing_a_font_query_keeps_the_recent_heading_visible() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{By, Queryable};
        for query in ["a", "x"] {
            // A short list clamps the bad offset to zero and would hide this regression.
            let fonts: Vec<String> = (0..24).map(|n| format!("Family {n:02}")).collect();
            let recent: Vec<String> = ["Georgia", "Courier New"].map(String::from).to_vec();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 600.0)).build_ui(move |ui| {
                font_combo_with_recent(ui, "recent_ci_reduced", 150.0, "", &fonts, &recent, None);
            });
            h.run_steps(2);
            h.get(By::new().role(Role::TextInput)).focus();
            h.step();
            h.event(egui::Event::Text(query.into()));
            h.run_steps(2);
            assert!(h.query(By::new().role(Role::TextInput).value(query)).is_some());
            assert!(h.query_by_label("Recently Used Fonts").is_none());
            h.key_press(egui::Key::Backspace);
            h.run_steps(2);
            assert!(h.query(By::new().role(Role::TextInput).value("")).is_some());
            assert!(h.query_by_label("Recently Used Fonts").is_some(), "cleared query {query:?}");
            assert!(h.query_by_label("All Fonts").is_some());
        }
    }

    /// The rows of a sectioned font list, and where the keyboard highlight lands among them.
    #[test]
    fn font_list_sections_skip_their_headings() {
        let items: Vec<String> = ["Arial", "Courier New"].map(String::from).to_vec();
        let recent: Vec<String> = ["Georgia"].map(String::from).to_vec();
        fn names<'a>(items: &'a [String], recent: &'a [String], n: usize) -> Vec<String> {
            let item = |k: usize| items.get(k);
            (0..n)
                .map(|k| match section_row(k, recent, &item) {
                    Some(Row::Heading(h)) => format!("[{h}]"),
                    Some(Row::Item(i)) => i.clone(),
                    None => "-".into(),
                })
                .collect()
        }
        assert_eq!(section_rows(1, 2), 5);
        assert_eq!(names(&items, &recent, 6), ["[Recently Used Fonts]", "Georgia", "[All Fonts]", "Arial", "Courier New", "-"]);
        assert_eq!([highlight_row(0, 1), highlight_row(1, 1), highlight_row(2, 1)], [1, 3, 4]);
        // Nothing recent: no headings, the rows are the items.
        assert_eq!(section_rows(0, 2), 2);
        assert_eq!(names(&items, &[], 3), ["Arial", "Courier New", "-"]);
        assert_eq!(highlight_row(1, 0), 1);
    }

    /// The field was rounded on all four corners and the arrow on none, so the two looked apart,
    /// and their borders stood side by side as a double-width divider. The field is rounded only
    /// on the left, the arrow only on the right, and the arrow's box lies under the field by one
    /// border width, so a single line divides them; focused or not.
    #[test]
    fn combo_field_and_arrow_share_one_outline() {
        let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
        let ctx = egui::Context::default();
        let cases = [crate::theme::Appearance::Light, crate::theme::Appearance::Dark].into_iter().flat_map(|a| [(a, false), (a, true)]);
        for (appearance, focused) in cases {
            crate::theme::apply(&ctx, appearance);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                if focused {
                    ui.memory_mut(|m| m.request_focus(ui.id().with(("combo_edit", "f"))));
                }
                combo(ui, "f", 150.0, "12", &sizes, None);
            });
            // Painted boxes in paint order: (order, rect, corners, border width).
            let boxes: Vec<(usize, Rect, CornerRadius, f32)> = std::mem::take(&mut out.shapes)
                .into_iter()
                .filter_map(|c| match c.shape {
                    egui::Shape::Rect(r) => Some((r.rect, r.corner_radius, r.stroke.width)),
                    _ => None,
                })
                .enumerate()
                .map(|(i, (rect, c, w))| (i, rect, c, w))
                .collect();
            out.drop_without_applying_deltas();
            let corners: Vec<CornerRadius> = boxes.iter().map(|b| b.2).collect();
            let field = boxes.iter().find(|b| b.2.nw > 0 && b.2.sw == b.2.nw && b.2.ne == 0 && b.2.se == 0);
            let field = field.unwrap_or_else(|| panic!("focused {focused}: no field rounded on the left only in {corners:?}"));
            let arrow_corners = CornerRadius { nw: 0, sw: 0, ne: field.2.nw, se: field.2.sw };
            let arrow = boxes.iter().find(|b| b.2 == arrow_corners);
            let arrow = arrow.unwrap_or_else(|| panic!("focused {focused}: no arrow rounded on the right only in {corners:?}"));
            assert!(!corners.iter().any(|c| c.nw > 0 && c.ne > 0), "focused {focused}: a part is still rounded all round: {corners:?}");
            // One divider as wide as the other borders: the arrow's left border lies exactly under
            // the field's right border, and the field is painted over it.
            assert!(field.3 > 0.0 && arrow.3 == field.3, "focused {focused}: borders {} and {} differ", field.3, arrow.3);
            assert!(
                (arrow.1.min.x - (field.1.max.x - field.3)).abs() < 0.01,
                "focused {focused}: arrow {:?} and field {:?} borders stand apart",
                arrow.1,
                field.1
            );
            assert!(arrow.0 < field.0, "focused {focused}: the arrow's box must be painted under the field");
            assert!(
                (arrow.1.min.y - field.1.min.y).abs() < 0.01 && (arrow.1.max.y - field.1.max.y).abs() < 0.01,
                "focused {focused}: the outlines' tops and bottoms differ"
            );
        }
    }

    #[test]
    fn a_long_combo_list_builds_only_the_rows_in_view() {
        // #121: the font menu built (and so loaded the font of) every installed family.
        let items: Vec<String> = (0..5000).map(|i| format!("Family {i}")).collect();
        let ctx = egui::Context::default();
        let built = std::cell::RefCell::new(Vec::new());
        let row = |ui: &mut Ui, name: &str| {
            built.borrow_mut().push(name.to_string());
            ui.allocate_exact_size(vec2(260.0, COMBO_PREVIEW_ROW_H), Sense::click()).1
        };
        for _ in 0..3 {
            built.borrow_mut().clear();
            ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(combo_list(ui, 150.0, "", &items, Some(&row)), None);
            })
            .drop_without_applying_deltas();
            let b = built.borrow();
            assert!(!b.is_empty() && b.len() <= 420 / COMBO_PREVIEW_ROW_H as usize + 2, "built {} rows", b.len());
            assert_eq!(b[0], "Family 0");
        }
        // Without previews too.
        ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(combo_list(ui, 52.0, "8", &items, None), None);
        })
        .drop_without_applying_deltas();
    }
}
