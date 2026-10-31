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

/// File names of a Hungarian Hunspell dictionary; the first is the key of its load error.
pub(crate) const NAMES: &[&str] = &["hu_HU", "hu-HU", "hu"];

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
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("Hungarian spelling: no Hunspell hu_HU dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
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

/// Is `c` a sign hu_HU lists with its endings (`%-os`, `‰-es`, `§-ával`, `°C-on`, `€-ért`)?
/// Other punctuation before a word (`(`, `„`) is no part of it.
fn is_sign(c: char) -> bool {
    matches!(c, '%' | '‰' | '§' | '°' | '€')
}

/// The texts the word at `a..b` of `text` is checked as, any of which may be correct; empty: the
/// word alone. A word right after a sign, or after a sign and a hyphen, is also checked with the
/// sign (`os` of `27%-os`, `C-on` of `5 °C-on`), which word splitting leaves out; a word that is
/// right alone stays right (`ért` of `€ért`). An ending after an abbreviation (`ben` in
/// `Kft.-ben.`) is checked with the abbreviation, so a wrong ending is marked alone. The
/// abbreviation is checked with its ending or else with its period alone (`Kft.` in `Kft.-bem` is
/// right); a word before a period with it (`pl.`, an abbreviation the dictionary may list with
/// it).
pub(crate) fn spelling_spans(text: &str, a: usize, b: usize) -> Vec<(usize, usize)> {
    let is_w = |c: char| c.is_alphanumeric();
    let word_end = |from: usize| text.get(from..).map(|r| from + r.find(|c: char| !is_w(c)).unwrap_or(r.len()));
    // The ending after an abbreviation: `ben` in `Kft.-ben`.
    if let Some(before) = text.get(..a).and_then(|t| t.strip_suffix(".-")) {
        let start = before.rfind(|c: char| !is_w(c)).map_or(0, |i| i + before.get(i..).and_then(|r| r.chars().next()).map_or(1, char::len_utf8));
        if start < before.len() {
            return vec![(start, b)];
        }
    }
    let rest = text.get(b..).unwrap_or("");
    let mut out = Vec::new();
    // A sign with an ending: `ával` in `§-ával`, `os` in `27%-os`, `C-on` in `°C-on`.
    let sign_before = |end: usize| text.get(..end)?.chars().next_back().filter(|&c| is_sign(c)).map(|c| end - c.len_utf8());
    if let Some(s) = text.get(..a).and_then(|t| t.strip_suffix('-')).and_then(|t| sign_before(t.len())).or_else(|| sign_before(a)) {
        out.push((s, b));
    }
    if rest.starts_with(".-")
        && let Some(end) = word_end(b + 2).filter(|&e| e > b + 2)
    {
        out.push((a, end));
    }
    if rest.starts_with('.') {
        out.push((a, b + 1));
    }
    out
}

/// Is `word` spelled correctly in Hungarian? Numbers and words with digits (`2026-ban` follows
/// the number's own rules), single letters, short words in capitals (acronyms: `MÁV`, `OTP`) and
/// words in other scripts are accepted, and so is every word when no Hungarian dictionary is
/// installed. A word given with its period (`pl.`) may be an abbreviation the dictionary lists
/// with it (`pl.`, `stb.`, `kb.`); without one it is checked as a word.
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
    let abbreviation = word.trim_end().ends_with('.') && d.check(&format!("{w}."));
    // A sign with its ending, as the dictionary lists it (`%-os`, `§-ával`): trimming took the sign.
    let signed = || word.trim().starts_with(is_sign) && d.check(word.trim());
    if abbreviation || d.check(w) || signed() || crate::in_user_dictionary(&w.to_lowercase()) {
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
    fn abbreviations_are_checked_with_their_period() {
        // The dictionary lists `pl.` and `stb.` with the period; without it they are no words,
        // as with the Hunspell command line.
        test_dict::install();
        assert!(is_correct("pl.") && is_correct("stb.") && is_correct("Pl."));
        assert!(!is_correct("pl") && !is_correct("stb"));
        assert!(is_correct("ház.") && !is_correct("hazx."), "a word at the end of a sentence");
        assert!(crate::check_spelling_in("A ház pl. szép, stb.", crate::ProofLang::Hu).is_empty());
        let text = "A ház pl szép";
        let issues = crate::check_spelling_in(text, crate::ProofLang::Hu);
        assert_eq!(issues.iter().map(|i| &text[i.start..i.end]).collect::<Vec<_>>(), ["pl"]);
    }

    #[test]
    fn abbreviations_take_hyphenated_endings() {
        // `Kft.-ban` is one word, as with the Hunspell command line (`Kft.-ben`, `stb.-vel`): its
        // ending isn't a word of its own. Without the period it stays wrong.
        test_dict::install();
        let ok = "A Kft.-ban szép ház.";
        assert!(crate::check_spelling_in(ok, crate::ProofLang::Hu).is_empty(), "{:?}", crate::check_spelling_in(ok, crate::ProofLang::Hu));
        let text = "A Kft-ban szép ház.";
        let issues = crate::check_spelling_in(text, crate::ProofLang::Hu);
        assert!(!issues.is_empty());
        assert_eq!(spelling_spans("Kft.-ban x", 0, 3), [(0, 8), (0, 4)]);
        assert_eq!(spelling_spans("Kft.-ban x", 5, 8), [(0, 8)]);
        assert_eq!(spelling_spans("Kft.-ban.", 5, 8), [(0, 8)], "the sentence's period isn't the ending's");
        assert_eq!(spelling_spans("pl. x", 0, 2), [(0, 3)]);
        assert_eq!(spelling_spans("Kft.- x", 0, 3), [(0, 4)]);
        assert!(spelling_spans("ház x", 0, 4).is_empty());
        // At the end of a sentence, and with a wrong ending only the ending is marked.
        assert!(crate::check_spelling_in("Szép ház a Kft.-ban.", crate::ProofLang::Hu).is_empty());
        let text = "A Kft.-bam szép.";
        let marked: Vec<&str> = crate::check_spelling_in(text, crate::ProofLang::Hu).iter().map(|i| &text[i.start..i.end]).collect();
        assert_eq!(marked, ["bam"]);
    }

    #[test]
    fn long_compounds_follow_the_six_three_rule() {
        // Past two words a compound may have six syllables: `almafaház` has four,
        // `almafaházkulcsasztal` seven.
        test_dict::install();
        assert!(is_correct("almafaház") && is_correct("házkulcs"));
        assert!(!is_correct("almafaházkulcsasztal"));
    }

    #[test]
    fn signs_take_the_endings_the_dictionary_lists() {
        // `27%-os`, `§-ban`, `°C-on` and `€-ért` are words of the dictionary, as with the Hunspell
        // command line: the ending isn't checked alone. A wrong ending is marked alone.
        test_dict::install();
        let marks = |text: &str| -> Vec<String> {
            crate::check_spelling_in(text, crate::ProofLang::Hu).iter().map(|i| text[i.start..i.end].to_string()).collect()
        };
        assert!(marks("A 27%-os ház, a 2. §-ban, 5 °C-on és 9 €-ért.").is_empty());
        assert!(marks("§-ban és €-ért.").is_empty(), "a sign at the start of the text");
        assert!(is_correct("%-os") && is_correct("§-ban") && is_correct("€-ért"));
        assert!(!is_correct("%-ox"));
        assert_eq!(spelling_spans("27%-os x", 4, 6), [(2, 6)]);
        assert_eq!(spelling_spans("5 °C-on", 4, 8), [(2, 8)]);
        assert_eq!(spelling_spans("€-ért", 4, 8), [(0, 8)], "a multi-byte sign at the start of the text");
        assert_eq!(marks("A 27%-ox ház és a §-bam."), ["ox", "bam"]);
        // Punctuation before a word is no sign: the word alone is checked, a typo stays marked.
        assert!(spelling_spans("a (ház", 3, 6).is_empty() && spelling_spans("„-ház", 4, 7).is_empty());
        assert_eq!(marks("A (hazx) és „hazx”."), ["hazx", "hazx"]);
        assert!(marks("A (ház) és „ház”.").is_empty());
    }

    #[test]
    fn a_failed_load_is_reported_under_the_hungarian_name() {
        assert_eq!(crate::load_error_key(crate::ProofLang::Hu), Some("hu_HU"));
        assert_eq!(NAMES.first(), Some(&"hu_HU"));
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
