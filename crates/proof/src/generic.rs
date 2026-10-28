//! Spelling for languages that have only a Hunspell dictionary here: Bulgarian, Romanian,
//! Slovenian, Albanian, Lithuanian and Bosnian. Each reads the installed dictionary (never
//! bundled; LibreOffice's collection and most Linux distributions have one), and its suggestions
//! use that dictionary's own MAP/REP/TRY/KEY tables ([`crate::cs::suggest::suggest_in`]). There
//! are no grammar rules for them, and hyphenation keeps what the language had (Bulgarian's
//! syllable rules, else the English fallback).
//!
//! A language joins by getting a [`ProofLang`] and a row in [`LANGUAGES`]: nothing else here is
//! specific to one language.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::ProofLang;
use crate::affix::AffixDict;

/// A language spelled with a Hunspell dictionary alone.
pub struct Language {
    /// Its proofing language.
    pub lang: ProofLang,
    /// File names its dictionary may have, the usual one first (also the key of its load error).
    pub names: &'static [&'static str],
    /// Its English name in the log.
    pub english: &'static str,
    /// The spelling mark's message, in the language.
    pub message: &'static str,
    /// The letters its dictionary covers: words with none of them (another script) are skipped.
    pub script: fn(char) -> bool,
}

/// Is `c` a letter of the Latin script?
fn latin(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `c` a Cyrillic letter?
fn cyrillic(c: char) -> bool {
    c.is_alphabetic() && ('\u{0400}'..='\u{052F}').contains(&c)
}

/// The languages this module spells.
pub const LANGUAGES: &[Language] = &[
    Language {
        lang: ProofLang::Bg, names: &["bg_BG", "bg-BG", "bg"], english: "Bulgarian", message: "Възможна правописна грешка", script: cyrillic
    },
    Language {
        lang: ProofLang::Ro,
        names: &["ro_RO", "ro-RO", "ro"],
        english: "Romanian",
        message: "Posibilă greșeală de ortografie",
        script: latin,
    },
    Language { lang: ProofLang::Sl, names: &["sl_SI", "sl-SI", "sl"], english: "Slovenian", message: "Mogoča pravopisna napaka", script: latin },
    Language {
        lang: ProofLang::Sq, names: &["sq_AL", "sq-AL", "sq"], english: "Albanian", message: "Gabim i mundshëm drejtshkrimor", script: latin
    },
    Language { lang: ProofLang::Lt, names: &["lt_LT", "lt-LT", "lt"], english: "Lithuanian", message: "Galima rašybos klaida", script: latin },
    Language { lang: ProofLang::Bs, names: &["bs_BA", "bs-BA", "bs"], english: "Bosnian", message: "Moguća pravopisna greška", script: latin },
];

/// The row of `lang`, if this module spells it.
pub fn language(lang: ProofLang) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.lang == lang)
}

/// The loaded dictionaries: a language is missing until first use, then has the installed one
/// (or an empty one).
static DICTS: RwLock<BTreeMap<ProofLang, Arc<AffixDict>>> = RwLock::new(BTreeMap::new());

/// The dictionary of `lang`: the installed Hunspell one (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available or `lang` isn't spelled here
/// (no spelling marks then).
pub fn dictionary(lang: ProofLang) -> Arc<AffixDict> {
    if let Some(d) = DICTS.read().unwrap_or_else(|e| e.into_inner()).get(&lang) {
        return d.clone();
    }
    let loaded = Arc::new(language(lang).map(load_installed).unwrap_or_default());
    let mut dicts = DICTS.write().unwrap_or_else(|e| e.into_inner());
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    dicts.entry(lang).or_insert(loaded).clone()
}

fn load_installed(l: &Language) -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(l.names) else {
        crate::hunspell::clear_load_error(l.names[0]);
        log::info!("{} spelling: no Hunspell {} dictionary installed", l.english, l.names[0]);
        return AffixDict::default();
    };
    crate::hunspell::note_load(l.names[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("{} spelling: {e}", l.english);
        AffixDict::default()
    })
}

/// Use this dictionary for `lang` from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8). On error, or for a language not spelled here, the current dictionary stays.
pub fn install_dictionary(lang: ProofLang, aff: &str, dic: &str) -> Result<(), String> {
    if language(lang).is_none() {
        return Err(format!("{lang:?} isn't spelled with a Hunspell dictionary alone"));
    }
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    DICTS.write().unwrap_or_else(|e| e.into_inner()).insert(lang, Arc::new(d));
    Ok(())
}

/// Forget every loaded dictionary, so the next check looks for the installed ones again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionaries() {
    DICTS.write().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Forget the loaded dictionary of `lang`.
pub fn reload_dictionary(lang: ProofLang) {
    DICTS.write().unwrap_or_else(|e| e.into_inner()).remove(&lang);
}

/// The `.dic` file of the dictionary found for `lang` in the dictionaries folder or the system
/// locations now; `None` when there is none.
pub fn installed_path(lang: ProofLang) -> Option<std::path::PathBuf> {
    crate::hunspell::find(language(lang)?.names).map(|(_, dic)| dic)
}

/// Is a dictionary for `lang` available (its spelling is checked)?
pub fn has_dictionary(lang: ProofLang) -> bool {
    !dictionary(lang).is_empty()
}

