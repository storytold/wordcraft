use super::chrome::buttons;
use super::{MeasureExt, Sense, Serialize, Tokens, Ui, Value, WordApp, json, semibold, vec2};

/// The Custom tab of File › Info › Properties › Advanced Properties (#2982): the document's
/// custom properties, edited as a working copy. OK sends the whole difference from what the
/// dialog opened with as one undoable `file.properties` step (marking the document changed and
/// refreshing File › Info); Cancel sends nothing. Text properties only for now: Word's
/// number/date/yes-no types aren't offered, and a property of another type keeps its stored
/// form — the dialog lists it but refuses to change it (Delete still removes one outright).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPropertiesForm {
    /// The working copy, in document order.
    pub props: Vec<wordcraft_doc::CustomProp>,
    /// What the document held when the dialog opened; OK sends only what differs.
    start: Vec<wordcraft_doc::CustomProp>,
    /// The row the list selection is on.
    pub selected: usize,
    /// The name/value boxes under the list.
    pub name: String,
    pub value: String,
    /// The last error to show under the form (a refused name, a failed command).
    pub message: String,
}

impl CustomPropertiesForm {
    pub(crate) fn read(app: &WordApp) -> Self {
        let props = app.session.doc.custom_props.clone();
        let mut form = Self { props: props.clone(), start: props, selected: 0, name: String::new(), value: String::new(), message: String::new() };
        // The first row is highlighted, so its fields are ready to modify, as in Word.
        form.select(0);
        form
    }

    /// Clicking a row: it fills the name and value boxes, as in Word.
    pub(crate) fn select(&mut self, i: usize) {
        self.selected = i;
        self.message.clear();
        match self.props.get(i) {
            Some(p) => {
                self.name = p.name.clone();
                self.value = p.value.clone();
            }
            None => {
                self.name.clear();
                self.value.clear();
            }
        }
    }

    /// The typed name and value, validated against the engine's own limits
    /// (`file.properties`, which refuses them the same way).
    fn form(&self) -> Result<(String, String), String> {
        let max_name = wordcraft_engine::cmd::file::MAX_CUSTOM_NAME;
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > max_name {
            return Err(crate::i18n::fmt(tl!("Property names are 1–{max} characters."), &[("max", &max_name.to_string())]));
        }
        if self.value.len() > wordcraft_engine::cmd::file::MAX_CUSTOM_VALUE {
            let max = wordcraft_engine::cmd::file::MAX_CUSTOM_VALUE;
            return Err(crate::i18n::fmt(tl!("Property values are at most {max} bytes."), &[("max", &max.to_string())]));
        }
        Ok((name.to_string(), self.value.clone()))
    }

    /// Add: the typed name and value join the list, replacing a property of the same name
    /// (names compare case-insensitively, as in Word and the engine).
    pub(crate) fn add(&mut self) {
        match self.form() {
            Ok((name, value)) => {
                self.message.clear();
                match self.props.iter().position(|p| p.name.eq_ignore_ascii_case(&name)) {
                    Some(i) if self.props[i].kind != "lpwstr" => {
                        self.message = tl!("Only text properties can be changed.").to_string();
                    }
                    Some(i) => {
                        self.props[i].value = value;
                        self.selected = i;
                    }
                    None => {
                        self.props.push(wordcraft_doc::CustomProp { name, kind: "lpwstr".into(), value, ..Default::default() });
                        self.selected = self.props.len().saturating_sub(1);
                    }
                }
            }
            Err(m) => self.message = m,
        }
    }

    /// Modify: the selected property takes the typed name and value; a different name moves it
    /// there (replacing a property already of that name). A refused form keeps the list as it
    /// was.
    pub(crate) fn modify(&mut self) {
        let Some(old) = self.props.get(self.selected).cloned() else {
            self.message = tl!("Select a property to change.").to_string();
            return;
        };
        if old.kind != "lpwstr" {
            self.message = tl!("Only text properties can be changed.").to_string();
            return;
        }
        let (name, value) = match self.form() {
            Ok(f) => f,
            Err(m) => {
                self.message = m;
                return;
            }
        };
        if self.props.iter().enumerate().any(|(i, p)| i != self.selected && p.kind != "lpwstr" && p.name.eq_ignore_ascii_case(&name)) {
            self.message = tl!("Only text properties can be changed.").to_string();
            return;
        }
        self.message.clear();
        self.props.remove(self.selected);
        match self.props.iter().position(|p| p.name.eq_ignore_ascii_case(&name)) {
            Some(i) => {
                self.props[i].value = value;
                self.props[i].link_target = old.link_target;
                self.selected = i;
            }
            None => {
                let i = self.selected.min(self.props.len());
                self.props.insert(i, wordcraft_doc::CustomProp { name, value, ..old });
                self.selected = i;
            }
        }
    }

