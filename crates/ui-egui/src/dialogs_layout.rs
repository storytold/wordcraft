//! Layout, Review and Mailings dialogs (#407): Line Numbers (Layout › Line Numbers › Line
//! Numbering Options…), Hyphenation (Hyphenation Options… and Manual…), Language (Review ›
//! Language), Envelopes and Labels (Mailings › Create), Formula (Table Layout › Data, #1201)
//! Sort (Table Layout › Data, #1200), Split Cells (Table Layout › Merge), Convert Text to Table
//! (Insert › Table, #2592), Size (Picture Format › Size, #2573) and Format Page Numbers (Insert ›
//! Header & Footer › Page Number ▸ Format Page Numbers…, #1113). Each ends by running a
//! command (`layout.lineNumbers`, `layout.hyphenation`, `layout.manualHyphenation` with
//! `caret.set` and `text.optionalHyphen`, `review.language`, `mailings.envelopes`,
//! `mailings.labels`, `table.formula`, `table.sort`, `table.split`, `table.fromText`,
//! `picture.size`, `layout.pageNumberFormat`), so scripts and agents get the same result without
//! the dialog. Shown through [`crate::dialogs::Dialog::Layout`].

use egui::{Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};

use wordcraft_doc::section::LineNumberRestart;
use wordcraft_engine::cmd::mailings::{ENVELOPE_SIZES, LABEL_PRODUCTS};
use wordcraft_engine::cmd::objects::fit_size;

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
    SplitCells(SplitCellsForm),
    TextToTable(TextToTableForm),
    TableToText(TableToTextForm),
    Size(SizeForm),
    PageNumbers(PageNumbersForm),
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
            LayoutDialog::SplitCells(_) => "splitCells",
            LayoutDialog::TextToTable(_) => "textToTable",
            LayoutDialog::TableToText(_) => "tableToText",
            LayoutDialog::Size(_) => "pictureSize",
            LayoutDialog::PageNumbers(_) => "pageNumbers",
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
            LayoutDialog::SplitCells(_) => "Split Cells",
            LayoutDialog::TextToTable(_) => "Convert Text to Table",
            LayoutDialog::TableToText(_) => "Convert to Text",
            LayoutDialog::Size(_) => "Size",
            LayoutDialog::PageNumbers(_) => "Page Number Format",
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
        "splitCells" => LayoutDialog::SplitCells(SplitCellsForm::read(app)?),
        "textToTable" => LayoutDialog::TextToTable(TextToTableForm::read(app)?),
        "tableToText" => LayoutDialog::TableToText(TableToTextForm::read(app)?),
        "pictureSize" => LayoutDialog::Size(SizeForm::read(app)?),
        "pageNumbers" => LayoutDialog::PageNumbers(PageNumbersForm::read(app)),
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
        LayoutDialog::SplitCells(f) => split_cells_ui(app, ui, f),
        LayoutDialog::TextToTable(f) => text_to_table_ui(app, ui, f),
        LayoutDialog::TableToText(f) => table_to_text_ui(app, ui, f),
        LayoutDialog::Size(f) => size_ui(app, ui, f),
        LayoutDialog::PageNumbers(f) => page_numbers_ui(app, ui, f),
    }
}

/// The accent button and Cancel; returns (ok, cancel).
fn buttons(ui: &mut Ui, ok: &str, ok_enabled: bool) -> (bool, bool) {
    let mut r = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (okb, cancelb) =
                crate::dialogs::ok_cancel_responses(ui, |ui| ui.add_enabled(ok_enabled, crate::widgets::primary_button(ui, tl!(ok))), tl!("Cancel"));
            if cancelb.clicked() || crate::dialogs::cancel_key_pressed(ui) {
                r.1 = true;
            }
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
pub const LANGUAGES: [(&str, &str); 38] = [
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("en-AU", "English (Australia)"),
    ("en-CA", "English (Canada)"),
    ("en-IN", "English (India)"),
    ("en-IE", "English (Ireland)"),
    ("en-NZ", "English (New Zealand)"),
    ("en-ZA", "English (South Africa)"),
    ("de-DE", "German (Germany)"),
    ("fr-FR", "French (France)"),
    ("es-ES", "Spanish (Spain)"),
    ("es-MX", "Spanish (Mexico)"),
    ("es-419", "Spanish (Latin America)"),
    ("es-UY", "Spanish (Uruguay)"),
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
    ("id-ID", "Indonesian"),
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
        let style = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.style.as_deref());
        let here = s.doc.styles.resolve_char(style, &s.typing_props());
        let lang = here.lang.unwrap_or_else(|| "en-US".into());
        LanguageForm { lang, no_proof: here.no_proof, detect: s.prefs.detect_language }
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

// ---------------------------------------------------------------------------------------------
// Split Cells

/// The Split Cells dialog's fields. As in Word, the defaults follow the selection: twice its
/// columns and its rows. Merge cells before split is offered, and on, when several cells (or one
/// cell merged across columns) are selected (`several`). A merged selection splits into at most
/// `max_rows` rows, a vertically merged cell alone into at most its `height`; more gets Word's
/// message ([`SplitCellsForm::check`]). Without merging each cell splits into columns on its own
/// and the row count stays 1.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitCellsForm {
    pub columns: u32,
    pub rows: u32,
    pub merge_first: bool,
    pub several: bool,
    /// The selected cells, a vertically merged cell counted once.
    pub cells: u32,
    /// The rows a single selected cell covers (more than 1 when it is merged down).
    pub height: u32,
    /// The selected rows: the default row count and its limit while merging.
    pub max_rows: u32,
    /// The default column count: twice the most cells selected in a row.
    pub default_columns: u32,
    /// Word's message for a row count out of range, shown until OK is pressed again.
    #[serde(skip)]
    pub error: Option<String>,
}

impl SplitCellsForm {
    /// The selected cells; none outside a table.
    pub fn read(app: &WordApp) -> Option<SplitCellsForm> {
        let s = &app.session;
        let (tp, _, _) = s.sel.focus.path.cell()?;
        let mut cells = wordcraft_engine::cmd::table::selected_cells(s);
        cells.sort_unstable();
        cells.dedup();
        let mut rows: Vec<usize> = cells.iter().map(|&(r, _)| r).collect();
        rows.dedup();
        let per_row = rows.iter().map(|&r| cells.iter().filter(|c| c.0 == r).count()).max().unwrap_or(1);
        // A vertically merged cell is one cell, however many rows it covers.
        let targets = wordcraft_engine::cmd::table::split_targets(s);
        let t = s.doc.table(s.sel.focus.story, &tp);
        let (span, height) = match (targets.as_slice(), t) {
            ([(top, g)], Some(t)) => t
                .cell_at_grid(*top, *g)
                .map(|ci| {
                    (t.rows.get(*top).and_then(|row| row.cells.get(ci)).map_or(1, |c| c.span()), t.vertical_extent(*top, ci).map_or(1, |(_, k)| k))
                })
                .unwrap_or((1, 1)),
            _ => (1, 1),
        };
        // Word offers Merge for one cell merged across columns too.
        let several = targets.len() > 1 || span > 1;
        let max_rows = if targets.len() > 1 { rows.len().clamp(1, 63) as u32 } else { 1 };
        let default_columns = per_row.saturating_mul(2).clamp(2, 63) as u32;
        Some(SplitCellsForm {
            columns: default_columns,
            rows: max_rows,
            merge_first: several,
            several,
            cells: targets.len().clamp(1, 63 * 63) as u32,
            height: height.clamp(1, 32_767) as u32,
            max_rows,
            default_columns,
            error: None,
        })
    }

