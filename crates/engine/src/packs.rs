//! Language packs (CP4.3): spelling dictionaries and hyphenation patterns for languages beyond
//! the embedded English, one pack per locale, downloaded and verified by the font store
//! (craft-libs `craft-font-store`, item kind `language-pack`) and read by `craft-proof` from the
//! store's `language-packs/` folder.
//!
//! The commands (`proof.packs`, `proof.installPack`, `proof.removePack`), Options › Language,
//! the missing-pack bar and About › Language data all go through this module. It never asks
//! anything: an install needs the font store's single consent setting (downloading on), which only
//! a person in the app can turn on ([`Backend::enable_network`], called from an Install click).
//! Agents (MCP, the CLI, the control channel) install only while it is on.
//!
//! The [`Backend`] is set once at start-up by the desktop app and the CLI ([`use_font_store`]).
//! Without one (tests, the web build, `ui_shot`) nothing is available and nothing is downloaded.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use serde::Serialize;

use crate::{CmdError, Session};

#[cfg(not(target_arch = "wasm32"))]
mod store;

/// Longest locale tag taken, in bytes (BCP 47 tags in practice stay under 20).
pub const MAX_LOCALE_LEN: usize = 35;
/// Most packs an index may list that are read.
const MAX_PACKS: usize = 1000;
/// Paragraphs of a document looked at for its languages (the missing-pack bar).
const MAX_PARAGRAPHS_SCANNED: usize = 20_000;
/// Distinct languages of a document collected.
const MAX_DOC_LANGUAGES: usize = 64;

/// A pack the pinned index lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Pack {
    /// Lowercase BCP 47 (`nl`, `pt-pt`).
    pub locale: String,
    /// The language's own name (`Nederlands`).
    pub name: String,
    /// The craft-i18n commit the pack was built from.
    pub version: String,
    /// The archive.
    pub url: String,
    /// The upstream folder the data comes from, at its pinned commit (the index's optional
    /// `source`), shown before install; the archive when the index gives none.
    pub source: Option<String>,
    /// Bytes of the archive.
    pub size: u64,
    /// SPDX expression.
    pub licence: String,
    /// `Embeddable` or `Download-only`.
    pub class: String,
    /// Has a spelling dictionary.
    pub spelling: bool,
    /// Has hyphenation patterns.
    pub hyphenation: bool,
}

/// An installed pack.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Installed {
    pub locale: String,
    pub name: String,
    pub licence: String,
    pub path: PathBuf,
}