    /// Delete: the selected property leaves the list.
    pub(crate) fn delete(&mut self) {
        if self.selected < self.props.len() {
            self.props.remove(self.selected);
            self.select(self.selected.min(self.props.len().saturating_sub(1)));
        }
    }

    /// The one `file.properties` call OK makes: a property gone since the dialog opened is
    /// null, one added or changed carries its value and any bookmark target. Untouched properties aren't sent, so one of
    /// another type (Zotero's raw XML, a number) keeps its stored form. A rename that only
    /// changes case sends the old spelling as null too, so the stored spelling follows it.
    pub(crate) fn params(&self) -> Value {
        let mut custom = serde_json::Map::new();
        for p in &self.start {
            match self.props.iter().find(|q| q.name.eq_ignore_ascii_case(&p.name)) {
                Some(q) if q.name == p.name => {}
                _ => {
                    custom.insert(p.name.clone(), Value::Null);
                }
            }
        }
        for p in &self.props {
            let changed = self
                .start
                .iter()
                .find(|q| q.name.eq_ignore_ascii_case(&p.name))
                .is_none_or(|was| was.name != p.name || was.value != p.value || was.link_target != p.link_target);
            if changed {
                // A rename removes the old property before creating the new one. Carry its
                // target too, including an explicit unlink when replacing a linked property.
                let linked = p.link_target.is_some() || self.start.iter().any(|q| q.name.eq_ignore_ascii_case(&p.name) && q.link_target.is_some());
                let value = if linked { json!({"value": p.value, "linkTarget": p.link_target}) } else { json!(p.value) };
                custom.insert(p.name.clone(), value);
            }
        }
        json!({"custom": custom})
    }
}

/// The Table Properties dialog's fields. Lengths are in the interface unit
/// ([`wordcraft_geom::Unit`], inches by default).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableForm {
    pub width_on: bool,
    pub width: f32,
    /// `left`, `center` or `right`.
    pub align: String,
    pub indent: f32,
    /// Table direction: right to left (`w:bidiVisual`).
    pub rtl: bool,
    pub row_height_on: bool,
    pub row_height: f32,
    pub row_exact: bool,
    pub allow_break: bool,
    pub header_row: bool,
    /// The settings the selected rows differ in (`allowBreak`, `headerRow`): shown as neither
    /// on nor off, and left as each row has them unless clicked (#4857).
    pub mixed: std::collections::BTreeSet<String>,
    pub column_width: f32,
    pub cell_width_on: bool,
    pub cell_width: f32,
    /// `top`, `center` or `bottom`.
    pub valign: String,
}

/// The Cell Margins dialog's values, in the interface unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellMarginsForm {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

impl CellMarginsForm {
    pub(crate) fn read(app: &WordApp) -> Option<Self> {
        let (tp, _, _) = app.session.sel.focus.path.cell()?;
        let table = app.session.doc.table(app.session.sel.focus.story, &tp)?;
        let scale = crate::options::unit().pt_per_unit();
        let style = table.props.style.as_deref().and_then(|id| app.session.doc.styles.table_style(id));
        let margins = table.props.cell_margins.or_else(|| style.and_then(|s| s.parts.cell_margins)).unwrap_or([0.0, 5.4, 0.0, 5.4]);
        let field = |value: f32, fallback: f32| if value.is_finite() { value.clamp(0.0, 144.0) / scale } else { fallback / scale };
        Some(Self { top: field(margins[0], 0.0), left: field(margins[1], 5.4), bottom: field(margins[2], 0.0), right: field(margins[3], 5.4) })
    }

    /// Send all four values together: changing one side must preserve the other three.
    pub(crate) fn changes(&self, basis: &CellMarginsForm) -> Value {
        if [self.top, self.left, self.bottom, self.right]
            .into_iter()
            .zip([basis.top, basis.left, basis.bottom, basis.right])
            .all(|(a, b)| (a - b).abs() <= 1e-4)
        {
            return json!({});
        }
        let scale = crate::options::unit().pt_per_unit();
        let points = |v: f32, old: f32| if v.is_finite() { (v * scale).clamp(0.0, 144.0) } else { old * scale };
        json!({
            "top": points(self.top, basis.top),
            "left": points(self.left, basis.left),
            "bottom": points(self.bottom, basis.bottom),
            "right": points(self.right, basis.right),
        })
    }
}

