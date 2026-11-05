//! An add-in's dialog (protocol 1.2): a list to pick from, a form to fill in, or a message, drawn
//! by WordCraft with its own widgets and theme. The add-in has already answered when it shows:
//! OK, a double-click on an item or one of its buttons runs another of the add-in's commands
//! through `plugins.run`, as a ribbon button does, and that command's answer may bring the next
//! dialog (dialogs chain). Cancel, Escape and the window's close button just close.
//!
//! The add-in's words (title, prompt, labels) are shown as given: the add-in translates them.

use std::collections::BTreeSet;

use egui::{Color32, RichText, Sense, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_plugin::dialog::{Dialog, DialogKind, FieldKind, with};

use crate::WordApp;
use crate::theme::Tokens;

/// A list shows a filter box when it has more items than this.
const FILTER_ABOVE: usize = 20;
/// Height of a list row.
const ROW_H: f32 = 34.0;
/// The icon items whose icon name the app doesn't know get.
const GENERIC_ICON: &str = "file";

/// A field's value while the user edits it.
#[derive(Clone, Debug, PartialEq)]
enum Edit {
    /// Text, secret and number fields (a number is checked on OK).
    Text(String),
    Check(bool),
    /// The picked option's id.
    Choice(String),
}

/// An add-in's dialog on screen. What a script reading the UI's state sees of it leaves out what
/// the user typed (a password among it).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PluginDialog {
    /// The add-in whose commands the dialog runs.
    pub plugin: String,
    pub dialog: Dialog,
    filter: String,
    /// Indices of the items for `filtered_for`'s filter.
    #[serde(skip)]
    filtered: Vec<usize>,
    #[serde(skip)]
    filtered_for: Option<String>,
    /// Selected items (indices into `dialog.items`).
    selected: BTreeSet<usize>,
    #[serde(skip)]
    edits: Vec<Edit>,
    /// Why the last OK or button didn't go through (a number that isn't one, the add-in's error).
    pub error: String,
}

/// What the user did this frame.
enum Action {
    None,
    Close,
    /// Run one of the add-in's commands with these params.
    Run(String, Value),
}

impl PluginDialog {
    pub fn new(plugin: &str, dialog: Dialog) -> PluginDialog {
        let edits = dialog
            .fields
            .iter()
            .map(|f| match f.kind {
                FieldKind::Checkbox => Edit::Check(f.value.as_bool().unwrap_or(false)),
                FieldKind::Choice => Edit::Choice(f.value.as_str().unwrap_or("").to_string()),
                FieldKind::Number => Edit::Text(match &f.value {
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                }),
                FieldKind::Text | FieldKind::Secret => Edit::Text(f.value.as_str().unwrap_or("").to_string()),
            })
            .collect();
        PluginDialog {
            plugin: plugin.to_string(),
            dialog,
            filter: String::new(),
            filtered: Vec::new(),
            filtered_for: None,
            selected: BTreeSet::new(),
            edits,
            error: String::new(),
        }
    }

    /// The window's title: the dialog's, else the add-in's name.
    pub fn title(&self) -> String {
        if !self.dialog.title.trim().is_empty() {
            return self.dialog.title.clone();
        }
        wordcraft_plugin::get(&self.plugin).map(|p| p.manifest.name.clone()).unwrap_or_else(|| self.plugin.clone())
    }

    /// Selects the item with this id (tests and agents), as a click does. False when none has it.
    pub fn select(&mut self, id: &str) -> bool {
        let Some(i) = self.dialog.items.iter().position(|it| it.id == id) else { return false };
        if !self.dialog.multi {
            self.selected.clear();
        }
        self.selected.insert(i);
        true
    }

    /// Sets a form field's value as typed (tests and agents). False when there's no such field.
    pub fn set_field(&mut self, id: &str, value: &str) -> bool {
        let Some(i) = self.dialog.fields.iter().position(|f| f.id == id) else { return false };
        let Some(e) = self.edits.get_mut(i) else { return false };
        *e = match e {
            Edit::Check(_) => Edit::Check(matches!(value, "true" | "1" | "on")),
            Edit::Choice(_) => Edit::Choice(value.to_string()),
            Edit::Text(_) => Edit::Text(value.to_string()),
        };
        true
    }

    /// The items the filter lets through, as indices.
    fn visible(&mut self) -> &[usize] {
        let filter = self.filter.trim().to_lowercase();
        if self.filtered_for.as_deref() != Some(filter.as_str()) {
            self.filtered = self
                .dialog
                .items
                .iter()
                .enumerate()
                .filter(|(_, it)| filter.is_empty() || it.label.to_lowercase().contains(&filter))
                .map(|(i, _)| i)
                .collect();
            self.filtered_for = Some(filter);
        }
        &self.filtered
    }

