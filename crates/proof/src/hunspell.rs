//! Hunspell dictionaries installed on the system, read at runtime through the shared
//! `craft-proof` crate.
//!
//! WordCraft doesn't bundle large spelling dictionaries. For languages with an affix dictionary
//! (Polish, Portuguese, Russian), it reads the standard Hunspell `.aff`/`.dic` pair installed by the
//! operating system, a package manager or LibreOffice. When none is installed the language has no
//! spelling check (no marks, no errors), like Word without the language's proofing tools.
//!
//! Search order: the directory set with [`set_dictionary_dir`] (a setting), then
//! `WORDCRAFT_DICTIONARIES`, a `WordCraft` dictionaries folder under the user's application data
//! (`%LOCALAPPDATA%`, `~/Library/Application Support` or `$XDG_DATA_HOME`), and the usual system
//! locations. On the web there are none.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub use craft_proof::hunspell::{decode, find_in, load};

/// An extra directory searched first (a setting), set by the front end.
static EXTRA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Why an installed dictionary failed to load, by `.dic` name (`pt_BR`): the file and the reason.
/// Without it a file that is there but unreadable looks the same as no file (#2830).
static LOAD_ERRORS: RwLock<Vec<(String, PathBuf, String)>> = RwLock::new(Vec::new());

/// Remember (or, for `Ok`, forget) why the dictionary `key` found at `dic` failed to load, and
/// hand the result back unchanged.
pub fn note_load<T>(key: &str, dic: &Path, result: Result<T, String>) -> Result<T, String> {
    let mut errors = LOAD_ERRORS.write().unwrap_or_else(|e| e.into_inner());
    errors.retain(|(k, _, _)| k != key);
    if let Err(e) = &result {
        errors.push((key.to_string(), dic.to_path_buf(), e.clone()));
    }
    result
}

/// Forget the load error of the dictionary `key` (none found, or it is about to be searched for again).
pub fn clear_load_error(key: &str) {
    LOAD_ERRORS.write().unwrap_or_else(|e| e.into_inner()).retain(|(k, _, _)| k != key);
}

/// The `.dic` file and the reason of the last failed load of the dictionary `key`, if any.
pub fn load_error(key: &str) -> Option<(PathBuf, String)> {
    LOAD_ERRORS.read().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _, _)| k == key).map(|(_, p, e)| (p.clone(), e.clone()))
}

/// Search `dir` before the system locations (a "dictionaries folder" setting); `None` clears it.
/// Takes effect for dictionaries not loaded yet.
pub fn set_dictionary_dir(dir: Option<PathBuf>) {
    *EXTRA_DIR.write().unwrap_or_else(|e| e.into_inner()) = dir;
    LOAD_ERRORS.write().unwrap_or_else(|e| e.into_inner()).clear(); // they belong to the old search path
}

/// WordCraft's search: its own folder setting, `WORDCRAFT_DICTIONARIES` and `WordCraft` folders.
fn search() -> craft_proof::hunspell::Search {
    let extra_dir = EXTRA_DIR.read().unwrap_or_else(|e| e.into_inner()).clone();
    craft_proof::hunspell::Search { extra_dir, ..craft_proof::hunspell::Search::for_app("WordCraft") }
}

/// Directories that may hold Hunspell dictionaries, most specific first. `lo_exts`, when a
/// language is being searched for, adds LibreOffice's extension folders for it (`dict-pt-BR`).
pub fn search_dirs(lo_exts: Option<&[String]>) -> Vec<PathBuf> {
    let mut dirs = search().dirs(lo_exts);
    // LibreOffice installed on another drive (e.g. `D:\Program Files\LibreOffice`): check C..Z.
    if cfg!(windows)
        && let Some(exts) = lo_exts
    {
        for drive in b'C'..=b'Z' {
            for pf in ["Program Files", "Program Files (x86)"] {
                let base = PathBuf::from(format!(r"{}:\{pf}\LibreOffice\share\extensions", drive as char));
                if base.is_dir() {
                    for d in exts.iter().map(|e| base.join(e)) {
                        if !dirs.contains(&d) {
                            dirs.push(d);
                        }
                    }
                }
            }
        }
    }
    dirs
}

/// The first `<name>.aff`/`<name>.dic` pair found for any of `names` (`pl_PL`, `pl`).
pub fn find(names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    search().find(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn russian_dictionaries_in_older_encodings_decode() {
        // `ёлка` in KOI8-R and in Windows-1251 (#894), decoded by craft-proof.
        assert_eq!(decode(b"SET KOI8-R\n", b"1\n\xa3\xcc\xcb\xc1\n").unwrap().1, "1\nёлка\n");
        assert_eq!(decode(b"SET microsoft-cp1251\n", b"1\n\xb8\xeb\xea\xe0\n").unwrap().1, "1\nёлка\n");
    }

    /// #2830: a dictionary that is there but can't be read leaves its reason behind; a good
    /// load or no file clears it.
    #[test]
    fn load_error_is_remembered_until_the_next_good_load() {
        let key = "test_unreadable_2830";
        assert_eq!(load_error(key), None);
        let dic = Path::new("/dicts/test.dic");
        assert!(note_load(key, dic, decode(b"SET SHIFT_JIS\n", b"")).is_err());
        let (file, reason) = load_error(key).expect("error kept");
        assert_eq!(file, dic);
        assert_eq!(reason, "unsupported encoding SHIFT_JIS");
        assert!(note_load(key, dic, decode(b"SET UTF-8\n", b"")).is_ok());
        assert_eq!(load_error(key), None);
        let _ = note_load::<()>(key, dic, Err("too large".into()));
        clear_load_error(key);
        assert_eq!(load_error(key), None);
    }

    #[test]
    fn decodes_latin1() {
        // `orçamento esperávamos` in ISO 8859-1: ç = 0xE7, á = 0xE1.
        let (a, d) = decode(b"SET ISO8859-1\nTRY \xe7\n", b"1\nor\xe7amento/A\nesper\xe1vamos\n").unwrap();
        assert!(a.contains("TRY ç"), "{a}");
        assert_eq!(d, "1\norçamento/A\nesperávamos\n");
        assert!(decode(b"SET ISO-8859-1\n", b"\xe7").is_ok());
    }

    #[test]
    fn wordcraft_names_its_folder_and_variable() {
        let s = search();
        assert_eq!(s.env_var.as_deref(), Some("WORDCRAFT_DICTIONARIES"));
        assert_eq!(s.app_dir.as_deref(), Some("WordCraft"));
    }

    #[test]
    fn the_folder_setting_is_searched_first() {
        let dir = std::env::temp_dir().join(format!("wordcraft-hunspell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("xx_XX.aff"), "SET UTF-8\nSFX A Y 1\nSFX A 0 y .\n").unwrap();
        std::fs::write(dir.join("xx_XX.dic"), "1\nkot/A\n").unwrap();
        set_dictionary_dir(Some(dir.clone()));
        assert_eq!(search_dirs(None).first(), Some(&dir));
        let (aff, dic) = find(&["xx_XX"]).unwrap();
        assert!(load(&aff, &dic).unwrap().check("koty"));
        assert!(find_in(Path::new(&dir), &["yy"]).is_none());
        set_dictionary_dir(None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
