//! French proofing.
//!
//! - **Spelling** against the Hunspell French dictionary installed on the system (found by
//!   [`crate::hunspell`]: `fr_FR.aff`/`fr_FR.dic`, else `fr` or a regional one, from the OS, a
//!   package manager or LibreOffice; it isn't bundled — the upstream files are MPL 2.0).
//!   Without it French text gets no spelling marks. The dictionary itself knows elisions
//!   (`l'arbre`, `qu'elle`, `jusqu'à`) and both apostrophes (`’` is read as `'`); hyphenated
//!   compounds it doesn't list are checked part by part (`va-t-il`); the user dictionary applies
//!   as in English, after an elision too.
//! - **Suggestions** tuned for French ([`suggest`]): missing accents first (`ecole` → `école`),
//!   then the dictionary's `REP` confusions (`soeur` → `sœur`), a missing space, one-letter
//!   typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): French typography and a few reliable rules, with French messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::{suggest, suggest_in};

/// The French dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a French Hunspell dictionary: France first, then the generic and regional ones.
const NAMES: &[&str] = &["fr_FR", "fr", "fr-FR", "fr_BE", "fr_CA", "fr_CH", "fr_LU", "fr_MC"];

/// The French dictionary: the installed Hunspell `fr_FR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (French spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    cached_dictionary(&DICT, load_installed)
}

/// Initialize an empty cache while holding its write lock, so a concurrent reload clears the
/// completed load instead of allowing an old-folder result to be published after the reload.
fn cached_dictionary(cache: &RwLock<Option<Arc<AffixDict>>>, load: impl FnOnce() -> AffixDict) -> Arc<AffixDict> {
    if let Some(d) = cache.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return Arc::clone(d);
    }
    let mut slot = cache.write().unwrap_or_else(|e| e.into_inner());
    Arc::clone(slot.get_or_insert_with(|| Arc::new(load())))
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("French spelling: no Hunspell fr_FR dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("French spelling: {e}");
        AffixDict::default()
    })
}

/// Use this French dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8),
/// for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    crate::hunspell::clear_load_error(NAMES[0]);
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The `.dic` file of the Hunspell French dictionary found in the dictionaries folder or the
/// system locations now (File › Options › Proofing shows it, #1063); `None` when there is none.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a French dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the French dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Words that elide before a vowel or a mute `h` (`l'`, `qu'`, `jusqu'`…), lower case, without
/// the apostrophe. After one, a word of the user dictionary is accepted (`l'Wordcraft`).
const ELISIONS: &[&str] = &["l", "d", "j", "m", "t", "s", "n", "c", "qu", "jusqu", "lorsqu", "puisqu", "quoiqu", "presqu", "quelqu"];

/// Is `word` spelled correctly in French? Numbers, single letters, short words in capitals
/// (acronyms: `SNCF`, `ONU`), words with digits and words in other scripts are accepted, and so
/// is every word when no French dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices. Disabling either ignore
/// option checks the corresponding words against the French dictionary.
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
        return true; // no French dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && !crate::is_combining_mark(c) && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']).replace('’', "'");
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
    if known(d, &w) {
        return true;
    }
    if let Some((head, tail)) = w.split_once('\'')
        && ELISIONS.contains(&head.to_lowercase().as_str())
        && !tail.is_empty()
        && !tail.contains('\'')
        && crate::in_user_dictionary(&tail.to_lowercase())
    {
        return true;
    }
    // Compounds the dictionary doesn't list (`va-t-il`, `maison-école`): every part.
    w.contains('-') && w.split('-').all(|p| !p.is_empty() && (p.chars().count() <= 1 || known(d, p)))
}

