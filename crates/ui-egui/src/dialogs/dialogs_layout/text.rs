use craft::behavior::{Selection, SelectionChange, keyed};
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup, Segmented};
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, FieldLayout, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, SideLabel,
};
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::list::{List, ListRow};
// PLAN: assumed `FormButtons` re-exported beside `FormCx` (dialogs-w5)
use craft::ui::patterns::form_dialogs::{FormButtons, FormCx};
use craft::ui::runtime::env::{Label, LengthUnits};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::{NamedOr, NumberName};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::section::LineNumberRestart;

use super::{WordApp, count_field, length_field, named_field, number_or_zero, points};
use crate::dialogs::host::{Asks, Filed, WordForm};

/// The field's one name: no limit on consecutive hyphens.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NoLimit {
    NoLimit,
}

impl NumberName for NoLimit {
    fn id(self) -> u32 {
        0
    }

    fn from_id(id: u32) -> Option<Self> {
        (id == 0).then_some(NoLimit::NoLimit)
    }
}

// ---------------------------------------------------------------------------------------------
// Line Numbers

/// The Line Numbers dialog's fields. `distance` is in points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LineNumbersForm {
    pub on: bool,
    pub start: u32,
    /// From text: automatic, or `distance`.
    pub auto_distance: bool,
    pub distance: f32,
    pub count_by: u32,
    /// `restartPage`, `restartSection` or `continuous`.
    pub restart: String,
    /// The user's length unit, which only the app knows; read with the form.
    #[serde(skip)]
    lengths: Option<LengthUnits>,
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
            distance: if cur.distance > 0.0 { cur.distance } else { 0.25 * 72.0 },
            count_by: cur.count_by.max(1),
            restart: match cur.restart {
                LineNumberRestart::Page => "restartPage",
                LineNumberRestart::Section => "restartSection",
                LineNumberRestart::Continuous => "continuous",
            }
            .into(),
            lengths: Some(crate::options::lengths(app.session.units())),
        }
    }

    /// `layout.lineNumbers` parameters.
    pub fn params(&self) -> Value {
        if !self.on {
            return json!({"value": "none"});
        }
        let distance = if self.auto_distance { 0.0 } else { self.distance.max(0.0) };
        json!({"value": self.restart, "start": self.start, "countBy": self.count_by, "distance": distance})
    }
}

impl WordForm for LineNumbersForm {
    fn key(&self) -> &'static str {
        "lineNumbers"
    }

    fn title(&self) -> Label {
        tl!("Line Numbers").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let lengths = self.lengths.unwrap_or_else(points);
        Checkbox::new(Bind::bound(&mut self.on)).label(tl!("Add line numbering").to_string()).show(ui);
        let on = self.on;
        count_field(ui, tl!("Start at:"), &mut self.start, 1..=32_767, on);
        Checkbox::new(Bind::bound(&mut self.auto_distance)).label(tl!("Auto").to_string()).enabled(on).show(ui);
        length_field(ui, tl!("From text:"), &mut self.distance, lengths, 0.0..=22.0 * 72.0, on && !self.auto_distance);
        count_field(ui, tl!("Count by:"), &mut self.count_by, 1..=100, on);
        let opts = [("restartPage", tl!("Restart Each Page")), ("restartSection", tl!("Restart Each Section")), ("continuous", tl!("Continuous"))];
        RadioGroup::new(tl!("Numbering:").to_string(), Bind::bound(&mut self.restart)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, cx| {
                for (at, (v, l)) in opts.iter().enumerate() {
                    cx.labelled(ui, at, Label::from(*l), SideLabel::new().layout(FieldLayout::After), Radio::new((*v).to_string()));
                }
            });
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("layout.lineNumbers".to_string(), self.params())]
    }
}

// ---------------------------------------------------------------------------------------------
// Hyphenation

/// The Hyphenation dialog's fields. `zone` is in points; `limit` 0 = no limit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HyphenationForm {
    pub auto: bool,
    pub caps: bool,
    pub zone: f32,
    pub limit: u32,
    /// The user's length unit, which only the app knows; read with the form.
    #[serde(skip)]
    lengths: Option<LengthUnits>,
}

impl HyphenationForm {
    pub fn read(app: &WordApp) -> HyphenationForm {
        let s = &app.session.doc.settings;
        HyphenationForm {
            auto: s.auto_hyphenation,
            caps: s.hyphenate_caps,
            zone: s.hyphenation_zone.max(0.0),
            limit: s.consecutive_hyphen_limit,
            lengths: Some(crate::options::lengths(app.session.units())),
        }
    }

    /// `layout.hyphenation` parameters.
    pub fn params(&self) -> Value {
        json!({"value": self.auto, "caps": self.caps, "zone": self.zone.max(0.0), "limit": self.limit})
    }
}

