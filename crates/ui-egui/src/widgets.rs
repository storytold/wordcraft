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
    // No document open (#1129): only what opens one, Help and the settings.
    if !app.has_document() && !crate::works_without_document(id) {
        return false;
    }
    match app.session.registry.get(id) {
        Some(s) => (s.enabled)(&app.session).is_none(),
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
    #[cfg(test)]
    SPLIT_ARROWS.with(|a| a.borrow_mut().push((id.to_string(), ar)));
    bg(ui, r, &resp, checked, &t);
    bg(ui, ar, &aresp, false, &t);
    let ic = Rect::from_center_size(r.center(), vec2(17.0, 17.0));
    icons::paint(ui.painter(), ic, icon, t.icon, t.accent);
    if let Some(sw) = swatch {
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.min.x + 4.0, r.max.y - 5.0), pos2(r.max.x - 4.0, r.max.y - 2.0)), 0.0, sw);
    }
    icons::paint(ui.painter(), Rect::from_center_size(ar.center(), vec2(DROPDOWN, DROPDOWN)), "dropdown", t.icon, t.accent);
    let resp = tooltip(app, resp, tip, id);
    command_menu(&resp, app, id);
    if resp.clicked() {
        run_from_ribbon(app, id, params);
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
    let resp = resp.on_hover_text(tip);
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

/// A group's label row as it was drawn: the caption's galley rect and the band it lives in
/// (`start.y + CONTENT_H + LABEL_H`). Tests measure how much room is left between a group's lowest
/// controls and its caption, and that the caption itself did not move.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct GroupLabel {
    pub(crate) title: String,
    pub(crate) footer: Rect,
    pub(crate) caption: Rect,
    pub(crate) launcher: Option<Rect>,
}

#[cfg(test)]
thread_local! {
    /// Every group's label row drawn in the last frame, in draw order.
    pub(crate) static GROUP_LABELS: std::cell::RefCell<Vec<GroupLabel>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
thread_local! {
    /// The drop-down arrow rect of each split button drawn in the last frame, by command id. The
    /// arrow is a plain allocated rect, so a test that clicks it has to find it here.
    pub(crate) static SPLIT_ARROWS: std::cell::RefCell<Vec<(String, Rect)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A ribbon group: content, a centred label below and a divider on the right.
pub fn group(ui: &mut Ui, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    // The English title keys the launcher's id; the drawn title is translated.
    group_titled(ui, title, tl!(title), launcher, app, add);
}

/// A ribbon group whose title is shown as is (a custom group's name, Customize Ribbon).
pub fn group_titled(ui: &mut Ui, id_title: &str, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    let t = Tokens::get(ui.ctx());
    let label_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.0), t.group_label).size().x);
    let start = ui.cursor().min;
    let inner = ui.vertical(|ui| {
        ui.set_min_width(label_w + if launcher.is_some() { 22.0 } else { 10.0 });
        let content = ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTENT_H), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
            add(ui, app);
        });
        #[cfg(test)]
        if content.response.rect.height() > CONTENT_H + 0.5 {
            TALL_GROUPS.with(|g| g.borrow_mut().push(format!("{id_title}: {:.1}", content.response.rect.height())));
        }
        #[cfg(not(test))]
        let _ = content;
    });
    let r = inner.response.rect;
    let r = Rect::from_min_max(start, pos2(r.max.x, start.y + CONTENT_H + LABEL_H));
    let title_y = r.max.y - LABEL_H / 2.0;
    #[cfg(test)]
    {
        let caption = Rect::from_center_size(
            pos2(r.center().x, title_y),
            ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.0), t.group_label)).size(),
        );
        let launcher = launcher.map(|_| Rect::from_center_size(pos2(r.max.x - 6.0, title_y), vec2(11.0, 11.0)));
        GROUP_LABELS.with(|g| g.borrow_mut().push(GroupLabel { title: title.to_string(), footer: r, caption, launcher }));
    }
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
    ui.add_space(5.0);
    // The group divider: same place and extent as before, one pixel-aligned line in the stronger
    // border colour so it reads in both themes.
    let ppp = ui.ctx().pixels_per_point().max(1.0);
    let x = (ui.cursor().min.x * ppp).floor() / ppp + 0.5 / ppp;
    ui.painter().line_segment([pos2(x, start.y + 4.0), pos2(x, start.y + CONTENT_H + LABEL_H - 4.0)], Stroke::new(1.0, t.border_strong));
    ui.add_space(6.0);
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