    /// Ticks or unticks Merge cells before split; unticked, each cell splits into two columns
    /// in its own row, as Word resets the boxes.
    pub fn set_merge(&mut self, on: bool) {
        self.merge_first = on && self.several;
        (self.columns, self.rows) = if self.merge_first { (self.default_columns, self.max_rows) } else { (2, 1) };
    }

    /// Word's message when the row count is out of range: a merged selection splits into 1 to
    /// its rows, a merged cell into 1 to its height.
    pub fn check(&self) -> Option<String> {
        let limit = if self.cells > 1 && self.merge_first {
            self.max_rows
        } else if self.cells <= 1 && self.height > 1 {
            self.height
        } else {
            return None;
        };
        if self.rows > limit {
            return Some(crate::i18n::fmt(tl!("The number must be between 1 and {max}."), &[("max", &limit.to_string())]));
        }
        // A merged cell splits into a divisor of its height, as in Word (the engine refuses the rest).
        let divides = self.cells <= 1 && self.height > 1 && self.rows > 1 && !self.height.is_multiple_of(self.rows);
        divides.then(|| crate::i18n::fmt(tl!("The number must be a divisor of {max}."), &[("max", &self.height.to_string())]))
    }

    /// `table.split` parameters for OK.
    pub fn params(&self) -> Value {
        json!({"columns": self.columns, "rows": self.rows, "mergeFirst": self.several && self.merge_first})
    }
}

/// An alert line under a dialog's fields, in the error colour.
fn alert(ui: &mut Ui, text: &Option<String>) {
    if let Some(e) = text {
        ui.colored_label(ui.visuals().error_fg_color, e.as_str());
    }
}

