//! Layout, Review and Mailings dialogs (#407): Line Numbers (Layout › Line Numbers › Line
//! Numbering Options…), Hyphenation (Hyphenation Options… and Manual…), Language (Review ›
//! Language), Envelopes and Labels (Mailings › Create), Formula (Table Layout › Data, #1201)
//! and Sort (Table Layout › Data, #1200). Each ends by running a command (`layout.lineNumbers`,
//! `layout.hyphenation`, `layout.manualHyphenation` with `caret.set` and `text.optionalHyphen`,
//! `review.language`, `mailings.envelopes`, `mailings.labels`, `table.formula`, `table.sort`), so
//! scripts and agents get the same result without the dialog. Shown through
//! [`crate::dialogs::Dialog::Layout`].

use egui::{Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};

use wordcraft_doc::section::LineNumberRestart;
use wordcraft_engine::cmd::mailings::{ENVELOPE_SIZES, LABEL_PRODUCTS};

use crate::WordApp;
use crate::options::MeasureExt;
use crate::theme::semibold;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum LayoutDialog {
    LineNumbers(LineNumbersForm),
    Hyphenation(HyphenationForm),
    ManualHyphenation(ManualForm),
    Language(LanguageForm),
    Envelopes(EnvelopeForm),
    Labels(LabelsForm),
    Formula(FormulaForm),
    Sort(SortForm),
}

impl LayoutDialog {
    pub fn name(&self) -> &'static str {
        match self {
            LayoutDialog::LineNumbers(_) => "lineNumbers",
            LayoutDialog::Hyphenation(_) => "hyphenation",
            LayoutDialog::ManualHyphenation(_) => "manualHyphenation",
            LayoutDialog::Language(_) => "language",
            LayoutDialog::Envelopes(_) => "envelopes",
            LayoutDialog::Labels(_) => "labels",
            LayoutDialog::Formula(_) => "tableFormula",
            LayoutDialog::Sort(_) => "tableSort",
        }
    }

    /// The window title (English; the caller translates it).
    pub fn title(&self) -> &'static str {
        match self {
            LayoutDialog::LineNumbers(_) => "Line Numbers",
            LayoutDialog::Hyphenation(_) => "Hyphenation",
            LayoutDialog::ManualHyphenation(_) => "Manual Hyphenation",
            LayoutDialog::Language(_) => "Language",
            LayoutDialog::Envelopes(_) => "Envelopes",
            LayoutDialog::Labels(_) => "Labels",
            LayoutDialog::Formula(_) => "Formula",
            LayoutDialog::Sort(_) => "Sort",
        }
    }
}

/// The dialog `ui.dialog` opens by `name`.
pub fn open(name: &str, app: &mut WordApp) -> Option<LayoutDialog> {
    Some(match name {
        "lineNumbers" => LayoutDialog::LineNumbers(LineNumbersForm::read(app)),
        "hyphenation" => LayoutDialog::Hyphenation(HyphenationForm::read(app)),
        "manualHyphenation" => LayoutDialog::ManualHyphenation(ManualForm::start(app)),
        "language" => LayoutDialog::Language(LanguageForm::read(app)),
        "envelopes" => LayoutDialog::Envelopes(EnvelopeForm::read(app)),
        "labels" => LayoutDialog::Labels(LabelsForm::read(app)),
        "tableFormula" => LayoutDialog::Formula(FormulaForm::read(app)?),
        "tableSort" => LayoutDialog::Sort(SortForm::read(app)?),
        _ => return None,
    })
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut LayoutDialog) -> bool {
    match d {
        LayoutDialog::LineNumbers(f) => line_numbers_ui(app, ui, f),
        LayoutDialog::Hyphenation(f) => hyphenation_ui(app, ui, f),
        LayoutDialog::ManualHyphenation(f) => manual_ui(app, ui, f),
        LayoutDialog::Language(f) => language_ui(app, ui, f),
        LayoutDialog::Envelopes(f) => envelopes_ui(app, ui, f),
        LayoutDialog::Labels(f) => labels_ui(app, ui, f),
        LayoutDialog::Formula(f) => formula_ui(app, ui, f),
        LayoutDialog::Sort(f) => sort_ui(app, ui, f),
    }
}

/// The accent button and Cancel; returns (ok, cancel).
fn buttons(ui: &mut Ui, ok: &str, ok_enabled: bool) -> (bool, bool) {
    let mut r = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                r.1 = true;
            }
            let okb = ui.add_enabled(ok_enabled, crate::widgets::primary_button(ui, tl!(ok)));
            if okb.clicked() {
                r.0 = true;
            }
        });
    });
    r
}

fn heading(ui: &mut Ui, s: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
}

/// Runs a dialog's command; a refusal shows in the status bar and keeps the dialog open.
fn run(app: &mut WordApp, id: &str, params: Value) -> bool {
    match app.run(id, params) {
        Ok(_) => true,
        Err(e) => {
            app.status(e);
            false
        }
    }
}

fn unit() -> wordcraft_geom::Unit {
    crate::options::unit()
}

// ---------------------------------------------------------------------------------------------
// Line Numbers

/// The Line Numbers dialog's fields. `distance` is in the interface unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineNumbersForm {
    pub on: bool,
    pub start: u32,
    /// From text: automatic, or `distance`.
    pub auto_distance: bool,
    pub distance: f32,
    pub count_by: u32,
    /// `restartPage`, `restartSection` or `continuous`.
    pub restart: String,
}

