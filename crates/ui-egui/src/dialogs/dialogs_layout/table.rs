use craft::theme::variant::Tone;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::Stack;
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Label, LengthUnits};
use craft::ui::sections::Section;
use craft::ui::text::{NamedOr, NumberName, TextField};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{WordApp, count_field, named_field, number_or_zero, points};
use crate::dialogs::host::{Asks, WordForm};

/// The field's one name: a width that shares the text width out.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Auto {
    Auto,
}

impl NumberName for Auto {
    fn id(self) -> u32 {
        0
    }

    fn from_id(id: u32) -> Option<Self> {
        (id == 0).then_some(Auto::Auto)
    }
}

// ---------------------------------------------------------------------------------------------
// Formula

/// The functions `table.formula` works out, offered by Paste function.
pub const FORMULA_FUNCTIONS: [&str; 6] = ["AVERAGE", "COUNT", "MAX", "MIN", "PRODUCT", "SUM"];

/// The table Formula dialog's field: the formula, suggested as Word does from the numbers next
/// to the caret's cell (`=SUM(ABOVE)`, else `=SUM(LEFT)`, else a bare `=`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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

impl WordForm for FormulaForm {
    fn key(&self) -> &'static str {
        "tableFormula"
    }

    fn title(&self) -> Label {
        tl!("Formula").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Field::new(tl!("Formula:").to_string()).id_salt("formula").show(ui, |ui, _| {
            TextField::new(tl!("Formula:").to_string(), Bind::bound(&mut self.formula)).show(ui);
        });
        let functions: Vec<Entry<String>> = FORMULA_FUNCTIONS.iter().map(|n| Entry::new((*n).to_string(), (*n).to_string())).collect();
        let mut picked = String::new();
        Select::new(tl!("Paste function:").to_string(), Bind::bound(&mut picked), &functions).show(ui);
        if !picked.is_empty() {
            self.paste(&picked);
        }
        self.formula.trim().starts_with('=') && self.formula.trim().len() > 1
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("table.formula".to_string(), self.params())]
    }
}

// ---------------------------------------------------------------------------------------------
// Sort

/// The table Sort dialog's fields: the column to sort by, the order and whether the first row is
/// a header (kept in place). `headers` is the first row's text, which names the columns when it
/// is a header.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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

impl WordForm for SortForm {
    fn key(&self) -> &'static str {
        "tableSort"
    }

    fn title(&self) -> Label {
        tl!("Sort").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Section::new(tl!("Sort by").to_string()).show(ui, |ui, _| {
            let columns: Vec<Entry<String>> = (0..self.headers.len()).map(|i| Entry::new(i.to_string(), self.column_label(i))).collect();
            let mut column = self.column.to_string();
            Select::new(tl!("Sort by").to_string(), Bind::bound(&mut column), &columns).show(ui);
            self.column = column.parse().unwrap_or(self.column);
            let mut order = if self.descending { "descending" } else { "ascending" }.to_string();
            let orders = [("ascending", tl!("Ascending")), ("descending", tl!("Descending"))];
            RadioGroup::new(tl!("Order").to_string(), Bind::bound(&mut order)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, cx| {
                    for (at, (v, l)) in orders.iter().enumerate() {
                        cx.labelled(ui, at, Label::from(*l), SideLabel::new().layout(FieldLayout::After), Radio::new((*v).to_string()));
                    }
                });
            });
            self.descending = order == "descending";
        });
        Section::new(tl!("My list has").to_string()).show(ui, |ui, _| {
            let mut header = if self.header { "header" } else { "none" }.to_string();
            let headers = [("header", tl!("Header row")), ("none", tl!("No header row"))];
            RadioGroup::new(tl!("My list has").to_string(), Bind::bound(&mut header)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, cx| {
                    for (at, (v, l)) in headers.iter().enumerate() {
                        cx.labelled(ui, at, Label::from(*l), SideLabel::new().layout(FieldLayout::After), Radio::new((*v).to_string()));
                    }
                });
            });
            self.header = header == "header";
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("table.sort".to_string(), self.params())]
    }
}

