//! Where WordCraft keeps its settings and the files beside them: the settings store (with the
//! one-time import of the old `ui.json`), the custom dictionary and the user's fonts.
//!
//! Every path comes from [`wordcraft_control_key::settings_dir`], the one settings-folder rule,
//! so `config.toml`, `window.json`, logs, recovery, fonts and templates share a folder. Runs
//! without preferences (`WORDCRAFT_NO_PREFS`, agents' test runs) neither read nor write any of it.

use std::io::Read;
use std::path::{Path, PathBuf};

use craft::settings::{LegacyFile, LoadReport, Location, OpenOptions, SettingsStore, SharedSettings, Source};
use craft::window::restore::Store;
use wordcraft_ui_egui::prefs::{Prefs, import_legacy_window, legacy_ui_json};

/// The most of an old `ui.json` read for its window entry; the file is a few kilobytes.
const LEGACY_LIMIT: u64 = 8 << 20;

/// Whether this run reads and writes preferences. `WORDCRAFT_NO_PREFS` turns them off; a Nightly
/// profile always keeps its own, so an inherited developer switch cannot leave the app with no
/// preferences folder at all.
pub fn enabled() -> bool {
    crate::launch_profile::current().is_nightly() || std::env::var_os("WORDCRAFT_NO_PREFS").is_none()
}

/// `custom-dictionary.txt` in the settings folder: the words Add to Dictionary kept (#40).
pub fn user_dict_path() -> Option<PathBuf> {
    wordcraft_control_key::settings_dir().map(|b| b.join(wordcraft_engine::Session::USER_DICT_FILE))
}

/// Opens the settings store in the settings folder, or in memory when `enabled` is false or there
/// is no folder. On a first run after the move from `ui.json` the old file's settings are
/// imported, and its window entry goes into `placement`, which `LegacyFile::parse` cannot reach.
pub fn open(enabled: bool, placement: &mut Store) -> (SharedSettings<Prefs>, LoadReport) {
    open_in(wordcraft_control_key::settings_dir().filter(|_| enabled), placement)
}

/// [`open`] over an explicit settings folder (`None` keeps everything in memory).
fn open_in(dir: Option<PathBuf>, placement: &mut Store) -> (SharedSettings<Prefs>, LoadReport) {
    let mut options = match &dir {
        Some(dir) => OpenOptions::new(Location::Dir(dir.clone())),
        None => OpenOptions::new(Location::Memory),
    };
    if let Some(dir) = &dir {
        options = options.legacy(vec![LegacyFile { path: dir.join("ui.json"), parse: legacy_ui_json }]);
    }
    let (store, report) = SettingsStore::<Prefs>::open_with(options);
    if let Source::Legacy(path) = &report.source {
        import_window(path, placement);
    }
    (SharedSettings::new(store), report)
}

/// Moves the window place of the legacy file at `path` into `placement` and saves it.
fn import_window(path: &Path, placement: &mut Store) {
    let mut bytes = Vec::new();
    if let Err(e) = std::fs::File::open(path).and_then(|f| f.take(LEGACY_LIMIT).read_to_end(&mut bytes)) {
        log::debug!("preferences: cannot read {} for its window place: {e}", path.display());
        return;
    }
    import_legacy_window(&bytes, placement);
    if let Err(e) = placement.save() {
        log::warn!("window place: {e}");
    }
}

/// Loads the custom dictionary into the spell checker and counts its words as loaded settings:
/// the words only the file has are in `Prefs::dictionary`, so the app's preferences list is the
/// whole dictionary (#3773). `default_path` is `custom-dictionary.txt` in the settings folder.
///
/// A bound custom dictionary file (`editing.custom_dictionary`, #5840) that reads is the source of
/// truth and the write target. If it is missing or unreadable, the default file is read for
/// spelling only and `user_dict_path` stays unset, so later writes never go to the default file
/// while the setting still names the custom one. With no custom file, the default is read and written.
pub fn load_user_dictionary(session: &mut wordcraft_engine::Session, settings: &SharedSettings<Prefs>, default_path: PathBuf) {
    session.default_user_dict_path = Some(default_path.clone());
    let custom = settings.lock().get().editing.custom_dictionary.as_ref().map(PathBuf::from);
    let mut words = Vec::new();
    let mut loaded = false;
    if let Some(p) = &custom {
        if p.is_file() {
            match wordcraft_engine::custom_dict::load_custom_dictionary_replacing(p) {
                Ok(w) => {
                    words = w;
                    session.user_dict_path = Some(p.clone());
                    loaded = true;
                }
                Err(e) => log::warn!("custom dictionary: failed to load '{}': {e}", p.display()),
            }
        } else {
            log::warn!("custom dictionary: '{}' is not a regular file", p.display());
        }
    }
    if !loaded {
        if default_path.is_file() {
            if custom.is_some() {
                match wordcraft_engine::custom_dict::load_custom_dictionary_replacing(&default_path) {
                    Ok(w) => words = w,
                    Err(e) => log::warn!("default dictionary: failed to load '{}': {e}", default_path.display()),
                }
            } else {
                words = wordcraft_proof::load_user_dictionary(&default_path);
                session.user_dict_path = Some(default_path);
            }
        } else if custom.is_none() {
            // A missing default file is not a replacement: keep any words restored from preferences.
            session.user_dict_path = Some(default_path);
        }
    }
    settings.lock().update(|prefs| {
        for word in words {
            if !prefs.dictionary.contains(&word) {
                prefs.dictionary.push(word);
            }
        }
    });
}

