use super::{Color32, MeasureExt, Rect, Sense, Serialize, Stroke, Tokens, Ui, Value, WordApp, buttons, json, pos2, semibold, vec2};

// ---------------------------------------------------------------------------------------------
// Drop Cap (#2601)

/// The Drop Cap dialog's fields (Insert › Drop Cap › Drop Cap Options…). `distance` is in the
/// interface unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropCapForm {
    /// `none`, `dropped` or `margin`.
    pub position: String,
    /// The letter's font: its own, else the text's.
    pub font: String,
    /// The text's font: choosing it leaves the letter without a font of its own.
    pub text_font: String,
    pub lines: u32,
    pub distance: f32,
}

impl DropCapForm {
    /// The paragraph's drop cap; `None` where it can't take one (Word greys the button out).
    pub fn read(app: &WordApp) -> Option<DropCapForm> {
        if !crate::widgets::enabled(app, "insert.dropCap") {
            return None;
        }
        let st = wordcraft_engine::cmd::insert::drop_cap_state(&app.session);
        Some(DropCapForm {
            position: st.position.into(),
            font: st.font.clone().unwrap_or_else(|| st.text_font.clone()),
            text_font: st.text_font,
            lines: u32::from(st.lines.clamp(1, 10)),
            distance: st.distance.max(0.0) / crate::options::unit().pt_per_unit(),
        })
    }

    /// `insert.dropCap` parameters.
    pub fn params(&self) -> Value {
        if self.position == "none" {
            return json!({"position": "none"});
        }
        let font = Some(self.font.trim()).filter(|f| !f.is_empty() && *f != self.text_font);
        json!({
            "position": self.position,
            "lines": self.lines.clamp(1, 10),
            "distance": (self.distance * crate::options::unit().pt_per_unit()).max(0.0),
            "font": font,
        })
    }
}

pub(super) fn drop_cap_ui(app: &mut WordApp, ui: &mut Ui, f: &mut DropCapForm) -> bool {
    ui.label(egui::RichText::new(tl!("Position")).font(semibold(12.5)));
    ui.horizontal(|ui| {
        for (v, l) in [("none", "None"), ("dropped", "Dropped"), ("margin", "In margin")] {
            ui.radio_value(&mut f.position, v.to_string(), tl!(l));
        }
    });
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Options")).font(semibold(12.5)));
    let fams = app.previews.families();
    let preview = app.previews.font_preview_fn();
    let unit = crate::options::unit();
    ui.add_enabled_ui(f.position != "none", |ui| {
        egui::Grid::new("dropcap").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label(tl!("Font:"));
            if let Some(v) = crate::widgets::font_combo(ui, "dropcap_font", 190.0, &f.font, &fams, Some(&*preview))
                && v != crate::widgets::ADD_FONT
            {
                f.font = v;
            }
            ui.end_row();
            ui.label(tl!("Lines to drop:"));
            ui.add(egui::DragValue::new(&mut f.lines).range(1..=10));
            ui.end_row();
            ui.label(tl!("Distance from text:"));
            ui.add(egui::DragValue::new(&mut f.distance).speed(0.01).range(0.0..=22.0 * 72.0 / unit.pt_per_unit()).measure(unit).max_decimals(2));
            ui.end_row();
        });
    });
    let (ok, cancel) = buttons(ui, "OK", "Cancel", true);
    if ok {
        let _ = app.run("insert.dropCap", f.params());
    }
    ok || cancel
}

// ---------------------------------------------------------------------------------------------
// Columns

/// Narrowest column, points (the engine refuses narrower ones).
const MIN_COL_PT: f32 = 18.0;

/// The Columns dialog's fields. Lengths are in the interface unit ([`wordcraft_geom::Unit`]).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnsForm {
    pub count: u32,
    pub equal: bool,
    /// Width and the spacing after it, per column (the last column's spacing is unused).
    pub cols: Vec<(f32, f32)>,
    /// Line between columns.
    pub separator: bool,
    /// `section`, `document`, `forward` (this point forward) or `selection` (selected text).
    pub apply: String,
    /// The section's text width, in the interface unit.
    pub text_width: f32,
    /// The document has more than one section (offer "This section").
    pub sections: bool,
    /// Text is selected that can take columns of its own (offer "Selected text", #2024).
    pub selection: bool,
}