/// Stops a download. On the desktop it is the font store's own flag, which a download watches
/// even while the server sends nothing.
#[derive(Clone, Debug, Default)]
pub struct Cancel(#[cfg(not(target_arch = "wasm32"))] pub(crate) craft_font_store::Cancel, #[cfg(target_arch = "wasm32")] Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn cancel(&self) {
        self.0.cancel();
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
    #[cfg(target_arch = "wasm32")]
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    #[cfg(target_arch = "wasm32")]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Where packs come from: the font store on the desktop, a fixed list in tests and `ui_shot`.
pub trait Backend: Send + Sync {
    /// The pinned index as last fetched (no request); empty when it never was.
    fn index(&self) -> Result<Vec<Pack>, String>;
    /// Fetch the pinned index (a request; only while downloading is on).
    fn fetch_index(&self, cancel: &Cancel) -> Result<Vec<Pack>, String>;
    /// Whether the font store may download.
    fn network_on(&self) -> bool;
    /// Turn downloading on. Only from the person's own action in the app (an Install click):
    /// the commands never call it, and a backend for an agent refuses.
    fn enable_network(&self) -> Result<(), String>;
    /// Download, verify and unpack `pack` into the packs folder. `progress` gets (done, total) bytes.
    fn install(&self, pack: &Pack, cancel: &Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String>;
    /// Remove the installed pack for `locale`.
    fn remove(&self, locale: &str) -> Result<(), String>;
    /// The installed-packs folder (`<font store>/language-packs`).
    fn packs_dir(&self) -> Option<PathBuf>;
    /// The packs installed now.
    fn installed(&self) -> Vec<Installed> {
        installed_from_proof()
    }
    /// The test index in use instead of the pinned one (`CRAFT_LANGUAGE_PACK_INDEX`), if any.
    fn index_override(&self) -> Option<String> {
        None
    }
}

static BACKEND: RwLock<Option<Arc<dyn Backend>>> = RwLock::new(None);
/// The index last read, so the bar and the list don't read the store's cache every frame.
static INDEX: Mutex<Option<Vec<Pack>>> = Mutex::new(None);
/// Bumped whenever [`INDEX`] changes.
static INDEX_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn set_index(index: Option<Vec<Pack>>) {
    *INDEX.lock().unwrap_or_else(PoisonError::into_inner) = index;
    INDEX_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Changes whenever the list of available packs does (a fetch, another backend).
pub fn index_generation() -> u64 {
    INDEX_GENERATION.load(Ordering::Relaxed)
}

/// Use `backend` for packs from now on (`None`: no packs), and point proofing at its folder.
pub fn set_backend(backend: Option<Arc<dyn Backend>>) {
    let dir = backend.as_ref().and_then(|b| b.packs_dir());
    *BACKEND.write().unwrap_or_else(PoisonError::into_inner) = backend;
    set_index(None);
    jobs().errors.clear();
    wordcraft_proof::packs::set_packs_dir(dir);
}

/// Use the suite's font store for packs (`<font store>/language-packs`), as the person in the app
/// (`user`: an Install click may turn downloading on) or as an agent (the CLI, headless MCP: it
/// never can). The pack index is the pinned one, or `CRAFT_LANGUAGE_PACK_INDEX` (a URL or a path,
/// for testing packs before they are published), which the font store reads itself. Returns
/// whether there is a store folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn use_font_store(user: bool) -> bool {
    match store::FontStoreBackend::new(user) {
        Some(b) => {
            set_backend(Some(Arc::new(b)));
            true
        }
        None => false,
    }
}

/// [`use_font_store`] with the store in `dir` instead of the person's (`ui_shot`, tests).
#[cfg(not(target_arch = "wasm32"))]
pub fn use_font_store_in(dir: PathBuf, user: bool) {
    set_backend(Some(Arc::new(store::FontStoreBackend::in_dir(dir, user))));
}

/// The backend in use.
pub fn backend() -> Option<Arc<dyn Backend>> {
    BACKEND.read().unwrap_or_else(PoisonError::into_inner).clone()
}

fn installed_from_proof() -> Vec<Installed> {
    wordcraft_proof::packs::installed().into_iter().map(|p| Installed { locale: p.locale, name: p.name, licence: p.licence, path: p.path }).collect()
}

/// The test source packs come from instead of the pinned index (`CRAFT_LANGUAGE_PACK_INDEX`):
/// Options › Language says so.
pub fn index_override() -> Option<String> {
    backend().and_then(|b| b.index_override())
}

/// The packs installed now (none without a backend).
pub fn installed() -> Vec<Installed> {
    backend().map(|b| b.installed()).unwrap_or_default()
}

/// The available packs: the index as last read, read from the store's cache the first time.
pub fn available() -> Vec<Pack> {
    let Some(b) = backend() else { return Vec::new() };
    let mut cached = INDEX.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(i) = cached.as_ref() {
        return i.clone();
    }
    let index = clean_index(b.index().unwrap_or_default());
    *cached = Some(index.clone());
    drop(cached);
    INDEX_GENERATION.fetch_add(1, Ordering::Relaxed);
    index
}

/// How many packs [`available`] lists, without copying them (the window asks every frame).
pub fn available_len() -> usize {
    if let Some(i) = INDEX.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        return i.len();
    }
    available().len()
}

/// Fetch the index again (downloading must be on).
pub fn refresh(cancel: &Cancel) -> Result<Vec<Pack>, String> {
    let b = backend().ok_or_else(no_store)?;
    if !b.network_on() {
        return Err(network_off());
    }
    let index = clean_index(b.fetch_index(cancel)?);
    set_index(Some(index.clone()));
    Ok(index)
}

/// Drop packs with a malformed locale, duplicates and English (embedded), and cap the list.
fn clean_index(mut packs: Vec<Pack>) -> Vec<Pack> {
    packs.truncate(MAX_PACKS);
    let mut seen = BTreeSet::new();
    packs.retain(|p| valid_locale(&p.locale) && !is_embedded(&p.locale) && seen.insert(p.locale.clone()));
    packs.sort_by(|a, b| a.locale.cmp(&b.locale));
    packs
}

/// English is embedded in every build; it never comes as a pack.
fn is_embedded(locale: &str) -> bool {
    locale == "en" || locale.starts_with("en-")
}

/// A lowercase BCP 47-shaped tag: letters, digits and `-`, starting with a letter.
pub fn valid_locale(locale: &str) -> bool {
    !locale.is_empty()
        && locale.len() <= MAX_LOCALE_LEN
        && locale.starts_with(|c: char| c.is_ascii_lowercase())
        && locale.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !locale.contains("--")
        && !locale.ends_with('-')
}

/// A locale from a command's params: trimmed, lowercased, `_` as `-`.
pub fn parse_locale(raw: &str) -> Result<String, CmdError> {
    let l = raw.trim().replace('_', "-").to_ascii_lowercase();
    if valid_locale(&l) { Ok(l) } else { Err(CmdError::Params(format!("`locale` must be a language tag like nl or pt-PT, not {raw:?}"))) }
}

fn no_store() -> String {
    if cfg!(target_arch = "wasm32") {
        "Language packs aren't available in the browser".into()
    } else {
        "there is no font store folder for language packs".into()
    }
}

fn network_off() -> String {
    "downloading is turned off: turn it on in WordCraft (File › Options › Language, or an Install button there)".into()
}

/// The pack for `locale` in the index.
pub fn find(locale: &str) -> Option<Pack> {
    available().into_iter().find(|p| p.locale == locale)
}

/// Whether a pack is installed for `locale`.
pub fn is_installed(locale: &str) -> bool {
    installed().iter().any(|p| p.locale == locale)
}

/// Install the pack for `locale` (downloading must be on): the commands and the Install button.
/// Proofing picks it up at once.
pub fn install(locale: &str, cancel: &Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<Installed, String> {
    let b = backend().ok_or_else(no_store)?;
    let pack = find(locale).ok_or_else(|| format!("there is no language pack for {locale}"))?;
    if !b.network_on() {
        return Err(network_off());
    }
    if let Some(dir) = b.packs_dir() {
        crate::file_access::check_write(&dir.to_string_lossy())?;
    }
    b.install(&pack, cancel, progress)?;
    reload(&b);
    b.installed().into_iter().find(|p| p.locale == locale).ok_or_else(|| format!("the {locale} pack didn't install"))
}

/// Remove the installed pack for `locale`.
pub fn remove(locale: &str) -> Result<(), String> {
    let b = backend().ok_or_else(no_store)?;
    if let Some(job) = jobs().installs.iter().find(|j| j.locale == locale) {
        return Err(if job.stopping() {
            format!("the {locale} pack's cancelled install is still stopping: try again in a moment")
        } else {
            format!("the {locale} pack is being installed: cancel it first")
        });
    }
    if !b.installed().iter().any(|p| p.locale == locale) {
        return Err(format!("no language pack is installed for {locale}"));
    }
    if let Some(dir) = b.packs_dir() {
        crate::file_access::check_write(&dir.to_string_lossy())?;
    }
    b.remove(locale)?;
    reload(&b);
    Ok(())
}

/// What the app does with a long pack request (an install or a fetch of the list): `None` runs it
/// in the caller (the CLI, headless MCP, tests); `Some(notify)` runs it on a thread, so the window
/// keeps drawing whoever asked (the person, the control channel, MCP), and calls `notify` on each
/// change (the window repaints). Set once by the desktop app.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

static BACKGROUND: RwLock<Option<Notify>> = RwLock::new(None);

/// An install running on a thread.
struct InstallJob {
    locale: String,
    /// Bytes done.
    done: u64,
    /// Bytes in all.
    total: u64,
    /// Stops this run. A cancelled job stays listed (hidden from [`installing`]) until its thread
    /// notices and ends, so two downloads of a locale never write its folder at once.
    cancel: Cancel,
    /// Install was asked again after a cancel, before the cancelled run ended: the thread starts
    /// over (with the fresh `cancel`) instead of ending.
    restart: bool,
}

impl InstallJob {
    /// Cancelled, and its thread hasn't ended yet.
    fn stopping(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Installs and fetches running on a thread, and how the finished ones ended.
#[derive(Default)]
struct Jobs {
    /// Running installs, one per locale.
    installs: Vec<InstallJob>,
    fetching: bool,
    /// The last failure of each locale's install ("" for the list), until it is tried again.
    errors: Vec<(String, String)>,
    /// Bumped as each job ends: the window relays out and shows the result.
    finished: u64,
}

static JOBS: Mutex<Jobs> = Mutex::new(Jobs { installs: Vec::new(), fetching: false, errors: Vec::new(), finished: 0 });

fn jobs() -> std::sync::MutexGuard<'static, Jobs> {
    JOBS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Run long pack requests on a thread (`Some`: the desktop app), or in the caller (`None`).
pub fn set_background(notify: Option<Notify>) {
    *BACKGROUND.write().unwrap_or_else(PoisonError::into_inner) = notify;
}

fn background() -> Option<Notify> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    BACKGROUND.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Whether installs and fetches run on a thread ([`set_background`]).
pub fn runs_in_background() -> bool {
    background().is_some()
}

/// A running install: (locale, bytes done, bytes in all).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub locale: String,
    pub done: u64,
    pub total: u64,
}

/// The installs running now (not the cancelled ones still stopping).
pub fn installing() -> Vec<Progress> {
    jobs().installs.iter().filter(|j| !j.stopping()).map(|j| Progress { locale: j.locale.clone(), done: j.done, total: j.total }).collect()
}

/// Whether the list is being fetched.
pub fn fetching() -> bool {
    jobs().fetching
}

/// The last failure of `locale`'s install ("" for the list), until it is tried again.
pub fn error(locale: &str) -> Option<String> {
    jobs().errors.iter().find(|(l, _)| l == locale).map(|(_, e)| e.clone())
}

/// Every recorded failure, (locale or "" for the list, message).
pub fn errors() -> Vec<(String, String)> {
    jobs().errors.clone()
}

/// Counts finished jobs: when it changes, the window lays the documents out again.
pub fn finished() -> u64 {
    jobs().finished
}

/// Stop `locale`'s running install. The person's own choice: it ends without an error, and
/// [`installing`] stops listing it at once (Install shows again).
pub fn cancel(locale: &str) {
    if let Some(job) = jobs().installs.iter_mut().find(|j| j.locale == locale) {
        job.restart = false;
        job.cancel.cancel();
    }
}

fn set_error(locale: &str, e: Option<String>) {
    let mut j = jobs();
    j.errors.retain(|(l, _)| l != locale);
    if let Some(e) = e {
        j.errors.push((locale.to_string(), e));
    }
}

/// Start installing `locale` ([`install`]): on a thread when [`runs_in_background`], else now.
/// What's wrong up front (no pack, downloading off, outside the write root) is an error at once.
/// Returns whether it is still running (`false`: done). Asked again while it runs, it carries on;
/// right after a cancel, before the cancelled download has stopped, it starts over once that
/// one ends (never two downloads of a pack at once).
pub fn start_install(locale: &str) -> Result<bool, String> {
    let b = backend().ok_or_else(no_store)?;
    let pack = find(locale).ok_or_else(|| format!("there is no language pack for {locale}"))?;
    if !b.network_on() {
        return Err(network_off());
    }
    if let Some(dir) = b.packs_dir() {
        crate::file_access::check_write(&dir.to_string_lossy())?;
    }
    let Some(notify) = background() else {
        let r = install(locale, &Cancel::new(), &mut |_, _| {});
        set_error(locale, r.as_ref().err().cloned());
        return r.map(|_| false);
    };
    let cancel = Cancel::new();
    {
        let mut j = jobs();
        j.errors.retain(|(l, _)| l != locale);
        if let Some(job) = j.installs.iter_mut().find(|j| j.locale == locale) {
            if job.stopping() {
                // Cancelled and not ended yet: its thread starts over when it does.
                job.done = 0;
                job.total = pack.size;
                job.cancel = cancel;
                job.restart = true;
            }
            return Ok(true);
        }
        j.installs.push(InstallJob { locale: locale.to_string(), done: 0, total: pack.size, cancel: cancel.clone(), restart: false });
    }
    let key = locale.to_string();
    let locale = locale.to_string();
    let started = spawn(move || {
        let mut cancel = cancel;
        loop {
            let progress_notify = notify.clone();
            let r = install(&locale, &cancel, &mut |done, total| {
                if let Some(job) = jobs().installs.iter_mut().find(|j| j.locale == locale) {
                    job.done = done;
                    job.total = total;
                }
                progress_notify();
            });
            let mut j = jobs();
            if let Some(job) = j.installs.iter_mut().find(|j| j.locale == locale && j.restart) {
                job.restart = false;
                cancel = job.cancel.clone();
                drop(j);
                notify();
                continue;
            }
            j.installs.retain(|j| j.locale != locale);
            // The person's own cancel isn't a failure: nothing to show.
            if let Err(e) = r
                && !cancel.is_cancelled()
            {
                j.errors.push((locale, e));
            }
            j.finished = j.finished.wrapping_add(1);
            drop(j);
            notify();
            break;
        }
    });
    if let Err(e) = started {
        jobs().installs.retain(|j| j.locale != key);
        return Err(e);
    }
    Ok(true)
}

/// Start fetching the list again ([`refresh`]): on a thread when [`runs_in_background`], else now.
/// Returns whether it is still running.
pub fn start_refresh() -> Result<bool, String> {
    let b = backend().ok_or_else(no_store)?;
    if !b.network_on() {
        return Err(network_off());
    }
    let Some(notify) = background() else {
        let r = refresh(&Cancel::new());
        set_error("", r.as_ref().err().cloned());
        return r.map(|_| false);
    };
    {
        let mut j = jobs();
        if j.fetching {
            return Ok(true);
        }
        j.fetching = true;
        j.errors.retain(|(l, _)| !l.is_empty());
    }
    let started = spawn(move || {
        let r = refresh(&Cancel::new());
        let mut j = jobs();
        j.fetching = false;
        if let Err(e) = r {
            j.errors.push((String::new(), e));
        }
        j.finished = j.finished.wrapping_add(1);
        drop(j);
        notify();
    });
    if let Err(e) = started {
        jobs().fetching = false;
        return Err(e);
    }
    Ok(true)
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(f: impl FnOnce() + Send + 'static) -> Result<(), String> {
    std::thread::Builder::new().name("language-pack".into()).spawn(f).map(|_| ()).map_err(|e| format!("couldn't start the download: {e}"))
}

#[cfg(target_arch = "wasm32")]
fn spawn(_: impl FnOnce() + Send + 'static) -> Result<(), String> {
    Err(no_store())
}

/// Have proofing look at the packs folder again (its dictionaries are reloaded).
fn reload(b: &Arc<dyn Backend>) {
    if wordcraft_proof::packs::packs_dir() == b.packs_dir() {
        wordcraft_proof::packs::reload();
    } else {
        wordcraft_proof::packs::set_packs_dir(b.packs_dir());
    }
}

/// The full licence texts for About › Language data: the embedded English data, then each
/// installed pack.
pub fn licence_texts() -> Vec<(String, String)> {
    let mut texts = wordcraft_proof::packs::licence_texts();
    // SCOWL's notices (with the Ispell BSD licence of its affix files), when English spells from
    // the embedded SCOWL data; after the Moby and supplement entries.
    if let Some(notice) = wordcraft_proof::scowl::notice() {
        let at = texts.len().min(2);
        texts.insert(at, (SCOWL_TITLE.into(), notice.into()));
    }
    texts
}

/// About › Language data's title for SCOWL's notices.
pub const SCOWL_TITLE: &str = "English spelling dictionaries (SCOWL)";

/// The pack locale for a document language tag: the whole tag (`pt-pt`), else its language
/// (`nl` for `nl-NL`), whichever the index has. `None` for English and tags with no pack.
pub fn pack_for_tag<'a>(tag: &str, packs: &'a [Pack]) -> Option<&'a Pack> {
    let tag = tag.trim().replace('_', "-").to_ascii_lowercase();
    let base = tag.split('-').next().unwrap_or("");
    if base.is_empty() || base == "en" {
        return None;
    }
    packs.iter().find(|p| p.locale == tag).or_else(|| packs.iter().find(|p| p.locale == base))
}

/// The languages (`w:lang`, resolved through the styles) of the body text, in first-seen order.
pub fn document_languages(s: &Session) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in s.doc.para_paths(wordcraft_doc::StoryRef::Body).into_iter().take(MAX_PARAGRAPHS_SCANNED) {
        let Some(p) = s.doc.para(wordcraft_doc::StoryRef::Body, &path) else { continue };
        if p.text.trim().is_empty() {
            continue;
        }
        let style = p.props.style.as_deref();
        for (range, c) in p.run_ranges() {
            let Some(text) = p.text.get(range) else { continue };
            if !text.chars().any(char::is_alphabetic) {
                continue;
            }
            let rc = s.doc.styles.resolve_char_in(style, None, c);
            if rc.no_proof {
                continue;
            }
            let tag = rc.lang.unwrap_or_else(|| "en-US".into());
            if !out.contains(&tag) {
                out.push(tag);
                if out.len() >= MAX_DOC_LANGUAGES {
                    return out;
                }
            }
        }
    }
    out
}

/// Whether proofing reads a pack for text in `tag`: every language craft-proof can name, its own
/// (Polish…) and any other (`ProofLang::Pack`, read from the pack alone); not `und`, `zxx` or a
/// malformed tag. The one place the bar's language rule lives.
pub fn proofable(tag: &str) -> bool {
    wordcraft_proof::ProofLang::from_tag(Some(tag)) != wordcraft_proof::ProofLang::Other
}

/// The Hunspell dictionary `tag`'s language needs and doesn't have, for the status bar's
/// "(no dictionary)": craft-proof's [`wordcraft_proof::missing_dictionary`], except for a
/// language craft-proof has no rules of its own for (`ProofLang::Pack`: Japanese, Chinese,
/// Thai…) when no pack for it is available either, as no Hunspell dictionary may exist at all.
pub fn missing_dictionary(tag: Option<&str>) -> Option<&'static str> {
    let name = wordcraft_proof::missing_dictionary(tag)?;
    let lang = wordcraft_proof::ProofLang::from_tag(tag);
    if lang.pack_locale().is_some() && !has_pack_for_tag(tag.unwrap_or("")) {
        return None;
    }
    Some(name)
}

