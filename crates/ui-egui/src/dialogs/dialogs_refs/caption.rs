use craft::behavior::Mixed;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::Block;

use super::labels::{BUILT_IN_LABELS, NUMBER_FORMATS, SEPARATORS, own_labels};
use crate::WordApp;
use crate::dialogs::host::{Asks, ChildAnswer, Filed, WordForm, open_form};
use egui::Ui;

/// The body action that deletes the form's label from the preferences.
const DELETE_LABEL: u32 = 1;

/// The Caption dialog's fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptionForm {
    pub label: String,
    /// Built-in labels and the user's own (kept with the interface settings, like Word's
    /// Normal template keeps them).
    pub labels: Vec<String>,
    pub text: String,
    /// `below` or `above`.
    pub position: String,
    pub exclude_label: bool,
    /// `\*` format: `ARABIC`, `alphabetic`, `ALPHABETIC`, `roman`, `ROMAN`.
    pub format: String,
    pub chapter: bool,
    /// Heading level that starts a chapter (1–9).
    pub chapter_level: u8,
    pub separator: String,
}

impl CaptionForm {
    pub fn read(app: &WordApp) -> CaptionForm {
        let labels = own_labels(app);
        // In a table the caption is for the table, unless a picture or shape in it is selected.
        let in_table = wordcraft_engine::cmd::objects::object_selection(&app.session).is_none()
            && app.session.sel.focus.path.depth() > 0
            && app
                .session
                .doc
                .body
                .get(app.session.sel.focus.path.0.first().copied().unwrap_or(0) as usize)
                .is_some_and(|b| matches!(&**b, Block::Table(_)));
        let last = &app.ui.last_caption_label;
        let label = if labels.contains(last) { last.as_str() } else { "Figure" };
        CaptionForm {
            label: if in_table { "Table" } else { label }.into(),
            labels,
            text: String::new(),
            position: "below".into(),
            exclude_label: false,
            format: "ARABIC".into(),
            chapter: false,
            chapter_level: 1,
            separator: "-".into(),
        }
    }

    /// The number as it will show (`Figure 1-A`).
    pub fn preview(&self) -> String {
        let n = match self.format.as_str() {
            "alphabetic" => "a",
            "ALPHABETIC" => "A",
            "roman" => "i",
            "ROMAN" => "I",
            _ => "1",
        };
        let chapter = if self.chapter { format!("1{}", self.separator) } else { String::new() };
        let label = if self.exclude_label { String::new() } else { format!("{} ", self.label) };
        format!("{label}{chapter}{n}")
    }

    /// `references.caption` parameters.
    pub fn params(&self) -> Value {
        let mut v = json!({
            "label": self.label,
            "text": self.text.trim(),
            "position": self.position,
            "excludeLabel": self.exclude_label,
            "format": self.format,
        });
        if self.chapter {
            v["chapter"] = json!(self.chapter_level);
            v["separator"] = json!(self.separator);
        }
        v
    }
}

impl WordForm for CaptionForm {
    fn key(&self) -> &'static str {
        "caption"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Caption").to_string())
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("references.caption".to_string(), self.params())]
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let caption = Label::from(tl!("Caption:").to_string());
        if let Some(text) = Field::new(caption.clone())
            .show(ui, |ui, _| {
                TextField::new(caption.clone(), Bind::given(self.text.clone()))
                    .prefix(Label::from(self.preview()))
                    .placeholder(Label::from(tl!("Caption text").to_string()))
                    .show(ui)
                    .value()
                    .cloned()
            })
            .inner
            .flatten()
        {
            self.text = text;
        }
        let label_title = Label::from(tl!("Label:").to_string());
        let labels: Vec<Entry<String>> = self.labels.iter().map(|l| Entry::new(l.clone(), Label::from(tl!(l).to_string()))).collect();
        if let Some(label) = Field::new(label_title.clone())
            .show(ui, |ui, _| Select::new(label_title.clone(), Bind::given(Mixed::Same(self.label.clone())), &labels).show(ui).value().cloned())
            .inner
            .flatten()
        {
            self.label = label;
        }
        let position_title = Label::from(tl!("Position:").to_string());
        let positions = [
            Entry::new("below".to_string(), Label::from(tl!("Below the item").to_string())),
            Entry::new("above".to_string(), Label::from(tl!("Above the item").to_string())),
        ];
        if let Some(position) = Field::new(position_title.clone())
            .show(ui, |ui, _| {
                Select::new(position_title.clone(), Bind::given(Mixed::Same(self.position.clone())), &positions).show(ui).value().cloned()
            })
            .inner
            .flatten()
        {
            self.position = position;
        }
        let exclude = Label::from(tl!("Leave the label out of the caption").to_string());
        if let Some(on) = Checkbox::new(Bind::given(self.exclude_label)).label(exclude).show(ui).value().copied() {
            self.exclude_label = on;
        }
        if Button::new(Label::from(tl!("New Label…").to_string())).show(ui).clicked() {
            asks.push(Asks::Open("newLabel".to_string(), Value::Null));
        }
        let custom = !BUILT_IN_LABELS.contains(&self.label.as_str());
        if Button::new(Label::from(tl!("Delete Label").to_string())).enabled(custom).show(ui).clicked() {
            self.labels.retain(|l| *l != self.label);
            self.label = "Figure".into();
            asks.push(Asks::Act(DELETE_LABEL));
        }
        if Button::new(Label::from(tl!("Numbering…").to_string())).show(ui).clicked() {
            asks.push(Asks::Open("captionNumbering".to_string(), json!(NumberingForm(self.clone()))));
        }
        true
    }

    fn answer(&mut self, answer: ChildAnswer) {
        let ChildAnswer::Form { key, value } = answer else { return };
        match key.as_str() {
            "newLabel" => {
                if let Some(name) = serde_json::from_value::<NewLabelForm>(value).ok().and_then(|f| f.label()) {
                    if !self.labels.contains(&name) {
                        self.labels.push(name.clone());
                    }
                    self.label = name;
                }
            }
            "captionNumbering" => {
                if let Ok(NumberingForm(n)) = serde_json::from_value(value) {
                    self.format = n.format;
                    self.chapter = n.chapter;
                    self.chapter_level = n.chapter_level;
                    self.separator = n.separator;
                }
            }
            _ => {}
        }
    }

    /// Every preference write of the Caption dialog: the last label after OK, and the custom labels
    /// the form no longer lists after Delete Label.
    fn applied(&mut self, app: &mut WordApp, filed: Filed, _outcome: Result<&[Value], &str>) {
        match filed {
            Filed::Ok => app.ui.last_caption_label = self.label.clone(),
            Filed::Act(DELETE_LABEL) => app.ui.caption_labels.retain(|l| self.labels.contains(l)),
            _ => {}
        }
    }
}

