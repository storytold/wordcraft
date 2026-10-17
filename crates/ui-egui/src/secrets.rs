//! Secrets kept in the system's credential store rather than in `ui.json` (#764): today the
//! LanguageTool API key. macOS keeps it in the Keychain, Windows in the Credential Manager, Linux
//! and the BSDs in the Secret Service (GNOME Keyring, KWallet, KeePassXC…). `ui.json` only says
//! that the store holds it (`languageToolKeyInStore`).
//!
//! Where there is no store (the browser, a desktop without a Secret Service) or it refuses the
//! key, the key stays in `ui.json` as before and File › Options says so. A store error is never
//! fatal: the key is kept where it was.

/// A credential store: a few named secrets for WordCraft. Behind a trait so tests use
/// [`MemoryStore`] and never touch the user's keychain.
pub trait SecretStore {
    /// The secret saved under `name`; `Ok(None)` when there is none.
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    /// Save `secret` under `name`, replacing any earlier one.
    fn set(&self, name: &str, secret: &str) -> Result<(), String>;
    /// Forget the secret saved under `name`; forgetting one that isn't there is not an error.
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// The service the secrets are filed under in the store.
pub const SERVICE: &str = "WordCraft";
/// The LanguageTool API key's name in the store.
pub const LANGUAGE_TOOL_KEY: &str = "LanguageTool API key";
/// Longest secret read back from the store, as for one read from `ui.json` (a longer one is
/// ignored).
pub const MAX_SECRET_LEN: usize = 512;

/// The system's credential store, or `None` where it can't be set up.
#[cfg(not(target_arch = "wasm32"))]
pub fn os_store() -> Option<Box<dyn SecretStore>> {
    match keyring::Entry::store_status() {
        Ok(()) => Some(Box::new(OsStore)),
        Err(e) => {
            log::info!("credential store unavailable: {}", describe(e));
            None
        }
    }
}

/// The system's credential store, through `keyring`.
#[cfg(not(target_arch = "wasm32"))]
struct OsStore;

#[cfg(not(target_arch = "wasm32"))]
impl SecretStore for OsStore {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(SERVICE, name).and_then(|e| e.get_password()) {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(describe(&e)),
        }
    }
    fn set(&self, name: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, name).and_then(|e| e.set_password(secret)).map_err(|e| describe(&e))
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, name).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(describe(&e)),
        }
    }
}

/// A store error for the log and File › Options, never the secret: a badly encoded secret's
/// error carries its bytes.
#[cfg(not(target_arch = "wasm32"))]
fn describe(e: &keyring::Error) -> String {
    match e {
        keyring::Error::BadEncoding(_) => "the saved secret is not text".into(),
        e => e.to_string(),
    }
}