impl LineNumbersForm {
    pub fn read(app: &WordApp) -> LineNumbersForm {
        let sp = wordcraft_engine::cmd::page::sect(&app.session);
        let l = sp.line_numbers.clone();
        let cur = l.clone().unwrap_or_default();
        LineNumbersForm {
            on: l.is_some(),
            start: cur.start.max(1),
            auto_distance: cur.distance <= 0.0,
            distance: if cur.distance > 0.0 { cur.distance / unit().pt_per_unit() } else { 0.25 * 72.0 / unit().pt_per_unit() },
            count_by: cur.count_by.max(1),
            restart: match cur.restart {
                LineNumberRestart::Page => "restartPage",
                LineNumberRestart::Section => "restartSection",
                LineNumberRestart::Continuous => "continuous",
            }
            .into(),
        }
    }

    /// `layout.lineNumbers` parameters.
    pub fn params(&self) -> Value {
        if !self.on {
            return json!({"value": "none"});
        }
        let distance = if self.auto_distance { 0.0 } else { (self.distance * unit().pt_per_unit()).max(0.0) };
        json!({"value": self.restart, "start": self.start, "countBy": self.count_by, "distance": distance})
    }
}

fn line_numbers_ui(app: &mut WordApp, ui: &mut Ui, f: &mut LineNumbersForm) -> bool {
    ui.checkbox(&mut f.on, tl!("Add line numbering"));
    ui.add_enabled_ui(f.on, |ui| {
        egui::Grid::new("lnum").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label(tl!("Start at:"));
            ui.add(egui::DragValue::new(&mut f.start).range(1..=32_767));
            ui.end_row();
            ui.label(tl!("From text:"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut f.auto_distance, tl!("Auto"));
                ui.add_enabled(
                    !f.auto_distance,
                    egui::DragValue::new(&mut f.distance).speed(0.01).range(0.0..=22.0 * 72.0 / unit().pt_per_unit()).measure(unit()).max_decimals(2),
                );
            });
            ui.end_row();
            ui.label(tl!("Count by:"));
            ui.add(egui::DragValue::new(&mut f.count_by).range(1..=100));
            ui.end_row();
        });
        heading(ui, "Numbering:");
        for (v, l) in [("restartPage", "Restart Each Page"), ("restartSection", "Restart Each Section"), ("continuous", "Continuous")] {
            ui.radio_value(&mut f.restart, v.to_string(), tl!(l));
        }
    });
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        return run(app, "layout.lineNumbers", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Hyphenation

/// The Hyphenation dialog's fields. `zone` is in the interface unit; `limit` 0 = no limit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HyphenationForm {
    pub auto: bool,
    pub caps: bool,
    pub zone: f32,
    pub limit: u32,
}

impl HyphenationForm {
    pub fn read(app: &WordApp) -> HyphenationForm {
        let s = &app.session.doc.settings;
        HyphenationForm {
            auto: s.auto_hyphenation,
            caps: s.hyphenate_caps,
            zone: s.hyphenation_zone.max(0.0) / unit().pt_per_unit(),
            limit: s.consecutive_hyphen_limit,
        }
    }

    /// `layout.hyphenation` parameters.
    pub fn params(&self) -> Value {
        json!({"value": self.auto, "caps": self.caps, "zone": (self.zone * unit().pt_per_unit()).max(0.0), "limit": self.limit})
    }
}

fn hyphenation_ui(app: &mut WordApp, ui: &mut Ui, f: &mut HyphenationForm) -> bool {
    ui.checkbox(&mut f.auto, tl!("Automatically hyphenate document"));
    ui.checkbox(&mut f.caps, tl!("Hyphenate words in CAPS"));
    egui::Grid::new("hyph").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Hyphenation zone:"));
        ui.add(egui::DragValue::new(&mut f.zone).speed(0.01).range(0.0..=22.0 * 72.0 / unit().pt_per_unit()).measure(unit()).max_decimals(2));
        ui.end_row();
        ui.label(tl!("Limit consecutive hyphens to:"));
        let no_limit = tl!("No limit").to_string();
        ui.add(
            egui::DragValue::new(&mut f.limit).range(0..=99).custom_formatter(move |n, _| if n < 0.5 { no_limit.clone() } else { format!("{n:.0}") }),
        );
        ui.end_row();
    });
    ui.add_space(6.0);
    let manual = ui.button(tl!("Manual…")).clicked();
    let (ok, cancel) = buttons(ui, "OK", true);
    if (ok || manual) && !run(app, "layout.hyphenation", f.params()) {
        return false;
    }
    // Manual… keeps what was set here and moves on to manual hyphenation.
    if manual {
        app.dialog = crate::dialogs::Dialog::open("manualHyphenation", app);
    }
    ok || cancel || manual
}

// ---------------------------------------------------------------------------------------------
// Manual hyphenation

/// One word at a time: where it may be hyphenated and where the hyphen goes.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualForm {
    pub word: String,
    /// Where the word starts.
    pub pos: Value,
    /// Char indices a hyphen may go before.
    pub points: Vec<usize>,
    pub choice: usize,
    /// No more words.
    pub done: bool,
}

impl ManualForm {
    /// From the selection, or from the start of the document.
    pub fn start(app: &mut WordApp) -> ManualForm {
        let from = if app.session.sel.is_collapsed() { Value::Null } else { wordcraft_engine::cmd::pos_json(&app.session.sel.ordered().0) };
        let from = if from.is_null() { json!({"block": 0, "off": 0}) } else { from };
        Self::next(app, from)
    }