fn split_cells_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SplitCellsForm) -> bool {
    // Several cells split one by one keep their rows.
    let rows_on = f.cells <= 1 || f.merge_first;
    egui::Grid::new("split_cells").num_columns(2).show(ui, |ui| {
        ui.label(tl!("Number of columns:"));
        ui.add(egui::DragValue::new(&mut f.columns).range(1..=63));
        ui.end_row();
        ui.label(tl!("Number of rows:"));
        ui.add_enabled(rows_on, egui::DragValue::new(&mut f.rows).range(1..=63));
        ui.end_row();
    });
    let mut merge = f.merge_first;
    if ui.add_enabled(f.several, egui::Checkbox::new(&mut merge, tl!("Merge cells before split"))).changed() {
        f.set_merge(merge);
    }
    alert(ui, &f.error);
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        // Out of range: Word's message, and the dialog stays open.
        f.error = f.check();
        return f.error.is_none() && run(app, "table.split", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Convert Text to Table

/// The Convert Text to Table dialog's fields: the selected paragraphs (`lines`), where to
/// separate them (`paragraph`, `comma`, `tab` or `other` with `other_char`), the column count
/// (guessed from the text for the separator, as Word does) and the AutoFit behaviour: an initial
/// column width (`width`, "Auto" or a measurement up to 22 in), or AutoFit to contents or to the
/// window. `rows` is the resulting row count, shown read-only.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextToTableForm {
    #[serde(skip)]
    pub lines: Vec<String>,
    pub columns: u32,
    pub rows: usize,
    pub autofit: String,
    /// Initial column width as typed: "Auto" (or 0) shares the text width out.
    pub width: String,
    pub separator: String,
    pub other_char: String,
    /// Word's message for a width out of range, shown until OK is pressed again.
    #[serde(skip)]
    pub error: Option<String>,
}

impl TextToTableForm {
    /// The selected paragraphs; none when the selection can't be converted (a table inside it).
    pub fn read(app: &WordApp) -> Option<TextToTableForm> {
        let lines = wordcraft_engine::cmd::table::text_table_lines(&app.session).ok()?;
        // Tabs when the text has them, else paragraphs (Word doesn't guess commas).
        let separator = if lines.iter().any(|l| l.contains('\t')) { "tab" } else { "paragraph" };
        let mut f = TextToTableForm {
            lines,
            columns: 1,
            rows: 0,
            autofit: "initial".into(),
            width: tl!("Auto").into(),
            separator: separator.into(),
            other_char: "-".into(),
            error: None,
        };
        f.guess_columns();
        Some(f)
    }

    fn sep(&self) -> Option<char> {
        match self.separator.as_str() {
            "comma" => Some(','),
            "tab" => Some('\t'),
            "other" => Some(self.other_char.chars().next().unwrap_or('-')),
            _ => None,
        }
    }

    /// The column count the text gives with the chosen separator.
    pub fn guess_columns(&mut self) {
        let cols = wordcraft_engine::cmd::table::text_table_cells(&self.lines, self.sep(), None).1;
        self.columns = cols.clamp(1, 63) as u32;
        self.count_rows();
    }

    /// The row count for the chosen separator and columns.
    pub fn count_rows(&mut self) {
        self.rows = wordcraft_engine::cmd::table::text_table_cells(&self.lines, self.sep(), Some(self.columns as usize)).0.len();
    }

    /// The initial column width in points: 0 for Auto, `None` when it isn't a measurement from
    /// 0 to 22 in.
    fn width_pt(&self) -> Option<f32> {
        let text = self.width.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("auto") || text == tl!("Auto") {
            return Some(0.0);
        }
        wordcraft_geom::parse_measure(text, unit()).filter(|pt| (0.0..=22.0 * 72.0).contains(pt))
    }

    /// Word's message when the initial column width is out of range (only with that option).
    pub fn check(&self) -> Option<String> {
        if self.autofit != "initial" || self.width_pt().is_some() {
            return None;
        }
        let sep = crate::options::decimal();
        let (min, max) = (unit().format_with(0.0, sep), unit().format_with(22.0 * 72.0, sep));
        Some(crate::i18n::fmt(tl!("The measurement must be between {min} and {max}."), &[("min", &min), ("max", &max)]))
    }

    /// `table.fromText` parameters for OK.
    pub fn params(&self) -> Value {
        let mut v = json!({"separator": self.separator, "columns": self.columns, "autofit": self.autofit});
        if self.separator == "other" {
            v["char"] = json!(self.other_char);
        }
        if self.autofit == "initial"
            && let Some(w) = self.width_pt().filter(|&w| w > 0.0)
        {
            v["width"] = json!(w);
        }
        v
    }
}

fn text_to_table_ui(app: &mut WordApp, ui: &mut Ui, f: &mut TextToTableForm) -> bool {
    let before = (f.separator.clone(), f.other_char.clone(), f.columns);
    heading(ui, "Table size");
    egui::Grid::new("text_to_table").num_columns(2).show(ui, |ui| {
        ui.label(tl!("Number of columns:"));
        ui.add(egui::DragValue::new(&mut f.columns).range(1..=63));
        ui.end_row();
        ui.label(tl!("Number of rows:"));
        ui.label(f.rows.to_string());
        ui.end_row();
    });
    heading(ui, "AutoFit behavior");
    ui.horizontal(|ui| {
        ui.radio_value(&mut f.autofit, "initial".to_string(), tl!("Initial column width:"));
        ui.add_enabled(f.autofit == "initial", egui::TextEdit::singleline(&mut f.width).desired_width(72.0));
    });
    ui.radio_value(&mut f.autofit, "contents".to_string(), tl!("AutoFit to contents"));
    ui.radio_value(&mut f.autofit, "window".to_string(), tl!("AutoFit to window"));
    heading(ui, "Separate text at");
    egui::Grid::new("text_to_table_sep").num_columns(2).show(ui, |ui| {
        ui.radio_value(&mut f.separator, "paragraph".to_string(), tl!("Paragraphs"));
        ui.radio_value(&mut f.separator, "comma".to_string(), tl!("Commas"));
        ui.end_row();
        ui.radio_value(&mut f.separator, "tab".to_string(), tl!("Tabs"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut f.separator, "other".to_string(), tl!("Other:"));
            let r = ui.add(egui::TextEdit::singleline(&mut f.other_char).char_limit(1).desired_width(24.0));
            if r.changed() {
                f.separator = "other".into();
            }
        });
        ui.end_row();
    });
    if (f.separator.as_str(), f.other_char.as_str()) != (before.0.as_str(), before.1.as_str()) {
        f.guess_columns();
    } else if f.columns != before.2 {
        f.count_rows();
    }
    alert(ui, &f.error);
    let ready = f.separator != "other" || !f.other_char.is_empty();
    let (ok, cancel) = buttons(ui, "OK", ready);
    if ok {
        // Out of range: Word's message, and the dialog stays open.
        f.error = f.check();
        return f.error.is_none() && run(app, "table.fromText", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Convert to Text

/// The Convert to Text dialog (Table Layout › Data): what sets the cells of a row apart in the
/// text (`paragraph`, `tab`, `comma` or `other` with `other_char`). It opens on Tabs.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableToTextForm {
    pub separator: String,
    pub other_char: String,
}

impl TableToTextForm {
    /// None when the caret isn't in a table.
    pub fn read(app: &WordApp) -> Option<TableToTextForm> {
        app.session.sel.focus.path.cell()?;
        Some(TableToTextForm { separator: "tab".into(), other_char: "-".into() })
    }

    /// `table.toText` parameters for OK.
    pub fn params(&self) -> Value {
        let mut v = json!({"separator": self.separator});
        if self.separator == "other" {
            v["char"] = json!(self.other_char);
        }
        v
    }
}

fn table_to_text_ui(app: &mut WordApp, ui: &mut Ui, f: &mut TableToTextForm) -> bool {
    heading(ui, "Separate text with");
    ui.radio_value(&mut f.separator, "paragraph".to_string(), tl!("Paragraph marks"));
    ui.radio_value(&mut f.separator, "tab".to_string(), tl!("Tabs"));
    ui.radio_value(&mut f.separator, "comma".to_string(), tl!("Commas"));
    ui.horizontal(|ui| {
        ui.radio_value(&mut f.separator, "other".to_string(), tl!("Other:"));
        let r = ui.add(egui::TextEdit::singleline(&mut f.other_char).char_limit(1).desired_width(24.0));
        if r.changed() {
            f.separator = "other".into();
        }
    });
    let ready = f.separator != "other" || !f.other_char.is_empty();
    let (ok, cancel) = buttons(ui, "OK", ready);
    if ok {
        return run(app, "table.toText", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Size

/// The Size dialog's fields (Picture Format › Size launcher, #2573) for the selected picture or
/// shape. Lengths are in points; the fields show them in the interface unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeForm {
    /// Where the object sized is (the selected one when the dialog opened).
    pub at: wordcraft_doc::Pos,
    pub width: f32,
    pub height: f32,
    /// Changing one side changes the other in proportion (on for pictures, as in Word).
    pub lock: bool,
    /// The scale counts from the original size; otherwise from the size the dialog opened with.
    pub relative: bool,
    /// The size the dialog opened with.
    pub start: (f32, f32),
    /// What the crop leaves of the picture's original size: Reset's size. None for a shape, or a
    /// picture whose image can't be read.
    pub original: Option<(f32, f32)>,
    /// The original size before the crop, and a bitmap's pixels, as shown.
    pub uncropped: Option<(f32, f32)>,
    pub pixels: Option<(u32, u32)>,
    /// What the form was read from (see [`seen`]).
    #[serde(skip)]
    seen: Seen,
}

/// An object's picture, crop and size, bit for bit: what [`SizeForm::follow`] compares.
type Seen = (String, [u32; 4], [u32; 2]);

fn seen(o: &wordcraft_doc::para::InlineObject) -> Seen {
    let (w, h) = o.frame().map_or((0.0, 0.0), |(w, h, _)| (w, h));
    match o {
        wordcraft_doc::para::InlineObject::Image { media, crop, .. } => (media.clone(), crop.map(f32::to_bits), [w.to_bits(), h.to_bits()]),
        _ => (String::new(), [0; 4], [w.to_bits(), h.to_bits()]),
    }
}

impl SizeForm {
    /// The selected picture's or shape's size; None when nothing that has one is selected.
    pub fn read(app: &WordApp) -> Option<SizeForm> {
        use wordcraft_doc::para::InlineObject;
        let (at, o) = wordcraft_engine::cmd::objects::selected(&app.session)?;
        let (w, h, _) = o.frame()?;
        let full = match &o {
            InlineObject::Image { media, .. } => wordcraft_engine::cmd::objects::original_size(&app.session, media),
            _ => None,
        };
        let original = wordcraft_engine::cmd::objects::cropped_original_size(&app.session, &o);
        Some(SizeForm {
            at,
            width: w,
            height: h,
            lock: matches!(o, InlineObject::Image { .. }),
            relative: original.is_some(),
            start: (w, h),
            original,
            uncropped: full.map(|f| (f.w, f.h)),
            pixels: full.and_then(|f| f.pixels),
            seen: seen(&o),
        })
    }

    /// Follow the selection: dialogs aren't modal, so a click on the page may pick another object,
    /// and the ribbon or the handles may crop, change or resize this one. The form then starts over
    /// on what is selected, or there is nothing to size (false: close the dialog). OK and Reset
    /// never work from numbers read from another object or an older state of this one.
    pub fn follow(&mut self, app: &WordApp) -> bool {
        match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((at, o)) if at == self.at && seen(&o) == self.seen => true,
            Some(_) => SizeForm::read(app).map(|f| *self = f).is_some(),
            None => false,
        }
    }

    /// What the scale counts from.
    fn base(&self) -> (f32, f32) {
        match self.original {
            Some(o) if self.relative => o,
            _ => self.start,
        }
    }

    /// The width and height as percentages of [`Self::base`].
    pub fn scale(&self) -> (f32, f32) {
        let (bw, bh) = self.base();
        (self.width / bw.max(0.01) * 100.0, self.height / bh.max(0.01) * 100.0)
    }

    /// A new width; locked, the height follows in proportion, both kept within 1–4000 pt by one
    /// factor (as `picture.size` does), so the limit never changes the shape.
    pub fn set_width(&mut self, w: f32) {
        if self.lock && self.width > 0.0 {
            (self.width, self.height) = fit_size(w, self.height * w / self.width, true);
        } else {
            self.width = fit_size(w, self.height, false).0;
        }
    }

    /// A new height; locked, the width follows (see [`Self::set_width`]).
    pub fn set_height(&mut self, h: f32) {
        if self.lock && self.height > 0.0 {
            (self.width, self.height) = fit_size(self.width * h / self.height, h, true);
        } else {
            self.height = fit_size(self.width, h, false).1;
        }
    }

    /// The width as a percentage of [`Self::base`].
    pub fn set_scale_width(&mut self, percent: f32) {
        self.set_width(self.base().0 * percent / 100.0);
    }

    /// The height as a percentage of [`Self::base`].
    pub fn set_scale_height(&mut self, percent: f32) {
        self.set_height(self.base().1 * percent / 100.0);
    }

    /// Reset: the picture's original size, less what is cropped; one past 4000 pt (a large photo)
    /// shrinks as a whole.
    pub fn reset(&mut self) {
        if let Some((w, h)) = self.original {
            (self.width, self.height) = fit_size(w, h, true);
        }
    }

    /// `picture.size` parameters: both sides, as set here.
    pub fn params(&self) -> Value {
        json!({"width": self.width, "height": self.height, "lockAspect": false})
    }
}

/// The Size dialog; returns true to close it.
fn size_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SizeForm) -> bool {
    if !f.follow(app) {
        return true;
    }
    let unit = unit();
    let k = unit.pt_per_unit();
    // A length in the interface unit that takes any typed unit (`300 px` too), applied once typed
    // (Enter, Tab or a click elsewhere, OK included) so a half-typed number never resizes the other
    // side; `None` until its value changes (while
    // typing, the field reports each keystroke as a change). The range bounds what is typed or
    // dragged only: a value already outside it (a 1 px picture is 0.75 pt) is shown as it is, not
    // clamped, which egui would report as a change and so apply unasked.
    let length = |ui: &mut Ui, label: &str, pt: f32| {
        let name = ui.label(tl!(label));
        let was = pt / k;
        let mut v = was;
        let field =
            egui::DragValue::new(&mut v).speed(1.0 / k).range(1.0 / k..=4000.0 / k).clamp_existing_to_range(false).measure(unit).max_decimals(2);
        let changed = ui.add(field.update_while_editing(false)).labelled_by(name.id).changed();
        (changed && v != was).then_some(v * k)
    };
    let percent = |ui: &mut Ui, label: &str, was: f32| {
        let name = ui.label(tl!(label));
        let mut v = was;
        let field = egui::DragValue::new(&mut v).speed(1.0).range(1.0..=10_000.0).clamp_existing_to_range(false).suffix(" %").max_decimals(1);
        let changed = ui.add(field.update_while_editing(false)).labelled_by(name.id).changed();
        (changed && v != was).then_some(v)
    };
    heading(ui, "Size");
    egui::Grid::new("size_lengths").num_columns(4).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        if let Some(h) = length(ui, "Height:", f.height) {
            f.set_height(h);
        }
        if let Some(w) = length(ui, "Width:", f.width) {
            f.set_width(w);
        }
        ui.end_row();
    });
    heading(ui, "Scale");
    let (sw, sh) = f.scale();
    egui::Grid::new("size_scale").num_columns(4).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        if let Some(p) = percent(ui, "Height:", sh) {
            f.set_scale_height(p);
        }
        if let Some(p) = percent(ui, "Width:", sw) {
            f.set_scale_width(p);
        }
        ui.end_row();
    });
    ui.checkbox(&mut f.lock, tl!("Lock aspect ratio"));
    ui.add_enabled(f.original.is_some(), egui::Checkbox::new(&mut f.relative, tl!("Relative to original picture size")));
    if let Some((w, h)) = f.uncropped {
        heading(ui, "Original size");
        ui.horizontal(|ui| {
            ui.label(format!("{} {}", tl!("Height:"), crate::options::length(h)));
            ui.add_space(12.0);
            ui.label(format!("{} {}", tl!("Width:"), crate::options::length(w)));
            if let Some((pw, ph)) = f.pixels {
                ui.add_space(12.0);
                let (pw, ph) = (pw.to_string(), ph.to_string());
                ui.weak(crate::i18n::fmt(tl!("{width} × {height} pixels"), &[("width", &pw), ("height", &ph)]));
            }
            ui.add_space(12.0);
            if ui.button(tl!("Reset")).clicked() {
                f.reset();
            }
        });
    }
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        return run(app, "picture.size", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Format Page Numbers

/// The Format Page Numbers dialog's fields (#1113): the number format, and where numbering
/// starts — `cont` numbers on from the section before (no `w:start`), else the pages start at
/// `start`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageNumbersForm {
    /// The section the fields were read from (see [`Self::follow`]).
    pub section: usize,
    /// OOXML number format (`decimal`, `upperRoman`, …).
    pub format: String,
    pub cont: bool,
    pub start: u32,
}

