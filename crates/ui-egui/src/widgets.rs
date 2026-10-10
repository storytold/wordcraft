//! Ribbon widgets: large and small buttons, split buttons, groups, combo fields, colour grids.

use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use serde_json::Value;

use crate::theme::{Tokens, regular};
use crate::{WordApp, icons};

/// Height of the ribbon content area (without group labels).
pub const CONTENT_H: f32 = 66.0;
pub const LABEL_H: f32 = 16.0;

/// Shortcut text for a command (`⌘B` on macOS, `Ctrl+B` elsewhere).
pub fn shortcut_text(app: &WordApp, id: &str) -> String {
    let Some(spec) = app.session.registry.get(id) else { return String::new() };
    let sc = spec.shortcut.split(" / ").next().unwrap_or("");
    if sc.is_empty() {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        sc.replace("Mod+", "⌘").replace("Shift+", "⇧").replace("Alt+", "⌥").replace("Ctrl+", "⌃")
    } else {
        sc.replace("Mod+", "Ctrl+")
    }
}

/// `label` is already in the interface language.
fn tooltip(app: &WordApp, resp: Response, label: &str, id: &str) -> Response {
    let sc = shortcut_text(app, id);
    let desc = app.session.registry.get(id).map(|s| crate::i18n::location(s.location)).unwrap_or_default();
    let enabled = app.session.registry.get(id).map(|s| (s.enabled)(&app.session).is_none()).unwrap_or(true);
    resp.on_hover_ui(|ui| {
        ui.set_max_width(260.0);
        if sc.is_empty() {
            ui.label(egui::RichText::new(label).font(crate::theme::semibold(12.0)));
        } else {
            ui.label(egui::RichText::new(format!("{label} ({sc})")).font(crate::theme::semibold(12.0)));
        }
        if !desc.is_empty() {
            ui.label(egui::RichText::new(desc).small().weak());
        }
        if !enabled {
            ui.label(egui::RichText::new(tl!("Not available right now")).small().weak());
        }
    })
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
    match app.session.registry.get(id) {
        Some(s) => (s.enabled)(&app.session).is_none(),
        // Zotero: one command at a time, and only in the desktop app.
        None if id.starts_with("ui.zotero.") => app.zotero.busy.is_none() && !cfg!(target_arch = "wasm32"),
        None => id.starts_with("ui."),
    }
}

fn bg(ui: &Ui, r: Rect, resp: &Response, checked: bool, t: &Tokens) {
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

/// A large ribbon button: 32 px icon over a (possibly two-line) label.
pub fn big(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, params: Value, menu: bool) -> Response {
    big_button(ui, app, icon, label, id, params, menu, false)
}

/// A large ribbon button for a mode that stays on (`checked` draws it pressed), run without params.
pub fn big_toggle(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, checked: bool) -> Response {
    big_button(ui, app, icon, label, id, Value::Object(Default::default()), false, checked)
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
    let ic = Rect::from_center_size(pos2(r.center().x, r.min.y + 20.0), vec2(32.0, 32.0));
    let (c, a) = if on { (t.icon, t.accent) } else { (t.text_disabled, t.text_disabled) };
    icons::paint(ui.painter(), ic, icon, c, a);
    let mut y = r.min.y + 43.0;
    let lines: Vec<&str> = label.split('\n').collect();
    for (i, l) in lines.iter().enumerate() {
        let txt = if menu && i + 1 == lines.len() { format!("{l} ▾") } else { (*l).to_string() };
        ui.painter().text(pos2(r.center().x, y), Align2::CENTER_TOP, txt, regular(11.5), if on { t.text } else { t.text_disabled });
        y += 13.0;
    }
    let resp = tooltip(app, resp, &label.replace('\n', " "), id);
    if resp.clicked() && on && !menu {
        let _ = app.run(id, params);
    }
    resp
}

/// A small button: 16 px icon with an optional label; `checked` draws the toggled state.
pub fn small(ui: &mut Ui, app: &mut WordApp, icon: &str, label: Option<&str>, tip: &str, id: &str, params: Value, checked: bool) -> Response {
    let (label, tip) = (label.map(|l| tl!(l)), tl!(tip));
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
    if resp.clicked() && on {
        let _ = app.run(id, params);
    }
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
    bg(ui, r, &resp, checked, &t);
    bg(ui, ar, &aresp, false, &t);
    let ic = Rect::from_center_size(r.center(), vec2(17.0, 17.0));
    icons::paint(ui.painter(), ic, icon, t.icon, t.accent);
    if let Some(sw) = swatch {
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.min.x + 4.0, r.max.y - 5.0), pos2(r.max.x - 4.0, r.max.y - 2.0)), 0.0, sw);
    }
    icons::paint(ui.painter(), Rect::from_center_size(ar.center(), vec2(10.0, 10.0)), "dropdown", t.icon, t.accent);
    let resp = tooltip(app, resp, tip, id);
    if resp.clicked() {
        let _ = app.run(id, params);
    }
    egui::Popup::menu(&aresp).show(|ui| {
        ui.set_min_width(180.0);
        menu(ui, app);
    });
}

