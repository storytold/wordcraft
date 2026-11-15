//! Library thumbnails drawn off the UI thread (#2868). Decoding a 24–48 MP camera photo to draw
//! its thumbnail takes the best part of a second, so a pane that drew them as it scrolled would
//! stall. A [`Drawer`] draws them on a worker thread of its own instead: the pane asks
//! ([`Drawer::ask`]), gets the thumbnail at once when it has been drawn, and otherwise
//! [`Answer::Drawing`] while it is drawn; the worker wakes the front end when one is finished,
//! and [`Drawer::take_done`] hands it over.
//!
//! Every bound is here: one worker, so at most one file ([`MAX_FILE_BYTES`]) and one decode
//! ([`wordcraft_geom::MAX_DECODED_BYTES`]) in memory at a time, as drawing one on the UI thread
//! held; at most [`MAX_QUEUE`] pictures waiting, newest first (the ones in view), the oldest
//! dropped past it; and at most [`MAX_KEPT`] finished thumbnails ([`KEPT_BYTES`] of PNG) kept.
//! [`Drawer::cancel`] drops what is waiting when the person moves on (another folder, the pane
//! closed); the picture being drawn can't be stopped half-way and finishes.
//!
//! The worker reads through [`crate::io::library_read`] under the automation read roots that
//! were in force where the picture was asked for, so confinement applies to its reads as to the
//! commands'. It opens and draws through the Library's [`Kind`], so what the Library lists is
//! what it draws, and a panic in a decoder is that picture's error, not the worker's end.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use craft_library::Kind;
use serde_json::{Value, json};

use super::{Confined, MAX_FILE_BYTES, MAX_ITEMS, MAX_THUMB_HEIGHT, Pictures, Stamp, THUMB_WIDTH, THUMB_WIDTHS, Thumbnail};
use crate::Session;
use crate::file_access::{self, AutomationRoots};

/// Pictures waiting at once. The pane the Craft apps share keeps 256 thumbnails, and asks for a
/// picture only when it comes into view, so more than this many waiting are ones it has let go.
pub const MAX_QUEUE: usize = 256;
/// Finished thumbnails kept, and the PNG bytes they may add up to (the Library's own cache keeps
/// as much), the least recently asked for dropped first.
pub const MAX_KEPT: usize = 1_024;
pub const KEPT_BYTES: usize = 32 << 20;

/// A drawer's bounds; [`Limits::default`] is the constants above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub queue: usize,
    pub kept: usize,
    pub kept_bytes: usize,
    /// The largest file read, in bytes.
    pub file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { queue: MAX_QUEUE, kept: MAX_KEPT, kept_bytes: KEPT_BYTES, file_bytes: MAX_FILE_BYTES }
    }
}

/// One thumbnail: item `item` of the file at `path` (made real by the Library), `width` pixels wide.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub path: PathBuf,
    pub item: usize,
    pub width: u32,
}

/// What [`Drawer::ask`] answers.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// The thumbnail, as `library.thumbnail` answers it: `{png: base64, width, height}`.
    Drawn(Value),
    /// Being drawn: [`Drawer::take_done`] gives it with this key once it is finished, and asking
    /// again then answers it.
    Drawing(Key),
}

struct Job {
    key: Key,
    stamp: Stamp,
    /// The automation read roots in force where it was asked for; `None`: none.
    roots: Option<Arc<AutomationRoots>>,
}

struct Kept {
    stamp: Stamp,
    thumb: Result<Thumbnail, String>,
    used: u64,
}

impl Kept {
    fn bytes(&self) -> usize {
        self.thumb.as_ref().map_or(0, |t| t.png.len())
    }
}

#[derive(Default)]
struct Inner {
    /// Newest last; the worker takes from the back.
    queue: VecDeque<Job>,
    /// The picture being drawn.
    busy: Option<Key>,
    kept: HashMap<Key, Kept>,
    kept_bytes: usize,
    /// Finished since [`Drawer::take_done`] last took them.
    done: VecDeque<(Key, Result<Thumbnail, String>)>,
    tick: u64,
    /// The drawer was dropped: the worker stops.
    closed: bool,
    /// The worker is running.
    alive: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    work: Condvar,
    limits: Limits,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    /// A worker that panicked while holding the lock left nothing half-changed that matters
    /// here, so a poisoned lock is used as it is.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Draws Library thumbnails on a worker thread; dropping it stops the worker after the picture
/// it is drawing (it is not waited for).
pub struct Drawer<K: Kind = Pictures> {
    shared: Arc<Shared>,
    kind: std::marker::PhantomData<fn() -> K>,
}

impl<K: Kind> Drop for Drawer<K> {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.work.notify_all();
    }
}

