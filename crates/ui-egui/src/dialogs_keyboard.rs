//! Tools › Customize Keyboard (#368; File › Options › Keyboard shortcuts, command search): pick
//! a category and a command, see its keys, press a new key combination, see which command uses
//! it now, then Assign / Remove / Reset All. Every change runs `tools.customizeKeyboard`, so
//! agents get the same result without the dialog; the keys persist with the preferences.

use egui::{Sense, Ui, vec2};
use serde::Serialize;
use serde_json::json;
use wordcraft_engine::keymap::normalize;

use crate::WordApp;
use crate::theme::{Tokens, semibold};
use crate::widgets::key_text;

/// The category listing every command.
const ALL: &str = "";

/// The dialog's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardForm {
    /// A ribbon tab (the first part of commands' locations), or empty for All Commands.
    pub category: String,
    /// The selected command id.
    pub command: String,
    /// The key picked in "Keys for this command" (for Remove).
    pub current: String,
    /// The key combination pressed in "New shortcut", canonical (`Mod+Shift+K`).
    pub pressed: String,
    /// Reset All asked; waiting for Yes / No.
    pub confirm_reset: bool,
}

impl KeyboardForm {
    pub fn read(app: &WordApp) -> KeyboardForm {
        let category = categories(app).into_iter().next().unwrap_or_default();
        let command = commands(app, &category).first().map(|(id, _)| (*id).to_string()).unwrap_or_default();
        KeyboardForm { category, command, ..Default::default() }
    }
}

/// Categories: the ribbon tabs commands live on, in ribbon order, then the other places
/// (Navigation, Editing…) alphabetically.
fn categories(app: &WordApp) -> Vec<String> {
    let mut other: Vec<String> = Vec::new();
    let mut tabs: Vec<&str> = Vec::new();
    for spec in app.session.registry.all() {
        let tab = spec.location.split(" › ").next().unwrap_or("");
        if crate::ribbon::TABS.contains(&tab) {
            if !tabs.contains(&tab) {
                tabs.push(tab);
            }
        } else if !tab.is_empty() && !other.iter().any(|t| t == tab) {
            other.push(tab.to_string());
        }
    }
    other.sort();
    crate::ribbon::TABS.iter().filter(|t| tabs.contains(t)).map(|t| (*t).to_string()).chain(other).collect()
}

/// A category's commands as (id, label in the interface language), sorted by label.
fn commands(app: &WordApp, category: &str) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = app
        .session
        .registry
        .all()
        .iter()
        .filter(|s| category == ALL || s.location.split(" › ").next() == Some(category))
        .map(|s| (s.id, tl!(s.label).to_string()))
        .collect();
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(b.0)));
    out
}

/// Only combinations with Mod, Ctrl or Alt, or function keys, can be assigned from the dialog
/// (plain letters and Shift+letter type text).
fn assignable(key: &str) -> bool {
    let name = key.rsplit('+').next().unwrap_or("");
    let function = name.strip_prefix('F').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    key.contains("Mod+") || key.contains("Ctrl+") || key.contains("Alt+") || function
}

/// The key combination a focused "New shortcut" box received this frame, if any.
fn pressed_key(ui: &Ui) -> Option<String> {
    let (events, mods) = ui.input(|i| (i.events.clone(), i.modifiers));
    let mut out = None;
    for e in events {
        let key = match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } => crate::keys::key_name(key).map(|n| crate::keys::combo(modifiers, n, true)),
            // The shell turns these into clipboard events rather than keys.
            egui::Event::Copy => Some(crate::keys::combo(mods, "C", true)),
            egui::Event::Cut => Some(crate::keys::combo(mods, "X", true)),
            egui::Event::Paste(_) => Some(crate::keys::combo(mods, "V", true)),
            _ => None,
        };
        if let Some(k) = key.as_deref().and_then(normalize).filter(|k| assignable(k)) {
            out = Some(k);
        }
    }
    out
}

/// A bordered, scrolling list of (value, text) rows; returns the clicked value.
fn list(ui: &mut Ui, id: &str, size: egui::Vec2, rows: &[(String, String)], selected: &str) -> Option<String> {
    let mut picked = None;
    egui::Frame::new().stroke(ui.visuals().widgets.noninteractive.bg_stroke).inner_margin(2.0).show(ui, |ui| {
        ui.set_width(size.x);
        ui.set_height(size.y);
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for (value, text) in rows.get(range).unwrap_or_default() {
                    if ui.add(egui::Button::selectable(value == selected, text.as_str()).min_size(vec2(0.0, row_h))).clicked() {
                        picked = Some(value.clone());
                    }
                }
            });
        });
    });
    picked
}

/// Run `tools.customizeKeyboard` with these params; errors go to the status bar.
fn change(app: &mut WordApp, params: serde_json::Value) -> bool {
    match app.run("tools.customizeKeyboard", params) {
        Ok(_) => true,
        Err(e) => {
            app.status(e);
            false
        }
    }
}

