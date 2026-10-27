use super::*;

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

pub(super) fn formula_ui(app: &mut WordApp, ui: &mut Ui, f: &mut FormulaForm) -> bool {
    ui.label(tl!("Formula:"));
    ui.add(egui::TextEdit::singleline(&mut f.formula).desired_width(300.0));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Paste function:"));
        let mut picked = None;
        craft_ui::ComboBox::from_id_salt("formula_paste").selected_text("").width(140.0).show_ui(ui, |ui| {
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

pub(super) fn sort_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SortForm) -> bool {
    heading(ui, "Sort by");
    ui.horizontal(|ui| {
        craft_ui::ComboBox::from_id_salt("sort_column").selected_text(f.column_label(f.column)).width(180.0).show_ui(ui, |ui| {
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

pub(super) fn split_cells_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SplitCellsForm) -> bool {
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

pub(super) fn text_to_table_ui(app: &mut WordApp, ui: &mut Ui, f: &mut TextToTableForm) -> bool {
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

pub(super) fn table_to_text_ui(app: &mut WordApp, ui: &mut Ui, f: &mut TableToTextForm) -> bool {
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
