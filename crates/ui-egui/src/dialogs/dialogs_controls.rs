//! Developer › Controls › Properties: a content control's title, tag, locks and, by kind, its
//! list entries, date format or check box symbols. OK runs `developer.properties` with the
//! settings, so scripts and agents get the same result without the dialog.

use std::borrow::Cow;
use std::cell::RefCell;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft::ui::dialog::DialogId;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{ListRow, RowEvent, TrailingFace, TrailingPart};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::runtime::env::Label;
use craft::ui::sections::Section;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::host::{Asks, WordForm, open_form};
use crate::WordApp;

/// The display text column of the list entries table.
const DISPLAY: ColumnKey = ColumnKey::new("display");
/// The value column of the list entries table.
const VALUE: ColumnKey = ColumnKey::new("value");

/// One list entry being edited.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub display: String,
    pub value: String,
}

/// The values of the Content Control Properties dialog; the serde names are the dialog's fields for automation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// A translated word as a framework label.
fn word(text: &str) -> Label {
    Label::from(tl!(text).to_string())
}

/// A labelled one-line text field editing `value`.
fn text_field(ui: &mut Ui, salt: &'static str, label: &str, value: &mut String) {
    Field::new(word(label)).id_salt(salt).show(ui, |ui, _| {
        TextField::new(word(label), Bind::bound(value)).show(ui);
    });
}

/// A check box over `value`.
fn check(ui: &mut Ui, label: &str, value: &mut bool) {
    Checkbox::new(Bind::bound(value)).label(word(label)).show(ui);
}

/// The entries as the table's item source: the position is the key.
struct Items<'a>(&'a [Entry]);

impl DisplayOrder<usize> for Items<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|entry| Cow::Borrowed(entry.display.as_str()))
    }
}

impl ItemSource<usize> for Items<'_> {}

impl ControlForm {
    /// The list entries of a combo box or drop-down list: a table of display text and value with a Remove part on each
    /// row, and Add below it until the document's limit.
    fn entries(&mut self, ui: &mut Ui) {
        Section::fixed(word("List entries")).show(ui, |ui, _| {
            let columns = vec![
                ColumnSpec::new(DISPLAY, word("Display text"), ColumnWidth::fill(1.0)),
                ColumnSpec::new(VALUE, word("Value"), ColumnWidth::fill(1.0)),
            ];
            // Each edited cell, by row and column, applied after the table has drawn.
            let edits: RefCell<Vec<(usize, ColumnKey, String)>> = RefCell::new(Vec::new());
            let rows = self.items.clone();
            let out = Table::<usize>::new(word("List entries"), columns)
                .select(SelectMode::Single)
                .min_rows(1)
                .max_rows(6)
                .selection(Bind::given(Selection::default()))
                .show_items(ui, &Items(&rows), &mut |key, icx| {
                    let at = *key;
                    let entry = rows.get(at).cloned().unwrap_or_default();
                    let edits = &edits;
                    let display = TextField::new(word("Display text"), Bind::given(entry.display.clone()));
                    let value = TextField::new(word("Value"), Bind::given(entry.value.clone()));
                    ListRow::new(entry.display.clone())
                        .cells(icx.cells())
                        .cell_field(DISPLAY, display, move |out| {
                            if let Some(text) = out.events.value() {
                                edits.borrow_mut().push((at, DISPLAY, text.clone()));
                            }
                        })
                        .cell_field(VALUE, value, move |out| {
                            if let Some(text) = out.events.value() {
                                edits.borrow_mut().push((at, VALUE, text.clone()));
                            }
                        })
                        .trailing_part(TrailingPart::new(0), TrailingFace::Strong(word("Remove")), word("Remove"))
                });
            for (at, column, text) in edits.into_inner() {
                if let Some(entry) = self.items.get_mut(at) {
                    if column == DISPLAY {
                        entry.display = text;
                    } else {
                        entry.value = text;
                    }
                }
            }
            let removed = out.events.iter().find_map(|event| match event {
                CollectionEvent::Item(at, RowEvent::Trailing { .. }) => Some(*at),
                _ => None,
            });
            if let Some(at) = removed
                && at < self.items.len()
            {
                self.items.remove(at);
            }
            if self.items.len() < wordcraft_doc::control::MAX_LIST_ITEMS && clicked(&Button::new(word("Add")).show(ui)) {
                self.items.push(Entry::default());
            }
        });
    }
}

impl WordForm for ControlForm {
    fn key(&self) -> &'static str {
        "controlProperties"
    }

    fn title(&self) -> Label {
        word("Content Control Properties")
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Stack::new().show(ui, |ui, _| {
            text_field(ui, "title", "Title", &mut self.title);
            text_field(ui, "tag", "Tag", &mut self.tag);
            if !matches!(self.kind.as_str(), "checkBox" | "picture" | "repeatingSection" | "repeatingSectionItem") {
                text_field(ui, "placeholder", "Placeholder text", &mut self.placeholder);
            }
            check(ui, "Can't be deleted", &mut self.lock_delete);
            check(ui, "Contents can't be edited", &mut self.lock_edit);
            check(ui, "Remove the control once edited", &mut self.temporary);
            match self.kind.as_str() {
                "plainText" => check(ui, "Allow line breaks", &mut self.multi_line),
                "comboBox" | "dropDown" => self.entries(ui),
                "date" => {
                    text_field(ui, "format", "Date format", &mut self.format);
                    Row::new().show(ui, |ui, _| {
                        for pic in ["M/d/yyyy", "d.M.yyyy", "yyyy-MM-dd", "dddd, MMMM d, yyyy"] {
                            if clicked(&Button::new(Label::from(pic.to_string())).show(ui)) {
                                self.format = pic.to_string();
                            }
                        }
                    });
                }
                "checkBox" => {
                    text_field(ui, "checkedSymbol", "Checked symbol", &mut self.checked_symbol);
                    text_field(ui, "uncheckedSymbol", "Unchecked symbol", &mut self.unchecked_symbol);
                }
                _ => {}
            }
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("developer.properties".to_string(), self.params())]
    }
}

/// Opens the dialog for the control at the caret (`controlProperties`, dg.A2). With the caret in no control it says
/// so on the status line and opens nothing.
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match ControlForm::read(app) {
        Some(form) => Some(open_form(app, parent, form)),
        None => {
            app.status(tl!("Put the caret in a content control first."));
            None
        }
    }
}