/// Is `w` in the dictionary (any elision included) or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reload concurrent with a suspended load must clear its result. A local cache avoids
    /// interfering with the installed dictionaries used by other tests, and channels force the
    /// interleaving without sleeps. Completion waits are bounded even if a regression deadlocks.
    #[test]
    fn dictionary_load_is_serialized_with_reload() {
        use std::sync::mpsc;
        use std::time::Duration;

        let timeout = Duration::from_secs(10);
        let cache = Arc::new(RwLock::new(None));
        let old = AffixDict::from_hunspell("SET UTF-8\n", "1\nancien\n", &[]).unwrap();
        let new = AffixDict::from_hunspell("SET UTF-8\n", "1\nnouveau\n", &[]).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (loaded_tx, loaded_rx) = mpsc::channel();
        let loading_cache = Arc::clone(&cache);
        let loading = std::thread::spawn(move || {
            let loaded = cached_dictionary(&loading_cache, || {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(timeout).unwrap();
                old
            });
            loaded_tx.send(loaded).unwrap();
        });
        started_rx.recv_timeout(timeout).unwrap();
        let loading_holds_write_lock = cache.try_write().is_err();
        let (reload_started_tx, reload_started_rx) = mpsc::channel();
        let (reloaded_tx, reloaded_rx) = mpsc::channel();
        let reloading_cache = Arc::clone(&cache);
        let reloading = std::thread::spawn(move || {
            reload_started_tx.send(()).unwrap();
            // This is reload_dictionary's cache-clear operation, on the isolated cache.
            *reloading_cache.write().unwrap() = None;
            reloaded_tx.send(()).unwrap();
        });
        reload_started_rx.recv_timeout(timeout).unwrap();
        // If loading regresses to outside the lock, force the stale-publication order rather
        // than relying on scheduling luck: reload finishes before the old load can publish.
        if !loading_holds_write_lock {
            reloaded_rx.recv_timeout(timeout).unwrap();
        }
        release_tx.send(()).unwrap();
        let old = loaded_rx.recv_timeout(timeout).unwrap();
        if loading_holds_write_lock {
            reloaded_rx.recv_timeout(timeout).unwrap();
        }
        loading.join().unwrap();
        reloading.join().unwrap();
        assert!(loading_holds_write_lock, "reload must wait for the in-progress load");
        assert!(old.check("ancien"));
        assert!(cache.read().unwrap().is_none(), "reload clears the completed old-folder load");
        let current = cached_dictionary(&cache, || new);
        assert!(current.check("nouveau") && !current.check("ancien"), "the next check loads the replacement dictionary");
        assert!(!Arc::ptr_eq(&old, &current));
    }

    #[test]
    fn test_dictionary_loads() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.len() > 100, "{}", d.len());
        assert!(d.rule_count() >= 20, "the test dictionary's affix rules");
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn regional_dictionary_files_are_found() {
        // Without fr_FR, a Belgian, Canadian or Swiss dictionary is used.
        let dir = std::env::temp_dir().join(format!("wc-fr-ca-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("fr_CA.aff"), test_dict::AFF).expect("aff");
        std::fs::write(dir.join("fr_CA.dic"), test_dict::DIC).expect("dic");
        let found = crate::hunspell::find_in(&dir, NAMES);
        let ok = found.as_ref().is_some_and(|(a, d)| a.ends_with("fr_CA.aff") && d.ends_with("fr_CA.dic"));
        assert!(ok, "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["école", "l'arbre", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(install_dictionary("COMPOUNDFLAG X\n", "1\nchat\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn french_words_are_correct() {
        test_dict::install();
        for w in [
            "maison",
            "maisons",
            "école",
            "écoles",
            "grand",
            "grande",
            "grands",
            "grandes",
            "aimer",
            "aime",
            "aimons",
            "aiment",
            "suis",
            "est",
            "sommes",
            "sont",
            "a",
            "avons",
            "ont",
            "Paris",
            "Montréal",
            "œuvre",
            "sœur",
            "aujourd'hui",
            "aujourd’hui",
            "peut-être",
            "arc-en-ciel",
            "km",
            "Maison",
            "MAISON",
            "SNCF",
            "2026",
            "très",
            "rivière",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn elisions_follow_the_dictionary() {
        test_dict::install();
        for w in ["l'arbre", "l’arbre", "L'arbre", "d'enfants", "l'homme", "qu'elle", "qu’il", "j'aime", "j’ai", "d'avoir", "l'école", "jusqu'à"]
        {
            assert!(is_correct(w), "{w}");
        }
        // The dictionary decides: no elision before an aspirated h or a consonant, none on a plural.
        for w in ["l'hasard", "l'chat", "l'arbres", "l'arbrre", "qu'maison"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    /// File › Options must reach both French regions and the actual spelling issues.
    #[test]
    fn proofing_options_reach_the_french_check() {
        test_dict::install();
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        let strict = crate::Options { ignore_uppercase: false, ignore_numbers: false, ..defaults };
        for lang in [crate::ProofLang::Fr, crate::ProofLang::FrCa] {
            assert!(crate::is_correct_in_with("SNCF", lang, &defaults));
            assert!(!crate::is_correct_in_with("SNCF", lang, &check_caps));
            assert!(crate::is_correct_in_with("MAISON", lang, &check_caps));
            assert!(crate::is_correct_in_with("mp3x", lang, &defaults));
            assert!(!crate::is_correct_in_with("mp3x", lang, &check_digits));
            assert!(crate::is_correct_in_with("2026", lang, &strict), "numbers alone stay correct");
            assert!(crate::is_correct_in_with("SNCF", lang, &check_digits), "the uppercase option is independent");
            assert!(crate::is_correct_in_with("mp3x", lang, &check_caps), "the numbers option is independent");
            let text = "école SNCF mp3x";
            let spans = [(0..text.len(), lang)];
            assert!(crate::check_text_with(text, &spans, &defaults, false, false).is_empty());
            let issues = crate::check_text_with(text, &spans, &strict, false, false);
            let flagged: Vec<_> = issues.iter().map(|i| (&text[i.start..i.end], i.lang)).collect();
            assert_eq!(flagged, [("SNCF", lang), ("mp3x", lang)]);
        }
        assert!(!is_correct_with_options("SNCF", &check_caps));
        assert!(!is_correct_with_options("mp3x", &check_digits));
        assert!(is_correct("SNCF") && is_correct("mp3x"), "direct checks keep the defaults");
        assert!(check(&AffixDict::default(), "SNCF", &strict), "missing tools produce no spelling marks");
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["maisonn", "ecole", "arbrre", "beaucoups", "aimmer", "soeur", "Km", "grandd", "aim"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn compounds_are_checked_part_by_part() {
        test_dict::install();
        assert!(is_correct("maison-école") && is_correct("est-il") && is_correct("a-t-il"));
        assert!(!is_correct("maison-ecole"));
    }

    #[test]
    fn user_dictionary_applies_after_an_elision_too() {
        test_dict::install();
        assert!(!is_correct("zxqmotfrancais") && !is_correct("l'zxqmotfrancais"));
        crate::add_word("Zxqmotfrancais");
        assert!(is_correct("zxqmotfrancais") && is_correct("l'zxqmotfrancais") && is_correct("d’zxqmotfrancais"));
    }

    /// The French dictionary installed on this machine, when there is one (Dicollecte `fr_FR`, as
    /// Debian's `hunspell-fr` installs it); the test passes without checking anything otherwise.
    fn system_dictionary() -> Option<AffixDict> {
        let dir = std::path::Path::new("/usr/share/hunspell");
        let (aff, dic) = crate::hunspell::find_in(dir, &["fr_FR"])?;
        Some(crate::hunspell::load(&aff, &dic).expect("the installed fr_FR dictionary loads"))
    }

    #[test]
    fn installed_french_dictionary_checks_real_text() {
        let Some(d) = system_dictionary() else { return };
        assert!(d.len() > 50_000, "{}", d.len());
        for w in [
            "maison",
            "maisons",
            "Paris",
            "mangeons",
            "finissaient",
            "étaient",
            "l'arbre",
            "l’arbre",
            "d'enfants",
            "qu'elles",
            "j'aime",
            "s'il",
            "aujourd'hui",
            "suis",
            "est",
            "sommes",
            "allons",
            "vais",
            "irons",
            "belle",
            "beaux",
            "heureuse",
            "Allemagne",
            "l'Allemagne",
            "chevaux",
            "œuvre",
            "écrivit",
            "c'est",
            "n'est",
            "jusqu'à",
            "lorsqu'il",
            "Maison",
            "MAISON",
            "m'appelle",
            "t'aime",
            "l'homme",
            "d'une",
            "qu'on",
            "prends",
            "voudrions",
            "peut-être",
            "va-t-il",
        ] {
            assert!(is_correct_with(&d, w), "{w}");
        }
        for w in [
            "mangons",
            "enfans",
            "arbrre",
            "beaucoups",
            "l'arbrre",
            "écrivut",
            "maisonn",
            "chevals",
            "heureusse",
            "étaitent",
            "jaime",
            "sommmes",
            "Parris",
            "finisaient",
        ] {
            assert!(!is_correct_with(&d, w), "{w}");
        }
        let first = |w: &str| suggest::suggest_in(&d, w, 5);
        assert!(first("maisonn").contains(&"maison".to_string()), "{:?}", first("maisonn"));
        assert!(first("ecole").contains(&"école".to_string()), "{:?}", first("ecole"));
    }

    #[test]
    fn suggestions_on_the_installed_dictionary_are_fast_enough() {
        let Some(d) = system_dictionary() else { return };
        let words = ["maisonn", "ecole", "beaucoups", "l'arbrre", "anticonstitutionelement", "xqzvwemotlong", "finisaient", "Parris"];
        let _ = suggest::suggest_in(&d, "bien", 1);
        let t = std::time::Instant::now();
        for w in words {
            let _ = suggest::suggest_in(&d, w, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("fr_FR: {} suggestion calls in {ms:.0} ms ({:.0} ms each)", words.len(), ms / words.len() as f64);
        assert!(ms < 30_000.0, "{ms} ms");
        // NOSUGGEST entries of the real dictionary (unit symbols) are correct but stay out.
        for (typo, unit) in [("dastt", "dast"), ("cgrr", "cgr")] {
            assert!(is_correct_with(&d, unit), "{unit}");
            let s = suggest::suggest_in(&d, typo, 8);
            assert!(!s.iter().any(|x| x == unit), "{typo} → {s:?}");
        }
        // `bar` is also an ordinary noun (only its unit entry is NOSUGGEST): still suggested.
        let s = suggest::suggest_in(&d, "barr", 8);
        assert!(s.iter().any(|x| x == "bar"), "barr → {s:?}");
    }
}
