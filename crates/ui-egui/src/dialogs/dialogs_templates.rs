//! Templates and Add-ins (View › Templates) and the style Organizer (#377). Each button ends in
//! one command (`tools.attachTemplate`, `tools.detachTemplate`, `tools.linkStyles`,
//! `styles.copyFrom`, `styles.delete`, `styles.rename`), so agents get the same result without
//! the dialogs.

use craft::behavior::{EditEvent, Selection, SelectionChange, item_source::keyed};
use craft::host::PickRequest;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::{FormCx, PathRow};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::{DialogRole, FooterButton};
use egui::{Context, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::host::{Asks, DialogHooks, Filed, Settle, WordForm, open_form, open_with};
use super::styles::{action, label};
use crate::WordApp;

const ORGANIZER_KEY: &str = "organizer";

/// The Templates and Add-ins dialog's fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TemplatesForm {
    pub path: String,
    /// "Automatically update document styles" (`w:linkStyles`).
    pub link: bool,
    /// The attached template when the dialog opened.
    #[serde(skip)]
    was_path: String,
    /// The link setting when the dialog opened.
    #[serde(skip)]
    was_link: bool,
}

/// Why Browse is off on a host whose pickers don't answer back to the dialog (the web's picker opens what it picks instead).
fn browse_reason(ui: &Ui) -> Option<Label> {
    let can_pick = Env::of(ui).view().host().is_some_and(|host| host.files().can_pick());
    (!can_pick).then(|| label(tl!("Pick a template on the desktop app")))
}

/// A template path row over `value`, with Browse switched off for `reason` when there is one.
fn template_row<'a>(name: Label, value: Bind<'a, String>, reason: Option<Label>) -> PathRow<'a> {
    let row = PathRow::new(name, value).pick(template_pick());
    match reason {
        Some(reason) => row.browse_reason(reason),
        None => row,
    }
}

fn template_pick() -> PickRequest {
    PickRequest::open("template")
}

impl TemplatesForm {
    pub fn read(app: &mut WordApp) -> TemplatesForm {
        let s = &app.session.doc.settings;
        let path = s.attached_template.clone().unwrap_or_default();
        TemplatesForm { was_path: path.clone(), path, link: s.link_styles, was_link: s.link_styles }
    }

    /// OK: attach (updating the styles now when they follow the template), detach, or just switch
    /// the automatic update.
    // PLAN: the sheet gives `commands(&self, app)`; the `WordForm` contract has `commands(&self)` and the form carries what it needs.
    fn commands(&self) -> Vec<(&'static str, Value)> {
        let path = self.path.trim();
        if path.is_empty() {
            if !self.was_path.trim().is_empty() {
                return vec![("tools.detachTemplate", json!({}))];
            }
            if self.link != self.was_link {
                return vec![("tools.linkStyles", json!({"value": self.link}))];
            }
            return Vec::new();
        }
        if path != self.was_path || self.link != self.was_link || self.link {
            return vec![("tools.attachTemplate", json!({"path": path, "linkStyles": self.link, "update": self.link}))];
        }
        Vec::new()
    }
}

impl WordForm for TemplatesForm {
    fn key(&self) -> &'static str {
        "templates"
    }

    fn title(&self) -> Label {
        label(tl!("Templates and Add-ins"))
    }

    fn body(&mut self, ui: &mut Ui, _: &mut FormCx<'_, '_>, _: &mut Vec<Asks>) -> bool {
        let reason = browse_reason(ui);
        let picked = Field::new(label(tl!("Document template:")))
            .show(ui, |ui, _| template_row(label(tl!("Document template:")), Bind::bound(&mut self.path), reason.clone()).show(ui).picked)
            .inner
            .flatten();
        if let Some(path) = picked {
            self.path = path;
        }
        Checkbox::new(Bind::bound(&mut self.link)).label(label(tl!("Automatically update document styles"))).show(ui);
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        TemplatesForm::commands(self).into_iter().map(|(id, params)| (id.to_string(), params)).collect()
    }

    fn extra_buttons(&self) -> Vec<FooterButton> {
        vec![FooterButton::new(label(tl!("Organizer…")), DialogRole::Other).stays_open(true)]
    }

    fn applied(&mut self, app: &mut WordApp, filed: Filed, _outcome: Result<&[Value], &str>) {
        if matches!(filed, Filed::Extra(0)) {
            open_organizer(app, self.path.trim(), Some(dialog_id(self.key())));
        }
    }
}

