//! Word Count and Zoom.

use egui::{Context, Ui};
use serde_json::{Value, json};

use craft::behavior::ColumnWidth;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::{SliderField, SliderFieldLayout};
use craft::ui::layout_components::{Grid, Row};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::value_drag::SliderScale;

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

/// Opens `wordCount` or `zoom`; any other name opens nothing.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "wordCount" => Some(open_with(app, parent, WordCountDialog::new(), DialogHooks::prepare_only(prepare_word_count))),
        "zoom" => {
            // The view keeps the zoom as a fraction (1.0 = 100%); the dialog shows and answers percent, as `view.zoom` takes it.
            let percent = (f64::from(app.session.view.zoom) * 100.0).round();
            Some(open_with(app, parent, ZoomDialog { percent, fit: None }, DialogHooks { prepare: None, settle: settle_zoom }))
        }
        _ => None,
    }
}

/// Reads the six counts from the session before each pass; the dialog never keeps them beyond the pass.
fn prepare_word_count(app: &mut WordApp, id: DialogId) {
    let include = app.dialog_host.get_by_id::<WordCountDialog>(id).is_none_or(|d| d.include_text_boxes);
    let stats = app.run("review.wordCount", json!({"includeTextBoxes": include})).ok();
    if let Some(d) = app.dialog_host.get_mut_by_id::<WordCountDialog>(id) {
        d.stats = stats;
    }
}

/// Runs `view.zoom` with the percent or the fit name the dialog answered with.
fn settle_zoom(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    match app.dialog_host.take_by_id::<ZoomDialog>(id) {
        Some(DialogResult::Ok(value)) => {
            let _ = app.run("view.zoom", json!({"value": value}));
            Settle::Done
        }
        Some(_) => Settle::Done,
        None => Settle::Keep,
    }
}

/// The rows of the Word Count dialog: the label and the key of the count in `review.wordCount`'s result.
const ROWS: [(&str, &str); 6] = [
    ("Pages", "pages"),
    ("Words", "words"),
    ("Characters (no spaces)", "characters"),
    ("Characters (with spaces)", "charactersWithSpaces"),
    ("Paragraphs", "paragraphs"),
    ("Lines", "lines"),
];

/// The Word Count table's columns: the labels, then the counts.
const COUNT_COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content; 2];

/// The Word Count dialog.
pub(crate) struct WordCountDialog {
    /// The counts of the last `prepare`.
    stats: Option<Value>,
    /// Whether text boxes and notes count.
    include_text_boxes: bool,
}

impl WordCountDialog {
    fn new() -> Self {
        Self { stats: None, include_text_boxes: true }
    }
}

impl StackDialog for WordCountDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("wordCount")
    }

    fn key(&self) -> Option<&str> {
        Some("wordCount")
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let stats = self.stats.clone().unwrap_or(Value::Null);
        let include = &mut self.include_text_boxes;
        let shown = Dialog::new(tl!("Word Count").to_string())
            // The old dialog's pair: Close as the default button, and Cancel; both only close it.
            .buttons([
                FooterButton::new(tl!("Close").to_string(), DialogRole::Accept),
                FooterButton::new(tl!("Cancel").to_string(), DialogRole::Cancel),
            ])
            .show(ui, |ui, _| {
                // A plain two-column table, the label and its count, both at the start, as the old dialog showed them:
                // its rows a control's height apart and the counts well clear of the labels.
                Grid::new(&COUNT_COLUMNS).gap(Gap::Section).show(ui, |ui, _| {
                    for (label, key) in ROWS {
                        let text = stats.get(key).map(Value::to_string).unwrap_or_default();
                        Text::new(Label::from(tl!(label).to_string())).show(ui);
                        Text::new(Label::from(text)).show(ui);
                    }
                });
                let toggled = Checkbox::new(Bind::given(*include))
                    .label(Label::from(tl!("Count words in text boxes and notes").to_string()))
                    .id_salt("includeTextBoxes")
                    .show(ui);
                if let Some(on) = toggled.value().cloned() {
                    *include = on;
                }
            });
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// The zoom percentages the radio group offers.
const PRESETS: [f64; 3] = [200.0, 100.0, 75.0];

/// The fit modes and their labels: each is a button under the slider that answers with the mode's name.
const FITS: [(&str, &str); 3] = [("pageWidth", "Page width"), ("onePage", "Whole page"), ("multiplePages", "Many pages")];

/// The Zoom dialog: a row of 200%, 100% and 75% radios, a log slider 10..=500 with its percent field, and a row of
/// fit buttons over OK and Cancel. It answers with a percent or a fit name, as `view.zoom` takes them.
pub(crate) struct ZoomDialog {
    percent: f64,
    /// The fit button pressed in this pass: it answers in place of the percent.
    fit: Option<&'static str>,
}

#[cfg(test)]
impl ZoomDialog {
    /// The percent the dialog shows.
    pub(super) fn percent(&self) -> f64 {
        self.percent
    }
}

impl StackDialog for ZoomDialog {
    type Result = Value;

    fn id(&self) -> DialogId {
        dialog_id("zoom")
    }

    fn key(&self) -> Option<&str> {
        Some("zoom")
    }

    /// OK answers with the percent the dialog shows, as its OK button does; it holds nothing.
    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Value> {
        Some(json!(self.percent))
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<Value>> {
        let fit = &mut self.fit;
        let out = FormDialog::new(tl!("Zoom").to_string(), Bind::bound(&mut self.percent)).show(ui, |ui, cx, percent| {
            zoom_body(ui, percent);
            // A fit button closes the dialog as OK does, answering with its mode.
            if let Some(name) = fit_buttons(ui) {
                *fit = Some(name);
                cx.accept();
            }
            true
        });
        if out.closed.is_some()
            && let Some(name) = self.fit.take()
        {
            return Some(DialogResult::Ok(json!(name)));
        }
        match (out.accepted, out.closed) {
            (Some(percent), _) => Some(DialogResult::Ok(json!(percent))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// The radios and the slider over one percent; no radio is picked when the percent matches none.
fn zoom_body(ui: &mut Ui, percent: &mut f64) {
    let picked = PRESETS.iter().find(|p| (**p - *percent).abs() < 1e-6).map(|p| format!("{p}")).unwrap_or_default();
    let chosen = RadioGroup::new(Label::from(tl!("Zoom to").to_string()), Bind::given(picked)).id_salt("presets").show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            for preset in PRESETS {
                Radio::new(format!("{preset}")).label(Label::from(format!("{preset}%"))).show(ui);
            }
        });
    });
    if let Some(next) = chosen.output.value().cloned()
        && let Ok(value) = next.parse::<f64>()
    {
        *percent = value;
    }
    // The slider and its percent field on one line, with no caption shown, as the old dialog had them.
    SliderField::new(Label::from(tl!("Percent").to_string()), Bind::bound(percent), 10.0..=500.0)
        .layout(SliderFieldLayout::Inline)
        .scale(SliderScale::Log)
        .step(1.0)
        .unit_kind(UnitKind::Percent)
        .show(ui);
}

/// Page width, Whole page and Many pages in one row; the mode of the one pressed in this pass.
fn fit_buttons(ui: &mut Ui) -> Option<&'static str> {
    let mut picked = None;
    Row::new().show(ui, |ui, _| {
        for (name, label) in FITS {
            if Button::new(tl!(label).to_string()).show(ui).clicked() {
                picked = Some(name);
            }
        }
    });
    picked
}