/// Marks the worker as gone however it ends.
struct Alive(Arc<Shared>);

impl Drop for Alive {
    fn drop(&mut self) {
        let mut g = self.0.lock();
        g.alive = false;
        g.busy = None;
    }
}

impl<K: Kind + Send + 'static> Drawer<K> {
    /// Start a worker drawing `kind`'s thumbnails; `wake` is called (on the worker) each time
    /// one is finished. The browser build has no threads: there it fails, and thumbnails are
    /// drawn as the pane asks.
    pub fn start(kind: K, wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        Self::with_limits(kind, Limits::default(), wake)
    }

    pub fn with_limits(kind: K, limits: Limits, wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        if cfg!(target_arch = "wasm32") {
            return Err("thumbnails are drawn as they are asked for in the browser build".into());
        }
        let limits = Limits { queue: limits.queue.max(1), kept: limits.kept.max(1), ..limits };
        let shared =
            Arc::new(Shared { inner: Mutex::new(Inner { alive: true, ..Inner::default() }), work: Condvar::new(), limits, wake: Box::new(wake) });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("library-thumbnails".into())
            .spawn(move || work(kind, worker))
            .map_err(|e| format!("the thumbnail drawer could not start: {e}"))?;
        Ok(Drawer { shared, kind: std::marker::PhantomData })
    }
}

impl<K: Kind> Drawer<K> {
    /// The thumbnail `key` names, when it has been drawn from the file as `stamp` describes it.
    pub fn get(&self, key: &Key, stamp: Stamp) -> Option<Result<Thumbnail, String>> {
        let mut g = self.shared.lock();
        g.tick = g.tick.wrapping_add(1);
        let now = g.tick;
        let kept = g.kept.get_mut(key).filter(|k| k.stamp == stamp)?;
        kept.used = now;
        Some(kept.thumb.clone())
    }

    /// Draw `key`'s thumbnail of the file as `stamp` describes it, before anything asked for
    /// earlier; asking again for one that is waiting moves it to the front. Fails when the
    /// worker has stopped.
    pub fn request(&self, key: Key, stamp: Stamp) -> Result<(), String> {
        let roots = file_access::current();
        let mut g = self.shared.lock();
        if !g.alive || g.closed {
            return Err("the thumbnail drawer has stopped".into());
        }
        if g.busy.as_ref() == Some(&key) || g.kept.get(&key).is_some_and(|k| k.stamp == stamp) {
            return Ok(());
        }
        g.queue.retain(|j| j.key != key);
        g.queue.push_back(Job { key, stamp, roots });
        while g.queue.len() > self.shared.limits.queue {
            g.queue.pop_front();
        }
        drop(g);
        self.shared.work.notify_one();
        Ok(())
    }

    /// Drop every picture waiting (the one being drawn finishes); how many were dropped.
    pub fn cancel(&self) -> usize {
        let mut g = self.shared.lock();
        let n = g.queue.len();
        g.queue.clear();
        n
    }

    /// The thumbnails finished since this was last asked, oldest first (at most as many as may
    /// wait, and as many bytes as are kept: the oldest go past them).
    pub fn take_done(&self) -> Vec<(Key, Result<Thumbnail, String>)> {
        self.shared.lock().done.drain(..).collect()
    }

    /// Pictures waiting.
    pub fn queued(&self) -> usize {
        self.shared.lock().queue.len()
    }

    /// Is `key` waiting or being drawn?
    pub fn pending(&self, key: &Key) -> bool {
        let g = self.shared.lock();
        g.busy.as_ref() == Some(key) || g.queue.iter().any(|j| &j.key == key)
    }

    /// Is a picture being drawn?
    pub fn busy(&self) -> bool {
        self.shared.lock().busy.is_some()
    }

    /// Is the worker running?
    pub fn alive(&self) -> bool {
        self.shared.lock().alive
    }

    /// Finished thumbnails kept, and their PNG bytes.
    pub fn kept(&self) -> (usize, usize) {
        let g = self.shared.lock();
        (g.kept.len(), g.kept_bytes)
    }

    pub fn limits(&self) -> Limits {
        self.shared.limits
    }
}