/// Opens Templates and Add-ins over `parent`.
pub(crate) fn open_templates(app: &mut WordApp, parent: Option<DialogId>) -> DialogId {
    let form = TemplatesForm::read(app);
    open_form(app, parent, form)
}

/// Opens the dialog `name` (`templates` or `organizer`); `None` for another name.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "templates" => Some(open_templates(app, parent)),
        "organizer" => Some(open_organizer(app, "", parent)),
        _ => None,
    }
}

/// What the Organizer asks the app to do.
enum Request {
    Load(String),
    Copy,
    Delete(String),
    Rename { style: String, name: String },
}

/// The Organizer: this document's styles on the left (read live), another document's or template's on the right.
pub(crate) struct OrganizerDialog {
    id: DialogId,
    /// The other file (empty: none open).
    file: String,
    /// Its styles: (name, type).
    other: Vec<(String, String)>,
    /// This document's styles, refreshed by `prepare`.
    mine: Vec<Value>,
    /// Selected style id in this document.
    left: String,
    /// Selected style name in the other file.
    right: String,
    message: String,
    requests: Vec<Request>,
}

fn glyph(ty: &str) -> &'static str {
    match ty {
        "character" => "a",
        // Plain characters: the interface fonts have no glyph for the box-plus and three-bar symbols.
        "table" => "#",
        "numbering" => "1.",
        _ => "¶",
    }
}

