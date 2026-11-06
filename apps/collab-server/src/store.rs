//! Snapshot storage for document rooms (ADR-0005, ADR-0011).
//!
//! A snapshot is the model JSON **plus** the source `.docx` bytes, because
//! `Document::kept` is `#[serde(skip)]` and unmodelled paragraph markup can
//! only be rebuilt from the file it came from. The store writes atomically
//! (temporary file + rename, metadata last), so a crash leaves either the old
//! snapshot or the new one, never a half-written pair.

use std::fs;
use std::path::{Path, PathBuf};

/// One stored snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub version: u64,
    pub epoch: u64,
    /// `serde_json` bytes of `wordcraft_doc::Document`.
    pub model: Vec<u8>,
    /// The `.docx` the model was read from, kept for `kept`/`passthrough`.
    pub source: Option<Vec<u8>>,
}

/// A directory of per-document snapshots.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Load the snapshot of `doc`, if one was written.
    pub fn load(&self, doc: &str) -> Option<Snapshot> {
        let dir = self.doc_dir(doc);
        let model = fs::read(dir.join("model.json")).ok()?;
        let meta = fs::read_to_string(dir.join("meta")).ok()?;
        let mut version = 0u64;
        let mut epoch = 0u64;
        for line in meta.lines() {
            match line.split_once('=') {
                Some(("version", value)) => version = value.parse().unwrap_or(0),
                Some(("epoch", value)) => epoch = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        let source = fs::read(dir.join("source.docx")).ok();
        Some(Snapshot { version, epoch, model, source })
    }

    /// Write a snapshot and rotate the previous one to `prev/`.
    pub fn save(&self, doc: &str, snapshot: &Snapshot) -> Result<(), String> {
        let dir = self.doc_dir(doc);
        fs::create_dir_all(&dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        // Keep one previous pair: a snapshot written over a broken one is still
        // recoverable until the next rotation.
        let prev = dir.join("prev");
        if dir.join("model.json").exists() {
            let _ = fs::create_dir_all(&prev);
            let _ = fs::copy(dir.join("model.json"), prev.join("model.json"));
            let _ = fs::copy(dir.join("meta"), prev.join("meta"));
            let _ = fs::copy(dir.join("source.docx"), prev.join("source.docx"));
        }

        write_atomic(&dir.join("model.json"), &snapshot.model)?;
        if let Some(source) = &snapshot.source {
            write_atomic(&dir.join("source.docx"), source)?;
        }
        // Metadata last: its presence means the pair is complete.
        let meta = format!("version={}\nepoch={}\n", snapshot.version, snapshot.epoch);
        write_atomic(&dir.join("meta"), meta.as_bytes())
    }

    pub fn doc_dir(&self, doc: &str) -> PathBuf {
        self.dir.join(sanitize(doc))
    }
}

/// Keep a document id from escaping the data directory.
fn sanitize(doc: &str) -> String {
    let mut out = String::with_capacity(doc.len().min(64));
    for ch in doc.chars().take(64) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "doc".to_string() } else { out }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|err| format!("cannot write {}: {err}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        format!("cannot replace {}: {err}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-collab-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_snapshot_round_trips() {
        let dir = temp_dir("roundtrip");
        let store = Store::new(&dir);
        let snapshot = Snapshot { version: 7, epoch: 2, model: b"{\"body\":[]}".to_vec(), source: Some(b"PK\x03\x04".to_vec()) };
        store.save("doc-1", &snapshot).expect("save");
        assert_eq!(store.load("doc-1"), Some(snapshot));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_snapshot_is_none() {
        let dir = temp_dir("missing");
        let store = Store::new(&dir);
        assert!(store.load("never-saved").is_none());
    }

    #[test]
    fn the_previous_snapshot_is_kept() {
        let dir = temp_dir("rotate");
        let store = Store::new(&dir);
        store.save("doc-1", &Snapshot { version: 1, epoch: 0, model: b"one".to_vec(), source: None }).expect("first");
        store.save("doc-1", &Snapshot { version: 2, epoch: 0, model: b"two".to_vec(), source: None }).expect("second");
        assert_eq!(store.load("doc-1").map(|s| s.version), Some(2));
        let prev = fs::read(store.doc_dir("doc-1").join("prev/model.json")).expect("previous model");
        assert_eq!(prev, b"one");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hostile_document_id_stays_inside_the_store() {
        let dir = temp_dir("hostile");
        let store = Store::new(&dir);
        let path = store.doc_dir("../../etc/passwd");
        assert!(path.starts_with(&dir), "{} must stay under {}", path.display(), dir.display());
        assert!(!path.to_string_lossy().contains(".."));
    }
}
