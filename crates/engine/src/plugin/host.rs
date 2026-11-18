//! The sandbox's view of the session.

use craft_plugin::{Access, Manifest};
use serde_json::Value;

use super::{MAX_NOTE_CHARS, MAX_NOTES, MAX_RUN_STEPS, new_ui_requests, refusal, selection_json, set_setting};
use crate::Session;

/// The sandbox's view of the session: every call is checked by [`refusal`] and run through
/// [`Session::run`], as the ribbon, the CLI or MCP would run it. A call that asks the window for
/// something (a dialog, a pane, a file picker) or that, from a read-only plug-in, changes the
/// document is put back and refused.
pub(super) struct EngineHost<'s> {
    s: &'s mut Session,
    manifest: &'s Manifest,
    /// [`Session::undo_mark`] when the run started.
    start: u64,
    notes: Vec<String>,
}

impl<'s> EngineHost<'s> {
    pub(super) fn new(s: &'s mut Session, manifest: &'s Manifest, start: u64) -> Self {
        EngineHost { s, manifest, start, notes: Vec::new() }
    }

    /// What the run asked to show the user.
    pub(super) fn finish(self) -> Vec<String> {
        self.notes
    }
}

impl craft_plugin::Host for EngineHost<'_> {
    fn call(&mut self, command: &str, params: &str) -> Result<String, String> {
        let command = command.trim();
        let params: Value = serde_json::from_str(params).map_err(|e| format!("the params for `{command}` aren't JSON: {e}"))?;
        if let Some(why) = refusal(self.manifest, command, &params) {
            return Err(why);
        }
        if self.s.undo_mark().saturating_sub(self.start) >= MAX_RUN_STEPS {
            return Err(format!("a plug-in run may make at most {MAX_RUN_STEPS} edits; split the work into several runs"));
        }
        let reading = self.manifest.permissions.access != Access::Write;
        // The registry says which commands edit: a read-only plug-in is refused them up front.
        if reading && self.s.registry.get(command).is_some_and(|c| c.mutates) {
            craft_plugin::check_read_only(self.manifest, command, true)?;
        }
        let mark = self.s.call_mark();
        let r = self.s.run(command, &params);
        // Commands only ask the window for dialogs, panes and pickers, which a run can't answer;
        // scrolling to the caret is harmless.
        let asks = new_ui_requests(mark.ui_requests(), &self.s.ui_requests).iter().any(|q| q.get("scrollTo").is_none());
        // And the registry can't know everything a command does: what a read-only plug-in's
        // call changed is put back too.
        let edited = reading && self.s.changed_since(&mark);
        if asks || edited {
            self.s.put_back(mark);
            return Err(if asks {
                format!("`{command}` opens a dialog or a pane, which plug-ins can't: pass its parameters instead")
            } else {
                craft_plugin::check_read_only(self.manifest, command, true).err().unwrap_or_else(|| format!("`{command}` edits the document"))
            });
        }
        r.map(|v| v.to_string()).map_err(|e| e.to_string())
    }

    fn selection(&mut self) -> String {
        selection_json(self.s).to_string()
    }

    fn notify(&mut self, message: &str) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(message.chars().take(MAX_NOTE_CHARS).collect());
        }
    }

    fn log(&mut self, line: &str) {
        // Also in the run's result (`logs`)
        log::info!("plug-in {}: {line}", self.manifest.id);
    }

    fn setting(&mut self, key: &str) -> Option<String> {
        Some(self.s.plugins.prefs.settings.get(&self.manifest.id)?.get(key)?.to_string())
    }

    fn set_setting(&mut self, key: &str, value: Option<&str>) -> Result<(), String> {
        set_setting(&mut self.s.plugins.prefs.settings, &self.manifest.id, key, value)
    }
}