    /// The next word from `from`.
    pub fn next(app: &mut WordApp, from: Value) -> ManualForm {
        let c = app.session.run("layout.manualHyphenation", &json!({"from": from})).unwrap_or_else(|_| json!({"done": true}));
        let points: Vec<usize> =
            c.get("points").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|n| n as usize).collect()).unwrap_or_default();
        // The word is selected, so the page shows which one is asked about.
        if let (Some(pos), Some(word)) = (c.get("pos"), c.get("word").and_then(Value::as_str)) {
            let end = with_off(pos, pos.get("off").and_then(Value::as_u64).unwrap_or(0).saturating_add(word.len() as u64));
            let _ = app.session.run("select.range", &json!({"anchor": pos, "focus": end}));
        }
        ManualForm {
            word: c.get("word").and_then(Value::as_str).unwrap_or("").to_string(),
            pos: c.get("pos").cloned().unwrap_or(Value::Null),
            choice: c.get("suggest").and_then(Value::as_u64).map(|n| n as usize).or_else(|| points.last().copied()).unwrap_or(0),
            points,
            done: c.get("done").and_then(Value::as_bool).unwrap_or(false) || c.get("word").is_none(),
        }
    }

    /// The word's position plus `bytes`.
    fn at(&self, bytes: usize) -> Value {
        let off = self.pos.get("off").and_then(Value::as_u64).unwrap_or(0);
        with_off(&self.pos, off.saturating_add(bytes as u64))
    }

    fn byte_of(&self, ch: usize) -> usize {
        self.word.char_indices().nth(ch).map_or(self.word.len(), |(i, _)| i)
    }

    /// Yes: an optional hyphen at the chosen point, then the next word.
    pub fn accept(&mut self, app: &mut WordApp) {
        let at = self.at(self.byte_of(self.choice));
        if run(app, "caret.set", json!({"pos": at})) && run(app, "text.optionalHyphen", json!({})) {
            let after = self.at(self.word.len() + '\u{ad}'.len_utf8());
            *self = Self::next(app, after);
        }
    }

    /// No: leave the word alone and go to the next one.
    pub fn skip(&mut self, app: &mut WordApp) {
        let after = self.at(self.word.len());
        *self = Self::next(app, after);
    }
}

/// A position (`{"story", "path", "off"}`) at another offset in the same paragraph.
fn with_off(pos: &Value, off: u64) -> Value {
    let mut p = pos.as_object().cloned().unwrap_or_default();
    p.insert("off".into(), json!(off));
    Value::Object(p)
}

fn manual_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ManualForm) -> bool {
    if f.done {
        ui.label(tl!("Manual hyphenation is complete."));
        ui.add_space(8.0);
        return ui.button(tl!("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter));
    }
    ui.label(tl!("Hyphenate at:"));
    // The word with every possible break; the chosen one is the hyphen that will be inserted.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut from = 0;
        for pt in f.points.clone() {
            ui.label(egui::RichText::new(f.word.chars().skip(from).take(pt.saturating_sub(from)).collect::<String>()).size(16.0));
            if ui.selectable_label(f.choice == pt, egui::RichText::new("-").size(16.0)).clicked() {
                f.choice = pt;
            }
            from = pt;
        }
        ui.label(egui::RichText::new(f.word.chars().skip(from).collect::<String>()).size(16.0));
    });
    ui.add_space(8.0);
    let mut close = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            if ui.button(tl!("No")).clicked() {
                f.skip(app);
            }
            if ui.add(crate::widgets::primary_button(ui, tl!("Yes"))).clicked() {
                f.accept(app);
            }
        });
    });
    close
}

// ---------------------------------------------------------------------------------------------
// Language

/// Proofing languages offered, by BCP 47 tag, with their English names (translated for display).
pub const LANGUAGES: [(&str, &str); 31] = [
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("de-DE", "German (Germany)"),
    ("de-AT", "German (Austria)"),
    ("de-CH", "German (Switzerland)"),
    ("fr-FR", "French (France)"),
    ("es-ES", "Spanish (Spain)"),
    ("es-MX", "Spanish (Mexico)"),
    ("it-IT", "Italian"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("pt-PT", "Portuguese (Portugal)"),
    ("nl-NL", "Dutch"),
    ("sv-SE", "Swedish"),
    ("da-DK", "Danish"),
    ("nb-NO", "Norwegian (Bokmål)"),
    ("fi-FI", "Finnish"),
    ("et-EE", "Estonian"),
    ("pl-PL", "Polish"),
    ("cs-CZ", "Czech"),
    ("uk-UA", "Ukrainian"),
    ("ru-RU", "Russian"),
    ("sr-Cyrl-RS", "Serbian (Cyrillic)"),
    ("sr-Latn-RS", "Serbian (Latin)"),
    ("el-GR", "Greek"),
    ("tr-TR", "Turkish"),
    ("ja-JP", "Japanese"),
    ("zh-CN", "Chinese (Simplified)"),
    ("zh-TW", "Chinese (Traditional)"),
    ("ko-KR", "Korean"),
    ("ar-SA", "Arabic"),
    ("he-IL", "Hebrew"),
];

/// The Language dialog's fields.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageForm {
    pub lang: String,
    pub no_proof: bool,
    pub detect: bool,
}

impl LanguageForm {
    pub fn read(app: &WordApp) -> LanguageForm {
        let s = &app.session;
        let here = s.typing_props();
        let lang = here.lang.clone().or_else(|| s.doc.styles.default_chr.lang.clone()).unwrap_or_else(|| "en-US".into());
        LanguageForm { lang, no_proof: here.no_proof.unwrap_or(false), detect: s.prefs.detect_language }
    }

    /// `review.language` parameters for OK.
    pub fn params(&self) -> Value {
        json!({"lang": self.lang, "noProof": self.no_proof, "detect": self.detect})
    }
}

/// A language's name in the interface language (the tag itself when we don't know it).
fn language_name(tag: &str) -> String {
    LANGUAGES.iter().find(|(t, _)| t.eq_ignore_ascii_case(tag)).map(|(_, n)| tl!(n).to_string()).unwrap_or_else(|| tag.to_string())
}