impl WordForm for HyphenationForm {
    fn key(&self) -> &'static str {
        "hyphenation"
    }

    fn title(&self) -> Label {
        tl!("Hyphenation").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let lengths = self.lengths.unwrap_or_else(points);
        Checkbox::new(Bind::bound(&mut self.auto)).label(tl!("Automatically hyphenate document").to_string()).show(ui);
        Checkbox::new(Bind::bound(&mut self.caps)).label(tl!("Hyphenate words in CAPS").to_string()).show(ui);
        length_field(ui, tl!("Hyphenation zone:"), &mut self.zone, lengths, 0.0..=22.0 * 72.0, true);
        let mut limit = if self.limit == 0 { NamedOr::Named(NoLimit::NoLimit) } else { NamedOr::Number(f64::from(self.limit)) };
        named_field(ui, tl!("Limit consecutive hyphens to:"), &mut limit, (NoLimit::NoLimit, tl!("No limit")), None, 1.0..=99.0, true);
        self.limit = number_or_zero(&limit).round().clamp(0.0, 99.0) as u32;
        // Manual… keeps what was set here and moves on to manual hyphenation, over this dialog.
        if Button::new(tl!("Manual…").to_string()).show(ui).clicked() {
            asks.push(Asks::Run("layout.hyphenation".to_string(), self.params()));
            asks.push(Asks::Open("manualHyphenation".to_string(), Value::Null));
        }
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("layout.hyphenation".to_string(), self.params())]
    }
}

// ---------------------------------------------------------------------------------------------
// Manual hyphenation

/// One word at a time: where it may be hyphenated and where the hyphen goes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
}

/// A position (`{"story", "path", "off"}`) at another offset in the same paragraph.
fn with_off(pos: &Value, off: u64) -> Value {
    let mut p = pos.as_object().cloned().unwrap_or_default();
    p.insert("off".into(), json!(off));
    Value::Object(p)
}

impl WordForm for ManualForm {
    fn key(&self) -> &'static str {
        "manualHyphenation"
    }

    fn title(&self) -> Label {
        tl!("Manual Hyphenation").to_string().into()
    }

    // PLAN: assumed `WordForm::buttons` and `WordForm::cancel_label` (dialogs-w13), `FormButtons::Close` (dialogs-w5)
    fn buttons(&self) -> FormButtons {
        FormButtons::Close
    }

    fn cancel_label(&self) -> Option<Label> {
        (!self.done).then(|| tl!("Cancel").to_string().into())
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        if self.done {
            Text::new(tl!("Manual hyphenation is complete.").to_string()).show(ui);
            return true;
        }
        // The word with every possible break: each segment is the piece up to a break and its value the break's char index.
        let mut from = 0;
        let mut entries: Vec<Entry<usize>> = Vec::new();
        for &pt in &self.points {
            let piece: String = self.word.chars().skip(from).take(pt.saturating_sub(from)).collect();
            entries.push(Entry::new(pt, format!("{piece}-")));
            from = pt;
        }
        if !entries.is_empty() {
            Segmented::new(tl!("Hyphenate at:").to_string(), Bind::bound(&mut self.choice), &entries).show(ui);
        }
        let last: String = self.word.chars().skip(from).collect();
        Text::new(last).show(ui);
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        Vec::new()
    }

    fn extra_buttons(&self) -> Vec<FooterButton> {
        if self.done {
            return Vec::new();
        }
        vec![
            FooterButton::new(tl!("Yes").to_string(), DialogRole::Accept).stays_open(true),
            FooterButton::new(tl!("No").to_string(), DialogRole::Other).stays_open(true),
        ]
    }

    /// Yes puts an optional hyphen at the chosen point; No changes nothing.
    fn extra(&mut self, at: usize) -> Vec<(String, Value)> {
        match at {
            0 => vec![("caret.set".to_string(), json!({"pos": self.at(self.byte_of(self.choice))})), ("text.optionalHyphen".to_string(), json!({}))],
            _ => Vec::new(),
        }
    }

    /// Both answers go on to the next word, after the word (and the soft hyphen Yes inserted).
    fn applied(&mut self, app: &mut WordApp, filed: Filed, outcome: Result<&[Value], &str>) {
        let Filed::Extra(at) = filed else { return };
        if let Err(e) = outcome {
            app.status(e.to_owned());
            return;
        }
        let past = if at == 0 { self.word.len() + '\u{ad}'.len_utf8() } else { self.word.len() };
        let after = self.at(past);
        *self = Self::next(app, after);
    }
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LanguageForm {
    pub lang: String,
    pub no_proof: bool,
    pub detect: bool,
    /// "Apply to the whole document": OK sends `scope: "document"` (every story), not the selection.
    pub whole_document: bool,
    /// The language and Do-not-check state the dialog opened with, to tell what the user changed.
    #[serde(skip)]
    pub initial: (String, bool),
    /// The Do-not-check box was clicked (even back to where it started). A whole-document OK sends
    /// `noProof` only then: the caret's value alone says nothing about runs that differ elsewhere,
    /// but a click is a request to mark the whole document that way (#3518).
    #[serde(skip)]
    pub proof_touched: bool,
}

