//! Indonesian proofing using the installed Hunspell `id_ID` dictionary (never bundled; Debian and
//! Ubuntu ship it as `hunspell-id`, LibreOffice as its `dict-id` extension). Suggestions use the
//! dictionary's own MAP/REP/TRY/KEY tables. Grammar is limited to generic repeated-word and
//! spacing checks with Indonesian messages; hyphenation keeps the English fallback (no Indonesian
//! patterns are bundled).

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Indonesian dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of an Indonesian Hunspell dictionary (`in` is the old ISO 639 code, still used by
/// Java locales).
const NAMES: &[&str] = &["id_ID", "id-ID", "id", "in_ID"];

/// Clitics PUEBI joins with a hyphen to a capitalized word, an abbreviation or a number
/// (`se-Indonesia`, `KTP-nya`, `Jakarta-lah`); they aren't dictionary words on their own.
const CLITICS: &[&str] = &["se", "nya", "ku", "mu", "lah", "kah", "pun"];

/// The Indonesian dictionary: the installed Hunspell `id_ID` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Indonesian spelling is off
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
        log::info!("Indonesian spelling: no Hunspell id_ID dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Indonesian spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Indonesian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
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

/// The `.dic` file of the Hunspell `id_ID` dictionary found in the dictionaries folder or the
/// system locations now (File › Options › Proofing shows it); `None` when there is none. A
/// dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is an Indonesian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Indonesian dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Indonesian? Numbers, single letters, short words in capitals
/// (acronyms: `DPR`, `KTP`), words with digits and words in other scripts are accepted, and so
/// is every word when no Indonesian dictionary is installed. A hyphenated word is correct when
/// the dictionary has it whole (`kupu-kupu`, `sayur-mayur`) or has each part (`anak-anak`,
/// `rumah-rumah`); a hyphenated clitic (`KTP-nya`, `se-Indonesia`) counts as known.
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
        return true; // no Indonesian dictionary installed: no spelling marks
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
                p.chars().count() <= 1
                    || CLITICS.iter().any(|c| p.eq_ignore_ascii_case(c))
                    || (opts.ignore_uppercase && p.chars().all(|c| !c.is_lowercase()) && p.chars().count() <= 6)
                    || known(d, p)
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
    fn dictionary_lookup_accepts_indonesian_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-id-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["id_ID", "id-ID", "id", "in_ID"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "rumah"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn indonesian_words_affixes_case_and_hyphens() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert_eq!(d.len(), 20);
        assert!(d.rule_count() > 0);
        for w in [
            "rumah",
            "makan",
            "makanan",
            "dimakan",
            "membaca",
            "bacaan",
            "dibaca",
            "Jakarta",
            "Rumah",
            "SEKOLAH",
            "anak-anak",
            "rumah-rumah",
            "kupu-kupu",
            "KTP-nya",
            "se-Indonesia",
            "Jakarta-lah",
            "DPR",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["rumha", "makanann", "skolah", "jakarta", "kupu", "anak-anka", "mAkan", "bukunya-xyz"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected_in_every_case() {
        test_dict::install();
        for w in ["zqterlarang", "Zqterlarang", "ZQTERLARANG"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, "rumha"));
        assert!(install_dictionary("FLAG long\n", "1\nrumah\n").is_err());
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for w in ["", "a", "2026", "RI", "ke-2", "日本語", "-", "--"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zqkatasendiri"));
        crate::add_word("Zqkatasendiri");
        assert!(is_correct("zqkatasendiri"));
    }

    #[test]
    fn indonesian_dispatch_messages_options_and_mixed_spans() {
        test_dict::install();
        assert!(crate::has_dictionary(Some("id-ID")));
        assert!(crate::is_correct_for("rumah", Some("ind")));
        let text = "Rumah sekolha";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Id)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "sekolha");
        assert_eq!(issues[0].message, "Kemungkinan salah eja");
        assert_eq!(issues[0].lang, crate::ProofLang::Id);
        assert!(crate::suggest_in("sekolha", 8, crate::ProofLang::Id).contains(&"sekolah".into()));
        let text = "Rumah this is is wrong";
        let issues = crate::check_text(text, &[(0..5, crate::ProofLang::Id), (6..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 6));
        let opts = crate::Options { ignore_uppercase: false, ..Default::default() };
        assert!(!crate::is_correct_in_with("ZQTERLARANG", crate::ProofLang::Id, &opts));
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Indonesian checks
    /// those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_indonesian_check() {
        test_dict::install();
        let lang = crate::ProofLang::Id;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("RUMHA", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("RUMHA", lang, &check_caps));
        assert!(crate::is_correct_in_with("RUMAH", lang, &check_caps));
        assert!(crate::is_correct_in_with("RUMHA-nya", lang, &defaults));
        assert!(!crate::is_correct_in_with("RUMHA-nya", lang, &check_caps), "a part in capitals is checked too");
        assert!(crate::is_correct_in_with("rumha2", lang, &defaults));
        assert!(!crate::is_correct_in_with("rumha2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("RUMHA"), "direct checks keep the defaults");
    }
}
