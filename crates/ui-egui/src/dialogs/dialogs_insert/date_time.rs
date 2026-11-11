//! Date & Time insertion dialog (Word-style format and language selection).
use egui::{Stroke, Ui};
use serde::Serialize;
use serde_json::{Value, json};

use super::buttons;
use super::field::DATE_FORMATS;
use crate::WordApp;
use crate::theme::Tokens;

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
    /// Brief status confirmation message (e.g. after setting default preferences).
    #[serde(skip)]
    pub status: Option<&'static str>,
}
impl DateTimeForm {
    /// Restore a saved format/language or choose a sensible default for the UI language.
    pub fn new(app: &WordApp) -> Self {
        let saved = app.ui.date_time_language.as_str();
        let language = date_language(if saved.is_empty() { crate::i18n::current().code() } else { saved }).to_string();
        let list = formats_of(&language);
        let format = if list.contains(&app.ui.date_time_format.as_str()) { app.ui.date_time_format.clone() } else { list[0].to_string() };
        Self { format, language, update: app.ui.date_time_update, error: None, status: None }
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
/// The dialog's language for a saved or interface language tag (`pt-PT`, `pt_PT.UTF-8`, `pt-pt`…):
/// Portuguese (Portugal) when a `pt` tag names the region `PT`, Portuguese (Brazil) for any other
/// `pt` tag, English (United States) otherwise. A private-use part (`pt-x-pt`) names no region.
fn date_language(tag: &str) -> &'static str {
    let mut parts = tag.trim().split(['-', '_', '.', '@']);
    if !parts.next().is_some_and(|p| p.eq_ignore_ascii_case("pt")) {
        return "en-US";
    }
    if parts.take_while(|p| !p.eq_ignore_ascii_case("x")).any(|p| p.eq_ignore_ascii_case("pt")) { "pt-PT" } else { "pt-BR" }
}

/// The pictures offered for a dialog language.
fn formats_of(language: &str) -> &'static [&'static str] {
    if language.starts_with("pt-") { PT_DATE_FORMATS } else { DATE_FORMATS }
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
            let selected = match f.language.as_str() {
                "pt-BR" => tl!("Portuguese (Brazil)"),
                "pt-PT" => tl!("Portuguese (Portugal)"),
                _ => tl!("English (United States)"),
            };
            craft_ui::ComboBox::from_id_salt("date_time_lang").selected_text(selected).show_ui(ui, |ui| {
                for (tag, name) in [("pt-BR", tl!("Portuguese (Brazil)")), ("pt-PT", tl!("Portuguese (Portugal)"))] {
                    if ui.selectable_label(f.language == tag, name).clicked() {
                        // The two share their pictures, so a chosen one stays chosen.
                        if !f.language.starts_with("pt-") {
                            f.format = PT_DATE_FORMATS[0].into();
                        }
                        f.language = tag.into();
                    }
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
        f.status = Some(tl!("Default format saved."));
    }
    if let Some(status) = f.status {
        ui.colored_label(tokens.text_dim, status);
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
