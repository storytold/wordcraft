//! Portuguese proofing.
//!
//! - **Spelling** against the Hunspell `pt_BR` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `pt_BR.aff`/`pt_BR.dic` — VERO and its
//!   derivatives — from the OS, a package manager or LibreOffice; it isn't bundled). Without it
//!   Portuguese text gets no spelling marks. Capitalized stems are names (`Brasília`); a sentence
//!   may start with any word and headings may be in capitals. Hyphenated compounds are checked
//!   part by part (`guarda-roupa`, `ex-marido`), and the user dictionary applies as in English.
//! - **Two spellings** ([`Variant`]): European Portuguese (`pt-PT`, [`crate::ProofLang::PtPt`])
//!   is checked against the Hunspell `pt_PT` dictionary (`pt_PT.aff`/`pt_PT.dic`, found the same
//!   way; LibreOffice ships it as `dict-pt-PT`), so `facto`, `equipa` and `económico` are words
//!   there and `econômico` isn't. Without a `pt_PT` dictionary European text falls back to the
//!   Brazilian one, as before the two were told apart.
//! - **Suggestions** tuned for Portuguese ([`suggest`]): missing accents first (`coraçao` →
//!   `coração`, `saude` → `saúde`), then the dictionary's `REP` confusions (`seje` → `seja`), a
//!   missing space, one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Portuguese messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// Which of the two Portuguese spellings a run of text follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Brazilian Portuguese, and Portuguese with no region ([`crate::ProofLang::Pt`]): the
    /// Hunspell `pt_BR` dictionary.
    Br,
    /// European Portuguese ([`crate::ProofLang::PtPt`]): the Hunspell `pt_PT` dictionary.
    Pt,
}

impl Variant {
    /// File names of this spelling's Hunspell dictionary; the first is the name it is known by
    /// (the status-bar hint, load errors).
    fn names(self) -> &'static [&'static str] {
        match self {
            Variant::Br => &["pt_BR", "pt-BR", "pt"],
            Variant::Pt => &["pt_PT", "pt-PT"],
        }
    }

    /// The Hunspell name this spelling's dictionary is known by (`pt_BR`, `pt_PT`).
    fn dictionary_name(self) -> &'static str {
        self.names().first().copied().unwrap_or("pt_BR")
    }

    /// The loaded dictionary: `None` until first use, then what was found.
    fn slot(self) -> &'static RwLock<Option<Loaded>> {
        static BR: RwLock<Option<Loaded>> = RwLock::new(None);
        static PT: RwLock<Option<Loaded>> = RwLock::new(None);
        match self {
            Variant::Br => &BR,
            Variant::Pt => &PT,
        }
    }
}

/// What a spelling found when it first looked for its dictionary.
#[derive(Clone)]
enum Loaded {
    /// Its own: the installed one, one given with [`install_dictionary_for`], or an empty one.
    Own(Arc<AffixDict>),
    /// European Portuguese without a `pt_PT` dictionary: the Brazilian one, looked up each time,
    /// so a Brazilian dictionary installed later is followed.
    Brazilian,
}

impl Loaded {
    fn dictionary(self) -> Arc<AffixDict> {
        match self {
            Loaded::Own(d) => d,
            Loaded::Brazilian => dictionary_for(Variant::Br),
        }
    }
}

/// The Portuguese dictionary: the installed Hunspell `pt_BR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Portuguese spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    dictionary_for(Variant::Br)
}

/// [`dictionary`] for one spelling. European Portuguese without a `pt_PT` dictionary uses the
/// Brazilian one.
pub fn dictionary_for(variant: Variant) -> Arc<AffixDict> {
    let cached = variant.slot().read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let loaded = cached.unwrap_or_else(|| {
        let found = load_installed(variant);
        let mut slot = variant.slot().write().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Another thread (or an install) may have filled it meanwhile: that one wins.
        slot.get_or_insert(found).clone()
    });
    loaded.dictionary()
}

fn load_installed(variant: Variant) -> Loaded {
    let key = variant.dictionary_name();
    let Some((aff, dic)) = crate::hunspell::find(variant.names()) else {
        crate::hunspell::clear_load_error(key);
        return not_installed(variant);
    };
    Loaded::Own(Arc::new(crate::hunspell::note_load(key, &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Portuguese spelling ({key}): {e}");
        AffixDict::default()
    })))
}

/// What a spelling uses when none of its dictionary's files is installed: nothing for Brazilian
/// Portuguese (no spelling marks), the Brazilian dictionary for European Portuguese.
fn not_installed(variant: Variant) -> Loaded {
    match variant {
        Variant::Br => {
            log::info!("Portuguese spelling: no Hunspell pt_BR dictionary installed");
            Loaded::Own(Arc::new(AffixDict::default()))
        }
        Variant::Pt => {
            log::info!("European Portuguese spelling: no Hunspell pt_PT dictionary installed; using pt_BR");
            Loaded::Brazilian
        }
    }
}

