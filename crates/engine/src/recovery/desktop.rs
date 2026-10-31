use super::RecoveryInfo;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use wordcraft_doc::Document;

const INTERVAL: Duration = Duration::from_secs(10);
const MAX_BYTES: u64 = 256 << 20;
const MAX_RECORDS: usize = 128;
const MAX_FILES: usize = 16;
const DISK_BUDGET: u64 = 2 << 30;
/// Times a record lock held for a moment (by a process being started) is tried again.
const LOCK_RETRIES: u32 = 10;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    version: u32,
    pub info: RecoveryInfo,
    generation: u64,
    pub document: Document,
}
/// A failed snapshot write. `permanent` failures come from the snapshot itself (its size, the
/// folder or disk budget, a value the format cannot keep): writing the same document again would
/// fail the same way, so it is not retried until the document changes (#2625).
#[derive(Debug)]
struct SaveError {
    message: String,
    permanent: bool,
}
impl SaveError {
    fn permanent(message: impl Into<String>) -> Self {
        Self { message: message.into(), permanent: true }
    }
}
impl From<String> for SaveError {
    fn from(message: String) -> Self {
        Self { message, permanent: false }
    }
}
impl From<&str> for SaveError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
struct Entry {
    info: RecoveryInfo,
    path: PathBuf,
}
struct Store {
    root: PathBuf,
    entries: BTreeMap<String, Entry>,
    // Advisory locks prevent another running instance from restoring/discarding a live copy.
    locks: BTreeMap<String, File>,
}
fn private_dir(path: &Path) -> io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(path)
}
fn private_file(path: &Path, exclusive: bool) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.read(true).write(true);
    if exclusive {
        o.create_new(true);
    } else {
        o.create(true).truncate(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}
fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
fn id_ok(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn new_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn generation(path: &Path) -> Option<u64> {
    path.file_name()?.to_str()?.strip_prefix("snapshot-")?.strip_suffix(".json")?.parse().ok()
}
fn read_snapshot(path: &Path) -> Result<Snapshot, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err("not a regular recovery file, or it exceeds 256 MiB".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("not a regular recovery file".into());
    }
    let s: Snapshot = serde_json::from_reader(BufReader::new(file.take(MAX_BYTES + 1))).map_err(|e| e.to_string())?;
    if s.version != 1 || !id_ok(&s.info.id) {
        return Err("unsupported recovery file".into());
    }
    Ok(s)
}
impl Store {
    fn new(root: PathBuf) -> Result<Self, String> {
        private_dir(&root).map_err(|e| e.to_string())?;
        Ok(Self { root, entries: BTreeMap::new(), locks: BTreeMap::new() })
    }
    fn directories(&self) -> Result<Vec<PathBuf>, String> {
        let mut dirs = Vec::new();
        for e in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            if e.file_type().map_err(|e| e.to_string())?.is_dir() && e.file_name().to_str().is_some_and(id_ok) {
                if dirs.len() >= MAX_RECORDS {
                    return Err("recovery folder has too many records".into());
                }
                dirs.push(e.path());
            }
        }
        Ok(dirs)
    }
    fn files(dir: &Path) -> Result<Vec<PathBuf>, String> {
        let mut paths = Vec::new();
        for e in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            if e.file_type().map_err(|e| e.to_string())?.is_file() && generation(&e.path()).is_some() {
                if paths.len() >= MAX_FILES {
                    return Err("recovery record has too many generations".into());
                }
                paths.push(e.path());
            }
        }
        paths.sort_by_key(|p| std::cmp::Reverse(generation(p)));
        Ok(paths)
    }
    /// Locks a record's `owner.lock`, retrying [`LOCK_RETRIES`] times 10 ms apart.
    fn lock(&self, id: &str) -> Result<File, String> {
        if !id_ok(id) {
            return Err("invalid recovery id".into());
        }
        let file = private_file(&self.root.join(id).join("owner.lock"), false).map_err(|e| e.to_string())?;
        // A process another part of the app (or a test) is starting holds the lock for a moment
        // (a forked child shares it until it execs): wait that out, but not a live owner.
        for _ in 0..LOCK_RETRIES {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(format!("recovery copy is in use or cannot be locked: {e}")),
            }
        }
        file.try_lock().map_err(|e| format!("recovery copy is in use or cannot be locked: {e}"))?;
        Ok(file)
    }
    fn acquire(&mut self, id: &str) -> Result<(), String> {
        if !self.locks.contains_key(id) {
            let lock = self.lock(id)?;
            self.locks.insert(id.to_string(), lock);
        }
        Ok(())
    }
    fn scan(&mut self) -> Result<(), String> {
        let mut entries = BTreeMap::new();
        let mut unreadable = false;
        for dir in self.directories()? {
            let Some(id) = dir.file_name().and_then(std::ffi::OsStr::to_str) else { continue };
            let guard = if self.locks.contains_key(id) {
                None
            } else {
                match self.lock(id) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        log::debug!("recovery: {e}");
                        continue;
                    }
                }
            };
            // A crashed writer can leave an incomplete temporary file. Only clean records
            // we have locked, so another instance's write is never interrupted.
            for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_type().map_err(|e| e.to_string())?.is_file()
                    && entry.file_name().to_str().is_some_and(|n| n.strip_prefix("pending-").and_then(|n| n.strip_suffix(".tmp")).is_some_and(id_ok))
                {
                    fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
                }
            }
            for path in Self::files(&dir)? {
                match read_snapshot(&path) {
                    Ok(s) if s.info.id == id && Some(s.generation) == generation(&path) => {
                        entries.insert(id.to_string(), Entry { info: s.info, path });
                        break;
                    }
                    Ok(_) => {
                        unreadable = true;
                        log::warn!("recovery: mismatched record {}", path.display());
                    }
                    Err(e) => {
                        unreadable = true;
                        log::warn!("recovery: unreadable {}: {e}", path.display());
                    }
                }
            }
            drop(guard);
            // A crash before the first commit leaves no document to recover. Reclaim
            // that slot only under a fresh lock and the discard path's entry checks.
            if Self::files(&dir)?.is_empty()
                && let Err(e) = self.discard(id)
            {
                log::warn!("recovery: cannot remove empty record: {e}");
            }
        }
        self.entries = entries;
        if unreadable { Err("Some recovery files could not be read. Earlier valid copies are listed when available.".into()) } else { Ok(()) }
    }
    fn inventory(&self) -> Vec<RecoveryInfo> {
        let mut v: Vec<_> = self.entries.values().map(|e| e.info.clone()).collect();
        v.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then(a.id.cmp(&b.id)));
        v
    }
    fn disk_used(&self) -> Result<u64, String> {
        let mut used = 0u64;
        for dir in self.directories()? {
            for p in Self::files(&dir)? {
                used = used.saturating_add(fs::metadata(p).map_err(|e| e.to_string())?.len());
            }
        }
        Ok(used)
    }
    fn save(&mut self, s: Snapshot) -> Result<(), SaveError> {
        let id = s.info.id.clone();
        let result = self.write_snapshot(s);
        // Include failures before temp-file creation (quota checks, locking, etc.).
        // A record containing any committed generation is never removed here.
        if result.is_err()
            && Self::files(&self.root.join(&id)).is_ok_and(|files| files.is_empty())
            && let Err(e) = self.discard(&id)
        {
            log::warn!("recovery: cannot remove failed empty record: {e}");
        }
        result
    }
    fn write_snapshot(&mut self, mut s: Snapshot) -> Result<(), SaveError> {
        let dir = self.root.join(&s.info.id);
        if !dir.exists() {
            if self.directories()?.len() >= MAX_RECORDS {
                return Err(SaveError::permanent("recovery folder is full; discard an older copy"));
            }
            private_dir(&dir).map_err(|e| e.to_string())?;
        }
        self.acquire(&s.info.id)?;
        let files = Self::files(&dir)?;
        if files.len() >= MAX_FILES {
            return Err("older recovery generations could not be pruned; keeping the last valid copy".into());
        }
        s.generation = files.first().and_then(|p| generation(p)).unwrap_or(0).checked_add(1).ok_or("recovery generation overflow")?;
        let previous = self.entries.get(&s.info.id).map(|e| e.path.clone());
        let limit = MAX_BYTES.min(DISK_BUDGET.saturating_sub(self.disk_used()?));
        let tmp = dir.join(format!("pending-{}.tmp", new_id()?));
        let final_path = dir.join(format!("snapshot-{:020}.json", s.generation));
        let result = (|| -> Result<(), SaveError> {
            let file = private_file(&tmp, true).map_err(|e| e.to_string())?;
            let mut writer = Limited { inner: BufWriter::new(file), remaining: limit, exceeded: false };
            let written = serde_json::to_writer(&mut writer, &s).map_err(|e| e.to_string()).and_then(|()| writer.flush().map_err(|e| e.to_string()));
            if let Err(e) = written {
                return Err(SaveError { message: e, permanent: writer.exceeded });
            }
            writer.inner.get_ref().sync_all().map_err(|e| e.to_string())?;
            drop(writer);
            // The copy was written and synchronized; one that does not decode back to the same
            // document fails the same way every time.
            let check = read_snapshot(&tmp).map_err(SaveError::permanent)?;
            if check.document != s.document {
                return Err(SaveError::permanent("recovery format did not preserve the full document"));
            }
            // A new generation is never written over the last valid file.
            fs::rename(&tmp, &final_path).map_err(|e| e.to_string())?;
            sync_dir(&dir).map_err(|e| e.to_string())?;
            self.entries.insert(s.info.id.clone(), Entry { info: s.info, path: final_path.clone() });
            for old in Self::files(&dir)?.into_iter().filter(|p| p != &final_path && Some(p) != previous.as_ref()) {
                if let Err(e) = fs::remove_file(old) {
                    log::warn!("recovery: cannot prune old generation: {e}");
                }
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }
    fn load(&mut self, id: &str) -> Result<Snapshot, String> {
        if !self.entries.contains_key(id) {
            return Err("no such recovery copy".into());
        }
        self.acquire(id)?;
        read_snapshot(&self.entries.get(id).ok_or("no such recovery copy")?.path)
    }
    fn discard(&mut self, id: &str) -> Result<(), String> {
        if !id_ok(id) {
            return Err("invalid recovery id".into());
        }
        let dir = self.root.join(id);
        if !dir.exists() {
            self.entries.remove(id);
            return Ok(());
        }
        self.acquire(id)?;
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let known = name == "owner.lock"
                || generation(&entry.path()).is_some()
                || name.to_str().is_some_and(|n| n.strip_prefix("pending-").and_then(|n| n.strip_suffix(".tmp")).is_some_and(id_ok));
            if !known || !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                return Err("recovery folder contains an unexpected entry".into());
            }
        }
        for p in Self::files(&dir)? {
            fs::remove_file(p).map_err(|e| e.to_string())?;
        }
        for e in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            let name = e.file_name();
            if name.to_str().is_some_and(|n| n.strip_prefix("pending-").and_then(|n| n.strip_suffix(".tmp")).is_some_and(id_ok)) {
                fs::remove_file(e.path()).map_err(|e| e.to_string())?;
            }
        }
        sync_dir(&dir).map_err(|e| e.to_string())?;
        self.entries.remove(id);
        self.locks.remove(id);
        fs::remove_file(dir.join("owner.lock")).map_err(|e| e.to_string())?;
        fs::remove_dir(&dir).map_err(|e| e.to_string())?;
        sync_dir(&self.root).map_err(|e| e.to_string())?;
        Ok(())
    }
}
struct Limited<W> {
    inner: W,
    remaining: u64,
    exceeded: bool,
}
impl<W: Write> Write for Limited<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            self.exceeded = true;
            return Err(io::Error::other("recovery snapshot or folder size limit exceeded"));
        }
        let n = self.inner.write(bytes)?;
        self.remaining -= n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
