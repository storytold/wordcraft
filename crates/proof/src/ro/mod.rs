//! Romanian proofing.
//!
//! - **Spelling** against the Hunspell `ro_RO` dictionary installed on the system (found by
//!   [`crate::hunspell`]: `ro_RO.aff`/`ro_RO.dic` from the OS, a package manager or
//!   LibreOffice; it isn't bundled — the upstream rospell file is GPL/LGPL/MPL-licensed).
//!   Without it Romanian text gets no spelling marks. The dictionary spells `ș` and `ț` with a
//!   comma below; the cedilla forms (`ş`, `ţ`) that older fonts and keyboards produce are read
//!   as the same letters, so text typed with them isn't flagged word by word. Hyphenated forms
//!   (`într-o`, `s-a`, `dă-mi`, `n-am`) are checked part by part — the dictionary lists the
//!   clipped pieces (`într`, `s`, `n`) as words. The user dictionary applies as in English.
//! - **Suggestions** tuned for Romanian ([`suggest`]): missing diacritics first (`fata` →
//!   `fată`/`față`, `si` → `și`), the old `î` inside words (`cîine` → `câine`), then the
//!   dictionary's `REP` confusions, a missing space, one-letter typos, and a dictionary scan for
//!   anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Romanian messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Romanian dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a Romanian Hunspell dictionary (Romania first, then Moldova).
const NAMES: &[&str] = &["ro_RO", "ro-RO", "ro_MD", "ro"];

/// The Romanian dictionary: the installed Hunspell `ro_RO` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Romanian spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(|e| e.into_inner());
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.get_or_insert(loaded).clone()
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("Romanian spelling: no Hunspell ro_RO dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Romanian spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Romanian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The installed Romanian `.dic` file, if any (File › Options › Proofing lists it).
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a Romanian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Romanian dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// `word` with the cedilla `ş`/`ţ` (U+015F, U+0163 and capitals) replaced by the comma-below
/// `ș`/`ț` the dictionary and the standard use.
pub fn comma_below(word: &str) -> String {
    word.chars()
        .map(|c| match c {
            'ş' => 'ș',
            'Ş' => 'Ș',
            'ţ' => 'ț',
            'Ţ' => 'Ț',
            c => c,
        })
        .collect()
}

/// Is `word` spelled correctly in Romanian? Numbers, single letters and words in other scripts
/// are accepted, and so is every word when no Romanian dictionary is installed. Words in
/// capitals (`UE`, `CFR`) and words with digits are left to File › Options › Proofing (the
/// `ignore_uppercase` / `ignore_numbers` choices, applied by [`crate::is_correct_in_with`]): with
/// them off, `CĂSUȚĂ` is checked like `căsuță`.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    if d.is_empty() {
        return true; // no Romanian dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
    let w = w.trim_matches('-');
    let n = w.chars().count();
    if n <= 1 || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if !w.chars().any(is_latin_letter) {
        return true; // other scripts: no dictionary
    }
    let w = comma_below(w);
    if known(d, &w) {
        return true;
    }
    // `într-o`, `s-a`, `dă-mi-l`, `nord-est`: every part is a word (or a one-letter clitic).
    w.contains('-') && w.split('-').all(|p| p.chars().count() <= 1 || known(d, p))
}

/// Is one word in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_loads() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.len() > 50, "{}", d.len());
        assert!(d.rule_count() >= 4, "the test dictionary's affix rules");
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn moldovan_dictionary_files_are_found() {
        // ro_MD is searched too, so users with only that one installed get Romanian spelling.
        let dir = std::env::temp_dir().join(format!("wc-ro-md-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("ro_MD.aff"), test_dict::AFF).expect("aff");
        std::fs::write(dir.join("ro_MD.dic"), test_dict::DIC).expect("dic");
        let found = crate::hunspell::find_in(&dir, NAMES);
        let ok = found.as_ref().is_some_and(|(a, d)| a.ends_with("ro_MD.aff") && d.ends_with("ro_MD.dic"));
        assert!(ok, "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["fată", "câine", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(install_dictionary("FLAG long\n", "1\ncasa\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn romanian_words_are_correct() {
        test_dict::install();
        for w in [
            "casă",
            "case",
            "casa",
            "fată",
            "față",
            "câine",
            "câinii",
            "mâine",
            "și",
            "într",
            "într-o",
            "într-un",
            "s-a",
            "n-am",
            "dă-mi",
            "Brașov",
            "București",
            "Ţară",
            "ţară",
            "şi",
            "Şi",
            "lucra",
            "lucrez",
            "lucrează",
            "reface",
            "CASĂ",
            "ȚARĂ",
            "2026",
            "a",
            "niciun",
            "nicio",
            "nord-est",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["cîine", "mîine", "fataa", "lcurez", "casaă", "București-xqz", "într-xqz", "niciunn"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn capitals_and_digits_follow_the_proofing_options() {
        test_dict::install();
        // Word's defaults skip words in capitals and with digits…
        for w in ["UE", "CFR", "CASAA", "B2B"] {
            assert!(crate::is_correct_in(w, crate::ProofLang::Ro), "{w}");
        }
        // …and with those choices off they are checked against the dictionary.
        let all = crate::Options { ignore_uppercase: false, ignore_numbers: false, ignore_internet: false };
        for w in ["CASAA", "CÎINE", "B2B"] {
            assert!(!crate::is_correct_in_with(w, crate::ProofLang::Ro, &all), "{w}");
        }
        assert!(crate::is_correct_in_with("CASĂ", crate::ProofLang::Ro, &all));
    }

    #[test]
    fn cedilla_letters_read_as_comma_below() {
        assert_eq!(comma_below("Ştiinţă şi ţară"), "Știință și țară");
        assert_eq!(comma_below("plain"), "plain");
        test_dict::install();
        assert!(is_correct("ştiinţă") && is_correct("știință"));
    }

    /// Against the dictionary installed on this machine (LibreOffice's rospell, a package…):
    /// `cargo test -p wordcraft-proof ro::tests::installed -- --ignored --nocapture`. Not run by
    /// default — CI machines have no Romanian dictionary, and it replaces the test one.
    #[test]
    #[ignore]
    fn installed_dictionary_checks_real_text() {
        reload_dictionary();
        if !has_dictionary() {
            eprintln!("no ro_RO dictionary installed");
            return;
        }
        let text = "Ieri dimineață, fata vecinilor s-a dus într-o librărie din București și și-a cumpărat \
                    două cărți despre științele naturii. Mâine vom merge împreună la munte, dacă nu plouă.";
        let bad: Vec<&str> = crate::words(text).iter().filter_map(|&(a, b)| text.get(a..b)).filter(|w| !is_correct(w)).collect();
        assert!(bad.is_empty(), "{bad:?}");
        for w in ["cîine", "stiinta", "fata", "Bucuresti", "intrun"] {
            eprintln!("{w} → {:?}", suggest(w, 5));
        }
        assert!(!is_correct("cîine") && !is_correct("Bucuresti"));
        assert_eq!(suggest("cîine", 3).first().map(String::as_str), Some("câine"));
        assert!(!suggest("cîine", 8).iter().any(|s| s.contains('-')), "{:?}", suggest("cîine", 8));
        assert_eq!(suggest("intrun", 3).first().map(String::as_str), Some("într-un"));
        assert!(suggest("Bucuresti", 5).iter().any(|s| s == "București"));
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zxqcuvântromânesc"));
        crate::add_word("Zxqcuvântromânesc");
        assert!(is_correct("zxqcuvântromânesc"));
    }
}
