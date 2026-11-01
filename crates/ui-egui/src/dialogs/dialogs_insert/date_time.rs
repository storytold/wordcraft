//! Date & Time insertion dialog (Word-style format and language selection).
use super::{DATE_FORMATS, Serialize, Stroke, Tokens, Ui, Value, WordApp, buttons, json};

/// Word-style Insert > Date & Time dialog.
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

/// State of the Word-like Insert > Date & Time dialog, including an optional validation error.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DateTimeForm {
    /// Selected Word field picture, not a Rust strftime format.
    pub format: String,
    /// Language of the resulting text or DATE field.
    pub language: String,
    /// Whether to insert a refreshable field rather than fixed text.
    pub update: bool,
    /// Last insertion failure; retain the window so the user can change the choice.
    #[serde(skip)]
    pub error: Option<String>,
}
impl DateTimeForm {
    /// Restore a saved format/language or choose a sensible default for the UI language.
    pub fn new(app: &WordApp) -> Self {
        let language = if app.ui.date_time_language.is_empty() {
            if crate::i18n::current().code().starts_with("pt") { "pt-BR" } else { "en-US" }.to_string()
        } else {
            app.ui.date_time_language.clone()
        };
        let language = if language == "pt-BR" { language } else { "en-US".into() };
        let list = if language == "pt-BR" { PT_DATE_FORMATS } else { DATE_FORMATS };
        let format = if list.contains(&app.ui.date_time_format.as_str()) { app.ui.date_time_format.clone() } else { list[0].to_string() };
        Self { format, language, update: app.ui.date_time_update, error: None }
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
/// Draw the interactive format list, language picker and update checkbox; close on successful
/// insertion or Cancel, keeping errors visible rather than silently dismissing the dialog.
pub(super) fn date_time_ui(app: &mut WordApp, ui: &mut Ui, f: &mut DateTimeForm) -> bool {
    let tokens = Tokens::get(ui.ctx());
    ui.set_width(520.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(350.0);
            ui.label(tl!("Date formats:"));
            egui::Frame::new().stroke(Stroke::new(1.0, tokens.border)).inner_margin(2.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("date_time_formats").max_height(245.0).auto_shrink([false, false]).show(ui, |ui| {
                    for picture in f.formats() {
                        let sample = wordcraft_engine::cmd::insert::format_date_in(picture, Some(&f.language));
                        if ui.selectable_label(f.format == *picture, sample).on_hover_text(*picture).clicked() {
                            f.format = (*picture).to_string();
                        }
                    }
                });
            });
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.label(tl!("Language"));
            craft_ui::ComboBox::from_id_salt("date_time_lang")
                .selected_text(if f.language == "pt-BR" { tl!("Portuguese (Brazil)") } else { tl!("English (United States)") })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(f.language == "pt-BR", tl!("Portuguese (Brazil)")).clicked() {
                        f.language = "pt-BR".into();
                        f.format = PT_DATE_FORMATS[0].into();
                    }
                    if ui.selectable_label(f.language == "en-US", tl!("English (United States)")).clicked() {
                        f.language = "en-US".into();
                        f.format = DATE_FORMATS[0].into();
                    }
                });
        });
    });
    ui.add_space(8.0);
    ui.checkbox(&mut f.update, tl!("Update automatically"));
    ui.add_space(6.0);
    if ui.button(tl!("Set as Default")).clicked() {
        app.ui.date_time_format = f.format.clone();
        app.ui.date_time_language = f.language.clone();
        app.ui.date_time_update = f.update;
    }
    if let Some(error) = &f.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let (ok, cancel) = buttons(ui, "OK", "Cancel", true);
    if ok {
        match app.run("insert.dateTime", f.params()) {
            Ok(_) => return true,
            Err(error) => {
                f.error = Some(error.to_string());
                return false;
            }
        }
    }
    cancel
}
