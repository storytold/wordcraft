//! Revision allocation for an edit transaction (#6188). Dates describe when it began;
//! they are not an identity and cannot split typing at the next clock second.

use wordcraft_doc::{Revision, RevisionKind};

use super::{Selection, Session};

pub(super) struct RevisionGroup {
    command: Option<&'static str>,
    document: u64,
    revision: u64,
    selection: Selection,
    author: String,
    insert: Option<u32>,
    delete: Option<u32>,
}

fn continuous_command(id: &str) -> Option<&'static str> {
    match id {
        "text.insert" => Some("text.insert"),
        "text.backspace" => Some("text.backspace"),
        "text.delete" => Some("text.delete"),
        _ => None,
    }
}

impl Session {
    pub(super) fn begin_revision_command(&mut self, id: &str) {
        let command =
            continuous_command(id).filter(|_| self.sel.is_collapsed() && self.column.is_none() && self.member.is_none() && self.math.is_none());
        let same_target = self
            .revision_group
            .as_ref()
            .is_some_and(|g| g.document == self.document_id && g.revision == self.rev && g.selection == self.sel && g.author == self.author);
        // Queries and view-only controls keep an otherwise unchanged edit open. Caret and
        // selection commands are boundaries even if a caller moves away and immediately back.
        let query = self.registry.get(id).is_some_and(|s| !s.mutates) && !id.starts_with("caret.") && !id.starts_with("select.");
        if query && same_target {
            return;
        }
        if command.is_some() && same_target && self.revision_group.as_ref().is_some_and(|g| g.command == command) {
            return;
        }
        let tracking = self.doc.settings.track_changes || self.doc.settings.protection.as_deref() == Some("trackedChanges");
        self.revision_group = tracking.then(|| RevisionGroup {
            command,
            document: self.document_id,
            revision: self.rev,
            selection: self.sel.clone(),
            author: self.author.clone(),
            insert: None,
            delete: None,
        });
    }

    pub(super) fn finish_revision_command(&mut self, success: bool) {
        let Some(g) = self.revision_group.as_mut() else { return };
        // Replacements, another target, failed transactions and automatic transformations
        // cannot leave an old identity eligible for the next ordinary keystroke.
        if !success
            || g.command.is_none()
            || g.document != self.document_id
            || g.author != self.author
            || !self.doc.settings.track_changes
            || !self.sel.is_collapsed()
            || g.selection.focus.story != self.sel.focus.story
            || g.selection.focus.path != self.sel.focus.path
            || self.ime_edit_outcome.transformed
        {
            self.revision_group = None;
            return;
        }
        g.selection = self.sel.clone();
        g.revision = self.rev;
    }

    /// Allocate at most one insertion/deletion identity per command, or reuse the one from
    /// this continuous edit. Called outside dispatch, each allocation starts a fresh edit.
    pub(crate) fn edit_revision(&mut self, kind: RevisionKind) -> u32 {
        let held = self.pending_move.as_ref().map(|m| m.rev);
        let slot = self.revision_group.as_mut().and_then(|g| match kind {
            RevisionKind::Insert => Some(&mut g.insert),
            RevisionKind::Delete => Some(&mut g.delete),
            _ => None,
        });
        if let Some(i) = slot.as_deref().copied().flatten()
            && held != Some(i)
            && self.doc.revisions.get(i as usize).is_some_and(|r| r.kind == kind && r.author == self.author && r.move_name.is_none())
        {
            return i;
        }
        self.doc.revisions.push(Revision { kind, author: self.author.clone(), date: crate::cmd::now_iso(), move_name: None });
        let i = self.doc.revisions.len().saturating_sub(1) as u32;
        if let Some(slot) = slot {
            *slot = Some(i);
        }
        i
    }
}