fn text_of(x: &Value, k: &str) -> String {
    x.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Opens the Organizer with `file` (the typed or attached template) on the right, over `parent` when it has one.
pub(crate) fn open_organizer(app: &mut WordApp, file: &str, parent: Option<DialogId>) -> DialogId {
    let mut dialog = OrganizerDialog::new(app, file);
    if !file.is_empty() {
        dialog.load(app, file);
    }
    open_with(app, parent, dialog, DialogHooks { prepare: Some(organizer_prepare), settle: organizer_settle })
}

impl OrganizerDialog {
    fn new(app: &mut WordApp, file: &str) -> OrganizerDialog {
        let file = if file.is_empty() { app.session.doc.settings.attached_template.clone().unwrap_or_default() } else { file.to_string() };
        OrganizerDialog {
            id: dialog_id(ORGANIZER_KEY),
            file,
            other: Vec::new(),
            mine: Vec::new(),
            left: String::new(),
            right: String::new(),
            message: String::new(),
            requests: Vec::new(),
        }
    }

    /// The styles of the file at `path`: (name, type).
    fn read_other(app: &mut WordApp, path: &str) -> Result<Vec<(String, String)>, String> {
        let v = app.session.run("styles.organizer", &json!({"path": path})).map_err(|e| e.to_string())?;
        let list = v.get("file").and_then(|f| f.get("styles")).and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(list.iter().map(|x| (text_of(x, "name"), text_of(x, "type"))).collect())
    }

    fn apply_loaded(&mut self, path: &str, loaded: Result<Vec<(String, String)>, String>) {
        match loaded {
            Ok(other) => {
                self.other = other;
                self.file = path.to_string();
                self.right.clear();
                self.message.clear();
            }
            Err(e) => self.message = e,
        }
    }

    fn load(&mut self, app: &mut WordApp, path: &str) {
        let loaded = Self::read_other(app, path);
        self.apply_loaded(path, loaded);
    }

    /// The command Copy runs: the selected style of the other file into this document.
    fn copy_command(&self) -> (&'static str, Value) {
        ("styles.copyFrom", json!({"path": self.file, "styles": [self.right]}))
    }

    fn left_builtin(&self) -> bool {
        self.mine.iter().find(|x| text_of(x, "id") == self.left).is_none_or(|x| x.get("builtIn").and_then(Value::as_bool).unwrap_or(true))
    }

    fn body(&mut self, ui: &mut Ui) {
        let reason = browse_reason(ui);
        Row::new().show(ui, |ui, _| {
            let mut selection = Selection::single(self.left.clone());
            let mine = &self.mine;
            let source = keyed(mine, |st| text_of(st, "id"));
            let out = List::new(label(tl!("Document")))
                .select(SelectMode::Single)
                .selection(Bind::bound(&mut selection))
                .rename(true)
                .can_rename(|key: &String| {
                    mine.iter().find(|x| text_of(x, "id") == *key).is_some_and(|x| !x.get("builtIn").and_then(Value::as_bool).unwrap_or(true))
                })
                .id_salt("mine")
                .show_items(ui, &source, &mut |_, icx| {
                    let Some(st) = mine.get(icx.at) else { return ListRow::spacer() };
                    ListRow::new(label(text_of(st, "name"))).lead_text(label(glyph(&text_of(st, "type"))), label("¶"))
                });
            for event in out.events {
                match event {
                    CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                        if let Some(key) = keys.last() {
                            self.left = key.clone();
                        }
                    }
                    CollectionEvent::Rename(EditEvent::Commit { after, .. }) => {
                        self.requests.push(Request::Rename { style: after.key, name: after.name });
                    }
                    _ => {}
                }
            }
            if action(ui, "copy", label(tl!("Copy")), !self.right.is_empty(), None) {
                self.requests.push(Request::Copy);
            }
            if action(ui, "delete", label(tl!("Delete")), !self.left_builtin(), None) {
                self.requests.push(Request::Delete(self.left.clone()));
            }
            let name = std::path::Path::new(&self.file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "—".into());
            Field::new(label(name)).show(ui, |ui, _| {
                let mut selection = Selection::single(self.right.clone());
                let other = &self.other;
                let source = keyed(other, |(n, _)| n.clone());
                let out = List::new(label(tl!("Template")))
                    .select(SelectMode::Single)
                    .selection(Bind::bound(&mut selection))
                    .id_salt("other")
                    .show_items(ui, &source, &mut |_, icx| {
                        let Some((n, ty)) = other.get(icx.at) else { return ListRow::spacer() };
                        ListRow::new(label(n)).lead_text(label(glyph(ty)), label("¶"))
                    });
                for event in out.events {
                    if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                        && let Some(key) = keys.last()
                    {
                        self.right = key.clone();
                    }
                }
                let picked = template_row(label(tl!("Template")), Bind::bound(&mut self.file), reason.clone()).show(ui).picked;
                if let Some(path) = picked {
                    self.requests.push(Request::Load(path));
                }
            });
        });
        if !self.message.is_empty() {
            Notice::new(craft::theme::variant::Tone::Bad, label(&self.message)).show(ui);
        }
    }
}

impl StackDialog for OrganizerDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(ORGANIZER_KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let close = [FooterButton::new(label(tl!("Close")), DialogRole::Cancel)];
        let shown = Dialog::new(label(tl!("Organizer"))).buttons(close).show(ui, |ui, _| self.body(ui));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"file": self.file, "left": self.left, "right": self.right}))
    }
}

