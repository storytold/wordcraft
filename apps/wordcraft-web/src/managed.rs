//! Bounded native snapshot acknowledgements for opt-in host-managed saves.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotKey {
    pub request_id: String,
    pub document: String,
    pub revision: String,
}

struct ManualSave {
    request_id: String,
    document: String,
    ticket: u64,
    snapshot: Option<SnapshotKey>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Completion {
    pub ticket: Option<u64>,
    pub mark_saved: bool,
}

/// Keep one manual save independent of the latest automatic snapshot. A newer autosnapshot
/// must not strand an older manual ticket or let its ACK mark newer edits saved.
#[derive(Default)]
pub struct ManagedSaves {
    manual: Option<ManualSave>,
    latest: Option<SnapshotKey>,
}

impl ManagedSaves {
    pub fn pending(&self) -> bool {
        self.manual.is_some()
    }

    pub fn start(&mut self, document: &str, ticket: u64) -> Result<String, String> {
        if self.pending() {
            return Err("native save already pending".into());
        }
        let request_id = format!("native-save-{ticket}");
        self.manual = Some(ManualSave { request_id: request_id.clone(), document: document.into(), ticket, snapshot: None });
        Ok(request_id)
    }

    pub fn check_export(&self, snapshot: &SnapshotKey) -> Result<(), String> {
        if let Some(manual) = &self.manual
            && manual.request_id == snapshot.request_id
            && (manual.document != snapshot.document || manual.snapshot.as_ref().is_some_and(|old| old != snapshot))
        {
            return Err("manual save snapshot already bound to another document or revision".into());
        }
        Ok(())
    }

    /// Call only after the native snapshot was validated and successfully posted to the host.
    pub fn exported(&mut self, snapshot: SnapshotKey) -> Result<(), String> {
        self.check_export(&snapshot)?;
        if let Some(manual) = &mut self.manual
            && manual.request_id == snapshot.request_id
        {
            manual.snapshot = Some(snapshot);
        } else {
            self.latest = Some(snapshot);
        }
        Ok(())
    }

    pub fn knows(&self, snapshot: &SnapshotKey) -> bool {
        self.manual.as_ref().and_then(|m| m.snapshot.as_ref()) == Some(snapshot) || self.latest.as_ref() == Some(snapshot)
    }

    pub fn knows_failure(&self, request_id: &str) -> bool {
        self.manual.as_ref().is_some_and(|m| m.request_id == request_id)
    }

    pub fn acknowledge(&mut self, snapshot: &SnapshotKey, document: &str, revision: &str) -> Option<Completion> {
        let ticket = if self.manual.as_ref().and_then(|m| m.snapshot.as_ref()) == Some(snapshot) {
            self.manual.take().map(|m| m.ticket)
        } else if self.latest.as_ref() == Some(snapshot) {
            self.latest = None;
            None
        } else {
            return None;
        };
        Some(Completion { ticket, mark_saved: snapshot.document == document && snapshot.revision == revision })
    }

    pub fn fail(&mut self, request_id: &str) -> Option<u64> {
        if self.knows_failure(request_id) { self.manual.take().map(|m| m.ticket) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(id: &str, revision: &str) -> SnapshotKey {
        SnapshotKey { request_id: id.into(), document: "document".into(), revision: revision.into() }
    }
    #[test]
    fn stale_manual_ack_settles_its_ticket_after_a_newer_autosnapshot() {
        let mut saves = ManagedSaves::default();
        let id = saves.start("document", 7).unwrap();
        let manual = key(&id, "r1");
        saves.exported(manual.clone()).unwrap();
        saves.exported(key("auto", "r2")).unwrap();
        assert_eq!(saves.acknowledge(&manual, "document", "r2"), Some(Completion { ticket: Some(7), mark_saved: false }));
        assert!(saves.start("document", 8).is_ok());
    }
    #[test]
    fn only_an_exported_exact_document_and_revision_can_mark_saved() {
        let mut saves = ManagedSaves::default();
        let id = saves.start("document", 7).unwrap();
        let manual = key(&id, "r1");
        assert!(!saves.knows(&manual));
        assert_eq!(saves.acknowledge(&manual, "document", "r1"), None);
        saves.exported(manual.clone()).unwrap();
        assert!(saves.knows(&manual));
        let mut wrong = manual.clone();
        wrong.document = "other".into();
        assert_eq!(saves.acknowledge(&wrong, "document", "r1"), None);
        assert_eq!(saves.acknowledge(&key("unknown", "r1"), "document", "r1"), None);
        assert!(saves.start("document", 8).is_err());
        assert_eq!(saves.acknowledge(&manual, "document", "r1"), Some(Completion { ticket: Some(7), mark_saved: true }));
        assert_eq!(saves.acknowledge(&manual, "document", "r1"), None);
    }
    #[test]
    fn changed_draft_or_document_keeps_dirty_but_completes_the_known_ticket() {
        for (document, revision) in [("document", "r1:draft2"), ("replacement", "r1")] {
            let mut saves = ManagedSaves::default();
            let id = saves.start("document", 4).unwrap();
            let snapshot = key(&id, "r1");
            saves.exported(snapshot.clone()).unwrap();
            assert_eq!(saves.acknowledge(&snapshot, document, revision), Some(Completion { ticket: Some(4), mark_saved: false }));
        }
    }
    #[test]
    fn manual_snapshot_cannot_be_overwritten_and_unknown_failure_cannot_consume_it() {
        let mut saves = ManagedSaves::default();
        let id = saves.start("document", 4).unwrap();
        saves.exported(key(&id, "r1")).unwrap();
        assert!(saves.exported(key(&id, "r2")).is_err());
        let mut wrong = key(&id, "r1");
        wrong.document = "replacement".into();
        assert!(saves.exported(wrong).is_err());
        assert_eq!(saves.fail("unknown"), None);
        assert_eq!(saves.fail(&id), Some(4));
        assert_eq!(saves.fail(&id), None);
        assert!(saves.start("document", 5).is_ok());
    }
    #[test]
    fn autosnapshot_ack_has_no_manual_ticket_and_requires_exact_current_token() {
        let mut saves = ManagedSaves::default();
        let snapshot = key("auto", "r1");
        saves.exported(snapshot.clone()).unwrap();
        assert_eq!(saves.acknowledge(&snapshot, "document", "r2"), Some(Completion { ticket: None, mark_saved: false }));
        saves.exported(snapshot.clone()).unwrap();
        assert_eq!(saves.acknowledge(&snapshot, "document", "r1"), Some(Completion { ticket: None, mark_saved: true }));
    }
}
