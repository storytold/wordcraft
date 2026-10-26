//! Spanish proofing (#3360, #3553) using the installed Hunspell dictionaries (never bundled;
//! LibreOffice ships them as its `dict-es` extension, Debian and Ubuntu as `hunspell-es`). Each
//! variety reads its own dictionary: Spain ([`ProofLang::Es`]) `es_ES`, Mexico
//! ([`ProofLang::EsMx`]) `es_MX`, Latin America ([`ProofLang::Es419`]) `es_419` and Uruguay
//! ([`ProofLang::EsUy`]) `es_UY`, each falling back to a nearby or generic one. Uruguayan Spanish
//! also accepts the `vos` forms of every verb in the dictionary ([`voseo`]: `tenés`, `vení`,
//! `decime`). Suggestions use the dictionary's own MAP/REP/TRY/KEY tables (the regional
//! dictionaries' REP tables carry their differences, such as `vosotros` forms in Mexico).
//! Grammar is limited to generic repeated-word and spacing checks and the Spanish opening
//! question and exclamation marks, with Spanish messages; hyphenation keeps the English fallback
//! (no Spanish patterns are bundled).

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;
pub mod voseo;

use std::sync::{Arc, PoisonError, RwLock};

use crate::ProofLang;
use crate::affix::AffixDict;

pub use suggest::suggest;

/// The dictionary of each variety (Spain, Mexico, Latin America, Uruguay): `None` until first
/// use, then the installed one (or an empty one).
static DICTS: [RwLock<Option<Arc<AffixDict>>>; 4] = [const { RwLock::new(None) }; 4];

/// File names of a Spanish Hunspell dictionary for Spain, most specific first (`es_ANY` is the
/// generic one RLA-ES publishes).
const SPAIN: &[&str] = &["es_ES", "es-ES", "es_ANY", "es"];
/// For Mexico: its own, then a Latin American or generic one.
const MEXICO: &[&str] = &["es_MX", "es-MX", "es_419", "es-419", "es_ANY", "es"];
/// For Latin America: its own or a generic one, then the United States' and Mexico's, which
/// LibreOffice ships (it has no `es_419`) and which use Latin American spelling, then
/// Argentina's, so `es-AR` text is checked when that is the only one installed.
const LATIN_AMERICA: &[&str] = &["es_419", "es-419", "es_ANY", "es", "es_US", "es_MX", "es_AR", "es-AR"];
/// For Uruguay: its own, then Argentina's (the same Rioplatense Spanish), then a Latin American
/// or generic one.
const URUGUAY: &[&str] = &["es_UY", "es-UY", "es_AR", "es-AR", "es_419", "es-419", "es_ANY", "es"];

/// `lang`'s variety: its slot in [`DICTS`], its English name (for the log) and its dictionary's
/// file names; Spain for any other language.
fn variety(lang: ProofLang) -> (usize, &'static str, &'static [&'static str]) {
    match lang {
        ProofLang::EsMx => (1, "Spanish (Mexico)", MEXICO),
        ProofLang::Es419 => (2, "Spanish (Latin America)", LATIN_AMERICA),
        ProofLang::EsUy => (3, "Spanish (Uruguay)", URUGUAY),
        _ => (0, "Spanish (Spain)", SPAIN),
    }
}

/// The Hunspell name each variety's dictionary is known by (`es_ES`, `es_MX`, `es_419`,
/// `es_UY`): the first of its file names, used for the status-bar hint and load errors.
pub fn dictionary_name(lang: ProofLang) -> &'static str {
    variety(lang).2.first().copied().unwrap_or("es_ES")
}

/// The Spanish dictionary of `lang`'s variety: the installed one (read on first use), one given
/// with [`install_dictionary`], or an empty one when none is available (spelling is off then;
/// see [`has_dictionary`]).
pub fn dictionary(lang: ProofLang) -> Arc<AffixDict> {
    let Some(slot) = DICTS.get(variety(lang).0) else { return Arc::new(AffixDict::default()) };
    if let Some(d) = slot.read().unwrap_or_else(PoisonError::into_inner).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed(lang));
    let mut slot = slot.write().unwrap_or_else(PoisonError::into_inner);
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.get_or_insert(loaded).clone()
}

