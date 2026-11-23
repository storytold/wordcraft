use craft::font_manager::picker::FontPickerSettings;
use craft::text::ui::DocUnit;
use craft::text::ui::columns::{ColumnsApply, TextColumns, TextColumnsDialog};
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::DialogId;
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::runtime::env::Label;
use craft::ui::text::NumberField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{settled, word};
use crate::WordApp;
use crate::dialogs::host::{Asks, DialogHooks, Keyed, Settle, WordForm, open_with};
use crate::options::lengths;
use crate::widgets::font_box::{CatalogRef, font_picker};

// ---------------------------------------------------------------------------------------------
// Drop Cap (#2601)

/// The Drop Cap dialog's fields (Insert › Drop Cap › Drop Cap Options…). `distance` is in points.
/// The skipped fields hold app data that `prepare` fills each pass, so an automation round trip
/// through serde loses nothing for good.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DropCapForm {
    /// `none`, `dropped` or `margin`.
    pub position: String,
    /// The letter's font: its own, else the text's.
    pub font: String,
    /// The text's font: choosing it leaves the letter without a font of its own.
    pub text_font: String,
    pub lines: u32,
    /// Points.
    pub distance: f32,
    /// The font list the font box offers.
    #[serde(skip)]
    catalog: CatalogRef,
    /// The font box's remembered fonts, written back to the app after each pass.
    #[serde(skip)]
    settings: FontPickerSettings,
    /// The interface length unit the distance shows in.
    #[serde(skip)]
    unit: Option<wordcraft_geom::Unit>,
}

impl DropCapForm {
    /// The paragraph's drop cap; `None` where it can't take one (Word greys the button out).
    pub fn read(app: &WordApp) -> Option<DropCapForm> {
        if !crate::app::command_board::command_enabled(app, "insert.dropCap") {
            return None;
        }
        let st = wordcraft_engine::cmd::insert::drop_cap_state(&app.session);
        Some(DropCapForm {
            position: st.position.into(),
            font: st.font.clone().unwrap_or_else(|| st.text_font.clone()),
            text_font: st.text_font,
            lines: u32::from(st.lines.clamp(1, 10)),
            distance: st.distance.max(0.0),
            ..DropCapForm::default()
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
            "distance": self.distance.max(0.0),
            "font": font,
        })
    }
}

impl WordForm for DropCapForm {
    fn key(&self) -> &'static str {
        "dropCap"
    }

    fn title(&self) -> Label {
        word("Drop Cap")
    }

    fn prepare(&mut self, app: &WordApp) {
        // PLAN: assumed `WordApp::{font_catalog, font_picker_settings}` (ribbon-w11, S-L284, S-L316).
        self.catalog = CatalogRef(Some(app.font_catalog()));
        self.settings = app.font_picker_settings();
        self.unit = Some(app.session.units());
    }

    fn write_back(&self, app: &mut WordApp) {
        // PLAN: assumed `WordApp::set_font_picker_settings` (ribbon-w11, S-L284, S-L316).
        app.set_font_picker_settings(self.settings.clone());
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let enabled = self.position != "none";
        Stack::new().show(ui, |ui, _| {
            RadioGroup::new(word("Position"), Bind::bound(&mut self.position)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, cx| {
                    for (at, (v, l)) in [("none", "None"), ("dropped", "Dropped"), ("margin", "In margin")].into_iter().enumerate() {
                        cx.labelled(ui, at, word(l), SideLabel::new().layout(FieldLayout::After), Radio::new(v.to_string()));
                    }
                });
            });
            FieldGroup::new(word("Options")).show(ui, |ui, _| {
                if let Some(catalog) = self.catalog.get() {
                    Field::new(word("Font:")).show(ui, |ui, _| {
                        font_picker(word("Font:"), Bind::bound(&mut self.font), catalog, Bind::bound(&mut self.settings), &[], None)
                            .enabled(enabled)
                            .show(ui);
                    });
                }
                let mut lines = Some(f64::from(self.lines));
                Field::new(word("Lines to drop:")).show(ui, |ui, _| {
                    NumberField::new(word("Lines to drop:"), Bind::bound(&mut lines)).range(1.0..=10.0).step(1.0).enabled(enabled).show(ui);
                });
                if let Some(n) = lines {
                    self.lines = n.round().clamp(1.0, 10.0) as u32;
                }
                if let Some(unit) = self.unit {
                    let mut distance = Some(f64::from(self.distance));
                    Field::new(word("Distance from text:")).show(ui, |ui, _| {
                        NumberField::length(word("Distance from text:"), Bind::bound(&mut distance), lengths(unit))
                            .range(0.0..=22.0 * 72.0)
                            .enabled(enabled)
                            .show(ui);
                    });
                    if let Some(d) = distance {
                        self.distance = d as f32;
                    }
                }
            });
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("insert.dropCap".to_string(), self.params())]
    }
}

