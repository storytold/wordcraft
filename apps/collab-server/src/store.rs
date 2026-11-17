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

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::Digest;
use wordcraft_collab::journal::DedupWindow;

use crate::cabinet::Published;
use crate::hex;

/// The files a snapshot is made of. `PAYLOAD` and `META` are the pair ADR-0024
/// is about: they belong to one generation, and the metadata is renamed last.
const PAYLOAD: &str = "model.json";
const META: &str = "meta";
const SOURCE: &str = "source.docx";
const DEDUP: &str = "dedup.json";
/// The previous generation, kept for a crash between the two renames.
const PREV: &str = "prev";

/// What one `save` left on disk, after it was read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    /// The room's version this generation holds.
    pub version: u64,
    /// The generation stamp both files of the pair carry.
    pub generation: u64,
}

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
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    /// One writer per document directory (ADR-0024 п.2). Two persists of the
    /// same room must not interleave their temporary files and renames; a
    /// unique temporary name alone does not make the pair they leave whole.
    /// Shared by every clone, so a second `AppState` on the same directory
    /// serializes too.
    writers: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl Clone for Store {
    fn clone(&self) -> Store {
        Store { dir: self.dir.clone(), writers: Arc::clone(&self.writers) }
    }
}

/// The shape of the payload file: the document's own bytes plus the generation
/// they belong to (ADR-0024 п.1). A payload written before generations existed
/// is a bare document and is still read as one (generation 0).
#[derive(Serialize)]
struct PayloadFile<'a> {
    /// Marks the envelope. A document never has this key, so a bare model is
    /// not mistaken for one.
    collab_snapshot: u32,
    generation: u64,
    /// The document bytes exactly as the room serialised them: read back
    /// unchanged, so a snapshot round-trips byte-for-byte.
    model: &'a RawValue,
}

#[derive(Deserialize)]
struct StoredPayload<'a> {
    collab_snapshot: u32,
    generation: u64,
    #[serde(borrow)]
    model: &'a RawValue,
}

/// A pair whose files agree.
struct Pair {
    generation: u64,
    snapshot: Snapshot,
}

