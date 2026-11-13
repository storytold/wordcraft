//! Snapshot storage for document rooms (ADR-0005, ADR-0011, ADR WC-0003).
//!
//! A snapshot is the model JSON **plus** the source `.docx` bytes, because
//! `Document::kept` is `#[serde(skip)]` and unmodelled paragraph markup can
//! only be rebuilt from the file it came from. The store writes atomically
//! (temporary file + rename, metadata last), so a crash leaves either the old
//! snapshot or the new one, never a half-written pair.
//!
//! [`Store::load`] tells three states apart: no snapshot was ever written
//! (`Ok(None)`), a snapshot was written and opens (`Ok(Some(..))`), and a
//! snapshot is there but cannot be opened (`Err(Corrupt)`). The last one is not
//! an empty document: a caller that starts a blank room instead would persist
//! it over the stored file on the first operation or on shutdown and destroy
//! the only copy (ADR WC-0003).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;
use wordcraft_collab::journal::DedupWindow;

use crate::cabinet::Published;
use crate::hex;

/// One stored snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub version: u64,
    pub epoch: u64,
    /// `serde_json` bytes of `wordcraft_doc::Document`.
    pub model: Vec<u8>,
    /// The `.docx` the model was read from, kept for `kept`/`passthrough`.
    pub source: Option<Vec<u8>>,
    /// The `opId` answers accepted up to `version` (ADR-0003). A retry that
    /// crosses a restart is answered from here instead of landing twice. It is
    /// server-side state: nothing about it reaches the wire.
    pub dedup: DedupWindow,
    /// The last commit the cabinet accepted (M87). The etag is what the next
    /// commit sends as `expectedVersion`, so a restart does not lose the
    /// conflict check and the next save cannot overwrite an external edit.
    pub cabinet: Option<Published>,
}

/// A directory of per-document snapshots.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

/// Largest model the store will hand out. The server holds and commits
/// documents up to `MAX_DOCUMENT`; a model over this bound is not one this
/// server wrote, and reading it would spend the memory before the size check
/// could refuse the join.
pub const MAX_MODEL: usize = 64 << 20;

/// A snapshot that is on disk and cannot be opened.
///
/// `Option` could not carry this: `None` meant both "never written" and "cannot
/// be read", and the second one was answered with an empty room (ADR WC-0003).
#[derive(Clone, Debug, PartialEq)]
pub struct Corrupt {
    /// The file inside the document directory that failed.
    pub file: &'static str,
    /// What is wrong with it, for the log. Never document content.
    pub reason: String,
}

impl Corrupt {
    pub(crate) fn new(file: &'static str, reason: impl Into<String>) -> Corrupt {
        Corrupt { file, reason: reason.into() }
    }
}