impl PageNumbersForm {
    /// The section's numbering as it is at the caret.
    pub fn read(app: &WordApp) -> PageNumbersForm {
        let sp = wordcraft_engine::cmd::page::sect(&app.session);
        PageNumbersForm {
            section: wordcraft_engine::cmd::page::section_index_of_focus(&app.session),
            format: sp.page_num_format.ooxml().to_string(),
            cont: sp.page_num_start.is_none(),
            start: sp.page_num_start.unwrap_or(1).clamp(1, 100_000),
        }
    }

    /// Follow the caret: dialogs aren't modal, so a click on the page may move it to another
    /// section. The form then shows that section's numbering, so OK never writes numbers read
    /// from another one (the Size dialog follows its object the same way).
    pub fn follow(&mut self, app: &WordApp) {
        if self.section != wordcraft_engine::cmd::page::section_index_of_focus(&app.session) {
            *self = PageNumbersForm::read(app);
        }
    }

    /// `layout.pageNumberFormat` parameters: no `start` (null) when the pages number on from the
    /// section before.
    pub fn params(&self) -> Value {
        let start = if self.cont { Value::Null } else { json!(self.start) };
        json!({"format": self.format, "start": start})
    }
}

fn page_numbers_ui(app: &mut WordApp, ui: &mut Ui, f: &mut PageNumbersForm) -> bool {
    f.follow(app);
    ui.add_space(4.0);
    ui.label(tl!("Number format"));
    let label = crate::dialogs_notes::FORMATS.iter().find(|(v, _)| *v == f.format.as_str()).map(|(_, l)| *l).unwrap_or_default();
    egui::ComboBox::from_id_salt("pnum_format").selected_text(label).width(150.0).show_ui(ui, |ui| {
        for (v, l) in crate::dialogs_notes::FORMATS {
            ui.selectable_value(&mut f.format, v.to_string(), l);
        }
    });
    heading(ui, "Page numbering");
    ui.radio_value(&mut f.cont, true, tl!("Continue from previous section"));
    ui.horizontal(|ui| {
        ui.radio_value(&mut f.cont, false, tl!("Start at:"));
        ui.add_enabled(!f.cont, egui::DragValue::new(&mut f.start).range(1..=100_000));
    });
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        return run(app, "layout.pageNumberFormat", f.params());
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

    /// #1113: Format Page Numbers opens with the section's numbering, OK sets the format and
    /// start, and "Continue from previous section" numbers on from the section before; a plain
    /// command call without `start` keeps the section's start.
    #[test]
    fn page_numbers_dialog_sets_the_format_and_start() {
        use wordcraft_doc::section::NumFormat;
        let mut a = app();
        a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
        let LayoutDialog::PageNumbers(mut f) = form(&mut a) else { panic!("page numbers") };
        assert!(f.cont && f.format == "decimal", "{f:?}");
        f.cont = false;
        f.start = 5;
        f.format = "upperRoman".into();
        a.run("layout.pageNumberFormat", f.params()).unwrap();
        let s = a.session.doc.last_section.clone();
        assert_eq!((s.page_num_format, s.page_num_start), (NumFormat::UpperRoman, Some(5)));
        // The reopened dialog shows the section's numbering; continuing clears the start.
        a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
        let LayoutDialog::PageNumbers(mut f) = form(&mut a) else { panic!("page numbers") };
        assert!(!f.cont && f.start == 5 && f.format == "upperRoman", "{f:?}");
        f.cont = true;
        a.run("layout.pageNumberFormat", f.params()).unwrap();
        let s = a.session.doc.last_section.clone();
        assert_eq!((s.page_num_format, s.page_num_start), (NumFormat::UpperRoman, None), "continue numbers on from the section before");
        // A call with no `start` keeps it (the ribbon's old plain call cleared it).
        a.run("layout.pageNumberFormat", json!({"start": 7})).unwrap();
        a.run("layout.pageNumberFormat", json!({})).unwrap();
        assert_eq!(a.session.doc.last_section.page_num_start, Some(7));
        // A valid start above 32,767 is not lowered by opening the dialog and accepting it.
        a.run("layout.pageNumberFormat", json!({"start": 40_000})).unwrap();
        a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
        let LayoutDialog::PageNumbers(f) = form(&mut a) else { panic!("page numbers") };
        assert_eq!(f.start, 40_000);
        a.run("layout.pageNumberFormat", f.params()).unwrap();
        assert_eq!(a.session.doc.last_section.page_num_start, Some(40_000));
        // The dialog follows the caret: moved to another section, it shows that section's
        // numbering, so OK never writes the numbers read from the first one.
        a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
        let LayoutDialog::PageNumbers(mut f) = form(&mut a) else { panic!("page numbers") };
        assert_eq!(f.section, 0);
        a.run("caret.end", json!({})).unwrap();
        a.run("layout.break", json!({"kind": "continuous"})).unwrap();
        a.run("caret.end", json!({})).unwrap();
        assert_eq!(wordcraft_engine::cmd::page::section_index_of_focus(&a.session), 1, "the caret is in the new section");
        a.run("layout.pageNumberFormat", json!({"format": "lowerRoman", "start": 9})).unwrap();
        f.follow(&a);
        assert_eq!((f.section, f.format.as_str(), f.start), (1, "lowerRoman", 9));
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

    /// #2717: language and no-proof can come from defaults or either kind of style.
    #[test]
    fn language_dialog_reads_inherited_proofing() {
        for source in ["default", "paragraph", "character"] {
            let mut a = app();
            a.run("select.all", json!({})).unwrap();
            let proofing = wordcraft_doc::CharProps { lang: Some("en-GB".into()), no_proof: Some(true), ..Default::default() };
            match source {
                "default" => a.session.doc.styles.default_chr.overlay(&proofing),
                "paragraph" => a.session.doc.styles.styles.iter_mut().find(|s| s.id == "Normal").unwrap().chr.overlay(&proofing),
                _ => {
                    a.session.doc.styles.styles.push(wordcraft_doc::Style {
                        id: "ProofingCharacter".into(),
                        kind: wordcraft_doc::StyleKind::Character,
                        chr: proofing,
                        ..Default::default()
                    });
                    let (from, to) = a.session.sel.ordered();
                    let (from, to) = (from.clone(), to.clone());
                    a.session.doc.format_range(&from, &to, &|c| c.style = Some("ProofingCharacter".into())).unwrap();
                }
            }
            let f = LanguageForm::read(&a);
            assert_eq!((f.lang.as_str(), f.no_proof), ("en-GB", true), "{source}");
            a.run("review.language", json!({"lang": "fr-FR", "noProof": false})).unwrap();
            let f = LanguageForm::read(&a);
            assert_eq!((f.lang.as_str(), f.no_proof), ("fr-FR", false), "direct formatting overrides {source}");
        }
    }

    /// #2717: opening and accepting the dialog must not reset inherited proofing.
    #[test]
    fn language_dialog_ok_keeps_inherited_proofing() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("ui.language", json!({"value": "en"})).unwrap();
        let normal = a.session.doc.styles.styles.iter_mut().find(|s| s.id == "Normal").unwrap();
        normal.chr.lang = Some("en-GB".into());
        normal.chr.no_proof = Some(true);
        a.run("select.all", json!({})).unwrap();
        let before = a.session.doc.clone();
        a.run("review.language", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(3);
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert_eq!(h.state().session.doc, before, "cancel must not format the selection");
        h.state_mut().run("review.language", json!({})).unwrap();
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().dialog.is_none());
        let state = h.state_mut().run("format.state", json!({})).unwrap();
        assert_eq!(state["lang"], "en-GB");
        assert_eq!(state["noProof"], true);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(h.state().session.doc, before, "one undo restores the inherited formatting");
    }

    /// Set As Default and OK persist the chosen proofing language in a fresh app.
    #[test]
    fn language_dialog_default_survives_a_restart() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.ui.language = "en".into();
        a.dialog = Some(Dialog::Layout(Box::new(LayoutDialog::Language(LanguageForm { lang: "fr-FR".into(), no_proof: false, detect: false }))));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                app.logic(&ui.ctx().clone());
                app.ui(ui);
            },
            a,
        );
        h.run_steps(3);
        h.get_by_label("Set As Default").click();
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        let saved = serde_json::to_vec(&h.state().prefs()).unwrap();
        let mut b = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        b.apply_prefs(serde_json::from_slice(&saved).unwrap());
        b.logic(&egui::Context::default());
        assert_eq!(LanguageForm::read(&b).lang, "fr-FR");
        assert!(!b.session.prefs.detect_language);
        assert!(!b.session.dirty && !b.session.can_undo());
        b.run("file.new", json!({})).unwrap();
        assert_eq!(LanguageForm::read(&b).lang, "fr-FR");
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

    fn table_shape(a: &WordApp) -> Vec<usize> {
        a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows.iter().map(|r| r.cells.len()).collect()
    }

    /// #2592: Split Cells opens its dialog from the ribbon's command (scripts with settings never
    /// see it); OK splits the cell into the entered columns and rows. Merge cells before split is
    /// offered, and on, when several cells are selected.
    #[test]
    fn split_cells_opens_its_dialog() {
        let mut a = table_app(2, &["A", "B", "C", "D"]);
        assert_eq!(a.run("table.split", json!({})).unwrap(), json!({"pending": "splitCells"}));
        assert_eq!(table_shape(&a), [2, 2], "nothing split yet");
        let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
        assert_eq!((f.columns, f.rows, f.several), (2, 1, false));
        assert_eq!(f.params()["mergeFirst"], false);
        (f.columns, f.rows) = (3, 2);
        a.run("table.split", f.params()).unwrap();
        assert_eq!(table_shape(&a), [2, 4, 4]);
        assert!(a.dialog.is_none());

        let mut a = table_app(2, &["A", "B", "C", "D"]);
        let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
        a.run("select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 1)})).unwrap();
        a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
        let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
        assert!(f.several && f.merge_first);
        (f.columns, f.rows) = (2, 1);
        a.run("table.split", f.params()).unwrap();
        // The merged cell spans both rows; its two parts do too.
        assert_eq!(table_shape(&a), [2, 2], "merged into one cell, then split in two");
        let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        assert!(t.rows[1].cells.iter().all(|c| c.props.vmerge == wordcraft_doc::props::VMerge::Continue));

        let mut a = app();
        a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
        assert!(a.dialog.is_none(), "no dialog outside a table");
    }

    /// #2592, as Word does: for a merged cell the dialog's message says the row count must
    /// divide its height (or lie in 1 to the height); the engine refuses the same counts and the
    /// document stays as it was.
    #[test]
    fn split_cells_dialog_checks_a_merged_cells_row_count() {
        let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
        let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
        a.run("select.range", json!({"anchor": cell(0, 0), "focus": cell(2, 0)})).unwrap();
        a.run("table.merge", json!({})).unwrap();
        a.run("caret.set", json!({"pos": cell(0, 0)})).unwrap();
        a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
        let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
        assert_eq!(f.height, 3);
        for (rows, ok) in [(1, true), (2, false), (3, true), (4, false)] {
            f.rows = rows;
            assert_eq!(f.check().is_none(), ok, "rows {rows}");
        }
        f.rows = 2;
        assert!(f.check().is_some_and(|m| m.contains("divisor of 3")));
        let (depth, shape) = (a.session.undo_depth(), table_shape(&a));
        assert!(a.run("table.split", f.params()).is_err());
        assert_eq!((a.session.undo_depth(), table_shape(&a)), (depth, shape));
    }

    /// #2592, as Word does: Split Cells starts with twice the selected columns and the selected
    /// rows; Merge is off and greyed for one cell. Unticking it splits each cell on its own: one
    /// row and two columns again.
    #[test]
    fn split_cells_defaults_follow_the_selection() {
        let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
        let defaults = |anchor: Value, focus: Value| {
            let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
            a.run("select.range", json!({"anchor": anchor, "focus": focus})).unwrap();
            a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
            let LayoutDialog::SplitCells(f) = form(&mut a) else { panic!("split cells") };
            f
        };
        let f = defaults(cell(1, 1), cell(1, 1));
        assert_eq!((f.columns, f.rows, f.several, f.merge_first), (2, 1, false, false), "one cell");
        let f = defaults(cell(0, 0), cell(1, 0));
        assert_eq!((f.columns, f.rows, f.merge_first), (2, 2, true), "two cells in a column");
        let f = defaults(cell(0, 0), cell(0, 2));
        assert_eq!((f.columns, f.rows, f.merge_first), (6, 1, true), "a row of three");
        let mut f = defaults(cell(0, 0), cell(1, 2));
        assert_eq!((f.columns, f.rows, f.merge_first, f.max_rows), (6, 2, true, 2), "two rows of three");
        f.set_merge(false);
        assert_eq!((f.columns, f.rows), (2, 1), "each cell on its own");
        assert_eq!(f.params()["mergeFirst"], false);

        // A merged cell is one cell, wherever in it the selection is.
        let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
        a.run("select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 0)})).unwrap();
        a.run("table.merge", json!({})).unwrap();
        a.run("select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 0)})).unwrap();
        a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
        let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
        assert_eq!((f.columns, f.rows, f.several, f.merge_first), (2, 1, false, false), "one merged cell");
        // Word allows 1 to its height rows, with its message, and keeps the dialog open.
        f.rows = 3;
        assert_eq!(f.check().as_deref(), Some("The number must be between 1 and 2."));
        f.rows = 2;
        assert_eq!(f.check(), None);

        // The merged cell and the two cells to its right: Merge ticked, 4 × 2.
        let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
        a.run("select.range", json!({"anchor": cell(0, 1), "focus": cell(1, 1)})).unwrap();
        a.run("table.merge", json!({})).unwrap();
        a.run("select.range", json!({"anchor": cell(0, 1), "focus": cell(1, 2)})).unwrap();
        a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
        let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
        assert_eq!((f.columns, f.rows, f.several, f.merge_first), (4, 2, true, true), "merged cell and neighbours");
        f.rows = 3;
        assert_eq!(f.check().as_deref(), Some("The number must be between 1 and 2."));

        // One cell merged across two columns: Merge offered and ticked, 2 × 1.
        let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
        a.run("select.range", json!({"anchor": cell(2, 0), "focus": cell(2, 1)})).unwrap();
        a.run("table.merge", json!({})).unwrap();
        a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
        let LayoutDialog::SplitCells(f) = form(&mut a) else { panic!("split cells") };
        assert_eq!((f.columns, f.rows, f.several, f.merge_first), (2, 1, true, true), "a wide cell");
        assert_eq!(f.check(), None);
    }

    /// #2592: Convert Text to Table opens its dialog; the separator and column count are guessed
    /// from the text, the row count follows the columns, and OK converts with the settings.
    #[test]
    fn text_to_table_opens_its_dialog() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("a\tb\tc\nd")), Services::default());
        a.run("select.all", json!({})).unwrap();
        assert_eq!(a.run("table.fromText", json!({})).unwrap(), json!({"pending": "textToTable"}));
        assert!(a.session.doc.body.iter().all(|b| b.as_table().is_none()), "nothing converted yet");
        let LayoutDialog::TextToTable(mut f) = form(&mut a) else { panic!("text to table") };
        assert_eq!((f.separator.as_str(), f.columns, f.rows, f.autofit.as_str(), f.width.as_str()), ("tab", 3, 2, "initial", "Auto"));
        assert!(f.params().get("width").is_none(), "Auto");
        f.width = "1 in".into();
        assert_eq!((f.check(), f.params()["width"].as_f64()), (None, Some(72.0)));
        f.width = "50 in".into();
        assert!(f.check().is_some_and(|e| e.starts_with("The measurement must be between 0")), "{:?}", f.check());
        f.width = "0".into();
        assert_eq!((f.check(), f.params().get("width")), (None, None), "0 is Auto");
        f.width = "Auto".into();
        f.separator = "paragraph".into();
        f.guess_columns();
        assert_eq!((f.columns, f.rows), (1, 2));
        f.separator = "tab".into();
        f.columns = 2;
        f.count_rows();
        assert_eq!(f.rows, 3, "a b / c / d");
        a.run("table.fromText", f.params()).unwrap();
        assert!(a.dialog.is_none());
        let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        assert_eq!(t.rows.len(), 3);
        assert!(!t.props.fixed, "the table still resizes to fit its contents");

        let mut a = app();
        a.run("select.all", json!({})).unwrap();
        a.run("ui.dialog", json!({"name": "textToTable"})).unwrap();
        let LayoutDialog::TextToTable(f) = form(&mut a) else { panic!("text to table") };
        assert_eq!((f.separator.as_str(), f.columns, f.rows), ("paragraph", 1, 2), "a comma in one paragraph only");

        // Word opens comma text on Paragraphs too; Commas counts the columns, short lines pad.
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("a,b,c\nd,e,f\ng,h\ni")), Services::default());
        a.run("select.all", json!({})).unwrap();
        a.run("table.fromText", json!({})).unwrap();
        let LayoutDialog::TextToTable(mut f) = form(&mut a) else { panic!("text to table") };
        assert_eq!((f.separator.as_str(), f.columns, f.rows, f.other_char.as_str()), ("paragraph", 1, 4, "-"));
        f.separator = "comma".into();
        f.guess_columns();
        assert_eq!((f.columns, f.rows), (3, 4));
        a.run("table.fromText", f.params()).unwrap();
        let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        assert_eq!(t.rows.iter().map(|r| r.cells.len()).collect::<Vec<_>>(), [3, 3, 3, 3]);
    }

    /// Convert to Text opens its dialog on Tabs; OK converts with the separator chosen.
    #[test]
    fn table_to_text_opens_its_dialog() {
        let mut a = table_app(2, &["a", "b", "c", "d"]);
        assert_eq!(a.run("table.toText", json!({})).unwrap(), json!({"pending": "tableToText"}));
        assert!(a.session.doc.body.iter().any(|b| b.as_table().is_some()), "nothing converted yet");
        let LayoutDialog::TableToText(mut f) = form(&mut a) else { panic!("table to text") };
        assert_eq!((f.separator.as_str(), f.other_char.as_str()), ("tab", "-"));
        assert_eq!(f.params(), json!({"separator": "tab"}));
        f.separator = "other".into();
        f.other_char = "/".into();
        assert_eq!(f.params(), json!({"separator": "other", "char": "/"}));
        a.run("table.toText", f.params()).unwrap();
        assert!(a.dialog.is_none());
        let texts: Vec<&str> = a.session.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.as_str()).collect();
        assert!(texts.contains(&"a/b") && texts.contains(&"c/d"), "{texts:?}");
        assert!(a.session.doc.body.iter().all(|b| b.as_table().is_none()));

        // Outside a table there is nothing to convert.
        let mut a = app();
        assert_eq!(a.run("ui.dialog", json!({"name": "tableToText"})).unwrap(), json!({"dialog": null}));
    }

    /// The envelope sizes go through the interface language's catalog and use the region's
    /// decimal separator (#1128; the untranslated-text audit found `#10   4.13 × 9.5 in` drawn in
    /// English).
    #[test]
    fn envelope_sizes_follow_the_interface_language_and_region() {
        let size = |name: &str| ENVELOPE_SIZES.iter().find(|s| s.0 == name).map(|(n, w, h)| envelope_label(n, *w, *h)).unwrap();
        assert_eq!(size("Envelope #10"), "#10   4.13 × 9.5 in");
        assert_eq!(size("Envelope DL"), "DL   110 × 220 mm");
        let mut a = app();
        a.run("file.options", json!({"region": "it-IT"})).unwrap();
        crate::options::sync(&a);
        assert_eq!(size("Envelope #10"), "#10   4,13 × 9,5 in");
        crate::i18n::pseudo::set(true);
        assert_eq!(size("Envelope #10"), "⟦#10   4,13 × 9,5 in⟧");
        assert_eq!(size("Envelope DL"), "⟦DL   110 × 220 mm⟧");
        crate::i18n::pseudo::set(false);
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

    /// A `w`×`h` pixel PNG, base64 for `insert.picture`.
    fn png_data(w: u32, h: u32) -> String {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        wordcraft_engine::cmd::insert::base64_encode(&png)
    }

    /// An app with a `w`×`h` pixel picture, inserted and selected as by Insert › Pictures.
    fn picture_app(w: u32, h: u32) -> WordApp {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("insert.picture", json!({"data": png_data(w, h)})).unwrap();
        a
    }

    fn selected_size(a: &WordApp) -> (f32, f32) {
        let (_, o) = wordcraft_engine::cmd::objects::selected(&a.session).unwrap();
        o.frame().map(|(w, h, _)| (w, h)).unwrap()
    }

    /// #2573: Picture Format › Size's launcher (`picture.size` without a size) opens the Size
    /// dialog on the selected picture: its size, the scale of its original size, Lock aspect ratio
    /// and Reset; OK resizes it in one undo step.
    #[test]
    fn size_dialog_scales_locks_and_resets_the_picture() {
        let mut a = picture_app(40, 20);
        assert_eq!(selected_size(&a), (30.0, 15.0), "40×20 px at 96 ppi");
        a.run("picture.size", json!({"width": 60.0, "height": 15.0, "lockAspect": false})).unwrap();
        assert_eq!(a.run("picture.size", json!({})).unwrap(), json!({"pending": "pictureSize"}));
        assert_eq!(selected_size(&a), (60.0, 15.0), "opening the dialog changes nothing");
        let LayoutDialog::Size(mut f) = form(&mut a) else { panic!("size") };
        assert!(f.lock && f.relative, "a picture opens locked, scaled from its original size");
        assert_eq!((f.original, f.uncropped, f.pixels), (Some((30.0, 15.0)), Some((30.0, 15.0)), Some((40, 20))));
        assert_eq!(f.scale(), (200.0, 100.0));
        // Locked, the other side follows in proportion, in size and in scale.
        f.set_scale_height(200.0);
        assert_eq!((f.width, f.height, f.scale()), (120.0, 30.0, (400.0, 200.0)));
        f.set_width(60.0);
        assert_eq!((f.width, f.height), (60.0, 15.0));
        f.lock = false;
        f.set_height(45.0);
        assert_eq!((f.width, f.height), (60.0, 45.0), "unlocked, one side alone");
        f.reset();
        assert_eq!((f.width, f.height, f.scale()), (30.0, 15.0, (100.0, 100.0)), "Reset: the original size");
        f.relative = false;
        assert_eq!(f.scale(), (50.0, 100.0), "from the size the dialog opened with");
        f.set_scale_width(150.0);
        assert_eq!((f.width, f.height), (90.0, 15.0));
        a.run("picture.size", f.params()).unwrap();
        assert_eq!(selected_size(&a), (90.0, 15.0));
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(selected_size(&a), (60.0, 15.0), "one undo step");
    }

    /// #2573: a cropped picture's Reset is what the crop leaves of the original; a shape opens the
    /// dialog unlocked with no original size; with nothing selected nothing opens.
    #[test]
    fn size_dialog_follows_the_crop_and_shapes_have_no_original() {
        let mut a = picture_app(40, 20);
        a.run("picture.crop", json!({"left": 0.25, "right": 0.25})).unwrap();
        a.run("picture.size", json!({})).unwrap();
        let LayoutDialog::Size(mut f) = form(&mut a) else { panic!("size") };
        assert_eq!((f.original, f.uncropped), (Some((15.0, 15.0)), Some((30.0, 15.0))));
        f.reset();
        assert_eq!((f.width, f.height), (15.0, 15.0));

        let mut a = app();
        assert!(!crate::widgets::enabled(&a, "picture.size"), "nothing to size");
        let _ = a.run("ui.dialog", json!({"name": "pictureSize"}));
        assert!(a.dialog.is_none());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("picture.size", json!({})).unwrap();
        let LayoutDialog::Size(f) = form(&mut a) else { panic!("size") };
        assert!(!f.lock && !f.relative && f.original.is_none() && f.uncropped.is_none());
        assert_eq!(f.scale(), (100.0, 100.0));
    }

    /// #2573 review: locked, a side past 4000 pt shrinks the other with it instead of flattening
    /// the picture, and so does Reset for a photo larger than 4000 pt; under 1 pt likewise.
    #[test]
    fn size_dialog_keeps_the_proportions_at_the_limits() {
        let close = |(w, h): (f32, f32), (ew, eh): (f32, f32)| (w - ew).abs() < 0.01 && (h - eh).abs() < 0.01;
        let mut a = picture_app(6000, 30);
        a.run("picture.size", json!({})).unwrap();
        let LayoutDialog::Size(mut f) = form(&mut a) else { panic!("size") };
        assert_eq!(f.original, Some((4500.0, 22.5)), "6000×30 px at 96 ppi");
        f.reset();
        assert!(close((f.width, f.height), (4000.0, 20.0)), "Reset: {}×{}", f.width, f.height);
        a.run("picture.size", f.params()).unwrap();
        assert!(close(selected_size(&a), (4000.0, 20.0)), "applied: {:?}", selected_size(&a));
        (f.width, f.height) = (15.0, 30.0);
        f.set_width(4000.0);
        assert_eq!((f.width, f.height), (2000.0, 4000.0));
        f.set_height(0.5);
        assert_eq!((f.width, f.height), (1.0, 2.0));
        f.lock = false;
        f.set_width(9000.0);
        assert_eq!((f.width, f.height), (4000.0, 2.0), "unlocked, one side alone");
    }

    /// #2573 review: dialogs aren't modal. Cropping the picture or picking another one while the
    /// Size dialog is open starts the form over, so OK never sizes a picture with stale numbers;
    /// with nothing left to size the dialog closes.
    #[test]
    fn size_dialog_follows_the_selection() {
        let mut a = picture_app(40, 20);
        a.run("picture.size", json!({})).unwrap();
        let LayoutDialog::Size(mut f) = form(&mut a) else { panic!("size") };
        f.set_width(300.0);
        assert!(f.follow(&a) && f.width == 300.0, "the same picture keeps the edits");
        // Cropped from the ribbon meanwhile: read again, Reset is what the new crop leaves.
        a.run("picture.crop", json!({"left": 0.25, "right": 0.25})).unwrap();
        assert!(f.follow(&a));
        assert_eq!((f.width, f.original), (30.0, Some((15.0, 15.0))));
        a.run("select.collapse", json!({"end": true})).unwrap();
        a.run("insert.picture", json!({"data": png_data(20, 40)})).unwrap();
        assert!(f.follow(&a), "another picture selected");
        assert_eq!((f.width, f.height, f.original), (15.0, 30.0, Some((15.0, 30.0))));
        a.run("select.collapse", json!({"end": true})).unwrap();
        a.run("text.insert", json!({"text": " caption"})).unwrap();
        assert!(!f.follow(&a), "nothing to size");
    }

    /// #2573 review: the Size dialog end to end. Its launcher (`picture.size` without a size)
    /// opens it; a height typed in pixels changes nothing until OK, which sizes the picture in
    /// proportion as one undo step and closes the dialog.
    #[test]
    fn size_dialog_typed_height_applies_on_ok_in_proportion() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut a = picture_app(40, 20);
        a.run("picture.size", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        // The Size section's Height (the Scale section has one too, after it).
        h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("the height field").click();
        h.run_steps(2);
        h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("editing").type_text("60 px");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert!(h.state().dialog.is_some());
        assert_eq!(selected_size(h.state()), (30.0, 15.0), "nothing applies before OK");
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(2);
        assert!(h.state().dialog.is_none(), "OK closes the dialog");
        assert_eq!(selected_size(h.state()), (90.0, 45.0), "60 px = 45 pt high, the width follows");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(selected_size(h.state()), (30.0, 15.0), "one undo step");
    }

    /// #2573 review: a value outside a field's range (a 1 px picture enlarged past 10000%) is shown
    /// as it is. egui would otherwise clamp it on the first frame and report a change, which the
    /// form would take as an edit and OK would apply unasked.
    #[test]
    fn size_dialog_leaves_out_of_range_values_alone() {
        let mut a = picture_app(1, 1);
        a.run("picture.size", json!({"width": 100.0, "height": 100.0, "lockAspect": false})).unwrap();
        a.run("picture.size", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let Some(Dialog::Layout(d)) = &h.state().dialog else { panic!("the Size dialog is open") };
        let LayoutDialog::Size(f) = d.as_ref() else { panic!("size") };
        assert!((f.scale().0 - 100.0 / 0.75 * 100.0).abs() < 1.0, "13333%: {:?}", f.scale());
        assert_eq!((f.width, f.height), (100.0, 100.0), "the form keeps the size");
        assert_eq!(selected_size(h.state()), (100.0, 100.0));
    }

    /// #2573 review: a typed size is kept when the field loses focus, as text fields do: typing a
    /// height and going straight for OK (a real click: press, then release frames later) applies
    /// it. Taking it only on Enter would throw the typed size away. Esc cancels, applying nothing.
    #[test]
    fn size_dialog_typed_height_then_ok_without_enter_applies_it() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let harness = |a: WordApp| {
            egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                a,
            )
        };
        let mut a = picture_app(40, 20);
        a.run("picture.size", json!({})).unwrap();
        let mut h = harness(a);
        h.run_steps(4);
        h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("the height field").click();
        h.run_steps(2);
        h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("editing").type_text("60 px");
        h.run_steps(2);
        let ok = h.get_by_role_and_label(Role::Button, "OK").rect().center();
        h.event(egui::Event::PointerMoved(ok));
        h.event(egui::Event::PointerButton { pos: ok, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: ok, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);
        assert!(h.state().dialog.is_none(), "OK closes the dialog");
        assert_eq!(selected_size(h.state()), (90.0, 45.0), "the typed height, in proportion");

        // Esc throws the typing away and, as Cancel, closes the dialog: nothing applied.
        let mut a = picture_app(40, 20);
        a.run("picture.size", json!({})).unwrap();
        let mut h = harness(a);
        h.run_steps(4);
        h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("the height field").click();
        h.run_steps(2);
        h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("editing").type_text("60 px");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().dialog.is_none(), "Esc cancels the dialog");
        assert_eq!(selected_size(h.state()), (30.0, 15.0), "nothing applied");
    }
}