/// Use this Portuguese dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    install_dictionary_for(Variant::Br, aff, dic)
}

/// [`install_dictionary`] for one spelling. A Brazilian one is also what European Portuguese
/// uses while it has no `pt_PT` of its own.
pub fn install_dictionary_for(variant: Variant, aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *variant.slot().write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Loaded::Own(Arc::new(d)));
    Ok(())
}

/// Forget the loaded dictionaries, so the next check looks for installed ones again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    for variant in [Variant::Br, Variant::Pt] {
        *variant.slot().write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// The `.dic` file of the installed Portuguese dictionary, searching the dictionaries folder and
/// the system locations now (File › Options › Proofing shows it, #1063); `None` when there is
/// none. A dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    installed_path_for(Variant::Br)
}

/// [`installed_path`] for one spelling: for European Portuguese its `pt_PT` file, else the
/// Brazilian one it falls back to.
pub fn installed_path_for(variant: Variant) -> Option<std::path::PathBuf> {
    match crate::hunspell::find(variant.names()) {
        Some((_, dic)) => Some(dic),
        None if variant == Variant::Pt => installed_path_for(Variant::Br),
        None => None,
    }
}

/// The Hunspell name whose load error applies to `variant` now: its own, or `pt_BR` while
/// European Portuguese falls back to the Brazilian dictionary, so an unreadable `pt_BR` is
/// reported as such rather than as a missing `pt_PT`. Reads nothing itself.
pub fn load_error_key(variant: Variant) -> &'static str {
    load_error_key_of(variant, variant.slot().read().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref())
}

fn load_error_key_of(variant: Variant, loaded: Option<&Loaded>) -> &'static str {
    match loaded {
        Some(Loaded::Brazilian) => Variant::Br.dictionary_name(),
        _ => variant.dictionary_name(),
    }
}

