//! The app's side of the framework command runtime: the catalog of commands and their default
//! chords, the per-frame state (enabled, checked, hint) and the running of what the runtime fired.

use std::collections::HashSet;
use std::sync::Arc;

use craft::keymap::Overrides;
use craft::ui::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandInput, CommandRef, CommandState, CommandStates, FireSource, Scope, TypingRule,
};
use craft::ui::runtime::frame::FrameOutput;

use super::{Value, WordApp, json};

/// Whether command `id` can run now, and the engine's reason when it can't.
///
/// One rule for every surface: no document open (#1129) leaves only what opens one, Help and the
/// settings; engine commands answer for themselves; the interface commands that depend on state
/// (Zotero, link text box, crop) say so; every other `ui.*` command is enabled.
fn gate(app: &WordApp, id: &str) -> (bool, Option<&'static str>) {
    if !app.has_document() && !super::commands::works_without_document(id) {
        return (false, None);
    }
    match app.session.registry.get(id) {
        Some(_) => {
            let reason = app.session.why_disabled(id);
            (reason.is_none(), reason)
        }
        None if id.starts_with("ui.zotero.") => (app.zotero.busy.is_none() && !cfg!(target_arch = "wasm32"), None),
        None if id == "ui.linkTextBox" => (command_enabled(app, "shape.link"), None),
        None if id == "ui.crop" => (crate::crop::can_toggle(app), None),
        None => (id.starts_with("ui."), None),
    }
}

/// Whether a command exists and is enabled.
pub(crate) fn command_enabled(app: &WordApp, id: &str) -> bool {
    gate(app, id).0
}

/// The interface commands whose enabled state depends on app state (`gate`): each has a catalog
/// entry without a chord and a command state, so bound controls resolve them like any command.
const GATED_UI_COMMANDS: &[(&str, &str)] = &[
    ("ui.crop", "Crop"),
    ("ui.linkTextBox", "Link Text Box"),
    ("ui.citations.provider", "Zotero"),
    ("ui.zotero.addEditCitation", "Add/Edit Citation"),
    ("ui.zotero.addNote", "Add Note"),
    ("ui.zotero.addEditBibliography", "Add/Edit Bibliography"),
    ("ui.zotero.refresh", "Refresh"),
    ("ui.zotero.setDocPrefs", "Document Preferences"),
    ("ui.zotero.removeCodes", "Unlink Citations"),
];

/// The default chords of an engine command: its `shortcut`, one binding per `" / "` piece.
fn default_bindings(shortcut: &str) -> impl Iterator<Item = Binding> + '_ {
    shortcut.split(" / ").filter(|piece| !piece.is_empty()).filter_map(|piece| Binding::parse(piece).ok())
}

/// The page view's canvas zoom chords (Ctrl+=, Ctrl++, Ctrl+-, Ctrl+0, Ctrl+1) are Word's Subscript,
/// Superscript, optional hyphen and paragraph spacing chords; the page zooms with Ctrl+wheel and the
/// View tab, as the old app did, so the canvas's defaults are dropped and the app's bindings take the keys.
fn with_word_chords(catalog: AppCatalog) -> AppCatalog {
    use craft::ui::canvas::ViewCommands;
    use craft::ui::runtime::commands::CommandSet as _;
    [ViewCommands::ZoomIn, ViewCommands::ZoomOut, ViewCommands::FitOnScreen, ViewCommands::ActualPixels]
        .into_iter()
        .fold(catalog, |catalog, command| catalog.replace_defaults(command.id(), Vec::new()))
}

