//! Preferences between visits (#2132): the same [`Prefs`] the desktop app keeps in its settings
//! file (theme, interface size, recent symbols, editing options…), kept by the settings store in the
//! browser's `localStorage`. A value stored by an earlier build under `wordcraft.ui` is imported once.

use craft::settings::SharedSettings;
use wordcraft_ui_egui::prefs::Prefs;

/// Opens the settings store over the browser's `localStorage`. Changes are written at once on the
/// next tick: a reload or closed tab never runs exit code.
#[cfg(target_arch = "wasm32")]
pub fn open() -> (SharedSettings<Prefs>, craft::settings::LoadReport) {
    use std::sync::Arc;
    use std::time::Duration;

    use craft::settings::{LegacyFile, LocalStorageFiles, Location, OpenOptions, SettingsStore};
    use wordcraft_ui_egui::prefs::legacy_ui_json;

    /// Largest stored value read back (`localStorage` holds about 5 MB per site).
    const MAX_STORED: usize = 4 << 20;
    let options = OpenOptions::new(Location::Dir("wordcraft".into()))
        .files(Arc::new(LocalStorageFiles::new(MAX_STORED)))
        .debounce(Duration::ZERO)
        .legacy(vec![LegacyFile { path: "wordcraft.ui".into(), parse: legacy_ui_json }]);
    let (store, report) = SettingsStore::open_with(options);
    (SharedSettings::new(store), report)
}

/// A host page's name for the user (`?author=`) and the user's own. The session signs with the
/// host's name, but the store keeps the user's own, which a visit outside the host would restore.
pub struct HostAuthor {
    host: String,
    own: String,
}

impl HostAuthor {
    /// The rule for a session that signs with `host` while `own` stays stored; `None` when they are
    /// the same name.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn new(host: &str, own: &str) -> Option<Self> {
        (host != own).then(|| Self { host: host.to_string(), own: own.to_string() })
    }

    /// The name to keep in the store in place of `stored`: the user's own when `stored` is the
    /// host's, `None` when `stored` stays as it is. A name the user chose in File › Options differs
    /// from the host's and is stored as usual.
    pub fn stored_author(&self, stored: Option<&str>) -> Option<&str> {
        (stored == Some(self.host.as_str())).then_some(self.own.as_str())
    }

    /// Puts the user's own name back into the store where the app wrote the host's.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn keep_own_author(&self, settings: &SharedSettings<Prefs>) {
        let mut store = settings.lock();
        if let Some(own) = self.stored_author(store.get().author.as_deref()).map(str::to_string) {
            store.update(|prefs| prefs.author = Some(own));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use craft::settings::testing::MemoryFiles;
    use craft::settings::{Location, OpenOptions, SettingsStore};

    use super::*;

    fn store() -> SharedSettings<Prefs> {
        let options: OpenOptions<Prefs> =
            OpenOptions::new(Location::Dir("wordcraft".into())).files(Arc::new(MemoryFiles::new())).debounce(Duration::ZERO);
        let (store, _report) = SettingsStore::<Prefs>::open_with(options);
        SharedSettings::new(store)
    }

    /// A host page's name for the user signs the visit but isn't stored; a name the user then
    /// chooses is.
    #[test]
    fn host_author_is_not_stored() {
        let settings = store();
        settings.lock().update(|p| p.author = Some("Grace Hopper".into()));
        let rule = HostAuthor::new("rob", "Grace Hopper").unwrap();
        // The app writes the session's author, the host's name, into the store.
        settings.lock().update(|p| p.author = Some("rob".into()));
        rule.keep_own_author(&settings);
        assert_eq!(settings.lock().get().author.as_deref(), Some("Grace Hopper"));
        // The user chooses a name of their own.
        settings.lock().update(|p| p.author = Some("Rob Hones".into()));
        rule.keep_own_author(&settings);
        assert_eq!(settings.lock().get().author.as_deref(), Some("Rob Hones"));
        assert!(HostAuthor::new("same", "same").is_none());
    }
}
