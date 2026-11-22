//! Insert Chart / Change Chart Type (a gallery of chart types with previews drawn in code) and
//! Edit Data (the chart's categories and series in an editable grid). Each ends by running one
//! command (`insert.chart`, `chart.type`, `chart.editData`), so agents get the same result
//! without the dialog. Opened by name through [`open`].

use std::cell::RefCell;

use craft::behavior::{ColumnKey, ColumnWidth, IconId, Selection, SelectionChange, keyed};
use craft::table::ui::{ChartPrimitive, paint_chart};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnEvent, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::list::{ListRow, ThumbAspect, ThumbSize};
// PLAN: assumed `FormCx::accept` (dialogs-w5, S-L285)
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::primitives::DataWidth;
use craft::ui::runtime::env::Label;
use craft::ui::style::DataColor;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use craft::ui::tile_grid::{Tile, TileGrid};
use egui::{Color32, Ui, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::chart::{ChartSpec, ChartType, MAX_POINTS, MAX_SERIES};

use crate::WordApp;
use crate::dialogs::host::{Asks, WordForm, open_form};

/// Insert › Chart (`change` false) or Chart Design › Change Chart Type (`change` true).
// PLAN: assumed `ChartType: Deserialize` (wordcraft-doc)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartTypeForm {
    #[serde(rename = "type")]
    pub kind: ChartType,
    pub change: bool,
    /// The theme's first accents, for the previews; filled from the app by `prepare` each pass.
    #[serde(skip)]
    colors: [Color32; 3],
}

impl ChartTypeForm {
    pub fn new(app: &WordApp, change: bool) -> Option<ChartTypeForm> {
        let current = wordcraft_engine::cmd::chart::selected_chart(&app.session);
        if change && current.is_none() {
            return None;
        }
        Some(ChartTypeForm { kind: current.map(|c| c.kind).unwrap_or_default(), change, colors: accents(app) })
    }
}

impl WordForm for ChartTypeForm {
    fn key(&self) -> &'static str {
        if self.change { "changeChartType" } else { "insertChart" }
    }

    fn title(&self) -> Label {
        Label::from(if self.change { tl!("Change Chart Type").to_string() } else { tl!("Insert Chart").to_string() })
    }

    fn prepare(&mut self, app: &WordApp) {
        self.colors = accents(app);
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        // A double click or Enter on a type is the OK press.
        if chart_grid(ui, self.colors, self) {
            cx.accept();
        }
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let command = if self.change { "chart.type" } else { "insert.chart" };
        vec![(command.to_string(), json!({"type": self.kind.id()}))]
    }
}

/// Chart Design › Edit Data: the grid as text, a row per category and a column per series.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartDataForm {
    pub categories: Vec<String>,
    pub names: Vec<String>,
    /// `cells[row][series]`.
    pub cells: Vec<Vec<String>>,
}

impl ChartDataForm {
    pub fn new(app: &WordApp) -> Option<ChartDataForm> {
        let c = wordcraft_engine::cmd::chart::selected_chart(&app.session)?;
        Some(Self::of(&c))
    }

    pub fn of(c: &ChartSpec) -> ChartDataForm {
        let cells = (0..c.points())
            .map(|i| c.series.iter().map(|s| s.values.get(i).copied().flatten().map(|v| format!("{v}")).unwrap_or_default()).collect())
            .collect();
        ChartDataForm { categories: c.categories.clone(), names: c.series.iter().map(|s| s.name.clone()).collect(), cells }
    }

    /// The `chart.editData` params, or the first cell that isn't a number.
    pub fn params(&self) -> Result<Value, String> {
        let mut series = Vec::new();
        for (j, name) in self.names.iter().enumerate() {
            let mut values = Vec::new();
            for (i, row) in self.cells.iter().enumerate() {
                let t = row.get(j).map(|t| t.trim()).unwrap_or("");
                if t.is_empty() {
                    values.push(Value::Null);
                    continue;
                }
                // A decimal comma is read as a point.
                match t.parse::<f64>().or_else(|_| t.replace(',', ".").parse::<f64>()).ok().filter(|v| v.is_finite()) {
                    Some(v) => values.push(json!(v)),
                    None => {
                        let at = self.categories.get(i).filter(|c| !c.is_empty()).cloned().unwrap_or_else(|| (i + 1).to_string());
                        return Err(crate::i18n::fmt(
                            tl!("“{text}” ({row}, {series}) isn't a number."),
                            &[("text", t), ("row", &at), ("series", name)],
                        ));
                    }
                }
            }
            series.push(json!({"name": name, "values": values}));
        }
        Ok(json!({"data": {"categories": self.categories, "series": series}}))
    }

