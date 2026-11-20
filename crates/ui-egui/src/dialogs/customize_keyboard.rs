//! Tools › Customize Keyboard (#368; File › Options › Keyboard shortcuts, command search): the framework's
//! shortcuts dialog in its command-list layout (Categories beside Commands, the command's keys, the New Shortcut
//! recorder, the holder line, Assign and Remove). Each edit applies at once, so there is no Cancel; the edits
//! reach the session's keymap in `settle`, and the hub saves the preferences. Every change a script makes goes
//! through `tools.customizeKeyboard`, so agents get the same result without the dialog.

use craft::keymap::{Binding, CommandDef, Keymap, Overrides};
use craft::menu::shortcuts::{DialogState, Layout, Options, Outcome, ShortcutsDialog};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use egui::{Context, Ui};
use serde_json::{Value, json};

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

/// The name the dialog opens under (dg.A2) and the key of its id.
const KEY: &str = "customizeKeyboard";

/// The fields automation may set; the rest of the dialog's report is read only.
const SETTABLE: [&str; 6] = ["search", "area", "command", "pressed", "assign", "remove"];

/// The dialog: the keymap it edits, the framework dialog's state and what `settle` still has to copy to the session.
pub(crate) struct CustomizeKeyboardDialog {
    /// The board's keymap, cloned on the first pass (it needs the context's command board).
    keymap: Option<Keymap>,
    state: DialogState,
    /// Fields set by automation, applied on the next pass: the framework applies them through a `Ui`.
    sets: Vec<(String, Value)>,
    /// The overrides after the last edit, until `settle` has copied them to the session.
    pending: Option<Overrides>,
}

impl CustomizeKeyboardDialog {
    /// A dialog that has not shown yet.
    pub(crate) fn new() -> Self {
        CustomizeKeyboardDialog { keymap: None, state: DialogState::new(), sets: Vec::new(), pending: None }
    }

    /// The overrides to hand to `settle` now, once.
    fn take_pending(&mut self) -> Option<Overrides> {
        self.pending.take()
    }
}

/// Only combinations with Mod, Ctrl or Alt, or function keys, can be assigned from the dialog (plain letters and
/// Shift+letter type text).
fn assignable(binding: &Binding) -> bool {
    let chord = &binding.trigger.chord;
    chord.mods.cmd || chord.mods.ctrl || chord.mods.alt || chord.is_function_key()
}

/// The framework dialog's options: the command-list layout, sorted by label, with the assign rule above.
fn options() -> Options {
    Options::default().with(|o| {
        o.layout = Layout::CommandLists;
        o.sort_by_label = true;
        o.title = Some(tl!("Customize Keyboard").to_string());
        o.accept = Some(Box::new(|_: &CommandDef, binding: &Binding| {
            if assignable(binding) { Ok(()) } else { Err(tl!("Use a key combination with Ctrl, Alt or a function key.").to_string()) }
        }));
    })
}

impl StackDialog for CustomizeKeyboardDialog {
    /// The overrides when the dialog closed, so the last edits reach the session even in the pass it closes.
    type Result = Option<Overrides>;

