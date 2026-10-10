//! Developer › Controls › Properties: a content control's title, tag, locks and, by kind, its
//! list entries, date format or check box symbols. OK runs `developer.properties` with the
//! settings, so scripts and agents get the same result without the dialog.

use egui::Ui;
use serde::Serialize;
use serde_json::{Value, json};

use crate::WordApp;
use crate::theme::semibold;

/// One list entry being edited.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Entry {
    pub display: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ControlForm {
    /// Where the control's content starts (it is found again there on OK).
    pub at: Value,
    pub kind: String,
    pub title: String,
    pub tag: String,
    pub lock_delete: bool,
    pub lock_edit: bool,
    pub temporary: bool,
    pub multi_line: bool,
    pub placeholder: String,
    pub items: Vec<Entry>,
    pub format: String,
    pub checked_symbol: String,
    pub unchecked_symbol: String,
}

impl ControlForm {
    /// The form for the control at the caret, if there is one.
    pub fn read(app: &mut WordApp) -> Option<ControlForm> {
        let v = app.session.run("developer.control", &json!({})).ok()?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        let at = v.get("start").cloned().unwrap_or(Value::Null);
        // The content starts just after the start marker.
        let at = match serde_json::from_value::<wordcraft_doc::Pos>(at) {
            Ok(p) => serde_json::to_value(wordcraft_doc::Pos { off: p.off + wordcraft_doc::para::OBJ.len_utf8(), ..p }).unwrap_or(Value::Null),
            Err(_) => Value::Null,
        };
        let items = v
            .get("items")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|i| Entry {
                        display: i.get("display").and_then(Value::as_str).unwrap_or("").to_string(),
                        value: i.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(ControlForm {
            at,
            kind: s("type"),
            title: s("title"),
            tag: s("tag"),
            lock_delete: b("lockDelete"),
            lock_edit: b("lockEdit"),
            temporary: b("temporary"),
            multi_line: b("multiLine"),
            placeholder: s("placeholder"),
            items,
            format: s("format"),
            checked_symbol: s("checkedSymbol"),
            unchecked_symbol: s("uncheckedSymbol"),
        })
    }

    /// The `developer.properties` parameters the form stands for.
    pub fn params(&self) -> Value {
        let mut v = json!({
            "at": self.at,
            "title": self.title,
            "tag": self.tag,
            "lockDelete": self.lock_delete,
            "lockEdit": self.lock_edit,
            "temporary": self.temporary,
        });
        if !self.placeholder.trim().is_empty() {
            v["placeholder"] = json!(self.placeholder);
        }
        match self.kind.as_str() {
            "plainText" => v["multiLine"] = json!(self.multi_line),
            "comboBox" | "dropDown" => {
                v["items"] = json!(
                    self.items
                        .iter()
                        .filter(|e| !e.display.trim().is_empty())
                        .map(|e| json!({"display": e.display, "value": if e.value.is_empty() { &e.display } else { &e.value }}))
                        .collect::<Vec<_>>()
                );
            }
            "date" => v["format"] = json!(self.format),
            "checkBox" => {
                v["checkedSymbol"] = json!(self.checked_symbol);
                v["uncheckedSymbol"] = json!(self.unchecked_symbol);
            }
            _ => {}
        }
        v
    }
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, f: &mut ControlForm) -> bool {
    ui.set_width(420.0);
    egui::Grid::new("cc_props").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label(tl!("Title"));
        ui.add(egui::TextEdit::singleline(&mut f.title).desired_width(280.0));
        ui.end_row();
        ui.label(tl!("Tag"));
        ui.add(egui::TextEdit::singleline(&mut f.tag).desired_width(280.0));
        ui.end_row();
        if !matches!(f.kind.as_str(), "checkBox" | "picture" | "repeatingSection" | "repeatingSectionItem") {
            ui.label(tl!("Placeholder text"));
            ui.add(egui::TextEdit::singleline(&mut f.placeholder).desired_width(280.0));
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.checkbox(&mut f.lock_delete, tl!("Can't be deleted"));
    ui.checkbox(&mut f.lock_edit, tl!("Contents can't be edited"));
    ui.checkbox(&mut f.temporary, tl!("Remove the control once edited"));
    match f.kind.as_str() {
        "plainText" => {
            ui.checkbox(&mut f.multi_line, tl!("Allow line breaks"));
        }
        "comboBox" | "dropDown" => entries(ui, f),
        "date" => {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Date format"));
                ui.add(egui::TextEdit::singleline(&mut f.format).desired_width(160.0));
            });
            ui.horizontal(|ui| {
                for pic in ["M/d/yyyy", "d.M.yyyy", "yyyy-MM-dd", "dddd, MMMM d, yyyy"] {
                    if ui.small_button(pic).clicked() {
                        f.format = pic.to_string();
                    }
                }
            });
        }
        "checkBox" => {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Checked symbol"));
                ui.add(egui::TextEdit::singleline(&mut f.checked_symbol).desired_width(40.0));
                ui.label(tl!("Unchecked symbol"));
                ui.add(egui::TextEdit::singleline(&mut f.unchecked_symbol).desired_width(40.0));
            });
        }
        _ => {}
    }
    let mut done = false;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = true;
            }
            if ui.button(egui::RichText::new(tl!("OK")).color(egui::Color32::WHITE)).clicked() {
                match app.run("developer.properties", f.params()) {
                    Ok(_) => done = true,
                    Err(e) => app.status(e),
                }
            }
        });
    });
    done
}

fn entries(ui: &mut Ui, f: &mut ControlForm) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("List entries")).font(semibold(12.0)));
    let mut remove = None;
    egui::Grid::new("cc_entries").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
        ui.label(egui::RichText::new(tl!("Display text")).small());
        ui.label(egui::RichText::new(tl!("Value")).small());
        ui.end_row();
        for (i, e) in f.items.iter_mut().enumerate() {
            ui.add(egui::TextEdit::singleline(&mut e.display).desired_width(170.0));
            ui.add(egui::TextEdit::singleline(&mut e.value).desired_width(140.0));
            if ui.small_button(tl!("Remove")).clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove
        && i < f.items.len()
    {
        f.items.remove(i);
    }
    if f.items.len() < wordcraft_doc::control::MAX_LIST_ITEMS && ui.button(tl!("Add")).clicked() {
        f.items.push(Entry::default());
    }
}