fn load_installed(lang: ProofLang) -> AffixDict {
    let (_, name, files) = variety(lang);
    let key = dictionary_name(lang);
    let Some((aff, dic)) = crate::hunspell::find(files) else {
        crate::hunspell::clear_load_error(key);
        log::info!("{name} spelling: no Hunspell {key} dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(key, &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("{name} spelling: {e}");
        AffixDict::default()
    })
}

/// Use this dictionary for `lang`'s variety from now on (Hunspell `.aff` and `.dic` text,
/// decoded to UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(lang: ProofLang, aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    if let Some(slot) = DICTS.get(variety(lang).0) {
        *slot.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(d));
    }
    Ok(())
}

/// Forget every variety's loaded dictionary, so the next check looks for an installed one again
/// (after [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    for slot in &DICTS {
        *slot.write().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The `.dic` file of the Hunspell dictionary `lang`'s variety finds in the dictionaries folder
/// or the system locations now (File › Options › Proofing shows it); `None` when there is none.
/// A dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path(lang: ProofLang) -> Option<std::path::PathBuf> {
    crate::hunspell::find(variety(lang).2).map(|(_, dic)| dic)
}

/// Is a dictionary available for `lang`'s variety (spelling is checked)?
pub fn has_dictionary(lang: ProofLang) -> bool {
    !dictionary(lang).is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Spanish dictionaries cover)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in `lang`'s Spanish? Numbers, single letters, short words in