impl Drawer<Pictures> {
    /// `library.thumbnail {file, picture, width?}` for a pane that can't wait: the file is found
    /// as the command finds it (in a Library folder, through the automation read roots), and
    /// its thumbnail answered when it has been drawn, else asked for. An error is what the
    /// command would say.
    pub fn ask(&self, s: &Session, v: &Value) -> Result<Answer, String> {
        let (path, stamp, _) = s.library.file(&Confined, &s.prefs.library_folders, v)?;
        let item = s.library.item_param(v)?;
        let width = match v.get("width") {
            None | Some(Value::Null) => THUMB_WIDTH,
            Some(w) => w
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| THUMB_WIDTHS.contains(n))
                .ok_or_else(|| format!("`width` is 32 to 512 pixels, not {w}"))?,
        };
        let key = Key { path, item, width };
        match self.get(&key, stamp) {
            Some(Ok(t)) => Ok(Answer::Drawn(answer(&t))),
            Some(Err(e)) => Err(e),
            None => {
                self.request(key.clone(), stamp)?;
                Ok(Answer::Drawing(key))
            }
        }
    }
}

/// A thumbnail as `library.thumbnail` answers it: `{png: base64, width, height}`.
pub fn answer(t: &Thumbnail) -> Value {
    json!({"png": craft_library::base64(&t.png), "width": t.width, "height": t.height})
}

/// The worker: draw the newest picture asked for until the drawer is dropped.
fn work<K: Kind>(kind: K, shared: Arc<Shared>) {
    let _alive = Alive(shared.clone());
    loop {
        let job = {
            let mut g = shared.lock();
            loop {
                if g.closed {
                    return;
                }
                if let Some(job) = g.queue.pop_back() {
                    g.busy = Some(job.key.clone());
                    break job;
                }
                g = shared.work.wait(g).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let thumb = draw(&kind, &job, shared.limits.file_bytes);
        {
            let mut g = shared.lock();
            g.busy = None;
            keep(&mut g, &shared.limits, &job, thumb.clone());
            g.done.push_back((job.key, thumb));
            // Finished ones nobody took (a front end that stopped asking): only the newest, within
            // as many as may wait and as many bytes as are kept.
            while g.done.len() > 1 && (g.done.len() > shared.limits.queue || done_bytes(&g.done) > shared.limits.kept_bytes) {
                g.done.pop_front();
            }
        }
        (shared.wake)();
    }
}

fn done_bytes(done: &VecDeque<(Key, Result<Thumbnail, String>)>) -> usize {
    done.iter().map(|(_, t)| t.as_ref().map_or(0, |t| t.png.len())).sum()
}

/// Keep a finished thumbnail, dropping the least recently asked for past the limits (never the
/// one just drawn).
fn keep(g: &mut Inner, limits: &Limits, job: &Job, thumb: Result<Thumbnail, String>) {
    g.tick = g.tick.wrapping_add(1);
    let kept = Kept { stamp: job.stamp, thumb, used: g.tick };
    g.kept_bytes = g.kept_bytes.saturating_add(kept.bytes());
    if let Some(old) = g.kept.insert(job.key.clone(), kept) {
        g.kept_bytes = g.kept_bytes.saturating_sub(old.bytes());
    }
    while g.kept.len() > 1 && (g.kept.len() > limits.kept || g.kept_bytes > limits.kept_bytes) {
        let Some(oldest) = g.kept.iter().filter(|(k, _)| **k != job.key).min_by_key(|(_, t)| t.used).map(|(k, _)| k.clone()) else { break };
        if let Some(t) = g.kept.remove(&oldest) {
            g.kept_bytes = g.kept_bytes.saturating_sub(t.bytes());
        }
    }
}

/// Read and draw one picture under the roots it was asked for under, as the Library would.
fn draw<K: Kind>(kind: &K, job: &Job, max: u64) -> Result<Thumbnail, String> {
    let name = job.key.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let run = || -> Result<Thumbnail, String> {
        if job.stamp.len > max {
            return Err(format!("{name}: larger than {} MB; not drawn", max >> 20));
        }
        let bytes = crate::io::library_read(&job.key.path, max)?;
        let doc = kind.open(&name, &bytes)?;
        drop(bytes);
        let n = kind.count(&doc).min(MAX_ITEMS);
        if job.key.item >= n {
            return Err(format!("{name} has {n} {}; {} isn't one of them (indices count from 0)", kind.nouns().many, job.key.item));
        }
        let width = job.key.width.clamp(*THUMB_WIDTHS.start(), *THUMB_WIDTHS.end());
        let mut t = kind.thumbnail(&doc, job.key.item, width)?;
        t.width = t.width.min(*THUMB_WIDTHS.end());
        t.height = t.height.min(MAX_THUMB_HEIGHT);
        Ok(t)
    };
    // As it was asked for: confined to the roots in force there, or to none.
    let scoped = || match &job.roots {
        Some(r) => file_access::confine(Some(r), run),
        None => file_access::unconfined(run),
    };
    catch_unwind(AssertUnwindSafe(scoped)).unwrap_or_else(|_| Err(format!("{name}: the picture could not be drawn")))
}

#[cfg(test)]
mod tests;
