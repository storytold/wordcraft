//! Local desktop recovery. Encoding and file I/O run on a worker, outside typing.
use crate::{CmdError, CmdResult, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryInfo {
    pub id: String,
    pub name: String,
    pub saved_at: String,
}

#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[cfg(not(target_arch = "wasm32"))]
use desktop::{Applied, Request};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use desktop::{DocRecovery, Recovery, Slot};

impl Session {
    /// The desktop host supplies the profile folder and a thread-safe wakeup.
    /// Headless sessions and the browser never write recovery files implicitly.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn enable_recovery(&mut self, root: std::path::PathBuf, wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        self.recovery = Some(Recovery::start(root, wake)?);
        Ok(())
    }
    /// Copies left by an earlier run (or another window's documents that have closed), newest
    /// first. The copies this session's open documents keep are not listed: they are not lost.
    pub fn recovery_documents(&self) -> Vec<RecoveryInfo> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &self.recovery {
            let held = |id: &str| self.recovery_slots().any(|(_, _, s)| s.id.as_deref() == Some(id)) || r.closing().any(|c| c == id);
            return r.documents.iter().filter(|d| !held(&d.id)).cloned().collect();
        }
        Vec::new()
    }
    /// Whether any unheld recovery documents are available.
    pub fn has_recovery_documents(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &self.recovery {
            let held = |id: &str| self.recovery_slots().any(|(_, _, s)| s.id.as_deref() == Some(id)) || r.closing().any(|c| c == id);
            return r.documents.iter().any(|d| !held(&d.id));
        }
        false
    }
    pub fn recovery_ready(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &self.recovery {
            return r.ready;
        }
        false
    }
    pub fn recovery_busy(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &self.recovery {
            return r.busy;
        }
        false
    }
    pub fn recovery_error(&self) -> Option<&str> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &self.recovery {
            return r.error.as_deref();
        }
        None
    }
    /// Called every frame: takes in the worker's answer, opens a restored copy, and queues the
    /// next job. Every open document's copy is kept, the parked ones' too, without making them
    /// active: a frame compares each document's revision and timer, and clones a document only
    /// when its copy is due.
    pub fn recovery_tick(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(mut r) = self.recovery.take() else { return };
            if let Some(e) = r.receive(false) {
                self.apply_recovery(&mut r, e);
            }
            self.open_held_restore(&mut r);
            if let Err(e) = r.tick(self.recovery_docs(), std::time::Instant::now()) {
                r.error = Some(e);
            }
            if let Some(e) = &r.error {
                self.status = e.clone();
            }
            self.recovery = Some(r);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn apply_recovery(&mut self, r: &mut Recovery, e: desktop::Event) {
        match r.apply(e) {
            // Opened by the next tick, never in the middle of a command.
            Applied::Loaded(snapshot, request) => r.held = Some((snapshot, request)),
            applied => {
                for d in self.recovery_docs() {
                    if let Applied::Failed { document, .. } = &applied
                        && *document != d.id
                    {
                        continue;
                    }
                    d.slot.apply(&applied);
                }
            }
        }
    }
    /// A restored copy opens as another document, or replaces the untouched blank document that
    /// asked for it once that document is active, so no work is ever replaced. The restored
    /// document keeps its copy in the record it came from.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_held_restore(&mut self, r: &mut Recovery) {
        let Some((_, request)) = &r.held else { return };
        let request = *request;
        if request.replace && self.is_open(request.document) && request.document != self.document_id() {
            return;
        }
        let Some((snapshot, _)) = r.held.take() else { return };
        let id = snapshot.info.id.clone();
        if !request.replace {
            self.add_document(snapshot.document);
        } else if request.document == self.document_id() && request.revision == self.rev() && self.password.is_none() {
            self.set_document(snapshot.document);
            self.path = None;
        } else {
            r.error = Some("Recovery cancelled: the document changed while the copy was loading.".into());
            return;
        }
        self.dirty = true;
        self.recovery_slot.adopt(id, self.rev());
    }
    pub fn recovery_wait(&self) -> Option<std::time::Duration> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &self.recovery {
            return r.wait(self.recovery_slots());
        }
        None
    }
    /// Waits for recovery to finish anything already under way, then deletes the copies no
    /// open document needs: closed documents', and those of documents saved or protected since
    /// their last copy. Dirty documents keep theirs. Desktop hosts call this right before the
    /// process exits, the same as they wait out pending print jobs: otherwise a save just before
    /// close can race the process exiting before a now-redundant recovery copy actually gets
    /// removed, so the next launch wrongly offers to recover a document that was in fact saved
    /// cleanly.
    pub fn finish_recovery(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mut r) = self.recovery.take() {
            if let Some(e) = r.receive(true) {
                self.apply_recovery(&mut r, e);
            }
            let mut discards: Vec<(String, Option<u64>)> = r.take_closed().into_iter().map(|id| (id, None)).collect();
            discards.extend(self.recovery_docs().filter(|d| !d.wants_copy).filter_map(|d| d.slot.id.clone().map(|id| (id, Some(d.id)))));
            for (id, document) in discards {
                match r.discard_now(&id, document) {
                    Ok(e) => self.apply_recovery(&mut r, e),
                    Err(e) => r.error = Some(e),
                }
            }
            if let Some(e) = &r.error {
                log::warn!("recovery: {e}");
            }
            self.recovery = Some(r);
        }
    }
    /// The user discarded the active document's unsaved changes without closing it (Don't Save
    /// when quitting): its recovery copy goes too, and another is made only after a further edit.
    pub fn discard_recovery_copy(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &mut self.recovery
            && let Some(id) = self.recovery_slot.id.take()
        {
            r.close(id);
            self.recovery_slot.revision = Some(self.rev());
        }
    }
    /// Only an explicit protect/save action waits for the worker and removes plaintext copies.
    /// A failure prevents applying the password; typing never waits.
    pub(crate) fn clear_recovery_before_password(&mut self) -> Result<(), CmdError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mut r) = self.recovery.take() {
            let result = self.purge_recovery(&mut r);
            self.recovery = Some(r);
            result.map_err(CmdError::Failed)?;
        }
        Ok(())
    }
    /// Waits out the job under way (it may be this document's copy being written), then deletes
    /// the active document's copy.
    #[cfg(not(target_arch = "wasm32"))]
    fn purge_recovery(&mut self, r: &mut Recovery) -> Result<(), String> {
        if let Some(e) = r.receive(true) {
            self.apply_recovery(r, e);
        }
        let Some(id) = self.recovery_slot.id.clone() else { return Ok(()) };
        let e = r.discard_now(&id, Some(self.document_id()))?;
        self.apply_recovery(r, e);
        if self.recovery_slot.id.is_some() {
            return Err(r.error.clone().unwrap_or_else(|| "the recovery copy could not be removed".into()));
        }
        self.recovery_slot.revision = None;
        Ok(())
    }
    pub(crate) fn recover(&mut self, v: &Value) -> CmdResult {
        let restore = v.get("restore");
        let discard = v.get("discard");
        if restore.is_some() && discard.is_some() {
            return Err(CmdError::Params("choose restore or discard".into()));
        }
        if restore.is_some() || discard.is_some() || v.get("refresh").is_some() {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let request =
                    Request { document: self.document_id(), revision: self.rev(), replace: self.is_untouched_blank() && self.password.is_none() };
                // Only listed copies: never one an open document keeps.
                let listed = self.recovery_documents();
                let known = |id: Option<&str>, what: &str| -> Result<String, CmdError> {
                    let id = id.ok_or_else(|| CmdError::Params(format!("{what} must be a recovery id")))?;
                    if listed.iter().any(|d| d.id == id) { Ok(id.to_string()) } else { Err(CmdError::Failed("no such recovery copy".into())) }
                };
                let r = self.recovery.as_mut().ok_or_else(|| CmdError::Disabled("local recovery requires the desktop host".into()))?;
                if let Some(id) = restore {
                    r.restore(&known(id.as_str(), "restore")?, request).map_err(CmdError::Failed)?;
                } else if let Some(id) = discard {
                    r.discard(&known(id.as_str(), "discard")?).map_err(CmdError::Failed)?;
                } else if v.get("refresh").and_then(Value::as_bool) == Some(true) {
                    r.refresh().map_err(CmdError::Failed)?;
                } else {
                    return Err(CmdError::Params("refresh must be true".into()));
                }
            }
            #[cfg(target_arch = "wasm32")]
            return Err(CmdError::Disabled("local recovery is available on desktop only".into()));
        }
        Ok(
            json!({"enabled": self.recovery_ready(), "busy": self.recovery_busy(), "documents": self.recovery_documents(), "error": self.recovery_error()}),
        )
    }
}