/// capitals (acronyms: `ONU`, `CDMX`), words with digits and words in other scripts are accepted,
/// and so is every word when no dictionary is installed for the variety. Hyphenated compounds
/// are checked part by part (`teórico-práctico`, `franco-mexicano`). In Uruguayan Spanish the
/// `vos` forms of the dictionary's verbs are correct too ([`voseo`]).
pub fn is_correct(word: &str, lang: ProofLang) -> bool {
    is_correct_with(&dictionary(lang), word, lang)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(word: &str, lang: ProofLang, opts: &crate::Options) -> bool {
    check(&dictionary(lang), word, lang, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str, lang: ProofLang) -> bool {
    check(d, word, lang, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, lang: ProofLang, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true; // no Spanish dictionary installed: no spelling marks
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
        return known(d, w, lang) || w.split('-').all(|p| p.chars().count() <= 1 || known(d, p, lang));
    }
    known(d, w, lang)
}

/// Is one word in the dictionary or the user dictionary, or (in Uruguayan Spanish) the `vos`
/// form of one of the dictionary's verbs?
fn known(d: &AffixDict, w: &str, lang: ProofLang) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase()) || (lang == ProofLang::EsUy && voseo::is_vos_form(w, |inf| d.check(inf)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARIETIES: [ProofLang; 4] = [ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy];

    #[test]
    fn each_variety_looks_for_its_own_dictionary_first() {
        let dir = std::env::temp_dir().join(format!("wordcraft-es-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (es, mx, la, uy) = (ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy);
        // Alone in the folder, each file name is found by the varieties that list it.
        for (name, found_by) in [
            ("es_ES", &[es][..]),
            ("es-ES", &[es]),
            ("es_MX", &[mx, la]),
            ("es-MX", &[mx]),
            ("es_419", &[mx, la, uy]),
            ("es-419", &[mx, la, uy]),
            ("es_US", &[la]),
            ("es_UY", &[uy]),
            ("es-UY", &[uy]),
            ("es_AR", &[la, uy]),
            ("es-AR", &[la, uy]),
            ("es_ANY", &VARIETIES),
            ("es", &VARIETIES),
        ] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            for lang in VARIETIES {
                let found = crate::hunspell::find_in(&dir, variety(lang).2);
                assert_eq!(found.is_some(), found_by.contains(&lang), "{name}: {lang:?}");
                if let Some((aff, dic)) = found {
                    assert!(is_correct_with(&crate::hunspell::load(&aff, &dic).unwrap(), "canción", lang));
                }
            }
            std::fs::remove_file(dir.join(format!("{name}.aff"))).unwrap();
            std::fs::remove_file(dir.join(format!("{name}.dic"))).unwrap();
        }
        // With them all, each variety reads its own.
        for name in ["es_ES", "es_MX", "es_419", "es_UY", "es_AR", "es_ANY"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
        }
        for (lang, name) in [(es, "es_ES"), (mx, "es_MX"), (la, "es_419"), (uy, "es_UY")] {
            assert_eq!(crate::hunspell::find_in(&dir, variety(lang).2).map(|(_, dic)| dic), Some(dir.join(format!("{name}.dic"))));
            assert_eq!(dictionary_name(lang), name);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spanish_words_affixes_case_and_hyphens() {
        test_dict::install();
        let d = dictionary(ProofLang::Es);
        assert!(has_dictionary(ProofLang::Es));
        assert!(d.rule_count() > 0);
        for w in [
            "canción",
            "canciones",
            "prueba",
            "pruebas",
            "corrector",
            "correctores",
            "también",
            "México",
            "Canción",
            "CANCIÓN",
            "teórico-práctico",
            "franco-mexicano",
            "ONU",
            "Año",
            "niño",
            "pingüino",
        ] {
            assert!(is_correct(w, ProofLang::Es), "{w}");
        }
        for w in ["prueva", "corector", "tambien", "mexico", "cancion", "teórico-prático", "cAnción", "nino"] {
            assert!(!is_correct(w, ProofLang::Es), "{w}");
        }
    }

    #[test]
    fn varieties_keep_their_own_dictionaries() {
        test_dict::install();
        // The Americas' fixture lacks `vosotros` forms and has American words; Spain's is the other way round.
        assert!(is_correct("habéis", ProofLang::Es) && !is_correct("habéis", ProofLang::EsMx));
        assert!(is_correct("platicar", ProofLang::EsMx) && !is_correct("platicar", ProofLang::Es));
        assert!(is_correct("platicar", ProofLang::Es419) && is_correct("computadora", ProofLang::Es419));
        assert!(!is_correct("habéis", ProofLang::Es419) && !is_correct("habéis", ProofLang::EsUy));
        assert!(is_correct("ordenador", ProofLang::Es) && !is_correct("ordenador", ProofLang::EsMx));
    }

    /// Uruguayan Spanish accepts the `vos` forms of the dictionary's verbs, which the other
    /// varieties flag.
    #[test]
    fn uruguayan_spanish_accepts_vos_forms() {
        test_dict::install();
        for w in ["tenés", "vení", "Vení", "decime", "decímelo", "andate", "mirá", "hablás", "querés", "venís", "fijate"] {
            assert!(is_correct(w, ProofLang::EsUy), "{w}");
            assert!(!is_correct(w, ProofLang::Es419) && !is_correct(w, ProofLang::Es), "{w}");
        }
        for w in ["tenéz", "venì", "decimelo", "decíme", "zqbailá"] {
            assert!(!is_correct(w, ProofLang::EsUy), "{w}");
        }
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Spanish checks
    /// those words against the dictionary instead of accepting them (CodeRabbit review on #3562).
    #[test]
    fn proofing_options_reach_the_spanish_check() {
        test_dict::install();
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        for lang in VARIETIES {
            assert!(crate::is_correct_in_with("PRUEVA", lang, &defaults), "{lang:?}: short capitals are acronyms by default");
            assert!(!crate::is_correct_in_with("PRUEVA", lang, &check_caps), "{lang:?}");
            assert!(crate::is_correct_in_with("PRUEBA", lang, &check_caps), "{lang:?}");
            assert!(crate::is_correct_in_with("prueva2", lang, &defaults), "{lang:?}");
            assert!(!crate::is_correct_in_with("prueva2", lang, &check_digits), "{lang:?}");
            assert!(crate::is_correct_in_with("2026", lang, &check_digits), "{lang:?}: numbers alone stay correct");
        }
        assert!(is_correct("PRUEVA", ProofLang::Es), "direct checks keep the defaults");
    }

    /// Maintainer review on #3562: a word typed decomposed (NFD: `e` + U+0301) is checked like its
    /// composed form (NFC) and keeps its own byte range, accents and all.
    #[test]
    fn decomposed_accents_are_checked_like_composed_ones() {
        test_dict::install();
        for lang in VARIETIES {
            for text in ["esto también", "esto tambie\u{301}n", "una cancio\u{301}n y pingu\u{308}ino", "CANCIO\u{301}N", "nin\u{303}o"] {
                let issues = crate::check_text(text, &[(0..text.len(), lang)]);
                assert!(issues.is_empty(), "{lang:?} {text:?}: {issues:?}");
            }
            // A misspelling that ends in a decomposed accent is flagged whole, at its own bytes.
            let text = "esto es una prueva tambie\u{301}nn";
            let issues = crate::check_text(text, &[(0..text.len(), lang)]);
            let flagged: Vec<(&str, usize, usize)> = issues.iter().map(|i| (&text[i.start..i.end], i.start, i.end)).collect();
            assert_eq!(flagged, [("prueva", 12, 18), ("tambie\u{301}nn", 19, text.len())], "{lang:?}");
            assert!(crate::suggest_in("tambie\u{301}nn", 8, lang).contains(&"también".into()), "{lang:?}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected_in_every_case() {
        test_dict::install();
        for w in ["zqprohibida", "Zqprohibida", "ZQPROHIBIDA"] {
            assert!(!is_correct(w, ProofLang::Es), "{w}");
        }
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, "prueva", ProofLang::Es));
        assert!(install_dictionary(ProofLang::Es, "FLAG long\n", "1\ncasa\n").is_err());
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for w in ["", "a", "2026", "EE", "1.º", "日本語", "-", "--"] {
            assert!(is_correct_with(&d, w, ProofLang::EsUy), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zqpalabrapropia", ProofLang::EsMx));
        crate::add_word("Zqpalabrapropia");
        assert!(is_correct("zqpalabrapropia", ProofLang::EsMx));
    }

    #[test]
    fn spanish_dispatch_messages_options_and_mixed_spans() {
        test_dict::install();
        for tag in ["es-ES", "es-MX", "es-419", "es-AR", "es-UY", "spa"] {
            assert!(crate::has_dictionary(Some(tag)), "{tag}");
        }
        assert!(crate::is_correct_for("canción", Some("es-MX")));
        assert!(crate::is_correct_for("tenés", Some("es-UY")) && !crate::is_correct_for("tenés", Some("es-MX")));
        // The sentence from #3553.
        let text = "esto es una prueva del corector";
        for lang in VARIETIES {
            let issues = crate::check_text(text, &[(0..text.len(), lang)]);
            let flagged: Vec<&str> = issues.iter().map(|i| &text[i.start..i.end]).collect();
            assert_eq!(flagged, ["prueva", "corector"], "{lang:?}: {issues:?}");
            assert!(issues.iter().all(|i| i.message == "Posible error ortográfico" && i.lang == lang), "{issues:?}");
            assert!(crate::suggest_in("prueva", 8, lang).contains(&"prueba".into()), "{lang:?}");
            assert!(crate::suggest_in("corector", 8, lang).contains(&"corrector".into()), "{lang:?}");
        }
        let text = "Canción this is is wrong";
        let issues = crate::check_text(text, &[(0..8, ProofLang::Es419), (9..text.len(), ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 9));
        let opts = crate::Options { ignore_uppercase: false, ..Default::default() };
        assert!(!crate::is_correct_in_with("ZQPROHIBIDA", ProofLang::Es, &opts));
    }
}
