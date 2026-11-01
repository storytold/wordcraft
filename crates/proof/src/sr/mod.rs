//! Serbian proofing using the installed Hunspell dictionaries (never bundled): one in Cyrillic
//! (`sr_RS`, LibreOffice's `sr`) and one in Latin script (`sr_Latn_RS`, `sh_RS`, LibreOffice's
//! `sr-Latn`). Both are `FLAG num` dictionaries with about 1,900 flags, which the affix engine
//! reads since #4967.
//!
//! Serbian is written in both scripts, and one document often mixes them, so the run's tag picks
//! the row ([`ProofLang::Sr`] for `sr`, `sr-RS`, `sr-Cyrl-*`; [`ProofLang::SrLatn`] for
//! `sr-Latn-*` and the legacy `sh`), but **each word is checked against the dictionary of its own
//! script**: a Latin word is never looked up in the Cyrillic dictionary, nor a Cyrillic word in
//! the Latin one (either would always be marked wrong). A word whose script has no dictionary
//! installed is not marked, as in any language without its dictionary.
//!
//! Suggestions are the dictionary-driven ones Czech uses ([`crate::cs::suggest::suggest_in`]),
//! from the dictionary of the word's script, so they are in the word's script too. There is no
//! Serbian grammar (no rules are hand-written here), and hyphenation keeps the fallback patterns.
//!
//! [`ProofLang::Sr`]: crate::ProofLang::Sr
//! [`ProofLang::SrLatn`]: crate::ProofLang::SrLatn

#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

/// The script of a Serbian dictionary, and of a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    /// Serbian Cyrillic (ћирилица): `sr_RS`.
    Cyrillic,
    /// Serbian Latin (latinica): `sr_Latn_RS`, `sh_RS`.
    Latin,
}

/// One script's dictionary, and whether it came from [`install_dictionary`] rather than a file
/// (it then has no file to report from [`installed_path`]): one lock, so a reload and an install
/// can't interleave their two fields.
struct DictState {
    dict: Option<Arc<AffixDict>>,
    in_memory: bool,
}

static CYRILLIC: RwLock<DictState> = RwLock::new(DictState { dict: None, in_memory: false });
static LATIN: RwLock<DictState> = RwLock::new(DictState { dict: None, in_memory: false });

/// File names of a Serbian Cyrillic Hunspell dictionary. `sr_RS` is Debian's and Fedora's name,
/// `sr` LibreOffice's; a name without a script is Cyrillic, BCP 47's default for Serbian. Names
/// whose script isn't known (Debian's `sr_ME`) are left out: guessing could cross scripts.
pub const CYRILLIC_NAMES: &[&str] = &["sr_RS", "sr-RS", "sr_Cyrl_RS", "sr-Cyrl-RS", "sr_Cyrl", "sr-Cyrl", "sr"];

/// File names of a Serbian Latin Hunspell dictionary: Debian's `sr_Latn_RS`, LibreOffice's
/// `sr-Latn`, and `sh` (Serbo-Croatian, the code older Serbian Latin dictionaries use).
pub const LATIN_NAMES: &[&str] = &["sr_Latn_RS", "sr-Latn-RS", "sr_Latn", "sr-Latn", "sh_RS", "sh-RS", "sh"];

impl Script {
    fn slot(self) -> &'static RwLock<DictState> {
        match self {
            Script::Cyrillic => &CYRILLIC,
            Script::Latin => &LATIN,
        }
    }

    /// The file names searched for, in order; the first is the one a "no dictionary" hint names.
    pub fn names(self) -> &'static [&'static str] {
        match self {
            Script::Cyrillic => CYRILLIC_NAMES,
            Script::Latin => LATIN_NAMES,
        }
    }

    /// The Hunspell name a "no dictionary" hint names (`sr_RS`, `sr_Latn_RS`).
    pub fn dictionary_name(self) -> &'static str {
        self.names().first().copied().unwrap_or("sr_RS")
    }

    fn label(self) -> &'static str {
        match self {
            Script::Cyrillic => "Serbian (Cyrillic)",
            Script::Latin => "Serbian (Latin)",
        }
    }
}