/// Explain how to enable German spelling when the selected region has no dictionary. Regional
/// dictionaries are deliberately separate: a German dictionary must not proof Swiss text.
pub(crate) fn missing_dictionary_hint(tag: &str) -> Option<String> {
    use wordcraft_engine::proof::{ProofLang, de};
    let lang = ProofLang::from_tag(Some(tag));
    let hint = german_dictionary_hint(lang)?;
    (!de::has_dictionary(lang)).then_some(hint)
}

fn german_dictionary_hint(lang: wordcraft_engine::proof::ProofLang) -> Option<String> {
    use wordcraft_engine::proof::ProofLang;
    let (dictionary, package) = match lang {
        ProofLang::De => ("de_DE.aff / de_DE.dic", "hunspell-de-de"),
        ProofLang::DeAt => ("de_AT.aff / de_AT.dic", "hunspell-de-at"),
        ProofLang::DeCh => ("de_CH.aff / de_CH.dic", "hunspell-de-ch"),
        _ => return None,
    };
    Some(crate::i18n::fmt(
        tl!(
            "German spelling needs the matching Hunspell dictionary: {dictionary}. Install {package}, or point WORDCRAFT_DICTIONARIES to a folder containing the matching .aff/.dic files."
        ),
        &[("dictionary", dictionary), ("package", package)],
    ))
}

fn language_ui(app: &mut WordApp, ui: &mut Ui, f: &mut LanguageForm) -> bool {
    ui.label(tl!("Mark selected text as:"));
    let mut list: Vec<(String, String)> = LANGUAGES.iter().map(|(t, _)| (t.to_string(), language_name(t))).collect();
    if !LANGUAGES.iter().any(|(t, _)| t.eq_ignore_ascii_case(&f.lang)) {
        list.push((f.lang.clone(), f.lang.clone()));
    }
    list.sort_by_key(|a| a.1.to_lowercase());
    egui::Frame::NONE.stroke(egui::Stroke::new(1.0, crate::theme::Tokens::get(ui.ctx()).border_strong)).inner_margin(4.0).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(220.0).min_scrolled_height(220.0).show(ui, |ui| {
            ui.set_min_width(300.0);
            for (tag, name) in &list {
                let on = f.lang.eq_ignore_ascii_case(tag);
                let r = ui.selectable_label(on, name.as_str());
                if r.clicked() {
                    f.lang = tag.clone();
                }
            }
        });
    });
    ui.add_space(4.0);
    ui.checkbox(&mut f.no_proof, tl!("Do not check spelling or grammar"));
    ui.checkbox(&mut f.detect, tl!("Detect language automatically"));
    if !f.no_proof
        && let Some(hint) = missing_dictionary_hint(&f.lang)
    {
        ui.add(egui::Label::new(egui::RichText::new(hint).small().weak()).wrap());
    }
    ui.add_space(4.0);
    let set_default = ui.button(tl!("Set As Default")).clicked();
    if set_default && run(app, "review.language", json!({"lang": f.lang, "default": true, "detect": f.detect})) {
        app.status(crate::i18n::fmt(tl!("{language} is now the document's default language."), &[("language", &language_name(&f.lang))]));
    }
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        return run(app, "review.language", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Envelopes

/// The Envelopes dialog's fields. `size` indexes [`ENVELOPE_SIZES`].
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopeForm {
    pub delivery: String,
    pub ret: String,
    pub omit_return: bool,
    pub size: usize,
}

impl EnvelopeForm {
    /// The delivery address starts as the selected text (an address in the letter).
    pub fn read(app: &WordApp) -> EnvelopeForm {
        let sel = if app.session.sel.is_collapsed() { String::new() } else { app.session.selected_text() };
        EnvelopeForm { delivery: sel.trim().replace("\r\n", "\n").replace('\r', "\n"), ret: String::new(), omit_return: false, size: 0 }
    }

    /// `mailings.envelopes` parameters; `add` puts the envelope at the start of this document.
    pub fn params(&self, add: bool) -> Value {
        let size = ENVELOPE_SIZES.get(self.size).map_or("Envelope #10", |s| s.0);
        let ret = if self.omit_return { "" } else { self.ret.as_str() };
        json!({"delivery": self.delivery, "return": ret, "size": size, "add": add})
    }
}

fn envelopes_ui(app: &mut WordApp, ui: &mut Ui, f: &mut EnvelopeForm) -> bool {
    ui.label(tl!("Delivery address:"));
    ui.add(egui::TextEdit::multiline(&mut f.delivery).desired_rows(4).desired_width(340.0));
    ui.horizontal(|ui| {
        ui.label(tl!("Return address:"));
        ui.checkbox(&mut f.omit_return, tl!("Omit"));
    });
    ui.add_enabled(!f.omit_return, egui::TextEdit::multiline(&mut f.ret).desired_rows(3).desired_width(340.0));
    ui.horizontal(|ui| {
        ui.label(tl!("Envelope size:"));
        let label = |i: usize| ENVELOPE_SIZES.get(i).map(|(n, w, h)| envelope_label(n, *w, *h)).unwrap_or_default();
        egui::ComboBox::from_id_salt("env_size").selected_text(label(f.size)).width(220.0).show_ui(ui, |ui| {
            for i in 0..ENVELOPE_SIZES.len() {
                ui.selectable_value(&mut f.size, i, label(i));
            }
        });
    });
    let ready = !f.delivery.trim().is_empty();
    ui.add_space(8.0);
    let (mut add, mut new, mut cancel) = (false, false, false);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel = ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            new = ui.add_enabled(ready, egui::Button::new(tl!("New Document"))).clicked();
            add = ui.add_enabled(ready, crate::widgets::primary_button(ui, tl!("Add to Document"))).clicked();
        });
    });
    if add || new {
        return run(app, "mailings.envelopes", f.params(add));
    }
    cancel
}