impl TableForm {
    /// The caret's table, row, column and cell as the dialog shows them; a row check box the
    /// selected rows differ in is mixed.
    pub fn read(app: &WordApp) -> Option<TableForm> {
        use wordcraft_doc::props::{Align, HeightRule, VAlign};
        let s = &app.session;
        let (tp, r, c) = s.sel.focus.path.cell()?;
        let t = s.doc.table(s.sel.focus.story, &tp)?;
        let k = crate::options::unit().pt_per_unit();
        let row = t.rows.get(r).map(|x| x.props.clone()).unwrap_or_default();
        let mut rows: Vec<usize> = wordcraft_engine::cmd::table::selected_cells(s).into_iter().map(|(r, _)| r).collect();
        rows.sort_unstable();
        rows.dedup();
        let selected: Vec<&wordcraft_doc::props::RowProps> = rows.iter().filter_map(|&i| t.rows.get(i)).map(|x| &x.props).collect();
        let differ = |f: fn(&wordcraft_doc::props::RowProps) -> bool| selected.iter().any(|p| f(p) != f(&row));
        let mixed = [("allowBreak", differ(|p| p.cant_split)), ("headerRow", differ(|p| p.header))]
            .into_iter()
            .filter(|(_, d)| *d)
            .map(|(k, _)| k.to_string())
            .collect();
        let cell = t.rows.get(r).and_then(|x| x.cells.get(c)).map(|x| x.props.clone()).unwrap_or_default();
        let tw = wordcraft_engine::cmd::page::sect(s).text_width();
        let width = t.props.width_pct.filter(|p| *p > 0.0).map(|p| tw * p.min(100.0) / 100.0).or(t.props.width.filter(|w| *w > 0.0));
        let fin = |v: f32| if v.is_finite() { v } else { 0.0 };
        Some(TableForm {
            width_on: width.is_some(),
            width: fin(width.unwrap_or(tw)) / k,
            align: match t.props.align {
                Some(Align::Center) => "center",
                Some(Align::Right) => "right",
                _ => "left",
            }
            .into(),
            indent: fin(t.props.indent.unwrap_or(0.0)) / k,
            rtl: t.props.bidi_visual,
            row_height_on: row.height.is_some() && row.height_rule != HeightRule::Auto,
            row_height: fin(row.height.unwrap_or(18.0)) / k,
            row_exact: row.height_rule == HeightRule::Exact,
            allow_break: !row.cant_split,
            header_row: row.header,
            mixed,
            column_width: fin(t.grid.get(t.grid_col(r, c)).copied().unwrap_or(72.0)) / k,
            cell_width_on: cell.width.is_some(),
            cell_width: fin(cell.width.unwrap_or(72.0)) / k,
            valign: match cell.valign {
                Some(VAlign::Center) => "center",
                Some(VAlign::Bottom) => "bottom",
                _ => "top",
            }
            .into(),
        })
    }

    /// `table.properties` parameters for what differs from `basis` (empty when nothing does).
    pub fn changes(&self, basis: &TableForm) -> Value {
        let k = crate::options::unit().pt_per_unit();
        let moved = |a: f32, b: f32| (a - b).abs() > 1e-4;
        let mut v = serde_json::Map::new();
        if self.width_on != basis.width_on || (self.width_on && moved(self.width, basis.width)) {
            v.insert("width".into(), if self.width_on { json!(self.width * k) } else { Value::Null });
        }
        if self.align != basis.align {
            v.insert("align".into(), json!(self.align));
        }
        if moved(self.indent, basis.indent) {
            v.insert("indent".into(), json!(self.indent * k));
        }
        if self.rtl != basis.rtl {
            v.insert("rtl".into(), json!(self.rtl));
        }
        if self.row_height_on != basis.row_height_on
            || (self.row_height_on && (moved(self.row_height, basis.row_height) || self.row_exact != basis.row_exact))
        {
            v.insert("rowHeight".into(), if self.row_height_on { json!(self.row_height * k) } else { Value::Null });
            v.insert("rowHeightRule".into(), json!(if self.row_exact { "exact" } else { "atLeast" }));
        }
        // A mixed box sends nothing until clicked; clicked, it sets every selected row.
        let set = |key: &str, on: bool, was: bool| !self.mixed.contains(key) && (basis.mixed.contains(key) || on != was);
        if set("allowBreak", self.allow_break, basis.allow_break) {
            v.insert("allowBreak".into(), json!(self.allow_break));
        }
        if set("headerRow", self.header_row, basis.header_row) {
            v.insert("headerRow".into(), json!(self.header_row));
        }
        if moved(self.column_width, basis.column_width) {
            v.insert("columnWidth".into(), json!(self.column_width * k));
        }
        if self.cell_width_on != basis.cell_width_on || (self.cell_width_on && moved(self.cell_width, basis.cell_width)) {
            v.insert("cellWidth".into(), if self.cell_width_on { json!(self.cell_width * k) } else { Value::Null });
        }
        if self.valign != basis.valign {
            v.insert("valign".into(), json!(self.valign));
        }
        Value::Object(v)
    }
}

