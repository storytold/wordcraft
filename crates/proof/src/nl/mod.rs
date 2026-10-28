//! Dutch proofing using the installed Hunspell `nl_NL` dictionary (never bundled; OpenTaal's,
//! shipped by Debian and Ubuntu as `hunspell-nl` and by LibreOffice as its `dict-nl` extension).
//! The one dictionary covers the Netherlands and Belgium, so every `nl` region uses it. It needs
//! the affix engine's two-character flags, `ICONV`/`OCONV` (it stores `ij` as the letter `ĳ`) and
//! compounding: Dutch writes compounds as one word (`zwembadbouwer`), and the dictionary builds
//! them from parts instead of listing every one. Suggestions use the dictionary's own
//! MAP/REP/TRY/KEY tables. Grammar is limited to generic repeated-word and spacing checks with
//! Dutch messages; hyphenation keeps the English fallback (no Dutch patterns are bundled).

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Dutch dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a Dutch Hunspell dictionary. OpenTaal's covers Belgium too, so a Belgian name is
/// a last resort only.
const NAMES: &[&str] = &["nl_NL", "nl-NL", "nl", "nl_BE", "nl-BE"];

/// The Dutch dictionary: the installed Hunspell `nl_NL` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Dutch spelling is off then;
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
        log::info!("Dutch spelling: no Hunspell nl_NL dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Dutch spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Dutch dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8), for
/// example one the user picked. On error the current dictionary stays.
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

/// The `.dic` file of the Hunspell `nl_NL` dictionary found in the dictionaries folder or the
/// system locations now (File › Options › Proofing shows it); `None` when there is none. A
/// dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a Dutch dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Dutch dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Dutch? Numbers, single letters, short words in capitals
/// (acronyms: `BTW`, `KvK`… in capitals), words with digits and words in other scripts are
/// accepted, and so is every word when no Dutch dictionary is installed. Compounds the
/// dictionary can build are correct (`zwembadbouwer`). A word the text splits off after an
/// apostrophe (`'s-Hertogenbosch` arrives as `s-Hertogenbosch`) is looked up with it again. A
/// hyphenated word is correct whole (`e-mail`, `Noord-Holland`) or when each part is
/// (`zwembad-renovatie`).
pub fn is_correct(word: &str) -> bool {
    is_correct_with_options(word, &crate::Options::default())
}

/// [`is_correct`] with the File > Options > Proofing choices.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true; // no Dutch dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’' && c != '-');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1
        || (opts.ignore_numbers && w.chars().any(|c| c.is_ascii_digit()))
        || !w.chars().any(char::is_alphabetic)
        || (opts.ignore_uppercase && w.chars().all(|c| !c.is_lowercase()) && n <= 6)
        || !w.chars().any(is_latin_letter)
    {
        return known(d, w) || exemption_permitted(d, w);
    }
    if known(d, w) || known(d, &format!("'{w}")) {
        return true;
    }
    let mut remaining = 128;
    check_parts(d, w, &mut remaining, opts)
}

/// Default breaks can retain a listed hyphenated component. Bound failed splits too, and
/// check a boundary-trimmed core before subdividing so forbidden spellings cannot escape.
fn check_parts(d: &AffixDict, word: &str, remaining: &mut usize, opts: &crate::Options) -> bool {
    if *remaining == 0 {
        return false;
    }
    *remaining -= 1;
    let n = word.chars().count();
    if ((n <= 1
        || (opts.ignore_uppercase && word.chars().all(|c| !c.is_lowercase()) && n <= 6)
        || (opts.ignore_numbers && word.chars().any(|c| c.is_ascii_digit())))
        && exemption_permitted(d, word))
        || known(d, word)
    {
        return true;
    }
    if !d.permits_hyphen_breaks() || !word.contains('-') || d.forbids(word) {
        return false;
    }
    let core = word.trim_matches('-');
    if core != word {
        return check_parts(d, core, remaining, opts);
    }
    for (i, _) in word.match_indices('-') {
        if *remaining == 0 {
            return false;
        }
        let (Some(left), Some(right)) = (word.get(..i), word.get(i + 1..)) else { continue };
        if check_parts(d, left, remaining, opts) && check_parts(d, right, remaining, opts) {
            return true;
        }
    }
    false
}

/// Short words and acronyms must not exempt a forbidden core between boundary hyphens.
fn exemption_permitted(d: &AffixDict, word: &str) -> bool {
    let core = word.trim_matches('-');
    !d.rejects_explicit_word(word) && (core == word || (!d.forbids(core) && (d.permits_hyphen_breaks() || !d.rejects_explicit_word(core))))
}

