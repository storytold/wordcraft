//! Symbol, Date and Time, Field, Columns and Drop Cap dialogs: Insert › Symbol › More Symbols…,
//! Insert › Date & Time, Insert › Quick Parts › Field…, Layout › Columns › More Columns… and
//! Insert › Drop Cap › Drop Cap Options…. Each ends by running one command (`insert.symbol`,
//! `insert.dateTime`, `insert.field`, `layout.columns`, `insert.dropCap`), so scripts and agents get
//! the same result without the dialog.

use craft::behavior::{Mixed, Selection, SelectionChange, keyed};
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing};
use craft::ui::dialog::DialogId;
use craft::ui::form::Field;
use craft::ui::list::{List, ListRow};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::Value;

use crate::WordApp;
use crate::dialogs::host::{Settle, open_form};

mod columns;
mod date_time;
mod field;
pub(crate) mod symbol;

pub use columns::DropCapForm;
pub use date_time::DateTimeForm;
#[cfg(test)]
pub(crate) use date_time::date_time_previews_at;
pub use field::{DATE_FORMATS, FIELD_CATEGORIES, FIELDS, FieldForm, FieldKind, NUMBER_FORMATS, STYLEREF_SWITCHES, field_kind};
pub(crate) use symbol::SymbolDialog;
pub use symbol::{RECENT_SYMBOLS, RecentSymbol, SPECIAL, SUBSETS, SymbolForm, remember_symbol, subset_of};

/// The dialog `ui.dialog` opens by `name`: `dateTime`, `columns`, `symbol`, `specialCharacters`,
/// `field`, `dropCap` (where the paragraph can take one), and the chart, diagram and bullet names
/// whose dialogs live in their own modules. `None` for another name or where a dialog cannot open.
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "dateTime" => {
            let form = DateTimeForm::new(app);
            Some(open_form(app, parent, form))
        }
        "columns" => columns::open(app, parent),
        "symbol" => Some(SymbolDialog::open(app, 0, parent)),
        "specialCharacters" => Some(SymbolDialog::open(app, 1, parent)),
        "field" => {
            let form = FieldForm::new(app);
            Some(open_form(app, parent, form))
        }
        "dropCap" => {
            let form = DropCapForm::read(app)?;
            Some(open_form(app, parent, form))
        }
        // PLAN: assumed `dialogs_chart::open`, `dialogs_smart_art::open` and `dialogs_bullet::open` take
        // `(app, name, params, parent) -> Option<DialogId>` (dialogs-w17, dialogs-w14).
        "insertChart" | "changeChartType" | "chartData" => crate::dialogs_chart::open(app, name, params, parent),
        "insertSmartArt" => crate::dialogs_smart_art::open(app, name, params, parent),
        "defineBullet" => crate::dialogs_bullet::open(app, name, params, parent),
        _ => None,
    }
}

/// A translated label for `text`.
pub(super) fn word(text: &str) -> Label {
    Label::from(tl!(text).to_string())
}

/// What a settle hook returns for the dialog `id`: done once it has closed, else it keeps waiting.
pub(super) fn settled(app: &WordApp, id: DialogId) -> Settle {
    if app.dialog_host.get_dyn(id).is_none() { Settle::Done } else { Settle::Keep }
}

/// A labelled single-line text field.
pub(super) fn text_field(ui: &mut Ui, salt: &'static str, label: &Label, value: &mut String) {
    Field::new(label.clone()).id_salt(salt).show(ui, |ui, _| {
        TextField::new(label.clone(), Bind::bound(value)).show(ui);
    });
}

/// A labelled select over `rows`; the index picked this pass.
pub(super) fn select_index(ui: &mut Ui, label: &Label, rows: &[Label], current: Option<usize>) -> Option<usize> {
    let entries: Vec<Entry<usize>> = rows.iter().enumerate().map(|(at, row)| Entry::new(at, row.clone())).collect();
    let current = current.map_or(Mixed::Mixed, Mixed::Same);
    Field::new(label.clone()).show(ui, |ui, _| Select::new(label.clone(), Bind::given(current), &entries).show(ui).value().copied()).inner.flatten()
}

/// What a single-selection list reported in one pass.
pub(super) struct Picked {
    /// The row selected this pass.
    pub selected: Option<usize>,
    /// The row a double click or Enter activated this pass.
    pub activated: Option<usize>,
}

/// A single-selection list of `rows`, `current` selected.
pub(super) fn list_index(ui: &mut Ui, label: &Label, rows: &[String], current: Option<usize>) -> Picked {
    let positions: Vec<usize> = (0..rows.len()).collect();
    let source = keyed(&positions, |at: &usize| *at);
    let mut selection = current.map_or_else(Selection::default, Selection::single);
    let events = List::new(label.clone())
        .select(SelectMode::Single)
        .sizing(Sizing::Content)
        .natural_height(true)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &source, &mut |at, _| ListRow::new(rows.get(*at).cloned().unwrap_or_default()))
        .events;
    let mut picked = Picked { selected: None, activated: None };
    for event in events {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(set)) => picked.selected = set.last().copied(),
            CollectionEvent::Activate(row) => {
                picked.selected = Some(row);
                picked.activated = Some(row);
            }
            _ => {}
        }
    }
    picked
}

#[cfg(test)]
mod tests;