/// Grid picker for Insert › Table (hover to size and preview on the page, click to insert).
pub fn table_grid_picker(ui: &mut Ui, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    let id = egui::Id::unique("table_picker_hover");
    let hover: (usize, usize) = ui.data(|d| d.get_temp(id)).unwrap_or((0, 0));
    ui.label(if hover.0 > 0 {
        crate::i18n::fmt(tl!("{cols}×{rows} Table"), &[("cols", &hover.1.to_string()), ("rows", &hover.0.to_string())])
    } else {
        tl!("Insert Table").to_string()
    });
    let mut new_hover = (0, 0);
    let mut clicked = None;
    egui::Grid::new("tgp").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
        for r in 1..=8 {
            for c in 1..=10 {
                let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                let on = r <= hover.0 && c <= hover.1;
                ui.painter().rect(
                    rect,
                    1.0,
                    if on { t.checked } else { t.input },
                    egui::Stroke::new(1.0, if on { t.accent } else { t.border_strong }),
                    egui::StrokeKind::Inside,
                );
                if resp.hovered() {
                    new_hover = (r, c);
                }
                if resp.clicked() {
                    clicked = Some((r, c));
                }
            }
            ui.end_row();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, new_hover));
    if new_hover != hover {
        // The page may already be drawn this frame (Focus drops the ribbon over it afterwards).
        ui.ctx().request_repaint();
    }
    if let Some((r, c)) = clicked {
        let _ = app.run("insert.table", json!({"rows": r, "cols": c}));
        ui.close();
    } else if new_hover.0 > 0 {
        // Word's Live Preview: the page shows the hovered table until a square is clicked.
        app.canvas.table_preview = Some(new_hover);
    }
}

/// The Table Properties dialog's fields: Table, Row, Column and Cell sections.
pub(super) fn table_properties(ui: &mut Ui, f: &mut TableForm) {
    let unit = crate::options::unit();
    fn len(v: &mut f32, lo: f32, unit: wordcraft_geom::Unit) -> egui::DragValue<'_> {
        egui::DragValue::new(v).speed(0.01).range(lo..=22.0).measure(unit).max_decimals(2)
    }
    let heading = |ui: &mut Ui, s: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
    };
    heading(ui, "Table");
    egui::Grid::new("tp_table").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.checkbox(&mut f.width_on, tl!("Preferred width:"));
        ui.add_enabled(f.width_on, len(&mut f.width, 0.1, unit));
        ui.end_row();
        ui.label(tl!("Alignment:"));
        ui.horizontal(|ui| {
            for (v, l) in [("left", "Left"), ("center", "Center"), ("right", "Right")] {
                ui.radio_value(&mut f.align, v.to_string(), tl!(l));
            }
        });
        ui.end_row();
        ui.label(tl!("Indent from left:"));
        ui.add_enabled(f.align == "left", egui::DragValue::new(&mut f.indent).speed(0.01).range(-11.0..=22.0).measure(unit).max_decimals(2));
        ui.end_row();
        ui.label(tl!("Table direction:"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut f.rtl, false, tl!("Left-to-right"));
            ui.radio_value(&mut f.rtl, true, tl!("Right-to-left"));
        });
        ui.end_row();
    });
    heading(ui, "Row");
    egui::Grid::new("tp_row").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let specify = ui.checkbox(&mut f.row_height_on, tl!("Specify height:"));
        ui.horizontal(|ui| {
            ui.add_enabled(f.row_height_on, len(&mut f.row_height, 0.02, unit));
            ui.add_enabled_ui(f.row_height_on, |ui| {
                let rule = craft_ui::ComboBox::from_id_salt("tp_rule").selected_text(tl!(if f.row_exact { "Exactly" } else { "At least" })).show_ui(
                    ui,
                    |ui| {
                        ui.selectable_value(&mut f.row_exact, false, tl!("At least"));
                        ui.selectable_value(&mut f.row_exact, true, tl!("Exactly"));
                    },
                );
                // A screen reader names the rule by the box it belongs to.
                rule.response.labelled_by(specify.id);
            });
        });
        ui.end_row();
    });
    super::paragraph::para_check(ui, &mut f.allow_break, tl!("Allow row to break across pages"), "allowBreak", &mut f.mixed);
    super::paragraph::para_check(ui, &mut f.header_row, tl!("Repeat as header row on each page"), "headerRow", &mut f.mixed);
    heading(ui, "Column");
    egui::Grid::new("tp_col").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Preferred width:"));
        ui.add(len(&mut f.column_width, 0.1, unit));
        ui.end_row();
    });
    heading(ui, "Cell");
    egui::Grid::new("tp_cell").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.checkbox(&mut f.cell_width_on, tl!("Preferred width:"));
        ui.add_enabled(f.cell_width_on, len(&mut f.cell_width, 0.1, unit));
        ui.end_row();
        ui.label(tl!("Vertical alignment:"));
        ui.horizontal(|ui| {
            for (v, l) in [("top", "Top"), ("center", "Center"), ("bottom", "Bottom")] {
                ui.radio_value(&mut f.valign, v.to_string(), tl!(l));
            }
        });
        ui.end_row();
    });
}

