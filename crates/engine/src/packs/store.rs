//! The font store as the packs [`Backend`](super::Backend): its folder, its consent setting, the
//! pinned pack index and verified installs (craft-libs `craft-font-store`, kind `language-pack`).

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use craft_font_catalog::FontEnv;
use craft_font_store::{FontStore, Network, PackEntry, PackIndex, PackIndexPin};

use super::{Cancel, Pack};

/// The pack index this build trusts: craft-i18n's first Language pack release.
///
/// TODO(CP4): language-packs-1 isn't published yet; set its `index.json` sha256 here when it is.
/// Until then fetching the pinned index fails its check, and `CRAFT_LANGUAGE_PACK_INDEX` (read by
/// the font store itself) points WordCraft at a local or test index instead.
pub const PACK_INDEX: PackIndexPin = PackIndexPin {
    url: "https://github.com/storytold/craft-i18n/releases/download/language-packs-1/index.json",
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
};

pub(super) struct FontStoreBackend {
    store: FontStore,
    /// The index last read or fetched; installs take their entry (url, size, sha256) from it.
    index: Mutex<Option<PackIndex>>,
}

impl FontStoreBackend {
    pub(super) fn new(user: bool) -> Option<Self> {
        let env = FontEnv::from_process();
        let store = if user { FontStore::for_user(&env)? } else { FontStore::for_agent(&env)? };
        Some(Self { store, index: Mutex::new(None) })
    }

    fn keep(&self, index: PackIndex) -> Vec<Pack> {
        let packs = index.packs.iter().map(pack).collect();
        *self.index.lock().unwrap_or_else(PoisonError::into_inner) = Some(index);
        packs
    }
}

fn pack(e: &PackEntry) -> Pack {
    Pack {
        locale: e.locale.clone(),
        name: e.name.clone(),
        version: e.version.clone(),
        url: e.url.clone(),
        // TODO(CP4): the index's optional `source` once craft-font-store's PackEntry carries it.
        source: None,
        size: e.size,
        licence: e.licence.clone(),
        class: e.class.clone(),
        spelling: e.spelling,
        hyphenation: e.hyphenation,
    }
}

fn cancel_of(c: &Cancel) -> craft_font_store::Cancel {
    let out = craft_font_store::Cancel::new();
    if c.is_cancelled() {
        out.cancel();
    }
    out
}

impl super::Backend for FontStoreBackend {
    fn index(&self) -> Result<Vec<Pack>, String> {
        Ok(self.store.cached_language_pack_index(&PACK_INDEX).map(|i| self.keep(i)).unwrap_or_default())
    }

    fn fetch_index(&self, cancel: &Cancel) -> Result<Vec<Pack>, String> {
        let index = self.store.fetch_language_pack_index(&PACK_INDEX, &cancel_of(cancel)).map_err(|e| e.to_string())?;
        Ok(self.keep(index))
    }

    fn network_on(&self) -> bool {
        self.store.network() == Network::On
    }

    fn enable_network(&self) -> Result<(), String> {
        self.store.enable_network().map_err(|e| e.to_string())
    }

    fn install(&self, pack: &Pack, cancel: &Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
        let index = {
            let held = self.index.lock().unwrap_or_else(PoisonError::into_inner).clone();
            match held {
                Some(i) => i,
                None => self.store.cached_language_pack_index(&PACK_INDEX).ok_or("the list of Language packs hasn't been fetched")?,
            }
        };
        let entry = index.pack(&pack.locale).ok_or_else(|| format!("there is no language pack for {}", pack.locale))?.clone();
        // The store's cancel is its own flag: a cancel during the download is passed on between
        // progress reports.
        let store_cancel = cancel_of(cancel);
        let watch = store_cancel.clone();
        self.store
            .install_language_pack(&index, &entry, &store_cancel, &mut |p| {
                if cancel.is_cancelled() {
                    watch.cancel();
                }
                progress(p.done, p.total);
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn remove(&self, locale: &str) -> Result<(), String> {
        self.store.remove_language_pack(locale).map_err(|e| e.to_string())
    }

    fn index_override(&self) -> Option<String> {
        craft_font_store::language_pack_index_overridden()
    }

    fn packs_dir(&self) -> Option<PathBuf> {
        Some(self.store.language_packs_dir())
    }
}
