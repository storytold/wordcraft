//! Home › Font › Phonetic Guide: ruby text over the selected text (`format.phonetic`).

use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::host::{Asks, WordForm, open_form};
use crate::WordApp;

/// The ruby alignments (OOXML names) and their words.
const ALIGNS: [(&str, &str); 6] = [
    ("center", "Center"),
    ("distributeLetter", "Distribute letters"),
    ("distributeSpace", "Distribute spacing"),
    ("left", "Left"),
    ("right", "Right"),
    ("rightVertical", "Right (vertical)"),
];

/// Phonetic Guide's fields.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct PhoneticForm {
    /// The selected text the guide goes over.
    pub base: String,
    pub ruby: String,
    /// OOXML alignment name (`center`, `distributeLetter`…).
    pub align: String,
    pub size: Option<f64>,
    pub offset: Option<f64>,
    /// The selection already has a phonetic guide (Remove is offered).
    pub existing: bool,
}

impl PhoneticForm {
    /// The selection's base text and guide.
    pub(crate) fn read(app: &mut WordApp) -> PhoneticForm {
        let v = wordcraft_engine::cmd::phonetic::state(&app.session);
        let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        PhoneticForm {
            base: s("base"),
            ruby: s("ruby"),
            align: s("align"),
            size: Some(f("size")),
            offset: Some(f("offset")),
            existing: v.get("existing").and_then(Value::as_bool).unwrap_or(false),
        }
    }
}

impl WordForm for PhoneticForm {
    fn key(&self) -> &'static str {
        "phonetic"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Phonetic Guide"))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Field::new(Label::from(tl!("Base text:"))).show(ui, |ui, _| {
            Text::new(Label::from(self.base.clone())).show(ui);
        });
        Field::new(Label::from(tl!("Ruby text:"))).show(ui, |ui, _| {
            TextField::new(Label::from(tl!("Ruby text:")), Bind::bound(&mut self.ruby)).show(ui);
        });
        Field::new(Label::from(tl!("Alignment:"))).show(ui, |ui, _| {
            let entries: Vec<Entry<String>> = ALIGNS.iter().map(|(k, l)| Entry::new((*k).to_string(), Label::from(tl!(l)))).collect();
            let current = if self.align.is_empty() { "center".to_string() } else { self.align.clone() };
            if let Some(a) = Select::new(Label::from(tl!("Alignment:")), Bind::given(current), &entries).show(ui).value().cloned() {
                self.align = a;
            }
        });
        Field::new(Label::from(tl!("Offset (pt):"))).show(ui, |ui, _| {
            NumberField::new(Label::from(tl!("Offset (pt):")), Bind::bound(&mut self.offset))
                .unit_kind(UnitKind::Points)
                .range(-100.0..=100.0)
                .show(ui);
        });
        Field::new(Label::from(tl!("Size (pt):"))).show(ui, |ui, _| {
            NumberField::new(Label::from(tl!("Size (pt):")), Bind::bound(&mut self.size)).unit_kind(UnitKind::Points).range(1.0..=200.0).show(ui);
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let align = if self.align.is_empty() { "center" } else { self.align.as_str() };
        vec![(
            "format.phonetic".into(),
            json!({"ruby": self.ruby, "align": align, "size": self.size.unwrap_or(12.0), "offset": self.offset.unwrap_or(0.0)}),
        )]
    }

    fn extra_buttons(&self) -> Vec<FooterButton> {
        if self.existing { vec![FooterButton::new(Label::from(tl!("Remove")), DialogRole::Destructive)] } else { Vec::new() }
    }

    fn extra(&mut self, _at: usize) -> Vec<(String, Value)> {
        vec![("format.phonetic".into(), json!({"ruby": null}))]
    }
}

/// Opens Phonetic Guide (name `phonetic`).
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let form = PhoneticForm::read(app);
    Some(open_form(app, parent, form))
}