/// What is on disk under one directory.
enum PairRead {
    /// No payload: nothing was ever written for this document.
    Absent,
    /// A payload and its metadata agree on generation, digest and version.
    Valid(Pair),
    /// A file is there and the pair does not hold (ADR-0024 п.4).
    Broken(Corrupt),
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
        Store { dir: dir.into(), writers: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The one writer of a document directory, for this process.
    fn writer(&self, doc: &str) -> Arc<Mutex<()>> {
        let key = sanitize(doc);
        let mut writers = self.writers.lock().unwrap_or_else(|poison| poison.into_inner());
        // A server sees many ids in its life: a lock nobody holds (only this map
        // does) is dropped instead of growing the map with them.
        writers.retain(|_, writer| Arc::strong_count(writer) > 1);
        Arc::clone(writers.entry(key).or_default())
    }

    /// Load the snapshot of `doc`.
    ///
    /// `Ok(None)` means no snapshot was ever written for this document: the
    /// caller may open a blank room. `Err` means a snapshot is there and cannot
    /// be opened — the caller must refuse the document instead of starting an
    /// empty room, which would be persisted over the stored file (ADR WC-0003).
    ///
    /// A pair whose files disagree on generation or digest is not read and not
    /// repaired: the previous generation in `prev/` is served when it is still
    /// whole (ADR-0024 п.4), and otherwise the join is refused.
    pub fn load(&self, doc: &str) -> Result<Option<Snapshot>, Corrupt> {
        let dir = self.doc_dir(doc);
        self.adopt_legacy(doc, &dir);
        match read_pair(&dir) {
            PairRead::Absent => Ok(None),
            PairRead::Valid(pair) => Ok(Some(pair.snapshot)),
            PairRead::Broken(why) => match read_pair(&dir.join(PREV)) {
                PairRead::Valid(pair) => Ok(Some(pair.snapshot)),
                _ => Err(why),
            },
        }
    }

    /// Write a snapshot, rotate the previous pair to `prev/`, and read the new
    /// pair back.
    ///
    /// One generation is written (ADR-0024): the payload carries it, the
    /// metadata carries it and the digest of the payload, the payload is
    /// synced and renamed first and the metadata last, so a crash leaves a pair
    /// that does not hold and `prev/` answers. Two persists of one document are
    /// serialized; the caller is told the pair is durable only after it was read
    /// back. The journal is the caller's to truncate, and only then.
    pub fn save(&self, doc: &str, snapshot: &Snapshot) -> Result<Committed, String> {
        let lock = self.writer(doc);
        let _writing = lock.lock().unwrap_or_else(|poison| poison.into_inner());
        let dir = self.doc_dir(doc);
        fs::create_dir_all(&dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;

        // The generation to write: one above the pair on disk. A current pair
        // that does not hold is not a base and is not rotated — overwriting the
        // last good copy with a broken one would leave nothing to fall back on
        // (ADR-0024 п.4) — so the previous generation sets the number instead.
        let current = match read_pair(&dir) {
            PairRead::Valid(pair) => Some(pair),
            _ => None,
        };
        let base = match &current {
            Some(pair) => Some(pair.generation),
            None => match read_pair(&dir.join(PREV)) {
                PairRead::Valid(pair) => Some(pair.generation),
                _ => None,
            },
        };
        let generation = base.map_or(1, |generation| generation.saturating_add(1));
        if current.is_some() {
            rotate(&dir);
        }

        let payload = payload_bytes(generation, &snapshot.model)?;
        write_synced(&dir.join(PAYLOAD), &payload)?;
        if let Some(source) = &snapshot.source {
            write_synced(&dir.join(SOURCE), source)?;
        }
        // The window travels with the snapshot it belongs to: the version and
        // the digest of the model it was written beside are its stamp, so a
        // later restore cannot pair it with a different model (review
        // 5855379673).
        let window = WindowStamp { version: snapshot.version, model: digest(&snapshot.model), window: &snapshot.dedup };
        let dedup = serde_json::to_vec(&window).map_err(|err| format!("cannot encode the opId window: {err}"))?;
        write_synced(&dir.join(DEDUP), &dedup)?;
        // Metadata last: its rename is the marker that commits the pair.
        write_synced(&dir.join(META), meta_text(snapshot, doc, generation, &payload).as_bytes())?;

        // Read back what is on disk (ADR-0024 п.5). A pair that cannot be read
        // is not reported as written: the caller must not truncate its journal
        // for a snapshot that is not there.
        let committed = match read_pair(&dir) {
            PairRead::Valid(pair) => pair,
            PairRead::Absent => return Err(format!("{PAYLOAD} disappeared after it was written")),
            PairRead::Broken(why) => return Err(format!("the written snapshot does not read back: {why}")),
        };
        if committed.generation != generation || committed.snapshot.version != snapshot.version || committed.snapshot.model != snapshot.model {
            return Err(format!(
                "the written snapshot reads back as generation {} version {}, not {generation} version {}",
                committed.generation, committed.snapshot.version, snapshot.version
            ));
        }
        Ok(Committed { version: snapshot.version, generation })
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

/// Read the pair in `dir` and decide whether it holds (ADR-0024 п.3/п.4).
fn read_pair(dir: &Path) -> PairRead {
    match read_pair_inner(dir) {
        Ok(Some(pair)) => PairRead::Valid(pair),
        Ok(None) => PairRead::Absent,
        Err(corrupt) => PairRead::Broken(corrupt),
    }
}

fn read_pair_inner(dir: &Path) -> Result<Option<Pair>, Corrupt> {
    let path = dir.join(PAYLOAD);
    // A bounded read: the file is rejected before it is in memory, which is
    // the whole point of `MAX_MODEL` (CodeRabbit 4533747500).
    let payload = match File::open(&path) {
        Ok(file) => {
            use std::io::Read as _;
            let mut bytes = Vec::new();
            file.take(MAX_MODEL as u64 + 1).read_to_end(&mut bytes).map_err(|err| Corrupt::new(PAYLOAD, format!("cannot be read: {err}")))?;
            bytes
        }
        // The payload is written before the metadata, so its absence is the one
        // state that means "never snapshotted".
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Corrupt::new(PAYLOAD, format!("cannot be read: {err}"))),
    };
    if payload.len() > MAX_MODEL {
        return Err(Corrupt::new(PAYLOAD, format!("is {} bytes, over the {MAX_MODEL}-byte model bound", payload.len())));
    }
    // `save` renames the metadata last, after every other rename: without it
    // the pair is a write that never committed.
    let meta = fs::read_to_string(dir.join(META)).map_err(|err| Corrupt::new(META, format!("is missing or unreadable: {err}")))?;
    let mut version = None;
    let mut epoch = 0u64;
    let mut generation = None;
    let mut payload_sha256 = None;
    let mut cabinet_version = None;
    let mut cabinet_etag = None;
    for line in meta.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "version" => version = Some(parse_number("version", value)?),
            "epoch" => epoch = parse_number("epoch", value)?,
            "generation" => generation = Some(parse_number("generation", value)?),
            "payload_sha256" => payload_sha256 = Some(value.trim().to_ascii_lowercase()),
            "cabinet_version" => cabinet_version = Some(parse_number("cabinet_version", value)?),
            "cabinet_etag" => cabinet_etag = Some(value.to_string()),
            // Unknown keys are ignored: a newer server may write more.
            _ => {}
        }
    }
    // The head version is what the room restores; without it there is nothing
    // to join. An absent `epoch` is an older snapshot and starts at 0, as it did
    // before the epoch was written.
    let Some(version) = version else {
        return Err(Corrupt::new(META, "has no version line"));
    };
    // Half a cabinet marker is a torn write. Dropping it would also drop the
    // conflict check and let the next commit overwrite an external edit.
    let cabinet = match (cabinet_version, cabinet_etag) {
        (Some(version), Some(etag)) => Some(Published { version, etag }),
        (None, None) => None,
        _ => return Err(Corrupt::new(META, "has a cabinet version or an etag without the other")),
    };
    let (payload_generation, model) = unpack(&payload)?;
    // The pair belongs to one generation: a payload from a newer write beside an
    // older metadata is exactly the crash between the two renames, and reading
    // either half would serve a state neither file describes (ADR-0024 п.3).
    match (generation, payload_generation) {
        (Some(meta_generation), Some(payload_generation)) if meta_generation != payload_generation => {
            return Err(Corrupt::new(PAYLOAD, format!("belongs to generation {payload_generation}, the metadata says {meta_generation}")));
        }
        (Some(_), None) => return Err(Corrupt::new(PAYLOAD, "carries no generation while the metadata does")),
        // A pair written before generations existed carries neither and is read
        // as generation 0.
        _ => {}
    }
    let generation = generation.or(payload_generation).unwrap_or(0);
    if let Some(expected) = &payload_sha256
        && *expected != digest(&payload)
    {
        return Err(Corrupt::new(PAYLOAD, "does not match the digest the metadata records"));
    }
    // The model has to be a document, not merely JSON: a truncated or foreign
    // file under this name is exactly what a blank room would overwrite.
    if let Err(err) = serde_json::from_slice::<wordcraft_doc::Document>(model) {
        return Err(Corrupt::new(PAYLOAD, format!("is not a document: {err}")));
    }
    let source = fs::read(dir.join(SOURCE)).ok();
    // A snapshot written before the window was stored has no `dedup.json`: the
    // room still opens, only without deduplication across the restart. A window
    // that cannot be tied to **this** snapshot is not read either: an answer it
    // holds may name an edit this model never had, and acknowledging that retry
    // would drop the edit (review 5855379673).
    let dedup = read_window(&dir.join(DEDUP), version, model);
    Ok(Some(Pair { generation, snapshot: Snapshot { version, epoch, model: model.to_vec(), source, dedup, cabinet } }))
}

/// The payload bytes for one generation: the document's own bytes inside an
/// envelope that names the generation (ADR-0024 п.1).
fn payload_bytes(generation: u64, model: &[u8]) -> Result<Vec<u8>, String> {
    let model = serde_json::from_slice::<&RawValue>(model).map_err(|err| format!("the model is not JSON: {err}"))?;
    serde_json::to_vec(&PayloadFile { collab_snapshot: 1, generation, model }).map_err(|err| format!("cannot encode the payload: {err}"))
}

/// Split a payload into the generation it names, `None` for a bare model, and
/// the document bytes themselves (borrowed from the payload, so they round-trip
/// unchanged).
fn unpack(payload: &[u8]) -> Result<(Option<u64>, &[u8]), Corrupt> {
    match serde_json::from_slice::<StoredPayload>(payload) {
        Ok(envelope) if envelope.collab_snapshot == 1 => Ok((Some(envelope.generation), envelope.model.get().as_bytes())),
        Ok(envelope) => Err(Corrupt::new(PAYLOAD, format!("has an unknown snapshot format {}", envelope.collab_snapshot))),
        // No envelope: a payload written before this format, read as the model
        // it is (generation 0).
        Err(_) => Ok((None, payload)),
    }
}

/// The metadata of one generation: the version to join at, the generation both
/// files carry and the digest of the payload beside it.
fn meta_text(snapshot: &Snapshot, doc: &str, generation: u64, payload: &[u8]) -> String {
    let mut meta = format!(
        "version={}\nepoch={}\ngeneration={}\npayload_sha256={}\ndoc_sha256={}\n",
        snapshot.version,
        snapshot.epoch,
        generation,
        digest(payload),
        digest(doc.as_bytes())
    );
    if let Some(cabinet) = &snapshot.cabinet {
        meta.push_str(&format!("cabinet_version={}\ncabinet_etag={}\n", cabinet.version, cabinet.etag));
    }
    meta
}

/// Keep one previous pair: a crash between the two renames of the next write
/// finds the last whole generation here (ADR-0024 п.3). Rotated only from a
/// pair that holds.
fn rotate(dir: &Path) {
    let prev = dir.join(PREV);
    let _ = fs::create_dir_all(&prev);
    for name in [PAYLOAD, META, SOURCE, DEDUP] {
        let _ = fs::copy(dir.join(name), prev.join(name));
    }
}

/// Write a file so that a crash leaves either the old bytes or the new ones: a
/// temporary name no other attempt shares, `sync_all`, rename, then a synced
/// directory so the rename itself is durable (ADR-0024 п.3).
fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static ATTEMPT: AtomicU64 = AtomicU64::new(0);
    let attempt = ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().map(|name| name.to_os_string()).unwrap_or_default();
    name.push(format!(".tmp.{}.{attempt}", std::process::id()));
    let tmp = path.with_file_name(name);
    let write = || -> std::io::Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    };
    write().map_err(|err| {
        let _ = fs::remove_file(&tmp);
        format!("cannot write {}: {err}", tmp.display())
    })?;
    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        format!("cannot replace {}: {err}", path.display())
    })?;
    // Best effort: not every file system lets a directory be synced, and the
    // pair is read back before it is called durable anyway.
    if let Some(dir) = path.parent()
        && let Ok(handle) = File::open(dir)
    {
        let _ = handle.sync_all();
    }
    Ok(())
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
        assert!(err.reason.contains("generation"), "a torn payload is caught by the generation it does not carry: {}", err.reason);
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
        let (generation, model) = unpack(&prev).expect("the previous payload holds");
        assert_eq!(generation, Some(1), "the previous pair is one generation");
        assert_eq!(model, b"{}");
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

    fn plain(version: u64, model: &[u8]) -> Snapshot {
        Snapshot { version, epoch: 0, model: model.to_vec(), source: None, dedup: DedupWindow::default(), cabinet: None }
    }

    /// ADR-0024 п.3: a crash after the payload is renamed and before the
    /// metadata is leaves two files of different generations. The pair does not
    /// hold, so the previous generation in `prev/` answers — not an empty
    /// document, and not the half-written state.
    #[test]
    fn a_crash_between_the_two_renames_serves_the_previous_generation() {
        let dir = temp_dir("crash-between-renames");
        let store = Store::new(&dir);
        store.save("doc-1", &plain(1, b"{\"body\":[]}")).expect("first generation");
        let doc_dir = store.doc_dir("doc-1");

        // The next persist as the crash leaves it: the previous pair is rotated,
        // the new payload is renamed into place, and the process dies before the
        // metadata rename that commits the pair.
        rotate(&doc_dir);
        let payload = payload_bytes(2, b"{}").expect("payload");
        write_synced(&doc_dir.join(PAYLOAD), &payload).expect("payload");

        let loaded = store.load("doc-1").expect("the previous generation answers, not an empty document").expect("a snapshot");
        assert_eq!(loaded.version, 1, "the generation that was whole");
        assert_eq!(loaded.model, b"{\"body\":[]}", "not the payload of the write that never committed");
        let _ = fs::remove_dir_all(&dir);
    }

    /// ADR-0024 п.4, ADR WC-0003: a payload that does not parse beside a
    /// metadata that does is damage, not an empty document. The join is refused
    /// and neither file is rewritten: an operator can still repair it.
    #[test]
    fn a_corrupt_payload_with_a_valid_meta_refuses_the_join() {
        let dir = temp_dir("corrupt-payload");
        let store = Store::new(&dir);
        store.save("doc-1", &plain(3, b"{}")).expect("save");
        let doc_dir = store.doc_dir("doc-1");
        let damaged = b"{\"collab_snapshot\":1,".to_vec();
        fs::write(doc_dir.join(PAYLOAD), &damaged).expect("damage");

        let err = store.load("doc-1").expect_err("a payload that does not parse is not an empty document");
        assert_eq!(err.file, PAYLOAD);
        assert_eq!(fs::read(doc_dir.join(PAYLOAD)).expect("still there"), damaged, "the file is left for an operator to repair");
        assert_eq!(fs::read_to_string(doc_dir.join(META)).expect("meta").lines().next(), Some("version=3"), "nor is the metadata rewritten");
        let _ = fs::remove_dir_all(&dir);
    }

    /// ADR-0024 п.2: two persists of one document are serialized. Each writes its
    /// own generation, the pair that stands is one generation, the metadata and
    /// the payload agree, and neither attempt leaves a temporary file behind.
    #[test]
    fn two_concurrent_persists_leave_one_generation() {
        let dir = temp_dir("two-writers");
        let store = Store::new(&dir);
        let persist = |version: u64, model: &'static [u8]| {
            let store = store.clone();
            std::thread::spawn(move || store.save("doc-1", &plain(version, model)).map(|committed| (committed, model)))
        };
        let first = persist(1, b"{}");
        let second = persist(2, b"{\"body\":[]}");
        let first = first.join().expect("thread").expect("first save");
        let second = second.join().expect("thread").expect("second save");
        let mut generations = [first.0.generation, second.0.generation];
        generations.sort_unstable();
        assert_eq!(generations, [1, 2], "one writer at a time: two generations, not one write over the other");

        let doc_dir = store.doc_dir("doc-1");
        let bytes = fs::read(doc_dir.join(PAYLOAD)).expect("payload");
        let (payload_generation, _) = unpack(&bytes).expect("the payload holds");
        let meta = fs::read_to_string(doc_dir.join(META)).expect("meta");
        let meta_generation = meta.lines().find_map(|line| line.strip_prefix("generation=")).and_then(|value| value.parse::<u64>().ok());
        assert_eq!(meta_generation, payload_generation, "the pair on disk belongs to one generation");
        assert_eq!(payload_generation, Some(2), "the write that took the lock second is the one that stands");

        let standing = if first.0.generation == 2 { first } else { second };
        let loaded = store.load("doc-1").expect("load").expect("a snapshot");
        assert_eq!(loaded.version, standing.0.version);
        assert_eq!(loaded.model, standing.1, "the document of the generation that stands");

        let leftovers: Vec<String> = fs::read_dir(&doc_dir)
            .expect("dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "no temporary file survives an attempt: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
