//! Android's app-private storage, below the `AndroidApp` data directory.
//!
//! Android has no system file picker for this app (rfd has no backend there), so opens, saves,
//! exports and settings default to private storage: no permissions and no Storage Access
//! Framework round-trip, though invisible to the Files app (the in-app browser can additionally
//! reach shared storage once "All files access" is granted; see `apps/wordcraft-android`). Pure
//! path logic, unit-tested on the host; the base directory comes from [`crate::run_inner`],
//! which records it before anything resolves the settings directory.
//!
//! Two folders below the base: `Documents/` (a place for the in-app browser to start and for
//! saves to land) and `config/` (preferences, logs, the user dictionary, user fonts, templates
//! and the control key file — what [`crate::settings_dir`] resolves to on Android).

// Only Android's launch resolves and creates these; the host build compiles the module for the
// tests below.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::path::{Path, PathBuf};

/// Shared internal storage's root on Android: the in-app browser's second root
/// (`Services::shared_storage_root`), entered only while the "All files access" grant holds —
/// `apps/wordcraft-android`'s `android_access.rs` checks and asks for it over JNI.
pub const SHARED_ROOT: &str = "/storage/emulated/0";

/// The storage folders below the app-private base directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The app-private root: what the in-app browser may reach (`Services::app_files_dir`).
    pub base: PathBuf,
    /// Where the in-app browser starts and where saves and exports default to.
    pub documents: PathBuf,
    /// Preferences, logs, the user dictionary, user fonts, templates and the control key file
    /// (the settings directory).
    pub config: PathBuf,
}

/// Split the app-private `base` into its folders.
pub fn layout(base: &Path) -> Layout {
    Layout { base: base.to_path_buf(), documents: base.join("Documents"), config: base.join("config") }
}

/// Create the folders (`Err` names the first failure; the app then keeps that feature off rather
/// than crashing).
pub fn ensure(layout: &Layout) -> Result<(), String> {
    for dir in [&layout.documents, &layout.config] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.to_string_lossy()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("wordcraft-android-storage-{tag}-{}", std::process::id()))
    }

    #[test]
    fn layout_splits_the_base_into_documents_and_config() {
        let base = Path::new("/data/data/ai.storyteller.wordcraft/files");
        let l = layout(base);
        assert_eq!(l.base, base);
        assert_eq!(l.documents, base.join("Documents"));
        assert_eq!(l.config, base.join("config"));
        // the settings folder is `base`'s `config` folder, so the split can be read back if needed
        assert_eq!(l.config.parent(), Some(base));
    }

    #[test]
    fn ensure_creates_every_folder() {
        let base = temp_base("ok");
        let _ = std::fs::remove_dir_all(&base);
        let l = layout(&base);
        ensure(&l).unwrap();
        assert!(l.documents.is_dir() && l.config.is_dir());
        // Re-running over existing folders is fine.
        ensure(&l).unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ensure_reports_a_folder_it_could_not_create() {
        let base = temp_base("blocked");
        let _ = std::fs::remove_dir_all(&base);
        // a file where a folder has to go: create_dir_all must fail, not panic
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("Documents"), b"x").unwrap();
        let err = ensure(&layout(&base)).unwrap_err();
        assert!(err.contains("Documents"), "{err}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