    /// The selection, as a command gets it: `id` (one item) or `ids` (`multi`); nothing when
    /// nothing is selected.
    fn selection(&self) -> Option<(&'static str, Value)> {
        let ids: Vec<&str> = self.selected.iter().filter_map(|i| self.dialog.items.get(*i)).map(|it| it.id.as_str()).collect();
        if ids.is_empty() {
            None
        } else if self.dialog.multi {
            Some(("ids", json!(ids)))
        } else {
            ids.first().map(|id| ("id", json!(id)))
        }
    }

    /// The form's values (`fields`), or the label of a number field that isn't a number.
    fn fields(&self) -> Result<Value, String> {
        let mut out = serde_json::Map::new();
        for (f, e) in self.dialog.fields.iter().zip(&self.edits) {
            let v = match (f.kind, e) {
                (FieldKind::Number, Edit::Text(t)) if t.trim().is_empty() => Value::Null,
                (FieldKind::Number, Edit::Text(t)) => {
                    match t.trim().replace(',', ".").parse::<f64>().ok().filter(|x| x.is_finite()).and_then(serde_json::Number::from_f64) {
                        Some(n) => Value::Number(n),
                        None => return Err(if f.label.trim().is_empty() { f.id.clone() } else { f.label.clone() }),
                    }
                }
                (_, Edit::Text(t)) => json!(t),
                (_, Edit::Check(b)) => json!(b),
                (_, Edit::Choice(c)) => json!(c),
            };
            out.insert(f.id.clone(), v);
        }
        Ok(Value::Object(out))
    }

    /// What OK does: run the command with the dialog's params plus the selection or the fields;
    /// close (a message without a command); or nothing yet (a list with nothing selected, a
    /// number that isn't one).
    fn ok(&mut self) -> Action {
        let Some(command) = self.dialog.command.clone() else { return Action::Close };
        match self.dialog.kind {
            DialogKind::List => match self.selection() {
                Some(sel) => Action::Run(command, with(&self.dialog.params, [sel])),
                None => Action::None,
            },
            DialogKind::Form => match self.fields() {
                Ok(fields) => Action::Run(command, with(&self.dialog.params, [("fields", fields)])),
                Err(label) => {
                    self.error = crate::i18n::fmt(tl!("{label} must be a number."), &[("label", &label)]);
                    Action::None
                }
            },
            DialogKind::Message => Action::Run(command, with(&self.dialog.params, [])),
        }
    }

    /// What extra button `i` runs: its command with its params plus the selection or the fields.
    fn button(&mut self, i: usize) -> Action {
        let Some(b) = self.dialog.buttons.get(i).cloned() else { return Action::None };
        let mut extra = Vec::new();
        match self.dialog.kind {
            DialogKind::List => extra.extend(self.selection()),
            DialogKind::Form => match self.fields() {
                Ok(f) => extra.push(("fields", f)),
                Err(label) => {
                    self.error = crate::i18n::fmt(tl!("{label} must be a number."), &[("label", &label)]);
                    return Action::None;
                }
            },
            DialogKind::Message => {}
        }
        Action::Run(b.command, with(&b.params, extra))
    }

    /// Whether OK can be pressed: a list needs a selection.
    fn ok_enabled(&self) -> bool {
        self.dialog.kind != DialogKind::List || !self.selected.is_empty()
    }
}

/// Runs what OK does in the open add-in dialog (tests and agents): false when nothing ran.
pub fn press_ok(app: &mut WordApp) -> bool {
    let Some(crate::dialogs::Dialog::Plugin(mut d)) = app.dialog.take() else { return false };
    let action = d.ok();
    finish(app, d, action)
}

/// Draws the add-in dialog in `app.dialog` (the caller made sure it is one) and does what the
/// user asks of it.
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    let Some(crate::dialogs::Dialog::Plugin(mut d)) = app.dialog.take() else { return };
    let mut open = true;
    let mut action = Action::None;
    let title = d.title();
    let id = egui::Id::unique(("plugin-dialog", &d.plugin));
    let window = egui::Window::new(RichText::new(title)).id(id).collapsible(false).resizable(false);
    // A form with a multi-line field (code, say) gets room for its lines (protocol 2.2).
    let wide = d.dialog.fields.iter().any(|f| f.multiline);
    let shown = crate::dialogs::place_dialog(ctx, id, window).open(&mut open).show(ctx, |ui| {
        ui.set_min_width(if wide { 640.0 } else { 380.0 });
        ui.set_max_width(if wide { 760.0 } else { 560.0 });
        action = body(ui, &mut d);
    });
    if shown.is_some() {
        crate::dialogs::count_dialog_frame(ctx, id);
    }
    if !open {
        action = Action::Close;
    }
    finish(app, d, action);
}