impl std::fmt::Display for Corrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.reason)
    }
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Load the snapshot of `doc`.
    ///
    /// `Ok(None)` means no snapshot was ever written for this document: the
    /// caller may open a blank room. `Err` means a snapshot is there and cannot
    /// be opened — the caller must refuse the document instead of starting an
    /// empty room, which would be persisted over the stored file (ADR WC-0003).
    pub fn load(&self, doc: &str) -> Result<Option<Snapshot>, Corrupt> {
        let dir = self.doc_dir(doc);
        self.adopt_legacy(doc, &dir);
        let path = dir.join("model.json");
        // A bounded read: the file is rejected before it is in memory, which is
        // the whole point of `MAX_MODEL` (CodeRabbit 4533747500).
        let model = match fs::File::open(&path) {
            Ok(file) => {
                use std::io::Read as _;
                let mut bytes = Vec::new();
                file.take(MAX_MODEL as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|err| Corrupt::new("model.json", format!("cannot be read: {err}")))?;
                bytes
            }
            // The model is written before the metadata, so its absence is the
            // one state that means "never snapshotted".
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(Corrupt::new("model.json", format!("cannot be read: {err}"))),
        };
        if model.len() > MAX_MODEL {
            return Err(Corrupt::new("model.json", format!("is {} bytes, over the {MAX_MODEL}-byte model bound", model.len())));
        }
        // `save` writes the metadata last, after every rename: without it the
        // pair is a write that never committed.
        let meta = match fs::read_to_string(dir.join("meta")) {
            Ok(text) => text,
            Err(err) => return Err(Corrupt::new("meta", format!("is missing or unreadable: {err}"))),
        };
        let mut version = None;
        let mut epoch = 0u64;
        let mut cabinet_version = None;
        let mut cabinet_etag = None;
        for line in meta.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "version" => version = Some(parse_number("version", value)?),
                "epoch" => epoch = parse_number("epoch", value)?,
                "cabinet_version" => cabinet_version = Some(parse_number("cabinet_version", value)?),
                "cabinet_etag" => cabinet_etag = Some(value.to_string()),
                // Unknown keys are ignored: a newer server may write more.
                _ => {}
            }
        }
        // The head version is what the room restores; without it there is
        // nothing to join. An absent `epoch` is an older snapshot and starts at
        // 0, as it did before the epoch was written.
        let Some(version) = version else {
            return Err(Corrupt::new("meta", "has no version line"));
        };
        // Half a cabinet marker is a torn write. Dropping it would also drop the
        // conflict check and let the next commit overwrite an external edit.
        let cabinet = match (cabinet_version, cabinet_etag) {
            (Some(version), Some(etag)) => Some(Published { version, etag }),
            (None, None) => None,
            _ => return Err(Corrupt::new("meta", "has a cabinet version or an etag without the other")),
        };
        // The model has to be a document, not merely JSON: a truncated or
        // foreign file under this name is exactly what a blank room would
        // overwrite.
        if let Err(err) = serde_json::from_slice::<wordcraft_doc::Document>(&model) {
            return Err(Corrupt::new("model.json", format!("is not a document: {err}")));
        }
        let source = fs::read(dir.join("source.docx")).ok();
        // A snapshot written before the window was stored has no `dedup.json`:
        // the room still opens, only without deduplication across the restart.
        // A window that cannot be tied to **this** snapshot is not read either:
        // an answer it holds may name an edit this model never had, and
        // acknowledging that retry would drop the edit (review 5855379673).
        let dedup = read_window(&dir.join("dedup.json"), version, &model);
        Ok(Some(Snapshot { version, epoch, model, source, dedup, cabinet }))
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
            let _ = fs::copy(dir.join("dedup.json"), prev.join("dedup.json"));
        }

        write_atomic(&dir.join("model.json"), &snapshot.model)?;
        if let Some(source) = &snapshot.source {
            write_atomic(&dir.join("source.docx"), source)?;
        }
        // The window travels with the snapshot it belongs to: the version and
        // the digest of the model it was written beside are its stamp, so a
        // later restore cannot pair it with a different model (review
        // 5855379673).
        let window = WindowStamp { version: snapshot.version, model: digest(&snapshot.model), window: &snapshot.dedup };
        let dedup = serde_json::to_vec(&window).map_err(|err| format!("cannot encode the opId window: {err}"))?;
        write_atomic(&dir.join("dedup.json"), &dedup)?;
        // Metadata last: its presence means the pair is complete.
        let mut meta = format!("version={}\nepoch={}\ndoc_sha256={}\n", snapshot.version, snapshot.epoch, digest(doc.as_bytes()));
        if let Some(cabinet) = &snapshot.cabinet {
            meta.push_str(&format!("cabinet_version={}\ncabinet_etag={}\n", cabinet.version, cabinet.etag));
        }
        write_atomic(&dir.join("meta"), meta.as_bytes())
    }

    pub fn doc_dir(&self, doc: &str) -> PathBuf {
        self.dir.join(sanitize(doc))
    }

    /// Move a snapshot written under the old, lossy directory name to the name
    /// the digest gives it, once.
    ///
    /// The old name could not tell `a/b` from `a_b` (review 5855379673). One id
    /// owned the directory and the other was handed its snapshot; the first
    /// load claims the data by renaming it, so exactly one of them keeps it and
    /// the other is left with nothing instead of another document. A snapshot
    /// that records its own id and belongs to a different one is not claimed:
    /// it stays where it is, untouched, and `save` never writes over it.
    fn adopt_legacy(&self, doc: &str, dir: &Path) {
        if dir.join("model.json").exists() {
            return;
        }
        let legacy = self.dir.join(legacy_name(doc));
        if legacy == dir || !legacy.join("model.json").exists() {
            return;
        }
        if let Ok(meta) = fs::read_to_string(legacy.join("meta"))
            && let Some(stored) = meta.lines().find_map(|line| line.strip_prefix("doc_sha256="))
            && stored != digest(doc.as_bytes())
        {
            return;
        }
        // A rename that fails (another id claimed it first, or the directory is
        // not ours to move) leaves the snapshot for the next load to find where
        // it is: nothing is copied over and nothing is deleted.
        let _ = fs::rename(&legacy, dir);
    }
}

