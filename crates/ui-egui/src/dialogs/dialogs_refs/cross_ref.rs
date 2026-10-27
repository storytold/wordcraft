use super::common::{BUILT_IN_LABELS, choice};
use super::shell::Next;
use super::*;

// ---------------------------------------------------------------------------------------------
// Cross-reference

/// Reference types: (`insert.crossReference`'s `to`, label).
const REF_TYPES: [(&str, &str); 5] =
    [("heading", "Heading"), ("bookmark", "Bookmark"), ("equation", "Equation"), ("figure", "Figure"), ("table", "Table")];

/// The Reference types to pick from: [`REF_TYPES`], then each caption label of the person's own
/// (Insert Caption › New Label, or already in the document's captions) as `label:<Name>` (#2940).
pub(super) fn ref_types(app: &WordApp) -> Vec<(String, String)> {
    let mut types: Vec<(String, String)> = REF_TYPES.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
    let in_document = wordcraft_engine::cmd::citations::caption_labels(&app.session);
    for l in app.ui.caption_labels.iter().chain(&in_document) {
        let value = format!("label:{l}");
        if !BUILT_IN_LABELS.contains(&l.as_str()) && !types.iter().any(|(v, _)| *v == value) {
            types.push((value, l.clone()));
        }
    }
    types
}

/// The Cross-reference dialog's fields.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossRefForm {
    /// `heading`, `bookmark`, `equation`, `figure`, `table`, or `label:<Name>` for a caption label
    /// of the person's own.
    pub to: String,
    /// `text`, `page`, `number` or `aboveBelow`.
    pub show: String,
    /// The document's targets of this type.
    pub targets: Vec<String>,
    pub selected: Option<usize>,
}

impl CrossRefForm {
    pub fn read(app: &WordApp, to: &str) -> CrossRefForm {
        let targets: Vec<String> = wordcraft_engine::cmd::citations::ref_targets(&app.session, to).into_iter().map(|(t, _)| t).collect();
        let selected = (!targets.is_empty()).then_some(0);
        CrossRefForm { to: to.into(), show: "text".into(), targets, selected }
    }

    /// What can be inserted for this type: (`show`, label).
    pub fn shows(&self) -> &'static [(&'static str, &'static str)] {
        match self.to.as_str() {
            "heading" => &[("text", "Heading text"), ("page", "Page number"), ("aboveBelow", "Above/below")],
            "bookmark" => &[("text", "Bookmark text"), ("page", "Page number"), ("aboveBelow", "Above/below")],
            _ => &[("text", "Entire caption"), ("number", "Only label and number"), ("page", "Page number"), ("aboveBelow", "Above/below")],
        }
    }

    /// `insert.crossReference` parameters, or `None` with nothing picked.
    pub fn params(&self) -> Option<Value> {
        let target = self.targets.get(self.selected?)?;
        Some(json!({"to": self.to, "target": target, "show": self.show}))
    }
}

/// Draws the Cross-reference dialog's reference-type and insert-reference-to selectors and the
/// target list, reloading the form when the type changes, and runs `insert.crossReference` on
/// Insert.
pub(super) fn cross_ref_ui(app: &mut WordApp, ui: &mut Ui, f: &mut CrossRefForm) -> Next {
    let was = f.to.clone();
    egui::Grid::new("xref_opts").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let type_label = ui.label(tl!("Reference type:"));
        let types = ref_types(app);
        let types: Vec<(String, &str)> = types.iter().map(|(v, l)| (v.clone(), l.as_str())).collect();
        choice(ui, "xref_type", &mut f.to, &types, &type_label);
        ui.end_row();
        let show_label = ui.label(tl!("Insert reference to:"));
        let shows: Vec<(String, &str)> = f.shows().iter().map(|(v, l)| (v.to_string(), *l)).collect();
        choice(ui, "xref_show", &mut f.show, &shows, &show_label);
        ui.end_row();
    });
    if f.to != was {
        *f = CrossRefForm::read(app, &f.to);
    }
    ui.add_space(4.0);
    ui.label(tl!("For which item:"));
    egui::Frame::group(ui.style()).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(320.0);
            if f.targets.is_empty() {
                ui.weak(tl!("Nothing of this type in the document."));
            }
            for (i, t) in f.targets.iter().enumerate() {
                if ui.selectable_label(f.selected == Some(i), t).clicked() {
                    f.selected = Some(i);
                }
            }
        });
    });
    let (ok, cancel) = buttons(ui, tl!("Insert"));
    if ok && let Some(params) = f.params() {
        if let Err(e) = app.run("insert.crossReference", params) {
            app.status(e);
        }
        return Next::Close;
    }
    if cancel { Next::Close } else { Next::Stay }
}
