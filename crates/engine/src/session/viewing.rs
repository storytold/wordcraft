//! Manual Viewing is an interaction lock, independent of document protection.

use super::{Session, Value};

pub(super) const REASON: &str = "Editing is blocked in Viewing mode.";

impl Session {
    /// Whether a command would write while this open document is in Viewing.
    /// `mutates` covers undoable edits; a few non-undoable or mixed query/write
    /// commands need their own check before dispatch, including nested macro calls.
    pub fn viewing_blocks_command(&self, id: &str, params: &Value) -> bool {
        if !self.view.viewing {
            return false;
        }
        if self.registry.get(id).is_some_and(|spec| spec.mutates) {
            return true;
        }
        let has = |keys: &[&str]| keys.iter().any(|key| params.get(*key).is_some());
        match id {
            "edit.undo"
            | "edit.undoTo"
            | "edit.redo"
            | "edit.repeat"
            | "file.encrypt"
            | "file.rename"
            | "equation.edit"
            | "equation.move"
            | "tools.recordMacro"
            | "edit.formatPainter"
            | "draw.lasso"
            | "draw.pen"
            | "draw.pencil"
            | "draw.highlighter"
            | "draw.eraser"
            | "mailings.start"
            | "mailings.filterRecipients"
            | "mailings.sortRecipients" => true,
            "file.save" | "file.saveAs" => has(&["password"]),
            "file.versions" => has(&["restore"]),
            "tools.macros" => has(&["run", "define", "delete"]),
            "mailings.recipients" => has(&["csv", "path", "data", "rows"]),
            "mailings.editRecipients" => has(&["rows"]),
            "review.trackingOptions" => has(&["trackMoves"]),
            "styles.apply" => has(&["style"]),
            _ => false,
        }
    }
}