/// The last answer of whether the index has a pack for a tag, for the index generation it was
/// worked out for: the status bar asks every frame while the caret is in a pack language, and
/// working it out copies the whole index.
#[derive(Default)]
struct PackLookup {
    /// (index generation, tag, has a pack).
    entry: Option<(u64, String, bool)>,
}

impl PackLookup {
    /// The remembered answer for `tag` at `generation`, else `compute`'s (then remembered).
    fn get(&mut self, generation: u64, tag: &str, compute: impl FnOnce() -> bool) -> bool {
        if let Some((g, t, has)) = &self.entry
            && *g == generation
            && t == tag
        {
            return *has;
        }
        let has = compute();
        self.entry = Some((generation, tag.to_string(), has));
        has
    }
}

static PACK_LOOKUP: Mutex<PackLookup> = Mutex::new(PackLookup { entry: None });

/// Whether the index has a pack for `tag` ([`pack_for_tag`]), worked out again only when the
/// index changes ([`index_generation`]) or the tag does.
fn has_pack_for_tag(tag: &str) -> bool {
    // Read before [`available`], which may fill the index and bump it: the answer is then worked
    // out once more, never kept for a list it wasn't worked out from.
    let generation = index_generation();
    PACK_LOOKUP.lock().unwrap_or_else(PoisonError::into_inner).get(generation, tag, || pack_for_tag(tag, &available()).is_some())
}

