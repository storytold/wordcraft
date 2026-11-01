use super::{LineNumberRestart, MeasureExt, Serialize, Ui, Value, WordApp, buttons, heading, json, run, unit, vec2};

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

pub(super) fn line_numbers_ui(app: &mut WordApp, ui: &mut Ui, f: &mut LineNumbersForm) -> bool {
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

pub(super) fn hyphenation_ui(app: &mut WordApp, ui: &mut Ui, f: &mut HyphenationForm) -> bool {
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

pub(super) fn manual_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ManualForm) -> bool {
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
pub(super) fn language_name(tag: &str) -> String {
    LANGUAGES.iter().find(|(t, _)| t.eq_ignore_ascii_case(tag)).map(|(_, n)| tl!(n).to_string()).unwrap_or_else(|| tag.to_string())
}

pub(super) fn language_ui(app: &mut WordApp, ui: &mut Ui, f: &mut LanguageForm) -> bool {
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