/// Reads this document's styles live, and keeps the left selection on one that exists.
fn organizer_prepare(app: &mut WordApp, id: DialogId) {
    let mine = wordcraft_engine::cmd::templates::style_list(&app.session.doc);
    let mine = mine.as_array().cloned().unwrap_or_default();
    if let Some(d) = app.dialog_host.get_mut_by_id::<OrganizerDialog>(id) {
        if !mine.iter().any(|x| text_of(x, "id") == d.left) {
            d.left.clear();
        }
        d.mine = mine;
    }
}

/// Runs what the Organizer asked for. True once the dialog is gone.
// PLAN: assumed `DialogHooks::settle` returns true when the dialog finished and its hooks can be dropped (dg.A1, dialogs-w13).
fn organizer_settle(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    if app.dialog_host.take_by_id::<OrganizerDialog>(id).is_some() {
        return Settle::Done;
    }
    let Some(d) = app.dialog_host.get_mut_by_id::<OrganizerDialog>(id) else { return Settle::Done };
    let requests = std::mem::take(&mut d.requests);
    for request in requests {
        let Some(d) = app.dialog_host.get_mut_by_id::<OrganizerDialog>(id) else { return Settle::Done };
        match request {
            Request::Load(path) => {
                let loaded = OrganizerDialog::read_other(app, &path);
                if let Some(d) = app.dialog_host.get_mut_by_id::<OrganizerDialog>(id) {
                    d.apply_loaded(&path, loaded);
                }
            }
            Request::Copy => {
                let (command, params) = d.copy_command();
                let r = app.run(command, params);
                set_message(app, id, r);
            }
            Request::Delete(style) => {
                let r = app.run("styles.delete", json!({"style": style}));
                if let Some(d) = app.dialog_host.get_mut_by_id::<OrganizerDialog>(id) {
                    d.left.clear();
                }
                set_message(app, id, r);
            }
            Request::Rename { style, name } => {
                let r = app.run("styles.rename", json!({"style": style, "name": name}));
                set_message(app, id, r);
            }
        }
    }
    Settle::Keep
}

fn set_message(app: &mut WordApp, id: DialogId, result: Result<Value, String>) {
    if let Some(d) = app.dialog_host.get_mut_by_id::<OrganizerDialog>(id) {
        d.message = result.err().unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use wordcraft_engine::Session;

    /// OK attaches the typed template; the Organizer copies a style from it into the document through
    /// the command its Copy button runs.
    #[test]
    fn templates_dialog_attaches_and_organizer_copies() {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("House.dotx");
        let mut t = wordcraft_doc::Document::new();
        std::sync::Arc::make_mut(&mut t.styles).upsert(wordcraft_doc::styles::Style {
            id: "Memo".into(),
            name: "Memo".into(),
            kind: wordcraft_doc::styles::StyleKind::Paragraph,
            ..Default::default()
        });
        wordcraft_engine::io::save_path(&path, &t).unwrap();
        let p = path.to_string_lossy().to_string();

        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut form = TemplatesForm::read(&mut a);
        form.path = p.clone();
        for (command, params) in form.commands() {
            a.run(command, params).unwrap();
        }
        assert_eq!(a.session.doc.settings.attached_template.as_deref(), Some(p.as_str()));

        // Agents and scripts get the state, never the dialog.
        assert_eq!(a.execute("tools.templates", json!({})).unwrap()["template"], json!(p));
        assert!(a.dialog_host.open_dialogs().is_empty());

        let id = open_organizer(&mut a, &p, None);
        {
            let org = a.dialog_host.get_mut_by_id::<OrganizerDialog>(id).unwrap();
            assert!(org.other.iter().any(|(n, _)| n == "Memo"), "{:?}", org.other);
            org.right = "Memo".into();
            org.requests.push(Request::Copy);
        }
        assert!(a.session.doc.styles.get("Memo").is_none());
        let settled = organizer_settle(&mut a, &egui::Context::default(), id);
        assert_eq!(settled, Settle::Keep, "the organizer stays open after a copy");
        assert!(a.session.doc.styles.get("Memo").is_some(), "the settler runs styles.copyFrom");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