impl ColumnsForm {
    fn k() -> f32 {
        crate::options::unit().pt_per_unit()
    }

    pub fn read(app: &WordApp) -> ColumnsForm {
        let k = Self::k();
        let sp = wordcraft_engine::cmd::page::sect(&app.session);
        let c = &sp.columns;
        let count = c.count.clamp(1, 12);
        let sections = app.session.doc.sections().len() > 1;
        let selection = wordcraft_engine::cmd::page::selection_sectionable(&app.session);
        let apply = match (selection, sections) {
            (true, _) => "selection",
            (false, true) => "section",
            (false, false) => "document",
        };
        let mut f = ColumnsForm {
            count,
            equal: true,
            cols: Vec::new(),
            separator: c.separator,
            apply: apply.into(),
            text_width: sp.text_width() / k,
            sections,
            selection,
        };
        if c.widths.len() == count as usize && count > 1 {
            f.equal = false;
            f.cols = c.widths.iter().map(|(w, s)| (w / k, s / k)).collect();
        } else {
            f.cols = vec![(0.0, c.space.max(0.0) / k); count as usize];
            f.even_out();
        }
        f
    }

    fn min_w() -> f32 {
        MIN_COL_PT / Self::k()
    }

    /// Spacing the first column has (or the default 0.5").
    fn spacing(&self) -> f32 {
        self.cols.first().map(|c| c.1).unwrap_or(36.0 / Self::k())
    }

    /// Equal widths for `count` columns sharing the first column's spacing.
    fn even_out(&mut self) {
        let n = self.count.clamp(1, 12) as usize;
        let room = (self.text_width - Self::min_w() * n as f32).max(0.0);
        let sp = if n > 1 { self.spacing().clamp(0.0, room / (n - 1) as f32) } else { self.spacing() };
        let w = (self.text_width - sp * (n as f32 - 1.0)) / n as f32;
        self.cols = (0..n).map(|i| (w, if i + 1 < n { sp } else { 0.0 })).collect();
        if n == 1
            && let Some(c) = self.cols.first_mut()
        {
            c.1 = sp;
        }
    }

    pub fn set_count(&mut self, n: u32) {
        self.count = n.clamp(1, 12);
        self.even_out();
    }

    /// One, Two, Three, Left or Right.
    pub fn preset(&mut self, p: &str) {
        let k = Self::k();
        let sp = 36.0 / k;
        match p {
            "left" | "right" => {
                self.count = 2;
                self.equal = false;
                let narrow = (self.text_width - sp) / 3.0;
                let wide = (self.text_width - sp) * 2.0 / 3.0;
                self.cols = if p == "left" { vec![(narrow, sp), (wide, 0.0)] } else { vec![(wide, sp), (narrow, 0.0)] };
            }
            _ => {
                self.count = match p {
                    "two" => 2,
                    "three" => 3,
                    _ => 1,
                };
                self.equal = true;
                if let Some(c) = self.cols.first_mut() {
                    c.1 = sp;
                }
                self.even_out();
            }
        }
    }