enum Job {
    /// Write `document`'s copy.
    Save(Box<Snapshot>, u64),
    Scan,
    Load(String, Request),
    /// Delete a record: an open document's copy it no longer needs, a closed document's, or one
    /// from an earlier run that the user discarded (`None`).
    Discard(String, Option<u64>),
}
enum Outcome {
    Done,
    Discarded(String),
    Loaded(Box<Snapshot>, Request),
}
pub(crate) struct Event {
    documents: Vec<RecoveryInfo>,
    outcome: Result<Outcome, String>,
    /// The failure would repeat for the same document; see [`SaveError`].
    permanent: bool,
    /// The open document a failed save or discard was for.
    document: Option<u64>,
    /// The record a failed discard was for.
    discard: Option<String>,
}
/// Who asked for a restore, and whether the copy replaces that document (an untouched blank
/// one) or opens as another document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub document: u64,
    pub revision: u64,
    pub replace: bool,
}
/// One open document's recovery state: the record its copy is kept in, the revision that copy
/// holds, and when the next copy may be written. Each open document has its own, parked with it.
#[derive(Clone, Debug)]
pub(crate) struct Slot {
    pub id: Option<String>,
    pub revision: Option<u64>,
    pub due: Instant,
    /// A failed discard of this document's copy is tried again from then on.
    pub retry: Option<Instant>,
}
impl Default for Slot {
    fn default() -> Self {
        Self { id: None, revision: None, due: Instant::now() + INTERVAL, retry: None }
    }
}
impl Slot {
    /// The document now keeps its copy in record `id`, which holds `revision`.
    pub fn adopt(&mut self, id: String, revision: u64) {
        self.id = Some(id);
        self.revision = Some(revision);
        self.due = Instant::now() + INTERVAL;
        self.retry = None;
    }
}
/// A worker report, for the session to apply to its documents.
pub(crate) enum Applied {
    Nothing,
    /// Record `id` is gone: no document keeps its copy there any more.
    Discarded(String),
    /// A restored copy, to open.
    Loaded(Box<Snapshot>, Request),
    /// Writing or discarding `document`'s copy failed.
    Failed {
        document: u64,
        discard: bool,
        permanent: bool,
    },
}
impl Slot {
    /// Take in a report about this document's copy.
    pub fn apply(&mut self, applied: &Applied) {
        match applied {
            Applied::Discarded(id) if self.id.as_deref() == Some(id) => {
                self.id = None;
                self.retry = None;
            }
            Applied::Failed { discard, permanent, .. } => {
                let now = Instant::now();
                if *discard {
                    self.retry = Some(now + INTERVAL);
                } else if !permanent {
                    // A transient failure (a lock, an I/O error) is retried at the next
                    // interval; one the snapshot itself causes waits for the next edit.
                    self.revision = None;
                }
                self.due = now + INTERVAL;
            }
            _ => {}
        }
    }
}
/// What [`Recovery::tick`] reads from an open document, and the document's [`Slot`].
pub(crate) struct DocRecovery<'a> {
    pub id: u64,
    /// Dirty and without a password: the document should have a current copy.
    pub wants_copy: bool,
    pub rev: u64,
    pub doc: &'a Document,
    pub path: Option<&'a Path>,
    pub slot: &'a mut Slot,
}
/// The process's recovery worker: one thread and one store for every open document, so records
/// are locked once per process and the list of copies is the same whichever document is shown.
pub(crate) struct Recovery {
    tx: mpsc::SyncSender<Job>,
    rx: mpsc::Receiver<Event>,
    /// Every record the store found or wrote, this process's own included.
    pub documents: Vec<RecoveryInfo>,
    pub ready: bool,
    pub busy: bool,
    pub error: Option<String>,
    alive: bool,
    /// Records of closed documents waiting to be deleted, and whether that is queued.
    closed: Vec<(String, bool)>,
    /// A loaded copy that replaces its (untouched blank) document once that document is active.
    pub held: Option<(Box<Snapshot>, Request)>,
}
impl Recovery {
    pub fn start(root: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        let (tx, jobs) = mpsc::sync_channel(1);
        let (events, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("wordcraft-recovery".into())
            .spawn(move || {
                let mut store = match Store::new(root) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = events.send(Event { documents: Vec::new(), outcome: Err(e), permanent: false, document: None, discard: None });
                        wake();
                        return;
                    }
                };
                let initial = store.scan().map(|_| Outcome::Done);
                if events.send(Event { documents: store.inventory(), outcome: initial, permanent: false, document: None, discard: None }).is_err() {
                    return;
                }
                wake();
                while let Ok(job) = jobs.recv() {
                    let mut permanent = false;
                    let (mut document, mut discard) = (None, None);
                    let result = match job {
                        Job::Save(s, d) => {
                            document = Some(d);
                            store.save(*s).map(|_| Outcome::Done).map_err(|e| {
                                permanent = e.permanent;
                                e.message
                            })
                        }
                        Job::Scan => store.scan().map(|_| Outcome::Done),
                        Job::Load(id, request) => store.load(&id).map(|s| Outcome::Loaded(Box::new(s), request)),
                        Job::Discard(id, d) => {
                            document = d;
                            let r = store.discard(&id).map(|_| Outcome::Discarded(id.clone()));
                            discard = Some(id);
                            r
                        }
                    };
                    if result.is_ok() {
                        (document, discard) = (None, None);
                    }
                    if events.send(Event { documents: store.inventory(), outcome: result, permanent, document, discard }).is_err() {
                        break;
                    }
                    wake();
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { tx, rx, documents: Vec::new(), ready: false, busy: true, error: None, alive: true, closed: Vec::new(), held: None })
    }
    fn queue(&mut self, job: Job) -> Result<(), String> {
        if !self.alive {
            return Err("recovery worker is unavailable".into());
        }
        if self.busy {
            return Err("recovery is busy; try again when it finishes".into());
        }
        self.tx.try_send(job).map_err(|e| e.to_string())?;
        self.busy = true;
        self.error = None;
        Ok(())
    }
    /// The worker's answer to the job under way: waits for it when `block`, else only takes one
    /// that has arrived.
    pub fn receive(&mut self, block: bool) -> Option<Event> {
        let e = if block {
            if !self.busy {
                return None;
            }
            self.rx.recv().map_err(|_| mpsc::TryRecvError::Disconnected)
        } else {
            self.rx.try_recv()
        };
        match e {
            Ok(e) => {
                self.busy = false;
                self.ready = true;
                self.documents.clone_from(&e.documents);
                Some(e)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.busy = false;
                self.alive = false;
                if self.error.is_none() {
                    self.error = Some("recovery worker is unavailable".into());
                }
                None
            }
        }
    }
    /// Run `job` now and wait for its answer.
    fn run_now(&mut self, job: Job) -> Result<Event, String> {
        self.queue(job)?;
        self.receive(true).ok_or_else(|| "recovery worker is unavailable".to_string())
    }
    /// Discard record `id` now (the worker must be idle) and wait for it.
    pub fn discard_now(&mut self, id: &str, document: Option<u64>) -> Result<Event, String> {
        self.run_now(Job::Discard(id.into(), document))
    }
    /// A closed document's record: deleted at the next chance, and not listed meanwhile.
    pub fn close(&mut self, id: String) {
        if !self.closed.iter().any(|(c, _)| *c == id) {
            self.closed.push((id, false));
        }
    }
    /// Records the open documents of this process don't hold any more but haven't deleted yet.
    pub fn closing(&self) -> impl Iterator<Item = &str> {
        self.closed.iter().map(|(id, _)| id.as_str())
    }
    /// Closed documents' records not yet queued for deletion.
    pub fn take_closed(&mut self) -> Vec<String> {
        self.closed
            .iter_mut()
            .filter(|(_, queued)| !*queued)
            .map(|(id, queued)| {
                *queued = true;
                id.clone()
            })
            .collect()
    }
    /// Take in what the worker reported: the record lists here, and what the session applies
    /// to its documents.
    pub fn apply(&mut self, e: Event) -> Applied {
        match e.outcome {
            Ok(Outcome::Done) => Applied::Nothing,
            Ok(Outcome::Discarded(id)) => {
                self.closed.retain(|(c, _)| *c != id);
                Applied::Discarded(id)
            }
            Ok(Outcome::Loaded(s, request)) => Applied::Loaded(s, request),
            Err(message) => {
                self.error = Some(message);
                if let Some(id) = &e.discard {
                    // A closed document's copy that can't be deleted stays listed, for the user
                    // to restore or discard.
                    self.closed.retain(|(c, _)| c != id);
                }
                match e.document {
                    Some(document) => Applied::Failed { document, discard: e.discard.is_some(), permanent: e.permanent },
                    None => Applied::Nothing,
                }
            }
        }
    }
    /// Queue at most one job: a closed document's discard first, then, document by document in
    /// the order given, a discard of a copy its document no longer needs (saved, closed without
    /// saving, password-protected) or a new copy of a dirty document whose interval has passed.
    pub fn tick<'a>(&mut self, docs: impl Iterator<Item = DocRecovery<'a>>, now: Instant) -> Result<(), String> {
        if self.busy || !self.ready || !self.alive {
            return Ok(());
        }
        if let Some(at) = self.closed.iter().position(|(_, queued)| !queued) {
            let id = self.closed.get(at).map(|(id, _)| id.clone()).ok_or("recovery record list changed")?;
            self.queue(Job::Discard(id, None))?;
            if let Some(c) = self.closed.get_mut(at) {
                c.1 = true;
            }
            return Ok(());
        }
        for d in docs {
            if !d.wants_copy {
                if let Some(id) = d.slot.id.clone()
                    && d.slot.retry.is_none_or(|t| now >= t)
                {
                    return self.queue(Job::Discard(id, Some(d.id)));
                }
                continue;
            }
            if d.slot.revision == Some(d.rev) || now < d.slot.due {
                continue;
            }
            let id = match &d.slot.id {
                Some(id) => id.clone(),
                None => new_id()?,
            };
            let name = d.path.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled draft".into());
            let info = RecoveryInfo { id: id.clone(), name, saved_at: crate::cmd::now_iso() };
            self.queue(Job::Save(Box::new(Snapshot { version: 1, info, generation: 0, document: d.doc.clone() }), d.id))?;
            d.slot.id = Some(id);
            d.slot.revision = Some(d.rev);
            d.slot.due = now + INTERVAL;
            return Ok(());
        }
        Ok(())
    }
    /// How long until [`tick`](Self::tick) has work, for a host that repaints only on demand.
    pub fn wait<'a>(&self, docs: impl Iterator<Item = (bool, u64, &'a Slot)>) -> Option<Duration> {
        if !self.alive || !self.ready || self.busy {
            return None;
        }
        let now = Instant::now();
        let min = Duration::from_millis(100);
        if self.closed.iter().any(|(_, queued)| !queued) || self.held.as_ref().is_some_and(|(_, r)| !r.replace) {
            return Some(min);
        }
        docs.filter_map(|(wants_copy, rev, slot)| match (wants_copy, &slot.id) {
            (false, Some(_)) => Some(slot.retry.map_or(min, |t| t.saturating_duration_since(now))),
            (false, None) => None,
            (true, _) if slot.revision == Some(rev) => None,
            (true, _) => Some(slot.due.saturating_duration_since(now)),
        })
        .min()
        .map(|d| d.max(min))
    }
    pub fn restore(&mut self, id: &str, request: Request) -> Result<(), String> {
        if !self.documents.iter().any(|d| d.id == id) {
            return Err("no such recovery copy".into());
        }
        if self.held.is_some() {
            return Err("another restored copy is waiting for its document".into());
        }
        self.queue(Job::Load(id.into(), request))
    }
    pub fn discard(&mut self, id: &str) -> Result<(), String> {
        if !self.documents.iter().any(|d| d.id == id) {
            return Err("no such recovery copy".into());
        }
        self.queue(Job::Discard(id.into(), None))
    }
    pub fn refresh(&mut self) -> Result<(), String> {
        self.queue(Job::Scan)
    }
}

#[cfg(test)]
mod tests;