/// A known `meta` number. A value that is not one is damage, not a default:
/// starting a room at version 0 would move the head backwards.
fn parse_number(key: &'static str, value: &str) -> Result<u64, Corrupt> {
    value.parse::<u64>().map_err(|_| Corrupt::new("meta", format!("has a {key} that is not a number: {value:?}")))
}

/// Keep a document id from escaping the data directory, and keep it apart from
/// every other id.
///
/// A readable prefix stays, so an operator can still tell a directory from its
/// name; the digest of the **whole** id is what makes the mapping injective.
/// The old, lossy name mapped `a/b` and `a_b` to one directory — a client could
/// be handed another document's snapshot and a save could overwrite it — and
/// cut every id at 64 characters, so two ids that agreed there collided too
/// (review 5855379673).
fn sanitize(doc: &str) -> String {
    let mut out = String::with_capacity(48 + 1 + 64);
    for ch in doc.chars().take(48) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out.push('-');
    out.push_str(&digest(doc.as_bytes()));
    out
}

/// The name this store used before the digest: only [`Store::adopt_legacy`]
/// reads it, and `save` never writes to it.
fn legacy_name(doc: &str) -> String {
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

/// `sha256` as lower-case hex: the identity a directory name and a window stamp
/// are built from.
fn digest(bytes: &[u8]) -> String {
    hex(&sha2::Sha256::digest(bytes))
}

/// The window file: the stored answers and the snapshot they belong to.
///
/// The stamp is the point. A window read on its own could come from a
/// divergent history — a restore that put `model.json` and `meta` back from one
/// backup while a newer `dedup.json` stayed behind — and hold an answer for an
/// edit the restored model never had; a retry would then be acknowledged
/// without being applied, which loses the text. A window that cannot prove it
/// belongs to this model is dropped, which costs a duplicate answer instead
/// (review 5855379673).
#[derive(Serialize)]
struct WindowStamp<'a> {
    version: u64,
    model: String,
    window: &'a DedupWindow,
}

#[derive(Deserialize)]
struct StoredWindow {
    version: u64,
    #[serde(default)]
    model: String,
    #[serde(default)]
    window: DedupWindow,
}