/// Fonts the user added (#582) live in `fonts` in the settings folder: load them, and store fonts
/// added later there. Runs without preferences neither read nor keep them.
pub fn load_user_fonts() {
    if !enabled() {
        return;
    }
    let Some(dir) = wordcraft_control_key::settings_dir().map(|d| d.join("fonts")) else { return };
    let (loaded, skipped) = wordcraft_fonts::FontDb::global().load_user_fonts(&dir);
    if !loaded.is_empty() || !skipped.is_empty() {
        log::info!("user fonts: {} loaded, {} skipped from {}", loaded.len(), skipped.len(), dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store over `dir`, as another WordCraft process opens it.
    fn process(dir: &Path) -> SharedSettings<Prefs> {
        open_in(Some(dir.to_path_buf()), &mut Store::in_memory()).0
    }

    /// The settings the next process loads from `dir`.
    fn reloaded(dir: &Path) -> Prefs {
        process(dir).lock().get().clone()
    }

    #[test]
    fn the_loaded_preferences_include_the_words_of_the_dictionary_file() {
        // A word only `custom-dictionary.txt` has is loaded, not added here: it is in the
        // preferences' list like the words `ui.json` held, so a save keeps both.
        let dir = tempfile::tempdir().unwrap();
        let dict = dir.path().join("custom-dictionary.txt");
        std::fs::write(dir.path().join("ui.json"), br#"{"dictionary": ["zqxinjson"]}"#).unwrap();
        std::fs::write(&dict, "zqxinfile\n").unwrap();
        let (settings, report) = open_in(Some(dir.path().to_path_buf()), &mut Store::in_memory());
        assert!(matches!(report.source, Source::Legacy(_)), "ui.json is imported: {:?}", report.source);
        load_user_dictionary(&mut wordcraft_engine::Session::new(wordcraft_doc::Document::new()), &settings, dict.clone());
        let words = settings.lock().get().dictionary.clone();
        for word in ["zqxinjson", "zqxinfile"] {
            assert!(words.iter().any(|w| w == word), "{word}: {words:?}");
        }
        wordcraft_engine::proof::remove_word("zqxinjson");
        wordcraft_engine::proof::remove_word("zqxinfile");
    }

    #[test]
    fn autocorrect_entries_and_exceptions_from_both_processes_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        // Both processes start from the same file.
        let first = process(dir.path());
        first.lock().update(|p| {
            p.editing.autocorrect.entries = vec![("teh".into(), "the".into())];
            p.editing.autocorrect.exceptions = vec!["approx.".into()];
        });
        first.lock().flush().unwrap();
        let (a, b) = (process(dir.path()), process(dir.path()));
        // B adds an entry and an exception and exits first; A adds another entry and removes `approx.`.
        b.lock().update(|p| {
            p.editing.autocorrect.entries.push(("teh2".into(), "the".into()));
            p.editing.autocorrect.exceptions.push("fig.".into());
        });
        b.lock().flush().unwrap();
        a.lock().update(|p| {
            p.editing.autocorrect.entries.push(("adn".into(), "and".into()));
            p.editing.autocorrect.exceptions.retain(|e| e != "approx.");
        });
        a.lock().flush().unwrap();
        let ac = reloaded(dir.path()).editing.autocorrect;
        for from in ["teh", "teh2", "adn"] {
            assert!(ac.entries.iter().any(|(f, _)| f == from), "{from}: {:?}", ac.entries);
        }
        assert_eq!(ac.exceptions, ["fig."], "A's removal stays and B's addition is kept");
    }

    #[test]
    fn a_toolbar_reset_by_one_process_stays_reset() {
        let dir = tempfile::tempdir().unwrap();
        let first = process(dir.path());
        first.lock().update(|p| p.ribbon.qat = Some(vec!["file.save".into(), "format.bold".into()]));
        first.lock().flush().unwrap();
        let (a, b) = (process(dir.path()), process(dir.path()));
        // B resets the toolbar to the default; A, which left it alone, changes something else.
        b.lock().update(|p| p.ribbon.qat = None);
        b.lock().flush().unwrap();
        a.lock().update(|p| p.dictionary.push("zqxtoolbar".into()));
        a.lock().flush().unwrap();
        let now = reloaded(dir.path());
        assert_eq!(now.ribbon.qat, None, "A's old copy of the toolbar does not bring it back");
        assert!(now.dictionary.iter().any(|w| w == "zqxtoolbar"), "A's own change is kept");
    }

    /// The tests that bind a custom dictionary replace the spell checker's words, which every
    /// test shares: one at a time.
    static CUSTOM_DICT: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// #5840: while the setting names a custom file that is missing, the default file is read
    /// for spelling only. Nothing becomes the write target, so Add to Dictionary never moves the
    /// person's words into `custom-dictionary.txt` behind the setting's back.
    #[test]
    fn a_missing_custom_dictionary_leaves_no_write_target() {
        let _one = CUSTOM_DICT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("custom-dictionary.txt");
        std::fs::write(&default, "zqxfallbackword\n").unwrap();
        let (settings, _) = open_in(Some(dir.path().to_path_buf()), &mut Store::in_memory());
        let missing = dir.path().join("gone").join("CUSTOM.DIC");
        settings.lock().update(|p| p.editing.custom_dictionary = Some(missing.to_string_lossy().into_owned()));
        let mut session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        load_user_dictionary(&mut session, &settings, default.clone());
        assert_eq!(session.user_dict_path, None, "no write target while the custom file is missing");
        assert_eq!(session.default_user_dict_path.as_deref(), Some(default.as_path()));
        assert!(wordcraft_engine::proof::user_dictionary().iter().any(|w| w == "zqxfallbackword"), "the default file still spells");
        wordcraft_engine::proof::remove_word("zqxfallbackword");
    }

    /// #5840: a custom file that is set but missing, with no default file either: nothing is a
    /// write target (the `custom.is_none()` arm does not claim the default), and nothing panics.
    #[test]
    fn a_missing_custom_and_default_dictionary_leave_no_write_target() {
        let _one = CUSTOM_DICT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("custom-dictionary.txt");
        let (settings, _) = open_in(Some(dir.path().to_path_buf()), &mut Store::in_memory());
        let missing = dir.path().join("gone").join("CUSTOM.DIC");
        settings.lock().update(|p| p.editing.custom_dictionary = Some(missing.to_string_lossy().into_owned()));
        let mut session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        load_user_dictionary(&mut session, &settings, default.clone());
        assert_eq!(session.user_dict_path, None, "no write target while the custom file is missing");
        assert_eq!(session.default_user_dict_path.as_deref(), Some(default.as_path()));
        assert_eq!(settings.lock().get().editing.custom_dictionary.as_deref(), Some(&*missing.to_string_lossy()), "the setting is kept");
    }

    /// #5840: with no custom file set, the default file is the write target, whether or not it
    /// exists yet; a readable custom file is the write target when it is set.
    #[test]
    fn the_write_target_is_the_default_file_unless_a_custom_one_reads() {
        let _one = CUSTOM_DICT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("custom-dictionary.txt");
        let (settings, _) = open_in(Some(dir.path().to_path_buf()), &mut Store::in_memory());
        let mut session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        load_user_dictionary(&mut session, &settings, default.clone());
        assert_eq!(session.user_dict_path.as_deref(), Some(default.as_path()), "a missing default file is still the target");

        std::fs::write(&default, "zqxdefaultword\n").unwrap();
        let mut session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        load_user_dictionary(&mut session, &settings, default.clone());
        assert_eq!(session.user_dict_path.as_deref(), Some(default.as_path()));

        let custom = dir.path().join("CUSTOM.DIC");
        std::fs::write(&custom, "zqxcustomword\r\n").unwrap();
        settings.lock().update(|p| p.editing.custom_dictionary = Some(custom.to_string_lossy().into_owned()));
        let mut session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        load_user_dictionary(&mut session, &settings, default.clone());
        assert_eq!(session.user_dict_path.as_deref(), Some(custom.as_path()), "a readable custom file is the target");
        let words = wordcraft_engine::proof::user_dictionary();
        assert!(words.iter().any(|w| w == "zqxcustomword") && !words.iter().any(|w| w == "zqxdefaultword"), "{words:?}");
        wordcraft_engine::proof::remove_word("zqxcustomword");
        wordcraft_engine::proof::remove_word("zqxdefaultword");
    }
}