    // Default names become document text (saved and exported), so no isolate marks.
    fn add_row(&mut self) {
        if self.categories.len() < MAX_POINTS {
            self.categories.push(crate::i18n::fmt_plain(tl!("Category {n}"), &[("n", &(self.categories.len() + 1).to_string())]));
            self.cells.push(vec![String::new(); self.names.len()]);
        }
    }

    fn add_series(&mut self) {
        if self.names.len() < MAX_SERIES {
            self.names.push(crate::i18n::fmt_plain(tl!("Series {n}"), &[("n", &(self.names.len() + 1).to_string())]));
            for r in &mut self.cells {
                r.push(String::new());
            }
        }
    }

    /// Drops series `j`; a chart keeps at least one series.
    fn remove_series(&mut self, j: usize) {
        if self.names.len() < 2 || j >= self.names.len() {
            return;
        }
        self.names.remove(j);
        for r in &mut self.cells {
            if j < r.len() {
                r.remove(j);
            }
        }
    }

    /// Drops row `i`; a chart keeps at least one category.
    fn remove_row(&mut self, i: usize) {
        if self.categories.len() < 2 || i >= self.categories.len() {
            return;
        }
        self.categories.remove(i);
        if i < self.cells.len() {
            self.cells.remove(i);
        }
    }
}

/// The theme's first accents, for previews.
fn accents(app: &WordApp) -> [Color32; 3] {
    let t = &app.session.doc.settings.theme_colors;
    let at = |i: usize, d: Color32| t.get(i).map(|c| Color32::from_rgb(c.0, c.1, c.2)).unwrap_or(d);
    [at(4, Color32::from_rgb(0x15, 0x60, 0x82)), at(5, Color32::from_rgb(0xE9, 0x71, 0x32)), at(6, Color32::from_rgb(0x19, 0x6B, 0x24))]
}

