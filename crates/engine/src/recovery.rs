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
pub(crate) use desktop::Recovery;

impl Session {
    /// The desktop host supplies the profile folder and a thread-safe wakeup.
    /// Headless sessions and the browser never write recovery files implicitly.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn enable_recovery(&mut self, root: std::path::PathBuf, wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        self.recovery = Some(Recovery::start(root, wake)?);
        Ok(())
    }
    pub fn recovery_documents(&self) -> Vec<RecoveryInfo> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &self.recovery {
            return r.documents.clone();
        }
        Vec::new()
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
    /// A delayed restore can't replace edits made while it was loading. Restores have no path.
    pub fn recovery_tick(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(mut r) = self.recovery.take() else { return };
            if let Some((snapshot, expected)) = r.poll() {
                if expected == (self.document_id(), self.rev()) && self.password.is_none() {
                    let id = snapshot.info.id;
                    self.set_document(snapshot.document);
                    self.path = None;
                    self.dirty = true;
                    r.adopt(id, self.document_id(), self.rev());
                } else {
                    r.error = Some("Recovery cancelled: the document changed while the copy was loading.".into());
                }
            }
            if let Err(e) = r.tick(self, std::time::Instant::now()) {
                r.error = Some(e);
            }
            if let Some(e) = &r.error {
                self.status = e.clone();
            }
            self.recovery = Some(r);
        }
    }
    pub fn recovery_wait(&self) -> Option<std::time::Duration> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &self.recovery
            && self.dirty
            && self.password.is_none()
            && !r.busy
        {
            return r.wait(self.document_id(), self.rev());
        }
        None
    }
    /// Waits for recovery to finish anything already under way, discarding this session's own
    /// copy first if it no longer needs one (the document was saved, say). Desktop hosts call
    /// this right before the process exits, the same as they wait out pending print jobs:
    /// otherwise a save just before close can race the process exiting before a now-redundant
    /// recovery copy actually gets removed, so the next launch wrongly offers to recover a
    /// document that was in fact saved cleanly.
    pub fn finish_recovery(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mut r) = self.recovery.take() {
            if let Err(e) = r.finish(self) {
                log::warn!("recovery: {e}");
            }
            self.recovery = Some(r);
        }
    }
    /// Only an explicit protect/save action waits for the worker and removes plaintext copies.
    /// Returns whether a copy was removed. A failure prevents applying the password; typing never waits.
    pub(crate) fn clear_recovery_before_password(&mut self) -> Result<bool, CmdError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let document = self.document_id();
            if let Some(r) = &mut self.recovery
                && r.document_is(document)
            {
                return r.purge().map_err(CmdError::Failed);
            }
        }
        Ok(false)
    }
    /// A failed save cannot leave an unprotected dirty document dependent only on memory.
    pub(crate) fn restore_recovery_after_failed_password_save(&mut self) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mut recovery) = self.recovery.take() {
            let result = recovery.restore_after_failed_password_save(self);
            if let Err(error) = &result {
                recovery.error = Some(error.clone());
            }
            self.recovery = Some(recovery);
            return result;
        }
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
                let expected = (self.document_id(), self.rev());
                let r = self.recovery.as_mut().ok_or_else(|| CmdError::Disabled("local recovery requires the desktop host".into()))?;
                if let Some(id) = restore {
                    r.restore(id.as_str().ok_or_else(|| CmdError::Params("restore must be a recovery id".into()))?, expected)
                        .map_err(CmdError::Failed)?;
                } else if let Some(id) = discard {
                    r.discard(id.as_str().ok_or_else(|| CmdError::Params("discard must be a recovery id".into()))?, expected)
                        .map_err(CmdError::Failed)?;
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
