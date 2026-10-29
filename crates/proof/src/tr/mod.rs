//! Turkish proofing.
//!
//! - **Spelling** against the Hunspell `tr_TR` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `tr_TR.aff`/`tr_TR.dic` from the OS, a
//!   package manager or LibreOffice; it is not bundled). Without it Turkish text gets no spelling
//!   marks. Capitalized stems are proper nouns (`Ankara`); foreign names inflected with an apostrophe
//!   are accepted (`WordCraft'ta`), and the user dictionary applies as in English.
//! - **Suggestions** tuned for Turkish ([`suggest`]): ASCII-diacritic restoration (`turkce` -> `Türkçe`),
//!   common confusions (`yalnış` -> `yanlış`), detached particles (`geldimi` -> `geldi mi`), and typos.
//! - **Grammar** ([`grammar`]): high-precision rules (reduplications, comma around conjunctions,
//!   sentence start with abbreviations).
//! - **Hyphenation** ([`hyphen`]): syllable division by Turkish Language Association (TDK) rules.

pub mod dict;
pub mod grammar;
pub mod hyphen;
pub mod suggest;

use std::sync::{Arc, RwLock};

pub use dict::TrDict;
pub use suggest::suggest;

/// The Turkish dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<TrDict>>> = RwLock::new(None);

/// File names of a Turkish Hunspell dictionary.
const NAMES: &[&str] = &["tr_TR", "tr-TR", "tr"];

/// The Turkish dictionary: the installed Hunspell `tr_TR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available.
pub fn dictionary() -> Arc<TrDict> {
    if let Some(d) = DICT.read().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    slot.get_or_insert(loaded).clone()
}

fn load_installed() -> TrDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("Turkish spelling: no Hunspell tr_TR dictionary installed");
        return TrDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Turkish spelling: {e}");
        TrDict::default()
    })
}

/// Read a Turkish Hunspell pair from disk: size-capped and decoded per the `.aff`'s `SET` line,
/// like the other languages' dictionaries (#3661).
pub(crate) fn load(aff: &std::path::Path, dic: &std::path::Path) -> Result<TrDict, String> {
    let (aff_text, dic_text) = crate::hunspell::decode(&crate::hunspell::read_capped(aff)?, &crate::hunspell::read_capped(dic)?)?;
    TrDict::from_hunspell(&aff_text, &dic_text)
}

/// Use this Turkish dictionary from now on.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = TrDict::from_hunspell(aff, dic)?;
    *DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(d));
    Ok(())
}

/// Reload the dictionary on next check.
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// The `.dic` file of the Hunspell `tr_TR` dictionary found in the dictionaries folder or the
/// system locations now (File › Options › Proofing shows it, #1063); `None` when there is none.
/// A dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a Turkish dictionary available?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `word` spelled correctly in Turkish?
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] against a given dictionary.
pub fn is_correct_with(d: &TrDict, word: &str) -> bool {
    if d.is_empty() {
        return true; // No Turkish dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    // Check user dictionary first
    if crate::in_user_dictionary(&suggest::turkish_to_lower(w)) || crate::in_user_dictionary(&w.to_lowercase()) {
        return true;
    }

    // Direct lookup or lowercase lookup
    if d.contains(w) {
        return true;
    }
    let lower = suggest::turkish_to_lower(w);
    if d.contains(&lower) {
        return true;
    }

    // Handle apostrophe for proper nouns (e.g., `Ankara'da`, `WordCraft'ta`)
    if let Some((stem, _suffix)) = w.split_once(['\'', '’'])
        && (d.contains(stem) || d.contains(&suggest::turkish_to_lower(stem)))
    {
        return true;
    }

    // Compound words separated by hyphens (e.g., `afyon-karahisar`)
    if w.contains('-') && w.split('-').all(|part| part.is_empty() || is_correct_with(d, part)) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_dictionary_everything_is_correct() {
        let empty = TrDict::default();
        assert!(is_correct_with(&empty, "herhangi"));
        assert!(is_correct_with(&empty, "xyzabc"));
    }

    #[test]
    fn apostrophe_and_compounds_are_handled() {
        let empty = TrDict::default();
        assert!(is_correct_with(&empty, "Ankara'da"));
        assert!(is_correct_with(&empty, "açık-mavi"));
    }

    #[test]
    fn installed_dictionary_checks_words_if_present() {
        // Machines without a Turkish Hunspell dictionary (CI, most desktops) have nothing to check.
        if !has_dictionary() {
            return;
        }
        assert!(is_correct("kitap"));
        assert!(is_correct("kitaplar"));
        assert!(is_correct("evde"));
        assert!(is_correct("olacaklardan"));
        assert!(is_correct("sonra"));
        assert!(!is_correct("sorna"));
        assert!(!is_correct("kitapppp"));
        assert!(!is_correct("evdekişşş"));

        // Test user dictionary
        assert!(!is_correct("benimbilmedigimkelime"));
        crate::add_word("benimbilmedigimkelime");
        assert!(is_correct("benimbilmedigimkelime"));
    }
}
