//! Insert Chart / Change Chart Type (a gallery of chart types with previews drawn in code) and
//! Edit Data (the chart's categories and series in an editable grid). Each ends by running one
//! command (`insert.chart`, `chart.type`, `chart.editData`), so agents get the same result
//! without the dialog. Shown through [`crate::dialogs_insert::InsertDialog`].

use egui::{Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::chart::{ChartSpec, ChartType, MAX_POINTS, MAX_SERIES};

use crate::WordApp;
use crate::theme::{Tokens, c32};

/// Insert › Chart (`change` false) or Chart Design › Change Chart Type (`change` true).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartTypeForm {
    #[serde(rename = "type")]
    pub kind: ChartType,
    pub change: bool,
}

impl ChartTypeForm {
    pub fn new(app: &WordApp, change: bool) -> Option<ChartTypeForm> {
        let current = wordcraft_engine::cmd::chart::selected_chart(&app.session);
        if change && current.is_none() {
            return None;
        }
        Some(ChartTypeForm { kind: current.map(|c| c.kind).unwrap_or_default(), change })
    }
}

/// Chart Design › Edit Data: the grid as text, a row per category and a column per series.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartDataForm {
    pub categories: Vec<String>,
    pub names: Vec<String>,
    /// `cells[row][series]`.
    pub cells: Vec<Vec<String>>,
    pub message: String,
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
        ChartDataForm { categories: c.categories.clone(), names: c.series.iter().map(|s| s.name.clone()).collect(), cells, message: String::new() }
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

    fn add_row(&mut self) {
        if self.categories.len() < MAX_POINTS {
            self.categories.push(crate::i18n::fmt(tl!("Category {n}"), &[("n", &(self.categories.len() + 1).to_string())]));
            self.cells.push(vec![String::new(); self.names.len()]);
        }
    }

    fn add_series(&mut self) {
        if self.names.len() < MAX_SERIES {
            self.names.push(crate::i18n::fmt(tl!("Series {n}"), &[("n", &(self.names.len() + 1).to_string())]));
            for r in &mut self.cells {
                r.push(String::new());
            }
        }
    }
}

/// The theme's first accents, for previews.
fn accents(app: &WordApp) -> [Color32; 3] {
    let t = &app.session.doc.settings.theme_colors;
    let at = |i: usize, d: Color32| t.get(i).map(|c| c32(*c)).unwrap_or(d);
    [at(4, Color32::from_rgb(0x15, 0x60, 0x82)), at(5, Color32::from_rgb(0xE9, 0x71, 0x32)), at(6, Color32::from_rgb(0x19, 0x6B, 0x24))]
}

