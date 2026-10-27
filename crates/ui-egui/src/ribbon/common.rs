use super::language::mi_check;
use super::*;

/// Custom groups (Customize Ribbon, #379): up to three commands as large buttons, more as small
/// labelled buttons in columns of three. Each shows the command's own icon and label.
pub(super) fn custom_groups(app: &mut WordApp, ui: &mut Ui, groups: &[wordcraft_engine::ribbon::CustomGroup]) {
    for g in groups {
        let ids: Vec<(&str, String)> =
            g.commands.iter().filter_map(|id| app.session.registry.get(id).map(|s| (s.id, tl!(s.label).to_string()))).collect();
        crate::widgets::group_titled(ui, &g.name, &g.name, None, app, |ui, app| {
            if ids.len() <= 3 {
                for (id, label) in &ids {
                    big(ui, app, crate::widgets::command_icon(id), &two_lines(label), id, json!({}), false);
                }
                return;
            }
            for column in ids.chunks(3) {
                stack(ui, |ui| {
                    for (id, label) in column {
                        small(ui, app, crate::widgets::command_icon(id), Some(label), label, id, json!({}), false);
                    }
                });
            }
        });
    }
}

/// A large button's label on two lines when it is long (broken at the space nearest the middle).
fn two_lines(label: &str) -> String {
    let n = label.chars().count();
    if n <= 10 {
        return label.to_string();
    }
    let mid = n / 2;
    let best = label.char_indices().filter(|(_, c)| *c == ' ').min_by_key(|(i, _)| label[..*i].chars().count().abs_diff(mid)).map(|(i, _)| i);
    match best {
        Some(i) => format!("{}\n{}", &label[..i], label[i..].trim_start()),
        None => label.to_string(),
    }
}

/// A chevron over a ribbon edge that is also a button; `true` on the frame it is clicked.
pub(super) fn edge_cue(ui: &mut Ui, rect: Rect, glyph: &str, hint: &str, t: &Tokens) -> bool {
    let resp = ui.interact(rect, ui.scope_id().with(glyph), Sense::click()).on_hover_text(hint);
    let ink = if resp.hovered() { t.text } else { t.text_dim };
    ui.painter().rect_filled(rect, 0.0, t.ribbon);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, egui::FontId::proportional(18.0), ink);
    resp.clicked()
}

/// Whether Esc just ended typing in a field (`r`, remembered under `id`). egui 0.36 keeps a
/// `DragValue`'s typed text after Esc and applies it a frame later, when the field still counts as
/// having just lost focus but Esc is no longer pressed: a change reported within two frames of the
/// Esc is that late value, to be dropped so that Esc cancels as in Word.
pub(super) fn esc_cancelled(ui: &Ui, r: &egui::Response, id: egui::Id) -> bool {
    let now = ui.ctx().cumulative_pass_nr();
    let (focus_key, esc_key) = (id.with("focused"), id.with("esc"));
    // Read outside `data_mut`: both lock the context.
    let focused = r.has_focus();
    let had_focus = ui.data(|d| d.get_temp::<bool>(focus_key)).unwrap_or(false);
    ui.data_mut(|d| d.insert_temp(focus_key, focused));
    if had_focus && !focused && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|d| d.insert_temp(esc_key, now));
    }
    ui.data(|d| d.get_temp::<u64>(esc_key)).is_some_and(|at| now.saturating_sub(at) <= 2)
}

pub(super) fn stack(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        add(ui);
    });
}

/// Layout › Size: the named paper sizes, then More Paper Sizes… for a custom size in Page
/// Setup (#1433).
pub(crate) fn size_menu(ui: &mut Ui, app: &mut WordApp) {
    // The current paper size is checked.
    let x = wordcraft_engine::cmd::page::sect(&app.session);
    let current = wordcraft_geom::paper_name(x.page_w, x.page_h);
    for (n, w, h) in wordcraft_geom::PAPER_SIZES {
        // Paper names are the industry's (Letter, A4, B5 (JIS)) and stay as they are; the
        // envelopes say what they are. Sizes in the measurement unit, written the region's
        // way (`21 cm × 29,7 cm`, #838).
        let name = if n.starts_with("Envelope ") { tl!(n) } else { n };
        let (width, height) = (crate::options::length(*w), crate::options::length(*h));
        let label = crate::i18n::fmt(tl!("{paper}   {width} × {height}"), &[("paper", name), ("width", &width), ("height", &height)]);
        mi_check(ui, app, &label, current == Some(*n), "layout.size", json!({"name": n}));
    }
    ui.separator();
    mi(ui, app, crate::dialogs::PAPER_LABELS.more, "ui.dialog", json!({"name": "pageSetup"}));
}

pub(crate) fn mi(ui: &mut Ui, app: &mut WordApp, label: &str, id: &str, params: Value) {
    mi_text(ui, app, tl!(label), id, params);
}

/// [`mi`] with a drawn icon before the label.
pub(super) fn mi_icon(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, params: Value) {
    let sc = crate::widgets::shortcut_text(app, id);
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled_ui(enabled, |ui| crate::widgets::icon_menu_item(ui, icon, tl!(label), &sc, 200.0)).inner;
    if resp.clicked() {
        crate::widgets::run_from_ribbon(app, id, params);
        ui.close();
    }
}

/// A menu item whose text is already in the interface language.
pub(super) fn mi_text(ui: &mut Ui, app: &mut WordApp, text: &str, id: &str, params: Value) {
    let sc = crate::widgets::shortcut_text(app, id);
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(sc).min_size(vec2(200.0, 0.0)));
    if resp.clicked() {
        crate::widgets::run_from_ribbon(app, id, params);
        ui.close();
    }
}

/// Chart Design and SmartArt Design › Size: the selected graphic's height and width.
pub(super) fn graphic_size(ui: &mut Ui, app: &mut WordApp) {
    group(ui, "Size", None, app, |ui, app| {
        let (w0, h0) =
            wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h))).unwrap_or((0.0, 0.0));
        stack(ui, |ui| {
            for (label, key, v0) in [("H:", "height", h0), ("W:", "width", w0)] {
                crate::widgets::row(ui, |ui| {
                    ui.label(egui::RichText::new(tl!(label)).small());
                    let mut v = v0;
                    // The range bounds what is typed or dragged; a size outside it is shown as it
                    // is (clamped, egui would report a change and resize the graphic unasked).
                    let r = ui.add(egui::DragValue::new(&mut v).speed(1.0).range(36.0..=2000.0).clamp_existing_to_range(false).suffix(" pt"));
                    if r.changed() && v != v0 {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let _ = app.run("picture.size", json!({key: v, "lockAspect": false}));
                    }
                });
            }
        });
    });
}
