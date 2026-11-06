//! Durable Version History sidecar storage (desktop). Versions are stored next to the
//! document as `<path>.wordcraft-versions.json`. Plaintext sidecars are skipped for
//! password-protected documents to avoid leaking content.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Session;
use wordcraft_doc::Document;

pub(crate) const DEFAULT_CAP: usize = 50;
#[cfg(not(target_arch = "wasm32"))]
const MAX_BYTES: u64 = 256 << 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    label: String,
    date: String,
    document: Document,
}

#[derive(Debug, Serialize, Deserialize)]
struct Sidecar {
    version: u32,
    entries: Vec<Entry>,
}

pub(crate) fn list_value(s: &Session) -> Value {
    Value::Array(s.versions.iter().enumerate().map(|(i, (label, date, _))| serde_json::json!({"index": i, "label": label, "date": date})).collect())
}

pub(crate) fn push_snapshot(s: &mut Session, label: String) {
    s.versions.push((label, crate::cmd::now_iso(), s.doc.clone()));
    cap(s, DEFAULT_CAP);
}

pub(crate) fn cap(s: &mut Session, max: usize) {
    let max = max.max(1);
    while s.versions.len() > max {
        s.versions.remove(0);
    }
}

pub(crate) fn delete_index(s: &mut Session, index: usize) -> Result<(), crate::CmdError> {
    if index >= s.versions.len() {
        return Err(crate::CmdError::Params("no such version".into()));
    }
    s.versions.remove(index);
    Ok(())
}

pub(crate) fn clear(s: &mut Session) {
    s.versions.clear();
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sidecar_path(doc_path: &Path) -> PathBuf {
    let mut name = doc_path.as_os_str().to_os_string();
    name.push(".wordcraft-versions.json");
    PathBuf::from(name)
}

#[cfg(not(target_arch = "wasm32"))]
fn can_persist(s: &Session) -> bool {
    s.password.is_none()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load_for_path(s: &mut Session, path: &Path) {
    if s.password.is_some() || !path.is_absolute() {
        return;
    }
    let sidecar = sidecar_path(path);
    let path_str = sidecar.to_string_lossy();
    if crate::file_access::check_read(&path_str).is_err() {
        return;
    }
    let meta = match std::fs::metadata(&sidecar) {
        Ok(m) => m,
        Err(_) => return,
    };
    if !meta.is_file() || meta.len() > MAX_BYTES {
        log::warn!("version history sidecar ignored (missing/too large): {}", sidecar.display());
        return;
    }
    let bytes = match std::fs::read(&sidecar) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("version history sidecar read failed ({}): {e}", sidecar.display());
            return;
        }
    };
    let parsed: Sidecar = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("version history sidecar parse failed ({}): {e}", sidecar.display());
            return;
        }
    };
    if parsed.version != 1 {
        log::warn!("version history sidecar version {} unsupported ({})", parsed.version, sidecar.display());
        return;
    }
    s.versions = parsed.entries.into_iter().map(|e| (e.label, e.date, e.document)).collect();
    cap(s, DEFAULT_CAP);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn persist(s: &Session) -> Result<bool, String> {
    let Some(doc_path) = s.path.as_deref() else { return Ok(false) };
    if !can_persist(s) || !doc_path.is_absolute() {
        return Ok(false);
    }
    let sidecar = sidecar_path(doc_path);
    let path_str = sidecar.to_string_lossy().into_owned();
    if s.versions.is_empty() {
        if sidecar.exists() {
            crate::file_access::check_write(&path_str).map_err(|e| e.to_string())?;
            std::fs::remove_file(&sidecar).map_err(|e| e.to_string())?;
        }
        return Ok(true);
    }
    let data = Sidecar {
        version: 1,
        entries: s
            .versions
            .iter()
            .map(|(label, date, document)| Entry { label: label.clone(), date: date.clone(), document: document.clone() })
            .collect(),
    };
    let bytes = serde_json::to_vec(&data).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("version history is too large to save".into());
    }
    crate::io::write_file(&path_str, &bytes)?;
    Ok(true)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn load_for_path(_s: &mut Session, _path: &std::path::Path) {}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn remove_persisted_for_path(path: &Path) -> Result<(), String> {
    let sidecar = sidecar_path(path);
    if sidecar.exists() {
        let path_str = sidecar.to_string_lossy();
        crate::file_access::check_write(&path_str).map_err(|e| e.to_string())?;
        std::fs::remove_file(&sidecar).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn clear_persisted(s: &Session) -> Result<(), String> {
    let Some(path) = s.path.as_deref() else { return Ok(()) };
    remove_persisted_for_path(path)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn persist(_s: &Session) -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn clear_persisted(_s: &Session) -> Result<(), String> {
    Ok(())
}