/// A small picture of a chart type in `r`, in the accents `col` with axis lines in `line`.
pub fn preview(p: &egui::Painter, r: Rect, kind: ChartType, col: [Color32; 3], line: Color32) {
    let r = r.shrink(r.width().min(r.height()) * 0.12);
    let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let bar = |x0: f32, y0: f32, x1: f32, y1: f32, c: Color32| p.rect_filled(Rect::from_min_max(at(x0, y0), at(x1, y1)), 0.0, c);
    let axis = Stroke::new(1.0, line);
    let data: [[f32; 4]; 3] = [[0.45, 0.6, 0.5, 0.8], [0.3, 0.4, 0.65, 0.5], [0.2, 0.3, 0.35, 0.6]];
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
                        bar(base, 1.0 - a1, base + v, 1.0 - a0, *c);
                    } else {
                        bar(a0, 1.0 - base - v, a1, 1.0 - base, *c);
                    }
                    if stacked {
                        base += v;
                    }
                }
            }
            if horizontal {
                p.line_segment([at(0.0, 0.0), at(0.0, 1.0)], axis);
            } else {
                p.line_segment([at(0.0, 1.0), at(1.0, 1.0)], axis);
            }
        }
        ChartType::Line | ChartType::Area => {
            for (j, c) in col.iter().enumerate().take(2) {
                let Some(d) = data.get(j) else { continue };
                let pts: Vec<Pos2> = d.iter().enumerate().map(|(i, v)| at(i as f32 / 3.0, 1.0 - v)).collect();
                if kind == ChartType::Area {
                    // Convex strips between neighbouring points.
                    for w in pts.windows(2) {
                        if let [a, b] = w {
                            p.add(Shape::convex_polygon(vec![*a, *b, pos2(b.x, r.bottom()), pos2(a.x, r.bottom())], *c, Stroke::NONE));
                        }
                    }
                } else {
                    p.add(Shape::line(pts, Stroke::new(2.0, *c)));
                }
            }
            p.line_segment([at(0.0, 1.0), at(1.0, 1.0)], axis);
        }
        ChartType::Pie | ChartType::Doughnut => {
            let c = r.center();
            let rad = r.width().min(r.height()) / 2.0;
            let shares = [0.42, 0.33, 0.25];
            let mut a = -std::f32::consts::FRAC_PI_2;
            for (k, share) in shares.iter().enumerate() {
                let b = a + share * std::f32::consts::TAU;
                let steps = 12;
                let mut pts = vec![c];
                for s in 0..=steps {
                    let t = a + (b - a) * s as f32 / steps as f32;
                    pts.push(c + vec2(t.cos(), t.sin()) * rad);
                }
                // Each slice is under half a turn, so its fan is convex.
                p.add(Shape::convex_polygon(pts, col.get(k).copied().unwrap_or(line), Stroke::NONE));
                a = b;
            }
            if kind == ChartType::Doughnut {
                p.circle_filled(c, rad * 0.5, Tokens::get(p.ctx()).input);
            }
        }
        ChartType::Scatter => {
            for (j, c) in col.iter().enumerate().take(2) {
                for i in 0..5 {
                    let x = 0.08 + i as f32 * 0.2;
                    let y = 0.85 - (i as f32 * 0.15) - j as f32 * 0.2 + if i % 2 == 0 { 0.05 } else { -0.05 };
                    p.circle_filled(at(x, y.clamp(0.05, 0.95)), 2.5, *c);
                }
            }
            p.line_segment([at(0.0, 1.0), at(1.0, 1.0)], axis);
            p.line_segment([at(0.0, 0.0), at(0.0, 1.0)], axis);
        }
    }
}

/// The chart type gallery; returns true to close.
pub fn type_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ChartTypeForm) -> bool {
    let t = Tokens::get(ui.ctx());
    let col = accents(app);
    let mut chosen = None;
    egui::Grid::new("chart_types").spacing(vec2(8.0, 8.0)).show(ui, |ui| {
        for (k, kind) in ChartType::ALL.into_iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(112.0, 92.0), Sense::click());
            let on = f.kind == kind;
            ui.painter().rect(
                rect,
                4.0,
                if on {
                    t.checked
                } else if resp.hovered() {
                    t.hover
                } else {
                    t.input
                },
                Stroke::new(1.0, if on { t.accent } else { t.border }),
                egui::StrokeKind::Inside,
            );
            let pic = Rect::from_min_size(rect.min + vec2(18.0, 6.0), vec2(76.0, 60.0));
            preview(ui.painter(), pic, kind, col, t.text_dim);
            ui.painter().text(
                pos2(rect.center().x, rect.bottom() - 13.0),
                egui::Align2::CENTER_CENTER,
                tl!(kind.label()),
                crate::theme::regular(11.0),
                t.text,
            );
            let resp = resp.on_hover_text(tl!(kind.label()));
            if resp.clicked() {
                f.kind = kind;
            }
            if resp.double_clicked() {
                chosen = Some(kind);
            }
            if k % 3 == 2 {
                ui.end_row();
            }
        }
    });
    let (ok, cancel) = crate::dialogs::buttons(ui, tl!("OK"));
    let Some(kind) = chosen.or(ok.then_some(f.kind)) else { return cancel };
    let r = if f.change { app.run("chart.type", json!({"type": kind.id()})) } else { app.run("insert.chart", json!({"type": kind.id()})) };
    r.is_ok() || cancel
}