    fn id(&self) -> DialogId {
        dialog_id(KEY)
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<Option<Overrides>>> {
        if self.keymap.is_none() {
            // PLAN: assumed `craft::ui::runtime::commands::keymap(&Context) -> Option<Arc<Keymap>>` (chrome-w6, ch.C22)
            self.keymap = craft::ui::runtime::commands::keymap(ui.ctx()).map(|board| (*board).clone());
        }
        let Some(keymap) = self.keymap.as_mut() else {
            // Without the command board there is no catalog to list.
            return Some(DialogResult::Cancel);
        };
        let options = options();
        // A frame can run two passes: the queue is taken once, so a second pass applies nothing twice.
        for (field, value) in std::mem::take(&mut self.sets) {
            // PLAN: a refused value is dropped; `StackDialog::set_json_field` has no `Ui` to answer with, so it validates the name only.
            let _ = ShortcutsDialog::new(keymap).options(&options).set_json_field(ui, &mut self.state, &field, &value);
        }
        let shown = ShortcutsDialog::new(keymap).options(&options).show(ui, &mut self.state);
        match shown.outcome {
            Some(Outcome::Changed) => {
                self.pending = Some(keymap.overrides().clone());
                None
            }
            Some(Outcome::Ok | Outcome::Cancel) => Some(DialogResult::Ok(Some(keymap.overrides().clone()))),
            _ if !self.state.is_open() => Some(DialogResult::Ok(Some(keymap.overrides().clone()))),
            _ => None,
        }
    }

    fn json_fields(&self) -> Option<Value> {
        let snapshot = self.state.snapshot(self.keymap.as_ref()?)?;
        Some(json!({
            "search": snapshot.query,
            "area": snapshot.area,
            "command": snapshot.selected.map(|command| command.key().to_string()),
            "pressed": snapshot.pressed,
            "holder": snapshot.holders.iter().map(|command| command.key().to_string()).collect::<Vec<_>>(),
        }))
    }

    fn set_json_field(&mut self, _ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        if field == "holder" {
            return Err(FieldRefusal::ReadOnly.message(field));
        }
        if !SETTABLE.contains(&field) {
            return Err(FieldRefusal::Unknown.message(field));
        }
        self.sets.push((field.to_owned(), value));
        Ok(())
    }
}

/// Copies the edits to the session's keymap, and drops the hook once the dialog has finished.
fn settle(app: &mut WordApp, _ctx: &Context, id: DialogId) -> Settle {
    let pending = app.dialog_host.get_mut_by_id::<CustomizeKeyboardDialog>(id).and_then(CustomizeKeyboardDialog::take_pending);
    if let Some(overrides) = pending {
        *app.session.keymap_mut() = overrides;
    }
    match app.dialog_host.take_by_id::<CustomizeKeyboardDialog>(id) {
        Some(result) => {
            if let Some(Some(overrides)) = result.ok() {
                *app.session.keymap_mut() = overrides;
            }
            Settle::Done
        }
        None => Settle::Keep,
    }
}

/// Opens the dialog (`customizeKeyboard`, dg.A2). It takes no parameters; a second open replaces the first.
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    Some(open_with(app, parent, CustomizeKeyboardDialog::new(), DialogHooks { prepare: None, settle }))
}

#[cfg(test)]
mod tests {
    use craft::keymap::Binding;
    use serde_json::json;
    use wordcraft_engine::Session;

    use crate::{Services, WordApp};

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    /// The bindings the session's overrides give `id`; `None` when the command has no override.
    fn chords(app: &WordApp, id: &str) -> Option<Vec<Binding>> {
        app.session.keymap().get(id).map(|texts| texts.iter().map(|text| Binding::parse(text).unwrap()).collect())
    }

    /// #368: a chord assigned through `tools.customizeKeyboard` leaves its built-in holder and survives a restart
    /// (an app-level round trip of the preferences; the engine side is shell-w7's `user_keys_override_and_remove_built_ins`).
    #[test]
    fn customize_keyboard_opens_and_custom_keys_persist() {
        let mut a = app();
        a.run("tools.customizeKeyboard", json!({"assign": {"command": "format.italic", "key": "Mod+B"}})).unwrap();
        assert_eq!(chords(&a, "format.bold"), Some(Vec::new()), "bold lost its built-in chord");
        let italic = chords(&a, "format.italic").expect("italic has an override");
        // The framework keymap adds a chord to the command's keys (a command may hold several, as in Word), so Italic
        // keeps its own chord and holds the one taken from Bold besides it.
        assert!(italic.contains(&Binding::parse("Mod+B").unwrap()), "italic holds the chord taken from bold");
        assert!(italic.contains(&Binding::parse("Mod+I").unwrap()), "and keeps its own");
        assert_eq!(italic.len(), 2, "and nothing else");
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(chords(&b, "format.bold"), Some(Vec::new()));
        assert_eq!(chords(&b, "format.italic"), Some(italic));
    }
}
