//! Tools › Customize Keyboard (#368; File › Options › Keyboard shortcuts, command search): the framework's
//! shortcuts dialog in its command-list layout (Categories beside Commands, the command's keys, the New Shortcut
//! recorder, the holder line, Assign and Remove). Each edit applies at once, so there is no Cancel; the edits
//! reach the session's keymap in `settle`, and the hub saves the preferences. Every change a script makes goes
//! through `tools.customizeKeyboard`, so agents get the same result without the dialog.

use craft::keymap::{Binding, Chord, CommandDef, Keymap, Overrides, Platform, Scope};
use craft::menu::shortcuts::Area;
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
    /// The document's styles (id, name) when the dialog opened: each is listed under Styles, so it can take a key
    /// (#3910), as in Word's Customize Keyboard.
    styles: Vec<(String, String)>,
}

impl CustomizeKeyboardDialog {
    /// A dialog that has not shown yet.
    pub(crate) fn new() -> Self {
        CustomizeKeyboardDialog { keymap: None, state: DialogState::new(), sets: Vec::new(), pending: None, styles: Vec::new() }
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

/// The board's keymap with an entry for each style in `styles` that has none yet (the board lists only the styles
/// that have keys), so the Styles category lists every style of the document.
fn with_styles(board: &Keymap, styles: &[(String, String)]) -> Keymap {
    let mut catalog = board.catalog().to_vec();
    let before = catalog.len();
    for (id, name) in styles {
        let command = wordcraft_engine::keymap::style_command(id);
        if board.bindable(&command).is_none() {
            catalog.push(CommandDef::new(command, name.clone(), Scope::app()));
        }
    }
    if catalog.len() == before {
        return board.clone();
    }
    // As the engine's keymap: off macOS, Alt+Q is the interface's command search key.
    let reserved: Vec<Chord> = if board.platform() == Platform::Mac { Vec::new() } else { Chord::parse("Alt+Q").ok().into_iter().collect() };
    Keymap::new(catalog, board.scopes().clone(), board.platform())
        .with_typing_rule(board.typing_rule())
        .with_reserved(reserved)
        .with_overrides(board.overrides().clone())
}

/// Whether a catalog entry is a style's key (`styles.apply` with the style).
fn is_style(def: &CommandDef) -> bool {
    wordcraft_engine::keymap::style_of(&def.command).is_some()
}

/// The framework dialog's options: the command-list layout, sorted by label, with the assign rule above, and a
/// Styles category.
fn options() -> Options {
    Options::default().with(|o| {
        o.areas.push(Area::new("styles", tl!("Styles"), is_style));
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
            self.keymap = craft::ui::runtime::commands::keymap(ui.ctx()).map(|board| with_styles(&board, &self.styles));
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

/// Opens the dialog (`customizeKeyboard`, dg.A2); a second open replaces the first. `params["style"]` (a style's
/// name or id, from Modify Style's Shortcut Key…) opens it on that style in the Styles category.
pub(crate) fn open(app: &mut WordApp, _name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let mut dialog = CustomizeKeyboardDialog::new();
    dialog.styles = app.session.doc.styles.styles.iter().map(|style| (style.id.clone(), style.name.clone())).collect();
    if let Some(style) = params.get("style").and_then(Value::as_str).and_then(|name| app.session.doc.styles.find(name)) {
        dialog.sets.push(("area".to_owned(), json!("styles")));
        dialog.sets.push(("command".to_owned(), json!(wordcraft_engine::keymap::style_command(&style.id).key().to_string())));
    }
    Some(open_with(app, parent, dialog, DialogHooks { prepare: None, settle }))
}

#[cfg(test)]
mod tests {
    use craft::keymap::{Binding, CommandRef};
    use craft::ui::dialog::dialog_id;
    use egui::accesskit::Role;
    use egui_kittest::kittest::{By, NodeT, Queryable};
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::KEY;
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

    /// #1905: a command with no shortcuts shows `None` in the Current shortcuts list, whereas a
    /// command with a built-in shortcut shows its binding instead of `None`.
    #[test]
    fn keys_list_without_shortcuts_says_none() {
        let mut a = app();
        super::open(&mut a, KEY, &json!({}), None).expect("dialog opens");
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
        h.run_steps(2);

        let keymap = craft::ui::runtime::commands::keymap(&h.ctx).expect("command board installs a keymap");
        let unbound_id = h
            .state()
            .session
            .registry
            .all()
            .iter()
            .find(|spec| {
                let cmd = CommandRef::new(spec.id);
                keymap.bindable(&cmd).is_some() && keymap.bindings(&cmd).is_empty()
            })
            .map(|spec| spec.id.to_string())
            .expect("a bindable command with no built-in shortcut");
        assert!(!keymap.bindings(&CommandRef::new("format.bold")).is_empty(), "format.bold has a built-in shortcut");

        let dlg_id = dialog_id(KEY);
        let ctx = h.ctx.clone();
        let select = |h: &mut egui_kittest::Harness<'_, WordApp>, id: &str| {
            h.state_mut().dialog_host.set_json_field(&ctx, dlg_id, "command", json!(id)).expect("command field accepts id");
            h.run_steps(3);
            let selected = h
                .state()
                .dialog_host
                .open_dialogs()
                .into_iter()
                .find(|d| d.id == dlg_id)
                .and_then(|d| d.fields)
                .and_then(|fields| fields.get("command").and_then(|v| v.as_str()).map(str::to_owned));
            assert_eq!(selected.as_deref(), Some(id), "dialog selected {id}");
        };
        let keys_list_labels = |h: &egui_kittest::Harness<'_, WordApp>| -> Vec<String> {
            let list = h.get_by_role_and_label(Role::List, "Current shortcuts:");
            let list_id = list.accesskit_node().id();
            let rect = list.rect();
            h.query_all(By::new())
                .filter(|n| n.accesskit_node().id() != list_id && !n.accesskit_node().is_hidden() && n.accesskit_node().bounding_box().is_some())
                .filter(|n| rect.contains_rect(n.rect()))
                .filter_map(|n| n.accesskit_node().label())
                .collect()
        };

        // Positive contrast: a bound command lists its shortcut and does not show the empty `None` placeholder.
        select(&mut h, "format.bold");
        let bound_labels = keys_list_labels(&h);
        assert!(!bound_labels.is_empty(), "bound command lists its shortcut");
        assert!(!bound_labels.iter().any(|t| t == "None"), "bound command keys list does not say None: {bound_labels:?}");

        // Target behavior: an unbound command shows `None` inside the Current shortcuts list.
        select(&mut h, &unbound_id);
        let unbound_labels = keys_list_labels(&h);
        assert!(unbound_labels.iter().any(|t| t == "None"), "empty keys list says None: {unbound_labels:?}");
    }

    /// The selected row and Show area of the open dialog.
    fn selected(h: &egui_kittest::Harness<'_, WordApp>) -> (Option<String>, Option<String>) {
        let fields = h.state().dialog_host.open_dialogs().into_iter().find(|d| d.id == dialog_id(KEY)).and_then(|d| d.fields).unwrap_or_default();
        let text = |name: &str| fields.get(name).and_then(|v| v.as_str()).map(str::to_owned);
        (text("command"), text("area"))
    }

    /// #3910: Modify Style's Shortcut Key… opens the dialog on that style in the Styles category, which lists every
    /// style of the document, also one with no key yet; a key given to a style is saved with the preferences.
    #[test]
    fn styles_category_lists_every_style_and_style_keys_persist() {
        let mut a = app();
        let heading2 = wordcraft_engine::keymap::style_command("Heading2").key().to_string();
        super::open(&mut a, KEY, &json!({"style": "Heading 2"}), None).expect("dialog opens");
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
        h.run_steps(3);
        assert_eq!(selected(&h), (Some(heading2.clone()), Some("styles".to_owned())));

        let mut a = app();
        a.run("tools.customizeKeyboard", json!({"assign": {"style": "Heading 2", "key": "Mod+Alt+Shift+H"}})).unwrap();
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        let keys = b.run("tools.customizeKeyboard", json!({"style": "Heading 2"})).unwrap();
        assert_eq!(keys["keys"].as_array().map(Vec::len), Some(1), "{keys}");
        assert_eq!(b.session.keymap().get(&heading2).map(|keys| keys.len()), Some(1));
    }
}