    /// The preset the current settings match, if any.
    pub fn current_preset(&self) -> Option<&'static str> {
        let near = |a: f32, b: f32| (a - b).abs() < 0.02;
        if self.equal || self.count == 1 {
            return match self.count {
                1 => Some("one"),
                2 => Some("two"),
                3 => Some("three"),
                _ => None,
            };
        }
        let (a, b) = (self.cols.first()?.0, self.cols.get(1)?.0);
        if self.count == 2 && near(a * 2.0, b) {
            Some("left")
        } else if self.count == 2 && near(b * 2.0, a) {
            Some("right")
        } else {
            None
        }
    }

    /// After column `i`'s width or spacing changed: the next column (the one before, for the
    /// last) takes up the difference so the columns fill the text width.
    pub fn rebalance(&mut self, i: usize) {
        let n = self.cols.len();
        if self.equal {
            if let (Some(&(w, sp)), true) = (self.cols.get(i), n > 1) {
                // Equal columns: a changed width sets the spacing, a changed spacing the width.
                let sp = if i == 0 && (w - self.cols.get(1).map(|c| c.0).unwrap_or(w)).abs() > 1e-4 {
                    (self.text_width - w * n as f32) / (n - 1) as f32
                } else {
                    sp
                };
                if let Some(c) = self.cols.first_mut() {
                    c.1 = sp.max(0.0);
                }
            }
            self.even_out();
            return;
        }
        if n < 2 {
            return;
        }
        if let Some(last) = self.cols.last_mut() {
            last.1 = 0.0;
        }
        let j = if i + 1 < n { i + 1 } else { i.saturating_sub(1) };
        let total: f32 = self.cols.iter().map(|(w, s)| w + s).sum();
        let excess = total - self.text_width;
        let min = Self::min_w();
        let short = match self.cols.get_mut(j) {
            Some(c) => {
                c.0 -= excess;
                let short = (min - c.0).max(0.0);
                c.0 = c.0.max(min);
                short
            }
            None => 0.0,
        };
        // The neighbour can't shrink further: take the rest back from the edited column.
        if short > 0.0
            && let Some(c) = self.cols.get_mut(i)
        {
            let from_w = short.min((c.0 - min).max(0.0));
            c.0 -= from_w;
            c.1 = (c.1 - (short - from_w)).max(0.0);
        }
    }

    /// The `layout.columns` parameters.
    pub fn params(&self) -> Value {
        let k = Self::k();
        let r = |x: f32| (x * k * 100.0).round() / 100.0;
        if self.equal || self.count <= 1 {
            json!({"count": self.count, "space": r(self.spacing()), "separator": self.separator, "apply": self.apply})
        } else {
            let widths: Vec<[f32; 2]> = self.cols.iter().map(|(w, s)| [r(*w), r(*s)]).collect();
            json!({"widths": widths, "separator": self.separator, "apply": self.apply})
        }
    }
}

/// A preset tile: a small page with its columns.
fn preset_tile(ui: &mut Ui, label: &str, widths: &[f32], on: bool, t: &Tokens) -> bool {
    let resp = ui
        .vertical(|ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(44.0, 52.0), Sense::click());
            let fill = if on {
                t.checked
            } else if resp.hovered() {
                t.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect(r, 3.0, fill, Stroke::new(1.0, if on { t.accent } else { t.border }), egui::StrokeKind::Inside);
            let page = Rect::from_min_size(r.min + vec2(9.0, 7.0), vec2(26.0, 38.0));
            ui.painter().rect(page, 0.0, t.input, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
            let inner = page.shrink(4.0);
            let gap = 3.5;
            let total: f32 = widths.iter().sum::<f32>().max(1.0);
            let avail = inner.width() - gap * (widths.len() as f32 - 1.0);
            let mut x = inner.min.x;
            for w in widths {
                let cw = avail * w / total;
                let mut y = inner.min.y;
                while y + 1.5 <= inner.max.y {
                    ui.painter().line_segment([pos2(x, y), pos2(x + cw, y)], Stroke::new(1.0, t.text_dim));
                    y += 3.0;
                }
                x += cw + gap;
            }
            ui.label(egui::RichText::new(tl!(label)).small());
            resp
        })
        .inner;
    resp.clicked()
}

/// The page preview: the columns (and the line between them) on a page.
fn columns_preview(ui: &mut Ui, f: &ColumnsForm, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(96.0, 124.0), Sense::hover());
    let page = Rect::from_center_size(r.center(), vec2(84.0, 110.0));
    ui.painter().rect(page, 0.0, t.input, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    let inner = page.shrink(9.0);
    let scale = inner.width() / f.text_width.max(0.1);
    let mut x = inner.min.x;
    let n = f.cols.len();
    for (i, (w, sp)) in f.cols.iter().enumerate() {
        let cw = (w * scale).max(2.0);
        let mut y = inner.min.y;
        while y + 2.0 <= inner.max.y {
            ui.painter().line_segment([pos2(x, y), pos2((x + cw).min(inner.max.x), y)], Stroke::new(1.2, t.text_dim));
            y += 4.0;
        }
        x += cw;
        if i + 1 < n {
            let gap = sp * scale;
            if f.separator {
                let mx = x + gap / 2.0;
                ui.painter().line_segment([pos2(mx, inner.min.y), pos2(mx, inner.max.y)], Stroke::new(1.0, t.text));
            }
            x += gap;
        }
    }
}

