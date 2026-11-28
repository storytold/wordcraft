//! Tools › Customize Keyboard (#368; File › Options › Keyboard shortcuts, command search): the framework's
//! shortcuts dialog in its command-list layout (Categories beside Commands, the command's keys, the New Shortcut
//! recorder, the holder line, Assign and Remove). Each edit applies at once, so there is no Cancel; the edits
//! reach the session's keymap in `settle`, and the hub saves the preferences. Every change a script makes goes
//! through `tools.customizeKeyboard`, so agents get the same result without the dialog.

use craft::keymap::{Binding, CommandDef, Keymap, Overrides};
use craft::menu::shortcuts::{Area, DialogState, Layout, Options, Outcome, ShortcutsDialog};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use egui::{Context, Ui};
use serde_json::{Value, json};
use wordcraft_engine::ribbon::{CONTEXTUAL_TABS, TABS};

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

/// The ribbon tab a command lives on, in English (the first part of its path).
fn tab(def: &CommandDef) -> &str {
    def.paths.first().and_then(|path| path.first()).map_or("", String::as_str)
}

/// Tabs that come up with the selection or a view, besides the engine's contextual ones; they share one Categories entry.
const EXTRA_CONTEXT_TABS: [&str; 3] = ["Header & Footer", "Outlining", "Reading"];

fn in_tab<const N: usize>(def: &CommandDef) -> bool {
    tab(def) == TABS[N]
}

fn in_context_tab(def: &CommandDef) -> bool {
    CONTEXTUAL_TABS.contains(&tab(def)) || EXTRA_CONTEXT_TABS.contains(&tab(def))
}

fn in_no_tab(def: &CommandDef) -> bool {
    !TABS.contains(&tab(def)) && !in_context_tab(def)
}

/// One filter per entry of `TABS` (the array's length makes the compiler check it).
const TAB_FILTERS: [fn(&CommandDef) -> bool; TABS.len()] = [
    in_tab::<0>,
    in_tab::<1>,
    in_tab::<2>,
    in_tab::<3>,
    in_tab::<4>,
    in_tab::<5>,
    in_tab::<6>,
    in_tab::<7>,
    in_tab::<8>,
    in_tab::<9>,
    in_tab::<10>,
    in_tab::<11>,
    in_tab::<12>,
];

/// Categories as Word lists them: the ribbon tabs (not the framework's menu and panel ids), the contextual tabs, the
/// rest, then All Commands.
fn areas() -> Vec<Area> {
    let mut areas: Vec<Area> = TABS.iter().copied().zip(TAB_FILTERS).map(|(name, rows)| Area::new(name, tl!(name), rows)).collect();
    areas.push(Area::new("Contextual Tabs", tl!("Contextual Tabs"), in_context_tab));
    areas.push(Area::new("Other", tl!("Other"), in_no_tab));
    areas.push(Area::new("All Commands", tl!("All Commands"), |_| true));
    areas
}

/// The framework dialog's options: the command-list layout, sorted by label, with the assign rule above.
fn options() -> Options {
    Options::default().with(|o| {
        o.layout = Layout::CommandLists;
        o.sort_by_label = true;
        o.own_areas = true;
        o.size = Some(egui::vec2(720.0, 700.0));
        o.areas = areas();
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

    /// #7971: Categories are the ribbon tabs a command is on, not menu or panel ids; a command with no tab is under Other,
    /// and every command is under All Commands.
    #[test]
    fn categories_are_ribbon_tabs() {
        use craft::keymap::{CommandDef, Scope};
        let on = |path: &[&str]| {
            let mut def = CommandDef::new(CommandRef::new("x"), "X", Scope::app());
            if !path.is_empty() {
                def = def.with_path(path.iter().map(|part| part.to_string()).collect());
            }
            def
        };
        let areas = super::areas();
        let listing = |def: &CommandDef| areas.iter().filter(|a| (a.rows)(def)).map(|a| a.id).collect::<Vec<_>>();
        assert_eq!(listing(&on(&["Home", "Font"])), ["Home", "All Commands"]);
        assert_eq!(listing(&on(&["Help", "Support"])), ["Help", "All Commands"], "every ribbon tab has its entry");
        assert_eq!(listing(&on(&["Table Design", "Styles"])), ["Contextual Tabs", "All Commands"]);
        assert_eq!(listing(&on(&[])), ["Other", "All Commands"]);
        assert_eq!(areas.last().map(|a| a.id), Some("All Commands"), "All Commands comes last");
    }

    /// #7971: under another language, and for the SmartArt tab that shows as "Diagram Design", commands still land
    /// under their tab (the catalog keeps English paths; only the display is translated).
    #[test]
    fn commands_stay_under_their_tab_in_another_language() {
        let ctx = egui::Context::default();
        crate::i18n::fixture_host(&ctx, "x-fixture", "\tHome\tHome-FX\n\tFont\tFont-FX\n\tDiagram Design\tDiagram-FX\n");
        let mut a = app();
        let catalog = a.catalog().expect("catalog");
        let areas = super::areas();
        let under = |id: &str, area: &str| {
            let def = catalog.defs().iter().find(|def| def.command == CommandRef::new(id)).unwrap_or_else(|| panic!("{id} in catalog"));
            areas.iter().any(|a| a.id == area && (a.rows)(def))
        };
        assert!(under("format.bold", "Home"), "Bold is under Home in a translated interface");
        assert!(!under("format.bold", "Other"));
        let smart = a.session.registry.all().iter().find(|spec| spec.location.starts_with("SmartArt Design")).expect("a SmartArt command");
        assert!(under(smart.id, "Contextual Tabs") && !under(smart.id, "Other"), "{} is under the contextual tabs", smart.id);
    }

    /// #7971: the board's catalog files each engine command under its ribbon tab.
    #[test]
    fn catalog_paths_follow_the_ribbon_location() {
        let mut a = app();
        let catalog = a.catalog().expect("catalog");
        let bold = catalog.defs().iter().find(|def| def.command == CommandRef::new("format.bold")).expect("format.bold");
        assert_eq!(bold.paths.first().and_then(|p| p.first()).map(String::as_str), Some("Home"));
    }
}