/// Is `word` spelled correctly in `lang`, with the default options? Numbers, single letters,
/// short words in capitals (acronyms), words with digits and words in another script are
/// accepted, and so is every word when `lang` has no dictionary. A hyphenated word is correct
/// when the dictionary has it whole or has each part.
pub fn is_correct(lang: ProofLang, word: &str) -> bool {
    is_correct_with_options(lang, word, &crate::Options::default())
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(lang: ProofLang, word: &str, opts: &crate::Options) -> bool {
    match language(lang) {
        Some(l) => check(&dictionary(lang), l.script, word, opts),
        None => true,
    }
}

/// [`is_correct`] against a given dictionary, whose letters `script` tells (an empty one accepts
/// everything).
pub fn is_correct_with(d: &AffixDict, script: fn(char) -> bool, word: &str) -> bool {
    check(d, script, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, script: fn(char) -> bool, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true;
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
    if !w.chars().any(script) {
        return true; // another script: not this dictionary's
    }
    if w.contains('-') {
        return known(d, w) || w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    known(d, w)
}

/// Is one word in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

/// Spelling suggestions for a word misspelled in `lang`, best first, at most `max`, in the
/// word's case.
pub fn suggest(lang: ProofLang, word: &str, max: usize) -> Vec<String> {
    if language(lang).is_none() {
        return Vec::new();
    }
    crate::cs::suggest::suggest_in(&dictionary(lang), word, max)
}

#[cfg(test)]
pub(crate) mod test_dict {
    //! A hand-written synthetic Romanian fixture (no words or rules were copied from a real
    //! dictionary); not a redistributable dictionary.

    pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY aeiurntăcsșțlmo
MAP 1
MAP aă
REP 1
REP ş ș
SFX A Y 1
SFX A 0 ul .
";

    pub const DIC: &str = "10
carte
casă
copil/A
drum/A
frumos
mergem
școală
țară
și
zqinterzis/F
";

    pub fn install() {
        if let Err(e) = super::install_dictionary(crate::ProofLang::Ro, AFF, DIC) {
            panic!("test dictionary: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_one_row_and_a_usual_file_name() {
        for (i, l) in LANGUAGES.iter().enumerate() {
            assert!(LANGUAGES[..i].iter().all(|o| o.lang != l.lang), "{:?} listed twice", l.lang);
            assert!(l.lang.is_proofed(), "{:?}", l.lang);
            let code = l.lang.code();
            assert!(l.names[0].starts_with(&format!("{code}_")) && l.names.contains(&code), "{:?}", l.names);
        }
        assert!(language(ProofLang::It).is_none() && language(ProofLang::Other).is_none());
        assert!(install_dictionary(ProofLang::It, test_dict::AFF, test_dict::DIC).is_err());
        assert!(is_correct(ProofLang::Other, "xyzzy") && suggest(ProofLang::Other, "xyzzy", 4).is_empty());
    }

    #[test]
    fn dictionary_lookup_accepts_each_file_name() {
        let dir = std::env::temp_dir().join(format!("wordcraft-generic-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in language(ProofLang::Ro).unwrap().names {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, language(ProofLang::Ro).unwrap().names).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, latin, "copilul"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn words_affixes_case_hyphens_and_scripts() {
        test_dict::install();
        assert!(has_dictionary(ProofLang::Ro));
        for w in ["carte", "Carte", "CASĂ", "copilul", "drumul", "școală", "țară", "și", "carte-drum", "ONU", "2026", "Москва", "x"] {
            assert!(is_correct(ProofLang::Ro, w), "{w}");
        }
        for w in ["crate", "copilull", "scoala", "cArte", "carte-drmu", "zqinterzis", "Zqinterzis", "ZQINTERZIS"] {
            assert!(!is_correct(ProofLang::Ro, w), "{w}");
        }
        crate::add_word("Zqcuvântmeu");
        assert!(is_correct(ProofLang::Ro, "zqcuvântmeu"));
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, the spelling-only
    /// languages check those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_generic_check() {
        test_dict::install();
        let lang = ProofLang::Ro;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("CRATE", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("CRATE", lang, &check_caps));
        assert!(crate::is_correct_in_with("CARTE", lang, &check_caps));
        assert!(crate::is_correct_in_with("crate2", lang, &defaults));
        assert!(!crate::is_correct_in_with("crate2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct(lang, "CRATE"), "direct checks keep the defaults");
    }

    #[test]
    fn suggestions_come_from_the_dictionary() {
        test_dict::install();
        for (word, want) in [("crate", "carte"), ("scoală", "școală"), ("tară", "țară"), ("Copilu", "Copilul")] {
            assert!(suggest(ProofLang::Ro, word, 8).contains(&want.into()), "{word}: {:?}", suggest(ProofLang::Ro, word, 8));
        }
        assert!(!suggest(ProofLang::Ro, "zqinterzi", 8).contains(&"zqinterzis".into()));
    }

    #[test]
    fn a_missing_dictionary_means_no_marks() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, latin, "crate"));
        assert!(install_dictionary(ProofLang::Ro, "FLAG long\n", "1\ncarte\n").is_err());
    }

    #[test]
    fn dispatch_messages_and_mixed_spans() {
        test_dict::install();
        assert!(crate::has_dictionary(Some("ro-RO")));
        assert!(crate::is_correct_for("copilul", Some("ron")));
        let text = "Copilul merge la şcoala";
        let issues = crate::check_text(text, &[(0..text.len(), ProofLang::Ro)]);
        let marked: Vec<&str> = issues.iter().map(|i| &text[i.start..i.end]).collect();
        assert!(marked.contains(&"merge") && marked.contains(&"şcoala"), "{marked:?}");
        assert!(issues.iter().all(|i| i.message == "Posibilă greșeală de ortografie" && i.lang == ProofLang::Ro));
        assert!(crate::suggest_in("crate", 8, ProofLang::Ro).contains(&"carte".into()));
        assert!(crate::check_grammar_in("carte carte", ProofLang::Ro).is_empty(), "no grammar rules");
        let text = "Copilul this is is wrong";
        let issues = crate::check_text(text, &[(0..7, ProofLang::Ro), (8..text.len(), ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 8));
    }
}
