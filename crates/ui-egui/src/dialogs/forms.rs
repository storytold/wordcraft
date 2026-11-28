use super::{Value, WordApp};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::json;

use craft::behavior::{Mixed, Selection, keyed};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Politeness, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Stack;
use craft::ui::list::{List, ListRow};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use craft::ui::sections::Section;
use craft::ui::style::TypeRole;
use craft::ui::text::{NumberField, TextField};

use super::host::{Asks, WordForm, open_form};

/// Opens the form dialog `name` when it is one of this file's: Table Properties (only with a table selection) or Custom
/// Properties.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "tableProperties" => {
            let form = TableProperties::read(app)?;
            Some(open_form(app, parent, form))
        }
        "customProperties" => Some(open_form(app, parent, CustomPropertiesForm::read(app))),
        _ => None,
    }
}

/// A muted caption under the rows.
fn note(ui: &mut Ui, text: String) {
    Text::new(Label::from(text)).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}

/// A check box over `value`; `enabled` off greys it.
fn flag(ui: &mut Ui, id: &'static str, label: String, value: &mut bool, enabled: bool) {
    let shown = Checkbox::new(Bind::given(*value)).label(Label::from(label)).id_salt(id).enabled(enabled).show(ui);
    if let Some(on) = shown.value().cloned() {
        *value = on;
    }
}

/// A radio group choosing one of `options` (key, label) into `value`.
fn choose(ui: &mut Ui, id: &'static str, label: String, value: &mut String, options: &[(&str, String)]) {
    let chosen = RadioGroup::new(Label::from(label), Bind::given(value.clone())).id_salt(id).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            for (key, text) in options {
                Radio::new((*key).to_string()).label(Label::from(text.clone())).show(ui);
            }
        });
    });
    if let Some(next) = chosen.output.value().cloned() {
        *value = next;
    }
}

/// The Custom tab of File › Info › Properties › Advanced Properties (#2982): the document's
/// custom properties, edited as a working copy. OK sends the whole difference from what the
/// dialog opened with as one undoable `file.properties` step (marking the document changed and
/// refreshing File › Info); Cancel sends nothing. Text properties only for now: Word's
/// number/date/yes-no types aren't offered, and a property of another type keeps its stored
/// form — the dialog lists it but refuses to change it (Delete still removes one outright).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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

impl WordForm for CustomPropertiesForm {
    fn key(&self) -> &'static str {
        "customProperties"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Custom Properties").to_string())
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let _ = (cx, asks);
        let rows: Vec<String> = self.props.iter().map(|p| format!("{} = {}", p.name, p.value)).collect();
        let positions: Vec<usize> = (0..rows.len()).collect();
        let mut selection = if self.selected < rows.len() { Selection::single(self.selected) } else { Selection::default() };
        List::new(Label::from(tl!("Properties").to_string()))
            .id_salt("props")
            .max_rows(6)
            .select(SelectMode::Single)
            .allow_empty(true)
            .empty(Label::from(tl!("This document has no custom properties.").to_string()))
            .selection(Bind::bound(&mut selection))
            .show_items(ui, &keyed(&positions, |at| *at), &mut |at, _| ListRow::new(rows.get(*at).cloned().unwrap_or_default()));
        if let Some(at) = selection.items.first().copied()
            && at != self.selected
        {
            self.select(at);
        }
        let has_row = self.selected < self.props.len();
        Stack::new().show(ui, |ui, _| {
            if Button::new(Label::from(tl!("Add").to_string())).id_salt("add").show(ui).clicked() {
                self.add();
            }
            if Button::new(Label::from(tl!("Modify").to_string())).id_salt("modify").enabled(has_row).show(ui).clicked() {
                self.modify();
            }
            if Button::new(Label::from(tl!("Delete").to_string())).id_salt("delete").enabled(has_row).show(ui).clicked() {
                self.delete();
            }
        });
        let typed = TextField::new(Label::from(tl!("Name:").to_string()), Bind::given(self.name.clone()))
            .id_salt("name")
            .limit(wordcraft_engine::cmd::file::MAX_CUSTOM_NAME)
            .show(ui);
        if let Some(text) = typed.value().cloned() {
            self.name = text;
        }
        let typed = TextField::new(Label::from(tl!("Value:").to_string()), Bind::given(self.value.clone())).id_salt("value").show(ui);
        if let Some(text) = typed.value().cloned() {
            self.value = text;
        }
        if !self.message.is_empty() {
            // A refusal is announced to a screen reader as it appears (an Alert node).
            Notice::new(Tone::Bad, Label::from(self.message.clone())).layout(NoticeLayout::Inline).announce(Politeness::Assertive).show(ui);
        }
        note(ui, tl!("Text properties only: Word's number, date and yes-no types aren't offered.").to_string());
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        if self.props == self.start { Vec::new() } else { vec![("file.properties".to_string(), self.params())] }
    }
}