impl WordApp {
    /// The app's commands as the keymap knows them: every engine command with its default
    /// chords, then the interface commands. The interface commands come last because a tie in the
    /// application scope goes to the first command in catalog order, so a chord the user bound to
    /// an engine command beats Ctrl+F1 (#1678).
    ///
    /// The catalog is rebuilt when the session holds another registry than the one it was built
    /// from. `None` only when the registry's rows cannot form a catalog (logged).
    pub(crate) fn catalog(&mut self) -> Option<AppCatalog> {
        let registry = self.session.registry.clone();
        // The interface chords are per platform: the window's own, which tests set.
        let mac = self.ctx.as_ref().map_or(cfg!(target_os = "macos"), |ctx| craft::ui::runtime::windows::os_of(ctx) == craft::host::target::Os::Mac);
        let os_changed = std::mem::replace(&mut self.command_catalog_mac, mac) != mac;
        // The labels are the interface language's: a button bound to a command without its own label, a menu row and
        // the command search show them, so the catalog is rebuilt when the language changes.
        let lang = crate::i18n::current().code();
        let lang_changed = std::mem::replace(&mut self.command_catalog_lang, lang) != lang;
        if os_changed || lang_changed || !self.command_catalog.as_ref().is_some_and(|(built_from, _)| Arc::ptr_eq(built_from, &registry)) {
            let generation = self.command_catalog.as_ref().map_or(0, |(_, catalog)| catalog.generation().wrapping_add(1));
            let mut seen = HashSet::new();
            let mut defs = Vec::with_capacity(registry.all().len() + 3);
            // Off macOS Alt+Q is the interface's command search, and no engine command or user binding takes it
            // (the engine reserves it, `engine/src/keymap.rs`). A tie in the application scope goes to the first
            // command in catalog order, so this one entry comes first; the other interface commands come last.
            if !mac {
                let mut def =
                    CommandDef::new(CommandRef::with_params("ui.dialog", json!({"name": "commands"})), tl!("Search Commands"), Scope::app());
                if let Ok(binding) = Binding::parse("Alt+Q") {
                    def = def.with_default(binding);
                }
                defs.push(def);
            }
            for spec in registry.all().iter().filter(|spec| seen.insert(spec.id)) {
                let mut def = CommandDef::new(CommandRef::new(spec.id), tl!(spec.label), Scope::app());
                for binding in default_bindings(spec.shortcut) {
                    def = def.with_default(binding);
                }
                defs.push(def);
            }
            let mut interface = |command: CommandRef, label: &str, chord: &str| {
                let mut def = CommandDef::new(command, tl!(label), Scope::app());
                if let Ok(binding) = Binding::parse(chord) {
                    def = def.with_default(binding);
                }
                defs.push(def);
            };
            if !mac {
                interface(CommandRef::new("ui.collapseRibbon"), "Collapse the Ribbon", "Ctrl+F1");
            } else {
                interface(CommandRef::new("ui.collapseRibbon"), "Collapse the Ribbon", "Mod+Alt+R");
            }
            interface(CommandRef::new("ui.resetInterfaceSize"), "Reset Interface Size", "Mod+Shift+0");
            for (id, label) in GATED_UI_COMMANDS {
                defs.push(CommandDef::new(CommandRef::new(*id), tl!(*label), Scope::app()));
            }
            // Word's Alt chords (Alt+Q, Alt+F7, Ctrl+Alt+1â€¦) fire while the person types in the page:
            // plain keys type, and every modified chord is offered to the keymap first.
            match AppCatalog::new(defs, generation) {
                Ok(catalog) => {
                    let catalog = catalog.typing_rule(TypingRule::PlainKeysWait);
                    self.command_catalog = Some((registry, with_word_chords(catalog)));
                }
                Err(error) => {
                    log::warn!("command catalog: {error:?}");
                    self.command_catalog = None;
                }
            }
        }
        self.command_catalog.as_ref().map(|(_, catalog)| catalog.clone())
    }

    /// The user's binding changes, as the keymap applies them: the session's own. The runtime
    /// rebuilds its keymap when this `Arc` changes, so the same one is handed out until the
    /// session's overrides differ from it.
    pub(crate) fn overrides(&mut self) -> Arc<Overrides> {
        if *self.key_overrides != *self.session.keymap() {
            self.key_overrides = Arc::new(self.session.keymap().clone());
        }
        self.key_overrides.clone()
    }

    /// This frame's command state: the catalog, the overrides and, for each command, whether it
    /// is enabled, whether it is checked and the reason a surface shows.
    pub(crate) fn command_input(&mut self) -> CommandInput {
        let registry = self.session.registry.clone();
        let mut input = CommandInput::new().overrides(self.overrides());
        if let Some(catalog) = self.catalog() {
            input = input.catalog(catalog);
        }
        let mut states = CommandStates::new();
        for spec in registry.all() {
            let (enabled, reason) = gate(self, spec.id); // A toggle's checked state is bound at its ribbon button, not here.
            let mut state = CommandState::new().enabled(enabled);
            if let Some(hint) = crate::widgets::command_hint(spec.location, spec.label, reason) {
                state = state.hint(hint);
            }
            states.set(CommandRef::new(spec.id), state);
        }
        // Print acts on the document, so with none open it is off like the engine's commands (#1129); a command with
        // no state here resolves as enabled.
        let ids = ["ui.collapseRibbon", "ui.resetInterfaceSize", "ui.print", "ui.printSubmit"]
            .into_iter()
            .chain(GATED_UI_COMMANDS.iter().map(|(id, _)| *id));
        for id in ids {
            states.set(CommandRef::new(id), CommandState::new().enabled(command_enabled(self, id)));
        }
        let commands = CommandRef::with_params("ui.dialog", json!({"name": "commands"}));
        states.set(commands, CommandState::new().enabled(command_enabled(self, "ui.dialog")));
        if *self.command_states != states {
            self.command_states.edit(|kept| *kept = states);
        }
        input.states(&self.command_states)
    }