/// Is a Portuguese dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// [`has_dictionary`] for one spelling.
pub fn has_dictionary_for(variant: Variant) -> bool {
    !dictionary_for(variant).is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Portuguese dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Portuguese? Numbers, single letters, short words in capitals
/// (acronyms: `USP`, `PT`), words with digits and words in other scripts are accepted, and so
/// is every word when no Portuguese dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] in one spelling (`facto` is a word in European Portuguese only).
pub fn is_correct_for(variant: Variant, word: &str) -> bool {
    is_correct_with(&dictionary_for(variant), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    is_correct_with_options_for(Variant::Br, word, opts)
}

/// [`is_correct_with_options`] in one spelling.
pub fn is_correct_with_options_for(variant: Variant, word: &str, opts: &crate::Options) -> bool {
    check(&dictionary_for(variant), word, opts)
}

/// Spelling suggestions for a word misspelled in one spelling ([`suggest`] is the Brazilian one).
pub fn suggest_for(variant: Variant, word: &str, max: usize) -> Vec<String> {
    suggest::suggest_in(&dictionary_for(variant), word, max)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true; // no Portuguese dictionary installed: no spelling marks
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
        return w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
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
    fn test_dictionary_loads() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.len() > 50, "{}", d.len());
        assert!(d.rule_count() >= 5, "the test dictionary's affix rules");
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn the_two_spellings_have_their_own_dictionaries() {
        test_dict::install();
        test_dict::install_european();
        assert!(has_dictionary_for(Variant::Br) && has_dictionary_for(Variant::Pt));
        for w in ["facto", "equipa", "autocarro", "económico", "Económico", "FACTO"] {
            assert!(is_correct_for(Variant::Pt, w), "{w}");
        }
        for w in ["facto", "equipa", "autocarro", "económico"] {
            assert!(!is_correct(w) && !is_correct_for(Variant::Br, w), "{w}");
        }
        for w in ["equipe", "ônibus", "econômico"] {
            assert!(is_correct(w) && !is_correct_for(Variant::Pt, w), "{w}");
        }
        // Words both write the same way are in both.
        assert!(is_correct("coração") && is_correct_for(Variant::Pt, "coração"));
        assert_eq!(suggest_for(Variant::Pt, "economico", 3).first().map(String::as_str), Some("económico"));
        assert_eq!(suggest_for(Variant::Br, "economico", 3).first().map(String::as_str), Some("econômico"));
        assert_eq!((Variant::Br.dictionary_name(), Variant::Pt.dictionary_name()), ("pt_BR", "pt_PT"));
        assert!(!Variant::Pt.names().contains(&"pt"), "a bare `pt` dictionary is the Brazilian fallback");
    }

    /// #1028 review: European Portuguese without a `pt_PT` reads the Brazilian dictionary each
    /// time instead of keeping a copy of it, so a Brazilian one installed later is followed.
    #[test]
    fn the_european_fallback_reads_the_current_brazilian_dictionary() {
        test_dict::install();
        assert!(matches!(not_installed(Variant::Pt), Loaded::Brazilian));
        assert!(matches!(not_installed(Variant::Br), Loaded::Own(d) if d.is_empty()));
        let fallback = Loaded::Brazilian.dictionary();
        assert!(fallback.check("equipe") && !fallback.check("facto"), "the Brazilian words");
    }

    /// #1028 review: while European Portuguese falls back to the Brazilian dictionary, that one's
    /// load error is the one to report; otherwise each spelling reports its own.
    #[test]
    fn load_errors_follow_the_dictionary_in_use() {
        let own = Loaded::Own(Arc::new(AffixDict::default()));
        assert_eq!(load_error_key_of(Variant::Pt, Some(&Loaded::Brazilian)), "pt_BR");
        assert_eq!(load_error_key_of(Variant::Pt, Some(&own)), "pt_PT");
        assert_eq!(load_error_key_of(Variant::Pt, None), "pt_PT", "not looked for yet");
        assert_eq!(load_error_key_of(Variant::Br, Some(&own)), "pt_BR");
    }

    /// Each spelling finds its own file; European Portuguese reports the Brazilian file it falls
    /// back to when there is no `pt_PT`.
    #[test]
    fn each_spelling_looks_for_its_own_dictionary_first() {
        let dir = std::env::temp_dir().join(format!("wordcraft-pt-variants-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, br, pt) in [("pt_BR", true, false), ("pt-BR", true, false), ("pt", true, false), ("pt_PT", false, true), ("pt-PT", false, true)] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            assert_eq!(crate::hunspell::find_in(&dir, Variant::Br.names()).is_some(), br, "{name}");
            assert_eq!(crate::hunspell::find_in(&dir, Variant::Pt.names()).is_some(), pt, "{name}");
            std::fs::remove_file(dir.join(format!("{name}.aff"))).unwrap();
            std::fs::remove_file(dir.join(format!("{name}.dic"))).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["coraçao", "saude", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(install_dictionary("FLAG long\n", "1\nkot\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn portuguese_words_are_correct() {
        test_dict::install();
        for w in [
            "coração",
            "saudade",
            "pássaro",
            "saúde",
            "árvore",
            "escrever",
            "computador",
            "praia",
            "conhecer",
            "Brasília",
            "São",
            "autoestima",
            "frequência",
            "exceção",
            "amanhã",
            "vocês",
            "coragem",
            "parabéns",
            "cantava",
            "cantando",
            "provável",
            "prováveis",
            "descobrir",
            "trabalho",
            "CORAGEM",
            "USP",
            "2026",
            "e-mail",
            "guarda-roupa",
            "ex-marido",
            "vice-presidente",
            "e",
            "não",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["coraçao", "saude", "trbalho", "excrever", "prsidente", "frequencia", "amnha", "cantaando"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected() {
        test_dict::install();
        // `frequencia/F` is listed with FORBIDDENWORD: the old spelling, never correct.
        assert!(!is_correct("frequencia") && !is_correct("Frequencia") && !is_correct("FREQUENCIA"));
        assert!(is_correct("frequência"));
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zxqportuguespalavra"));
        crate::add_word("Zxqportuguespalavra");
        assert!(is_correct("zxqportuguespalavra"));
    }

    #[test]
    fn user_dictionary_accepts_a_hyphenated_word_whole() {
        test_dict::install();
        let text = "Ele disse que está na Unigranrio-RJ.";
        assert_eq!(crate::check_spelling_in(text, crate::ProofLang::Pt).len(), 1, "Unigranrio isn't in the dictionary");
        // Added as the context menu adds it, hyphen and all (#2082).
        crate::add_word("Unigranrio-RJ");
        assert!(crate::check_spelling_in(text, crate::ProofLang::Pt).is_empty());
    }

    /// European Portuguese takes the File › Options › Proofing choices as Brazilian does.
    #[test]
    fn proofing_options_reach_the_european_portuguese_check() {
        test_dict::install();
        test_dict::install_european();
        let lang = crate::ProofLang::PtPt;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        assert!(crate::is_correct_in_with("FACTU", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("FACTU", lang, &check_caps));
        assert!(crate::is_correct_in_with("FACTO", lang, &check_caps));
        assert!(!crate::is_correct_in_with("FACTO", crate::ProofLang::Pt, &check_caps), "not a Brazilian word");
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Portuguese checks
    /// those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_portuguese_check() {
        test_dict::install();
        let lang = crate::ProofLang::Pt;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("PRAYA", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("PRAYA", lang, &check_caps));
        assert!(crate::is_correct_in_with("PRAIA", lang, &check_caps));
        assert!(crate::is_correct_in_with("praya2", lang, &defaults));
        assert!(!crate::is_correct_in_with("praya2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("PRAYA"), "direct checks keep the defaults");
    }
}