/// The Edit Data grid; returns true to close.
pub fn data_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ChartDataForm) -> bool {
    const CELL_W: f32 = 96.0;
    const ROW_H: f32 = 24.0;
    const X_W: f32 = 22.0;
    let gap = ui.spacing().item_spacing.x;
    let table_w = (f.names.len() as f32 + 1.0) * (CELL_W + X_W + 2.0 * gap) + 16.0;
    ui.set_min_width(table_w.clamp(360.0, 760.0));
    ui.label(tl!("Type the categories down the side and a column of values per series."));
    ui.add_space(6.0);
    let mut remove_series = None;
    let mut remove_row = None;
    let (can_remove_series, can_remove_row) = (f.names.len() > 1, f.categories.len() > 1);
    egui::ScrollArea::horizontal().id_salt("chart_data_h").max_width(760.0).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add_sized([CELL_W, ROW_H - 4.0], egui::Label::new(egui::RichText::new(tl!("Categories")).strong()));
                ui.allocate_exact_size(vec2(X_W, ROW_H - 4.0), Sense::hover());
                for (j, n) in f.names.iter_mut().enumerate() {
                    ui.add_sized([CELL_W, ROW_H - 4.0], egui::TextEdit::singleline(n).hint_text(tl!("Series name")));
                    let x = ui
                        .add_enabled_ui(can_remove_series, |ui| ui.add_sized([X_W, ROW_H - 4.0], egui::Button::new("×")))
                        .inner
                        .on_hover_text(tl!("Remove series"));
                    if x.clicked() {
                        remove_series = Some(j);
                    }
                }
            });
            ui.separator();
            let rows = f.categories.len();
            egui::ScrollArea::vertical().id_salt("chart_data_v").max_height(280.0).show_rows(ui, ROW_H, rows, |ui, range| {
                for i in range {
                    ui.horizontal(|ui| {
                        if let Some(c) = f.categories.get_mut(i) {
                            ui.add_sized([CELL_W, ROW_H - 4.0], egui::TextEdit::singleline(c));
                        }
                        let x = ui
                            .add_enabled_ui(can_remove_row, |ui| ui.add_sized([X_W, ROW_H - 4.0], egui::Button::new("×")))
                            .inner
                            .on_hover_text(tl!("Remove row"));
                        if x.clicked() {
                            remove_row = Some(i);
                        }
                        if let Some(row) = f.cells.get_mut(i) {
                            for cell in row.iter_mut() {
                                ui.add_sized([CELL_W, ROW_H - 4.0], egui::TextEdit::singleline(cell).horizontal_align(egui::Align::Max));
                                ui.allocate_exact_size(vec2(X_W, ROW_H - 4.0), Sense::hover());
                            }
                        }
                    });
                }
            });
        });
    });
    if let Some(j) = remove_series.filter(|_| f.names.len() > 1) {
        if j < f.names.len() {
            f.names.remove(j);
        }
        for r in &mut f.cells {
            if j < r.len() {
                r.remove(j);
            }
        }
    }
    if let Some(i) = remove_row.filter(|i| *i < f.categories.len() && f.categories.len() > 1) {
        f.categories.remove(i);
        if i < f.cells.len() {
            f.cells.remove(i);
        }
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(f.categories.len() < MAX_POINTS, egui::Button::new(tl!("Add Row"))).clicked() {
            f.add_row();
        }
        if ui.add_enabled(f.names.len() < MAX_SERIES, egui::Button::new(tl!("Add Series"))).clicked() {
            f.add_series();
        }
    });
    if !f.message.is_empty() {
        ui.label(egui::RichText::new(f.message.as_str()).color(Tokens::get(ui.ctx()).red));
    }
    let (ok, cancel) = crate::dialogs::buttons(ui, tl!("OK"));
    if !ok {
        return cancel;
    }
    match f.params().and_then(|p| app.run("chart.editData", p)) {
        Ok(_) => true,
        Err(e) => {
            f.message = e;
            false
        }
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
}