    /// Run what the runtime fired this frame, in order, in the window that is active. Automation
    /// goes through [`WordApp::execute`] (never asks), every other source through
    /// [`WordApp::run`]. A command fired from anything but a key hands focus back to the page,
    /// except the interface commands, which have their own target.
    pub(crate) fn run_fired(&mut self, ctx: &egui::Context, output: &FrameOutput) {
        for fired in &output.fired {
            if let Some(window) = self.windows.active() {
                self.enter_window(window);
            }
            let id = fired.command.id.as_str();
            let params = if fired.command.params.is_null() { json!({}) } else { fired.command.params.clone() };
            let from_key = fired.source == FireSource::Key;

            let (id, params) = if id == "edit.pasteFormat" && from_key { self.paste_format_command(ctx, params) } else { (id, params) };
            // An Undo or Redo that is not a focused field's own edit (S-L187 sends that one to the
            // field) first takes back the gesture under way and every running one (S-L364).
            if matches!(id, "edit.undo" | "edit.redo") {
                let mut host = std::mem::take(&mut self.edits);
                host.before_undo(ctx, self);
                self.edits = host;
            }
            let result = if fired.source == FireSource::Automation { self.execute(id, params) } else { self.run(id, params) };
            if from_key && result.is_ok() {
                crate::announce::after_shortcut(self, id, ctx);
            }
            if let Err(error) = result {
                log::debug!("command {id}: {error}");
            }
            if !from_key && !id.starts_with("ui.") {
                self.canvas.focus_page(ctx);
            }
        }
    }

    /// Paste Formatting from its key. Mod+Shift+V pastes the formatting Mod+Shift+C (or the Format
    /// Painter) picked up; with none picked up it pastes the clipboard as text only, as Word for
    /// Windows does (#1537, #4943). Another key bound to Paste Formatting only pastes formatting.
    /// The framework fires one command per chord, so nothing waits for a browser paste event.
    fn paste_format_command(&self, ctx: &egui::Context, params: Value) -> (&'static str, Value) {
        let formatting = self.session.painter.is_some() || self.session.copied_format.is_some();
        // The command runs a pass after its press, when a quick tap's modifiers are already up: with none held the fire
        // is Mod+Shift+V's while that chord is still the command's.
        let mods = ctx.input(|i| i.modifiers);
        let chord = if mods.any() {
            mods.command && mods.shift && !mods.alt
        } else {
            let default = Binding::parse("Mod+Shift+V").ok().map(|binding| binding.trigger.chord);
            craft::ui::runtime::commands::bindings(ctx, &CommandRef::new("edit.pasteFormat"))
                .iter()
                .any(|binding| Some(binding.trigger.chord) == default)
        };
        if !formatting && chord { ("edit.pasteText", json!({})) } else { ("edit.pasteFormat", params) }
    }
}

#[cfg(test)]
mod tests {
    use craft::behavior::{EditEvent, EditEvents, GestureId, Mixed};
    use craft::ui::runtime::commands::{CommandRef, FirePhase, FireSource, Fired};
    use craft::ui::runtime::frame::FrameOutput;
    use wordcraft_engine::Session;

    use super::{WordApp, json};

    /// A size-field gesture is live through `apply_edits`; a fired Undo takes the gesture back first
    /// (`GestureHost::before_undo`), then undoes the step from before it. The gesture's later
    /// Commit adds no step (S-L364).
    #[test]
    fn a_fired_undo_takes_back_the_running_gesture_first() {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("text.insert", json!({"text": "Hello"})).unwrap();
        app.run("select.text", json!({"text": "Hello"})).unwrap();
        let before = app.session.undo_depth();

        let id = GestureId::new(1);
        let apply = |app: &mut WordApp, size: f32| {
            let _ = app.run("format.size", json!({"size": size}));
        };
        let mut begin = EditEvents::new();
        begin.push(EditEvent::begin(id, Mixed::Same(11.0_f32), "Font Size".into(), None));
        begin.push(EditEvent::update(id, 20.0_f32));
        crate::undo_marks::apply_edits(&mut app, begin, apply);

        let ctx = egui::Context::default();
        let mut output = FrameOutput::default();
        output.fired.push(Fired::new(CommandRef::new("edit.undo"), FireSource::Pointer, FirePhase::Press, 0.0));
        app.run_fired(&ctx, &output);
        let after_undo = app.session.undo_depth();
        assert_eq!(after_undo + 1, before, "the gesture left no step, and Undo took the one from before it");

        let mut end = EditEvents::new();
        end.push(EditEvent::commit(id, Mixed::Same(11.0_f32), 20.0_f32));
        crate::undo_marks::apply_edits(&mut app, end, apply);
        assert_eq!(app.session.undo_depth(), after_undo, "the late Commit of the taken-back gesture adds no step");
    }
}