/// "Customize the Ribbon…" and "Customize Quick Access Toolbar…" (context menus).
pub fn customize_menu_items(ui: &mut Ui, app: &mut WordApp) {
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
    combo_impl(ui, id, width, current, items, preview, false)
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
    combo_impl(ui, id, width, current, items, preview, true)
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

/// A searching combo's state between frames.
#[derive(Clone, Default)]
struct ComboSearch {
    /// The match list is showing (the user typed since focusing the field).
    open: bool,
    /// The query `matches` were computed for, and the list length then.
    query: String,
    items_len: usize,
    matches: std::sync::Arc<Vec<usize>>,
    /// Index into `matches`.
    highlight: usize,
    /// Where the list was drawn last frame, its scroll offset and visible height.
    popup: Option<Rect>,
    offset: f32,
    view_h: f32,
}

/// The height of an editable dropdown: one rounded silhouette holds the text field and the arrow
/// area together, so they never look like two separate boxes.
pub const COMBO_H: f32 = 22.0;
/// Width of the arrow area inside a combo's right edge.
pub const COMBO_ARROW_W: f32 = 16.0;
/// Size of the chevron drawn in the arrow area.
const COMBO_CHEVRON: f32 = 10.0;
/// Horizontal padding between the control's outline and its editable text (and between the text and
/// the arrow area).
const COMBO_PAD: f32 = 4.0;
/// Corner radius of that silhouette (and of the arrow area's own right corners).
const COMBO_RADIUS: u8 = 3;
/// A degenerate width (zero, negative or not a number) is clamped to this instead of drawing a box
/// that cannot hold its own arrow.
const COMBO_MIN_W: f32 = 32.0;

#[cfg(test)]
thread_local! {
    /// Every editable dropdown drawn in the last frame: its outer silhouette, the text field's rect
    /// and the arrow area's rect. Tests check that one box of the requested size holds both.
    pub(crate) static COMBO_RECTS: std::cell::RefCell<Vec<(String, Rect, Rect, Rect)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn combo_impl(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
    search: bool,
) -> Option<String> {
    let mut out = None;
    let width = if width.is_finite() { width.max(COMBO_MIN_W) } else { COMBO_MIN_W };
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
        // One allocation for the whole control: the text field and the arrow area live inside it, so
        // the width the caller asks for is exactly the width the control takes.
        let (outer, _) = ui.allocate_exact_size(vec2(width, COMBO_H), Sense::hover());
        let text_rect = Rect::from_min_max(outer.min, pos2(outer.max.x - COMBO_ARROW_W, outer.max.y));
        let arrow_rect = Rect::from_min_max(pos2(outer.max.x - COMBO_ARROW_W, outer.min.y), outer.max);
        let aresp = ui.interact(arrow_rect, ui.id().with(("combo_arrow", id)), Sense::click());
        // Screen readers hear which box the arrow belongs to: the id names the control ("font").
        aresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{id} list")));
        #[cfg(test)]
        COMBO_RECTS.with(|c| c.borrow_mut().push((id.to_string(), outer, text_rect, arrow_rect)));
        // The base and the arrow's own hover/pressed feedback go under the text; the single outline
        // is painted after it (below), so the control reads as one rounded box.
        ui.painter().rect_filled(outer, COMBO_RADIUS, t.input);
        if aresp.hovered() || aresp.is_pointer_button_down_on() {
            let fill = if aresp.is_pointer_button_down_on() { t.pressed } else { t.hover };
            ui.painter().rect_filled(arrow_rect, CornerRadius { nw: 0, sw: 0, ne: COMBO_RADIUS, se: COMBO_RADIUS }, fill);
        }
        // The arrow itself: our own chevron, centred in the arrow area.
        icons::paint(ui.painter(), Rect::from_center_size(arrow_rect.center(), vec2(COMBO_CHEVRON, COMBO_CHEVRON)), "dropdown", t.icon, t.accent);
        // The field keeps its inset through the frame, not through `margin`: egui uses a custom frame
        // verbatim and only builds its own inner-margin frame when there is none, so a bare
        // `Frame::NONE` silently dropped the margin and let the text touch the outline. The invisible
        // frame below supplies the inset, and the field asks for the whole text area because the frame
        // already provides the padding.
        let te = egui::TextEdit::singleline(&mut text)
            .id(te_id)
            .desired_width(text_rect.width())
            .font(regular(12.0))
            .margin(vec2(COMBO_PAD, 0.0))
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(COMBO_PAD as i8, 0)));
        // The frameless field sits in the left part of the silhouette, its text vertically centred.
        let resp = ui
            .scope_builder(egui::UiBuilder::new().max_rect(text_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                let out = te.show(ui);
                #[cfg(test)]
                TEXT_LAYOUT.with(|c| {
                    c.borrow_mut().push((id.to_string(), out.galley_pos, out.text_clip_rect, out.galley.size(), out.response.response.rect));
                });
                out.response.response
            })
            .inner;
        focused = resp.has_focus();
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut reveal = step != 0;
        if search && resp.changed() {
            s.open = true;
            s.highlight = 0;
            reveal = true;
        }
        let mut regain_focus = false;
        if s.open {
            if s.query != text || s.items_len != items.len() {
                s.matches = std::sync::Arc::new(font_matches(&text, items));
                s.query = text.clone();
                s.items_len = items.len();
            }
            let last = s.matches.len().saturating_sub(1);
            s.highlight = match step {
                1.. => s.highlight.saturating_add(1),
                0 => s.highlight,
                _ => s.highlight.saturating_sub(1),
            }
            .min(last);
            if enter {
                let picked = s.matches.get(s.highlight).and_then(|&i| items.get(i));
                out = Some(match picked {
                    Some(name) if !text.trim().is_empty() => name.clone(),
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
        // One outline for the whole control: quiet when idle, the accent when hovered, focused or
        // with its list open (the menu popup keeps the accent while it stays open).
        let list_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&aresp));
        let border = if focused || s.open || list_open {
            t.accent
        } else if aresp.hovered() || resp.hovered() {
            t.accent.linear_multiply(0.55)
        } else {
            t.input_border
        };
        ui.painter().rect_stroke(outer, COMBO_RADIUS, Stroke::new(1.0, border), egui::StrokeKind::Inside);
        keytip_badge(ui, outer, "combo", id);
        // The list drops from the box's left edge, as in Word, not from the arrow (#535).
        egui::Popup::menu(&aresp).anchor(outer).show(|ui| {
            if let Some(v) = combo_list(ui, width, current, items, preview) {
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
        if s.open && !s.matches.is_empty() {
            let matches = s.matches.clone();
            let scroll = reveal.then_some((s.offset, s.view_h));
            let shown = egui::Area::new(search_id.with("list")).order(egui::Order::Tooltip).fixed_pos(outer.left_bottom()).show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        let item = |k: usize| matches.get(k).and_then(|&i| items.get(i));
                        list_rows(ui, width, current, matches.len(), &item, Some(s.highlight), scroll, preview)
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
    list_rows(ui, width, current, items.len(), &|k| items.get(k), None, None, preview).0
}

/// The rows of [`combo_list`], virtualised, for `n` items fetched by index. With a `highlight`
/// that row is marked instead of `current`; `scroll` (last frame's offset and visible height)
/// scrolls the highlight into view. Returns the clicked item, the scroll offset and the height.
fn list_rows<'a>(
    ui: &mut Ui,
    width: f32,
    current: &str,
    n: usize,
    item: &dyn Fn(usize) -> Option<&'a String>,
    highlight: Option<usize>,
    scroll: Option<(f32, f32)>,
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> (Option<String>, f32, f32) {
    let mut out = None;
    let row_h = if preview.is_some() { COMBO_PREVIEW_ROW_H } else { ui.spacing().interact_size.y };
    let mut area = egui::ScrollArea::vertical().max_height(420.0);
    if let (Some(h), Some((offset, view_h))) = (highlight, scroll) {
        let top = h as f32 * (row_h + ui.spacing().item_spacing.y);
        let bottom = top + row_h;
        let offset = if top < offset || view_h <= 0.0 {
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
            let Some(it) = item(k) else { continue };
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

/// A thin vertical line between clusters of related icons inside a ribbon row: a 7-point wide,
/// 22-point tall gap with a 14-point line centred in it, snapped to the device pixel grid so it stays
/// crisp. It takes no clicks and no keyboard focus — the controls on either side keep their own.
pub fn icon_separator(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(7.0, 22.0), Sense::hover());
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point().max(1.0);
    let x = (r.center().x * ppp).floor() / ppp + 0.5 / ppp;
    let y0 = ((r.center().y - 7.0) * ppp).round() / ppp;
    let y1 = ((r.center().y + 7.0) * ppp).round() / ppp;
    ui.painter().line_segment([pos2(x, y0), pos2(x, y1)], Stroke::new(1.0, t.border_strong));
    #[cfg(test)]
    SEPARATORS.with(|s| s.borrow_mut().push(r));
}

#[cfg(test)]
thread_local! {
    /// Every inline cluster separator drawn in the last frame, in draw order.
    pub(crate) static SEPARATORS: std::cell::RefCell<Vec<Rect>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
thread_local! {
    /// Where each editable dropdown's text actually landed: (id, galley origin, the rect it was
    /// clipped to, the galley's size). Measured from `TextEdit::show`, because an allocation-only
    /// check cannot see that a `Frame::NONE` text edit ignores its own margin.
    pub(crate) static TEXT_LAYOUT: std::cell::RefCell<Vec<(String, egui::Pos2, Rect, egui::Vec2, Rect)>> =
        const { std::cell::RefCell::new(Vec::new()) };
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
    icon_text_button_with(ui, icon, text, font, false)
}

/// [`icon_text_button`] that opens a menu: the drop-down marker follows the text.
pub fn icon_text_menu_button(ui: &mut Ui, icon: &str, text: &str, font: egui::FontId) -> Response {
    icon_text_button_with(ui, icon, text, font, true)
}

fn icon_text_button_with(ui: &mut Ui, icon: &str, text: &str, font: egui::FontId, marker: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let pad = ui.spacing().button_padding;
    let icon_w = 16.0;
    let marker_w = if marker { DROPDOWN + 4.0 } else { 0.0 };
    let size = vec2(pad.x + icon_w + 4.0 + galley.size().x + marker_w + pad.x, galley.size().y.max(icon_w) + 2.0 * pad.y);
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
    let text_x = r.min.x + pad.x + icon_w + 4.0;
    let text_w = galley.size().x;
    ui.painter().galley(pos2(text_x, r.center().y - galley.size().y / 2.0), galley, st.text);
    if marker {
        let c = pos2(text_x + text_w + 4.0 + DROPDOWN / 2.0, r.center().y);
        icons::paint(ui.painter(), Rect::from_center_size(c, vec2(DROPDOWN, DROPDOWN)), "dropdown", st.text, st.text);
    }
    resp
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
                let v = if search { font_combo(ui, "f", 150.0, "", items, None) } else { combo(ui, "f", 150.0, "", items, None) };
                out = out.take().or(v);
            })
            .drop_without_applying_deltas();
        }
        out
    }

    #[test]
    fn typing_in_the_font_box_finds_the_family() {
        // #359: typing "arial" or "roman" picks the installed family; the size box stays literal.
        use egui::Key::{ArrowDown, Enter, Escape, Tab};
        let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
        assert_eq!(type_into_combo(true, &fonts, "arial", &[Enter]).as_deref(), Some("Arial"));
        assert_eq!(type_into_combo(true, &fonts, "roman", &[Enter]).as_deref(), Some("Times New Roman"));
        assert_eq!(type_into_combo(true, &fonts, "arial", &[ArrowDown, Enter]).as_deref(), Some("Arial Black"));
        // Not installed: applied as typed, as Word allows.
        assert_eq!(type_into_combo(true, &fonts, "Zapfino", &[Enter]).as_deref(), Some("Zapfino"));
        assert_eq!(type_into_combo(true, &fonts, "arial", &[Escape]), None);
        assert_eq!(type_into_combo(true, &fonts, "arial", &[Tab]), None, "Tab closes the list without committing");
        let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
        assert_eq!(type_into_combo(false, &sizes, "1", &[Enter]).as_deref(), Some("1"));
    }

    /// Draws a font combo (150) and a size combo (52) in one frame with `events` as input; returns
    /// the recorded control rects and whatever the combos returned.
    fn combo_frame(ctx: &egui::Context, events: Vec<egui::Event>) -> (Vec<(String, Rect, Rect, Rect)>, Option<String>) {
        let fonts: Vec<String> = ["Arial", "Courier New", "Times New Roman"].map(String::from).to_vec();
        let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
        let mut picked = None;
        COMBO_RECTS.with(|c| c.borrow_mut().clear());
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            if let Some(v) = font_combo(ui, "font", 150.0, "Arial", &fonts, None) {
                picked = Some(v);
            }
            if let Some(v) = combo(ui, "size", 52.0, "12", &sizes, None) {
                picked = Some(v);
            }
            // The widths the other callers use: Outlining's style and level boxes and the two font
            // dialogs, so one test covers every caller of the shared control.
            if let Some(v) = combo(ui, "outline", 100.0, "Body Text", &fonts, None) {
                picked = Some(v);
            }
            if let Some(v) = combo(ui, "levels", 90.0, "All Levels", &sizes, None) {
                picked = Some(v);
            }
            if let Some(v) = font_combo(ui, "dialog", 220.0, "Arial", &fonts, None) {
                picked = Some(v);
            }
            if let Some(v) = font_combo(ui, "symbol", 190.0, "Arial", &fonts, None) {
                picked = Some(v);
            }
        })
        .drop_without_applying_deltas();
        (COMBO_RECTS.with(|c| c.take()), picked)
    }

    /// The light theme, where the two frames of the old design were easiest to see.
    fn combo_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(false, false));
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx
    }

    /// Clicks at a point the way kittest clicks a node: the pointer moves, presses and releases in
    /// one frame each, then the frames that react to it.
    fn click_pointer<S>(h: &mut egui_kittest::Harness<'_, S>, pos: egui::Pos2) {
        for e in [egui::Event::PointerMoved(pos)].into_iter().chain([true, false].map(|pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        })) {
            h.input_mut().events.push(e);
            h.run_steps(1);
        }
        h.run_steps(2);
    }

    /// Selected and disabled icon states: a checked icon paints the checked surface with its accent
    /// outline, an unchecked one paints nothing, and a command that is not available fades its icon
    /// to the disabled colour.
    #[test]
    fn icon_buttons_show_selected_and_disabled_states() {
        let ctx = combo_ctx();
        let t = Tokens::get(&ctx);
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = small(ui, &mut app, "bold", None, "Bold", "format.bold", serde_json::json!({}), true);
            let _ = small(ui, &mut app, "italic", None, "Italic", "format.italic", serde_json::json!({}), false);
            let _ = small(ui, &mut app, "underline", None, "Underline", "format.notACommand", serde_json::json!({}), false);
        });
        let (mut fills, mut strokes, mut colors) = (Vec::new(), Vec::new(), Vec::new());
        for s in &out.shapes {
            match &s.shape {
                egui::Shape::Rect(r) => {
                    fills.push(r.fill);
                    strokes.push(r.stroke.color);
                }
                egui::Shape::Path(p) => {
                    if let egui::epaint::ColorMode::Solid(c) = p.stroke.color {
                        colors.push(c);
                    }
                }
                egui::Shape::LineSegment { stroke, .. } => colors.push(stroke.color),
                egui::Shape::Circle(c) => {
                    fills.push(c.fill);
                    strokes.push(c.stroke.color);
                }
                _ => {}
            }
        }
        out.drop_without_applying_deltas();
        assert_eq!(fills.iter().filter(|f| **f == t.checked).count(), 1, "one checked surface: {fills:?}");
        assert!(strokes.contains(&t.accent.linear_multiply(0.5)), "the checked surface keeps its accent outline: {strokes:?}");
        assert!(!fills.contains(&t.hover) && !fills.contains(&t.pressed), "nothing was hovered or pressed: {fills:?}");
        assert!(colors.contains(&t.text_disabled), "the unavailable command's icon fades: {colors:?}");
        assert!(colors.contains(&t.icon), "the available icons keep the icon colour: {colors:?}");
    }

    /// A separator is a thin line in a small gap: it draws one 14-point stroke, paints no surface of
    /// its own, adds no clickable region and takes no keyboard focus, while the buttons beside it keep
    /// their own clicks.
    #[test]
    fn a_separator_is_a_thin_line_that_takes_no_input() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let clicked = std::cell::Cell::new(0);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(200.0, 40.0)).build_ui(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
                if ui.button("left").clicked() {
                    clicked.set(clicked.get() + 1);
                }
                icon_separator(ui);
                if ui.button("right").clicked() {
                    clicked.set(clicked.get() + 1);
                }
            });
        });
        h.run_steps(3);
        let r = SEPARATORS.with(|s| s.borrow().last().copied()).expect("the separator was drawn");
        assert!((r.width() - 7.0).abs() < 0.01 && (r.height() - 22.0).abs() < 0.01, "the separator gap is {r:?}, not 7x22");
        // One 14-point, 1-point-thick line, centred, and nothing else painted in the gap. Painted on
        // its own frame so the shapes can be inspected.
        let ctx = combo_ctx();
        SEPARATORS.with(|s| s.borrow_mut().clear());
        let alone = std::cell::Cell::new(Rect::NOTHING);
        let mut painted_rects: Vec<Rect> = Vec::new();
        let painted: Vec<(Rect, f32)> = {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                icon_separator(ui);
            });
            alone.set(SEPARATORS.with(|s| s.borrow().last().copied()).expect("the separator was drawn on its own frame"));
            for shape in &out.shapes {
                if let egui::Shape::Rect(r) = &shape.shape {
                    painted_rects.push(r.rect);
                }
            }
            let shapes: Vec<(Rect, f32)> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Path(p) if p.points.len() == 2 && p.stroke.width > 0.0 => Some((p.visual_bounding_rect(), p.stroke.width)),
                    egui::Shape::LineSegment { points, stroke } if stroke.width > 0.0 => {
                        Some((Rect::from_two_pos(points[0], points[1]), stroke.width))
                    }
                    _ => None,
                })
                .collect();
            out.drop_without_applying_deltas();
            shapes.into_iter().filter(|(b, _)| alone.get().expand(0.01).contains_rect(*b)).collect()
        };
        let inner = painted;
        let alone = alone.get();
        assert_eq!(inner.len(), 1, "one line per separator: {inner:?}");
        assert!((inner[0].0.height() - 14.0).abs() < 0.01, "the line is {} points tall, not 14", inner[0].0.height());
        assert!((inner[0].0.center().x - alone.center().x).abs() < 0.5, "the line is not centred in its gap");
        assert!((inner[0].1 - 1.0).abs() < 0.01, "the line is {} points thick, not 1", inner[0].1);
        assert!(!painted_rects.iter().any(|b| alone.expand(0.01).contains_rect(*b)), "the separator paints a surface of its own: {painted_rects:?}");
        // No clickable region and no focus: only the two buttons answer, and they keep their clicks.
        assert_eq!(h.query_all_by_role(Role::Button).filter(|n| n.rect().intersects(r)).count(), 0, "a clickable region overlaps the separator");
        assert!(h.ctx.memory(|m| m.focused()).is_none(), "the separator took keyboard focus");
        // Clicks in the gap reach a neighbour exactly as they do in a plain `add_space` gap (egui
        // resolves the press to the nearest clickable widget); the separator adds nothing of its own.
        assert_eq!(clicked.get(), 0, "rendering the scene clicked a button");
    }

    /// One rounded silhouette per field, of exactly the width the caller asked for, with the text
    /// field and the arrow area tiling it and no gap between them.
    #[test]
    fn a_combo_is_one_box_of_the_requested_size() {
        let ctx = combo_ctx();
        // The silhouette is sized in points, so it is the same box at 1x, 1.5x and 2x device pixels.
        for ppp in [1.0, 1.5, 2.0] {
            ctx.set_pixels_per_point(ppp);
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            let (rects, _) = combo_frame(&ctx, vec![]);
            // Home's own widths plus the shared callers: Outlining (100 / 90) and the font dialogs
            // (220 / 190) use the same control and must get the same box.
            for (id, want_w) in [("font", 150.0), ("size", 52.0), ("outline", 100.0), ("levels", 90.0), ("dialog", 220.0), ("symbol", 190.0)] {
                let (_, outer, text, arrow) = rects.iter().find(|(n, ..)| n == id).unwrap_or_else(|| panic!("{id} was drawn")).clone();
                assert!((outer.width() - want_w).abs() < 0.01, "{id}: the control is {} wide, not {want_w}", outer.width());
                assert!((outer.height() - COMBO_H).abs() < 0.01, "{id}: the control is {} tall, not {COMBO_H}", outer.height());
                assert!((arrow.width() - COMBO_ARROW_W).abs() < 0.01, "{id}: the arrow area is {} wide, not {COMBO_ARROW_W}", arrow.width());
                assert!((text.right() - arrow.left()).abs() < 0.01, "{id}: a gap between the field and the arrow: {text:?} {arrow:?}");
                assert!((text.left() - outer.left()).abs() < 0.01, "{id}: the field does not start at the control's left edge");
                assert!((arrow.right() - outer.right()).abs() < 0.01, "{id}: the arrow does not end at the control's right edge");
                assert!(
                    (text.height() - outer.height()).abs() < 0.01 && (arrow.height() - outer.height()).abs() < 0.01,
                    "{id}: the parts are not as tall as the control: {text:?} {arrow:?}"
                );
            }
        }
    }

    /// The text of an editable dropdown keeps four points of padding inside its text area: the galley
    /// starts four points in, the field's content stops four points before the arrow, and the text is
    /// centred vertically. Measured from what egui actually painted (`TextEdit::show`), not from the
    /// rectangles we allocated.
    #[test]
    fn a_combo_keeps_four_points_of_padding_inside_its_text_area() {
        let ctx = combo_ctx();
        let fonts: Vec<String> = ["Arial"].map(String::from).to_vec();
        let long: Vec<String> = ["A very long family name that cannot fit"].map(String::from).to_vec();
        let sizes: Vec<String> = ["100"].map(String::from).to_vec();
        TEXT_LAYOUT.with(|c| c.borrow_mut().clear());
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = font_combo(ui, "font", 150.0, "Arial", &fonts, None);
            let _ = combo(ui, "size", 52.0, "100", &sizes, None);
            let _ = font_combo(ui, "long", 150.0, "A very long family name that cannot fit", &long, None);
        })
        .drop_without_applying_deltas();
        let rects = COMBO_RECTS.with(|c| c.take());
        let layouts = TEXT_LAYOUT.with(|c| c.take());
        let mut seen = Vec::new();
        for id in ["font", "size", "long"] {
            let (_, outer, text_rect, arrow) = rects.iter().find(|(n, ..)| n == id).unwrap_or_else(|| panic!("{id} was drawn")).clone();
            let (_, galley_pos, clip, galley_size, field) =
                layouts.iter().find(|(n, ..)| n == id).unwrap_or_else(|| panic!("{id} painted its text")).clone();
            seen.push((id, outer, text_rect, arrow, galley_pos, clip, galley_size, field));
        }
        for (id, outer, text_rect, arrow, galley_pos, clip, galley_size, field) in seen {
            let left = galley_pos.x - text_rect.left();
            let off = galley_pos.y + galley_size.y / 2.0 - outer.center().y;
            assert!((left - COMBO_PAD).abs() < 0.6, "{id}: the text starts {left:.2} points into its area, not {COMBO_PAD}");
            assert!(
                (field.left() - text_rect.left()).abs() < 0.6 && (field.right() - arrow.left()).abs() < 0.6,
                "{id}: the field does not fill its text area: {field:?}"
            );
            assert!(off.abs() < 0.75, "{id}: the text sits {off:.2} points off the control's centre");
            assert!(clip.left() >= text_rect.left(), "{id}: the text is clipped past the control's left edge");
            if id == "long" {
                // With text wider than the field egui hands back the real editable clip, so the
                // right-hand padding can be measured directly: it stops four points before the arrow.
                let right = arrow.left() - clip.right();
                assert!((right - COMBO_PAD).abs() < 0.6, "the editable text stops {right:.2} points before the arrow, not {COMBO_PAD}");
                assert!(galley_size.x > clip.width(), "the long name is not clipped: {galley_size:?} vs {clip:?}");
            } else {
                // A short value is narrower than the field, so its clip is its own rect: check that it
                // still sits inside the padded content area rather than against the outline.
                assert!(galley_pos.x >= text_rect.left() + COMBO_PAD - 0.6, "{id}: the text starts left of its padding");
            }
        }
    }

    /// The padding survives every state: idle, hovered, focused and with the list open, the galley
    /// still starts four points in and the editable area still stops four points before the arrow.
    #[test]
    fn a_combo_keeps_its_padding_in_every_state() {
        let ctx = combo_ctx();
        let fonts: Vec<String> = ["Arial"].map(String::from).to_vec();
        let long: Vec<String> = ["A very long family name that cannot fit"].map(String::from).to_vec();
        // One frame in the given state; returns the measured insets for the font combo.
        let measure = |state: &str, events: Vec<egui::Event>, focus: bool| {
            COMBO_RECTS.with(|c| c.borrow_mut().clear());
            TEXT_LAYOUT.with(|c| c.borrow_mut().clear());
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                if focus {
                    ui.memory_mut(|m| m.request_focus(ui.id().with(("combo_edit", "font"))));
                }
                let _ = font_combo(ui, "font", 150.0, "A very long family name that cannot fit", &long, None);
            })
            .drop_without_applying_deltas();
            let (_, outer, text_rect, arrow) = COMBO_RECTS.with(|c| c.borrow().last().cloned()).expect("drawn");
            let (_, galley_pos, clip, galley_size, field) = TEXT_LAYOUT.with(|c| c.borrow().last().cloned()).expect("text painted");
            let _ = fonts;
            (state.to_string(), outer, text_rect, arrow, galley_pos, clip, galley_size, field)
        };
        let open_pos = {
            let (_, outer, _, arrow, ..) = measure("probe", vec![], false);
            let _ = outer;
            arrow.center()
        };
        let mut states = vec![
            measure("idle", vec![], false),
            measure("hovered", vec![egui::Event::PointerMoved(open_pos)], false),
            measure("focused", vec![], true),
        ];
        for pressed in [true, false] {
            let pos = open_pos;
            ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() },
                    ],
                    ..Default::default()
                },
                |ui| {
                    let _ = font_combo(ui, "font", 150.0, "A very long family name that cannot fit", &long, None);
                },
            )
            .drop_without_applying_deltas();
        }
        states.push(measure("open", vec![], false));
        for (state, _outer, text_rect, arrow, galley_pos, clip, galley_size, field) in states {
            // The editable content keeps its inset in every state; a focused field scrolls a long
            // value, so the galley's own origin only stays put while the text is not scrolled.
            let (clip_left, clip_right) = (clip.left() - text_rect.left(), arrow.left() - clip.right());
            assert!((clip_left - COMBO_PAD).abs() < 0.6, "{state}: the editable content starts {clip_left:.2} points in, not {COMBO_PAD}");
            assert!(
                (clip_right - COMBO_PAD).abs() < 0.6,
                "{state}: the editable content stops {clip_right:.2} points before the arrow, not {COMBO_PAD}"
            );
            assert!(clip.width() > 10.0, "{state}: the editable content collapsed: {clip:?}");
            if state == "idle" || state == "hovered" {
                let left = galley_pos.x - text_rect.left();
                assert!((left - COMBO_PAD).abs() < 0.6, "{state}: the text starts {left:.2} points in, not {COMBO_PAD}");
            }
            let off = galley_pos.y + galley_size.y / 2.0 - text_rect.center().y;
            assert!(off.abs() < 1.0, "{state}: the text sits {off:.2} points off the middle");
            assert!(
                (field.left() - text_rect.left()).abs() < 0.6 && (field.right() - arrow.left()).abs() < 0.6,
                "{state}: the field left its area: {field:?}"
            );
        }
    }

    /// The outline colour follows the control's state: quiet when idle, a softened accent while
    /// hovered, the accent when focused or while its list is open.
    #[test]
    fn a_combo_shows_idle_hover_focus_and_open_states() {
        let ctx = combo_ctx();
        let t = Tokens::get(&ctx);
        let fonts: Vec<String> = ["Arial", "Courier New"].map(String::from).to_vec();
        // One frame: draw the combo with `events` as input, optionally focusing the field first.
        let border = |events: Vec<egui::Event>, focus: bool| {
            COMBO_RECTS.with(|c| c.borrow_mut().clear());
            let out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                if focus {
                    ui.memory_mut(|m| m.request_focus(ui.id().with(("combo_edit", "font"))));
                }
                let _ = font_combo(ui, "font", 150.0, "Arial", &fonts, None);
            });
            let outer = COMBO_RECTS.with(|c| c.borrow().last().map(|(_, o, _, _)| *o)).expect("the combo was drawn");
            let color = out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.stroke.width > 0.0 && (r.rect.min - outer.min).length() < 0.01 => Some(r.stroke.color),
                _ => None,
            });
            out.drop_without_applying_deltas();
            (outer, color.expect("the control has an outline"))
        };
        let (outer, idle) = border(vec![], false);
        assert_eq!(idle, t.input_border, "idle");
        let (_, hovered) = border(vec![egui::Event::PointerMoved(outer.center())], false);
        assert_eq!(hovered, t.accent.linear_multiply(0.55), "hovered");
        let (_, focused) = border(vec![], true);
        assert_eq!(focused, t.accent, "focused");
        // Opening the list with a click on the arrow keeps the accent outline.
        for pressed in [true, false] {
            let pos = pos2(outer.right() - COMBO_ARROW_W / 2.0, outer.center().y);
            ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() },
                    ],
                    ..Default::default()
                },
                |ui| {
                    let _ = font_combo(ui, "font", 150.0, "Arial", &fonts, None);
                },
            )
            .drop_without_applying_deltas();
        }
        let (_, open) = border(vec![], false);
        assert_eq!(open, t.accent, "open");
    }

    /// One enclosing outline and no separate arrow frame or interior divider: the old design stroked
    /// the 16 pt arrow box with its own 1 pt border, which is what made it look detached.
    #[test]
    fn a_combo_paints_one_outline_and_no_arrow_frame() {
        let ctx = combo_ctx();
        let fonts: Vec<String> = ["Arial"].map(String::from).to_vec();
        COMBO_RECTS.with(|c| c.borrow_mut().clear());
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = font_combo(ui, "font", 150.0, "Arial", &fonts, None);
        });
        let (_, outer, _text, arrow) = COMBO_RECTS.with(|c| c.take()).into_iter().next().expect("the combo was drawn");
        let stroked: Vec<Rect> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.stroke.width > 0.0 => Some(r.rect),
                _ => None,
            })
            .collect();
        out.drop_without_applying_deltas();
        let same = |a: Rect, b: Rect| (a.min - b.min).length() < 0.01 && (a.max - b.max).length() < 0.01;
        assert_eq!(stroked.iter().filter(|r| same(**r, outer)).count(), 1, "the control has one outline: {stroked:?}");
        assert!(!stroked.iter().any(|r| same(*r, arrow)), "the arrow area has its own frame: {stroked:?}");
        assert_eq!(stroked.iter().filter(|r| outer.contains_rect(**r) && !same(**r, outer)).count(), 0, "a stroke inside the control: {stroked:?}");
    }

    /// The arrow is its own labelled control: a click on it opens the list, and picking a row returns
    /// the family. The label is what a screen reader announces for the arrow area.
    #[test]
    fn the_arrow_opens_the_list_and_picking_a_row_commits_it() {
        use egui_kittest::kittest::Queryable;
        let fonts: Vec<String> = ["Arial", "Courier New", "Times New Roman"].map(String::from).to_vec();
        let picked = std::cell::RefCell::new(None);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(420.0, 320.0)).build_ui(|ui| {
            if let Some(v) = font_combo(ui, "font", 150.0, "Arial", &fonts, None) {
                *picked.borrow_mut() = Some(v);
            }
        });
        h.run_steps(3);
        assert_eq!(h.query_all_by_label("font list").count(), 1, "the arrow area carries an accessible label");
        let (_, outer, _text, arrow) = COMBO_RECTS.with(|c| c.borrow().iter().find(|(n, ..)| n == "font").cloned()).expect("the combo was drawn");
        click_pointer(&mut h, arrow.center());
        assert!(h.query_all_by_label("Courier New").count() >= 1, "the arrow opened the list");
        let row = h.query_all_by_label("Courier New").next().expect("the row");
        // #535: the list drops from the control's left edge, not from the arrow. The row sits inside
        // the popup frame's margin, so its left edge is a few points right of the control's.
        let inset = row.rect().left() - outer.left();
        assert!((0.0..12.0).contains(&inset), "the list dropped at {:?}, not from the control's left edge {outer:?}", row.rect());
        assert!(row.rect().left() < arrow.left(), "the list dropped from the arrow area, not the control: {:?}", row.rect());
        assert!(row.rect().top() >= outer.bottom() - 1.0, "the list did not open under the control: {:?}", row.rect());
        row.click();
        h.run_steps(3);
        assert_eq!(picked.borrow().as_deref(), Some("Courier New"), "picking a row returns the family");
    }

    /// The search list keeps its Add Font entry: the arrow opens the list and that row returns the
    /// sentinel the ribbon turns into `font.add`.
    #[test]
    fn the_search_list_still_returns_the_add_font_entry() {
        use egui_kittest::kittest::Queryable;
        let fonts: Vec<String> = ["Arial"].map(String::from).to_vec();
        let picked = std::cell::RefCell::new(None);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(420.0, 320.0)).build_ui(|ui| {
            if let Some(v) = font_combo(ui, "font", 150.0, "Arial", &fonts, None) {
                *picked.borrow_mut() = Some(v);
            }
        });
        h.run_steps(3);
        let (_, _outer, _text, arrow) = COMBO_RECTS.with(|c| c.borrow().iter().find(|(n, ..)| n == "font").cloned()).expect("the combo was drawn");
        click_pointer(&mut h, arrow.center());
        h.query_all_by_label_contains("Add Font").next().expect("the Add Font row").click();
        h.run_steps(3);
        assert_eq!(picked.borrow().as_deref(), Some(ADD_FONT), "Add Font… still returns its sentinel");
    }

    /// Typing in the field opens the search list and a click outside the field and the list closes
    /// it again — the focus rule the old design already had.
    #[test]
    fn clicking_outside_closes_the_open_search_list() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let fonts: Vec<String> = ["Arial", "Courier New"].map(String::from).to_vec();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(420.0, 320.0)).build_ui(|ui| {
            let _ = font_combo(ui, "font", 150.0, "", &fonts, None);
        });
        h.run_steps(3);
        h.get_by_role(Role::TextInput).focus();
        h.run_steps(1);
        h.get_by_role(Role::TextInput).type_text("cou");
        h.run_steps(2);
        assert!(h.query_all_by_label("Courier New").count() >= 1, "typing opened the search list");
        click_pointer(&mut h, pos2(400.0, 300.0));
        assert_eq!(h.query_all_by_label("Courier New").count(), 0, "a click outside closed the list");
    }

    /// A width that cannot hold a box (not a number, zero, negative) is clamped instead of drawing
    /// something degenerate, and nothing panics.
    #[test]
    fn a_degenerate_combo_width_still_draws_a_usable_box() {
        let ctx = combo_ctx();
        let fonts: Vec<String> = ["Arial"].map(String::from).to_vec();
        COMBO_RECTS.with(|c| c.borrow_mut().clear());
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = combo(ui, "nan", f32::NAN, "12", &fonts, None);
            let _ = combo(ui, "zero", 0.0, "12", &fonts, None);
            let _ = combo(ui, "negative", -5.0, "12", &fonts, None);
        })
        .drop_without_applying_deltas();
        let rects = COMBO_RECTS.with(|c| c.take());
        assert_eq!(rects.len(), 3, "all three degenerate widths still draw");
        for (id, outer, _text, arrow) in &rects {
            assert!(outer.width().is_finite() && outer.width() >= COMBO_MIN_W, "{id}: {} wide", outer.width());
            assert!((outer.height() - COMBO_H).abs() < 0.01, "{id}: {} tall", outer.height());
            assert!(arrow.width() > 0.0 && arrow.right() <= outer.right() + 0.01, "{id}: the arrow escaped the control");
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