/// The primitives of a small picture of a chart type in a `size` box, in the accents `col` with axis lines in `line`.
pub fn chart_primitives(kind: ChartType, col: [Color32; 3], line: Color32, size: Vec2) -> Vec<ChartPrimitive> {
    let inset = size.x.min(size.y) * 0.12;
    let (left, top) = (inset, inset);
    let (w, h) = ((size.x - 2.0 * inset).max(0.0), (size.y - 2.0 * inset).max(0.0));
    let at = |x: f32, y: f32| [left + x * w, top + y * h];
    let data_color = |c: Color32| DataColor::new(c);
    let bar = |x0: f32, y0: f32, x1: f32, y1: f32, c: Color32| {
        let [ax, ay] = at(x0, y0);
        let [bx, by] = at(x1, y1);
        ChartPrimitive::rect(ax, ay, bx - ax, by - ay).filled(data_color(c))
    };
    let axis = |from: [f32; 2], to: [f32; 2]| ChartPrimitive::line(vec![from, to], data_color(line), DataWidth::points(1.0));
    let data: [[f32; 4]; 3] = [[0.45, 0.6, 0.5, 0.8], [0.3, 0.4, 0.65, 0.5], [0.2, 0.3, 0.35, 0.6]];
    let mut out = Vec::new();
    match kind {
        ChartType::Column | ChartType::ColumnStacked | ChartType::Bar | ChartType::BarStacked => {
            let stacked = matches!(kind, ChartType::ColumnStacked | ChartType::BarStacked);
            let horizontal = matches!(kind, ChartType::Bar | ChartType::BarStacked);
            for i in 0..3 {
                let band = (i as f32) / 3.0;
                let mut base = 0.0;
                for (j, c) in col.iter().enumerate() {
                    let v = data.get(j).and_then(|d| d.get(i)).copied().unwrap_or(0.5) * if stacked { 0.4 } else { 1.0 };
                    let (a0, a1) =
                        if stacked { (band + 0.06, band + 0.27) } else { (band + 0.04 + j as f32 * 0.085, band + 0.04 + (j + 1) as f32 * 0.085) };
                    if horizontal {
                        out.push(bar(base, 1.0 - a1, base + v, 1.0 - a0, *c));
                    } else {
                        out.push(bar(a0, 1.0 - base - v, a1, 1.0 - base, *c));
                    }
                    if stacked {
                        base += v;
                    }
                }
            }
            out.push(if horizontal { axis(at(0.0, 0.0), at(0.0, 1.0)) } else { axis(at(0.0, 1.0), at(1.0, 1.0)) });
        }
        ChartType::Line | ChartType::Area => {
            for (j, c) in col.iter().enumerate().take(2) {
                let Some(d) = data.get(j) else { continue };
                let pts: Vec<[f32; 2]> = d.iter().enumerate().map(|(i, v)| at(i as f32 / 3.0, 1.0 - v)).collect();
                if kind == ChartType::Area {
                    // Convex strips between neighbouring points.
                    let floor = top + h;
                    for pair in pts.windows(2) {
                        if let [a, b] = pair {
                            out.push(ChartPrimitive::polygon(vec![*a, *b, [b[0], floor], [a[0], floor]], data_color(*c)));
                        }
                    }
                } else {
                    out.push(ChartPrimitive::line(pts, data_color(*c), DataWidth::points(2.0)));
                }
            }
            out.push(axis(at(0.0, 1.0), at(1.0, 1.0)));
        }
        ChartType::Pie | ChartType::Doughnut => {
            let center = at(0.5, 0.5);
            let rad = w.min(h) / 2.0;
            let inner = if kind == ChartType::Doughnut { rad * 0.5 } else { 0.0 };
            let shares = [0.42, 0.33, 0.25];
            // Angles count clockwise from 12 o'clock.
            let mut a = 0.0;
            for (k, share) in shares.iter().enumerate() {
                let b = a + share * std::f32::consts::TAU;
                out.push(ChartPrimitive::wedge(center, (inner, rad), (a, b), data_color(col.get(k).copied().unwrap_or(line))));
                a = b;
            }
        }
        ChartType::Scatter => {
            for (j, c) in col.iter().enumerate().take(2) {
                for i in 0..5 {
                    let x = 0.08 + i as f32 * 0.2;
                    let y = 0.85 - (i as f32 * 0.15) - j as f32 * 0.2 + if i % 2 == 0 { 0.05 } else { -0.05 };
                    out.push(ChartPrimitive::circle(at(x, y.clamp(0.05, 0.95)), 2.5).filled(data_color(*c)));
                }
            }
            out.push(axis(at(0.0, 1.0), at(1.0, 1.0)));
            out.push(axis(at(0.0, 0.0), at(0.0, 1.0)));
        }
    }
    out
}

/// The chart types as tiles, one picked; true when a type was activated (double click or Enter).
fn chart_grid(ui: &mut Ui, colors: [Color32; 3], form: &mut ChartTypeForm) -> bool {
    let kinds: Vec<ChartType> = ChartType::ALL.to_vec();
    let positions: Vec<usize> = (0..kinds.len()).collect();
    let current = kinds.iter().position(|k| *k == form.kind).unwrap_or(0);
    let source = keyed(&positions, |i| *i);
    let out = TileGrid::<usize>::new(tl!("Chart types").to_string())
        .thumb_size(ThumbSize::Large)
        .zoom(1.6)
        .aspect(19.0 / 15.0)
        .select(SelectMode::Single)
        .card_columns(3)
        // Every type shows, as in production, so the dialog's body is the one scroll.
        .natural_height(true)
        .selection(Bind::given(Selection::single(current)))
        .show_items(ui, &source, &mut |key, _| {
            let kind = kinds.get(*key).copied().unwrap_or_default();
            Tile::new(tl!(kind.label()).to_string()).thumb(ThumbAspect::new(19, 15), move |cx, rect| {
                // PLAN: `ContentPainterCx` has no muted-ink accessor; the plain ink until it gains one
                let primitives = chart_primitives(kind, colors, cx.ink(), rect.size());
                cx.with_draw(|draw| paint_chart(draw, rect.min, &primitives, 1.0));
            })
        });
    let mut activated = false;
    for event in out.response.events {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some(kind) = keys.last().and_then(|k| kinds.get(*k)) {
                    form.kind = *kind;
                }
            }
            CollectionEvent::Selection(SelectionChange::Toggle(key)) => {
                if let Some(kind) = kinds.get(key) {
                    form.kind = *kind;
                }
            }
            CollectionEvent::Activate(key) => {
                if let Some(kind) = kinds.get(key) {
                    form.kind = *kind;
                }
                activated = true;
            }
            _ => {}
        }
    }
    activated
}