impl LanguageForm {
    pub fn read(app: &WordApp) -> LanguageForm {
        let s = &app.session;
        let style = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.style.as_deref());
        let here = s.doc.styles.resolve_char(style, &s.typing_props());
        let lang = here.lang.unwrap_or_else(|| "en-US".into());
        LanguageForm {
            initial: (lang.clone(), here.no_proof),
            lang,
            no_proof: here.no_proof,
            detect: s.prefs.detect_language,
            whole_document: false,
            proof_touched: false,
        }
    }

    /// `review.language` parameters for OK.
    pub fn params(&self) -> Value {
        if !self.whole_document {
            return json!({"lang": self.lang, "noProof": self.no_proof, "detect": self.detect});
        }
        // Whole document: the caret's values must not flatten what differs elsewhere, so send only
        // what was changed. With no edits at all the language goes (never the engine's en-US default).
        let (lang_changed, proof_touched) = (self.lang != self.initial.0, self.proof_touched);
        let mut v = json!({"scope": "document", "detect": self.detect});
        if let Some(m) = v.as_object_mut() {
            if lang_changed || !proof_touched {
                m.insert("lang".into(), json!(self.lang));
            }
            // Only a clicked box is sent: the caret's value untouched says nothing about runs that
            // differ elsewhere, and resending it would flatten them.
            if proof_touched {
                m.insert("noProof".into(), json!(self.no_proof));
            }
        }
        v
    }
}

/// A language's name in the interface language (the tag itself when we don't know it).
pub(super) fn language_name(tag: &str) -> String {
    crate::ribbon::display_name(Some(tag))
}

impl WordForm for LanguageForm {
    fn key(&self) -> &'static str {
        "language"
    }

    fn title(&self) -> Label {
        tl!("Language").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let heading = if self.whole_document { tl!("Mark the whole document as:") } else { tl!("Mark selected text as:") };
        let list = crate::ribbon::language_list(&self.lang);
        let at = list.iter().position(|(tag, _)| crate::ribbon::same_language(&self.lang, tag));
        let which: Vec<usize> = (0..list.len()).collect();
        let source = keyed(which.as_slice(), |i: &usize| *i);
        let selection = at.map_or_else(|| Selection::new(Vec::new(), None, None), Selection::single);
        let shown = List::<usize>::new(Label::from(heading.to_string()))
            .select(SelectMode::Single)
            .selection(Bind::given(selection))
            // The dialog body scrolls, and a collection owns its own scroll: the long language list is capped to rows.
            .max_rows(10)
            .show_items(ui, &source, &mut |i, _| ListRow::new(Label::from(list.get(*i).map(|(_, n)| n.clone()).unwrap_or_default())));
        // A language is always chosen: a Clear leaves it, and a Toggle of the chosen row picks it again.
        let picked = shown.events.into_iter().find_map(|event| match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => keys.last().copied(),
            CollectionEvent::Selection(SelectionChange::Toggle(key)) => Some(key),
            _ => None,
        });
        if let Some((tag, _)) = picked.and_then(|i| list.get(i)) {
            self.lang = tag.clone();
        }
        let no_proof = self.no_proof;
        Checkbox::new(Bind::bound(&mut self.no_proof)).label(tl!("Do not check spelling or grammar").to_string()).show(ui);
        if self.no_proof != no_proof {
            self.proof_touched = true;
        }
        Checkbox::new(Bind::bound(&mut self.detect)).label(tl!("Detect language automatically").to_string()).show(ui);
        Checkbox::new(Bind::bound(&mut self.whole_document)).label(tl!("Apply to the whole document").to_string()).show(ui);
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("review.language".to_string(), self.params())]
    }

    fn extra_buttons(&self) -> Vec<FooterButton> {
        vec![FooterButton::new(tl!("Set As Default").to_string(), DialogRole::Other).stays_open(true)]
    }

    fn extra(&mut self, at: usize) -> Vec<(String, Value)> {
        match at {
            0 => vec![("review.language".to_string(), json!({"lang": self.lang, "default": true, "detect": self.detect}))],
            _ => Vec::new(),
        }
    }

    fn applied(&mut self, app: &mut WordApp, filed: Filed, outcome: Result<&[Value], &str>) {
        match (filed, outcome) {
            (Filed::Extra(0), Ok(_)) => {
                let language = language_name(&self.lang);
                app.status(crate::i18n::fmt(tl!("{language} is now the document's default language."), &[("language", &language)]));
            }
            (Filed::Extra(_), Err(e)) => app.status(e.to_owned()),
            _ => {}
        }
    }
}