/// The script of a proofing language's own dictionary: Latin for [`crate::ProofLang::SrLatn`],
/// Cyrillic otherwise.
pub fn script_of(lang: crate::ProofLang) -> Script {
    if lang == crate::ProofLang::SrLatn { Script::Latin } else { Script::Cyrillic }
}

/// Is `c` a letter of the Cyrillic script?
fn is_cyrillic(c: char) -> bool {
    c.is_alphabetic() && matches!(c, '\u{0400}'..='\u{052F}' | '\u{1C80}'..='\u{1C8F}' | '\u{2DE0}'..='\u{2DFF}' | '\u{A640}'..='\u{A69F}')
}

/// Is `c` a letter of the Latin script?
fn is_latin(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// The script of a word: the one most of its letters are in (the first letter's on a tie);
/// `None` when it has no Cyrillic or Latin letter (Greek, digits…). A word mixing the scripts (a
/// keyboard switched mid-word, `кoмпјутер` with a Latin `o`) goes to its main script's
/// dictionary, which doesn't have it, so it is marked.
pub fn word_script(word: &str) -> Option<Script> {
    let (mut cyrillic, mut latin, mut first) = (0usize, 0usize, None);
    for c in word.chars() {
        let s = if is_cyrillic(c) {
            cyrillic += 1;
            Script::Cyrillic
        } else if is_latin(c) {
            latin += 1;
            Script::Latin
        } else {
            continue;
        };
        first.get_or_insert(s);
    }
    match cyrillic.cmp(&latin) {
        std::cmp::Ordering::Greater => Some(Script::Cyrillic),
        std::cmp::Ordering::Less => Some(Script::Latin),
        std::cmp::Ordering::Equal => first,
    }
}

/// The Serbian dictionary of `script`: the installed one (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (words in that script aren't
/// checked then; see [`has_dictionary`]).
pub fn dictionary(script: Script) -> Arc<AffixDict> {
    let slot = script.slot();
    if let Some(d) = slot.read().unwrap_or_else(std::sync::PoisonError::into_inner).dict.as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed(script));
    let mut state = slot.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    state.dict.get_or_insert(loaded).clone()
}

fn load_installed(script: Script) -> AffixDict {
    let names = script.names();
    let key = script.dictionary_name();
    let Some((aff, dic)) = crate::hunspell::find(names) else {
        crate::hunspell::clear_load_error(key);
        log::info!("{} spelling: no Hunspell {key} dictionary installed", script.label());
        return AffixDict::default();
    };
    crate::hunspell::note_load(key, &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("{} spelling: {e}", script.label());
        AffixDict::default()
    })
}

/// Use this Serbian dictionary for `script` from now on (Hunspell `.aff` and `.dic` text, decoded
/// to UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(script: Script, aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    let mut state = script.slot().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.dict = Some(Arc::new(d));
    state.in_memory = true;
    Ok(())
}

/// Forget both loaded dictionaries, so the next check looks for installed ones again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    for script in [Script::Cyrillic, Script::Latin] {
        let mut state = script.slot().write().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.dict = None;
        state.in_memory = false;
    }
}

/// The `.dic` file of the installed Serbian dictionary of `script`, searching the dictionaries
/// folder and the system locations now (File › Options › Proofing shows it); `None` when there is
/// none, or the dictionary was given with [`install_dictionary`].
pub fn installed_path(script: Script) -> Option<std::path::PathBuf> {
    if script.slot().read().unwrap_or_else(std::sync::PoisonError::into_inner).in_memory {
        return None;
    }
    crate::hunspell::find(script.names()).map(|(_, dic)| dic)
}