/// `DL   110 × 220 mm` / `#10   4.13 × 9.5 in`: metric sizes in millimetres, the others in
/// inches. The pattern is translated (the unit's name, the order) and the numbers use the
/// region's decimal separator.
pub(crate) fn envelope_label(name: &str, w: f32, h: f32) -> String {
    let short = name.strip_prefix("Envelope ").unwrap_or(name);
    let (template, w, h) = if short.starts_with('#') || short == "Monarch" {
        let i = |pt: f32| ((pt / 72.0) * 100.0).round() / 100.0;
        (tl!("{name}   {height} × {width} in"), i(w), i(h))
    } else {
        let mm = |pt: f32| (pt * 25.4 / 72.0).round();
        (tl!("{name}   {height} × {width} mm"), mm(w), mm(h))
    };
    crate::i18n::fmt(template, &[("name", short), ("height", &with_decimal(h.to_string())), ("width", &with_decimal(w.to_string()))])
}

/// A number written with `.` rewritten with the region's decimal separator (`2,625`).
fn with_decimal(n: String) -> String {
    let sep = crate::options::decimal();
    if sep == '.' { n } else { n.replace('.', sep.encode_utf8(&mut [0; 4])) }
}

// ---------------------------------------------------------------------------------------------
// Labels

/// The Labels dialog's fields. `product` indexes [`LABEL_PRODUCTS`]; `row`/`col` count from 1.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelsForm {
    pub text: String,
    pub product: usize,
    pub single: bool,
    pub row: usize,
    pub col: usize,
    /// One label per recipient (when a recipient list is loaded).
    pub recipients: bool,
}

impl LabelsForm {
    pub fn read(app: &WordApp) -> LabelsForm {
        let sel = if app.session.sel.is_collapsed() { String::new() } else { app.session.selected_text() };
        LabelsForm {
            text: sel.trim().replace("\r\n", "\n").replace('\r', "\n"),
            product: 0,
            single: false,
            row: 1,
            col: 1,
            recipients: !app.session.merge.rows.is_empty(),
        }
    }

    /// `mailings.labels` parameters.
    pub fn params(&self) -> Value {
        let id = LABEL_PRODUCTS.get(self.product).map_or("letter-3x10", |p| p.id);
        let mut v = json!({"text": self.text, "product": id, "fromRecipients": self.recipients && !self.single});
        if self.single {
            v["single"] = json!({"row": self.row, "col": self.col});
        }
        v
    }
}

/// `Address labels, 3 × 10, 2.625" × 1" (Letter)`.
fn product_label(i: usize) -> String {
    let Some(p) = LABEL_PRODUCTS.get(i) else { return String::new() };
    let kind = match p.kind {
        "shipping" => tl!("Shipping labels"),
        "return" => tl!("Return address labels"),
        _ => tl!("Address labels"),
    };
    // Inches on Letter, millimetres on A4, as the sheets are sold.
    let size = if p.paper == "A4" {
        let mm = |pt: f32| with_decimal(format!("{:.1}", pt * 25.4 / 72.0));
        crate::i18n::fmt(tl!("{width} × {height} mm"), &[("width", &mm(p.width)), ("height", &mm(p.height))])
    } else {
        let i = |pt: f32| with_decimal((((pt / 72.0) * 1000.0).round() / 1000.0).to_string());
        format!("{}\" × {}\"", i(p.width), i(p.height))
    };
    let (columns, rows) = (p.cols.to_string(), p.rows.to_string());
    crate::i18n::fmt(
        tl!("{kind}, {columns} × {rows}, {size} ({paper})"),
        &[("kind", kind), ("columns", &columns), ("rows", &rows), ("size", &size), ("paper", tl!(p.paper))],
    )
}