/// The packs the document's text asks for that are available and not installed: what the
/// missing-pack bar offers. Never a language without a pack, nor one proofing can't use.
pub fn missing(s: &Session) -> Vec<Pack> {
    let packs = available();
    if packs.is_empty() {
        return Vec::new();
    }
    let installed = installed();
    let mut out: Vec<Pack> = Vec::new();
    for tag in document_languages(s) {
        if proofable(&tag)
            && let Some(p) = pack_for_tag(&tag, &packs)
            && !installed.iter().any(|i| i.locale == p.locale)
            && !out.iter().any(|o| o.locale == p.locale)
        {
            out.push(p.clone());
        }
    }
    out
}

/// A backend with a fixed list and installs that only record themselves: tests and `ui_shot`.
#[derive(Default)]
pub struct FixedBackend {
    pub packs: Vec<Pack>,
    pub network: AtomicBool,
    /// Agents can't turn the network on.
    pub agent: bool,
    pub installed: Mutex<Vec<Installed>>,
}

impl Pack {
    /// Where the data comes from: the upstream source when the index names it, else the archive.
    pub fn source_link(&self) -> &str {
        self.source.as_deref().filter(|s| !s.is_empty()).unwrap_or(&self.url)
    }
}

impl FixedBackend {
    /// A few packs shaped like the published ones, for screenshots and tests.
    pub fn sample() -> Self {
        let pack = |locale: &str, name: &str, size: u64, licence: &str, class: &str| Pack {
            locale: locale.into(),
            name: name.into(),
            version: "0000000".into(),
            url: format!("https://github.com/storytold/craft-i18n/releases/download/language-packs-1/{locale}.tar.zst"),
            source: Some("https://github.com/LibreOffice/dictionaries".into()),
            size,
            licence: licence.into(),
            class: class.into(),
            spelling: true,
            hyphenation: true,
        };
        FixedBackend {
            packs: vec![
                pack("ca", "Català", 1_480_000, "GPL-2.0-or-later OR LGPL-2.1-or-later", "Download-only"),
                pack("el", "Ελληνικά", 2_310_000, "MPL-1.1 OR GPL-2.0-or-later OR LGPL-2.1-or-later", "Embeddable"),
                pack("fr", "Français", 1_120_000, "MPL-2.0", "Embeddable"),
                pack("nl", "Nederlands", 1_950_000, "BSD-3-Clause", "Embeddable"),
                pack("pl", "Polski", 4_620_000, "GPL-2.0-or-later OR LGPL-2.1-or-later OR MPL-1.1 OR Apache-2.0 OR CC-BY-4.0", "Embeddable"),
                pack("sk", "Slovenčina", 2_040_000, "GPL-2.0-only OR LGPL-2.1-only OR MPL-1.1", "Download-only"),
            ],
            ..Default::default()
        }
    }
}

