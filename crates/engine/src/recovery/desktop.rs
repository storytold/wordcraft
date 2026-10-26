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
mod tests {
    use super::*;
    use crate::Session;
    use serde_json::json;
    use wordcraft_doc::StoryRef;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("wordcraft-recovery-test-{}", new_id().unwrap()));
            private_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    /// A store reopened on `root` whose scan reports an unreadable record. The record's lock can be
    /// briefly held by a process another test is starting (a forked child holds the descriptor
    /// until it execs), and a record that can't be locked is skipped, not reported: so try again
    /// for up to a second rather than read a skipped record as a clean scan.
    fn reopen_reporting_unreadable(root: &std::path::Path) -> Store {
        for _ in 0..50 {
            let mut store = Store::new(root.to_path_buf()).unwrap();
            if store.scan().is_err() {
                return store;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the scan never reported the unreadable record");
    }
    /// `store.load(id)`, tried again for up to a second while the record can't be locked: a process
    /// another test is starting holds the lock for a moment (see [`reopen_reporting_unreadable`]).
    fn load_retrying(store: &mut Store, id: &str) -> Snapshot {
        let mut last = String::new();
        for _ in 0..50 {
            match store.load(id) {
                Ok(snapshot) => return snapshot,
                Err(e) => last = e,
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the record never loaded: {last}");
    }
    fn snapshot(id: &str, text: &str) -> Snapshot {
        Snapshot {
            version: 1,
            info: RecoveryInfo { id: id.into(), name: "Synthetic draft".into(), saved_at: crate::cmd::now_iso() },
            generation: 0,
            document: Document::from_text(text),
        }
    }
    fn settle(s: &mut Session) {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            s.recovery_tick();
            if s.recovery_ready() && !s.recovery_busy() {
                break;
            }
            assert!(Instant::now() < end, "recovery did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn session(root: &Path) -> Session {
        let mut s = Session::new(Document::new());
        s.enable_recovery(root.to_owned(), Arc::new(|| {})).unwrap();
        settle(&mut s);
        s
    }
    /// Every open document's interval has passed.
    fn due_now(s: &mut Session) {
        for d in s.recovery_docs() {
            d.slot.due = Instant::now();
        }
    }
    fn save_now(s: &mut Session) {
        due_now(s);
        s.recovery_tick();
        settle(s);
    }
    /// The records in the profile folder: the copies on disk.
    fn records(root: &Path) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        ids.sort();
        ids
    }

    /// #3140: a save that lands right before the app closes can race the process exiting before
    /// the now-redundant recovery copy is actually removed, so the next launch wrongly offers to
    /// recover a document that was in fact saved cleanly. `finish_recovery` (desktop hosts call it
    /// on exit, the same as they wait for pending print jobs) must discard it synchronously
    /// instead, with no regular tick in between — simulating closing right after Save, not after
    /// however long it'd take the next normal autosave tick to notice and clean up.
    #[test]
    fn finish_recovery_discards_its_own_copy_for_a_document_saved_right_before_exit() {
        let profile = Temp::new();
        let mut s = session(&profile.0);
        s.run("document.setText", &json!({"text": "About to be saved"})).unwrap();
        save_now(&mut s);
        assert_eq!(records(&profile.0).len(), 1, "a recovery copy exists for the dirty document");
        // A real Save does this, then the app can close on the very next frame, with no regular
        // recovery_tick() in between to notice `!s.dirty` and queue the discard itself.
        s.dirty = false;
        assert_eq!(records(&profile.0).len(), 1, "confirms the race: setting dirty=false alone (what Save does) doesn't discard anything by itself");
        s.finish_recovery();
        assert!(records(&profile.0).is_empty(), "finish_recovery should have discarded the now-redundant copy before returning");
    }

    /// `tick` resets `self.id` the moment the session's document no longer matches what it last
    /// saw (so it never discards under a stale identity); `finish_recovery` must guard the same
    /// way. Switching documents and exiting immediately, with no regular tick in between to reset
    /// that identity, must not let the *new* (clean, blank) document's state discard the
    /// *previous* document's still-live recovery copy.
    #[test]
    fn finish_recovery_does_not_discard_a_previous_document_s_copy_after_switching() {
        let profile = Temp::new();
        let mut s = session(&profile.0);
        s.run("document.setText", &json!({"text": "The document the user is still working on"})).unwrap();
        save_now(&mut s);
        assert_eq!(records(&profile.0).len(), 1, "a recovery copy exists for the dirty document");
        s.set_document(Document::new()); // the document is replaced, with no tick() in between
        assert!(!s.dirty, "a freshly switched-to document starts clean");
        s.finish_recovery();
        assert_eq!(records(&profile.0).len(), 1, "replacing the document must not discard the previous one's recovery copy");
        assert_eq!(s.recovery_documents().len(), 1, "no open document keeps it: it is listed");
    }

    #[test]
    fn recovery_commands_only_address_known_profile_records() {
        use crate::file_access::{AutomationRoots, confine};
        let profile = Temp::new();
        let files = Temp::new();
        let original = files.0.join("original.txt");
        fs::write(&original, b"External synthetic original").unwrap();
        let roots = AutomationRoots::new(Some(&files.0), Some(&files.0)).unwrap().unwrap();
        let mut s = session(&profile.0);
        s.run("document.setText", &json!({"text": "Recoverable synthetic draft"})).unwrap();
        save_now(&mut s);
        drop(s);
        let mut s = session(&profile.0);
        let id = s.recovery_documents()[0].id.clone();
        confine(Some(&roots), || {
            // A command cannot use a file path or choose a different storage folder.
            for bad in [original.to_string_lossy().into_owned(), "../original.txt".into()] {
                for action in ["restore", "discard"] {
                    let err = s.run("file.recover", &json!({action: bad, "root": files.0})).unwrap_err();
                    assert!(err.to_string().contains("no such recovery copy"), "{err}");
                }
            }
            assert_eq!(s.recovery_documents().len(), 1);
            s.run("file.recover", &json!({"restore": id})).unwrap();
            settle(&mut s);
            assert!(s.path.is_none());
            assert!(s.dirty);
            assert_eq!(s.doc.plain_text(StoryRef::Body), "Recoverable synthetic draft");
            // The restored document keeps its copy there: it isn't listed, and can't be discarded.
            assert!(s.recovery_documents().is_empty());
            let err = s.run("file.recover", &json!({"discard": id})).unwrap_err();
            assert!(err.to_string().contains("no such recovery copy"), "{err}");
            settle(&mut s);
            assert_eq!(records(&profile.0), std::slice::from_ref(&id));
        });
        assert_eq!(fs::read(&original).unwrap(), b"External synthetic original");
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
    }

    #[test]
    fn failed_write_keeps_last_valid_generation_and_discovery_falls_back() {
        let t = Temp::new();
        let id = new_id().unwrap();
        let mut store = Store::new(t.0.clone()).unwrap();
        store.save(snapshot(&id, "First valid copy")).unwrap();
        let first = store.entries[&id].path.clone();
        let bytes = fs::read(&first).unwrap();
        // Simulate a final rename failure without relying on permissions (root can bypass them).
        let obstruction = t.0.join(&id).join("snapshot-00000000000000000002.json");
        fs::create_dir(&obstruction).unwrap();
        assert!(store.save(snapshot(&id, "Failed copy")).is_err());
        assert_eq!(fs::read(&first).unwrap(), bytes);
        fs::remove_dir(obstruction).unwrap();
        store.save(snapshot(&id, "Second valid copy")).unwrap();
        let second = store.entries[&id].path.clone();
        fs::write(second, b"{truncated").unwrap();
        drop(store);
        let mut reopened = reopen_reporting_unreadable(&t.0);
        assert_eq!(load_retrying(&mut reopened, &id).document.plain_text(StoryRef::Body), "First valid copy");
        // Never reuse the corrupt generation's name or prune the valid fallback.
        reopened.save(snapshot(&id, "Third valid copy")).unwrap();
        assert_eq!(fs::read(first).unwrap(), bytes);
        assert_eq!(Store::files(&t.0.join(&id)).unwrap().len(), 2);
    }

    #[test]
    fn size_limit_and_lossy_values_fail_without_committing() {
        let mut out = Limited { inner: Vec::new(), remaining: 3, exceeded: false };
        assert!(out.write_all(b"four").is_err());
        assert!(out.inner.is_empty());
        assert!(out.exceeded);
        let t = Temp::new();
        let id = new_id().unwrap();
        let mut store = Store::new(t.0.clone()).unwrap();
        store.save(snapshot(&id, "valid")).unwrap();
        let first = store.entries[&id].path.clone();
        let bytes = fs::read(&first).unwrap();
        let mut invalid = snapshot(&id, "invalid");
        invalid.document.last_section.page_w = f32::NAN;
        assert!(store.save(invalid).is_err_and(|e| e.permanent));
        assert_eq!(fs::read(first).unwrap(), bytes);
        assert_eq!(Store::files(&t.0.join(id)).unwrap().len(), 1);
    }

    /// Copies are written with picture bytes in base64 (#586 follow-up); those written before kept
    /// them as arrays of numbers and are still listed and restored, bytes intact.
    #[test]
    fn copies_with_byte_arrays_written_before_still_restore() {
        let t = Temp::new();
        let id = new_id().unwrap();
        let mut s = snapshot(&id, "Synthetic draft with a picture");
        s.generation = 1;
        s.document.media.insert("media/image1.png".into(), Arc::new(vec![0, 1, 127, 128, 254, 255]));
        let mut value = serde_json::to_value(&s).unwrap();
        assert_eq!(value["document"]["media"]["media/image1.png"], "AAF/gP7/");
        value["document"]["media"]["media/image1.png"] = json!([0, 1, 127, 128, 254, 255]);
        private_dir(&t.0.join(&id)).unwrap();
        fs::write(t.0.join(&id).join("snapshot-00000000000000000001.json"), serde_json::to_vec(&value).unwrap()).unwrap();
        let mut store = Store::new(t.0.clone()).unwrap();
        store.scan().unwrap();
        assert_eq!(store.inventory().len(), 1);
        assert_eq!(store.load(&id).unwrap().document, s.document);
    }

    #[test]
    fn snapshot_that_cannot_be_copied_is_not_retried_until_the_document_changes() {
        // #2625: a failure caused by the snapshot itself (here a value the format cannot keep;
        // in the field, the 256 MiB limit) used to be rewritten every interval while dirty.
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("document.setText", &json!({"text": "Synthetic draft"})).unwrap();
        s.doc.last_section.page_w = f32::NAN;
        save_now(&mut s);
        assert!(s.recovery_error().is_some());
        assert!(s.recovery_wait().is_none());
        due_now(&mut s);
        s.recovery_tick();
        assert!(!s.recovery_busy(), "the unchanged document was snapshotted again");
        assert!(s.recovery_error().is_some());
        // An edit tries again.
        s.doc.last_section.page_w = 612.0;
        s.run("text.insert", &json!({"text": "!"})).unwrap();
        save_now(&mut s);
        assert!(s.recovery_error().is_none());
        assert_eq!(records(&t.0).len(), 1);
    }

    #[test]
    fn failed_pruning_cannot_push_a_record_beyond_the_discovery_limit() {
        let t = Temp::new();
        let id = new_id().unwrap();
        let mut store = Store::new(t.0.clone()).unwrap();
        store.save(snapshot(&id, "Last valid copy")).unwrap();
        let first = store.entries[&id].path.clone();
        let bytes = fs::read(&first).unwrap();
        // A failed cleanup must not allow the next write to exceed the reader's bound.
        for n in 2..=MAX_FILES {
            fs::write(t.0.join(&id).join(format!("snapshot-{n:020}.json")), b"{truncated").unwrap();
        }
        assert!(store.save(snapshot(&id, "New copy")).is_err());
        assert_eq!(fs::read(first).unwrap(), bytes);
        drop(store);
        let mut reopened = reopen_reporting_unreadable(&t.0);
        assert_eq!(load_retrying(&mut reopened, &id).document.plain_text(StoryRef::Body), "Last valid copy");
    }

    #[test]
    fn failed_first_write_does_not_consume_a_record_slot() {
        let t = Temp::new();
        let mut store = Store::new(t.0.clone()).unwrap();
        let mut invalid = snapshot(&new_id().unwrap(), "Synthetic draft");
        invalid.document.last_section.page_w = f32::NAN;
        assert!(store.save(invalid).is_err());
        assert!(store.directories().unwrap().is_empty());
    }

    #[test]
    fn discovery_reclaims_a_crashed_first_write_but_keeps_valid_records() {
        let t = Temp::new();
        let id = new_id().unwrap();
        let empty = new_id().unwrap();
        let mut store = Store::new(t.0.clone()).unwrap();
        store.save(snapshot(&id, "Valid synthetic draft")).unwrap();
        drop(store);
        let dir = t.0.join(&empty);
        private_dir(&dir).unwrap();
        fs::write(dir.join(format!("pending-{}.tmp", new_id().unwrap())), b"{unfinished").unwrap();
        let mut reopened = Store::new(t.0.clone()).unwrap();
        reopened.scan().unwrap();
        assert!(!dir.exists());
        assert_eq!(load_retrying(&mut reopened, &id).document.plain_text(StoryRef::Body), "Valid synthetic draft");
    }

    #[test]
    fn live_records_are_locked_and_discard_releases_storage() {
        let t = Temp::new();
        let id = new_id().unwrap();
        let mut owner = Store::new(t.0.clone()).unwrap();
        owner.save(snapshot(&id, "live")).unwrap();
        let mut other = Store::new(t.0.clone()).unwrap();
        other.scan().unwrap();
        assert!(other.inventory().is_empty());
        assert!(other.discard(&id).is_err());
        assert!(other.discard("../outside").is_err());
        drop(owner);
        other.scan().unwrap();
        assert_eq!(other.inventory().len(), 1);
        other.discard(&id).unwrap();
        assert!(!t.0.join(id).exists(), "discard must release the record slot");
    }

    #[test]
    fn new_draft_restarts_restores_unsaved_and_explicitly_discards() {
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("text.insert", &json!({"text": "Unsaved synthetic draft"})).unwrap();
        save_now(&mut s);
        assert!(s.recovery_wait().is_none(), "unchanged documents must not schedule idle work");
        let expected = s.doc.clone();
        drop(s);
        let mut s = session(&t.0);
        let id = s.recovery_documents()[0].id.clone();
        s.run("file.recover", &json!({"restore": id})).unwrap();
        settle(&mut s);
        assert_eq!(s.doc, expected);
        assert!(s.dirty && s.path.is_none());
        assert!(s.recovery_documents().is_empty(), "the restored document keeps the copy");
        // A crash leaves the restored document's copy, in the same record.
        drop(s);
        let mut s = session(&t.0);
        assert_eq!(s.recovery_documents().len(), 1);
        s.run("file.recover", &json!({"discard": id})).unwrap();
        settle(&mut s);
        save_now(&mut s);
        assert!(s.recovery_documents().is_empty());
        assert!(records(&t.0).is_empty());
    }

    #[test]
    fn existing_file_is_unchanged_and_successful_save_removes_copy() {
        let t = Temp::new();
        let file = t.0.join("original.docx");
        let root = t.0.join("profile");
        crate::io::save_path(&file, &Document::from_text("Original business note")).unwrap();
        let original = fs::read(&file).unwrap();
        let mut s = session(&root);
        s.run("file.open", &json!({"path": file})).unwrap();
        s.run("text.insert", &json!({"text": "Unsaved edit. "})).unwrap();
        save_now(&mut s);
        let expected = s.doc.clone();
        drop(s);
        let mut s = session(&root);
        let id = s.recovery_documents()[0].id.clone();
        s.run("file.recover", &json!({"restore": id})).unwrap();
        settle(&mut s);
        assert_eq!(s.doc, expected);
        assert!(s.path.is_none() && s.dirty);
        assert_eq!(fs::read(&file).unwrap(), original);
        let saved = t.0.join("recovered.json");
        s.run("file.save", &json!({"path": saved})).unwrap();
        settle(&mut s);
        assert!(records(&root).is_empty());
        assert_eq!(fs::read(file).unwrap(), original);
    }

    #[test]
    fn password_barrier_clears_in_flight_plaintext_and_blocks_new_copies() {
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("text.insert", &json!({"text": "Confidential synthetic draft"})).unwrap();
        due_now(&mut s);
        s.recovery_tick();
        assert!(s.recovery_busy());
        s.run("file.encrypt", &json!({"password": "synthetic-password"})).unwrap();
        assert!(records(&t.0).is_empty());
        s.run("text.insert", &json!({"text": "More confidential edits"})).unwrap();
        save_now(&mut s);
        assert!(fs::read_dir(&t.0).unwrap().next().is_none());
        // Removing protection makes future recovery possible again.
        s.run("file.encrypt", &json!({"password": null})).unwrap();
        save_now(&mut s);
        assert_eq!(records(&t.0).len(), 1);
    }

    #[test]
    fn failed_password_cleanup_rejects_protection_and_preserves_snapshot() {
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("text.insert", &json!({"text": "Synthetic text"})).unwrap();
        save_now(&mut s);
        let id = records(&t.0).remove(0);
        let dir = t.0.join(id);
        // An unknown file prevents removal: don't report that plaintext was purged.
        let first = Store::files(&dir).unwrap()[0].clone();
        let bytes = fs::read(&first).unwrap();
        fs::write(dir.join("unexpected"), b"unexpected").unwrap();
        assert!(s.run("file.encrypt", &json!({"password": "synthetic-password"})).is_err());
        assert!(s.password.is_none());
        assert_eq!(fs::read(first).unwrap(), bytes);
    }

    #[test]
    fn delayed_restore_cannot_replace_new_edits() {
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("text.insert", &json!({"text": "Saved draft"})).unwrap();
        save_now(&mut s);
        drop(s);
        let mut s = session(&t.0);
        let id = s.recovery_documents()[0].id.clone();
        s.run("file.recover", &json!({"restore": id})).unwrap();
        s.run("text.insert", &json!({"text": "New work"})).unwrap();
        settle(&mut s);
        assert_eq!(s.doc.plain_text(StoryRef::Body), "New work");
        assert!(s.recovery_error().unwrap().contains("document changed"));
    }

    /// A restore asked for in one document lands in that document, even when another document
    /// is active while the copy loads.
    #[test]
    fn delayed_restore_lands_in_the_document_that_asked_for_it() {
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("text.insert", &json!({"text": "Saved draft"})).unwrap();
        save_now(&mut s);
        drop(s);
        let mut s = session(&t.0);
        let first = s.document_id();
        let id = s.recovery_documents()[0].id.clone();
        s.run("file.recover", &json!({"restore": id})).unwrap();
        s.run("file.new", &json!({})).unwrap();
        let second = s.document_id();
        s.run("text.insert", &json!({"text": "Other work"})).unwrap();
        settle(&mut s);
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Other work", "the active document is left alone");
        s.activate(first).unwrap();
        settle(&mut s);
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Saved draft");
        assert_eq!(s.with_document(second, |s| s.doc.plain_text(StoryRef::Body)).unwrap(), "Other work");
        assert_eq!(s.documents().len(), 2);
    }

    #[test]
    fn opening_protected_file_and_password_save_never_leave_plaintext_copies() {
        let t = Temp::new();
        let original = t.0.join("protected.docx");
        let root = t.0.join("profile");
        crate::io::save_path_with(
            &original,
            &Document::from_text("Synthetic confidential note"),
            Some("synthetic-password"),
            &crate::io::ExportOptions::default(),
        )
        .unwrap();
        let bytes = fs::read(&original).unwrap();
        let mut s = session(&root);
        s.run("file.open", &json!({"path": original, "password": "synthetic-password"})).unwrap();
        s.run("text.insert", &json!({"text": "Confidential edit"})).unwrap();
        save_now(&mut s);
        assert!(fs::read_dir(&root).unwrap().next().is_none());
        assert_eq!(fs::read(&original).unwrap(), bytes);
        s.run("file.new", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "New confidential draft"})).unwrap();
        save_now(&mut s);
        assert_eq!(records(&root).len(), 1);
        let saved = t.0.join("saved.docx");
        s.run("file.save", &json!({"path": saved, "password": "synthetic-password"})).unwrap();
        settle(&mut s);
        assert!(fs::read_dir(root).unwrap().next().is_none());
        assert_eq!(fs::read(original).unwrap(), bytes);
    }

    /// Each open document keeps its own recovery copy: switching between documents doesn't start
    /// a new copy of either.
    #[test]
    fn each_open_document_keeps_its_own_copy() {
        let t = Temp::new();
        let mut s = session(&t.0);
        let first = s.document_id();
        s.run("text.insert", &json!({"text": "First synthetic draft"})).unwrap();
        save_now(&mut s);
        s.run("file.new", &json!({})).unwrap();
        settle(&mut s);
        s.run("text.insert", &json!({"text": "Second synthetic draft"})).unwrap();
        save_now(&mut s);
        s.activate(first).unwrap();
        s.run("text.insert", &json!({"text": ", edited"})).unwrap();
        save_now(&mut s);
        assert_eq!(records(&t.0).len(), 2, "one copy per document");
        assert!(s.recovery_documents().is_empty(), "the copies open documents keep are not listed");
    }

    #[test]
    fn protection_after_document_switch_does_not_discard_another_draft() {
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("text.insert", &json!({"text": "First synthetic draft"})).unwrap();
        save_now(&mut s);
        let first = s.document_id();
        s.run("file.new", &json!({})).unwrap();
        // Protection can be commanded before the host's next recovery tick.
        s.run("file.encrypt", &json!({"password": "synthetic-password"})).unwrap();
        settle(&mut s);
        assert!(s.with_document(first, |s| s.recovery_slot.id.is_some()).unwrap());
        assert_eq!(records(&t.0).len(), 1, "the first draft is kept");
    }

    /// Two dirty documents, one of them parked since before its first copy: both get copies once
    /// their interval has passed, without being made active.
    #[test]
    fn parked_documents_keep_their_copies() {
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("text.insert", &json!({"text": "Parked synthetic draft"})).unwrap();
        let parked = s.document_id();
        s.run("file.new", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Active synthetic draft"})).unwrap();
        let active = s.document_id();
        assert_ne!(parked, active);
        save_now(&mut s);
        assert_eq!(s.document_id(), active, "the parked document was not made active");
        assert_eq!(records(&t.0).len(), 2, "both documents have copies");
        // An edit to the parked document reaches its copy too.
        s.with_document(parked, |s| s.run("text.insert", &json!({"text": ", edited"})).unwrap()).unwrap();
        let id = s.with_document(parked, |s| s.recovery_slot.id.clone()).unwrap().unwrap();
        save_now(&mut s);
        drop(s);
        let mut store = Store::new(t.0.clone()).unwrap();
        store.scan().unwrap();
        assert_eq!(load_retrying(&mut store, &id).document.plain_text(StoryRef::Body), "Parked synthetic draft, edited");
    }

    /// Closing a dirty document and saving another delete both their copies; a third document
    /// keeps its own.
    #[test]
    fn closing_or_saving_a_document_deletes_only_its_copy() {
        let t = Temp::new();
        let files = Temp::new();
        let mut s = session(&t.0);
        let mut ids = Vec::new();
        for text in ["Closed draft", "Saved draft", "Kept draft"] {
            if !ids.is_empty() {
                s.run("file.new", &json!({})).unwrap();
            }
            s.run("text.insert", &json!({"text": text})).unwrap();
            ids.push(s.document_id());
        }
        save_now(&mut s);
        assert_eq!(records(&t.0).len(), 3);
        let kept = s.recovery_slot.id.clone().unwrap();
        s.close_document(ids[0]).unwrap();
        s.with_document(ids[1], |s| s.run("file.save", &json!({"path": files.0.join("saved.json")})).unwrap()).unwrap();
        settle(&mut s);
        assert_eq!(records(&t.0), [kept], "only the open dirty document's copy is left");
        assert!(s.recovery_documents().is_empty());
    }

    /// A crash with two dirty documents leaves two copies; the next run lists both and restores
    /// each into a document of its own, beside the work already there.
    #[test]
    fn a_crash_leaves_every_dirty_document_s_copy() {
        let t = Temp::new();
        let mut s = session(&t.0);
        s.run("text.insert", &json!({"text": "First synthetic draft"})).unwrap();
        s.run("file.new", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Second synthetic draft"})).unwrap();
        save_now(&mut s);
        drop(s); // no finish_recovery: the process died
        let mut s = session(&t.0);
        let copies = s.recovery_documents();
        assert_eq!(copies.len(), 2);
        s.run("text.insert", &json!({"text": "Work in progress"})).unwrap();
        let work = s.document_id();
        for copy in &copies {
            s.run("file.recover", &json!({"restore": copy.id})).unwrap();
            settle(&mut s);
        }
        assert!(s.recovery_documents().is_empty(), "the restored documents keep their copies");
        let mut texts: Vec<String> = s.documents().iter().map(|d| s.with_document(d.id, |s| s.doc.plain_text(StoryRef::Body)).unwrap()).collect();
        texts.sort();
        assert_eq!(texts, ["First synthetic draft", "Second synthetic draft", "Work in progress"]);
        assert!(s.with_document(work, |s| s.dirty).unwrap());
    }

    /// A clean exit deletes the copies of documents saved or closed since their last copy, the
    /// parked ones' too, and keeps the dirty ones'.
    #[test]
    fn finish_recovery_covers_every_open_document() {
        let t = Temp::new();
        let files = Temp::new();
        let mut s = session(&t.0);
        let mut ids = Vec::new();
        for text in ["Saved draft", "Closed draft", "Dirty draft"] {
            if !ids.is_empty() {
                s.run("file.new", &json!({})).unwrap();
            }
            s.run("text.insert", &json!({"text": text})).unwrap();
            ids.push(s.document_id());
        }
        save_now(&mut s);
        assert_eq!(records(&t.0).len(), 3);
        let dirty = s.recovery_slot.id.clone().unwrap();
        // No tick between these and the exit.
        s.with_document(ids[0], |s| s.run("file.save", &json!({"path": files.0.join("saved.json")})).unwrap()).unwrap();
        s.close_document(ids[1]).unwrap();
        s.finish_recovery();
        assert_eq!(records(&t.0), [dirty]);
    }
}
