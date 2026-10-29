//! Document open/save against a storage backend (issue #518).
//!
//! The CLI's `store-*` commands move raw bytes; this module moves *documents*: it serialises
//! first (pure bytes, never on the typing path) and then talks to any
//! [`StorageBackend`][wordcraft_storage::StorageBackend] — a local folder today, an oCIS
//! collection over WebDAV Bearer auth tomorrow — so a `.docx` round-trips backend → engine
//! [`Session`][crate::Session] → backend with the backend carrying the bytes exactly.
//!
//! Two layers, mirroring [`crate::io`]:
//! - [`open_from_storage`] / [`save_to_storage`] are free functions over any
//!   [`Document`][wordcraft_doc::Document] (headless use: MCP, CLI, tests).
//! - [`Session::open_from_storage`][crate::Session::open_from_storage] /
//!   [`Session::save_to_storage`][crate::Session::save_to_storage] do the same and keep the
//!   session consistent (history reset, caret, password, dirty flag), exactly as `file.open`
//!   and `file.save` do for local files.
//!
//! The write adapts to the backend's [`capabilities`][wordcraft_storage::StorageBackend::capabilities]:
//! backends with atomic staging (local folders) save atomically (stage, journal, rename); a
//! plain `PUT` is already whole-or-nothing, so WebDAV saves without it. Saves are
//! last-writer-wins (no `If-Match`): the session API mirrors `file.save`, not optimistic
//! concurrency — callers that need that use the backend's [`WriteOptions`][wordcraft_storage::WriteOptions]
//! directly.
//!
//! There is intentionally no `file.openFromStorage` command: credentials must not flow through
//! the command registry (params land in Repeat and recorded macros, which is why the session
//! password is excluded from them too). Hosts construct the backend — with its secret — and
//! call this API around it.
//!
//! Not built on `wasm32` (as [`crate::io::save_path_with`]: files go through the browser
//! there, and the WebDAV backend needs sockets).

use wordcraft_doc::Document;
use wordcraft_storage::{FileMeta, StorageBackend};

use crate::Session;
use crate::io::ExportOptions;

/// Read the document stored under `path` on `backend` and parse it as `path`'s format (see
/// [`crate::io::open_bytes_with`]). Returns the document and whether it was encrypted.
/// Storage failures name `path` (backend errors are path-free by contract); parse errors are
/// reported as [`crate::io`] reports them.
pub fn open_from_storage(backend: &dyn StorageBackend, path: &str, password: Option<&str>) -> Result<(Document, bool), String> {
    let bytes = backend.read(path).map_err(|e| format!("{path}: {e}"))?;
    crate::io::open_bytes_with(path, &bytes, password)
}

/// Serialise `doc` as `path`'s format (see [`crate::io::save_bytes_with`]) and store the bytes
/// under `path` on `backend`, creating parent collections as needed. The write is atomic
/// where the backend stages (local folders) and a single whole-or-nothing `PUT` elsewhere.
/// Returns what the backend reports about the stored file (size, ETag).
pub fn save_to_storage(
    backend: &dyn StorageBackend,
    path: &str,
    doc: &Document,
    password: Option<&str>,
    opts: &ExportOptions,
) -> Result<FileMeta, String> {
    let bytes = crate::io::save_bytes_with(path, doc, password, opts)?;
    let write_opts = wordcraft_storage::WriteOptions { if_match: None, atomic: backend.capabilities().atomic_write };
    backend.write(path, &bytes, &write_opts).map_err(|e| format!("{path}: {e}"))
}

/// Formats whose save keeps the whole document, so saving clears the dirty flag — the same
/// list `file.save` uses (other formats are lossy copies: PDF, text, HTML…).
fn is_full_fidelity(path: &str) -> bool {
    let ext = path.rsplit(['/', '\\']).next().unwrap_or(path).rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "docx" | "docm" | "dotx" | "dotm" | "odt" | "rtf" | "json" | "xml")
}

/// The file-name extension of `path`, lower-case; empty when there is none.
fn ext_of(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default()
}

impl Session {
    /// Replace this session's document with the one stored under `path` on `backend` (a
    /// password-protected Word document is decrypted with `password`). History, selection and
    /// layout are reset as `file.open` resets them; the local file path is cleared (this
    /// document lives on the backend now); a document opened with a password is saved with it
    /// again, as `file.open` does. Returns whether the stored file was encrypted.
    pub fn open_from_storage(&mut self, backend: &dyn StorageBackend, path: &str, password: Option<&str>) -> Result<bool, String> {
        let (mut doc, encrypted) = open_from_storage(backend, path, password)?;
        // Plain text and Markdown carry no language: they are in the editing language.
        if let Some(lang) = self.editing_lang.clone()
            && matches!(ext_of(path).as_str(), "txt" | "text" | "md" | "markdown")
        {
            doc.styles.default_chr.lang = Some(lang);
        }
        self.set_document(doc);
        self.path = None;
        if encrypted {
            self.password = password.map(crate::io::Password::new);
        }
        Ok(encrypted)
    }