impl Backend for FixedBackend {
    fn index(&self) -> Result<Vec<Pack>, String> {
        Ok(self.packs.clone())
    }
    fn fetch_index(&self, _: &Cancel) -> Result<Vec<Pack>, String> {
        Ok(self.packs.clone())
    }
    fn network_on(&self) -> bool {
        self.network.load(Ordering::Relaxed)
    }
    fn enable_network(&self) -> Result<(), String> {
        if self.agent {
            return Err("only the user can turn on downloading".into());
        }
        self.network.store(true, Ordering::Relaxed);
        Ok(())
    }
    fn install(&self, pack: &Pack, cancel: &Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        progress(pack.size, pack.size);
        let mut i = self.installed.lock().unwrap_or_else(PoisonError::into_inner);
        i.retain(|p| p.locale != pack.locale);
        i.push(Installed { locale: pack.locale.clone(), name: pack.name.clone(), licence: pack.licence.clone(), path: PathBuf::new() });
        Ok(())
    }
    fn remove(&self, locale: &str) -> Result<(), String> {
        self.installed.lock().unwrap_or_else(PoisonError::into_inner).retain(|p| p.locale != locale);
        Ok(())
    }
    fn packs_dir(&self) -> Option<PathBuf> {
        None
    }
    fn installed(&self) -> Vec<Installed> {
        self.installed.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

#[cfg(test)]
mod tests {
    /// CP4.4 (`cargo xtask language`): a build embeds English Language data and nothing it
    /// wasn't asked to. A Language build names the locales it embeds in `CRAFT_PROOF_EMBED`
    /// (`en` alone: English only); the default embeds the carry-over locales too, until Language
    /// packs are published and pinned (craft-proof's `DEFAULT_EMBED`; TODO(CP4.4): English only
    /// then).
    /// About › Language data shows SCOWL's notices (the Ispell BSD licence among them) whenever
    /// English spells from the embedded SCOWL data.
    #[test]
    fn about_shows_the_scowl_notice_when_scowl_is_embedded() {
        let texts = super::licence_texts();
        match wordcraft_proof::scowl::notice() {
            Some(notice) => {
                let entry = texts.iter().find(|(t, _)| t == super::SCOWL_TITLE).map(|(_, x)| x.as_str());
                assert_eq!(entry, Some(notice));
                assert!(notice.contains("Ispell") || notice.contains("ispell"), "the Ispell notice is part of it");
            }
            None => assert!(texts.iter().all(|(t, _)| t != super::SCOWL_TITLE), "no SCOWL data: no SCOWL entry"),
        }
    }

    /// FU.3: the status bar's "(no dictionary)" asks every frame whether the index has a pack
    /// for the caret's language; the answer is reused until the index changes (a new
    /// generation) or the tag does, so the index isn't copied every frame.
    #[test]
    fn the_pack_lookup_is_reused_until_the_index_changes() {
        let mut lookup = super::PackLookup::default();
        let mut computed = 0;
        let mut ask = |lookup: &mut super::PackLookup, generation: u64, tag: &str, answer: bool| {
            lookup.get(generation, tag, || {
                computed += 1;
                answer
            })
        };
        assert!(ask(&mut lookup, 1, "ja-JP", true));
        // Same generation and tag: the remembered answer, even where the list would now say no.
        for _ in 0..10 {
            assert!(ask(&mut lookup, 1, "ja-JP", false));
        }
        assert!(!ask(&mut lookup, 2, "ja-JP", false), "a new generation works it out again");
        assert!(ask(&mut lookup, 2, "th-TH", true), "another tag too");
        assert_eq!(computed, 3);
    }

    #[test]
    fn the_default_build_embeds_only_english() {
        const CARRY_OVER: [&str; 5] = ["de", "pl", "pt", "it", "ru"];
        let named: Vec<String> = std::env::var("CRAFT_PROOF_EMBED")
            .unwrap_or_default()
            .split(',')
            .map(|l| l.trim().to_ascii_lowercase())
            .filter(|l| !l.is_empty())
            .collect();
        let allowed = |base: &str| base == "en" || if named.is_empty() { CARRY_OVER.contains(&base) } else { named.iter().any(|n| n == base) };
        let embedded = wordcraft_proof::packs::embedded_locales();
        let extra: Vec<&&str> = embedded.iter().filter(|l| !allowed(l.split('-').next().unwrap_or(l))).collect();
        assert!(extra.is_empty(), "embedded beyond English and {named:?} (or the carry-over by default): {extra:?}");
    }
}
