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
    /// Filter-as-you-type query for the command list (#1845); UI-only dialog state, ignored
    /// while empty. Never persisted and never a command parameter.
    pub filter: String,
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
pub(crate) fn categories(app: &WordApp) -> Vec<String> {
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
pub(crate) fn commands(app: &WordApp, category: &str) -> Vec<(&'static str, String)> {
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

/// The commands to list for `category`, narrowed by `query` (#1845): an empty query keeps the
/// category scope; any other text searches every registered command by its id and its English or
/// translated label, case-insensitive (the id lets "file.save" or "save" find Save). Sorted as
/// [`commands`].
pub(crate) fn matching_commands(app: &WordApp, category: &str, query: &str) -> Vec<(String, String)> {
    let q = crate::i18n::lowercase(query.trim());
    if q.is_empty() {
        return commands(app, category).into_iter().map(|(id, l)| (id.to_string(), l)).collect();
    }
    let mut out: Vec<(String, String)> = app
        .session
        .registry
        .all()
        .iter()
        .filter(|s| s.id.to_lowercase().contains(&q) || s.label.to_lowercase().contains(&q) || crate::i18n::lowercase(tl!(s.label)).contains(&q))
        .map(|s| (s.id.to_string(), tl!(s.label).to_string()))
        .collect();
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(&b.0)));
    out
}