pub(super) fn columns_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ColumnsForm) -> bool {
    let t = Tokens::get(ui.ctx());
    let unit = crate::options::unit();
    ui.set_width(420.0);
    ui.label(egui::RichText::new(tl!("Presets")).font(semibold(12.5)));
    let current = f.current_preset();
    ui.horizontal(|ui| {
        for (key, label, widths) in [
            ("one", "One", &[1.0][..]),
            ("two", "Two", &[1.0, 1.0][..]),
            ("three", "Three", &[1.0, 1.0, 1.0][..]),
            ("left", "Left", &[1.0, 2.0][..]),
            ("right", "Right", &[2.0, 1.0][..]),
        ] {
            if preset_tile(ui, label, widths, current == Some(key), &t) {
                f.preset(key);
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Number of columns:"));
        let mut n = f.count;
        if ui.add(egui::DragValue::new(&mut n).range(1..=12)).changed() {
            f.set_count(n);
        }
        ui.add_space(16.0);
        ui.checkbox(&mut f.separator, tl!("Line between"));
    });
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Width and spacing")).font(semibold(12.5)));
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            let mut changed = None;
            egui::ScrollArea::vertical().id_salt("columns_widths").max_height(130.0).show(ui, |ui| {
                egui::Grid::new("columns_grid").num_columns(3).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                    ui.label(tl!("Col #:"));
                    ui.label(tl!("Width:"));
                    ui.label(tl!("Spacing:"));
                    ui.end_row();
                    let n = f.cols.len();
                    let max = f.text_width.max(0.1);
                    for i in 0..n {
                        let editable = i == 0 || !f.equal;
                        ui.label(format!("{}:", i + 1));
                        if let Some(c) = f.cols.get_mut(i) {
                            let w = egui::DragValue::new(&mut c.0).speed(0.01).range(ColumnsForm::min_w()..=max).measure(unit).max_decimals(2);
                            if ui.add_enabled(editable, w).changed() {
                                changed = Some(i);
                            }
                            if i + 1 < n {
                                let s = egui::DragValue::new(&mut c.1).speed(0.01).range(0.0..=max).measure(unit).max_decimals(2);
                                if ui.add_enabled(editable, s).changed() {
                                    changed = Some(i);
                                }
                            } else {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
            });
            if let Some(i) = changed {
                f.rebalance(i);
            }
            let mut equal = f.equal;
            if ui.add_enabled(f.count > 1, egui::Checkbox::new(&mut equal, tl!("Equal column width"))).changed() {
                f.equal = equal;
                if equal {
                    f.even_out();
                }
            }
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(tl!("Preview"));
            columns_preview(ui, f, &t);
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Apply to:"));
        let mut options = vec![("document", "Whole document"), ("forward", "This point forward")];
        if f.sections {
            options.insert(0, ("section", "This section"));
        }
        if f.selection {
            options.push(("selection", "Selected text"));
        }
        let shown = options.iter().find(|(k, _)| *k == f.apply).map(|(_, l)| *l).unwrap_or("This section");
        craft_ui::ComboBox::from_id_salt("columns_apply").selected_text(tl!(shown)).width(180.0).show_ui(ui, |ui| {
            for (k, l) in &options {
                ui.selectable_value(&mut f.apply, (*k).to_string(), tl!(l));
            }
        });
    });
    let (ok, cancel) = buttons(ui, "OK", "Cancel", true);
    if ok {
        let _ = app.run("layout.columns", f.params());
    }
    ok || cancel
}
