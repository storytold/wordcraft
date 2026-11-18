//! The app's side of the framework command runtime: the catalog of commands and their default
//! chords, the per-frame state (enabled, checked, hint) and the running of what the runtime fired.

use std::collections::HashSet;
use std::sync::Arc;

use craft::keymap::Overrides;
use craft::ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandRef, CommandState, FireSource, Scope};
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

impl WordApp {
    /// The app's commands as the keymap knows them: every engine command with its default
    /// chords, then the interface commands. The interface commands come last because a tie in the
    /// application scope goes to the first command in catalog order, so a chord the user bound to
    /// an engine command beats Ctrl+F1 (#1678).
    pub(crate) fn catalog(&mut self) -> AppCatalog {
        // PLAN: assumed `Registry::generation(&self) -> u64` and the field `command_catalog: AppCatalog` (app-hub).
        let generation = self.session.registry.generation();
        if self.command_catalog.generation() != generation {
            let registry = self.session.registry.clone();
            let mut seen = HashSet::new();
            let mut defs = Vec::with_capacity(registry.all().len() + 3);
            for spec in registry.all().iter().filter(|spec| seen.insert(spec.id)) {
                let mut def = CommandDef::new(CommandRef::new(spec.id), spec.label, Scope::app());
                for binding in default_bindings(spec.shortcut) {
                    def = def.with_default(binding);
                }
                defs.push(def);
            }
            let mut interface = |command: CommandRef, label: &str, chord: &str| {
                let mut def = CommandDef::new(command, label, Scope::app());
                if let Ok(binding) = Binding::parse(chord) {
                    def = def.with_default(binding);
                }
                defs.push(def);
            };
            if !cfg!(target_os = "macos") {
                interface(CommandRef::with_params("ui.dialog", json!({"name": "commands"})), "Search Commands", "Alt+Q");
                interface(CommandRef::new("ui.collapseRibbon"), "Collapse the Ribbon", "Ctrl+F1");
            } else {
                interface(CommandRef::new("ui.collapseRibbon"), "Collapse the Ribbon", "Mod+Alt+R");
            }
            interface(CommandRef::new("ui.resetInterfaceSize"), "Reset Interface Size", "Mod+Shift+0");
            for (id, label) in GATED_UI_COMMANDS {
                defs.push(CommandDef::new(CommandRef::new(*id), *label, Scope::app()));
            }
            match AppCatalog::new(defs, generation) {
                Ok(catalog) => self.command_catalog = catalog,
                Err(error) => log::warn!("command catalog: {error:?}"),
            }
        }
        self.command_catalog.clone()
    }

    /// The user's binding changes, as the keymap applies them.
    pub(crate) fn overrides(&self) -> Arc<Overrides> {
        // PLAN: assumed the field `key_overrides: Arc<Overrides>` (app-hub), filled from the store.
        self.key_overrides.clone()
    }

    /// This frame's command state: the catalog, the overrides and, for each command, whether it
    /// is enabled, whether it is checked and the reason a surface shows.
    pub(crate) fn command_input(&mut self) -> CommandInput {
        let registry = self.session.registry.clone();
        let mut input = CommandInput::new().catalog(self.catalog()).overrides(self.overrides());
        for spec in registry.all() {
            let (enabled, reason) = gate(self, spec.id);
            let mut state = CommandState::new().enabled(enabled);
            // PLAN: assumed `Session::command_checked(&self, id) -> Option<bool>`: the toggle's state from the view.
            if let Some(on) = self.session.command_checked(spec.id) {
                state = state.checked(on);
            }
            // PLAN: assumed `widgets::hint::command_hint(group, description, reason) -> Option<String>`.
            if let Some(hint) = crate::widgets::hint::command_hint(spec.location, spec.label, reason) {
                state = state.hint(hint);
            }
            input = input.state(CommandRef::new(spec.id), state);
        }
        let ids = ["ui.collapseRibbon", "ui.resetInterfaceSize"].into_iter().chain(GATED_UI_COMMANDS.iter().map(|(id, _)| *id));
        for id in ids {
            input = input.state(CommandRef::new(id), CommandState::new().enabled(command_enabled(self, id)));
        }
        let commands = CommandRef::with_params("ui.dialog", json!({"name": "commands"}));
        input.state(commands, CommandState::new().enabled(command_enabled(self, "ui.dialog")))
    }

    /// Run what the runtime fired this frame, in order, in the window that is active. Automation
    /// goes through [`WordApp::execute`] (never asks), every other source through
    /// [`WordApp::run`]. A command fired from anything but a key hands focus back to the page,
    /// except the interface commands, which have their own target.
    pub(crate) fn run_fired(&mut self, ctx: &egui::Context, output: &FrameOutput) {
        for fired in &output.fired {
            // PLAN: assumed `windows.list.active()` and `enter_window(key)` select the window the key was pressed in.
            if let Some(window) = self.windows.list.active() {
                self.enter_window(window);
            }
            let id = fired.command.id.as_str();
            let params = if fired.command.params.is_null() { json!({}) } else { fired.command.params.clone() };
            let from_key = fired.source == FireSource::Key;

            let (id, params) = if id == "edit.pasteFormat" && from_key { self.paste_format_params(ctx, params) } else { (id, params) };
            let Some(params) = params_or_skip(id, params) else { continue };
            let result = if fired.source == FireSource::Automation { self.execute(id, params) } else { self.run(id, params) };
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
    /// The result is the command to run, or a null `params` when the key press only waits for the
    /// browser's paste event.
    fn paste_format_params(&mut self, ctx: &egui::Context, params: Value) -> (&'static str, Value) {
        let formatting = self.session.painter.is_some() || self.session.copied_format.is_some();
        let chord = ctx.input(|i| i.modifiers.command && i.modifiers.shift && !i.modifiers.alt);
        // A browser sends a paste event after that chord (the desktop doesn't): after pasting
        // formatting it is ignored; with none, it pastes the text there, once.
        let paste_follows = chord && self.canvas.paste_event_follows_chord;
        self.canvas.ignore_next_paste = formatting && paste_follows;
        // The canvas's V release (Shift may go up first, so it reads as plain Mod+V) must not paste
        // again through the picture hook.
        if chord && self.canvas.focused(ctx) {
            self.canvas.pasted = true;
        }
        self.canvas.paste_wait_since = None;
        if !formatting && paste_follows {
            return ("edit.pasteFormat", Value::Null);
        }
        if !formatting && chord { ("edit.pasteText", json!({})) } else { ("edit.pasteFormat", params) }
    }
}

/// The parameters to run with, or `None` for the paste-format key that only waits for the paste event.
fn params_or_skip(id: &str, params: Value) -> Option<Value> {
    (!(id == "edit.pasteFormat" && params.is_null())).then_some(params)
}
