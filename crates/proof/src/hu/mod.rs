//! Hungarian proofing.
//!
//! - **Spelling** against the Hungarian Hunspell dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `hu_HU.aff`/`hu_HU.dic` from the OS, a package
//!   manager or LibreOffice; it isn't bundled). Without it Hungarian text gets no spelling marks.
//!   The dictionary describes Hungarian compounds (`almafa`, `házkulcsban`) and the 6–3 rule for
//!   long ones, which the affix engine applies ([`crate::affix`]); hyphenated words are split at
//!   the dictionary's own break points (`Petőfi-vers`, `SMS-ben`). Capitalized stems are names; a
//!   sentence may start with any word and headings may be in capitals. The user dictionary
//!   applies as in English.
//! - **Suggestions** tuned for Hungarian ([`suggest()`]): missing accents first (`kerek` →
//!   `kérek`, `orom` → `öröm`), then the dictionary's common confusions (`j`/`ly`), a missing space
//!   (`nemis` → `nem is`), one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Hungarian messages.
//! - **Hyphenation** ([`hyphen`]): the Hungarian patterns installed on the system
//!   (`hyph_hu_HU.dic`), or else the Hungarian syllable rules.
//!
//! # Licence of the dictionary data
//!
//! The Hungarian dictionary (Magyar Ispell, by László Németh and Ferenc Godó) and the Hungarian
//! hyphenation patterns (by László Németh and contributors) are tri-licensed MPL 1.1 / GPL 2.0+ /
//! LGPL 2.1+. WordCraft uses them under the MPL 1.1 option, which permits use in open-source
//! projects under other licences such as MIT and Apache-2.0. Neither is bundled: WordCraft reads
//! the copies installed on the system. The test dictionaries in this module are written for the
//! tests and contain no data from either.

pub mod grammar;
pub mod hyphen;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Hungarian dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a Hungarian Hunspell dictionary.
const NAMES: &[&str] = &["hu_HU", "hu-HU", "hu"];

/// The Hungarian dictionary: the installed Hunspell `hu_HU` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Hungarian spelling is off then;
/// see [`has_dictionary`]).
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
        log::info!("Hungarian spelling: no Hunspell hu_HU dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::load(&aff, &dic).unwrap_or_else(|e| {
        log::warn!("Hungarian spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Hungarian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Forget the loaded dictionary and hyphenation patterns, so the next check looks for installed
/// ones again (after [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = None;
    hyphen::reload_patterns();
}

/// The `.dic` file of the installed Hungarian dictionary, searching the dictionaries folder and
/// the system locations now (File › Options › Proofing shows it); `None` when there is none. A
/// dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a Hungarian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Hungarian dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Hungarian? Numbers and words with digits (`2026-ban` follows
/// the number's own rules), single letters, short words in capitals (acronyms: `MÁV`, `OTP`) and
/// words in other scripts are accepted, and so is every word when no Hungarian dictionary is
/// installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    if d.is_empty() {
        return true; // no Hungarian dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
    let w = w.trim_matches('-');
    let n = w.chars().count();
    if n <= 1 || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if w.chars().all(|c| !c.is_lowercase()) && n <= 6 && !w.contains('-') {
        return true;
    }
    if !w.chars().any(is_latin_letter) {
        return true; // other scripts: no dictionary
    }
    if d.check(w) || crate::in_user_dictionary(&w.to_lowercase()) {
        return true;
    }
    // A hyphenated word with a part from the user dictionary: the other parts must be words.
    // (Without one, the dictionary's own break rules have decided: `Rákóczi-híd` stays wrong.)
    let parts: Vec<&str> = w.split('-').filter(|p| !p.is_empty()).collect();
    parts.len() > 1
        && parts.iter().any(|p| crate::in_user_dictionary(&p.to_lowercase()))
        && parts.iter().all(|p| p.chars().count() <= 1 || d.check(p) || crate::in_user_dictionary(&p.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_loads() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.len() > 40, "{}", d.len());
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged_and_nothing_suggested() {
        let none = AffixDict::default();
        for w in ["kerek", "almafaa", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(suggest::suggest_in(&none, "kerek", 5).is_empty());
        assert!(install_dictionary("FLAG long\n", "1\nház\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn hungarian_words_are_correct() {
        test_dict::install();
        for w in [
            "ház",
            "házban",
            "házak",
            "Ház",
            "HÁZ",
            "almafa",
            "almafában",
            "házkulcs",
            "kulcsház",
            "Budapest",
            "Budapesten",
            "Petőfi-vers",
            "SMS-ben",
            "MÁV",
            "2026-ban",
            "3.",
            "e",
            "öröm",
            "örömmel",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["hazban", "almafaa", "budapest", "orom", "hajlak", "almaalma", "Rákóczi-híd", "házxyz"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zxqmagyarszó"));
        crate::add_word("Zxqmagyarszó");
        assert!(is_correct("zxqmagyarszó"));
        assert!(is_correct("zxqmagyarszó-ház"), "a user word in a hyphenated word");
        assert!(!is_correct("zxqmagyarszó-xyzq"), "the other part must still be a word");
    }
}