/// After a frame: close, keep the dialog, or run the add-in's command. A command that fails
/// leaves the dialog open with its error; one that succeeds closes it, and the next dialog its
/// answer brings (if any) takes its place.
fn finish(app: &mut WordApp, mut d: Box<PluginDialog>, action: Action) -> bool {
    match action {
        Action::None => {
            app.dialog = Some(crate::dialogs::Dialog::Plugin(d));
            false
        }
        Action::Close => {
            app.canvas.want_focus = true;
            false
        }
        Action::Run(command, params) => {
            let plugin = d.plugin.clone();
            match app.run("plugins.run", json!({"plugin": plugin, "command": command, "params": params})) {
                Ok(_) => {
                    if app.dialog.is_none() {
                        app.canvas.want_focus = true;
                    }
                }
                Err(e) => {
                    d.error = e;
                    if app.dialog.is_none() {
                        app.dialog = Some(crate::dialogs::Dialog::Plugin(d));
                    }
                }
            }
            true
        }
    }
}

/// The dialog's content and buttons.
fn body(ui: &mut Ui, d: &mut PluginDialog) -> Action {
    let t = Tokens::get(ui.ctx());
    let mut action = Action::None;
    let colors_open = crate::colors::is_open(ui.ctx());
    // A multi-line field has the keyboard focus: Enter is a new line there, not OK.
    let mut in_multiline = false;
    if !d.dialog.prompt.is_empty() {
        let prompt = d.dialog.prompt.clone();
        if d.dialog.kind == DialogKind::Message {
            egui::ScrollArea::vertical().max_height(360.0).id_salt("plugin-message").show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(prompt).color(t.text)).wrap());
            });
        } else {
            ui.add(egui::Label::new(RichText::new(prompt).color(t.text)).wrap());
        }
        ui.add_space(6.0);
    }
    match d.dialog.kind {
        DialogKind::List => {
            if let Some(a) = list(ui, d, &t) {
                action = a;
            }
        }
        DialogKind::Form => in_multiline = form(ui, d),
        DialogKind::Message => {}
    }
    if !d.error.is_empty() {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(&d.error).color(ui.visuals().error_fg_color)).wrap());
    }
    // Cancel, OK (primary), then the add-in's buttons, right to left.
    ui.add_space(8.0);
    let mut clicked = None;
    let mut ok = false;
    let mut cancel = false;
    let ok_label = d.dialog.ok_label.clone().unwrap_or_else(|| tl!("OK").to_string());
    let just_ok = d.dialog.kind == DialogKind::Message && d.dialog.command.is_none() && d.dialog.buttons.is_empty();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !just_ok && ui.button(tl!("Cancel")).clicked() {
                cancel = true;
            }
            let okb = ui.add_enabled(d.ok_enabled(), crate::widgets::primary_button(ui, ok_label));
            ok |= okb.clicked();
            for (i, b) in d.dialog.buttons.iter().enumerate().rev() {
                if ui.button(&b.label).clicked() {
                    clicked = Some(i);
                }
            }
        });
    });
    if !colors_open {
        cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
        // Enter is OK, except in a multi-line text or with nothing to run yet.
        ok |= !in_multiline && d.ok_enabled() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    }
    if cancel {
        return Action::Close;
    }
    if let Some(i) = clicked {
        return d.button(i);
    }
    if ok {
        return d.ok();
    }
    action
}

