use craft::behavior::{Mixed, Selection, keyed};
use craft::ui::contract::{Bind, Entry, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::form::Field;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::labels::{BUILT_IN_LABELS, caption_labels};
use crate::WordApp;
use crate::dialogs::host::{Asks, DialogHooks, FormHost, WordForm, open_form_with};

/// Reference types: (`insert.crossReference`'s `to`, label).
const REF_TYPES: [(&str, &str); 5] =
    [("heading", "Heading"), ("bookmark", "Bookmark"), ("equation", "Equation"), ("figure", "Figure"), ("table", "Table")];

/// The Reference types to pick from: [`REF_TYPES`], then each caption label of the person's own
/// (Insert Caption › New Label, or already in the document's captions) as `label:<Name>` (#2940).
pub(super) fn ref_types(app: &WordApp) -> Vec<(String, String)> {
    let mut types: Vec<(String, String)> = REF_TYPES.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
    for l in caption_labels(app) {
        if !BUILT_IN_LABELS.contains(&l.as_str()) {
            types.push((format!("label:{l}"), l));
        }
    }
    types
}

/// The document's targets of reference type `to`.
fn targets_of(app: &WordApp, to: &str) -> Vec<String> {
    wordcraft_engine::cmd::citations::ref_targets(&app.session, to).into_iter().map(|(t, _)| t).collect()
}

/// Opens the Cross-reference dialog on type `to`. The dialog stays beside the document, so its
/// types and targets are read again before every pass.
pub(super) fn open(app: &mut WordApp, parent: Option<DialogId>, to: &str) -> DialogId {
    open_form_with(app, parent, CrossRefForm::read(app, to), DialogHooks::prepare_only(refresh))
}

/// Brings the open dialog's types and targets up to date with the document.
fn refresh(app: &mut WordApp, id: DialogId) {
    let Some(to) = app.dialog_host.get_by_id::<FormHost<CrossRefForm>>(id).map(|h| h.form().to.clone()) else { return };
    let types = ref_types(app);
    let targets = targets_of(app, &to);
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<CrossRefForm>>(id) {
        host.form_mut().refresh(types, targets);
    }
}

/// The Cross-reference dialog's fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CrossRefForm {
    /// `heading`, `bookmark`, `equation`, `figure`, `table`, or `label:<Name>` for a caption label
    /// of the person's own.
    pub to: String,
    /// `text`, `page`, `number` or `aboveBelow`.
    pub show: String,
    /// The document's targets of this type.
    pub targets: Vec<String>,
    pub selected: Option<usize>,
    /// The reference types on offer: (value, label).
    pub types: Vec<(String, String)>,
}

impl CrossRefForm {
    pub fn read(app: &WordApp, to: &str) -> CrossRefForm {
        let mut form = CrossRefForm { to: to.to_string(), show: "text".into(), targets: Vec::new(), selected: None, types: Vec::new() };
        form.refresh(ref_types(app), targets_of(app, to));
        form
    }

    /// Takes the document's current types and the targets of this type. The target picked stays
    /// picked while it is still listed; otherwise the first one is.
    fn refresh(&mut self, types: Vec<(String, String)>, targets: Vec<String>) {
        if self.types != types {
            self.types = types;
        }
        if self.targets != targets {
            let kept = self.selected.and_then(|i| self.targets.get(i)).cloned();
            self.selected = kept.and_then(|t| targets.iter().position(|x| *x == t));
            self.targets = targets;
        }
        if self.selected.is_none() && !self.targets.is_empty() {
            self.selected = Some(0);
        }
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

impl WordForm for CrossRefForm {
    fn key(&self) -> &'static str {
        "crossReference"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Cross-reference").to_string())
    }

    fn ok_label(&self) -> Option<Label> {
        Some(Label::from(tl!("Insert").to_string()))
    }

    /// The dialog stays beside the document so several references can go in one after another.
    fn modeless(&self) -> bool {
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        self.params().map(|p| ("insert.crossReference".to_string(), p)).into_iter().collect()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let type_title = Label::from(tl!("Reference type:").to_string());
        let types: Vec<Entry<String>> = self.types.iter().map(|(v, l)| Entry::new(v.clone(), Label::from(tl!(l).to_string()))).collect();
        let picked = Field::new(type_title.clone())
            .show(ui, |ui, _| Select::new(type_title.clone(), Bind::given(Mixed::Same(self.to.clone())), &types).show(ui).value().cloned())
            .inner
            .flatten();
        if let Some(to) = picked
            && to != self.to
        {
            // The pass before the next one lists the new type's targets (`refresh`).
            self.to = to;
            self.targets.clear();
            self.selected = None;
        }
        let show_title = Label::from(tl!("Insert reference to:").to_string());
        let shows: Vec<Entry<String>> = self.shows().iter().map(|(v, l)| Entry::new(v.to_string(), Label::from(tl!(l).to_string()))).collect();
        let picked = Field::new(show_title.clone())
            .show(ui, |ui, _| Select::new(show_title.clone(), Bind::given(Mixed::Same(self.show.clone())), &shows).show(ui).value().cloned())
            .inner
            .flatten();
        if let Some(show) = picked {
            self.show = show;
        }
        let which: Vec<usize> = (0..self.targets.len()).collect();
        let source = keyed(&which, |i: &usize| *i);
        let mut selection = self.selected.map_or_else(|| Selection::new(Vec::new(), None, None), Selection::single);
        let targets = &self.targets;
        let _ = List::new(Label::from(tl!("For which item:").to_string()))
            .max_rows(8)
            .select(SelectMode::Single)
            // The dialog body scrolls, and a collection owns its own scroll: the list shows this many rows
            // and scrolls the rest itself.
            .max_rows(10)
            .empty(Label::from(tl!("Nothing of this type in the document.").to_string()))
            .selection(Bind::bound(&mut selection))
            .show_items(ui, &source, &mut |i, _| ListRow::new(Label::from(targets.get(*i).cloned().unwrap_or_default())));
        self.selected = selection.items.first().copied();
        self.params().is_some()
    }
}
