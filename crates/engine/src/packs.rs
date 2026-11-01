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

/// Stops a download between chunks.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
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
}

static BACKEND: RwLock<Option<Arc<dyn Backend>>> = RwLock::new(None);
/// The index last read, so the bar and the list don't read the store's cache every frame.
static INDEX: Mutex<Option<Vec<Pack>>> = Mutex::new(None);

/// Use `backend` for packs from now on (`None`: no packs), and point proofing at its folder.
pub fn set_backend(backend: Option<Arc<dyn Backend>>) {
    let dir = backend.as_ref().and_then(|b| b.packs_dir());
    *BACKEND.write().unwrap_or_else(PoisonError::into_inner) = backend;
    *INDEX.lock().unwrap_or_else(PoisonError::into_inner) = None;
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

/// The backend in use.
pub fn backend() -> Option<Arc<dyn Backend>> {
    BACKEND.read().unwrap_or_else(PoisonError::into_inner).clone()
}

fn installed_from_proof() -> Vec<Installed> {
    wordcraft_proof::packs::installed()
        .into_iter()
        .map(|p| Installed { locale: p.locale, name: p.name, licence: p.licence, path: p.path })
        .collect()
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
    index
}

/// Fetch the index again (downloading must be on).
pub fn refresh(cancel: &Cancel) -> Result<Vec<Pack>, String> {
    let b = backend().ok_or_else(no_store)?;
    if !b.network_on() {
        return Err(network_off());
    }
    let index = clean_index(b.fetch_index(cancel)?);
    *INDEX.lock().unwrap_or_else(PoisonError::into_inner) = Some(index.clone());
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
    wordcraft_proof::packs::licence_texts()
}

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

impl FixedBackend {
    /// A few packs shaped like the published ones, for screenshots and tests.
    pub fn sample() -> Self {
        let pack = |locale: &str, name: &str, size: u64, licence: &str, class: &str| Pack {
            locale: locale.into(),
            name: name.into(),
            version: "0000000".into(),
            url: format!("https://github.com/storytold/craft-i18n/releases/download/language-packs-1/{locale}.tar.zst"),
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
    /// CP4.4 (`cargo xtask language`): a default build embeds English Language data only; every
    /// other language comes as a Language pack. A Language build names the locales it embeds in
    /// `CRAFT_PROOF_EMBED`.
    #[test]
    fn the_default_build_embeds_only_english() {
        if std::env::var_os("CRAFT_PROOF_EMBED").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let embedded = wordcraft_proof::packs::embedded_locales();
        assert!(embedded.iter().all(|l| l.starts_with("en-")), "only en-* is embedded by default, got {embedded:?}");
    }
}