    /// Save this session's document under `path` on `backend` with the session's password and
    /// export options, as `file.save` saves it to a local file. `file.save` marks the
    /// document saved (`stamp_save`) before writing, so a failed save still leaves the
    /// pre-save undo state behind rather than a half-written file; this does the same. The
    /// dirty flag is cleared for full-fidelity formats only. Returns what the backend reports
    /// about the stored file (size, ETag).
    pub fn save_to_storage(&mut self, backend: &dyn StorageBackend, path: &str, opts: &ExportOptions) -> Result<FileMeta, String> {
        let saved_password = self.password.clone();
        let password = saved_password.as_ref().map(crate::io::Password::as_str);
        self.stamp_save();
        let meta = save_to_storage(backend, path, &self.doc, password, opts)?;
        if is_full_fidelity(path) {
            self.dirty = false;
        }
        Ok(meta)
    }

    /// Like [`Session::save_to_storage`], but the write lands only when the stored version
    /// still carries `base` (a tag from [`stat_storage`]); a stale tag reports
    /// [`StorageConflict`] instead of overwriting. Pair with a [`lease`][wordcraft_storage::lease]
    /// for the full Phase 3 recipe: hold the lease while editing, save conditioned on the tag
    /// seen at open, merge or save a conflict copy on conflict.
    pub fn save_to_storage_checked(
        &mut self,
        backend: &dyn StorageBackend,
        path: &str,
        opts: &ExportOptions,
        base: Option<&str>,
    ) -> Result<FileMeta, StorageConflict> {
        let saved_password = self.password.clone();
        let password = saved_password.as_ref().map(crate::io::Password::as_str);
        self.stamp_save();
        let meta = save_to_storage_if_match(backend, path, &self.doc, password, opts, base)?;
        if is_full_fidelity(path) {
            self.dirty = false;
        }
        Ok(meta)
    }
}

/// What a failed conditional save reports (issue #518, Phase 3): the document moved under us.
/// Never an overwrite — merge the remote work in or save a conflict copy instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageConflict {
    /// The document path the save targeted.
    pub path: String,
    /// The version tag the save conditioned on (what we last saw).
    pub base: Option<String>,
    /// The version tag stored now, when the backend reports one (`None` where untracked).
    pub remote: Option<String>,
}

impl std::fmt::Display for StorageConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: stored version changed since we last saw it", self.path)
    }
}

/// Metadata for the document stored under `path` (the version tag a conditional save
/// conditions on). Storage failures name `path`, as [`open_from_storage`] does.
pub fn stat_storage(backend: &dyn StorageBackend, path: &str) -> Result<FileMeta, String> {
    backend.stat(path).map_err(|e| format!("{path}: {e}"))
}

/// Like [`save_to_storage`], but the write lands only when the stored version still carries
/// `base` (a tag from [`stat_storage`]); `None` writes unconditionally. A stale tag reports
/// [`StorageConflict`] with the stored tag, so the caller can merge or save a conflict copy —
/// the bytes on the backend are never overwritten blind.
pub fn save_to_storage_if_match(
    backend: &dyn StorageBackend,
    path: &str,
    doc: &Document,
    password: Option<&str>,
    opts: &ExportOptions,
    base: Option<&str>,
) -> Result<FileMeta, StorageConflict> {
    let bytes = crate::io::save_bytes_with(path, doc, password, opts).map_err(|e| StorageConflict {
        path: format!("{path}: {e}"),
        base: base.map(str::to_string),
        remote: None,
    })?;
    let write_opts = wordcraft_storage::WriteOptions { if_match: base.map(str::to_string), atomic: backend.capabilities().atomic_write };
    match backend.write(path, &bytes, &write_opts) {
        Ok(meta) => Ok(meta),
        Err(wordcraft_storage::StorageError::Conflict(_)) => {
            let remote = backend.stat(path).ok().and_then(|m| m.etag);
            Err(StorageConflict { path: path.into(), base: base.map(str::to_string), remote })
        }
        Err(e) => Err(StorageConflict { path: format!("{path}: {e}"), base: base.map(str::to_string), remote: None }),
    }
}

