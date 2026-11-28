//! View › Macros (#4327): the macros recorded or defined this session, with Run and Delete beside the list and
//! Record below the name box. While a recording is running the box offers Stop Recording instead. The list is read
//! from the session, not from `tools.macros`, which would ask to open this dialog again. Run and Record close the
//! dialog; Delete leaves it open.

use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::patterns::form_dialogs::{ActionEnabled, ListManager, ListManagerAction, ListManagerChoice, ListManagerEntry, ListManagerWords};
use craft::ui::runtime::env::{Env, Label};
use egui::{Context, Ui};
use serde_json::{Value, json};

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

/// The name the dialog opens under (dg.A2) and the key of its id.
const KEY: &str = "macros";

/// The Macros dialog: the session's macros as list entries, and the presses `settle` still has to run.
pub(crate) struct MacrosDialog {
    /// The macros, one entry each; the id is the exact name, so a macro named `" Pad "` runs by that name.
    entries: Vec<ListManagerEntry>,
    /// A recording is running: Run is disabled and Record reads Stop Recording.
    recording: bool,
    /// The name the box opened with, then the last name a press carried.
    name: String,
    /// Presses that keep the dialog open (Delete), run by `settle`.
    requests: Vec<ListManagerChoice>,
    /// The last press filed in `requests`, so the pass that closes the dialog does not run it a second time.
    filed: Option<ListManagerChoice>,
}

impl MacrosDialog {
    /// A dialog whose name box opens on `name`.
    pub(crate) fn new(name: String) -> Self {
        MacrosDialog { entries: Vec::new(), recording: false, name, requests: Vec::new(), filed: None }
    }

    /// Copies the session's macros and recording state in; called before every host run.
    fn refresh(&mut self, names: Vec<String>, recording: bool) {
        // The list matches the name box's trimmed text against an entry's name, so the name shown is trimmed and the
        // id keeps the exact name the press runs or deletes.
        self.entries = names.into_iter().map(|name| ListManagerEntry::new(name.clone(), name.trim().to_string())).collect();
        self.recording = recording;
    }
}

impl StackDialog for MacrosDialog {
    /// The press that closed the dialog; `None` for Close.
    type Result = Option<ListManagerChoice>;

    fn id(&self) -> DialogId {
        dialog_id(KEY)
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<Option<ListManagerChoice>>> {
        let words = Env::of(ui).words::<ListManagerWords>();
        let run = if self.recording {
            ListManagerAction::new("run", words.run.clone(), ActionEnabled::Never).reason(tl!("Stop recording first").to_string())
        } else {
            ListManagerAction::run(&words)
        };
        let (record_label, record_enabled) =
            if self.recording { (tl!("Stop Recording"), ActionEnabled::Always) } else { (tl!("Record…"), ActionEnabled::OnValidName) };
        let actions = vec![run, ListManagerAction::delete(&words), ListManagerAction::new("record", record_label.to_string(), record_enabled)];
        let dialog = ListManager::new(tl!("Macros").to_string(), tl!("Macro name:").to_string(), self.entries.clone(), actions)
            .empty(Label::from(tl!("There are no macros yet. Record one to add it here.").to_string()))
            .cancel_label(tl!("Close").to_string())
            // PLAN: assumed `ListManager::name_box(self, Bind<'a, String>) -> Self` (dialogs-w5, S-L310)
            .name_box(Bind::bound(&mut self.name));
        let out = dialog.show(ui);
        match (out.closed, out.choice) {
            // A press already filed in an earlier pass is not run again when the closing pass repeats it (Record
            // would start and then stop).
            (Some(_), choice) => Some(DialogResult::Ok(choice.filter(|choice| self.filed.as_ref() != Some(choice)))),
            (None, Some(choice)) => {
                self.filed = Some(choice.clone());
                self.requests.push(choice);
                None
            }
            (None, None) => None,
        }
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"name": self.name}))
    }

    fn set_json_field(&mut self, _ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        match (field, value.as_str()) {
            ("name", Some(name)) => {
                self.name = name.to_owned();
                Ok(())
            }
            ("name", None) => Err(FieldRefusal::Rejects(&value).message(field)),
            _ => Err(FieldRefusal::Unknown.message(field)),
        }
    }
}

/// Runs what a press asked for; a failure goes to the status bar.
fn run_press(app: &mut WordApp, choice: &ListManagerChoice) {
    let result = match (choice.action.as_str(), choice.entry.as_deref()) {
        ("run", Some(name)) => app.run("tools.macros", json!({"run": name})),
        ("delete", Some(name)) => app.run("tools.macros", json!({"delete": name})),
        // Stopping takes no name; starting records under the one typed.
        ("record", _) if app.session.recording.is_some() => app.run("tools.recordMacro", json!({})),
        ("record", _) => app.run("tools.recordMacro", json!({"name": choice.name.trim()})),
        _ => return,
    };
    if let Err(error) = result {
        app.status(error);
    }
}

/// Before the host runs: the session's macros and recording state go into the dialog.
fn prepare(app: &mut WordApp, id: DialogId) {
    let names: Vec<String> = app.session.macros.keys().cloned().collect();
    let recording = app.session.recording.is_some();
    if let Some(dialog) = app.dialog_host.get_mut_by_id::<MacrosDialog>(id) {
        dialog.refresh(names, recording);
    }
}

/// After the host runs: Delete presses, then the press that closed the dialog. True once the dialog is gone.
fn settle(app: &mut WordApp, _ctx: &Context, id: DialogId) -> Settle {
    let presses = app.dialog_host.get_mut_by_id::<MacrosDialog>(id).map(|dialog| std::mem::take(&mut dialog.requests)).unwrap_or_default();
    for press in &presses {
        run_press(app, press);
    }
    match app.dialog_host.take_by_id::<MacrosDialog>(id) {
        Some(result) => {
            if let Some(Some(press)) = result.ok() {
                run_press(app, &press);
            }
            Settle::Done
        }
        None => Settle::Keep,
    }
}

/// Opens the dialog (`macros`, dg.A2); `params.name` fills the name box.
pub(crate) fn open(app: &mut WordApp, _name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
    // Opened again while it shows (Delete leaves it open): the open one takes the name and stays. A second dialog under
    // the same id would cancel the first, whose settle then drops the hooks the id shares, so the new one's Record or
    // Run would never run.
    let open = dialog_id(KEY);
    if let Some(dialog) = app.dialog_host.get_mut_by_id::<MacrosDialog>(open) {
        dialog.name = name;
        prepare(app, open);
        return Some(open);
    }
    let id = open_with(app, parent, MacrosDialog::new(name), DialogHooks { prepare: Some(prepare), settle });
    // The first pass shows the session's macros, not an empty list.
    prepare(app, id);
    Some(id)
}