fn labels_ui(app: &mut WordApp, ui: &mut Ui, f: &mut LabelsForm) -> bool {
    let has_recipients = !app.session.merge.rows.is_empty();
    if has_recipients {
        ui.checkbox(&mut f.recipients, tl!("One label per recipient"));
    }
    ui.add_enabled_ui(!(has_recipients && f.recipients && !f.single), |ui| {
        ui.label(tl!("Address:"));
        ui.add(egui::TextEdit::multiline(&mut f.text).desired_rows(4).desired_width(340.0));
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Label:"));
        egui::ComboBox::from_id_salt("label_product").selected_text(product_label(f.product)).width(300.0).show_ui(ui, |ui| {
            for i in 0..LABEL_PRODUCTS.len() {
                ui.selectable_value(&mut f.product, i, product_label(i));
            }
        });
    });
    let (rows, cols) = LABEL_PRODUCTS.get(f.product).map_or((1, 1), |p| (p.rows, p.cols));
    heading(ui, "Print");
    ui.radio_value(&mut f.single, false, tl!("Full page of the same label"));
    ui.horizontal(|ui| {
        ui.radio_value(&mut f.single, true, tl!("Single label"));
        ui.add_enabled_ui(f.single, |ui| {
            ui.label(tl!("Row:"));
            ui.add(egui::DragValue::new(&mut f.row).range(1..=rows));
            ui.label(tl!("Column:"));
            ui.add(egui::DragValue::new(&mut f.col).range(1..=cols));
        });
    });
    f.row = f.row.clamp(1, rows.max(1));
    f.col = f.col.clamp(1, cols.max(1));
    let ready = (has_recipients && f.recipients && !f.single) || !f.text.trim().is_empty();
    let (ok, cancel) = buttons(ui, "New Document", ready);
    if ok {
        return run(app, "mailings.labels", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Formula

/// The functions `table.formula` works out, offered by Paste function.
pub const FORMULA_FUNCTIONS: [&str; 6] = ["AVERAGE", "COUNT", "MAX", "MIN", "PRODUCT", "SUM"];

/// The table Formula dialog's field: the formula, suggested as Word does from the numbers next
/// to the caret's cell (`=SUM(ABOVE)`, else `=SUM(LEFT)`, else a bare `=`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaForm {
    pub formula: String,
}

impl FormulaForm {
    /// The caret's cell; none outside a table.
    pub fn read(app: &WordApp) -> Option<FormulaForm> {
        let s = &app.session;
        let (tp, r, c) = s.sel.focus.path.cell()?;
        let t = s.doc.table(s.sel.focus.story, &tp)?;
        let lang = wordcraft_engine::cmd::default_lang(s);
        let number = |row: Option<usize>, col: Option<usize>| {
            let cell = t.rows.get(row?)?.cells.get(col?)?;
            let text = cell.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join(" ");
            wordcraft_doc::locale_number::parse_number(text.trim(), lang.as_deref())
        };
        let formula = if number(r.checked_sub(1), Some(c)).is_some() {
            "=SUM(ABOVE)"
        } else if number(Some(r), c.checked_sub(1)).is_some() {
            "=SUM(LEFT)"
        } else {
            "="
        };
        Some(FormulaForm { formula: formula.into() })
    }

    /// Paste function: `NAME()` at the end of the formula.
    pub fn paste(&mut self, name: &str) {
        self.formula.push_str(name);
        self.formula.push_str("()");
    }

    /// `table.formula` parameters for OK.
    pub fn params(&self) -> Value {
        json!({"formula": self.formula.trim()})
    }
}

fn formula_ui(app: &mut WordApp, ui: &mut Ui, f: &mut FormulaForm) -> bool {
    ui.label(tl!("Formula:"));
    ui.add(egui::TextEdit::singleline(&mut f.formula).desired_width(300.0));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Paste function:"));
        let mut picked = None;
        egui::ComboBox::from_id_salt("formula_paste").selected_text("").width(140.0).show_ui(ui, |ui| {
            for name in FORMULA_FUNCTIONS {
                if ui.selectable_label(false, name).clicked() {
                    picked = Some(name);
                }
            }
        });
        if let Some(name) = picked {
            f.paste(name);
        }
    });
    let ready = f.formula.trim().starts_with('=') && f.formula.trim().len() > 1;
    let (ok, cancel) = buttons(ui, "OK", ready);
    if ok {
        return run(app, "table.formula", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Sort

/// The table Sort dialog's fields: the column to sort by, the order and whether the first row is
/// a header (kept in place). `headers` is the first row's text, which names the columns when it
/// is a header.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortForm {
    pub column: usize,
    pub descending: bool,
    pub header: bool,
    pub headers: Vec<String>,
}

impl SortForm {
    /// The caret's table; none outside a table.
    pub fn read(app: &WordApp) -> Option<SortForm> {
        let s = &app.session;
        let (tp, _, _) = s.sel.focus.path.cell()?;
        let first = s.doc.table(s.sel.focus.story, &tp)?.rows.first()?;
        let headers = first
            .cells
            .iter()
            .map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join(" ").trim().to_string())
            .collect();
        Some(SortForm { column: 0, descending: false, header: first.props.header, headers })
    }

    /// A column's name in Sort by: its header text, or `Column n`.
    pub fn column_label(&self, i: usize) -> String {
        match self.headers.get(i).filter(|h| self.header && !h.is_empty()) {
            Some(h) => h.clone(),
            None => crate::i18n::fmt(tl!("Column {n}"), &[("n", &(i + 1).to_string())]),
        }
    }

    /// `table.sort` parameters for OK.
    pub fn params(&self) -> Value {
        json!({"column": self.column, "descending": self.descending, "header": self.header})
    }
}

fn sort_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SortForm) -> bool {
    heading(ui, "Sort by");
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("sort_column").selected_text(f.column_label(f.column)).width(180.0).show_ui(ui, |ui| {
            for i in 0..f.headers.len() {
                let label = f.column_label(i);
                ui.selectable_value(&mut f.column, i, label);
            }
        });
        ui.vertical(|ui| {
            ui.radio_value(&mut f.descending, false, tl!("Ascending"));
            ui.radio_value(&mut f.descending, true, tl!("Descending"));
        });
    });
    heading(ui, "My list has");
    ui.horizontal(|ui| {
        ui.radio_value(&mut f.header, true, tl!("Header row"));
        ui.radio_value(&mut f.header, false, tl!("No header row"));
    });
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        return run(app, "table.sort", f.params());
    }
    cancel
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::*;
    use crate::dialogs::Dialog;
    use crate::{Services, WordApp};

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::from_text("Dear Jo,\nThanks.")), Services::default())
    }

    fn form(a: &mut WordApp) -> LayoutDialog {
        let Some(Dialog::Layout(d)) = a.dialog.take() else { panic!("no dialog: {:?}", a.dialog) };
        *d
    }

    /// #407: Line Numbering Options… opens with the section's numbering and OK sets every
    /// option; Hyphenation Options sets the document's hyphenation settings.
    #[test]
    fn line_numbers_and_hyphenation_options_apply() {
        let mut a = app();
        a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
        let LayoutDialog::LineNumbers(mut f) = form(&mut a) else { panic!("line numbers") };
        assert!(!f.on && f.auto_distance && f.restart == "restartPage");
        f.on = true;
        f.start = 10;
        f.count_by = 5;
        f.restart = "continuous".into();
        a.run("layout.lineNumbers", f.params()).unwrap();
        let l = a.session.doc.last_section.line_numbers.clone().unwrap();
        assert_eq!((l.start, l.count_by, l.distance, l.restart), (10, 5, 0.0, LineNumberRestart::Continuous));
        a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
        assert!(matches!(form(&mut a), LayoutDialog::LineNumbers(f) if f.on && f.start == 10 && f.restart == "continuous"));

        a.run("ui.dialog", json!({"name": "hyphenation"})).unwrap();
        let LayoutDialog::Hyphenation(mut h) = form(&mut a) else { panic!("hyphenation") };
        assert!(!h.auto && h.caps && h.limit == 0);
        (h.auto, h.caps, h.limit) = (true, false, 2);
        a.run("layout.hyphenation", h.params()).unwrap();
        let s = &a.session.doc.settings;
        assert!(s.auto_hyphenation && !s.hyphenate_caps && s.consecutive_hyphen_limit == 2);
        assert!((s.hyphenation_zone - 18.0).abs() < 0.01, "zone untouched: {}", s.hyphenation_zone);
    }

    /// #755: the Line Numbers "From text" distance and the Hyphenation zone follow the
    /// File › Options unit (centimeters here); `params` sends points whatever the unit.
    #[test]
    fn line_numbers_and_hyphenation_measure_in_the_options_unit() {
        let mut a = app();
        a.run("file.options", json!({"units": "centimeters"})).unwrap();
        a.run("layout.lineNumbers", json!({"value": "restartPage", "start": 1, "countBy": 1, "distance": 18.0})).unwrap();
        a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
        let LayoutDialog::LineNumbers(f) = form(&mut a) else { panic!("line numbers") };
        assert!(!f.auto_distance);
        let cm = 18.0 / wordcraft_geom::PT_PER_CM;
        assert!((f.distance - cm).abs() < 0.01, "distance was {} cm", f.distance);
        let sent = f.params()["distance"].as_f64().unwrap_or_default();
        assert!((sent - 18.0).abs() < 0.05, "params sent {sent} pt");

        a.run("layout.hyphenation", json!({"value": true, "zone": 36.0})).unwrap();
        a.run("ui.dialog", json!({"name": "hyphenation"})).unwrap();
        let LayoutDialog::Hyphenation(h) = form(&mut a) else { panic!("hyphenation") };
        assert!((h.zone - 36.0 / wordcraft_geom::PT_PER_CM).abs() < 0.01, "zone was {} cm", h.zone);
        let sent = h.params()["zone"].as_f64().unwrap_or_default();
        assert!((sent - 36.0).abs() < 0.05, "params sent {sent} pt");
    }

    /// #407: the Language button opens the dialog (scripts with a language never see it); OK
    /// marks the selection and "do not check", Set As Default changes the document default.
    #[test]
    fn language_dialog_marks_the_selection() {
        let mut a = app();
        a.run("select.all", json!({})).unwrap();
        assert_eq!(a.run("review.language", json!({})).unwrap(), json!({"pending": "language"}));
        let LayoutDialog::Language(mut f) = form(&mut a) else { panic!("language") };
        assert_eq!(f.lang, "en-US");
        f.lang = "sr-Latn-RS".into();
        f.no_proof = true;
        a.run("review.language", f.params()).unwrap();
        let c = a.session.typing_props();
        assert_eq!((c.lang.as_deref(), c.no_proof), (Some("sr-Latn-RS"), Some(true)));
        a.run("review.language", json!({"lang": "fr-FR", "default": true})).unwrap();
        assert_eq!(a.session.doc.styles.default_chr.lang.as_deref(), Some("fr-FR"));
        assert!(a.dialog.is_none());
        assert_eq!(language_name("xx-YY"), "xx-YY", "an unknown tag shows as itself");
    }

    #[test]
    fn german_dictionary_setup_names_the_selected_region() {
        use wordcraft_engine::proof::ProofLang;
        for (lang, tag, package) in
            [(ProofLang::De, "de_DE", "hunspell-de-de"), (ProofLang::DeAt, "de_AT", "hunspell-de-at"), (ProofLang::DeCh, "de_CH", "hunspell-de-ch")]
        {
            let hint = german_dictionary_hint(lang).unwrap();
            assert!(hint.contains(&format!("{tag}.aff")), "{hint}");
            assert!(hint.contains(&format!("{tag}.dic")), "{hint}");
            assert!(hint.contains(package), "{hint}");
            assert!(hint.contains("WORDCRAFT_DICTIONARIES"), "{hint}");
        }
        assert_eq!(missing_dictionary_hint("en-US"), None);
        assert_eq!(missing_dictionary_hint("fr-FR"), None);
        assert!(LANGUAGES.contains(&("de-AT", "German (Austria)")));
        assert!(LANGUAGES.contains(&("de-CH", "German (Switzerland)")));
    }

    /// #407: Envelopes starts from the selected address; Add to Document puts an envelope
    /// section before the letter without asking to save (nothing is replaced).
    #[test]
    fn envelope_dialog_adds_an_envelope_section() {
        let mut a = app();
        a.run("select.paragraph", json!({})).unwrap();
        a.session.dirty = true;
        assert_eq!(a.run("mailings.envelopes", json!({})).unwrap(), json!({"pending": "envelopes"}));
        let LayoutDialog::Envelopes(mut f) = form(&mut a) else { panic!("envelopes") };
        assert_eq!(f.delivery, "Dear Jo,");
        f.delivery = "Jo Doe\n1 Main St".into();
        f.ret = "Me".into();
        f.size = ENVELOPE_SIZES.iter().position(|s| s.0 == "Envelope DL").unwrap();
        let document = a.session.document_id();
        a.run("mailings.envelopes", f.params(true)).unwrap();
        assert!(a.dialog.is_none(), "no save prompt");
        assert_eq!(a.session.document_id(), document);
        let secs = a.session.doc.sections();
        assert_eq!(secs.len(), 2);
        assert!((secs[0].1.page_w - 623.6).abs() < 0.1);
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Thanks."));
    }

    /// A table `cols` wide, typed cell by cell (empty cells are skipped over); the caret ends in
    /// the last cell.
    fn table_app(cols: usize, cells: &[&str]) -> WordApp {
        let mut a = app();
        a.run("insert.table", json!({"rows": cells.len() / cols, "cols": cols})).unwrap();
        for (k, c) in cells.iter().enumerate() {
            if k > 0 {
                a.run("text.tab", json!({})).unwrap();
            }
            if !c.is_empty() {
                a.run("text.insert", json!({"text": c})).unwrap();
            }
        }
        a
    }

    fn fields(a: &WordApp) -> Vec<(String, String)> {
        let mut out = Vec::new();
        a.session.doc.objects_in_reading_order(&mut |o| {
            if let wordcraft_doc::InlineObject::Field { instr, result, .. } = o {
                out.push((instr.clone(), result.clone()));
            }
        });
        out
    }

    /// #1201: Table Layout › Formula opens its dialog instead of inserting `=SUM(ABOVE)` at once.
    /// The formula is suggested from the numbers next to the cell (above, else to the left), Paste
    /// function adds a function to it, and OK inserts the field of the formula as edited.
    #[test]
    fn table_formula_opens_its_dialog() {
        let mut a = table_app(2, &["Item", "Cost", "Tea", "3", "Jam", "4", "Total", ""]);
        assert_eq!(a.run("table.formula", json!({})).unwrap(), json!({"pending": "tableFormula"}));
        assert!(fields(&a).is_empty(), "nothing inserted yet");
        let LayoutDialog::Formula(mut f) = form(&mut a) else { panic!("formula") };
        assert_eq!(f.formula, "=SUM(ABOVE)");
        f.formula = "=".into();
        f.paste("MAX");
        assert_eq!(f.formula, "=MAX()");
        f.formula = "=MAX(ABOVE)".into();
        a.run("table.formula", f.params()).unwrap();
        assert_eq!(fields(&a), [("=MAX(ABOVE)".to_string(), "4".to_string())]);
        assert!(a.dialog.is_none());

        let mut a = table_app(3, &["Tea", "3", "", "Jam", "4", ""]);
        a.run("table.formula", json!({})).unwrap();
        let LayoutDialog::Formula(f) = form(&mut a) else { panic!("formula") };
        assert_eq!(f.formula, "=SUM(LEFT)", "nothing above, numbers to the left");

        let mut a = app();
        a.run("ui.dialog", json!({"name": "tableFormula"})).unwrap();
        assert!(a.dialog.is_none(), "no dialog outside a table");
    }

    fn first_column(a: &WordApp) -> Vec<String> {
        let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        t.rows.iter().map(|r| r.cells[0].blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect()).collect()
    }

    /// #1200: Table Layout › Sort opens its dialog instead of sorting by the first column at once;
    /// with a header row the columns are named by it, and OK sorts the rows under it by the
    /// chosen column (as numbers here: 2, 7, 10).
    #[test]
    fn table_sort_opens_its_dialog() {
        let mut a = table_app(2, &["Name", "Score", "Bea", "10", "Al", "2", "Cy", "7"]);
        assert_eq!(a.run("table.sort", json!({})).unwrap(), json!({"pending": "tableSort"}));
        assert_eq!(first_column(&a), ["Name", "Bea", "Al", "Cy"], "nothing sorted yet");
        let LayoutDialog::Sort(mut f) = form(&mut a) else { panic!("sort") };
        assert!(!f.header && f.column == 0 && !f.descending);
        assert_eq!(f.column_label(1), "Column 2");
        f.header = true;
        assert_eq!(f.column_label(1), "Score");
        f.column = 1;
        a.run("table.sort", f.params()).unwrap();
        assert_eq!(first_column(&a), ["Name", "Al", "Cy", "Bea"]);
        assert!(a.dialog.is_none());
    }

    /// The envelope sizes are in the interface language with the region's decimal separator
    /// (#1128; the untranslated-text audit found `#10   4.13 × 9.5 in` drawn in English).
    #[test]
    fn envelope_sizes_follow_the_interface_language_and_region() {
        let size = |name: &str| ENVELOPE_SIZES.iter().find(|s| s.0 == name).map(|(n, w, h)| envelope_label(n, *w, *h)).unwrap();
        assert_eq!(size("Envelope #10"), "#10   4.13 × 9.5 in");
        assert_eq!(size("Envelope DL"), "DL   110 × 220 mm");
        let mut a = app();
        a.run("file.options", json!({"region": "it-IT"})).unwrap();
        crate::options::sync(&a);
        crate::i18n::set_current(crate::i18n::Lang::from_code("it").unwrap());
        assert_eq!(size("Envelope #10"), "#10   4,13 × 9,5 pollici");
        assert_eq!(size("Envelope DL"), "DL   110 × 220 mm");
        crate::i18n::set_current(crate::i18n::Lang::EN);
        a.run("file.options", json!({"region": "en-US"})).unwrap();
        crate::options::sync(&a);
    }

    /// #407: Labels builds a new document of the chosen sheet, or a single label.
    #[test]
    fn labels_dialog_builds_a_sheet() {
        let mut a = app();
        assert_eq!(a.run("mailings.labels", json!({})).unwrap(), json!({"pending": "labels"}));
        let LayoutDialog::Labels(mut f) = form(&mut a) else { panic!("labels") };
        f.text = "Ada Lovelace\nLondon".into();
        f.product = LABEL_PRODUCTS.iter().position(|p| p.id == "a4-3x8").unwrap();
        let r = a.session.run("mailings.labels", &f.params()).unwrap();
        assert_eq!(r["labels"], 24);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).matches("Ada Lovelace").count(), 24);
        f.single = true;
        (f.row, f.col) = (3, 2);
        assert_eq!(a.session.run("mailings.labels", &f.params()).unwrap()["filled"], 1);
        assert!(product_label(f.product).contains("63.5 × 33.9 mm (A4)"));
    }
}