/// The file-name stem of `suggestion`, safe for a conflict-copy name: anything that isn't a
/// letter, digit, space, dash, underscore or dot becomes `_`, capped at 64 characters.
fn sanitise_holder(suggestion: &str) -> String {
    let clean: String = suggestion.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' }).collect();
    let trimmed = clean.trim();
    let cut: String = trimmed.chars().take(64).collect();
    let cut = cut.trim();
    if cut.is_empty() { "unknown".into() } else { cut.into() }
}

/// Save `doc` beside `path` without touching what's stored: `memo.docx` becomes
/// `memo (conflict alice).docx` (a counter when that name is taken). Always unconditional —
/// the whole point is a copy that can never overwrite. Returns the path actually written.
pub fn save_conflict_copy(
    backend: &dyn StorageBackend,
    path: &str,
    doc: &Document,
    password: Option<&str>,
    opts: &ExportOptions,
    holder: &str,
) -> Result<(String, FileMeta), String> {
    let bytes = crate::io::save_bytes_with(path, doc, password, opts)?;
    let (dir, file) = match path.rsplit_once('/') {
        Some((d, f)) => (format!("{d}/"), f),
        None => (String::new(), path),
    };
    let (stem, ext) = file.rsplit_once('.').unwrap_or((file, ""));
    let who = sanitise_holder(holder);
    let mut candidate = if ext.is_empty() { format!("{dir}{stem} (conflict {who})") } else { format!("{dir}{stem} (conflict {who}).{ext}") };
    for n in 2..1000 {
        if backend.stat(&candidate).is_err() {
            break;
        }
        candidate = if ext.is_empty() { format!("{dir}{stem} (conflict {who} {n})") } else { format!("{dir}{stem} (conflict {who} {n}).{ext}") };
    }
    let write_opts = wordcraft_storage::WriteOptions { if_match: None, atomic: backend.capabilities().atomic_write };
    let meta = backend.write(&candidate, &bytes, &write_opts).map_err(|e| format!("{candidate}: {e}"))?;
    Ok((candidate, meta))
}

/// What [`merge_remote_as_tracked`] did with the remote edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    /// The merged document (our edits plus theirs as tracked changes) saved over `path`.
    Saved {
        /// The document path saved.
        path: String,
        /// What the backend reports about the stored file.
        meta: FileMeta,
    },
    /// The remote moved again mid-merge: nothing overwritten, the merged document went to a
    /// conflict copy instead (see [`save_conflict_copy`]).
    ConflictCopy {
        /// The copy's path.
        path: String,
        /// What the backend reports about the stored file.
        meta: FileMeta,
    },
}

/// A temporary file this module staged, removed however the merge ends (error, race, panic).
/// Always paired with [`crate::io::write_file`]: the engine's file access goes through `io`,
/// never `std::fs`, so the automation roots stay enforced.
struct StagedFile(String);

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = crate::io::remove_file(&self.0);
    }
}

/// Fold the document currently stored under `path` into this session as tracked changes
/// (Word's Combine, via `review.compare`, labelled `author`) and save the merged document.
///
/// The save conditions on a fresh tag, so a remote that moves mid-merge still can't be
/// overwritten: then the merged document goes to a conflict copy and the outcome says so.
/// Temporary files are removed whatever happens. Paragraph-level three-way merge stays out of
/// scope (it needs stable paragraph ids first): every remote edit arrives as a tracked change
/// the user accepts or rejects, exactly as #518 prescribes.
pub fn merge_remote_as_tracked(
    backend: &dyn StorageBackend,
    path: &str,
    session: &mut Session,
    author: &str,
    opts: &ExportOptions,
) -> Result<MergeOutcome, String> {
    let remote = backend.read(path).map_err(|e| format!("{path}: {e}"))?;
    let ext = path.rsplit(['/', '\\']).next().unwrap_or(path).rsplit_once('.').map(|(_, e)| e).unwrap_or("docx");
    let tmp = std::env::temp_dir().join(format!("wordcraft-merge-{}-{}.{}", std::process::id(), merge_counter(), ext));
    let tmp_name = tmp.to_string_lossy().to_string();
    crate::io::write_file(&tmp_name, &remote)?;
    let staged = StagedFile(tmp_name.clone());
    session.run("review.compare", &serde_json::json!({"path": tmp_name, "author": author})).map_err(|e| e.to_string())?;
    drop(staged);
    let saved_password = session.password.clone();
    let password = saved_password.as_ref().map(crate::io::Password::as_str);
    session.stamp_save();
    let fresh = backend.stat(path).map_err(|e| format!("{path}: {e}"))?.etag;
    match save_to_storage_if_match(backend, path, &session.doc, password, opts, fresh.as_deref()) {
        Ok(meta) => {
            if is_full_fidelity(path) {
                session.dirty = false;
            }
            Ok(MergeOutcome::Saved { path: path.into(), meta })
        }
        Err(_) => {
            let (copy, meta) = save_conflict_copy(backend, path, &session.doc, password, opts, author)?;
            Ok(MergeOutcome::ConflictCopy { path: copy, meta })
        }
    }
}