/// Keep `wanted` when it is still listed, else fall back to the first row (#1845).
pub(crate) fn select_in(rows: &[(String, String)], wanted: &str) -> String {
    if rows.iter().any(|(id, _)| id == wanted) { wanted.to_string() } else { rows.first().map(|(id, _)| id.clone()).unwrap_or_default() }
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
pub(crate) fn list(ui: &mut Ui, id: &str, size: egui::Vec2, rows: &[(String, String)], selected: &str) -> Option<String> {
    let mut picked = None;
    egui::Frame::new().stroke(ui.visuals().widgets.noninteractive.bg_stroke).inner_margin(2.0).show(ui, |ui| {
        ui.set_width(size.x);
        ui.set_height(size.y);
        if rows.is_empty() {
            // #1905: an empty list says so instead of a blank box.
            ui.centered_and_justified(|ui| ui.label(egui::RichText::new(tl!("(none)")).weak()));
            return;
        }
        // Virtual rows must include the button's padding and minimum interaction height;
        // otherwise the last row can extend beyond the scroll area's declared content height.
        let row_h = (ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y).max(ui.spacing().interact_size.y);
        // Bring the selected row into view when the selection (or the list) changes, so the row
        // an action targets is never off-screen; scrolling by hand doesn't change the selection.
        let seen = ui.make_persistent_id(("scrolled_to", id));
        let frame = ui.ctx().cumulative_frame_nr();
        let key = (selected.to_string(), rows.len());
        let scroll_to = ui.data_mut(|d| {
            let last = d.get_temp::<(String, usize, u64)>(seen);
            d.insert_temp(seen, (key.0.clone(), key.1, frame));
            !last.is_some_and(|(s, n, f)| s == key.0 && n == key.1 && f.saturating_add(1) >= frame)
        });
        let row_step = row_h + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
            if scroll_to && let Some(index) = rows.iter().position(|(value, _)| value == selected) {
                let top = ui.max_rect().top() + (index as f32 - range.start as f32) * row_step;
                ui.scroll_to_rect(egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=top + row_h), None);
            }
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
    ui.add(egui::TextEdit::singleline(&mut f.filter).hint_text(tl!("Search")).desired_width(left + right + 8.0));
    let mut cats: Vec<(String, String)> = categories(app).into_iter().map(|c| (c.clone(), tl!(&c).to_string())).collect();
    cats.push((ALL.to_string(), tl!("All Commands").to_string()));
    let cmds: Vec<(String, String)> = matching_commands(app, &f.category, &f.filter);
    f.command = select_in(&cmds, &f.command);
    if cmds.is_empty() {
        ui.label(egui::RichText::new(tl!("No matching commands")).weak());
    }
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Categories:"));
            if let Some(c) = list(ui, "kb_categories", vec2(left, 170.0), &cats, &f.category) {
                f.category = c;
                f.filter.clear();
                f.command = matching_commands(app, &f.category, &f.filter).first().map(|(id, _)| id.clone()).unwrap_or_default();
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
        c.apply_prefs(serde_json::from_str(r#"{"keyboard": {"assigned": {"Mod+Q": "gone.command", "Bad+": 1}, "removed": 7}}"#).unwrap());
        assert!(c.session.keymap.is_empty());
    }

    /// #1905: a command with no shortcut shows (none) instead of a blank keys box.
    #[test]
    fn keys_list_without_shortcuts_says_none() {
        use super::customize_keyboard;

        let app = || WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut a = app();
        a.run("tools.customizeKeyboard", json!({})).unwrap();
        let id = a
            .session
            .registry
            .all()
            .iter()
            .find(|s| a.session.keymap.keys_for(&a.session.registry, s.id).is_empty())
            .map(|s| s.id.to_string())
            .expect("a command with no shortcut");
        let mut form = match a.dialog.take() {
            Some(crate::dialogs::Dialog::CustomizeKeyboard { form }) => form,
            _ => panic!("expected the Customize Keyboard dialog"),
        };
        form.command = id;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
            customize_keyboard(&mut a, ui, &mut form);
        });
        output.textures_delta.clear();
        a.dialog = Some(crate::dialogs::Dialog::CustomizeKeyboard { form });
        let texts: Vec<String> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("(none)")), "empty keys list says (none)");
    }

    /// The dialog on its own in a harness; fonts are installed on the first frame.
    fn harness(category: &str, command: Option<&str>) -> egui_kittest::Harness<'static, (WordApp, super::KeyboardForm)> {
        let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut form = super::KeyboardForm::read(&app);
        form.category = category.to_string();
        form.command = match command {
            Some(c) => c.to_string(),
            None => super::commands(&app, category).first().map(|(id, _)| (*id).to_string()).unwrap_or_default(),
        };
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 500.0)).build_ui_state(
            move |ui, (app, form): &mut (WordApp, super::KeyboardForm)| {
                if fonts {
                    super::customize_keyboard(app, ui, form);
                } else {
                    crate::theme::install_fonts_for(ui.ctx(), false);
                    fonts = true;
                }
            },
            (app, form),
        );
        for _ in 0..4 {
            h.step();
        }
        h
    }

    type KbHarness<'a> = egui_kittest::Harness<'a, (WordApp, super::KeyboardForm)>;

    /// The area of the Commands list that is on screen: 170 high, below the title, the item
    /// spacing (3) and the frame's stroke and margin (1 + 2); a pixel of slack for rounding.
    fn commands_view(h: &KbHarness<'_>) -> egui::Rect {
        use egui_kittest::kittest::Queryable;
        let title = h.get_by_label("Commands:").rect();
        egui::Rect::from_min_size(egui::pos2(title.left(), title.bottom() + 6.0), egui::vec2(310.0, 170.0)).expand(1.0)
    }

    /// Whether a row labelled `label` is drawn inside the visible part of the Commands list.
    fn row_in_view(h: &KbHarness<'_>, label: &str) -> bool {
        use egui_kittest::kittest::Queryable;
        let view = commands_view(h);
        h.query_all_by_label(label).any(|n| view.contains_rect(n.rect()))
    }

    fn settle(h: &mut KbHarness<'_>) {
        for _ in 0..30 {
            h.step();
        }
    }

    /// #1696: the selected command is scrolled into view when the dialog opens, so Assign never
    /// targets a command the list doesn't show (also for rows far down the list).
    #[test]
    fn selected_command_is_scrolled_into_view() {
        let probe = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let all = super::commands(&probe, "");
        for index in [all.len() / 2, all.len() - 1] {
            let (id, label) = &all[index];
            let mut h = harness("", Some(id));
            settle(&mut h);
            assert!(row_in_view(&h, label), "the selected command {id} ({label}, row {index}) is in view");
        }
    }

    /// #1696: after scrolling the Commands list away from the selection, picking another category
    /// selects its first command and shows it.
    #[test]
    fn changing_category_shows_the_new_selection() {
        use egui_kittest::kittest::Queryable;
        let probe = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let all = super::commands(&probe, "");
        let mut h = harness("", None);
        let wheel = |h: &mut KbHarness<'_>, at: egui::Pos2, dy: f32| {
            h.event(egui::Event::PointerMoved(at));
            for _ in 0..60 {
                h.event(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, dy),
                    phase: egui::TouchPhase::Move,
                    modifiers: Default::default(),
                    source: egui::MouseWheelSource::Unknown,
                });
                h.step();
            }
        };
        let commands_at = commands_view(&h).center();
        wheel(&mut h, commands_at, -3.0);
        // The Categories list opened scrolled to All Commands (the last one); bring Home back.
        let categories_at = h.get_by_label("Categories:").rect().center() + egui::vec2(0.0, 40.0);
        wheel(&mut h, categories_at, 3.0);
        settle(&mut h);
        let first_of_home = super::commands(&probe, "Home").remove(0);
        assert!(!row_in_view(&h, &all[0].1), "the list was scrolled away from its first command");
        assert!(!row_in_view(&h, &first_of_home.1), "the list was scrolled away from the new selection");
        h.get_by_label("Home").click();
        settle(&mut h);
        assert_eq!(h.state().1.command, first_of_home.0);
        assert!(row_in_view(&h, &first_of_home.1), "the new selection {} is in view", first_of_home.0);
    }

    /// #1845: the command filter searches every command by label or id, ignoring the category.
    #[test]
    fn command_filter_searches_all_commands() {
        use super::{matching_commands, select_in};

        let app = || WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let a = app();
        // Empty query keeps the category scope.
        let file_cmds = matching_commands(&a, "File", "");
        assert!(!file_cmds.is_empty());
        assert!(
            file_cmds
                .iter()
                .all(|(id, _)| crate::i18n::location(a.session.registry.get(id).map(|s| s.location).unwrap_or_default()).starts_with("File"))
        );
        assert!(file_cmds.iter().any(|(id, _)| id == "file.save"));
        // A label finds its command from another category's scope.
        let bold = matching_commands(&a, "File", "bold");
        assert!(bold.iter().any(|(id, _)| id == "format.bold"));
        // Matching is case-insensitive and covers ids.
        assert!(matching_commands(&a, "File", "SAVE").iter().any(|(id, _)| id == "file.save"));
        assert!(matching_commands(&a, "File", "format.b").iter().any(|(id, _)| id == "format.bold"));
        // Nothing matches: empty rows and an empty selection.
        let none = matching_commands(&a, "File", "zzz-no-such-command");
        assert!(none.is_empty());
        assert_eq!(select_in(&none, "file.save"), "");
        // The selection survives when still listed, else falls back to the first row.
        assert_eq!(select_in(&bold, "format.bold"), "format.bold");
        let first = bold.first().map(|(id, _)| id.clone()).unwrap_or_default();
        assert_eq!(select_in(&bold, "file.save"), first);
    }
}
