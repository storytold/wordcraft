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

/// Predefined Portuguese date and time format pictures, for Brazil and Portugal alike.
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

/// What the Date and Time preview list shows at the instant `unix`: document values, not
/// interface strings. The i18n audit recognizes these instead of treating month names, dates and
/// times as labels.
#[cfg(test)]
pub(crate) fn date_time_previews_at(unix: u64) -> impl Iterator<Item = String> {
    let offset = wordcraft_engine::cmd::local_offset_secs(unix);
    PT_DATE_FORMATS
        .iter()
        .chain(DATE_FORMATS.iter())
        .flat_map(move |picture| ["pt-BR", "en-US"].map(|lang| wordcraft_engine::cmd::insert::format_date_at_in(picture, Some(lang), unix, offset)))
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
        let saved = app.ui.date_time_language.as_str();
        let language = date_language(if saved.is_empty() { crate::i18n::current().code() } else { saved }).to_string();
        let list = formats_of(&language);
        let format = if list.contains(&app.ui.date_time_format.as_str()) { app.ui.date_time_format.clone() } else { list[0].to_string() };
        Self { format, language, update: app.ui.date_time_update }
    }
    /// Pictures available in the chosen language.
    fn formats(&self) -> &'static [&'static str] {
        formats_of(&self.language)
    }
    /// Engine command parameters corresponding to the current controls.
    pub fn params(&self) -> Value {
        json!({"format": self.format, "language": self.language, "update": self.update})
    }
}

/// The regions that write Portuguese like Portugal: Portugal and the regions CLDR gives `pt_PT` as
/// their parent locale (Angola, Switzerland, Cabo Verde, France, Equatorial Guinea, Guinea-Bissau,
/// Luxembourg, Macao, Mozambique, São Tomé and Príncipe, Timor-Leste).
const EUROPEAN_PORTUGUESE: [&str; 12] = ["PT", "AO", "CH", "CV", "FR", "GQ", "GW", "LU", "MO", "MZ", "ST", "TL"];

/// The dialog's language for a saved or interface language tag (`pt-PT`, `pt_PT.UTF-8`, `pt-pt`…):
/// Portuguese (Portugal) when a `pt` tag names one of [`EUROPEAN_PORTUGUESE`], Portuguese (Brazil)
/// for any other `pt` tag (`pt` alone, `pt-BR`), English (United States) otherwise. The subtags after
/// a singleton (an extension such as `pt-BR-t-pt-PT`, or private use such as `pt-x-pt`) name no
/// region of the text, and a POSIX encoding or modifier (`.UTF-8`, `@euro`) is not part of the tag.
fn date_language(tag: &str) -> &'static str {
    let tag = tag.trim().split(['.', '@']).next().unwrap_or("");
    let mut parts = tag.split(['-', '_']);
    if !parts.next().is_some_and(|p| p.eq_ignore_ascii_case("pt")) {
        return "en-US";
    }
    let european = parts.take_while(|p| p.len() > 1).any(|p| EUROPEAN_PORTUGUESE.iter().any(|r| r.eq_ignore_ascii_case(p)));
    if european { "pt-PT" } else { "pt-BR" }
}

/// The pictures offered for a dialog language.
fn formats_of(language: &str) -> &'static [&'static str] {
    if language.starts_with("pt-") { PT_DATE_FORMATS } else { DATE_FORMATS }
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
                const LANGUAGES: [&str; 3] = ["pt-BR", "pt-PT", "en-US"];
                let entries = [
                    Entry::new(0u8, word("Portuguese (Brazil)")),
                    Entry::new(1u8, word("Portuguese (Portugal)")),
                    Entry::new(2u8, word("English (United States)")),
                ];
                let current = Mixed::Same(LANGUAGES.iter().position(|l| *l == self.language).and_then(|i| u8::try_from(i).ok()).unwrap_or(2));
                let picked = Field::new(word("Language"))
                    .show(ui, |ui, _| Select::new(word("Language"), Bind::given(current), &entries).show(ui).value().copied())
                    .inner
                    .flatten();
                // A language change starts the format over from that language's first picture, but
                // Brazil and Portugal share their pictures, so one chosen there stays chosen.
                if let Some(language) = picked.and_then(|i| LANGUAGES.get(usize::from(i)))
                    && *language != self.language
                {
                    if formats_of(language) != self.formats() {
                        self.format = formats_of(language)[0].into();
                    }
                    self.language = (*language).into();
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