/// Read the `opId` window back only when it carries the stamp of `version` and
/// `model`. Anything else — a window from another snapshot, a window written
/// before the stamp existed, a damaged file — is no window: the answer it holds
/// would be about an edit this model does not have.
fn read_window(path: &Path, version: u64, model: &[u8]) -> DedupWindow {
    let Ok(bytes) = fs::read(path) else { return DedupWindow::default() };
    match serde_json::from_slice::<StoredWindow>(&bytes) {
        Ok(stored) if stored.version == version && stored.model == digest(model) => stored.window,
        _ => DedupWindow::default(),
    }
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
        let dedup = DedupWindow {
            entries: vec![wordcraft_collab::wire::Applied {
                op_id: wordcraft_collab::wire::OpId::new("web-1", 7),
                version: 7,
                epoch: 0,
                delta: Some(serde_json::json!({"op": "insert_text", "text": "x"})),
                conflict: false,
                history: None,
                nodes: None,
            }],
        };
        let snapshot =
            Snapshot { version: 7, epoch: 2, model: b"{\"body\":[]}".to_vec(), source: Some(b"PK\x03\x04".to_vec()), dedup, cabinet: None };
        store.save("doc-1", &snapshot).expect("save");
        assert_eq!(store.load("doc-1"), Ok(Some(snapshot)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cabinet_etag_survives_a_restart() {
        let dir = temp_dir("cabinet");
        let store = Store::new(&dir);
        let snapshot = Snapshot {
            version: 4,
            epoch: 0,
            model: b"{}".to_vec(),
            source: None,
            dedup: DedupWindow::default(),
            cabinet: Some(Published { version: 42, etag: "1794186677".into() }),
        };
        store.save("doc-1", &snapshot).expect("save");
        let loaded = store.load("doc-1").expect("load").expect("a snapshot");
        assert_eq!(loaded.cabinet, Some(Published { version: 42, etag: "1794186677".into() }), "the next commit expects the cabinet's etag");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_op_id_window_round_trips_with_the_snapshot() {
        let dir = temp_dir("dedup");
        let store = Store::new(&dir);
        let dedup = DedupWindow {
            entries: (1..=3)
                .map(|seq| wordcraft_collab::wire::Applied {
                    op_id: wordcraft_collab::wire::OpId::new("ui-1", seq),
                    version: seq,
                    epoch: 0,
                    delta: None,
                    conflict: seq == 3,
                    history: None,
                    nodes: None,
                })
                .collect(),
        };
        store
            .save("doc-1", &Snapshot { version: 3, epoch: 0, model: b"{}".to_vec(), source: None, dedup: dedup.clone(), cabinet: None })
            .expect("save");
        assert_eq!(store.load("doc-1").expect("load").map(|s| s.dedup), Some(dedup));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_snapshot_without_a_window_still_opens() {
        // Snapshots written before the window was stored have no `dedup.json`.
        let dir = temp_dir("no-dedup");
        let store = Store::new(&dir);
        let doc_dir = store.doc_dir("doc-old");
        fs::create_dir_all(&doc_dir).expect("dir");
        fs::write(doc_dir.join("model.json"), b"{}").expect("model");
        fs::write(doc_dir.join("meta"), b"version=4\nepoch=1\n").expect("meta");
        let snapshot = store.load("doc-old").expect("load").expect("an older snapshot opens");
        assert_eq!(snapshot.version, 4);
        assert!(snapshot.dedup.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_window_does_not_lose_the_document() {
        let dir = temp_dir("bad-dedup");
        let store = Store::new(&dir);
        store
            .save("doc-1", &Snapshot { version: 2, epoch: 0, model: b"{}".to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None })
            .expect("save");
        fs::write(store.doc_dir("doc-1").join("dedup.json"), b"not json").expect("corrupt");
        let snapshot = store.load("doc-1").expect("load").expect("the document still opens");
        assert_eq!(snapshot.version, 2);
        assert!(snapshot.dedup.is_empty(), "a broken window falls back to none, never to a panic");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_snapshot_is_none() {
        let dir = temp_dir("missing");
        let store = Store::new(&dir);
        assert_eq!(store.load("never-saved"), Ok(None), "never written is not damaged");
    }

    /// The distinction ADR WC-0003 is about: a file that is there and unreadable
    /// is not the absence of a file.
    #[test]
    fn a_damaged_model_is_corrupt_not_absent() {
        let dir = temp_dir("bad-model");
        let store = Store::new(&dir);
        store
            .save("doc-1", &Snapshot { version: 3, epoch: 0, model: b"{}".to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None })
            .expect("save");
        // A truncated write is JSON that does not parse, which is what a crash
        // in the middle of a non-atomic writer leaves.
        fs::write(store.doc_dir("doc-1").join("model.json"), b"{\"body\":[").expect("truncate");
        let err = store.load("doc-1").expect_err("a truncated model is not an empty document");
        assert_eq!(err.file, "model.json");
        assert!(err.reason.contains("not a document"), "{}", err.reason);
        let _ = fs::remove_dir_all(&dir);
    }

    /// `save` renames the model before writing the metadata, so a model without
    /// metadata is a write that never committed.
    #[test]
    fn a_snapshot_without_its_metadata_is_corrupt() {
        let dir = temp_dir("torn");
        let store = Store::new(&dir);
        let doc_dir = store.doc_dir("doc-1");
        fs::create_dir_all(&doc_dir).expect("dir");
        fs::write(doc_dir.join("model.json"), b"{}").expect("model");
        let err = store.load("doc-1").expect_err("a model without meta never committed");
        assert_eq!(err.file, "meta");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn metadata_that_is_not_a_number_is_corrupt() {
        let dir = temp_dir("bad-meta");
        let store = Store::new(&dir);
        store
            .save("doc-1", &Snapshot { version: 5, epoch: 1, model: b"{}".to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None })
            .expect("save");
        fs::write(store.doc_dir("doc-1").join("meta"), b"version=5\nepoch=now\n").expect("meta");
        let err = store.load("doc-1").expect_err("an epoch that is not a number is damage");
        assert_eq!(err.file, "meta");
        assert!(err.reason.contains("epoch"), "{}", err.reason);

        fs::write(store.doc_dir("doc-1").join("meta"), b"version=five\nepoch=1\n").expect("meta");
        assert_eq!(store.load("doc-1").expect_err("a head that is not a number is damage").file, "meta");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Dropping half the cabinet marker would drop the conflict check and let
    /// the next commit overwrite an external edit.
    #[test]
    fn half_a_cabinet_marker_is_corrupt() {
        let dir = temp_dir("half-cabinet");
        let store = Store::new(&dir);
        let snapshot = Snapshot {
            version: 2,
            epoch: 0,
            model: b"{}".to_vec(),
            source: None,
            dedup: DedupWindow::default(),
            cabinet: Some(Published { version: 9, etag: "1794186677".into() }),
        };
        store.save("doc-1", &snapshot).expect("save");
        fs::write(store.doc_dir("doc-1").join("meta"), b"version=2\nepoch=0\ncabinet_version=9\n").expect("meta");
        let err = store.load("doc-1").expect_err("an etag without a version is damage");
        assert_eq!(err.file, "meta");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_previous_snapshot_is_kept() {
        let dir = temp_dir("rotate");
        let store = Store::new(&dir);
        // Both models are documents: the store refuses to hand out anything else
        // (ADR WC-0003), so the test cannot use arbitrary bytes here.
        store
            .save("doc-1", &Snapshot { version: 1, epoch: 0, model: b"{}".to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None })
            .expect("first");
        store
            .save(
                "doc-1",
                &Snapshot { version: 2, epoch: 0, model: b"{\"body\":[]}".to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None },
            )
            .expect("second");
        assert_eq!(store.load("doc-1").expect("load").map(|s| s.version), Some(2));
        let prev = fs::read(store.doc_dir("doc-1").join("prev/model.json")).expect("previous model");
        assert_eq!(prev, b"{}");
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

    /// The directory name is a key, not a label: two documents that the old
    /// lossy name mapped together must never share a snapshot, or a client is
    /// handed another document and a save overwrites it (review 5855379673).
    #[test]
    fn ids_the_old_name_could_not_tell_apart_are_stored_apart() {
        let dir = temp_dir("injective");
        let store = Store::new(&dir);
        assert_ne!(store.doc_dir("a/b"), store.doc_dir("a_b"), "a separator and an underscore are different documents");
        let shared = "x".repeat(64);
        assert_ne!(store.doc_dir(&format!("{shared}1")), store.doc_dir(&format!("{shared}2")), "an id is not cut at 64 characters");
        assert!(store.doc_dir("a/b").starts_with(&dir));

        store
            .save("a/b", &Snapshot { version: 1, epoch: 0, model: b"{}".to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None })
            .expect("save");
        assert_eq!(store.load("a_b").expect("load"), None, "the other id is not handed this snapshot");
        assert!(store.load("a/b").expect("load").is_some(), "and its owner still has it");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A snapshot written under the old name is adopted once, by the id it
    /// belongs to; the data is moved, never copied over or deleted.
    #[test]
    fn an_old_snapshot_directory_is_adopted_once() {
        let dir = temp_dir("legacy");
        let store = Store::new(&dir);
        let legacy = dir.join("a_b");
        fs::create_dir_all(&legacy).expect("dir");
        fs::write(legacy.join("model.json"), b"{}").expect("model");
        fs::write(legacy.join("meta"), b"version=3\nepoch=0\n").expect("meta");

        let snapshot = store.load("a/b").expect("load").expect("the snapshot written under the old name is found");
        assert_eq!(snapshot.version, 3);
        assert!(!legacy.exists(), "the data moved to the name the digest gives it");
        assert_eq!(store.load("a_b").expect("load"), None, "the id that never owned it is left with nothing, not another document");
        assert_eq!(store.load("a/b").expect("load").map(|snapshot| snapshot.version), Some(3), "the owner still opens it");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A snapshot that records an id of its own is never claimed by another, so
    /// a lost-looking directory is left for its owner instead of being reused.
    #[test]
    fn a_snapshot_of_another_id_is_not_adopted() {
        let dir = temp_dir("legacy-stamped");
        let store = Store::new(&dir);
        let legacy = dir.join("a_b");
        fs::create_dir_all(&legacy).expect("dir");
        fs::write(legacy.join("model.json"), b"{}").expect("model");
        fs::write(legacy.join("meta"), format!("version=5\nepoch=0\ndoc_sha256={}\n", digest(b"someone-else")).as_bytes()).expect("meta");

        assert_eq!(store.load("a/b").expect("load"), None, "the snapshot says it belongs to another document");
        assert!(legacy.join("model.json").exists(), "and it is left where it is");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A window that cannot prove which snapshot it was written with is not
    /// read: an answer in it may name an edit this model never had, and
    /// acknowledging that retry would drop the edit (review 5855379673).
    #[test]
    fn a_window_without_the_snapshot_stamp_is_not_used() {
        let dir = temp_dir("untrusted-window");
        let store = Store::new(&dir);
        store
            .save("doc-1", &Snapshot { version: 2, epoch: 0, model: b"{}".to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None })
            .expect("save");
        // The shape an older server wrote: the answers alone, no stamp.
        let answers = DedupWindow {
            entries: vec![wordcraft_collab::wire::Applied {
                op_id: wordcraft_collab::wire::OpId::new("web-1", 1),
                version: 2,
                epoch: 0,
                delta: Some(serde_json::json!({"op": "insert_text", "text": "lost"})),
                conflict: false,
                history: None,
                nodes: None,
            }],
        };
        fs::write(store.doc_dir("doc-1").join("dedup.json"), serde_json::to_vec(&answers).expect("json")).expect("dedup");

        let snapshot = store.load("doc-1").expect("load").expect("the document still opens");
        assert!(snapshot.dedup.is_empty(), "a window with no stamp is not evidence about this model");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A window stamped by a snapshot at the same version but with another
    /// model belongs to a different history and is dropped too.
    #[test]
    fn a_window_stamped_for_another_model_is_not_used() {
        let dir = temp_dir("foreign-window");
        let store = Store::new(&dir);
        store
            .save("doc-1", &Snapshot { version: 2, epoch: 0, model: b"{}".to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None })
            .expect("save");
        let foreign = serde_json::json!({
            "version": 2,
            "model": digest(b"{\"body\":[]}"),
            "window": {"entries": [{
                "opId": {"client": "web-1", "seq": 1},
                "version": 2,
                "epoch": 0,
                "delta": {"op": "insert_text", "text": "lost"},
                "conflict": false,
            }]},
        });
        fs::write(store.doc_dir("doc-1").join("dedup.json"), serde_json::to_vec(&foreign).expect("json")).expect("dedup");

        let snapshot = store.load("doc-1").expect("load").expect("the document still opens");
        assert_eq!(snapshot.version, 2);
        assert!(snapshot.dedup.is_empty(), "the window belongs to a model this snapshot is not");
        let _ = fs::remove_dir_all(&dir);
    }
}
