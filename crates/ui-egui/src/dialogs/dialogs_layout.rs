//! Layout, Review and Mailings dialogs (#407): Line Numbers, Hyphenation (with Manual…), Language,
//! Envelopes and Labels, Formula, Sort, Split Cells, Convert Text to Table and Convert to Text,
//! Size and Format Page Numbers. Each is one [`WordForm`]; OK runs its command (`layout.lineNumbers`,
//! `layout.hyphenation`, `review.language`, `table.formula`, `table.sort`, `table.split`,
//! `table.fromText`, `table.toText`, `picture.size`, `layout.pageNumberFormat`), so scripts and
//! agents get the same result without the dialog.

use std::ops::RangeInclusive;

use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::dialog::DialogId;
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::text::{NamedOr, NumberField, NumberName, NumberPick};
use egui::Ui;
use serde_json::Value;

use crate::WordApp;
use crate::dialogs::host::{DialogHooks, open_form, open_form_with};

mod page;
pub(crate) mod print;
mod table;
mod text;

pub use page::{PageNumbersForm, SizeForm};
#[cfg(test)]
pub(crate) use print::envelope_label;
#[cfg(test)]
use print::product_label;
pub use print::{EnvelopeForm, LabelsForm};
pub use table::{FORMULA_FUNCTIONS, FormulaForm, SortForm, SplitCellsForm, TableToTextForm, TextToTableForm};
#[cfg(test)]
use text::language_name;
pub use text::{HyphenationForm, LANGUAGES, LanguageForm, LineNumbersForm, ManualForm};

/// Opens the dialog `ui.dialog` names, over `parent` when given; `None` for a name this module does not own or when
/// there is nothing to show (the table and picture dialogs need their selection).
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    Some(match name {
        "lineNumbers" => {
            let form = LineNumbersForm::read(app);
            open_form(app, parent, form)
        }
        "hyphenation" => {
            let form = HyphenationForm::read(app);
            open_form(app, parent, form)
        }
        "manualHyphenation" => {
            let form = ManualForm::start(app);
            open_form(app, parent, form)
        }
        "language" => {
            let form = LanguageForm::read(app);
            open_form(app, parent, form)
        }
        "envelopes" | "labels" => return print::open(app, name, params, parent),
        "tableFormula" => {
            let form = FormulaForm::read(app)?;
            open_form(app, parent, form)
        }
        "tableSort" => {
            let form = SortForm::read(app)?;
            open_form(app, parent, form)
        }
        "splitCells" => {
            let form = SplitCellsForm::read(app)?;
            open_form(app, parent, form)
        }
        "textToTable" => {
            let form = TextToTableForm::read(app)?;
            open_form(app, parent, form)
        }
        "tableToText" => {
            let form = TableToTextForm::read(app)?;
            open_form(app, parent, form)
        }
        "pictureSize" => {
            let form = SizeForm::read(app)?;
            // PLAN: assumed `DialogHooks::prepare_only(prepare: fn(&mut WordApp, DialogId)) -> DialogHooks` (dialogs-w13)
            open_form_with(app, parent, form, DialogHooks::prepare_only(page::prepare_size))
        }
        "pageNumbers" => {
            let form = PageNumbersForm::read(app);
            open_form_with(app, parent, form, DialogHooks::prepare_only(page::prepare_page_numbers))
        }
        _ => return None,
    })
}

/// Lengths kept and shown in points: what a form shows before `read` gives it the user's unit.
pub(crate) fn points() -> LengthUnits {
    LengthUnits::new(UnitKind::Points, UnitKind::Points, 96.0)
}

/// A whole-number field over `value` within `range`; true when the user changed it.
pub(crate) fn count_field(ui: &mut Ui, label: &str, value: &mut u32, range: RangeInclusive<u32>, enabled: bool) -> bool {
    let was = f64::from(*value);
    let mut shown = Some(was);
    NumberField::new(label.to_string(), Bind::bound(&mut shown)).range(f64::from(*range.start())..=f64::from(*range.end())).enabled(enabled).show(ui);
    match shown {
        Some(n) if n != was => {
            *value = (n.round().max(0.0) as u32).clamp(*range.start(), *range.end());
            true
        }
        _ => false,
    }
}

/// The value a number field's events leave, `None` when they carry none.
fn edited(events: &craft::behavior::EditEvents<Option<f64>>) -> Option<f64> {
    events.iter().filter_map(|e| e.value().copied().flatten()).next_back()
}

/// A field over a length in points, shown in the user's unit; true when the user changed it.
pub(crate) fn length_field(ui: &mut Ui, label: &str, value: &mut f32, lengths: LengthUnits, range: RangeInclusive<f64>, enabled: bool) -> bool {
    let was = f64::from(*value);
    // The form takes the field's edit events, not a write into a copy that lives one pass: an edit that commits as the
    // dialog files its OK (Enter, or a click on OK) reaches the form in the pass that files it.
    let out = NumberField::length(label.to_string(), Bind::given(Some(was)), lengths).range(range).enabled(enabled).show(ui);
    match edited(&out.edit.events) {
        Some(n) if n != was => {
            *value = n as f32;
            true
        }
        _ => false,
    }
}

/// A percentage field over `value`; true when the user changed it.
pub(crate) fn percent_field(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f64>) -> bool {
    let was = f64::from(*value);
    let out = NumberField::new(label.to_string(), Bind::given(Some(was))).range(range).unit("%").show(ui);
    match edited(&out.edit.events) {
        Some(n) if n != was => {
            *value = n as f32;
            true
        }
        _ => false,
    }
}

/// A field that holds a number or one named entry (Auto, No limit) chosen from its presets list. `lengths` makes the
/// number a length.
pub(crate) fn named_field<K: NumberName + 'static>(
    ui: &mut Ui,
    label: &str,
    value: &mut NamedOr<K>,
    name: (K, &str),
    lengths: Option<LengthUnits>,
    range: RangeInclusive<f64>,
    enabled: bool,
) {
    let presets = vec![Entry::new(NumberPick::Choice(name.0.id()), name.1.to_string()).checked(matches!(*value, NamedOr::Named(_)))];
    let mut field = NumberField::named(label.to_string(), Bind::bound(value)).range(range).presets(&presets).enabled(enabled);
    if let Some(lengths) = lengths {
        field = field.lengths(lengths);
    }
    field.show(ui);
}

/// The number a named field holds, 0 while it holds its name.
pub(crate) fn number_or_zero<K>(value: &NamedOr<K>) -> f64 {
    match value {
        NamedOr::Number(n) => *n,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests;