/// The Table Properties dialog's fields. Lengths are in points; the length fields show them in the interface unit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
    /// The interface unit the length fields show, set when the dialog opens; not part of the dialog's values.
    #[serde(skip)]
    unit: Option<wordcraft_geom::Unit>,
}

impl TableForm {
    /// The caret's table, row, column and cell as the dialog shows them; a row check box the
    /// selected rows differ in is mixed.
    pub fn read(app: &WordApp) -> Option<TableForm> {
        use wordcraft_doc::props::{Align, HeightRule, VAlign};
        let s = &app.session;
        let (tp, r, c) = s.sel.focus.path.cell()?;
        let t = s.doc.table(s.sel.focus.story, &tp)?;
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
            width: fin(width.unwrap_or(tw)),
            align: match t.props.align {
                Some(Align::Center) => "center",
                Some(Align::Right) => "right",
                _ => "left",
            }
            .into(),
            indent: fin(t.props.indent.unwrap_or(0.0)),
            rtl: t.props.bidi_visual,
            row_height_on: row.height.is_some() && row.height_rule != HeightRule::Auto,
            row_height: fin(row.height.unwrap_or(18.0)),
            row_exact: row.height_rule == HeightRule::Exact,
            allow_break: !row.cant_split,
            header_row: row.header,
            mixed,
            column_width: fin(t.grid.get(t.grid_col(r, c)).copied().unwrap_or(72.0)),
            cell_width_on: cell.width.is_some(),
            cell_width: fin(cell.width.unwrap_or(72.0)),
            valign: match cell.valign {
                Some(VAlign::Center) => "center",
                Some(VAlign::Bottom) => "bottom",
                _ => "top",
            }
            .into(),
            unit: Some(s.units()),
        })
    }

    /// `table.properties` parameters for what differs from `basis` (empty when nothing does).
    pub fn changes(&self, basis: &TableForm) -> Value {
        let moved = |a: f32, b: f32| (a - b).abs() > 1e-4;
        let mut v = serde_json::Map::new();
        if self.width_on != basis.width_on || (self.width_on && moved(self.width, basis.width)) {
            v.insert("width".into(), if self.width_on { json!(self.width) } else { Value::Null });
        }
        if self.align != basis.align {
            v.insert("align".into(), json!(self.align));
        }
        if moved(self.indent, basis.indent) {
            v.insert("indent".into(), json!(self.indent));
        }
        if self.rtl != basis.rtl {
            v.insert("rtl".into(), json!(self.rtl));
        }
        if self.row_height_on != basis.row_height_on
            || (self.row_height_on && (moved(self.row_height, basis.row_height) || self.row_exact != basis.row_exact))
        {
            v.insert("rowHeight".into(), if self.row_height_on { json!(self.row_height) } else { Value::Null });
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
            v.insert("columnWidth".into(), json!(self.column_width));
        }
        if self.cell_width_on != basis.cell_width_on || (self.cell_width_on && moved(self.cell_width, basis.cell_width)) {
            v.insert("cellWidth".into(), if self.cell_width_on { json!(self.cell_width) } else { Value::Null });
        }
        if self.valign != basis.valign {
            v.insert("valign".into(), json!(self.valign));
        }
        Value::Object(v)
    }
}

/// The Table Properties dialog's form: the fields and what they were when it opened.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TableProperties {
    pub form: TableForm,
    basis: TableForm,
}

impl TableProperties {
    /// The form for the caret's table; `None` outside a table.
    pub(crate) fn read(app: &WordApp) -> Option<Self> {
        let form = TableForm::read(app)?;
        Some(Self { basis: form.clone(), form })
    }
}