// ---------------------------------------------------------------------------------------------
// Split Cells

/// The Split Cells dialog's fields. As in Word, the defaults follow the selection: twice its
/// columns and its rows. Merge cells before split is offered, and on, when several cells (or one
/// cell merged across columns) are selected (`several`). A merged selection splits into at most
/// `max_rows` rows, a vertically merged cell alone into at most its `height`; more gets Word's
/// message ([`SplitCellsForm::check`]). Without merging each cell splits into columns on its own
/// and the row count stays 1.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
    /// Word's message for a row count out of range, shown while the count stays out of range.
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

impl WordForm for SplitCellsForm {
    fn key(&self) -> &'static str {
        "splitCells"
    }

    fn title(&self) -> Label {
        tl!("Split Cells").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        // Several cells split one by one keep their rows.
        let rows_on = self.cells <= 1 || self.merge_first;
        count_field(ui, tl!("Number of columns:"), &mut self.columns, 1..=63, true);
        count_field(ui, tl!("Number of rows:"), &mut self.rows, 1..=63, rows_on);
        let mut merge = self.merge_first;
        Checkbox::new(Bind::bound(&mut merge)).label(tl!("Merge cells before split").to_string()).enabled(self.several).show(ui);
        if merge != self.merge_first {
            self.set_merge(merge);
        }
        self.error = self.check();
        if let Some(e) = &self.error {
            Notice::new(Tone::Bad, e.clone()).show(ui);
        }
        self.error.is_none()
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("table.split".to_string(), self.params())]
    }
}

// ---------------------------------------------------------------------------------------------
// Convert Text to Table

/// The Convert Text to Table dialog's fields: the selected paragraphs (`lines`), where to
/// separate them (`paragraph`, `comma`, `tab` or `other` with `other_char`), the column count
/// (guessed from the text for the separator, as Word does) and the AutoFit behaviour: an initial
/// column width (`width` in points, 0 = Auto, up to 22 in), or AutoFit to contents or to the
/// window. `rows` is the resulting row count, shown read-only.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextToTableForm {
    #[serde(skip)]
    pub lines: Vec<String>,
    pub columns: u32,
    pub rows: usize,
    pub autofit: String,
    /// Initial column width in points: 0 (Auto) shares the text width out.
    pub width: f32,
    pub separator: String,
    pub other_char: String,
    /// The user's length unit, which only the app knows; read with the form.
    #[serde(skip)]
    lengths: Option<LengthUnits>,
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
            width: 0.0,
            separator: separator.into(),
            other_char: "-".into(),
            lengths: Some(crate::options::lengths(app.session.units())),
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

    /// `table.fromText` parameters for OK.
    pub fn params(&self) -> Value {
        let mut v = json!({"separator": self.separator, "columns": self.columns, "autofit": self.autofit});
        if self.separator == "other" {
            v["char"] = json!(self.other_char);
        }
        if self.autofit == "initial" && self.width > 0.0 {
            v["width"] = json!(self.width);
        }
        v
    }
}