/// The dialog body; true closes it.
pub fn customize_keyboard(app: &mut WordApp, ui: &mut Ui, f: &mut KeyboardForm) -> bool {
    let reg = app.session.registry.clone();
    let heading = |ui: &mut Ui, s: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
    };
    let (left, right) = (170.0, 300.0);

    heading(ui, "Choose a command");
    let mut cats: Vec<(String, String)> = categories(app).into_iter().map(|c| (c.clone(), tl!(&c).to_string())).collect();
    cats.push((ALL.to_string(), tl!("All Commands").to_string()));
    let cmds: Vec<(String, String)> = commands(app, &f.category).into_iter().map(|(id, l)| (id.to_string(), l)).collect();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Categories:"));
            if let Some(c) = list(ui, "kb_categories", vec2(left, 170.0), &cats, &f.category) {
                f.category = c;
                f.command = commands(app, &f.category).first().map(|(id, _)| (*id).to_string()).unwrap_or_default();
                f.current.clear();
            }
        });
        ui.vertical(|ui| {
            ui.label(tl!("Commands:"));
            if let Some(c) = list(ui, "kb_commands", vec2(right, 170.0), &cmds, &f.command) {
                f.command = c;
                f.current.clear();
            }
        });
    });

    heading(ui, "Choose a shortcut");
    let keys = app.session.keymap.keys_for(&reg, &f.command);
    if !keys.contains(&f.current) {
        f.current = keys.first().cloned().unwrap_or_default();
    }
    let mut close = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Keys for this command:"));
            let rows: Vec<(String, String)> = keys.iter().map(|k| (k.clone(), key_text(k))).collect();
            if let Some(k) = list(ui, "kb_current", vec2(left, 70.0), &rows, &f.current) {
                f.current = k;
            }
        });
        ui.vertical(|ui| {
            ui.label(tl!("New shortcut:"));
            close = capture(ui, f);
            ui.add_space(4.0);
            if !f.pressed.is_empty() {
                let owner = match app.session.keymap.resolve(&reg, &f.pressed) {
                    Some(s) => crate::i18n::fmt(tl!("Now used by {command}"), &[("command", tl!(s.label))]),
                    None => tl!("Not used by any command").to_string(),
                };
                ui.label(owner);
            }
        });
    });
    if let Some(spec) = reg.get(&f.command) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(format!("{} · {}", crate::i18n::location(spec.location), spec.id)).small().weak());
    }

    ui.add_space(8.0);
    if f.confirm_reset {
        ui.label(tl!("Remove every custom shortcut and restore the built-in keys?"));
        ui.horizontal(|ui| {
            if ui.button(tl!("Yes")).clicked() {
                change(app, json!({"reset": true}));
                f.confirm_reset = false;
            }
            if ui.button(tl!("No")).clicked() {
                f.confirm_reset = false;
            }
        });
        return close;
    }
    ui.horizontal(|ui| {
        let can_assign = !f.command.is_empty() && !f.pressed.is_empty() && !keys.contains(&f.pressed);
        if ui.add_enabled(can_assign, egui::Button::new(tl!("Assign"))).clicked()
            && change(app, json!({"assign": {"command": f.command, "key": f.pressed}}))
        {
            f.current = std::mem::take(&mut f.pressed);
        }
        if ui.add_enabled(!f.current.is_empty(), egui::Button::new(tl!("Remove"))).clicked()
            && change(app, json!({"remove": {"command": f.command, "key": f.current}}))
        {
            f.current.clear();
        }
        if ui.add_enabled(!app.session.keymap.is_empty(), egui::Button::new(tl!("Reset All…"))).clicked() {
            f.confirm_reset = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Close")).clicked() {
                close = true;
            }
        });
    });
    close
}

/// The "New shortcut" box: takes keyboard focus and records the combination pressed.
/// True when plain Escape asks to close the dialog.
fn capture(ui: &mut Ui, f: &mut KeyboardForm) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(300.0, 26.0), Sense::click());
    // Keys belong to the box whenever nothing else in the dialog has focus, so a shortcut pressed
    // here never runs on the document behind it.
    if resp.clicked() || ui.memory(|m| m.focused().is_none()) {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    let mut close = false;
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape) && i.modifiers.is_none()) {
            close = true;
        }
        if let Some(k) = pressed_key(ui) {
            f.pressed = k;
        }
    }
    let stroke = egui::Stroke::new(if focused { 1.5 } else { 1.0 }, if focused { t.accent } else { t.input_border });
    ui.painter().rect(rect, 2.0, t.input, stroke, egui::StrokeKind::Inside);
    let (text, color) = if f.pressed.is_empty() { (tl!("Press a key combination").to_string(), t.text_dim) } else { (key_text(&f.pressed), t.text) };
    ui.painter().text(rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(13.0), color);
    close
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use crate::dialogs::Dialog;
    use crate::{Services, WordApp};

    /// #368: Tools › Customize Keyboard opens the dialog; a custom key survives a restart, and a
    /// damaged saved map doesn't lose the other preferences.
    #[test]
    fn customize_keyboard_opens_and_custom_keys_persist() {
        let app = || WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut a = app();
        a.run("tools.customizeKeyboard", json!({})).unwrap();
        assert!(matches!(a.dialog, Some(Dialog::CustomizeKeyboard { .. })));
        a.run("tools.customizeKeyboard", json!({"assign": {"command": "format.italic", "key": "Mod+B"}})).unwrap();
        assert_eq!(crate::widgets::shortcut_text(&a, "format.bold"), "");
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.session.keymap.resolve(&b.session.registry, "Mod+B").map(|s| s.id), Some("format.italic"));

        let mut c = app();
        c.apply_prefs(
            serde_json::from_str(r#"{"tab": "Insert", "keyboard": {"assigned": {"Mod+Q": "gone.command", "Bad+": 1}, "removed": 7}}"#).unwrap(),
        );
        assert_eq!(c.ui.tab, "Insert");
        assert!(c.session.keymap.is_empty());
    }
}
