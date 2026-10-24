//! Slovak proofing using the installed Hunspell `sk_SK` dictionary (never bundled; Debian and
//! Ubuntu ship it as `hunspell-sk`, LibreOffice as its `dict-sk` extension). Suggestions use the
//! dictionary's own MAP/REP/TRY/KEY tables through the ranking Czech uses
//! ([`crate::cs::suggest`]). The current sk-spell dictionary marks its `naj…ší` superlatives with
//! `CIRCUMFIX`, which the affix engine reads ([`crate::affix`]). Grammar is limited to the
//! generic repeated-word and spacing checks, with Slovak messages; hyphenation keeps the English
//! fallback (no Slovak patterns are bundled).

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Slovak dictionary and whether it came from [`install_dictionary`] rather than a file on
/// disk (it then has no file of its own to report from [`installed_path`]): one lock, so a reload
/// and an install can't interleave their two fields into an inconsistent combination.
static DICT: RwLock<DictState> = RwLock::new(DictState { dict: None, in_memory: false });

struct DictState {
    dict: Option<Arc<AffixDict>>,
    in_memory: bool,
}

/// File names of a Slovak Hunspell dictionary.
const NAMES: &[&str] = &["sk_SK", "sk-SK", "sk"];

/// Each Slovak letter with its forms with and without a diacritic: the `MAP` table suggestions
/// use when the installed dictionary has none (the common `sk_SK` only lists single-letter `REP`
/// pairs, so a word typed without two or more of its diacritics would get no suggestion).
pub(crate) const DIACRITICS: &[&str] = &["aáä", "cč", "dď", "eé", "ií", "lľĺ", "nň", "oóô", "rŕ", "sš", "tť", "uú", "yý", "zž"];

/// The Slovak dictionary: the installed Hunspell `sk_SK` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Slovak spelling is off then;
/// see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(std::sync::PoisonError::into_inner).dict.as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.dict.get_or_insert(loaded).clone()
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        log::info!("Slovak spelling: no Hunspell sk_SK dictionary installed");
        crate::hunspell::clear_load_error(NAMES[0]);
        return AffixDict::default();
    };
    match crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)) {
        Ok(d) => d.with_default_map(DIACRITICS),
        Err(e) => {
            log::warn!("Slovak spelling: {e}");
            AffixDict::default()
        }
    }
}

/// Use this Slovak dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8),
/// for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?.with_default_map(DIACRITICS);
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    slot.dict = Some(Arc::new(d));
    slot.in_memory = true;
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    slot.dict = None;
    slot.in_memory = false;
}

/// The `.dic` file of the installed Slovak dictionary, searching the dictionaries folder and the
/// system locations now (File › Options › Proofing shows it); `None` when there is none. A
/// dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    if DICT.read().unwrap_or_else(std::sync::PoisonError::into_inner).in_memory {
        return None;
    }
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a Slovak dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Slovak dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Slovak? Numbers, single letters, short words in capitals
/// (acronyms: `SR`, `EÚ`), words with digits and words in other scripts are accepted, and so is
/// every word when no Slovak dictionary is installed.
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
        return true; // no Slovak dictionary installed: no spelling marks
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
    // A hyphenated word is right as a whole (the dictionary lists `bielo-čierny` and its
    // forms, `najbielo-čiernejší`) or part by part (`slovensko-anglický`), as Hunspell checks it.
    if w.contains('-') {
        return known(d, w) || w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    known(d, w)
}

/// Is one word (no hyphens) in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_lookup_accepts_slovak_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-sk-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["sk_SK", "sk-SK", "sk"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "dĺžka"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn slovak_words_affixes_chains_case_and_diacritics() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert_eq!(d.len(), 26);
        for w in [
            "mesto",
            "mesta",
            "meste",
            "mestu",
            "žena",
            "ženy",
            "žene",
            "ženou",
            "ženu",
            "rýchly",
            "rýchlo",
            "rýchlejší",
            "nerýchly",
            "Bratislava",
            "Bratislave",
            "Bratislavu",
            "Mesto",
            "POČÍTAČ",
            "piatok",
            "ďakujeme",
            "kôň",
            "stôl",
            "päť",
            "mäso",
            "dĺžka",
            "vŕba",
            "ľúbiť",
            "ťava",
            "dzbán",
            "džús",
            "chlieb",
            "móda",
            "škola-počítač",
            // A hyphenated stem whose parts aren't words on their own.
            "bielo-čierny",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["mestoo", "rychly", "bratislava", "pocitac", "dakujeme", "kon", "maso", "dlžka", "vrba", "mÄso", "škola-pocitac", "bielo-čiernyy"]
        {
            assert!(!is_correct(w), "{w}");
        }
        // Every letter of the Slovak alphabet with a diacritic is in the fixture.
        for c in "áäčďéíĺľňóôŕšťúýž".chars() {
            assert!(test_dict::DIC.contains(c), "UTF-8 fixture covers {c}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected_in_every_case() {
        test_dict::install();
        for w in ["zqzakázané", "Zqzakázané", "ZQZAKÁZANÉ"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, "pocitac"));
        assert!(install_dictionary("FLAG long\n", "1\nstrom\n").is_err());
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for w in ["", "a", "2026", "SR", "EÚ", "článok2", "日本語"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zqslovenskévlastnéslovo"));
        crate::add_word("Zqslovenskévlastnéslovo");
        assert!(is_correct("zqslovenskévlastnéslovo"));
    }

    #[test]
    fn slovak_dispatch_messages_options_and_mixed_spans() {
        test_dict::install();
        assert!(crate::has_dictionary(Some("sk-SK")));
        assert!(crate::is_correct_for("mäso", Some("slk")));
        let text = "Mesto pocitac";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Sk)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "pocitac");
        assert_eq!(issues[0].message, "Možná pravopisná chyba");
        assert_eq!(issues[0].lang, crate::ProofLang::Sk);
        assert!(crate::suggest_in("pocitac", 8, crate::ProofLang::Sk).contains(&"počítač".into()));
        let text = "Mesto this is is wrong";
        let issues = crate::check_text(text, &[(0..6, crate::ProofLang::Sk), (7..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 7));
        let opts = crate::Options { ignore_uppercase: false, ..Default::default() };
        assert!(!crate::is_correct_in_with("ZQZAKÁZANÉ", crate::ProofLang::Sk, &opts));
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Slovak checks
    /// those words against the dictionary instead of accepting them (as Czech, #3966).
    #[test]
    fn proofing_options_reach_the_slovak_check() {
        test_dict::install();
        let lang = crate::ProofLang::Sk;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("ŠKULA", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("ŠKULA", lang, &check_caps));
        assert!(crate::is_correct_in_with("ŠKOLA", lang, &check_caps));
        assert!(crate::is_correct_in_with("škula2", lang, &defaults));
        assert!(!crate::is_correct_in_with("škula2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("ŠKULA"), "direct checks keep the defaults");
    }
}