/// An in-memory store for tests; clones share their secrets. `fail` makes every call fail, as a
/// locked or missing store does.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct MemoryStore {
    pub secrets: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, String>>>,
    pub fail: std::rc::Rc<std::cell::Cell<bool>>,
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        if self.fail.get() {
            return Err("store locked".into());
        }
        Ok(self.secrets.borrow().get(name).cloned())
    }
    fn set(&self, name: &str, secret: &str) -> Result<(), String> {
        if self.fail.get() {
            return Err("store locked".into());
        }
        self.secrets.borrow_mut().insert(name.into(), secret.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        if self.fail.get() {
            return Err("store locked".into());
        }
        self.secrets.borrow_mut().remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Services, UiState, WordApp};
    use serde_json::json;
    use wordcraft_doc::Document;
    use wordcraft_engine::Session;

    fn app(store: Option<&MemoryStore>) -> WordApp {
        let secrets = store.map(|s| Box::new(s.clone()) as Box<dyn SecretStore>);
        WordApp::new(Session::new(Document::new()), Services { secrets, ..Services::default() })
    }

    fn load(app: &mut WordApp, json: &str) {
        app.apply_prefs(UiState::from_json(json.as_bytes()).unwrap());
    }

    fn saved(app: &WordApp) -> String {
        serde_json::to_string_pretty(&app.prefs()).unwrap()
    }

    const OLD: &str = r#"{"languageTool": {"enabled": true, "username": "me@example.org", "apiKey": "SECRET-764"}}"#;

    #[test]
    fn a_key_in_ui_json_moves_to_the_store_and_never_comes_back_to_ui_json() {
        let store = MemoryStore::default();
        let mut a = app(Some(&store));
        load(&mut a, OLD);
        assert!(a.sync_secrets(), "the key moved, so ui.json is to be rewritten");
        assert_eq!(store.secrets.borrow().get(LANGUAGE_TOOL_KEY).map(String::as_str), Some("SECRET-764"));
        assert_eq!(a.ui.language_tool.api_key, "SECRET-764", "checking still has the key");
        let json = saved(&a);
        assert!(!json.contains("SECRET-764"), "{json}");
        assert!(json.contains(r#""languageToolKeyInStore": true"#), "{json}");
        assert!(!a.sync_secrets(), "nothing changed since");

        // The next run reads it back from the store.
        let mut b = app(Some(&store));
        load(&mut b, &json);
        assert_eq!(b.ui.language_tool.api_key, "SECRET-764");
        assert!(b.lt_key.error.is_none());
        assert!(!saved(&b).contains("SECRET-764"));

        // A new key replaces it in the store, never in ui.json.
        b.run("ui.languageTool", json!({"apiKey": "NEW-KEY-764"})).unwrap();
        assert!(b.sync_secrets());
        assert_eq!(store.secrets.borrow().get(LANGUAGE_TOOL_KEY).map(String::as_str), Some("NEW-KEY-764"));
        assert!(!saved(&b).contains("NEW-KEY-764"));

        // Clearing the key removes it from the store, and ui.json no longer points there.
        b.ui.language_tool.api_key.clear();
        assert!(b.sync_secrets());
        assert!(store.secrets.borrow().get(LANGUAGE_TOOL_KEY).is_none());
        assert!(saved(&b).contains(r#""languageToolKeyInStore": false"#));
    }

    #[test]
    fn a_store_that_fails_keeps_the_key_in_ui_json_and_says_why() {
        let store = MemoryStore::default();
        store.fail.set(true);
        let mut a = app(Some(&store));
        load(&mut a, OLD);
        assert!(!a.sync_secrets());
        assert!(store.secrets.borrow().is_empty());
        let json = saved(&a);
        assert!(json.contains("SECRET-764"), "the key stays where it was: {json}");
        assert!(json.contains(r#""languageToolKeyInStore": false"#), "{json}");
        assert!(matches!(&a.lt_key.error, Some(crate::LtKeyError::Store(e)) if e.contains("store locked")));
        // Once the store works again, the next save moves the key there.
        store.fail.set(false);
        assert!(a.sync_secrets());
        assert!(!saved(&a).contains("SECRET-764"));
        assert!(a.lt_key.error.is_none());
    }

    #[test]
    fn a_store_that_cannot_be_read_keeps_pointing_at_the_key() {
        let store = MemoryStore::default();
        store.secrets.borrow_mut().insert(LANGUAGE_TOOL_KEY.into(), "SECRET-764".into());
        store.fail.set(true);
        let mut a = app(Some(&store));
        load(&mut a, r#"{"languageToolKeyInStore": true}"#);
        assert_eq!(a.ui.language_tool.api_key, "");
        assert!(a.lt_key.error.is_some());
        assert!(!a.sync_secrets(), "the key wasn't deleted");
        assert!(saved(&a).contains(r#""languageToolKeyInStore": true"#));
        // The next run, with the store unlocked, has the key again.
        store.fail.set(false);
        let mut b = app(Some(&store));
        load(&mut b, &saved(&a));
        assert_eq!(b.ui.language_tool.api_key, "SECRET-764");
    }

    #[test]
    fn hostile_store_contents_are_ignored() {
        let store = MemoryStore::default();
        store.secrets.borrow_mut().insert(LANGUAGE_TOOL_KEY.into(), "é".repeat(MAX_SECRET_LEN));
        let mut a = app(Some(&store));
        load(&mut a, r#"{"languageToolKeyInStore": true}"#);
        assert_eq!(a.ui.language_tool.api_key, "", "longer than a key from ui.json may be");
    }

    #[test]
    fn a_key_the_store_lost_is_reported() {
        const KEPT: &str = "The API key is kept in the system's credential store.";
        // Removed outside WordCraft, or too long to be a key.
        for entry in [None, Some("x".repeat(MAX_SECRET_LEN + 1))] {
            let store = MemoryStore::default();
            if let Some(e) = entry {
                store.secrets.borrow_mut().insert(LANGUAGE_TOOL_KEY.into(), e);
            }
            let mut a = app(Some(&store));
            load(&mut a, r#"{"languageToolKeyInStore": true}"#);
            assert_eq!(a.ui.language_tool.api_key, "");
            assert!(a.lt_key.error.is_some(), "the key didn't vanish silently");
            assert!(!a.sync_secrets());
            let note = crate::options::api_key_note(&a);
            assert_ne!(note, KEPT, "an empty field isn't a key kept in the store");
            assert!(note.contains("no longer has the API key"), "{note}");
            // Entering the key again puts it back.
            a.run("ui.languageTool", json!({"apiKey": "AGAIN-764"})).unwrap();
            assert!(a.sync_secrets());
            assert!(a.lt_key.error.is_none());
            assert_eq!(crate::options::api_key_note(&a), KEPT);
        }
    }

    #[test]
    fn a_key_the_store_cannot_forget_is_reported() {
        let store = MemoryStore::default();
        store.secrets.borrow_mut().insert(LANGUAGE_TOOL_KEY.into(), "SECRET-764".into());
        let mut a = app(Some(&store));
        load(&mut a, r#"{"languageToolKeyInStore": true}"#);
        store.fail.set(true);
        a.ui.language_tool.api_key.clear();
        a.sync_secrets();
        assert!(matches!(&a.lt_key.error, Some(crate::LtKeyError::NotRemoved(e)) if e.contains("store locked")));
        let note = crate::options::api_key_note(&a);
        assert!(note.contains("old API key is still in the system's credential store"), "{note}");
        assert!(!saved(&a).contains("SECRET-764"));
    }

    #[test]
    fn without_a_store_the_key_stays_in_ui_json_as_before() {
        let mut a = app(None);
        load(&mut a, OLD);
        assert!(!a.sync_secrets());
        assert!(saved(&a).contains("SECRET-764"));
        // A flag from another machine's ui.json reads no key and isn't kept.
        let mut b = app(None);
        load(&mut b, r#"{"languageToolKeyInStore": true}"#);
        assert!(saved(&b).contains(r#""languageToolKeyInStore": false"#));
    }

    #[test]
    fn dumps_never_show_the_key() {
        let store = MemoryStore::default();
        let mut a = app(Some(&store));
        load(&mut a, OLD);
        a.sync_secrets();
        assert!(!format!("{:?}", a.prefs()).contains("SECRET-764"));
        assert!(!format!("{:?}", a.ui).contains("SECRET-764"));
        let inspected = crate::control::inspect(&mut a, &egui::Context::default());
        assert!(!inspected.to_string().contains("SECRET-764"));
        assert_eq!(inspected["ui"]["languageTool"]["apiKey"], "<set>", "agents still see that a key is set");
    }
}
