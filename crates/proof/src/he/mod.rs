//! Hebrew proofing using the installed Hunspell `he_IL` dictionary (never bundled; Debian and
//! Ubuntu ship it as `hunspell-he` / `hspell`, LibreOffice as its `dict-he` extension).
//! Suggestions use the dictionary's own MAP/REP/TRY/KEY tables. Grammar is limited to generic
//! repeated-word and spacing checks with Hebrew messages; hyphenation keeps the fallback.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Hebrew dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a Hebrew Hunspell dictionary (`iw` is the legacy ISO 639-1 code).
const NAMES: &[&str] = &["he_IL", "he-IL", "he", "iw_IL", "iw-IL", "iw"];

/// Inseparable Hebrew prefixes (`אותיות מש"ה וכל"ב`) that may precede a hyphen or maqaf before
/// numbers, acronyms, or foreign words (`ב-2026`, `ה-PDF`, `שלא-כרגיל`).
const PREFIXES: &[&str] =
    &["ה", "ו", "ב", "ל", "כ", "מ", "ש", "וה", "וב", "ול", "וכ", "ומ", "וש", "שה", "שב", "של", "שכ", "שמ", "מה", "כש", "מש", "לכש"];

/// Returns the active Hebrew dictionary: the installed Hunspell `he_IL` (read on first use), one
/// given with [`install_dictionary`], or an empty one when none is available.
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    slot.get_or_insert(loaded).clone()
}

/// Loads the installed Hebrew Hunspell dictionary from the configured or system dictionary paths.
fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("Hebrew spelling: no Hunspell he_IL dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Hebrew spelling: {e}");
        AffixDict::default()
    })
}

/// Installs a Hebrew dictionary from Hunspell `.aff` and `.dic` UTF-8 text. On error the current
/// dictionary stays unchanged.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(d));
    Ok(())
}

/// Clears the cached Hebrew dictionary so the next lookup reloads from disk.
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Returns the `.dic` file path of the installed Hunspell `he_IL` dictionary, or `None` if absent.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Returns `true` when a non-empty Hebrew dictionary is loaded.
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Returns `true` when `c` is a standard Hebrew letter (`א`..=`ת`, U+05D0..=U+05EA).
pub fn is_hebrew_letter(c: char) -> bool {
    ('\u{05D0}'..='\u{05EA}').contains(&c)
}

/// Returns `true` when `c` is a Hebrew niqqud vowel point or cantillation mark.
fn is_hebrew_mark(c: char) -> bool {
    matches!(c, '\u{0591}'..='\u{05BD}' | '\u{05BF}' | '\u{05C1}'..='\u{05C2}' | '\u{05C4}'..='\u{05C5}' | '\u{05C7}')
}

/// Strips Hebrew niqqud and cantillation marks from `w` and normalizes Hebrew geresh/gershayim to
/// ASCII `'` and `"` (as Hspell/Hunspell `he_IL` dictionaries store acronyms and abbreviations).
fn normalize_hebrew(w: &str) -> String {
    let mut out = String::with_capacity(w.len());
    for c in w.chars() {
        if is_hebrew_mark(c) {
            continue;
        }
        match c {
            '’' | '\u{05F3}' => out.push('\''),
            '”' | '“' | '\u{05F4}' => out.push('"'),
            _ => out.push(c),
        }
    }
    out
}

/// Checks whether `word` is spelled correctly in Hebrew using the active dictionary.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// Checks whether `word` is spelled correctly in Hebrew under `opts`.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// Checks whether `word` is spelled correctly in Hebrew against dictionary `d`.
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// Checks a single word against `d` under `opts`, handling niqqud, geresh/gershayim, and maqaf.
fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true;
    }
    let w = word.trim_matches(|c: char| {
        !c.is_alphanumeric() && !is_hebrew_mark(c) && !matches!(c, '\'' | '’' | '\u{05F3}' | '"' | '”' | '“' | '\u{05F4}' | '-' | '\u{05BE}')
    });
    let w = w.trim_start_matches(['\'', '’', '\u{05F3}', '"', '”', '“', '\u{05F4}', '-', '\u{05BE}']);
    let w = w.trim_end_matches(['"', '”', '“', '\u{05F4}', '-', '\u{05BE}']);
    let norm = normalize_hebrew(w);
    let letter_count = norm.chars().filter(|&c| is_hebrew_letter(c) || c.is_alphabetic()).count();
    if letter_count <= 1 || (opts.ignore_numbers && norm.chars().any(|c| c.is_ascii_digit())) || !norm.chars().any(char::is_alphabetic) {
        return true;
    }
    if opts.ignore_uppercase && norm.chars().any(char::is_uppercase) && norm.chars().all(|c| !c.is_lowercase()) && letter_count <= 6 {
        return true;
    }
    if !norm.chars().any(is_hebrew_letter) {
        return true;
    }
    if norm.contains(['-', '\u{05BE}']) {
        return known(d, w, &norm)
            || norm
                .split(['-', '\u{05BE}'])
                .all(|p| p.chars().count() <= 1 || PREFIXES.contains(&p) || !p.chars().any(is_hebrew_letter) || known(d, p, p));
    }
    known(d, w, &norm)
}

/// Checks whether `raw` or its normalized/unpointed form `norm` is in `d` or the user dictionary.
fn known(d: &AffixDict, raw: &str, norm: &str) -> bool {
    d.check(raw)
        || (norm != raw && d.check(norm))
        || crate::in_user_dictionary(&raw.to_lowercase())
        || crate::in_user_dictionary(&norm.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that all supported Hebrew dictionary file names (`he_IL`, `iw_IL`, etc.) load.
    #[test]
    fn dictionary_lookup_accepts_hebrew_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-he-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["he_IL", "he-IL", "he", "iw_IL"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "שלום"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    /// Verifies Hebrew stems, prefixes, suffixes, niqqud stripping, gershayim, and maqaf compounds.
    #[test]
    fn hebrew_words_affixes_niqqud_and_punctuation() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.rule_count() > 0);
        for w in [
            "שלום",
            "השלום",
            "ובשלום",
            "בית",
            "הבית",
            "בבית",
            "ספר",
            "ספרים",
            "הספרים",
            "שָׁלוֹם",
            "צה\"ל",
            "צה״ל",
            "וגו'",
            "וגו׳",
            "בית-ספר",
            "בית־ספר",
            "ב-2026",
            "ה-PDF",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["שלומםם", "בייית", "ספפרים", "מילהאסורה"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    /// Verifies that `is_correct_in_with` flags misspelled Hebrew words under default options.
    #[test]
    fn default_options_do_not_exempt_hebrew_words_as_uppercase() {
        test_dict::install();
        let opts = crate::Options::default();
        assert!(opts.ignore_uppercase);
        assert!(crate::is_correct_in_with("שלום", crate::ProofLang::He, &opts));
        assert!(!crate::is_correct_in_with("שלומםם", crate::ProofLang::He, &opts));
        assert!(crate::is_correct_in_with("PDF", crate::ProofLang::He, &opts));
    }
}