pub(super) fn cell_margins_dialog(ui: &mut Ui, f: &mut CellMarginsForm) {
    let unit = crate::options::unit();
    let max = 144.0 / unit.pt_per_unit();
    egui::Grid::new("cell_margins").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        for (label, value) in [("Top:", &mut f.top), ("Left:", &mut f.left), ("Bottom:", &mut f.bottom), ("Right:", &mut f.right)] {
            ui.label(tl!(label));
            ui.add(egui::DragValue::new(value).speed(0.01).range(0.0..=max).measure(unit).max_decimals(2));
            ui.end_row();
        }
    });
}

/// A custom property's one-line list preview: its first line, at most 80 characters, with an
/// ellipsis when either is cut. The Value box keeps the whole value.
pub(super) fn custom_value_preview(value: &str) -> String {
    const MAX: usize = 80;
    let first = value.lines().next().unwrap_or("");
    let mut chars = first.chars();
    let mut shown: String = chars.by_ref().take(MAX).collect();
    if chars.next().is_some() || first.len() < value.len() {
        shown.push('…');
    }
    shown
}

/// The Custom tab of File › Info › Properties › Advanced Properties (#2982): the document's
/// custom properties with Add, Modify and Delete over a name/value form. OK is one undo step
/// (one `file.properties` call), Cancel leaves the document as it was.
pub(super) fn custom_properties(app: &mut WordApp, ui: &mut Ui, f: &mut CustomPropertiesForm) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    let rows = f.props.len().clamp(1, 8);
    let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical().id_salt("custom_props").max_height(rows as f32 * row_h + 4.0).auto_shrink([false, true]).show(ui, |ui| {
        if f.props.is_empty() {
            ui.label(egui::RichText::new(tl!("This document has no custom properties.")).color(t.text_dim));
        }
        for (i, p) in f.props.iter().enumerate() {
            let row = ui.selectable_label(f.selected == i, format!("{} = {}", p.name, custom_value_preview(&p.value)));
            if row.clicked() {
                clicked = Some(i);
            }
        }
    });
    if let Some(i) = clicked {
        f.select(i);
    }
    ui.horizontal(|ui| {
        if ui.button(tl!("Add")).clicked() {
            f.add();
        }
        if ui.add_enabled(f.selected < f.props.len(), egui::Button::new(tl!("Modify"))).clicked() {
            f.modify();
        }
        if ui.add_enabled(f.selected < f.props.len(), egui::Button::new(tl!("Delete"))).clicked() {
            f.delete();
        }
    });
    ui.separator();
    egui::Grid::new("custom_props_form").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let name = ui.label(tl!("Name:"));
        ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(300.0).char_limit(wordcraft_engine::cmd::file::MAX_CUSTOM_NAME))
            .labelled_by(name.id);
        ui.end_row();
        let value = ui.label(tl!("Value:"));
        ui.add(egui::TextEdit::singleline(&mut f.value).desired_width(300.0)).labelled_by(value.id);
        ui.end_row();
    });
    if !f.message.is_empty() {
        ui.colored_label(ui.visuals().error_fg_color, f.message.as_str());
    }
    ui.label(egui::RichText::new(tl!("Text properties only: Word's number, date and yes-no types aren't offered.")).small().weak());
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if cancel {
        return true;
    }
    if ok {
        let params = f.params();
        if params["custom"].as_object().is_some_and(serde_json::Map::is_empty) {
            return true;
        }
        // One call: one undo step, the document marked changed, File › Info reads it afresh.
        match app.run("file.properties", params) {
            Ok(_) => return true,
            Err(e) => f.message = e,
        }
    }
    false
}
