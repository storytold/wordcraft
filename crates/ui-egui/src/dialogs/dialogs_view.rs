//! Word Count and Zoom.

use egui::{Context, Ui};
use serde_json::{Value, json};

use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::{Field, FieldGroup, SliderField};
use craft::ui::layout_components::Stack;
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
            let percent = f64::from(app.session.view.zoom);
            Some(open_with(app, parent, ZoomDialog { percent }, DialogHooks { prepare: None, settle: settle_zoom }))
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
            .buttons([FooterButton::new(tl!("Close").to_string(), DialogRole::Cancel)])
            .show(ui, |ui, _| {
                FieldGroup::new(tl!("Counts").to_string()).show(ui, |ui, _| {
                    for (label, key) in ROWS {
                        let text = stats.get(key).map(Value::to_string).unwrap_or_default();
                        Field::new(tl!(label).to_string()).show(ui, |ui, _| Text::new(Label::from(text.clone())).show(ui));
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

/// The fit modes and their labels: each is a footer button that answers with the mode's name.
const FITS: [(&str, &str); 3] = [("pageWidth", "Page width"), ("onePage", "Whole page"), ("multiplePages", "Many pages")];

/// The Zoom dialog: 200%, 100% or 75% radios, a log slider 10..=500 and the fit buttons. It answers with a percent
/// or a fit name, as `view.zoom` takes them.
pub(crate) struct ZoomDialog {
    percent: f64,
}

impl StackDialog for ZoomDialog {
    type Result = Value;

    fn id(&self) -> DialogId {
        dialog_id("zoom")
    }

    fn key(&self) -> Option<&str> {
        Some("zoom")
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<Value>> {
        let fits = FITS.iter().map(|(_, label)| FooterButton::new(tl!(label).to_string(), DialogRole::Other));
        let out = FormDialog::new(tl!("Zoom").to_string(), Bind::bound(&mut self.percent)).extra_buttons(fits).show(ui, |ui, _, percent| {
            zoom_body(ui, percent);
            true
        });
        // The extra buttons follow OK and Cancel in the picked index.
        if let Some((name, _)) = out.picked.and_then(|at| at.checked_sub(2)).and_then(|at| FITS.get(at)) {
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
        Stack::new().show(ui, |ui, _| {
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
    SliderField::new(Label::from(tl!("Percent").to_string()), Bind::bound(percent), 10.0..=500.0)
        .scale(SliderScale::Log)
        .step(1.0)
        .unit_kind(UnitKind::Percent)
        .show(ui);
}