/// A button that only opens a menu (icon + label + ▾).
pub fn menu_button(
    ui: &mut Ui,
    app: &mut WordApp,
    icon: &str,
    label: Option<&str>,
    tip: &str,
    big_btn: bool,
    menu: impl FnOnce(&mut Ui, &mut WordApp),
) {
    let (label, tip) = (label.map(|l| tl!(l)), tl!(tip));
    let t = Tokens::get(ui.ctx());
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
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.min.y + 20.0), vec2(32.0, 32.0)), icon, t.icon, t.accent);
        let lines: Vec<&str> = label.unwrap_or("").split('\n').collect();
        let mut y = r.min.y + 43.0;
        for (i, l) in lines.iter().enumerate() {
            let txt = if i + 1 == lines.len() { format!("{l} ▾") } else { (*l).to_string() };
            ui.painter().text(pos2(r.center().x, y), Align2::CENTER_TOP, txt, regular(11.5), t.text);
            y += 13.0;
        }
        resp
    } else {
        let text_w = label.map(|l| ui.ctx().fonts_mut(|f| f.layout_no_wrap(l.to_string(), regular(11.5), t.text).size().x) + 6.0).unwrap_or(0.0);
        let (r, resp) = ui.allocate_exact_size(vec2(24.0 + text_w + 10.0, 22.0), Sense::click());
        keytip_badge(ui, r, "menu", tip);
        bg(ui, r, &resp, false, &t);
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(17.0, 17.0)), icon, t.icon, t.accent);
        if let Some(l) = label {
            ui.painter().text(pos2(r.min.x + 24.0, r.center().y), Align2::LEFT_CENTER, l, regular(11.5), t.text);
        }
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 6.0, r.center().y), vec2(9.0, 9.0)), "dropdown", t.icon, t.accent);
        resp
    };
    let resp = resp.on_hover_text(tip);
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(200.0);
        menu(ui, app);
    });
}