// ---------------------------------------------------------------------------------------------
// Columns

/// The Columns dialog's opening state: the section's columns, the Apply to choices and the one
/// chosen at first.
pub(super) fn read(app: &WordApp) -> (TextColumns, Vec<ColumnsApply>, ColumnsApply, f32) {
    let sp = wordcraft_engine::cmd::page::sect(&app.session);
    let c = &sp.columns;
    let count = c.count.clamp(1, 12);
    let sections = app.session.doc.sections().len() > 1;
    let selection = wordcraft_engine::cmd::page::selection_sectionable(&app.session);
    let unequal = c.widths.len() == count as usize && count > 1;
    let mut choices = vec![ColumnsApply::Document, ColumnsApply::Forward];
    if sections {
        choices.insert(0, ColumnsApply::Section);
    }
    if selection {
        choices.push(ColumnsApply::Selection);
    }
    let current = match (selection, sections) {
        (true, _) => ColumnsApply::Selection,
        (false, true) => ColumnsApply::Section,
        (false, false) => ColumnsApply::Document,
    };
    // `TextColumns` is `#[non_exhaustive]`: its fields are set one by one.
    let mut columns = TextColumns::default();
    columns.count = Some(count);
    columns.spacing = Some(c.space.max(0.0));
    columns.widths = if unequal { c.widths.clone() } else { Vec::new() };
    columns.equal_width = Some(!unequal);
    columns.line_between = Some(c.separator);
    columns.apply = Some(current);
    (columns, choices, current, sp.text_width())
}

/// The `layout.columns` parameters for what the dialog changed; a field the user left alone is
/// left out, so the section keeps it. Lengths are rounded to 0.01 pt.
pub(super) fn columns_params(c: &TextColumns, apply: ColumnsApply) -> Value {
    let r = |x: f32| (x * 100.0).round() / 100.0;
    let mut m = Map::new();
    if c.equal_width == Some(false) && !c.widths.is_empty() {
        let widths: Vec<[f32; 2]> = c.widths.iter().map(|(w, s)| [r(*w), r(*s)]).collect();
        m.insert("widths".into(), json!(widths));
    } else {
        if let Some(n) = c.count {
            m.insert("count".into(), json!(n));
        }
        if let Some(s) = c.spacing {
            m.insert("space".into(), json!(r(s)));
        }
    }
    if let Some(sep) = c.line_between {
        m.insert("separator".into(), json!(sep));
    }
    let apply = match apply {
        ColumnsApply::Section => "section",
        ColumnsApply::Forward => "forward",
        ColumnsApply::Selection => "selection",
        _ => "document",
    };
    m.insert("apply".into(), json!(apply));
    Value::Object(m)
}

/// Opens the Columns dialog; its OK is held until `layout.columns` has run.
pub(super) fn open(app: &mut WordApp, parent: Option<DialogId>) -> Option<DialogId> {
    let (columns, choices, current, text_width) = read(app);
    let units = lengths(app.session.units());
    let dialog = TextColumnsDialog::new(columns)
        .word(text_width)
        .max_columns(12)
        .apply_to(&choices, current)
        .unit(DocUnit::new(units.shown, units.pixels_per_inch))
        .held_ok();
    Some(open_with(app, parent, Keyed::new("columns", dialog), DialogHooks { prepare: None, settle: settle_columns }))
}

/// Runs `layout.columns` for the answer OK filed and tells the dialog how it went.
fn settle_columns(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(filed) = app.dialog_host.get_mut_by_id::<Keyed<TextColumnsDialog>>(id).map(|k| k.inner_mut().take_ok()) else { return Settle::Done };
    let Some(answer) = filed else { return settled(app, id) };
    let result = match answer {
        // Nothing changed: OK has nothing to apply.
        None => Ok(()),
        Some(cols) => {
            let apply = cols.apply.unwrap_or(ColumnsApply::Document);
            let mut params = columns_params(&cols, apply);
            // `layout.columns` sets every field the section has and takes a missing one as its default (36 pt
            // spacing, no separator), so what the dialog left alone is the section's own value, sent back.
            let section = wordcraft_engine::cmd::page::sect(&app.session).columns;
            if let Some(m) = params.as_object_mut() {
                m.entry("separator").or_insert(json!(section.separator));
                if !m.contains_key("widths") {
                    m.entry("count").or_insert(json!(section.count));
                    m.entry("space").or_insert(json!(section.space));
                }
            }
            app.run("layout.columns", params).map(|_| ()).map_err(|e| Label::from(e.to_string()))
        }
    };
    if let Some(k) = app.dialog_host.get_mut_by_id::<Keyed<TextColumnsDialog>>(id) {
        k.inner_mut().answer_ok(result);
    }
    settled(app, id)
}