static MERGE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn merge_counter() -> u64 {
    MERGE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::StoryRef;
    use wordcraft_storage::local::LocalBackend;

    struct Scratch {
        path: std::path::PathBuf,
    }

    impl Scratch {
        fn create(name: &str) -> Scratch {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("wordcraft-engine-storage-{name}-{}-{id}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Scratch { path }
        }

        fn backend(&self) -> LocalBackend {
            LocalBackend::open(&self.path).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn text_of(doc: &Document) -> String {
        doc.plain_text(StoryRef::Body)
    }

    #[test]
    fn docx_round_trips_backend_to_session_to_backend() {
        let scratch = Scratch::create("docx");
        let store = scratch.backend();
        let opts = ExportOptions::default();
        let doc = crate::sample::sample_document();
        let before = text_of(&doc);

        // Free functions: the backend carries exactly the serialised bytes.
        let expected = crate::io::save_bytes_with("handbook.docx", &doc, None, &opts).unwrap();
        let meta = save_to_storage(&store, "handbook.docx", &doc, None, &opts).unwrap();
        assert_eq!(meta.size, expected.len() as u64);
        assert_eq!(store.read("handbook.docx").unwrap(), expected);

        // Into a session, back out under another name: the text survives both crossings.
        let mut session = Session::new(crate::sample::sample_document());
        assert!(!session.open_from_storage(&store, "handbook.docx", None).unwrap());
        assert!(session.path.is_none(), "a storage document has no local path");
        assert!(!session.can_undo(), "opening resets history");
        assert!(text_of(&session.doc).contains(before.lines().next().unwrap_or("Open Studio")));
        let meta2 = session.save_to_storage(&store, "handbook-copy.docx", &opts).unwrap();
        assert!(!session.dirty, "a docx save clears the dirty flag");
        assert_eq!(store.read("handbook-copy.docx").unwrap().len() as u64, meta2.size);

        let mut reopened = Session::new(Document::new());
        reopened.open_from_storage(&store, "handbook-copy.docx", None).unwrap();
        assert_eq!(text_of(&reopened.doc), text_of(&session.doc));
    }

    #[test]
    fn lossy_format_save_keeps_the_dirty_flag() {
        let scratch = Scratch::create("lossy");
        let store = scratch.backend();
        let opts = ExportOptions::default();
        let mut session = Session::new(crate::sample::sample_document());
        session.dirty = true;
        session.save_to_storage(&store, "notes.txt", &opts).unwrap();
        assert!(session.dirty, "a text copy is lossy: the document still needs saving");
        assert!(store.read("notes.txt").unwrap().starts_with(b"The Open Studio"));
    }

    #[test]
    fn storage_failures_name_the_path() {
        let scratch = Scratch::create("missing");
        let store = scratch.backend();
        let err = open_from_storage(&store, "gone/memo.docx", None).unwrap_err();
        assert!(err.starts_with("gone/memo.docx: "), "{err}");
        let mut session = Session::new(Document::new());
        let err = session.open_from_storage(&store, "gone/memo.docx", None).unwrap_err();
        assert!(err.starts_with("gone/memo.docx: "), "{err}");
    }

    #[test]
    fn webdav_bearer_backend_builds_without_io() {
        // No sockets here: construction, URL shaping and scheme guards only.
        let bearer = wordcraft_storage::webdav::WebDavBackend::with_bearer("http://ocis:9200/remote.php/dav/files/user/", "t").unwrap();
        assert_eq!(bearer.base_url(), "http://ocis:9200/remote.php/dav/files/user/");
        assert!(wordcraft_storage::webdav::WebDavBackend::with_bearer("https://ocis:9200/dav/", "t").is_err(), "no TLS in this build");
        assert!(wordcraft_storage::webdav::WebDavBackend::with_bearer("http://ocis:9200/dav/", "").is_err(), "empty tokens never authenticate");
    }

    #[test]
    fn checked_save_conflicts_instead_of_overwriting() {
        let scratch = Scratch::create("checked");
        let store = scratch.backend();
        let opts = ExportOptions::default();
        let mut a = Session::new(crate::sample::sample_document());
        let first = a.save_to_storage(&store, "memo.docx", &opts).unwrap();
        let stale_tag = first.etag.clone().unwrap();

        // A second writer lands first; our tag is stale now.
        let mut b = Session::new(crate::sample::sample_document());
        b.open_from_storage(&store, "memo.docx", None).unwrap();
        b.save_to_storage(&store, "memo.docx", &opts).unwrap();

        let conflict = a.save_to_storage_checked(&store, "memo.docx", &opts, Some(&stale_tag)).unwrap_err();
        assert_eq!(conflict.path, "memo.docx");
        assert_eq!(conflict.base.as_deref(), Some(stale_tag.as_str()));
        let remote_tag = stat_storage(&store, "memo.docx").unwrap().etag.unwrap();
        assert_eq!(conflict.remote.as_deref(), Some(remote_tag.as_str()));
        // With the fresh tag the same save lands.
        a.save_to_storage_checked(&store, "memo.docx", &opts, Some(&remote_tag)).unwrap();
        assert!(!a.dirty);
    }

    #[test]
    fn conflict_copies_never_overwrite() {
        let scratch = Scratch::create("copies");
        let store = scratch.backend();
        let opts = ExportOptions::default();
        let doc = crate::sample::sample_document();
        save_to_storage(&store, "memo.docx", &doc, None, &opts).unwrap();
        let before = store.read("memo.docx").unwrap();

        let (first, _) = save_conflict_copy(&store, "memo.docx", &doc, None, &opts, "alice/tablet").unwrap();
        assert_eq!(first, "memo (conflict alice_tablet).docx", "{first}");
        let (second, _) = save_conflict_copy(&store, "memo.docx", &doc, None, &opts, "alice/tablet").unwrap();
        assert_eq!(second, "memo (conflict alice_tablet 2).docx", "{second}");
        let (third, _) = save_conflict_copy(&store, "reports/memo.docx", &doc, None, &opts, "../../evil").unwrap();
        assert_eq!(third, "reports/memo (conflict .._.._evil).docx", "{third}");
        assert_eq!(store.read("memo.docx").unwrap(), before, "the original is untouched");
    }

    #[test]
    fn merge_brings_remote_edits_in_as_tracked_changes() {
        let scratch = Scratch::create("merge");
        let store = scratch.backend();
        let opts = ExportOptions::default();
        // Two sessions diverge from the same stored version.
        let mut a = Session::new(crate::sample::sample_document());
        a.save_to_storage(&store, "memo.docx", &opts).unwrap();
        let mut b = Session::new(crate::sample::sample_document());
        b.open_from_storage(&store, "memo.docx", None).unwrap();
        a.open_from_storage(&store, "memo.docx", None).unwrap();
        b.run("document.setText", &serde_json::json!({"text": "Bob rewrote everything"})).unwrap();
        b.save_to_storage(&store, "memo.docx", &opts).unwrap();

        let outcome = merge_remote_as_tracked(&store, "memo.docx", &mut a, "bob", &opts).unwrap();
        let saved = match outcome {
            MergeOutcome::Saved { path, .. } => path,
            MergeOutcome::ConflictCopy { path, .. } => panic!("no race here; expected Saved, got copy {path}"),
        };
        assert_eq!(saved, "memo.docx");
        assert!(!a.doc.revisions.is_empty(), "remote edits arrive as tracked changes");
        let mut reopened = Session::new(Document::new());
        reopened.open_from_storage(&store, "memo.docx", None).unwrap();
        assert!(text_of(&reopened.doc).contains("Bob rewrote everything"));
    }

    #[test]
    fn lease_guarded_save_round_trip() {
        // The Phase 3 recipe end to end on a local backend: lease, conditional save, release.
        let scratch = Scratch::create("recipe");
        let store = scratch.backend();
        let opts = ExportOptions::default();
        let hold = wordcraft_storage::lease::acquire(&store, "memo.docx", "alice", "studio-mac", 120).unwrap();
        let mut session = Session::new(crate::sample::sample_document());
        let base = stat_storage(&store, "memo.docx").ok().and_then(|m| m.etag);
        session.save_to_storage_checked(&store, "memo.docx", &opts, base.as_deref()).unwrap();
        hold.renew(&store, 120).unwrap();
        hold.release(&store).unwrap();
        assert!(matches!(wordcraft_storage::lease::status(&store, "memo.docx").unwrap(), wordcraft_storage::lease::LeaseStatus::Absent));
    }
}