/// The key of the category column; series `j` is column `j + 1` and the row remove control the column after the series.
const CATEGORY: ColumnKey = ColumnKey::index(0);

fn column_at(at: usize) -> ColumnKey {
    ColumnKey::index(u32::try_from(at).unwrap_or(u32::MAX))
}

/// A change the grid reports in one pass.
enum Edit {
    Category(usize, String),
    Cell(usize, usize, String),
    Name(usize, String),
    RemoveRow(usize),
}

impl WordForm for ChartDataForm {
    fn key(&self) -> &'static str {
        "chartData"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Edit Data").to_string())
    }

    /// OK is enabled while every cell is a number; the cell that is not is named under the grid.
    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        data_body(ui, self);
        match self.params() {
            Ok(_) => true,
            Err(message) => {
                Text::new(message).show(ui);
                false
            }
        }
    }

    fn commands(&self) -> Vec<(String, Value)> {
        self.params().map(|params| vec![("chart.editData".to_string(), params)]).unwrap_or_default()
    }
}

/// The data rows the Edit Data grid shows before it scrolls.
const DATA_ROWS_SHOWN: u16 = 11;

/// The grid of categories and series, with Add Row and Add Series under it.
fn data_body(ui: &mut Ui, f: &mut ChartDataForm) {
    Text::new(tl!("Type the categories down the side and a column of values per series.").to_string()).show(ui);
    let series = f.names.len();
    let rows = f.categories.len();
    let mut columns = vec![ColumnSpec::new(CATEGORY, tl!("Categories").to_string(), ColumnWidth::fill(1.0))];
    for (j, name) in f.names.iter().enumerate() {
        columns.push(
            ColumnSpec::new(column_at(j + 1), name.clone(), ColumnWidth::fill(1.0)).numeric().removable(tl!("Remove series").to_string(), series > 1),
        );
    }
    let remove_row_column = column_at(series + 1);
    columns.push(ColumnSpec::new(remove_row_column.clone(), "", ColumnWidth::Controls(1.0)));

    let edits = RefCell::new(Vec::<Edit>::new());
    // Production scrolls the rows in 280 pt (about eleven rows); the cap also makes the table the one owner of its
    // scroll inside the dialog's scrolling body.
    let mut table = Table::<usize>::new(tl!("Chart data").to_string(), columns).select(SelectMode::Single).max_rows(DATA_ROWS_SHOWN);
    for (j, name) in f.names.iter().enumerate() {
        let edits = &edits;
        let field = TextField::new(tl!("Series name").to_string(), Bind::given(name.clone())).placeholder(tl!("Series name").to_string());
        table = table.header_field(column_at(j + 1), field, move |out| {
            if let Some(text) = out.value().cloned() {
                edits.borrow_mut().push(Edit::Name(j, text));
            }
        });
    }
    let positions: Vec<usize> = (0..rows).collect();
    let source = keyed(&positions, |i| *i);
    let can_remove_row = rows > 1;
    let shown = &*f;
    let out = table.show_items(ui, &source, &mut |key, icx| {
        let at = *key;
        let edits = &edits;
        let category = shown.categories.get(at).cloned().unwrap_or_default();
        let mut row = ListRow::new(category.clone()).cells(icx.cells()).cell_field(
            CATEGORY,
            TextField::new(tl!("Category").to_string(), Bind::given(category)),
            move |out| {
                if let Some(text) = out.value().cloned() {
                    edits.borrow_mut().push(Edit::Category(at, text));
                }
            },
        );
        for j in 0..series {
            let cell = shown.cells.get(at).and_then(|r| r.get(j)).cloned().unwrap_or_default();
            row = row.cell_field(column_at(j + 1), TextField::new(tl!("Value").to_string(), Bind::given(cell)), move |out| {
                if let Some(text) = out.value().cloned() {
                    edits.borrow_mut().push(Edit::Cell(at, j, text));
                }
            });
        }
        row.cell_field(
            remove_row_column.clone(),
            IconButton::new(IconId::new("wordcraft.close")).label(tl!("Remove row").to_string()).enabled(can_remove_row),
            move |out| {
                if out.clicked() {
                    edits.borrow_mut().push(Edit::RemoveRow(at));
                }
            },
        )
    });

    for edit in edits.into_inner() {
        match edit {
            Edit::Category(i, text) => {
                if let Some(c) = f.categories.get_mut(i) {
                    *c = text;
                }
            }
            Edit::Cell(i, j, text) => {
                if let Some(cell) = f.cells.get_mut(i).and_then(|r| r.get_mut(j)) {
                    *cell = text;
                }
            }
            Edit::Name(j, text) => {
                if let Some(name) = f.names.get_mut(j) {
                    *name = text;
                }
            }
            Edit::RemoveRow(i) => f.remove_row(i),
        }
    }
    for event in out.events {
        // A series column is `index(j + 1)`; the category column (0) and the row control (after the series) never report a removal.
        if let CollectionEvent::Column(ColumnEvent::Removed { key, .. }) = event
            && let Some(position) = key.position()
            && position >= 1
        {
            f.remove_series(position as usize - 1);
        }
    }

    Row::new().show(ui, |ui, _| {
        if Button::new(tl!("Add Row").to_string()).enabled(f.categories.len() < MAX_POINTS).show(ui).clicked() {
            f.add_row();
        }
        if Button::new(tl!("Add Series").to_string()).enabled(f.names.len() < MAX_SERIES).show(ui).clicked() {
            f.add_series();
        }
    });
}

