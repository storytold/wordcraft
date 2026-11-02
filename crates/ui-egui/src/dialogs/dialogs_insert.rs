//! Columns, Symbol and Field dialogs (#321): Layout › Columns › More Columns…, Insert › Symbol ›
//! More Symbols… and Insert › Quick Parts › Field…. Each ends by running one command
//! (`layout.columns`, `insert.symbol`, `insert.field`), so scripts and agents get the same result
//! without the dialog. Shown through [`crate::dialogs::Dialog::Insert`].

use egui::{Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::WordApp;
use crate::options::MeasureExt;
use crate::theme::{Tokens, regular, semibold};

mod columns;
mod field;
mod symbol;

pub use columns::{ColumnsForm, DropCapForm};
use columns::{columns_ui, drop_cap_ui};
use field::field_ui;
pub use field::{DATE_FORMATS, FIELD_CATEGORIES, FIELDS, FieldForm, FieldKind, NUMBER_FORMATS, STYLEREF_SWITCHES, field_kind};
use symbol::symbol_ui;
pub use symbol::{RECENT_SYMBOLS, RecentSymbol, SPECIAL, SUBSETS, SymbolForm, remember_symbol, subset_of};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum InsertDialog {
    Columns(ColumnsForm),
    Symbol(SymbolForm),
    Field(FieldForm),
    SmartArt(crate::dialogs_smart_art::SmartArtForm),
    ChartType(crate::dialogs_chart::ChartTypeForm),
    ChartData(crate::dialogs_chart::ChartDataForm),
    DropCap(DropCapForm),
}

impl InsertDialog {
    pub fn name(&self) -> &'static str {
        match self {
            InsertDialog::Columns(_) => "columns",
            InsertDialog::Symbol(_) => "symbol",
            InsertDialog::Field(_) => "field",
            InsertDialog::SmartArt(_) => "insertSmartArt",
            InsertDialog::ChartType(f) if f.change => "changeChartType",
            InsertDialog::ChartType(_) => "insertChart",
            InsertDialog::ChartData(_) => "chartData",
            InsertDialog::DropCap(_) => "dropCap",
        }
    }

    /// The window title (English; the caller translates it).
    pub fn title(&self) -> &'static str {
        match self {
            InsertDialog::Columns(_) => "Columns",
            InsertDialog::Symbol(_) => "Symbol",
            InsertDialog::Field(_) => "Field",
            InsertDialog::SmartArt(_) => "Insert SmartArt",
            InsertDialog::ChartType(f) if f.change => "Change Chart Type",
            InsertDialog::ChartType(_) => "Insert Chart",
            InsertDialog::ChartData(_) => "Edit Data",
            InsertDialog::DropCap(_) => "Drop Cap",
        }
    }
}

/// The dialog `ui.dialog` opens by `name`: `columns`, `symbol`, `specialCharacters`, `field`,
/// `insertChart`, `changeChartType` or `chartData` (the last two for the selected chart), or
/// `insertSmartArt`, or `dropCap` (Drop Cap Options…, where the paragraph can take one).
pub fn open(name: &str, app: &WordApp) -> Option<InsertDialog> {
    Some(match name {
        "columns" => InsertDialog::Columns(ColumnsForm::read(app)),
        "symbol" => InsertDialog::Symbol(SymbolForm::new(app, 0)),
        "specialCharacters" => InsertDialog::Symbol(SymbolForm::new(app, 1)),
        "field" => InsertDialog::Field(FieldForm::new(app)),
        "insertSmartArt" => InsertDialog::SmartArt(crate::dialogs_smart_art::SmartArtForm::new(app)),
        "insertChart" => InsertDialog::ChartType(crate::dialogs_chart::ChartTypeForm::new(app, false)?),
        "changeChartType" => InsertDialog::ChartType(crate::dialogs_chart::ChartTypeForm::new(app, true)?),
        "chartData" => InsertDialog::ChartData(crate::dialogs_chart::ChartDataForm::new(app)?),
        "dropCap" => InsertDialog::DropCap(DropCapForm::read(app)?),
        _ => return None,
    })
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut InsertDialog) -> bool {
    match d {
        InsertDialog::Columns(f) => columns_ui(app, ui, f),
        InsertDialog::Symbol(f) => symbol_ui(app, ui, f),
        InsertDialog::Field(f) => field_ui(app, ui, f),
        InsertDialog::SmartArt(f) => crate::dialogs_smart_art::picker_ui(app, ui, f),
        InsertDialog::ChartType(f) => crate::dialogs_chart::type_ui(app, ui, f),
        InsertDialog::ChartData(f) => crate::dialogs_chart::data_ui(app, ui, f),
        InsertDialog::DropCap(f) => drop_cap_ui(app, ui, f),
    }
}

/// OK / Cancel (or another OK label); returns (ok, cancel). `ok_enabled` greys OK out.
pub(crate) fn buttons(ui: &mut Ui, ok: &str, cancel: &str, ok_enabled: bool) -> (bool, bool) {
    let mut r = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (okb, cancelb) =
                crate::dialogs::ok_cancel_responses(ui, |ui| ui.add_enabled(ok_enabled, crate::widgets::primary_button(ui, tl!(ok))), tl!(cancel));
            if cancelb.clicked() || crate::dialogs::cancel_key_pressed(ui) {
                r.1 = true;
            }
            if okb.clicked() || (ok_enabled && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                r.0 = true;
            }
        });
    });
    r
}

#[cfg(test)]
mod tests;