impl WordForm for TextToTableForm {
    fn key(&self) -> &'static str {
        "textToTable"
    }

    fn title(&self) -> Label {
        tl!("Convert Text to Table").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let lengths = self.lengths.unwrap_or_else(points);
        let before = (self.separator.clone(), self.other_char.clone(), self.columns);
        Section::new(tl!("Table size").to_string()).show(ui, |ui, _| {
            count_field(ui, tl!("Number of columns:"), &mut self.columns, 1..=63, true);
            Text::new(format!("{} {}", tl!("Number of rows:"), self.rows)).show(ui);
        });
        Section::new(tl!("AutoFit behavior").to_string()).show(ui, |ui, _| {
            let fits = [("initial", tl!("Initial column width:")), ("contents", tl!("AutoFit to contents")), ("window", tl!("AutoFit to window"))];
            RadioGroup::new(tl!("AutoFit behavior").to_string(), Bind::bound(&mut self.autofit)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, cx| {
                    for (at, (v, l)) in fits.iter().enumerate() {
                        cx.labelled(ui, at, Label::from(*l), SideLabel::new().layout(FieldLayout::After), Radio::new((*v).to_string()));
                    }
                });
            });
            let mut width = if self.width <= 0.0 { NamedOr::Named(Auto::Auto) } else { NamedOr::Number(f64::from(self.width)) };
            named_field(
                ui,
                tl!("Initial column width:"),
                &mut width,
                (Auto::Auto, tl!("Auto")),
                Some(lengths),
                0.0..=22.0 * 72.0,
                self.autofit == "initial",
            );
            self.width = number_or_zero(&width).clamp(0.0, 22.0 * 72.0) as f32;
        });
        Section::new(tl!("Separate text at").to_string()).show(ui, |ui, _| {
            let separators = [("paragraph", tl!("Paragraphs")), ("comma", tl!("Commas")), ("tab", tl!("Tabs")), ("other", tl!("Other:"))];
            RadioGroup::new(tl!("Separate text at").to_string(), Bind::bound(&mut self.separator)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, cx| {
                    for (at, (v, l)) in separators.iter().enumerate() {
                        cx.labelled(ui, at, Label::from(*l), SideLabel::new().layout(FieldLayout::After), Radio::new((*v).to_string()));
                    }
                });
            });
            let other = self.other_char.clone();
            Field::new(tl!("Other:").to_string()).id_salt("other_char").show(ui, |ui, _| {
                TextField::new(tl!("Other:").to_string(), Bind::bound(&mut self.other_char)).show(ui);
            });
            // One character only, and typing in the box selects "Other".
            self.other_char = self.other_char.chars().next().map(String::from).unwrap_or_default();
            if self.other_char != other {
                self.separator = "other".into();
            }
        });
        if (self.separator.as_str(), self.other_char.as_str()) != (before.0.as_str(), before.1.as_str()) {
            self.guess_columns();
        } else if self.columns != before.2 {
            self.count_rows();
        }
        self.separator != "other" || !self.other_char.is_empty()
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("table.fromText".to_string(), self.params())]
    }
}

// ---------------------------------------------------------------------------------------------
// Convert to Text

/// The Convert to Text dialog (Table Layout › Data): what sets the cells of a row apart in the
/// text (`paragraph`, `tab`, `comma` or `other` with `other_char`). It opens on Tabs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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

impl WordForm for TableToTextForm {
    fn key(&self) -> &'static str {
        "tableToText"
    }

    fn title(&self) -> Label {
        tl!("Convert to Text").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Section::new(tl!("Separate text with").to_string()).show(ui, |ui, _| {
            let separators = [("paragraph", tl!("Paragraph marks")), ("tab", tl!("Tabs")), ("comma", tl!("Commas")), ("other", tl!("Other:"))];
            RadioGroup::new(tl!("Separate text with").to_string(), Bind::bound(&mut self.separator)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, cx| {
                    for (at, (v, l)) in separators.iter().enumerate() {
                        cx.labelled(ui, at, Label::from(*l), SideLabel::new().layout(FieldLayout::After), Radio::new((*v).to_string()));
                    }
                });
            });
            let other = self.other_char.clone();
            Field::new(tl!("Other:").to_string()).id_salt("other_char").show(ui, |ui, _| {
                TextField::new(tl!("Other:").to_string(), Bind::bound(&mut self.other_char)).show(ui);
            });
            self.other_char = self.other_char.chars().next().map(String::from).unwrap_or_default();
            if self.other_char != other {
                self.separator = "other".into();
            }
        });
        self.separator != "other" || !self.other_char.is_empty()
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("table.toText".to_string(), self.params())]
    }
}