/// Opens the dialog `name` names: `insertChart`, `changeChartType` (the selected chart) or `chartData` (the selected
/// chart); `None` for another name or without a chart to change.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "insertChart" | "changeChartType" => {
            let form = ChartTypeForm::new(app, name == "changeChartType")?;
            Some(open_form(app, parent, form))
        }
        "chartData" => {
            let form = ChartDataForm::new(app)?;
            Some(open_form(app, parent, form))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_grid_round_trips_the_chart() {
        let c = ChartSpec::sample(ChartType::Column);
        let mut f = ChartDataForm::of(&c);
        let p = f.params().unwrap();
        assert_eq!(p["data"]["categories"][3], "Q4");
        assert_eq!(p["data"]["series"][1]["values"][2], 3.9);
        f.add_series();
        f.add_row();
        f.cells[0][3] = "1,5".into();
        let p = f.params().unwrap();
        assert_eq!(p["data"]["series"][3]["values"][0], 1.5);
        assert!(p["data"]["series"][3]["values"][1].is_null());
        f.cells[1][0] = "abc".into();
        assert!(f.params().is_err());
    }

    /// Default names are saved into the document, so an RTL interface must not add isolate marks.
    #[test]
    fn default_names_in_an_rtl_interface_have_no_isolates() {
        let ctx = egui::Context::default();
        let arabic = crate::i18n::fixture_host(&ctx, "ar", "\tCategory {n}\tالفئة {n}\n\tSeries {n}\tالسلسلة {n}\n");
        crate::i18n::apply_language(arabic);
        let mut f = ChartDataForm::of(&ChartSpec::sample(ChartType::Column));
        f.add_row();
        f.add_series();
        crate::i18n::apply_language(crate::i18n::Lang::EN);
        let (category, series) = (f.categories.last().unwrap(), f.names.last().unwrap());
        assert!(category.starts_with("الفئة") && series.starts_with("السلسلة"), "{category:?} {series:?}");
        for name in [category, series] {
            assert!(!name.chars().any(|c| ('\u{2066}'..='\u{2069}').contains(&c)), "{name:?}");
        }
    }
}
