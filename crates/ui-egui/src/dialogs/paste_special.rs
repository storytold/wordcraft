use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, FieldLayout, Show as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::runtime::env::Label;
use serde::{Deserialize, Serialize};

use super::dialogs_insert::select_index;
use super::host::{Asks, WordForm, open_form};
use super::{Value, WordApp};
use egui::Ui;
use serde_json::json;

/// Edit › Paste Special: the formats the clipboard offers, the one picked and the payload the paste
/// runs with. The payload is kept as JSON so the form stays serialisable.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PasteSpecialForm {
    /// The format ids on offer, from `paste::available`.
    formats: Vec<String>,
    /// The index into `formats` that is picked.
    choice: usize,
    /// The clipboard text, html, rtf, picture and the ownership flag the paste needs.
    payload: Value,
    /// Why nothing can be pasted, when `formats` is empty.
    note: String,
    /// The resolution of a picture of WordCraft's own copy, DPI; 0 is the default
    /// (`paste::PICTURE_DEFAULT_DPI`).
    #[serde(default)]
    dpi: u32,
}

impl PasteSpecialForm {
    /// The form for a clipboard `payload` (`text`, `html`, `rtf`; empty is WordCraft's own copy).
    pub(crate) fn read(app: &WordApp, payload: &Value) -> Self {
        let formats: Vec<String> = wordcraft_engine::cmd::paste::available(&app.session, payload).into_iter().map(str::to_string).collect();
        let mut kept = json!({});
        for k in ["text", "html", "rtf", "picture"] {
            if let Some(t) = payload.get(k).and_then(Value::as_str) {
                kept[k] = json!(t);
            }
        }
        if let Some(own) = payload.get("privateClipboard").and_then(Value::as_bool) {
            kept["privateClipboard"] = json!(own);
        }
        let note = if formats.is_empty() { crate::system_clipboard::nothing_to_paste(app) } else { String::new() };
        PasteSpecialForm { formats, choice: 0, payload: kept, note, dpi: 0 }
    }

    /// Whether `format` pastes a picture drawn of WordCraft's own copy (#7743): Picture with no
    /// picture on the system clipboard.
    fn draws_our_copy(&self, format: &str) -> bool {
        format == "picture" && self.payload.get("picture").and_then(Value::as_str).is_none_or(str::is_empty)
    }

    /// The resolution picked for a picture of our own copy, DPI.
    fn dpi(&self) -> u32 {
        if self.dpi == 0 { wordcraft_engine::cmd::paste::PICTURE_DEFAULT_DPI } else { self.dpi }
    }

    /// The text under the list when the clipboard holds nothing: the engine's wording, sentence-cased.
    fn nothing_text(&self) -> String {
        if self.note.is_empty() || self.note == "the clipboard is empty" {
            return tl!("The clipboard is empty.").to_string();
        }
        let mut c = self.note.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect::<String>() + ".").unwrap_or_default()
    }
}

impl WordForm for PasteSpecialForm {
    fn key(&self) -> &'static str {
        "pasteSpecial"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Paste Special"))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        if self.formats.is_empty() {
            Notice::new(craft::theme::variant::Tone::Neutral, Label::from(self.nothing_text())).show(ui);
            return true;
        }
        let formats = self.formats.clone();
        RadioGroup::new(Label::from(tl!("Paste as:")), Bind::bound(&mut self.choice)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, cx| {
                for (at, f) in formats.iter().enumerate() {
                    let label = wordcraft_engine::cmd::paste::FORMATS.iter().find(|(id, _)| id == f).map(|(_, l)| *l).unwrap_or(f.as_str());
                    cx.labelled(ui, at, Label::from(tl!(label).to_owned()), SideLabel::new().layout(FieldLayout::After), Radio::new(at));
                }
            });
        });
        if let Some(f) = formats.get(self.choice) {
            Text::new(Label::from(tl!("Result"))).show(ui);
            let hint = match f.as_str() {
                "formatted" => "Keeps the fonts and formatting of the copied text.",
                "rtf" => "Reads the clipboard's rich text and keeps its formatting.",
                "html" => "Reads the clipboard's web content and keeps its formatting.",
                "picture" if self.draws_our_copy(f) => "Pastes the copied text and tables as a picture.",
                "picture" => "Pastes the clipboard's picture.",
                _ => "Drops all formatting and pastes plain text.",
            };
            Text::new(Label::from(tl!(hint))).show(ui);
            if self.draws_our_copy(f) {
                let choices = wordcraft_engine::cmd::paste::PICTURE_DPI_CHOICES;
                let rows: Vec<Label> = choices.iter().map(|d| Label::from(crate::i18n::fmt(tl!("{dpi} DPI"), &[("dpi", &d.to_string())]))).collect();
                let current = choices.iter().position(|&d| d == self.dpi());
                if let Some(at) = select_index(ui, &Label::from(tl!("Resolution:")), &rows, current)
                    && let Some(&d) = choices.get(at)
                {
                    self.dpi = d;
                }
            }
        }
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let Some(f) = self.formats.get(self.choice) else {
            return Vec::new();
        };
        let mut params = self.payload.clone();
        params["as"] = json!(f);
        if self.draws_our_copy(f) {
            params["dpi"] = json!(self.dpi());
        }
        vec![("edit.pasteSpecial".to_string(), params)]
    }
}

/// Opens Paste Special over the clipboard payload in `params` (an empty object for WordCraft's own copy).
pub(crate) fn open(app: &mut WordApp, _name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let form = PasteSpecialForm::read(app, params);
    Some(open_form(app, parent, form))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #7743: Picture of WordCraft's own copy pastes at the resolution picked (300 DPI to start);
    /// a picture on the system clipboard is pasted as it is, with no resolution to pick.
    #[test]
    fn a_picture_of_our_copy_takes_the_resolution_picked() {
        let paste = |params: Value| vec![("edit.pasteSpecial".to_string(), params)];
        let mut f = PasteSpecialForm { formats: vec!["formatted".into(), "picture".into()], choice: 1, payload: json!({}), ..Default::default() };
        assert_eq!(f.commands(), paste(json!({"as": "picture", "dpi": 300})));
        f.dpi = 1200;
        assert_eq!(f.commands(), paste(json!({"as": "picture", "dpi": 1200})));
        f.choice = 0;
        assert_eq!(f.commands(), paste(json!({"as": "formatted"})), "only a picture of the copy has a resolution");
        f.choice = 1;
        f.payload = json!({"picture": "iVBORw0KGgo="});
        assert_eq!(f.commands(), paste(json!({"picture": "iVBORw0KGgo=", "as": "picture"})));
    }
}
