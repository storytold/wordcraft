//! Preferences between visits (#2132): the same [`UiState`] the desktop app keeps in `ui.json`
//! (theme, interface size, recent symbols, editing options…), kept in the browser's
//! `localStorage` under [`KEY`]. Restored at startup; written back whenever it changes, so a
//! reload or a closed tab keeps the last setting (browsers don't reliably run exit code).

use wordcraft_ui_egui::{UiState, WordApp};

/// The `localStorage` key.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const KEY: &str = "wordcraft.ui";
/// Largest stored value read back (`localStorage` holds about 5 MB per site).
const MAX_STORED: usize = 4 << 20;

/// Apply preferences stored on an earlier visit. Missing, oversized or unreadable values leave
/// the defaults.
pub fn restore(app: &mut WordApp, stored: Option<&str>) {
    if let Some(s) = stored.filter(|s| s.len() <= MAX_STORED)
        && let Some(ui) = UiState::from_json(s.as_bytes())
    {
        app.apply_prefs(ui);
    }
}

/// Tracks what was last written, so the store is written only when a preference changed.
#[derive(Default)]
pub struct PrefsSync {
    last: Option<String>,
    /// A host page's name for the user (`?author=`) and the user's own, which is stored in its
    /// place while the user keeps the host's.
    host_author: Option<(String, String)>,
}

impl PrefsSync {
    /// Starts from what is already stored, so restoring doesn't write it straight back.
    pub fn new(stored: Option<String>) -> Self {
        Self { last: stored, host_author: None }
    }

    /// The session signs with `host`, a host page's name for the user, but `own` stays the
    /// stored name. A name the user then chooses in File › Options is stored as usual.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn keep_own_author(&mut self, host: &str, own: &str) {
        if host != own {
            self.host_author = Some((host.to_string(), own.to_string()));
        }
    }

    /// The JSON to store when the app's preferences differ from what was last written.
    pub fn changed(&mut self, app: &WordApp) -> Option<String> {
        let mut prefs = app.prefs();
        if let Some((host, own)) = &self.host_author
            && prefs.author == *host
        {
            prefs.author = own.clone();
        }
        let json = match serde_json::to_string(&prefs) {
            Ok(j) => j,
            Err(e) => {
                log::warn!("preferences: cannot serialize: {e}");
                return None;
            }
        };
        if self.last.as_deref() == Some(json.as_str()) {
            return None;
        }
        self.last = Some(json.clone());
        Some(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wordcraft_ui_egui::Services;

    fn app() -> WordApp {
        WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Services::default())
    }

    /// A host page's name for the user signs the visit but isn't stored; a name the user then
    /// chooses is.
    #[test]
    fn host_author_is_not_stored() {
        let mut a = app();
        a.session.author = "Grace Hopper".into();
        let mut sync = PrefsSync::new(None);
        let stored = sync.changed(&a).unwrap();
        a.session.author = "rob".into();
        sync.keep_own_author("rob", "Grace Hopper");
        a.run("ui.theme", json!({"value": "dark"})).unwrap();
        let json = sync.changed(&a).expect("the theme changed");
        assert_ne!(json, stored);
        let mut next = app();
        restore(&mut next, Some(&json));
        assert_eq!(next.session.author, "Grace Hopper");
        a.session.author = "Rob Hones".into();
        let json = sync.changed(&a).expect("the user chose a name");
        let mut next = app();
        restore(&mut next, Some(&json));
        assert_eq!(next.session.author, "Rob Hones");
    }

    /// #2132: dark mode chosen on one visit is there after a reload.
    #[test]
    fn dark_mode_survives_reload() {
        let mut first = app();
        let mut sync = PrefsSync::new(None);
        let initial = sync.changed(&first);
        assert!(initial.is_some(), "first frame stores the preferences");
        assert_eq!(sync.changed(&first), None, "unchanged preferences are not rewritten");
        first.run("ui.theme", json!({"value": "dark"})).unwrap();
        let stored = sync.changed(&first).expect("the theme change is stored");

        let mut reloaded = app();
        restore(&mut reloaded, Some(&stored));
        assert_eq!(reloaded.ui.theme, wordcraft_ui_egui::theme::Appearance::Dark);
        assert_eq!(PrefsSync::new(Some(stored)).changed(&reloaded), None, "restoring doesn't write back");
    }

    #[test]
    fn junk_storage_keeps_defaults() {
        for junk in [None, Some(""), Some("not json"), Some("[1,2]"), Some("{\"theme\":42}")] {
            let mut a = app();
            restore(&mut a, junk);
            assert_eq!(a.ui.theme, wordcraft_ui_egui::theme::Appearance::System, "junk keeps the default appearance");
        }
    }
}