/// A ribbon group: content, a centred label below and a divider on the right.
pub fn group(ui: &mut Ui, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    // The English title keys the launcher's id; the drawn title is translated.
    let id_title = title;
    let title = tl!(title);
    let t = Tokens::get(ui.ctx());
    let label_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.0), t.group_label).size().x);
    let start = ui.cursor().min;
    let inner = ui.vertical(|ui| {
        ui.set_min_width(label_w + if launcher.is_some() { 22.0 } else { 10.0 });
        ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTENT_H), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
            add(ui, app);
        });
    });
    let r = inner.response.rect;
    let r = Rect::from_min_max(start, pos2(r.max.x, start.y + CONTENT_H + LABEL_H));
    ui.painter().text(pos2(r.center().x, r.max.y - 8.0), Align2::CENTER_CENTER, title, regular(11.0), t.group_label);
    if let Some(cmd) = launcher {
        let lr = Rect::from_center_size(pos2(r.max.x - 6.0, r.max.y - 8.0), vec2(11.0, 11.0));
        let resp = ui.interact(lr, ui.id().with(("launch", id_title)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(lr.expand(2.0), 2.0, t.hover);
        }
        icons::paint(ui.painter(), lr, "launcher", t.text_dim, t.accent);
        if resp.on_hover_text(crate::i18n::fmt(tl!("{group} settings"), &[("group", title)])).clicked() {
            let _ = app.run(cmd, serde_json::json!({}));
        }
    }
    ui.add_space(5.0);
    let x = ui.cursor().min.x;
    ui.painter().line_segment([pos2(x, start.y + 4.0), pos2(x, start.y + CONTENT_H + LABEL_H - 4.0)], Stroke::new(1.0, t.border));
    ui.add_space(6.0);
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

fn swatch(ui: &mut Ui, c: wordcraft_doc::Rgb) -> bool {
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

/// A ribbon-style text field with a dropdown list. Returns a chosen/typed value.
pub fn combo(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> Option<String> {
    let mut out = None;
    let edit_id = ui.id().with(("combo_text", id));
    let mut text = ui.data_mut(|d| d.get_temp::<String>(edit_id)).unwrap_or_else(|| current.to_string());
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let te = egui::TextEdit::singleline(&mut text).desired_width(width - 16.0).font(regular(12.0)).margin(vec2(4.0, 3.0));
        let resp = ui.add(te);
        keytip_badge(ui, resp.rect, "combo", id);
        if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(edit_id, text.clone()));
        } else {
            ui.data_mut(|d| d.remove::<String>(edit_id));
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            out = Some(text.clone());
        }
        let (ar, aresp) = ui.allocate_exact_size(vec2(16.0, resp.rect.height()), Sense::click());
        ui.painter().rect(ar, 0.0, if aresp.hovered() { t.hover } else { t.input }, Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
        icons::paint(ui.painter(), Rect::from_center_size(ar.center(), vec2(9.0, 9.0)), "dropdown", t.icon, t.accent);
        egui::Popup::menu(&aresp).show(|ui| {
            if let Some(v) = combo_list(ui, width, current, items, preview) {
                out = Some(v);
                ui.close();
            }
        });
    });
    out
}

/// Height of a row drawn by a combo's preview closure (`Previews::font_preview_fn`'s entries).
pub const COMBO_PREVIEW_ROW_H: f32 = 24.0;

/// A combo's scrolling list. Only the rows in view are built: the font menu lists every installed
/// family and each preview loads that font, so building all rows loaded every font on the system
/// (gigabytes on large font collections) and froze the app (#121).
pub fn combo_list(ui: &mut Ui, width: f32, current: &str, items: &[String], preview: Option<&dyn Fn(&mut Ui, &str) -> Response>) -> Option<String> {
    let mut out = None;
    let row_h = if preview.is_some() { COMBO_PREVIEW_ROW_H } else { ui.spacing().interact_size.y };
    egui::ScrollArea::vertical().max_height(420.0).show_rows(ui, row_h, items.len(), |ui, range| {
        ui.set_min_width(width + 60.0);
        for it in items.get(range).unwrap_or_default() {
            let clicked = match preview {
                Some(p) => p(ui, it).clicked(),
                None => ui.selectable_label(it == current, it).clicked(),
            };
            if clicked {
                out = Some(it.clone());
            }
        }
    });
    out
}

/// How a drawn button looks this frame. Hover fades in over egui's animation time and the
/// pressed state blends in while the pointer is down, as on egui's own buttons. Framed buttons
/// take egui's widget visuals (fill, stroke, text colour, expansion) so they match `ui.button`;
/// frameless ones take the ribbon's hover/pressed tokens, like [`small`] and [`big`].
pub struct DrawnState {
    pub fill: Color32,
    pub stroke: Stroke,
    pub text: Color32,
    pub expansion: f32,
}

pub fn drawn_state(ui: &Ui, resp: &Response, framed: bool) -> DrawnState {
    let pressed = resp.is_pointer_button_down_on();
    let h = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered() || pressed);
    let p = ui.ctx().animate_bool_with_time(resp.id.with("press"), pressed, 0.06);
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
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let pad = ui.spacing().button_padding;
    let icon_w = 16.0;
    let size = vec2(pad.x + icon_w + 4.0 + galley.size().x + pad.x, galley.size().y.max(icon_w) + 2.0 * pad.y);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
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
    ui.painter().galley(pos2(r.min.x + pad.x + icon_w + 4.0, r.center().y - galley.size().y / 2.0), galley, st.text);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

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