/// A list: the filter box (over 20 items), then the items with their icon, label and detail.
/// A click selects (with `multi`, toggles); a double-click runs OK on that item.
fn list(ui: &mut Ui, d: &mut PluginDialog, t: &Tokens) -> Option<Action> {
    if d.dialog.items.len() > FILTER_ABOVE {
        ui.horizontal(|ui| {
            ui.label(tl!("Filter"));
            ui.add(egui::TextEdit::singleline(&mut d.filter).desired_width(f32::INFINITY).id_salt("plugin-filter"));
        });
        ui.add_space(4.0);
    }
    let rows: Vec<usize> = d.visible().to_vec();
    if rows.is_empty() {
        ui.label(RichText::new(tl!("Nothing to show.")).color(t.text_dim));
        return None;
    }
    let mut picked = None;
    let mut double = None;
    egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(ROW_H * 9.0).auto_shrink([false, true]).id_salt("plugin-list").show_rows(
            ui,
            ROW_H,
            rows.len(),
            |ui, range| {
                for k in range {
                    let Some(&i) = rows.get(k) else { continue };
                    let Some(item) = d.dialog.items.get(i) else { continue };
                    let selected = d.selected.contains(&i);
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    let fill = if selected {
                        t.selection
                    } else if resp.hovered() {
                        t.hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(r, 2.0, fill);
                    let icon = egui::Rect::from_center_size(pos2(r.min.x + 18.0, r.center().y), vec2(20.0, 20.0));
                    crate::icons::paint_or(ui.painter(), icon, &item.icon, GENERIC_ICON, t.icon, t.accent);
                    let x = r.min.x + 36.0;
                    let font = egui::FontId::proportional(13.0);
                    let small = egui::FontId::proportional(11.0);
                    let clip = ui.painter().with_clip_rect(r.shrink(2.0));
                    if item.detail.is_empty() {
                        clip.text(pos2(x, r.center().y), egui::Align2::LEFT_CENTER, &item.label, font, t.text);
                    } else {
                        clip.text(pos2(x, r.min.y + 3.0), egui::Align2::LEFT_TOP, &item.label, font, t.text);
                        clip.text(pos2(x, r.max.y - 3.0), egui::Align2::LEFT_BOTTOM, &item.detail, small, t.text_dim);
                    }
                    let label = if item.detail.is_empty() { item.label.clone() } else { format!("{}, {}", item.label, item.detail) };
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, selected, &label));
                    if resp.double_clicked() {
                        double = Some(i);
                    } else if resp.clicked() {
                        picked = Some(i);
                    }
                }
            },
        );
    });
    if let Some(i) = picked {
        if d.dialog.multi {
            if !d.selected.remove(&i) {
                d.selected.insert(i);
            }
        } else {
            d.selected.clear();
            d.selected.insert(i);
        }
        d.error.clear();
    }
    if let Some(i) = double {
        // A double-click runs OK on that one item.
        if !d.dialog.multi {
            d.selected.clear();
        }
        d.selected.insert(i);
        return Some(d.ok());
    }
    None
}

/// A form: each field's label beside its box, check box or choice. True when a multi-line
/// field has the keyboard focus (Enter belongs to it).
fn form(ui: &mut Ui, d: &mut PluginDialog) -> bool {
    let mut in_multiline = false;
    egui::Grid::new("plugin-form").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        for (i, f) in d.dialog.fields.iter().enumerate() {
            let Some(e) = d.edits.get_mut(i) else { continue };
            match (f.kind, e) {
                (FieldKind::Checkbox, Edit::Check(b)) => {
                    ui.label("");
                    ui.checkbox(b, &f.label);
                }
                (FieldKind::Choice, Edit::Choice(c)) => {
                    let label = ui.label(&f.label);
                    let shown = f.options.iter().find(|o| o.id == *c).map(|o| o.label.clone()).unwrap_or_default();
                    let combo = craft_ui::ComboBox::from_id_salt(("plugin-choice", i)).width(260.0).selected_text(shown).show_ui(ui, |ui| {
                        for o in &f.options {
                            ui.selectable_value(c, o.id.clone(), &o.label);
                        }
                    });
                    combo.response.labelled_by(label.id);
                }
                (FieldKind::Text, Edit::Text(text)) if f.multiline => {
                    // Protocol 2.2: several lines (Enter is a new line), monospace with Tab
                    // indenting for code, scrolling past its height; bounded like the add-in's value.
                    let label = ui.label(&f.label);
                    let rows = f.rows.unwrap_or(wordcraft_plugin::dialog::DEFAULT_ROWS);
                    let mut edit = egui::TextEdit::multiline(text)
                        .desired_width(520.0)
                        .desired_rows(usize::from(rows))
                        .char_limit(wordcraft_plugin::dialog::MAX_MULTILINE_VALUE)
                        .id_salt(("plugin-field", i));
                    if f.code {
                        edit = edit.code_editor();
                    }
                    let style = if f.code { egui::TextStyle::Monospace } else { egui::TextStyle::Body };
                    let line_h = ui.text_style_height(&style);
                    let r = egui::ScrollArea::vertical()
                        .id_salt(("plugin-multiline", i))
                        .max_height(line_h * f32::from(rows) + 12.0)
                        .show(ui, |ui| ui.add(edit).labelled_by(label.id))
                        .inner;
                    in_multiline |= r.has_focus();
                    if i == 0 && !ui.ctx().memory(|m| m.focused().is_some()) {
                        r.request_focus();
                    }
                }
                (kind, Edit::Text(text)) => {
                    let label = ui.label(&f.label);
                    let edit = egui::TextEdit::singleline(text).desired_width(260.0).password(kind == FieldKind::Secret).id_salt(("plugin-field", i));
                    let r = ui.add(edit).labelled_by(label.id);
                    if i == 0 && !ui.ctx().memory(|m| m.focused().is_some()) {
                        r.request_focus();
                    }
                }
                _ => {
                    ui.label(&f.label);
                    ui.label("");
                }
            }
            ui.end_row();
        }
    });
    in_multiline
}
