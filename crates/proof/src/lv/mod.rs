//! Latvian proofing using the installed Hunspell `lv_LV` dictionary (never bundled; Debian and
//! Ubuntu ship it as `hunspell-lv`, LibreOffice as its Latvian spelling extension). Suggestions
//! use the dictionary's own MAP/REP/TRY/KEY tables. Grammar is limited to generic repeated-word
//! and spacing checks with Latvian messages; hyphenation keeps the English fallback (no Latvian
//! patterns are bundled).

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Latvian dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a Latvian Hunspell dictionary.
const NAMES: &[&str] = &["lv_LV", "lv-LV", "lv"];

/// The Latvian dictionary: the installed Hunspell `lv_LV` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Latvian spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.get_or_insert(loaded).clone()
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("Latvian spelling: no Hunspell lv_LV dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Latvian spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Latvian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(d));
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// The `.dic` file of the Hunspell `lv_LV` dictionary found in the dictionaries folder or the
/// system locations now (File › Options › Proofing shows it); `None` when there is none. A
/// dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is an Latvian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Latvian dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Latvian? Numbers, single letters, short words in capitals
/// (acronyms: `LR`, `ES`), words with digits and words in other scripts are accepted, and so
/// is every word when no Latvian dictionary is installed. A hyphenated word is correct when
/// the dictionary has it whole or has each part (`Rīga-Jūrmala`).
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true; // no Latvian dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || (opts.ignore_numbers && w.chars().any(|c| c.is_ascii_digit())) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if opts.ignore_uppercase && w.chars().all(|c| !c.is_lowercase()) && n <= 6 {
        return true;
    }
    if !w.chars().any(is_latin_letter) {
        return true; // other scripts: no dictionary
    }
    if w.contains('-') {
        return known(d, w)
            || w.split('-').all(|p| {
                p.chars().count() <= 1 || (opts.ignore_uppercase && p.chars().all(|c| !c.is_lowercase()) && p.chars().count() <= 6) || known(d, p)
            });
    }
    known(d, w)
}

/// Is one word in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_lookup_accepts_latvian_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-lv-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["lv_LV", "lv-LV", "lv"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "suns"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn latvian_words_affixes_case_and_hyphens() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert_eq!(d.len(), 15);
        assert!(d.rule_count() > 0);
        for w in ["suns", "Suns", "SUNS", "māja", "mājas", "mājai", "māju", "skolas", "kaķis", "ūdens", "Rīga", "Latvija", "Rīga-Latvija", "LR"]
        {
            assert!(is_correct(w), "{w}");
        }
        for w in ["suuuuuns", "sunss", "mājs", "kaķs", "rīga", "mājass", "suns-xyz"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected_in_every_case() {
        test_dict::install();
        for w in ["zqterlarangs", "Zqterlarangs", "ZQTERLARANGS"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, "suuuuuns"));
        assert!(install_dictionary("FLAG long\n", "1\nsuns\n").is_err());
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for w in ["", "a", "ā", "2026", "LV", "日本語", "-", "--"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zqkatasuns"));
        crate::add_word("Zqkatasuns");
        assert!(is_correct("zqkatasuns"));
    }

    #[test]
    fn latvian_dispatch_messages_options_and_mixed_spans() {
        test_dict::install();
        assert!(crate::has_dictionary(Some("lv-LV")));
        assert!(crate::is_correct_for("suns", Some("lav")));
        let text = "Suns suuuuuns";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Lv)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "suuuuuns");
        assert_eq!(issues[0].message, "Iespējama pareizrakstības kļūda");
        assert_eq!(issues[0].lang, crate::ProofLang::Lv);
        let text = "Suns this is is wrong";
        let issues = crate::check_text(text, &[(0..4, crate::ProofLang::Lv), (5..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 5));
        let opts = crate::Options { ignore_uppercase: false, ..Default::default() };
        assert!(!crate::is_correct_in_with("ZQTERLARANGS", crate::ProofLang::Lv, &opts));
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Latvian checks
    /// those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_latvian_check() {
        test_dict::install();
        let lang = crate::ProofLang::Lv;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("SUNSS", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("SUNSS", lang, &check_caps));
        assert!(crate::is_correct_in_with("SUNS", lang, &check_caps));
        assert!(crate::is_correct_in_with("sunss2", lang, &defaults));
        assert!(!crate::is_correct_in_with("sunss2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("SUNSS"), "direct checks keep the defaults");
    }
}