/// Is one word in the dictionary (as written, capitalized or in capitals, or as a compound) or
/// the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proofing_options_reach_the_dutch_check() {
        test_dict::install();
        let lang = crate::ProofLang::Nl;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        for word in ["QZX", "zwembad-QZX"] {
            assert!(crate::is_correct_in_with(word, lang, &defaults));
            assert!(!crate::is_correct_in_with(word, lang, &check_caps), "{word}");
        }
        for word in ["zwembad2", "zwembad-zwembad2"] {
            assert!(crate::is_correct_in_with(word, lang, &defaults));
            assert!(!crate::is_correct_in_with(word, lang, &check_digits), "{word}");
        }
        assert!(crate::is_correct_in_with("ZWEMBAD", lang, &check_caps));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits));
        assert!(is_correct("QZX"), "direct checks retain defaults");
    }

    #[test]
    fn mixed_case_acronyms_respect_explicit_keepcase_stems_and_affixes() {
        for breaks in ["", "BREAK 0\n"] {
            let aff = format!("SET UTF-8\n{breaks}KEEPCASE K\nFORBIDDENWORD F\nNOSUGGEST N\nSFX S N 1\nSFX S 0 s .\n");
            let d = AffixDict::from_hunspell(&aff, "3\niPod/KS\nLaSer/FS\nRaDar/NS\n", &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                for word in ["iPod", "iPods", "RADAR", "RADARS", "QZX"] {
                    assert!(is_correct_with(d, word), "{word}");
                }
                for word in ["IPOD", "IPODS", "LASER", "LASERS", "-IPOD-", "-IPODS-"] {
                    assert!(!is_correct_with(d, word), "{word}");
                }
            }
            let forbidden = AffixDict::from_hunspell(&aff, "1\niPod/FKS\n", &[]).unwrap();
            for word in ["IPOD", "IPODS", "-IPOD-", "-IPODS-"] {
                assert!(!is_correct_with(&forbidden, word), "{word}");
            }
        }
    }

    #[test]
    fn dictionary_lookup_accepts_dutch_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-nl-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["nl_NL", "nl-NL", "nl", "nl_BE", "nl-BE"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "zwembad"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn dutch_words_ij_compounds_case_and_hyphens() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.rule_count() > 0);
        for w in [
            "zwembad",
            "zwembaden",
            "zwembadbouwer",
            "Zwembadbouwer",
            "ZWEMBAD",
            "tuinhuis",
            "zijn",
            "Zijn",
            "IJsland",
            "auto's",
            "auto’s",
            "Noord-Holland",
            "s-Hertogenbosch",
            "zwembad-renovatie",
            "vijfhonderd",
            "BTW",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["zwembat", "zwembadbouwr", "zjin", "ijsland", "zwemmbad", "zwembadzwembad", "tuinHuis", "zeeegel", "fietsenhok"] {
            assert!(!is_correct(w), "{w}");
        }
        assert!(is_correct("zee-egel"), "a vowel clash is written with a hyphen");
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, "zwembat"));
        assert!(install_dictionary("FLAG num\n", "1\nzwembad\n").is_err());
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for w in ["", "a", "2026", "NL", "12-jarig", "日本語", "-", "--", "'t"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
    }

    #[test]
    fn forbidden_hyphens_do_not_fall_back_to_components() {
        let d = AffixDict::from_hunspell("SET UTF-8\nFORBIDDENWORD F\n", "3\nalpha\nbeta\nalpha-beta/F\n", &[]).unwrap();
        for word in ["alpha-beta", "Alpha-beta", "-alpha-beta", "alpha-beta-", "-alpha-beta-", "-Alpha-beta-"] {
            assert!(!is_correct_with(&d, word), "{word}");
        }
        assert!(is_correct_with(&d, "beta-alpha"));
        assert!(is_correct_with(&d, "-beta-alpha-"));
    }

    #[test]
    fn hyphen_breaks_retain_listed_components() {
        for disabled in [false, true] {
            let d = AffixDict::from_hunspell(if disabled { "BREAK 0\n" } else { "" }, "2\nNoord-Holland\nrenovatie\n", &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                assert!(is_correct_with(d, "Noord-Holland"));
                for word in ["Noord-Holland-renovatie", "renovatie-Noord-Holland", "-Noord-Holland-renovatie-"] {
                    assert_eq!(is_correct_with(d, word), !disabled, "{word}");
                }
                assert!(!is_correct_with(d, "Noord-Holland-unknown"));
            }
        }
    }

    #[test]
    fn dictionary_flags_precede_acronym_exemptions() {
        let aff = "FORBIDDENWORD F\nKEEPCASE K\nSFX S N 1\nSFX S 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\nzqfo/F\ndvd/KS\na/F\n", &[]).unwrap();
        for w in ["ZQFO", "-ZQFO", "-ZQFO-", "DVD", "DVDS", "a", "A"] {
            assert!(!is_correct_with(&d, w), "{w}");
        }
        for w in ["dvd", "dvds", "QZX", "-QZX-", "2026"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
    }

    #[test]
    fn boundary_acronyms_respect_disabled_breaks() {
        for disabled in [false, true] {
            let aff = format!("SET UTF-8\nKEEPCASE K\n{}", if disabled { "BREAK 0\n" } else { "" });
            let d = AffixDict::from_hunspell(&aff, "1\ndvd/K\n", &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                assert!(is_correct_with(d, "dvd"));
                assert!(!is_correct_with(d, "DVD"));
                // Independently observed with Hunspell 1.7.2 and this original dictionary.
                assert_eq!(is_correct_with(d, "-DVD-"), !disabled);
                assert_eq!(is_correct_with(d, "-dvd-"), !disabled);
            }
        }
        let listed = AffixDict::from_hunspell("BREAK 0\nKEEPCASE K\n", "2\ndvd/K\n-DVD-\n", &[]).unwrap();
        assert!(is_correct_with(&listed, "-DVD-"), "an exact listed spelling still wins");
    }

    #[test]
    fn hyphen_breaks_follow_the_dictionary_setting() {
        let aff = "COMPOUNDMIN 2\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nCOMPOUNDPERMITFLAG P\nFORCEUCASE U\nCHECKCOMPOUNDDUP\nSFX S N 1\nSFX S 0 -/BP .\n";
        for disabled in [false, true] {
            let aff = format!("{aff}{}", if disabled { "BREAK 0\n" } else { "" });
            let d = AffixDict::from_hunspell(&aff, "3\nalpha/BES\nbeta/EU\ngamma/E\n", &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                // Independently observed with Hunspell 1.7.2 and these original fixtures.
                assert_eq!(is_correct_with(d, "alpha-beta"), !disabled);
                assert_eq!(is_correct_with(d, "alpha-alpha"), !disabled);
                assert!(is_correct_with(d, "Alpha-beta"));
                assert!(is_correct_with(d, "alpha-gamma"));
                assert!(!is_correct_with(d, "alpha-unknown"));
            }
        }
    }

    #[test]
    fn boundary_hyphens_retain_dictionary_restrictions() {
        let d = AffixDict::from_hunspell("BREAK 0\nFORBIDDENWORD F\n", "2\nalpha\nalpha-/F\n", &[]).unwrap();
        assert!(!is_correct_with(&d, "alpha-"));
        assert!(!is_correct_with(&d, "Alpha-"));
        assert!(!is_correct_with(&d, "-alpha"));
        assert!(is_correct_with(&d, "alpha"));
        let no_break = AffixDict::from_hunspell("BREAK 0\n", "1\nalpha\n", &[]).unwrap();
        assert!(!is_correct_with(&no_break, "alpha-") && !is_correct_with(&no_break, "-alpha"));
        let default = AffixDict::from_hunspell("", "1\nalpha\n", &[]).unwrap();
        assert!(is_correct_with(&default, "alpha-") && is_correct_with(&default, "-alpha"));
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zqmijnwoord"));
        crate::add_word("Zqmijnwoord");
        assert!(is_correct("zqmijnwoord"));
    }

    #[test]
    fn dutch_dispatch_messages_and_mixed_spans() {
        test_dict::install();
        assert!(crate::has_dictionary(Some("nl-BE")));
        assert!(crate::is_correct_for("zwembad", Some("nld")));
        let text = "Het zwembat";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Nl)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "zwembat");
        assert_eq!(issues[0].message, "Mogelijke spelfout");
        assert_eq!(issues[0].lang, crate::ProofLang::Nl);
        assert!(crate::suggest_in("zwembat", 8, crate::ProofLang::Nl).contains(&"zwembad".into()));
        let text = "Het this is is wrong";
        let issues = crate::check_text(text, &[(0..3, crate::ProofLang::Nl), (4..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 4));
    }
}