/// A check box over a setting the selected rows may differ in: mixed while its key is in `mixed`, and a click makes it
/// checked or unchecked and removes the key.
fn mixed_check(ui: &mut Ui, id: &'static str, label: String, key: &str, value: &mut bool, mixed: &mut std::collections::BTreeSet<String>) {
    let mut state = if mixed.contains(key) { Mixed::Mixed } else { Mixed::Same(*value) };
    Checkbox::new(Bind::bound_mixed(&mut state)).label(Label::from(label)).id_salt(id).show(ui);
    if let Mixed::Same(v) = state {
        *value = v;
        mixed.remove(key);
    }
}

/// A length field over points, shown in the interface unit of `units`, within `lo..=hi` points.
fn length(ui: &mut Ui, id: &'static str, label: String, value: &mut f32, (lo, hi): (f32, f32), units: LengthUnits, enabled: bool) {
    let mut points = Some(f64::from(*value));
    NumberField::length(Label::from(label), Bind::bound(&mut points), units)
        .id_salt(id)
        .range(f64::from(lo)..=f64::from(hi))
        .enabled(enabled)
        .show(ui);
    if let Some(next) = points {
        *value = next as f32;
    }
}

impl WordForm for TableProperties {
    fn key(&self) -> &'static str {
        "tableProperties"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Table Properties").to_string())
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let _ = (cx, asks);
        let f = &mut self.form;
        let kind = f.unit.map_or(LengthUnits::new(UnitKind::Points, UnitKind::Points, 96.0), crate::options::lengths);
        let (wide, row, indent) = ((7.2, 1584.0), (1.44, 1584.0), (-792.0, 1584.0));
        Section::fixed(Label::from(tl!("Table").to_string())).id_salt("table").show(ui, |ui, _| {
            flag(ui, "widthOn", tl!("Preferred width").to_string(), &mut f.width_on, true);
            length(ui, "width", tl!("Width").to_string(), &mut f.width, wide, kind, f.width_on);
            choose(
                ui,
                "align",
                tl!("Alignment").to_string(),
                &mut f.align,
                &[("left", tl!("Left").to_string()), ("center", tl!("Center").to_string()), ("right", tl!("Right").to_string())],
            );
            length(ui, "indent", tl!("Indent from left").to_string(), &mut f.indent, indent, kind, f.align == "left");
            let mut dir = if f.rtl { "rtl" } else { "ltr" }.to_string();
            choose(
                ui,
                "direction",
                tl!("Table direction").to_string(),
                &mut dir,
                &[("ltr", tl!("Left-to-right").to_string()), ("rtl", tl!("Right-to-left").to_string())],
            );
            f.rtl = dir == "rtl";
        });
        Section::fixed(Label::from(tl!("Row").to_string())).id_salt("row").show(ui, |ui, _| {
            flag(ui, "heightOn", tl!("Specify height").to_string(), &mut f.row_height_on, true);
            length(ui, "height", tl!("Height").to_string(), &mut f.row_height, row, kind, f.row_height_on);
            let mut rule = if f.row_exact { "exact" } else { "atLeast" }.to_string();
            choose(
                ui,
                "rule",
                tl!("Row height is").to_string(),
                &mut rule,
                &[("atLeast", tl!("At least").to_string()), ("exact", tl!("Exactly").to_string())],
            );
            f.row_exact = rule == "exact";
            mixed_check(ui, "allowBreak", tl!("Allow row to break across pages").to_string(), "allowBreak", &mut f.allow_break, &mut f.mixed);
            mixed_check(ui, "headerRow", tl!("Repeat as header row on each page").to_string(), "headerRow", &mut f.header_row, &mut f.mixed);
        });
        Section::fixed(Label::from(tl!("Column").to_string())).id_salt("column").show(ui, |ui, _| {
            length(ui, "columnWidth", tl!("Width").to_string(), &mut f.column_width, wide, kind, true);
        });
        Section::fixed(Label::from(tl!("Cell").to_string())).id_salt("cell").show(ui, |ui, _| {
            flag(ui, "cellWidthOn", tl!("Preferred width").to_string(), &mut f.cell_width_on, true);
            length(ui, "cellWidth", tl!("Width").to_string(), &mut f.cell_width, wide, kind, f.cell_width_on);
            choose(
                ui,
                "valign",
                tl!("Vertical alignment").to_string(),
                &mut f.valign,
                &[("top", tl!("Top").to_string()), ("center", tl!("Center").to_string()), ("bottom", tl!("Bottom").to_string())],
            );
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let changes = self.form.changes(&self.basis);
        if changes.as_object().is_some_and(|m| m.is_empty()) { Vec::new() } else { vec![("table.properties".to_string(), changes)] }
    }
}
