//! Date & Time insertion dialog (Word-style format and language selection).
use craft::behavior::Mixed;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::field::DATE_FORMATS;
use super::{list_index, word};
use crate::WordApp;
use crate::dialogs::host::{Asks, Filed, WordForm};

/// Predefined Portuguese (Brazil) date and time format pictures.
const PT_DATE_FORMATS: &[&str] = &[
    "dd/MM/yyyy",
    "dddd, d 'de' MMMM 'de' yyyy",
    "d 'de' MMMM 'de' yyyy",
    "dd/MM/yy",
    "yyyy-MM-dd",
    "d-MMM-yy",
    "dd.MM.yyyy",
    "d MMMM yyyy",
    "MMM-yy",
    "dd/MM/yyyy HH:mm",
    "dd/MM/yyyy HH:mm:ss",
    "HH:mm",
    "HH:mm:ss",
];

/// The date preview list consists of document values (not interface strings).
/// The i18n audit recognizes these dynamic values instead of treating month names as UI labels.
#[cfg(test)]
pub(crate) fn date_time_preview_matches(value: &str) -> bool {
    PT_DATE_FORMATS.iter().chain(DATE_FORMATS.iter()).any(|picture| {
        wordcraft_engine::cmd::insert::format_date_in(picture, Some("pt-BR")) == value
            || wordcraft_engine::cmd::insert::format_date_in(picture, Some("en-US")) == value
    })
}

/// State of the Word-like Insert > Date & Time dialog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DateTimeForm {
    /// Selected Word field picture, not a Rust strftime format.
    pub format: String,
    /// Language of the resulting text or DATE field.
    pub language: String,
    /// Whether to insert a refreshable field rather than fixed text.
    pub update: bool,
}
impl DateTimeForm {
    /// Restore a saved format/language or choose a sensible default for the UI language.
    pub fn new(app: &WordApp) -> Self {
        let language = if app.ui.date_time_language.is_empty() {
            if crate::i18n::current().code().to_ascii_lowercase().starts_with("pt") { "pt-BR" } else { "en-US" }.to_string()
        } else if app.ui.date_time_language.to_ascii_lowercase().starts_with("pt") {
            "pt-BR".to_string()
        } else {
            "en-US".to_string()
        };
        let list = if language == "pt-BR" { PT_DATE_FORMATS } else { DATE_FORMATS };
        let format = if list.contains(&app.ui.date_time_format.as_str()) { app.ui.date_time_format.clone() } else { list[0].to_string() };
        Self { format, language, update: app.ui.date_time_update }
    }
    /// Pictures available in the chosen language.
    fn formats(&self) -> &'static [&'static str] {
        if self.language == "pt-BR" { PT_DATE_FORMATS } else { DATE_FORMATS }
    }
    /// Engine command parameters corresponding to the current controls.
    pub fn params(&self) -> Value {
        json!({"format": self.format, "language": self.language, "update": self.update})
    }
}

impl WordForm for DateTimeForm {
    fn key(&self) -> &'static str {
        "dateTime"
    }

    fn title(&self) -> Label {
        word("Date and Time")
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Row::new().show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Text::new(word("Date formats:")).show(ui);
                let formats = self.formats();
                let samples: Vec<String> = formats.iter().map(|p| wordcraft_engine::cmd::insert::format_date_in(p, Some(&self.language))).collect();
                let current = formats.iter().position(|p| *p == self.format);
                if let Some(at) = list_index(ui, &word("Date formats:"), &samples, current).selected
                    && let Some(picture) = formats.get(at)
                {
                    self.format = (*picture).to_string();
                }
            });
            Stack::new().show(ui, |ui, _| {
                let entries = [Entry::new(0u8, word("Portuguese (Brazil)")), Entry::new(1u8, word("English (United States)"))];
                let current = Mixed::Same(if self.language == "pt-BR" { 0u8 } else { 1u8 });
                let picked = Field::new(word("Language"))
                    .show(ui, |ui, _| Select::new(word("Language"), Bind::given(current), &entries).show(ui).value().copied())
                    .inner
                    .flatten();
                // A language change starts the format over from that language's first picture.
                match picked {
                    Some(0) if self.language != "pt-BR" => {
                        self.language = "pt-BR".into();
                        self.format = PT_DATE_FORMATS[0].into();
                    }
                    Some(1) if self.language != "en-US" => {
                        self.language = "en-US".into();
                        self.format = DATE_FORMATS[0].into();
                    }
                    _ => {}
                }
                Checkbox::new(Bind::bound(&mut self.update)).label(word("Update automatically")).show(ui);
            });
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("insert.dateTime".to_string(), self.params())]
    }

    fn extra_buttons(&self) -> Vec<FooterButton> {
        vec![FooterButton::new(word("Set as Default"), DialogRole::Other).stays_open(true)]
    }

    fn applied(&mut self, app: &mut WordApp, filed: Filed, _outcome: Result<&[Value], &str>) {
        if matches!(filed, Filed::Extra(0)) {
            app.ui.date_time_format = self.format.clone();
            app.ui.date_time_language = self.language.clone();
            app.ui.date_time_update = self.update;
            app.status(tl!("Default format saved."));
        }
    }
}