/// Opens the child dialog `name` (`newLabel` or `captionNumbering`) over its Caption dialog.
pub(super) fn open_child(app: &mut WordApp, name: &str, params: &Value, caption: DialogId) -> DialogId {
    if name == "newLabel" {
        return open_form(app, Some(caption), NewLabelForm::default());
    }
    let form = serde_json::from_value(params.clone()).unwrap_or_else(|_| NumberingForm(CaptionForm::read(app)));
    open_form(app, Some(caption), form)
}

/// Caption › New Label…: the name of a label of the person's own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewLabelForm {
    pub name: String,
}

impl NewLabelForm {
    /// The name without quotes, backslashes and control characters, at most 40 characters; `None` when nothing is left.
    fn label(&self) -> Option<String> {
        let name: String = self.name.chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).take(40).collect::<String>().trim().to_string();
        (!name.is_empty()).then_some(name)
    }
}

impl WordForm for NewLabelForm {
    fn key(&self) -> &'static str {
        "newLabel"
    }

    fn title(&self) -> Label {
        Label::from(tl!("New Label").to_string())
    }

    fn commands(&self) -> Vec<(String, Value)> {
        Vec::new()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let label = Label::from(tl!("Label:").to_string());
        if let Some(name) = Field::new(label.clone())
            .show(ui, |ui, _| TextField::new(label.clone(), Bind::given(self.name.clone())).show(ui).value().cloned())
            .inner
            .flatten()
        {
            self.name = name;
        }
        self.label().is_some()
    }

    fn applied(&mut self, app: &mut WordApp, filed: Filed, _outcome: Result<&[Value], &str>) {
        if let (Filed::Ok, Some(name)) = (filed, self.label())
            && !BUILT_IN_LABELS.contains(&name.as_str())
            && !app.ui.caption_labels.contains(&name)
        {
            app.ui.caption_labels.push(name);
        }
    }
}

/// Caption › Numbering…: the Caption form's numbering fields, edited on a copy of the whole form
/// so the example shows the label.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NumberingForm(pub CaptionForm);

impl WordForm for NumberingForm {
    fn key(&self) -> &'static str {
        "captionNumbering"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Caption Numbering").to_string())
    }

    fn commands(&self) -> Vec<(String, Value)> {
        Vec::new()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let f = &mut self.0;
        let format_title = Label::from(tl!("Format:").to_string());
        let formats: Vec<Entry<String>> = NUMBER_FORMATS.iter().map(|(v, l)| Entry::new(v.to_string(), Label::from(l.to_string()))).collect();
        if let Some(format) = Field::new(format_title.clone())
            .show(ui, |ui, _| Select::new(format_title.clone(), Bind::given(Mixed::Same(f.format.clone())), &formats).show(ui).value().cloned())
            .inner
            .flatten()
        {
            f.format = format;
        }
        let chapter = Label::from(tl!("Add the chapter number").to_string());
        if let Some(on) = Checkbox::new(Bind::given(f.chapter)).label(chapter).show(ui).value().copied() {
            f.chapter = on;
        }
        let level_title = Label::from(tl!("Chapters begin at:").to_string());
        let levels: Vec<Entry<String>> =
            (1..=9u8).map(|l| Entry::new(l.to_string(), Label::from(crate::i18n::fmt(tl!("Heading {n}"), &[("n", &l.to_string())])))).collect();
        if let Some(level) = Field::new(level_title.clone())
            .show(ui, |ui, _| {
                Select::new(level_title.clone(), Bind::given(Mixed::Same(f.chapter_level.to_string())), &levels)
                    .enabled(f.chapter)
                    .show(ui)
                    .value()
                    .cloned()
            })
            .inner
            .flatten()
            .and_then(|l| l.parse().ok())
        {
            f.chapter_level = level;
        }
        let separator_title = Label::from(tl!("Separator:").to_string());
        let separators: Vec<Entry<String>> = SEPARATORS.iter().map(|(v, l)| Entry::new(v.to_string(), Label::from(l.to_string()))).collect();
        if let Some(separator) = Field::new(separator_title.clone())
            .show(ui, |ui, _| {
                Select::new(separator_title.clone(), Bind::given(Mixed::Same(f.separator.clone())), &separators)
                    .enabled(f.chapter)
                    .show(ui)
                    .value()
                    .cloned()
            })
            .inner
            .flatten()
        {
            f.separator = separator;
        }
        Text::new(crate::i18n::fmt(tl!("Example: {sample}"), &[("sample", &f.preview())])).show(ui);
        true
    }
}