/// Is the Serbian dictionary of `script` available (words in it are checked)?
pub fn has_dictionary(script: Script) -> bool {
    !dictionary(script).is_empty()
}

/// Is `word` spelled correctly in Serbian? It is looked up in the dictionary of its own script
/// ([`word_script`]). Numbers, single letters, short words in capitals (acronyms: `САД`, `EU`),
/// words with digits and words in other scripts are accepted, and so is every word whose
/// script has no dictionary installed. A hyphenated word is correct when each part is.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    let Some(script) = word_script(word) else { return true };
    check(&dictionary(script), word, opts)
}

/// [`is_correct_with_options`] with the default options, against a given dictionary (an empty
/// one accepts everything). The caller picks the dictionary; [`is_correct_with_options`] picks
/// it by the word's script.
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true; // no dictionary for this script: no spelling marks
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
    if !w.chars().any(|c| is_cyrillic(c) || is_latin(c)) {
        return true; // other scripts: no dictionary
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

/// Spelling suggestions for a Serbian word, best first, at most `max`, in the word's case and
/// script: from the dictionary of the word's script, with the dictionary-driven ranking Czech
/// uses. None when the word's script has no dictionary.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    let Some(script) = word_script(word) else { return Vec::new() };
    crate::cs::suggest::suggest_in(&dictionary(script), word, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProofLang;

    #[test]
    fn words_are_sorted_by_their_own_script() {
        assert_eq!(word_script("рачунар"), Some(Script::Cyrillic));
        assert_eq!(word_script("računar"), Some(Script::Latin));
        assert_eq!(word_script("Љубав"), Some(Script::Cyrillic));
        assert_eq!(word_script("džep"), Some(Script::Latin));
        // Mixed: most letters win (a Latin `o` in a Cyrillic word), the first letter on a tie.
        assert_eq!(word_script("кoмпјутер"), Some(Script::Cyrillic));
        assert_eq!(word_script("kompjuтер"), Some(Script::Latin));
        assert_eq!(word_script("аb"), Some(Script::Cyrillic));
        assert_eq!(word_script("bа"), Some(Script::Latin));
        for w in ["", "2026", "λόγος", "日本", "--"] {
            assert_eq!(word_script(w), None, "{w}");
        }
    }

    /// The file names distributions and LibreOffice use are found, each for its own script, and
    /// a Latin file is never taken for the Cyrillic dictionary (or the other way round).
    #[test]
    fn dictionary_files_are_found_by_script() {
        let dir = std::env::temp_dir().join(format!("wordcraft-sr-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(crate::hunspell::find_in(&dir, CYRILLIC_NAMES).is_none(), "nothing installed: nothing found");
        for (name, script) in [
            ("sr_RS", Script::Cyrillic),
            ("sr-Cyrl-RS", Script::Cyrillic),
            ("sr", Script::Cyrillic),
            ("sr_Latn_RS", Script::Latin),
            ("sr-Latn", Script::Latin),
            ("sh_RS", Script::Latin),
            ("sh", Script::Latin),
        ] {
            let (aff, dic) = match script {
                Script::Cyrillic => (test_dict::CYRILLIC_AFF, test_dict::CYRILLIC_DIC),
                Script::Latin => (test_dict::LATIN_AFF, test_dict::LATIN_DIC),
            };
            std::fs::write(dir.join(format!("{name}.aff")), aff).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), dic).unwrap();
            let other = if script == Script::Cyrillic { Script::Latin } else { Script::Cyrillic };
            assert!(crate::hunspell::find_in(&dir, other.names()).is_none(), "{name} is not a {other:?} dictionary");
            let (aff, dic) = crate::hunspell::find_in(&dir, script.names()).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            let word = if script == Script::Cyrillic { "градови" } else { "gradovi" };
            assert!(is_correct_with(&d, word), "{name}");
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        // Debian's `sr_ME` doesn't say its script, so it is neither.
        std::fs::write(dir.join("sr_ME.aff"), test_dict::CYRILLIC_AFF).unwrap();
        std::fs::write(dir.join("sr_ME.dic"), test_dict::CYRILLIC_DIC).unwrap();
        assert!(crate::hunspell::find_in(&dir, CYRILLIC_NAMES).is_none() && crate::hunspell::find_in(&dir, LATIN_NAMES).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fixtures_are_flag_num_dictionaries() {
        for aff in [test_dict::CYRILLIC_AFF, test_dict::LATIN_AFF] {
            assert!(aff.lines().any(|l| l.trim() == "FLAG num"));
        }
        for dic in [test_dict::CYRILLIC_DIC, test_dict::LATIN_DIC] {
            assert!(dic.lines().any(|l| l.split_once('/').is_some_and(|(_, f)| f.contains(','))), "a stem with several numeric flags");
        }
        assert!(test_dict::CYRILLIC_DIC.chars().any(is_cyrillic) && !test_dict::CYRILLIC_DIC.chars().any(is_latin));
        assert!(test_dict::LATIN_DIC.chars().any(is_latin) && !test_dict::LATIN_DIC.chars().any(is_cyrillic));
    }

    #[test]
    fn cyrillic_words_affixes_case_and_forbidden_words() {
        test_dict::install();
        assert!(has_dictionary(Script::Cyrillic));
        let opts = crate::Options::default();
        for w in [
            "град",
            "града",
            "граду",
            "градови",
            "градовима",
            "кућа",
            "куће",
            "кући",
            "кућом",
            "Београд",
            "Београду",
            "рачунар",
            "рачунара",
            "универзитет",
            "универзитету",
            "књига",
            "књиге",
            "читати",
            "читам",
            "чита",
            "читају",
            "прочитати",
            "прочитам",
            "Кућа",
            "КУЋА",
            "црно-бели",
            "САД",
        ] {
            assert!(is_correct_with_options(w, &opts), "{w}");
        }
        for w in ["градд", "куча", "кучом", "београд", "рацунар", "универзитетт", "читаммо", "кућa", "зкзабрањено", "Зкзабрањено", "црно-белии"]
        {
            assert!(!is_correct_with_options(w, &opts), "{w}");
        }
    }

    #[test]
    fn latin_words_affixes_case_and_forbidden_words() {
        test_dict::install();
        assert!(has_dictionary(Script::Latin));
        let opts = crate::Options::default();
        for w in [
            "grad",
            "grada",
            "gradovi",
            "kuća",
            "kući",
            "kućom",
            "Beograd",
            "Beogradu",
            "računar",
            "računara",
            "univerzitet",
            "knjiga",
            "knjige",
            "čitati",
            "čitam",
            "čitaju",
            "pročitati",
            "Kuća",
            "KUĆA",
            "crno-beli",
            "džep",
            "džepovi",
            "ljubav",
            "ljubavlju",
            "EU",
        ] {
            assert!(is_correct_with_options(w, &opts), "{w}");
        }
        for w in ["gradd", "kuca", "kucom", "beograd", "racunar", "univerzitett", "čitammo", "zkzabranjeno", "Zkzabranjeno", "dzep", "ljubavljuu"] {
            assert!(!is_correct_with_options(w, &opts), "{w}");
        }
    }

    /// #4968: scripts are never crossed. With only one script's dictionary, words in the other
    /// script aren't marked (they'd all be wrong in it); the dictionaries themselves don't know
    /// each other's words.
    #[test]
    fn a_word_is_never_checked_against_the_other_scripts_dictionary() {
        let cyrillic = AffixDict::from_hunspell(test_dict::CYRILLIC_AFF, test_dict::CYRILLIC_DIC, &[]).unwrap();
        let latin = AffixDict::from_hunspell(test_dict::LATIN_AFF, test_dict::LATIN_DIC, &[]).unwrap();
        assert!(!is_correct_with(&cyrillic, "računar") && !is_correct_with(&latin, "рачунар"), "crossing would mark every word");
        test_dict::install();
        for (word, script) in [("računar", Script::Latin), ("рачунар", Script::Cyrillic), ("racunar", Script::Latin), ("рацунар", Script::Cyrillic)]
        {
            assert_eq!(word_script(word), Some(script));
            let right = if script == Script::Cyrillic { &cyrillic } else { &latin };
            assert_eq!(is_correct_with_options(word, &crate::Options::default()), is_correct_with(right, word), "{word}");
        }
        // Suggestions come from the word's own script.
        assert_eq!(suggest("racunar", 4).first().map(String::as_str), Some("računar"));
        assert_eq!(suggest("рацунар", 4).first().map(String::as_str), Some("рачунар"));
        assert!(suggest("racunar", 8).iter().all(|s| word_script(s) == Some(Script::Latin)));
        assert!(suggest("рацунар", 8).iter().all(|s| word_script(s) == Some(Script::Cyrillic)));
    }

    /// #4968 through the crate's entry points: a run's tag picks the row, each word its script's
    /// dictionary, in a paragraph that mixes the scripts and English.
    #[test]
    fn tagged_runs_route_each_word_to_its_scripts_dictionary() {
        test_dict::install();
        for tag in ["sr-Latn-RS", "sh_RS", "sr-Latn"] {
            assert_eq!(ProofLang::from_tag(Some(tag)), ProofLang::SrLatn, "{tag}");
            assert!(crate::has_dictionary(Some(tag)) && crate::is_correct_for("računar", Some(tag)), "{tag}");
            assert!(!crate::is_correct_for("racunar", Some(tag)), "{tag}");
        }
        for tag in ["sr-Cyrl-RS", "sr_RS", "sr"] {
            assert_eq!(ProofLang::from_tag(Some(tag)), ProofLang::Sr, "{tag}");
            assert!(crate::is_correct_for("рачунар", Some(tag)) && !crate::is_correct_for("рацунар", Some(tag)), "{tag}");
        }
        // One Serbian Latin run with a Cyrillic word in it, then a Cyrillic run with Latin words:
        // every correct word passes, in either script, and each mistake is found once.
        let text = "Kuća u gradu кући i kucom. Град у кући i racunar градд. Good wrods.";
        let latin_end = text.find(" Град").unwrap();
        let cyrillic_end = text.find(" Good").unwrap();
        let spans = [(0..latin_end, ProofLang::SrLatn), (latin_end + 1..cyrillic_end, ProofLang::Sr), (cyrillic_end + 1..text.len(), ProofLang::En)];
        let issues = crate::check_text(text, &spans);
        let marked: Vec<(&str, ProofLang, &str)> = issues.iter().map(|i| (&text[i.start..i.end], i.lang, i.message.as_str())).collect();
        assert_eq!(
            marked,
            [
                ("kucom", ProofLang::SrLatn, "Moguća pravopisna greška"),
                ("racunar", ProofLang::Sr, "Могућа правописна грешка"),
                ("градд", ProofLang::Sr, "Могућа правописна грешка"),
                ("wrods", ProofLang::En, "Possible spelling mistake"),
            ],
            "{issues:?}"
        );
        // The Latin mistake in the Cyrillic run gets Latin suggestions.
        assert_eq!(crate::suggest_in("racunar", 4, ProofLang::Sr).first().map(String::as_str), Some("računar"));
        assert_eq!(crate::suggest_in("градд", 4, ProofLang::SrLatn).first().map(String::as_str), Some("град"));
        assert!(crate::check_grammar_in("Град град је.", ProofLang::Sr).is_empty(), "no Serbian grammar rules");
        let opts = crate::Options { ignore_uppercase: false, ..Default::default() };
        assert!(
            !crate::is_correct_in_with("ЗКЗАБРАЊЕНО", ProofLang::Sr, &opts) && !crate::is_correct_in_with("ZKZABRANJENO", ProofLang::SrLatn, &opts)
        );
        assert!(crate::is_correct_in_with("КУЋА", ProofLang::Sr, &opts) && !crate::is_correct_in_with("КУЧА", ProofLang::Sr, &opts));
    }

    #[test]
    fn suggestions_in_both_scripts_keep_the_case() {
        test_dict::install();
        for (word, want) in [
            ("универзитетт", "универзитет"),
            ("Беорград", "Београд"),
            ("кућма", "кућом"),
            ("грда", "града"),
            ("univerzitett", "univerzitet"),
            ("Beorgrad", "Beograd"),
            ("kuca", "kuća"),
            ("dzep", "džep"),
            ("KNIJGA", "KNJIGA"),
        ] {
            let got = suggest(word, 6);
            assert!(got.contains(&want.to_string()), "{word}: {got:?}");
        }
        assert!(!suggest("зкзабрањеноо", 8).iter().any(|s| s == "зкзабрањено"), "forbidden words aren't offered");
        assert!(suggest("λόγος", 4).is_empty() && suggest("", 4).is_empty());
    }

    /// A missing dictionary is the usual not-found state: no marks, no suggestions, and the hint
    /// names the dictionary of the run's own script. No crash on a dictionary that doesn't read.
    #[test]
    fn a_missing_or_broken_dictionary_marks_nothing() {
        let empty = AffixDict::default();
        for w in ["рацунар", "racunar", "зкзабрањено"] {
            assert!(is_correct_with(&empty, w), "{w}");
        }
        assert!(crate::cs::suggest::suggest_in(&empty, "racunar", 4).is_empty());
        assert_eq!(Script::Cyrillic.dictionary_name(), "sr_RS");
        assert_eq!(Script::Latin.dictionary_name(), "sr_Latn_RS");
        assert_eq!(crate::dictionary_needed(ProofLang::Sr, false), Some("sr_RS"));
        assert_eq!(crate::dictionary_needed(ProofLang::SrLatn, false), Some("sr_Latn_RS"));
        assert_eq!(crate::dictionary_needed(ProofLang::SrLatn, true), None);
        // A dictionary that doesn't parse is refused, and the dictionary in use stays.
        assert!(install_dictionary(Script::Latin, "FLAG bogus\n", "1\ngrad\n").is_err());
        assert!(install_dictionary(Script::Cyrillic, "FLAG num\nSFX 1 Y 1\nSFX 1 0 а .\nFLAG long\n", "1\nград/1\n").is_err());
        let dir = std::env::temp_dir().join(format!("wordcraft-sr-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sr_RS.aff"), "SET KOI8-U\nFLAG num\n").unwrap();
        std::fs::write(dir.join("sr_RS.dic"), "1\nград\n").unwrap();
        let (aff, dic) = crate::hunspell::find_in(&dir, CYRILLIC_NAMES).unwrap();
        assert!(crate::hunspell::load(&aff, &dic).is_err(), "an unreadable dictionary is an error, not a panic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_dictionary_applies_in_both_scripts() {
        test_dict::install();
        let opts = crate::Options::default();
        assert!(!is_correct_with_options("зкмојареч", &opts) && !is_correct_with_options("zkmojareč", &opts));
        crate::add_word("Зкмојареч");
        crate::add_word("zkmojareč");
        assert!(is_correct_with_options("зкмојареч", &opts) && is_correct_with_options("zkmojareč", &opts));
    }

    /// The suggester stays quick on the fixture (the real dictionaries are timed in the PR).
    #[test]
    fn suggestions_are_fast_enough() {
        test_dict::install();
        let _ = suggest("град", 1);
        let t = std::time::Instant::now();
        for word in ["универзитеттт", "рацунара", "zkveomadugačkareč", "pročitatii"] {
            let _ = suggest(word, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        assert!(ms < 20_000.0, "{ms} ms");
    }
}
